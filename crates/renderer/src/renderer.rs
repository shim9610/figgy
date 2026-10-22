//! Public-facing wgpu renderer.
//!
//! `Renderer::try_new` builds the shared GPU resources (pool, axis pipeline,
//! bind-group layouts, sampler, quad VB) in one call so users don't have to
//! wire `data_render::create_*_pipeline` / `ColumnPool::new` manually.
//! Line / scatter / errorbar pipelines and the errorbar extent engine compile
//! on first use.
//!
//! Ownership: holds `Arc<wgpu::Device>` and `Arc<wgpu::Queue>` so the same
//! device can be shared with the host (egui_wgpu, iced_wgpu, etc.) without
//! lifetime gymnastics.
//!
//! `try_new` returns `Result<_, FiggyError>` so unsupported target formats and
//! device resource limits are reported before wgpu validation can abort.

mod export;
pub use export::*;
mod picking;
pub use picking::*;
mod device;
pub use device::*;
mod streaming;
use streaming::*;
mod fit;
pub use fit::*;
mod registry;
pub use registry::*;
mod style;
pub use style::*;
mod preparation;
pub use preparation::{PipelinePreparation, PreparationFeature, PreparedPipelines};

use std::collections::HashMap;
use std::sync::Arc;

mod selection_prepare;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "shader_compile_tests.rs"]
mod shader_compile_tests;

#[path = "streaming_runtime.rs"]
mod streaming_runtime;
pub use streaming_runtime::{
    StreamingOperation, StreamingSelectionRequest, StreamingSelectionTicket,
};
#[cfg(test)]
mod streaming_request_tests;
#[path = "streaming_surface.rs"]
mod streaming_surface;

use crate::axis_render;
use crate::chart::Chart;
use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, Config, DrawStyle, PickedDataRef, PickedPointRef};
use crate::data::{ColumnSource, HiLoColumnSource};
use crate::data_config::{
    DataBarBinStyleConfig, DataBarStyleConfig, DataErrorBarPointStyleConfig,
    DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterPointStyleConfig,
    DataScatterStyleConfig, ErrorRef, LineFill, LineInterpolation, ScatterShape, SeriesConfig,
    SizeScale,
};
use crate::data_render::{
    self, AxisLayer, ColumnBarSelectionLayer, ColumnErrorBarDraw, ColumnFieldSelectionLayer,
    ColumnHandle, ColumnId, ColumnLineLayer, ColumnPickRingLayer, ColumnPool, ColumnScatterLayer,
    DefragPolicy, ERRORBAR_HAS_X, ERRORBAR_HAS_Y, PrimitiveStyle,
};
use crate::error::{FiggyError, Result};
use crate::gpu_memory::{
    GpuLedger, GpuMemoryUsage, GpuResourceKind, ResidentAdmission, ResidentAdmissionRequest,
    ResidentAdmissionStatus, TrackedBuffer, TrackedTexture,
};
use crate::init::{
    InitEvent, finished, observe_result, observe_result_async, observe_value, observe_value_async,
    started,
};
use crate::layout::Rect;
use crate::line::LineStylePreset;
use crate::select::HitId;

/// Uniformly fit a logical chart document into a physical surface.
pub fn fit_display_panel(logical_size: (u32, u32), surface_size: (u32, u32)) -> (f32, Rect) {
    let doc_w = logical_size.0.max(1) as f32;
    let doc_h = logical_size.1.max(1) as f32;
    let surface_w = surface_size.0.max(1);
    let surface_h = surface_size.1.max(1);
    let scale = ((surface_w as f32) / doc_w).min((surface_h as f32) / doc_h);
    let panel_w = ((doc_w * scale).round().max(1.0) as u32).min(surface_w);
    let panel_h = ((doc_h * scale).round().max(1.0) as u32).min(surface_h);
    let x = (surface_w - panel_w) / 2;
    let y = (surface_h - panel_h) / 2;
    (
        scale,
        Rect {
            x,
            y,
            width: panel_w,
            height: panel_h,
        },
    )
}

/// The annotation raster of a chart at its config's scale: the config
/// already carries export or display scaling in its pixel dims, exactly as
/// the axis raster is drawn, so the annotation raster is drawn at 1:1.
fn annotation_raster_for_chart(
    chart: &Chart,
    selected: Option<usize>,
) -> crate::annotation::AnnotationRaster {
    let config = chart.config();
    let area = config.chart_area.0;
    crate::annotation::rasterize(&crate::annotation::RasterRequest {
        layer: &config.annotations,
        space: crate::annotation::pixel_space_for_config(config),
        origin: [area.x as f32, area.y as f32],
        size: (area.width.max(1), area.height.max(1)),
        scale: 1.0,
        selected,
        policy: crate::text_render::FontPolicy::for_style(&config.draw_style),
    })
}

/// Derive the scaled display config and centered panel for a surface.
///
/// The whole document ([`Config::document_area`], the union of the parent's
/// area and every subplot's area) is fitted to the surface. The returned
/// rect is where the parent's `chart_area` lands, the `panel_rect` that
/// `create_chart_view` and `refresh_axis` take; for a document without
/// subplots it is the centered panel itself.
pub fn display_config_for_surface(
    config: &Config,
    surface_size: (u32, u32),
) -> (Config, Rect, f32) {
    let logical = config.document_area();
    let (scale, document_rect) = fit_display_panel((logical.width, logical.height), surface_size);
    let display_config = place_document(config, scale, document_rect);
    let panel_rect = display_config.chart_area.0;
    (display_config, panel_rect, scale)
}

/// `config` scaled by `scale` and moved so its document area starts at
/// `document_rect`'s origin. A document without subplots is placed exactly
/// as before subplots existed: its `chart_area` becomes `document_rect`.
pub fn place_document(config: &Config, scale: f32, document_rect: Rect) -> Config {
    let mut placed = config.scaled(scale);
    if config.subplots.is_empty() {
        placed.chart_area = crate::layout::ChartArea(document_rect);
        return placed;
    }
    let document = placed.document_area();
    placed.translate_areas(
        i64::from(document_rect.x) - i64::from(document.x),
        i64::from(document_rect.y) - i64::from(document.y),
    );
    placed
}

/// A grid declaration resolved against the pool — see `Renderer::resolve_grid`.
struct ResolvedGrid {
    x: ColumnHandle,
    y: ColumnHandle,
    grid: Vec<data_render::GridColumnGpu>,
    epochs: Vec<u64>,
    extent: MatrixExtent,
    columns_are_y: bool,
    centers: bool,
}

/// The contour levels a render type declares, as the f32s both the fill's band
/// quantization and the implicit contour shader compare against.
///
/// Padded to one entry when there are none: a storage binding cannot be
/// zero-sized, and the real count travels in the params uniform.
/// What every colour-mapped primitive's GPU params take from the chart
/// colourbar: the z bounds as hi/lo pairs, already log10 under a logarithmic
/// bar, and the premultiplied `nan_color`.
///
/// The log bounds are taken here, in f64: doing it in the shader would lose
/// the precision the bounds were written with, and `validate_renderer_config`
/// has already refused a log colourbar whose lower bound is not positive. One
/// place for the field, the point colour map and the vector colour map, so
/// their shaders (which share the ramp, SHADER_COMMON.md §5.4) read bounds
/// computed the same way.
struct ColorbarGpu {
    log_z: bool,
    z_min: [f32; 2],
    z_max: [f32; 2],
    nan_color_premul: [f32; 4],
}

fn colorbar_gpu(bar: &crate::config::ColorBarOptions) -> ColorbarGpu {
    let log_z = matches!(bar.axis.scale, AxisScale::Logarithmic);
    let bound = |value: f64| {
        let transformed = if log_z { value.log10() } else { value };
        let (hi, lo) = crate::data::split_f64_to_f32_pair(transformed);
        [hi, lo]
    };
    let nan = bar.nan_color;
    let alpha = nan.a.clamp(0.0, 1.0);
    ColorbarGpu {
        log_z,
        z_min: bound(bar.axis.min),
        z_max: bound(bar.axis.max),
        nan_color_premul: [nan.r * alpha, nan.g * alpha, nan.b * alpha, alpha],
    }
}

/// The colourbar's ramp control points as straight RGBA for a storage
/// binding. `stop_count` is the real count; the table keeps one padding entry
/// when the map has none, because a storage binding cannot be zero-sized.
struct ColorbarStops {
    stops: Vec<[f32; 4]>,
    stop_count: u32,
}

fn colorbar_stops(
    label: &'static str,
    bar: &crate::config::ColorBarOptions,
) -> Result<ColorbarStops> {
    let mut stops = crate::gpu_contour::try_collect(
        label,
        bar.colormap
            .stops()
            .iter()
            .map(|color| [color.r, color.g, color.b, color.a]),
    )?;
    let stop_count = u32::try_from(stops.len()).unwrap_or(u32::MAX);
    if stops.is_empty() {
        stops.push([0.0; 4]);
    }
    Ok(ColorbarStops { stops, stop_count })
}

fn contour_levels(rt: &DataRenderType) -> Result<Vec<f32>> {
    let empty: [f64; 0] = [];
    let source = extract_contour(rt).map_or(&empty[..], |contour| &contour.levels);
    let mut levels =
        crate::gpu_contour::try_collect("contour levels", source.iter().map(|l| *l as f32))?;
    if levels.is_empty() {
        levels.push(0.0);
    }
    Ok(levels)
}

/// How many levels the declaration actually names — the padding
/// `contour_levels` adds is not one of them.
fn declared_level_count(rt: &DataRenderType) -> usize {
    extract_contour(rt).map_or(0, |contour| contour.levels.len())
}

fn validate_contour_level_count(series_id: &str, rt: &DataRenderType) -> Result<()> {
    let count = declared_level_count(rt);
    if count > crate::data_config::MAX_CONTOUR_LEVELS {
        return Err(FiggyError::InvalidSeriesConfig {
            series_id: series_id.to_owned(),
            reason: format!(
                "contour level count {count} exceeds the supported maximum {}",
                crate::data_config::MAX_CONTOUR_LEVELS
            ),
        });
    }
    Ok(())
}

fn validate_contour_label_config(series_id: &str, rt: &DataRenderType) -> Result<()> {
    let Some(labels) = extract_contour(rt).and_then(|contour| contour.labels.as_ref()) else {
        return Ok(());
    };
    labels
        .validate()
        .map_err(|reason| FiggyError::InvalidSeriesConfig {
            series_id: series_id.to_owned(),
            reason: reason.to_owned(),
        })
}

fn contour_labels_use_automatic(rt: &DataRenderType) -> bool {
    let Some(contour) = extract_contour(rt) else {
        return false;
    };
    let Some(labels) = contour.labels.as_ref() else {
        return false;
    };
    labels.visible
        && labels.font_size > 0.0
        && !contour.levels.is_empty()
        && !labels
            .anchors
            .iter()
            .any(|anchor| (anchor.level_index as usize) < contour.levels.len())
}

/// Validate the frame-dependent spacing product and return the exact value the
/// automatic dispatch must consume. Explicit placement never multiplies it.
fn preflight_contour_label_spacing(
    series_id: &str,
    rt: &DataRenderType,
    contour_enabled: bool,
    scale: f32,
) -> Result<Option<f32>> {
    validate_contour_label_config(series_id, rt)?;
    if !contour_enabled || !contour_labels_use_automatic(rt) {
        return Ok(None);
    }
    let labels = extract_contour(rt)
        .and_then(|contour| contour.labels.as_ref())
        .expect("automatic contour labels have a label config");
    let spacing_px = labels.spacing_px * scale;
    if !spacing_px.is_finite() || spacing_px <= 0.0 {
        return Err(FiggyError::InvalidSeriesConfig {
            series_id: series_id.to_owned(),
            reason:
                "automatic contour label spacing_px * scale must be finite and greater than zero"
                    .to_owned(),
        });
    }
    Ok(Some(spacing_px))
}

/// Whether a series declares contour labels that would actually be drawn.
///
/// Hidden labels and a non-positive font size are the same answer as no label
/// declaration at all — nothing to bake, so nothing to compile a pipeline for.
fn series_wants_contour_labels(rt: &DataRenderType) -> bool {
    extract_contour(rt)
        .and_then(|contour| contour.labels.as_ref())
        .is_some_and(|labels| labels.visible && labels.font_size > 0.0)
}

/// Clear a per-series scratch cache that is about to exceed its bound.
///
/// Same rule as `arc_cache`: a host cycling through thousands of series ids must
/// not grow a cache without limit, and dropping the lot is simpler than a
/// recency policy nothing has asked for.
fn evict_if_full<V>(cache: &mut HashMap<String, V>, series_id: &str, limit: usize) {
    let known = cache.contains_key(series_id);
    if cache.len() > limit || (!known && cache.len() >= limit) {
        cache.clear();
    }
}

/// Per-series contour scratch cache limit, mirroring `ARC_PREFIX_CACHE_LIMIT`.
const CONTOUR_SCRATCH_CACHE_LIMIT: usize = 256;

/// What a cached contour's GPU state was resolved from.
///
/// Equality means the same isolines would be drawn: the levels, their colours,
/// the grid, the coordinate columns and the pool layout are all in it. Shaped
/// like [`FieldSignature`] because since design B.4.9 a contour *is* a field
/// draw — the same quad and the same group-2 tables, through `fs_contour`.
///
/// The colourmap stops are absent on purpose: `fs_contour` takes its colour from
/// `level_colors` and never samples the ramp, so the stop table is padding for
/// the layout and cannot change what is drawn.
#[derive(Debug, PartialEq)]
struct ContourSignature {
    pool_layout_generation: u64,
    params: data_render::FieldParamsGpu,
    grid: Vec<data_render::GridColumnGpu>,
    epochs: Vec<u64>,
    levels: Vec<f32>,
    /// Per-level stroke colour, premultiplied — as drawn, so comparing
    /// signatures compares pixels.
    level_colors: Vec<[f32; 4]>,
    /// `None` when the series draws no labels.
    label: Option<ContourLabelSignature>,
}

/// What a contour series' baked label atlas and anchor buffer were resolved
/// from — the part of it not already in [`ContourSignature`].
///
/// The level GPU values and stroke colours are in the outer signature already,
/// so this holds only what the atlas adds: the final strings, the independent
/// label colour, the face, the background, and any host anchor override.
/// `spacing_px` is
/// deliberately absent — it is a screen distance the anchor pass reads every
/// frame, not something an atlas is baked for.
#[derive(Debug, PartialEq)]
struct ContourLabelSignature {
    /// Final formatted strings are the atlas' exact content authority. This
    /// captures f64 level distinctions and colourbar/level-spacing changes
    /// without accidentally keying the atlas to an unrelated chart axis.
    texts: Vec<String>,
    /// `font_size * scale` — an export at 2x re-bakes rather than magnifying.
    font_bits: u32,
    color: [u32; 4],
    policy: crate::text_render::FontPolicy,
    family: String,
    bg: Option<[u32; 4]>,
    bg_padding_bits: u32,
    /// Runtime font registration invalidates a previously baked atlas even
    /// when every chart-owned formatting field is unchanged.
    font_generation: u64,
    /// The resolved host override. Empty means the GPU picks the anchors.
    anchors: Vec<crate::gpu_contour::LabelAnchorGpu>,
}

/// One contour series' resolved GPU state — the same shape a field's is.
struct ContourScratchEntry {
    signature: ContourSignature,
    style_bg: wgpu::BindGroup,
    field_bg: wgpu::BindGroup,
    charge: crate::gpu_memory::SharedCharge,
    sources: Arc<GridSources>,
    /// False when the grid has no cell to interpolate a contour across.
    drawable: bool,
    /// `None` when the series draws no labels — hidden, no label declaration, or
    /// every formatted string empty. Atlas/device-limit failures are errors.
    label: Option<crate::gpu_contour::ContourLabelState>,
}

/// Per-series field scratch cache limit, mirroring `ARC_PREFIX_CACHE_LIMIT`.
const FIELD_SCRATCH_CACHE_LIMIT: usize = 256;
/// Per-series vector scratch cache limit, mirroring `FIELD_SCRATCH_CACHE_LIMIT`.
const VECTOR_SCRATCH_CACHE_LIMIT: usize = 256;

/// Everything a vector series' group-2 state depends on. Compared before any
/// upload, so a steady frame builds nothing.
#[derive(Clone, Debug, PartialEq)]
struct VectorSignature {
    pool_layout_generation: u64,
    params: data_render::vector::VectorParamsGpu,
    grid: Vec<data_render::GridColumnGpu>,
    epochs: Vec<u64>,
    stops: Vec<[f32; 4]>,
    /// Series colour bits: the group-1 style is built here too.
    color: [u32; 4],
}

/// One vector series' resolved GPU state.
struct VectorScratch {
    signature: VectorSignature,
    input: data_render::vector::VectorInput,
    style_bg: wgpu::BindGroup,
    vector_bg: wgpu::BindGroup,
    charge: crate::gpu_memory::SharedCharge,
    /// Grid form only: the `u` grid as a field's group 2, so the selected
    /// cell highlight runs the field pipeline unchanged. The grid's arrow
    /// pick binds `vector_bg` (`pick_vector_cell`).
    field: Option<(wgpu::BindGroup, crate::gpu_memory::SharedCharge)>,
    sources: Arc<GridSources>,
    /// Arrows the draw issues; zero draws nothing.
    count: u32,
}

/// Per-series point value-map scratch cache limit, mirroring the field limit.
const VALUE_MAP_SCRATCH_CACHE_LIMIT: usize = 256;

/// What a cached point value-map bind group (group 3) was built from: the
/// params uniform, compared by its bytes so a NaN `nan_color` still compares
/// equal to itself, and the colourbar's colormap (SSOT config, copied). Equality
/// means the cached bind group is still exactly right, so a prepare that
/// changes nothing uploads nothing — the same contract as [`FieldSignature`].
#[derive(Debug)]
struct ValueMapSignature {
    params: data_render::PointValueMapParamsGpu,
    colormap: Option<crate::colormap::ColorMap>,
}

impl PartialEq for ValueMapSignature {
    fn eq(&self, other: &Self) -> bool {
        bytemuck::bytes_of(&self.params) == bytemuck::bytes_of(&other.params)
            && self.colormap == other.colormap
    }
}

/// One scatter series' resolved value-map state: the group-3 bind group of
/// the mapped scatter and pick-ring pipelines.
struct ValueMapScratch {
    signature: ValueMapSignature,
    bind_group: wgpu::BindGroup,
    _charge: crate::gpu_memory::SharedCharge,
}

/// The grid columns a prepared field or contour read, and the allocation epochs
/// they had.
///
/// Shared with the `PreparedFrame` by `Arc`, so validating a token walks it
/// without allocating and a matrix' thousands of columns cost one pointer in the
/// token rather than a `HashMap` entry each.
#[derive(Debug)]
struct GridSources {
    ids: Vec<ColumnId>,
    epochs: Vec<u64>,
}

/// What a cached field bind group was built from.
///
/// Everything the group-2 buffers hold, plus the pool layout it was resolved
/// against. Equality means the cached bind group is still exactly right, so a
/// prepare that changes nothing uploads nothing — which is the difference
/// between a 40 KB grid table per frame and one per change.
#[derive(Debug, PartialEq)]
struct FieldSignature {
    pool_layout_generation: u64,
    params: data_render::FieldParamsGpu,
    grid: Vec<data_render::GridColumnGpu>,
    epochs: Vec<u64>,
    levels: Vec<f32>,
    stops: Vec<[f32; 4]>,
    nan_color: [u32; 4],
}

/// One field series' resolved GPU state.
struct FieldScratch {
    signature: FieldSignature,
    style_bg: wgpu::BindGroup,
    field_bg: wgpu::BindGroup,
    charge: crate::gpu_memory::SharedCharge,
    sources: Arc<GridSources>,
    drawable: bool,
}

/// A dashed series' GPU arc-length prefix: the buffer (bound as the line
/// pipeline's vertex slots 4/5) and its used byte length.
type ArcPrefix = (Arc<wgpu::Buffer>, u64);
const ARC_PREFIX_CACHE_LIMIT: usize = 256;
const ARC_RESULT_CACHE_LIMIT_PER_SERIES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ArcResultKey {
    n: u32,
    x_base: u32,
    y_base: u32,
    pool_layout_generation: u64,
    x_allocation_epoch: u64,
    y_allocation_epoch: u64,
    /// The arc scan reads the first 64 bytes of `ScatterTransform` and its
    /// polar words (`polar_angle`, `polar_extent`, the last 32 bytes); the
    /// style parameters between them are left out, so a style change does
    /// not retraverse the data.
    geometry_transform_bits: [u32; 16],
    polar_transform_bits: [u32; 8],
    star_pitch_bits: Option<u32>,
    /// Step interpolation mode: a stepped path has a different arc length.
    step_mode: u32,
}

struct ArcResultEntry {
    key: ArcResultKey,
    scratch: data_render::line_arc::ArcScratch,
}

/// Stable identity for one renderer-owned chart state.
///
/// Values are issued monotonically by a [`Renderer`] and are never reused.
/// The inner value is intentionally private so hosts cannot synthesize ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChartId {
    renderer_identity: u64,
    sequence: u64,
}

/// Opaque checked identity for renderer-owned visual state.
///
/// Hosts may compare or hash this value, but cannot construct one or perform
/// arithmetic on it. All successors are issued by `Renderer` with overflow
/// checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderRevision {
    renderer_identity: u64,
    sequence: u64,
}

impl RenderRevision {
    fn initial(renderer_identity: u64) -> Self {
        Self {
            renderer_identity,
            sequence: 0,
        }
    }

    fn successor(self, counter: &'static str) -> Result<Self> {
        self.sequence
            .checked_add(1)
            .map(|sequence| Self {
                renderer_identity: self.renderer_identity,
                sequence,
            })
            .ok_or(FiggyError::CounterExhausted { counter })
    }
}

/// Opaque O(1) identity for all renderer-visible output, including fonts
/// registered through the process-global font registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RendererVisualStamp {
    revision: RenderRevision,
    font_generation: u64,
}

/// Opaque O(1) identity for one renderer-owned chart's draw dependencies.
///
/// Hosts compare this value instead of cloning `Config` or `SeriesConfig`.
/// The renderer remains the sole owner and decides which revision changes
/// require a draw or raster refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChartRenderStamp {
    renderer_identity: u64,
    chart_id: ChartId,
    desired: RenderRevision,
    raster: RenderRevision,
    annotation: RenderRevision,
}

impl ChartRenderStamp {
    pub fn needs_draw_since(&self, previous: Option<&Self>) -> bool {
        previous != Some(self)
    }

    pub fn needs_raster_since(&self, previous: Option<&Self>) -> bool {
        previous.is_none_or(|previous| {
            previous.renderer_identity != self.renderer_identity
                || previous.chart_id != self.chart_id
                || previous.raster != self.raster
        })
    }

    /// Whether the annotation layer's rasters must be redone since
    /// `previous`: an edit of `annotations.items`. A `refresh_axis` redoes
    /// them as well, so a host that refreshes the axis raster needs no
    /// separate annotation refresh for the same frame; otherwise it calls
    /// [`Renderer::refresh_annotations`]. Toggling `annotations.visible`
    /// never sets this: it only skips the composite.
    pub fn needs_annotation_raster_since(&self, previous: Option<&Self>) -> bool {
        previous.is_none_or(|previous| {
            previous.renderer_identity != self.renderer_identity
                || previous.chart_id != self.chart_id
                || previous.annotation != self.annotation
        })
    }
}

/// Owned snapshot of one series' constellation/milkyway star pass, cloned
/// out of the arc scratch by the prepare phase. wgpu resources are
/// internally refcounted, so these clones keep the GPU objects alive (and
/// the draw phase cache-independent) even if `arc_cache` evicts or rebuilds
/// the entry before `paint_prepared` runs.
struct PreparedStar {
    indirect: wgpu::Buffer,
    vs_bg: wgpu::BindGroup,
}

/// One series' arc-scan snapshot inside a token: the prefix buffer slice the
/// line binds and the star pass when the style has one. The frame's captured
/// column allocation epochs guard the source identity at paint time.
struct PreparedArc {
    prefix: ArcPrefix,
    star: Option<PreparedStar>,
    charge: crate::gpu_memory::SharedCharge,
}

struct PreparedArcSeries {
    arc: Option<PreparedArc>,
}

struct PreparedArcItem {
    view_revision: u64,
    series: Vec<PreparedArcSeries>,
}

struct PreparedContourScratch {
    style_bg: wgpu::BindGroup,
    field_bg: wgpu::BindGroup,
    charge: crate::gpu_memory::SharedCharge,
    drawable: bool,
    label: Option<crate::gpu_contour::ContourLabelSnapshot>,
}

struct PreparedContourSeries {
    scratch: Option<PreparedContourScratch>,
}

struct PreparedContourItem {
    series: Vec<PreparedContourSeries>,
}

struct PreparedSeriesProducts<'a> {
    arcs: &'a [PreparedArcSeries],
    contours: &'a [PreparedContourSeries],
}

struct PreparedAreaLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    pool_buffer: wgpu::Buffer,
    x: ColumnHandle,
    y: ColumnHandle,
    base: ColumnHandle,
    verts_per_instance: u32,
}

struct PreparedLineLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    pool_buffer: wgpu::Buffer,
    x: ColumnHandle,
    y: ColumnHandle,
    arc: Option<ArcPrefix>,
    verts_per_instance: u32,
    texture_bg: Option<wgpu::BindGroup>,
}

struct PreparedScatterLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    style_map_bg: Option<wgpu::BindGroup>,
    value_map_bg: Option<wgpu::BindGroup>,
    quad_vb: wgpu::Buffer,
    pool_buffer: wgpu::Buffer,
    x: ColumnHandle,
    y: ColumnHandle,
    style_index: Option<ColumnHandle>,
    color_value: Option<ColumnHandle>,
    size_value: Option<ColumnHandle>,
    texture_bg: Option<wgpu::BindGroup>,
}

struct PreparedPickRingLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    style_map_bg: Option<wgpu::BindGroup>,
    value_map_bg: Option<wgpu::BindGroup>,
    quad_vb: wgpu::Buffer,
    pool_buffer: wgpu::Buffer,
    x: ColumnHandle,
    y: ColumnHandle,
    style_index: Option<ColumnHandle>,
    color_value: Option<ColumnHandle>,
    size_value: Option<ColumnHandle>,
    instance: u32,
}

struct PreparedBarSelectionLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    selection_bg: wgpu::BindGroup,
    selection_charge: crate::gpu_memory::SharedCharge,
    pool_buffer: wgpu::Buffer,
    edges: ColumnHandle,
    values: ColumnHandle,
    instance: u32,
}

struct PreparedFieldSelectionLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    selection_bg: wgpu::BindGroup,
    selection_charge: crate::gpu_memory::SharedCharge,
    field_bg: wgpu::BindGroup,
    charge: crate::gpu_memory::SharedCharge,
    drawable: bool,
}

struct PreparedStarLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    texture_bg: wgpu::BindGroup,
    star_bg: wgpu::BindGroup,
    indirect: wgpu::Buffer,
}

/// Owned field layer. The grid table, level list, colourmap stops, lookup
/// metadata and `FieldParams` are all inside `field_bg`, which owns their
/// buffers — so the token needs nothing else to redraw the field.
struct PreparedFieldLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    field_bg: wgpu::BindGroup,
    charge: crate::gpu_memory::SharedCharge,
    drawable: bool,
}

/// Owned vector layer: refcounted clones of the group-2 state, like the field.
struct PreparedVectorLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    vector_bg: wgpu::BindGroup,
    charge: crate::gpu_memory::SharedCharge,
    count: u32,
}

/// Owned contour layer. The group-2 tables, the lump charge and the label state
/// are refcounted, so the token draws from its own clones even after a later
/// prepare replaces the scratch they came from.
struct PreparedContourLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    field_bg: wgpu::BindGroup,
    charge: crate::gpu_memory::SharedCharge,
    drawable: bool,
    label: Option<PreparedContourLabel>,
}

/// Owned label layer. Same reasoning as the lines above: the pipeline, bind group
/// and buffers are refcounted clones, so a later prepare replacing the scratch
/// cannot pull them out from under a token that can still be painted.
struct PreparedContourLabel {
    pipeline: wgpu::RenderPipeline,
    snapshot: crate::gpu_contour::ContourLabelSnapshot,
}

struct PreparedBarLayer {
    envelope: Option<data_render::bar_envelope::Snapshot>,
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    style_map_bg: Option<wgpu::BindGroup>,
    pool_buffer: wgpu::Buffer,
    edges: ColumnHandle,
    values: ColumnHandle,
}

struct PreparedErrorBarLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    style_map_bg: Option<wgpu::BindGroup>,
    pool_buffer: wgpu::Buffer,
    x: ColumnHandle,
    y: ColumnHandle,
    err_y_lo: ColumnHandle,
    err_y_hi: ColumnHandle,
    err_x_lo: ColumnHandle,
    err_x_hi: ColumnHandle,
    style_index: Option<ColumnHandle>,
}

/// Complete owned draw packet for one series. Every pipeline, bind group,
/// buffer, and column handle needed by paint is resolved during prepare.
struct PreparedSeries {
    _arc_charge: Option<crate::gpu_memory::SharedCharge>,
    _column_charge: Option<crate::gpu_memory::SharedCharge>,
    _style_charge: Option<crate::gpu_memory::SharedCharge>,
    _stream_style_charge: Option<crate::gpu_memory::SharedCharge>,
    _stream_transform_charge: Option<crate::gpu_memory::SharedCharge>,
    area: Option<PreparedAreaLayer>,
    field: Option<PreparedFieldLayer>,
    bar: Option<PreparedBarLayer>,
    contour: Option<PreparedContourLayer>,
    vector: Option<PreparedVectorLayer>,
    errorbar: Option<PreparedErrorBarLayer>,
    line: Option<PreparedLineLayer>,
    line_extra: Option<PreparedStarLayer>,
    scatter: Option<PreparedScatterLayer>,
    selected_bars: Vec<PreparedBarSelectionLayer>,
    selected_fields: Vec<PreparedFieldSelectionLayer>,
    picked: Vec<PreparedPickRingLayer>,
}

/// Refcounted GPU view bindings captured by `prepare`. Bind groups keep their
/// backing textures/buffers alive, so paint needs no `ChartView` borrow.
struct PreparedView {
    grid_bind_group: wgpu::BindGroup,
    decoration_bind_group: wgpu::BindGroup,
    /// The annotation layer's textures, `None` where the layer drew nothing
    /// on that side; skipped entirely while the layer is hidden.
    annotation_underlay_bind_group: Option<wgpu::BindGroup>,
    annotation_overlay_bind_group: Option<wgpu::BindGroup>,
    annotations_visible: bool,
    _grid_charge: crate::gpu_memory::SharedCharge,
    _decoration_charge: crate::gpu_memory::SharedCharge,
    _annotation_charges: Vec<crate::gpu_memory::SharedCharge>,
    _transform_charge: crate::gpu_memory::SharedCharge,
    content_revision: Arc<std::sync::atomic::AtomicU64>,
    expected_content_revision: u64,
    panel_rect: Rect,
}

impl PreparedView {
    /// Snapshot a view's bindings. `annotations_visible` comes from the
    /// chart config at prepare time, so toggling the layer needs no refresh.
    fn capture(
        view: &ChartView,
        expected_content_revision: u64,
        annotations_visible: bool,
    ) -> Self {
        Self {
            grid_bind_group: view.grid_bind_group.clone(),
            decoration_bind_group: view.decoration_bind_group.clone(),
            annotation_underlay_bind_group: view
                .annotation_underlay
                .as_ref()
                .map(|l| l.bind_group.clone()),
            annotation_overlay_bind_group: view
                .annotation_overlay
                .as_ref()
                .map(|l| l.bind_group.clone()),
            annotations_visible,
            _grid_charge: view.grid_texture.shared_charge(),
            _decoration_charge: view.decoration_texture.shared_charge(),
            _annotation_charges: view
                .annotation_underlay
                .iter()
                .chain(view.annotation_overlay.iter())
                .map(|l| l.texture.shared_charge())
                .collect(),
            _transform_charge: view.transform_buffer.shared_charge(),
            content_revision: Arc::clone(&view.content_revision),
            expected_content_revision,
            panel_rect: view.panel_rect,
        }
    }

    fn annotation_layers<'a>(
        &'a self,
        pipeline: &'a wgpu::RenderPipeline,
    ) -> (Option<AxisLayer<'a>>, Option<AxisLayer<'a>>) {
        if !self.annotations_visible {
            return (None, None);
        }
        let layer = |bind_group: &'a Option<wgpu::BindGroup>| {
            bind_group.as_ref().map(|bind_group| AxisLayer {
                pipeline,
                bind_group,
            })
        };
        (
            layer(&self.annotation_underlay_bind_group),
            layer(&self.annotation_overlay_bind_group),
        )
    }
}

/// Complete owned render input for one panel.
struct PreparedItem {
    view: PreparedView,
    data_area: Rect,
    axis_pipeline: wgpu::RenderPipeline,
    series: Vec<PreparedSeries>,
    stream_display: Option<streaming_runtime::StreamDisplayPacket>,
    /// The panel's cached resident composite, when the panel qualifies; see
    /// [`ResidentDisplay`].
    resident_display: Option<ResidentDisplayPacket>,
    /// [`column_fingerprint`] of the columns this panel's series read through
    /// the pool lookup (grid columns come from the scratch receipts).
    column_fingerprint: u64,
}

/// An order-independent fingerprint of `(column id, allocation epoch)` pairs:
/// it changes when any of the columns is replaced, removed and added again,
/// or relocated into a new allocation, and stays when an unrelated column
/// changes. A sum of per-pair hashes, so it needs no sorted copy.
fn column_fingerprint<'a>(pairs: impl Iterator<Item = (&'a str, u64)>) -> u64 {
    use std::hash::{Hash, Hasher};
    pairs.fold(0u64, |sum, (id, epoch)| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut hasher);
        epoch.hash(&mut hasher);
        sum.wrapping_add(hasher.finish())
    })
}

/// One panel's cached series bodies: every fill and every line, marker,
/// bar, field, contour and vector body of the panel's series, drawn once into
/// a transparent panel-sized texture. `prepare` draws it again only when its
/// [`ResidentDisplayKey`] changes; `paint_prepared` blits it between the grid
/// and the live picked highlights, so a repaint costs one textured quad
/// instead of every point again.
///
/// The cache holds the data layer only. Grid, decoration, annotation layers
/// and the picked and selected highlights keep their own textures or draw
/// live, and the key reads nothing they depend on, so editing or selecting
/// any of them never draws the bodies again. Each panel of a layered document
/// has its own entry, so editing one layer leaves every other layer's bodies.
///
/// Only Precise panels qualify: their pipelines blend premultiplied over,
/// which composites the same through a transparent texture (up to UNORM
/// rounding at partly covered pixels, as for the stream display). Styled blend
/// equations (additive, max) depend on what is under them, so those panels
/// keep drawing directly.
struct ResidentDisplay {
    /// The panel view's content-revision cell. Its address names the panel;
    /// once the view and every token holding it are gone, the entry retires.
    view: std::sync::Weak<std::sync::atomic::AtomicU64>,
    key: ResidentDisplayKey,
    serial: u64,
    format: wgpu::TextureFormat,
    sample_count: u32,
    /// Resolved, panel-sized composite the blit samples.
    texture: TrackedTexture,
    /// Multisampled target resolved into `texture`; `None` at 1x.
    msaa: Option<TrackedTexture>,
    bind_group: wgpu::BindGroup,
}

/// Everything the series bodies are drawn from, and nothing else: the
/// data-to-pixel transform, the parts of the chart configuration the body
/// pipelines read, each series' configuration and style scale, the font
/// generation (contour labels), the allocation epochs of exactly the columns
/// the panel reads, and the pool layout and pipeline generations. A column
/// another panel reads never draws this one again.
/// Titles, legend, axes, grid, annotations and selections are not in it.
struct ResidentDisplayKey {
    size: (u32, u32),
    /// The data area relative to the panel.
    data_area: Rect,
    transform: data_render::ScatterTransform,
    draw_style: crate::config::DrawStyle,
    polar: Option<crate::polar::PolarOptions>,
    colorbar: Option<crate::config::ColorBarOptions>,
    /// Each series' configuration and the display scale its style was built
    /// at: a style is a pure function of the two, so a host or the registry
    /// rebuilding every style after one series edit draws only the panels
    /// whose series changed.
    series: Vec<(SeriesConfig, u32)>,
    font_generation: u64,
    columns: u64,
    pool_layout_generation: u64,
    target_pipeline_generation: u64,
}

/// The current frame's inputs to compare against a [`ResidentDisplayKey`]
/// without copying them.
struct ResidentDisplayProbe<'a> {
    size: (u32, u32),
    data_area: Rect,
    transform: data_render::ScatterTransform,
    chart_config: &'a Config,
    series: &'a [Series<'a>],
    font_generation: u64,
    columns: u64,
    pool_layout_generation: u64,
    target_pipeline_generation: u64,
}

impl ResidentDisplayKey {
    fn matches(&self, probe: &ResidentDisplayProbe<'_>) -> bool {
        self.size == probe.size
            && self.data_area == probe.data_area
            && bytemuck::bytes_of(&self.transform) == bytemuck::bytes_of(&probe.transform)
            && self.font_generation == probe.font_generation
            && self.columns == probe.columns
            && self.pool_layout_generation == probe.pool_layout_generation
            && self.target_pipeline_generation == probe.target_pipeline_generation
            && self.draw_style == probe.chart_config.draw_style
            && self.polar == probe.chart_config.polar
            && self.colorbar == probe.chart_config.colorbar
            && self.series.len() == probe.series.len()
            && self
                .series
                .iter()
                .zip(probe.series)
                .all(|((config, scale), now)| {
                    config == now.config && *scale == now.style.display_scale.to_bits()
                })
    }

    fn from_probe(probe: &ResidentDisplayProbe<'_>) -> Self {
        Self {
            size: probe.size,
            data_area: probe.data_area,
            transform: probe.transform,
            draw_style: probe.chart_config.draw_style,
            polar: probe.chart_config.polar.clone(),
            colorbar: probe.chart_config.colorbar.clone(),
            series: probe
                .series
                .iter()
                .map(|series| (series.config.clone(), series.style.display_scale.to_bits()))
                .collect(),
            font_generation: probe.font_generation,
            columns: probe.columns,
            pool_layout_generation: probe.pool_layout_generation,
            target_pipeline_generation: probe.target_pipeline_generation,
        }
    }
}

/// A token's hold on one [`ResidentDisplay`]: the bind group to blit and the
/// serial it was drawn at, so a later re-draw makes an older token stale.
struct ResidentDisplayPacket {
    /// Address of the view's content-revision cell, the entry's identity.
    view: usize,
    serial: u64,
    bind_group: wgpu::BindGroup,
    _charge: crate::gpu_memory::SharedCharge,
}

/// Token returned by [`Renderer::prepare`] and consumed (by reference,
/// repeatably) by [`Renderer::paint_prepared`].
///
/// Owned and `'static`: it captures the resolved pipelines, bind groups,
/// buffers, column handles, panel geometry, and prepare-only GPU products.
/// Hosts pass only this token to the immutable paint phase.
#[derive(Debug)]
pub struct PreparedFrame {
    /// Prevents a token from being recorded through a different renderer,
    /// even when both renderers happen to have matching pool generations.
    renderer_identity: u64,
    items: Vec<PreparedItem>,
    column_sources: HashMap<ColumnId, u64>,
    registered_charts: Vec<PreparedChartStamp>,
    /// Pool layout stamp — defrag, clear, or a buffer-relocating upsert can
    /// move column bytes or replace the backing buffer. Per-column allocation
    /// epochs above catch same-layout replacement of any captured draw input
    /// without invalidating a token for an unrelated column mutation.
    pool_layout_generation: u64,
    /// Monotonic identity of the target pipeline set. This detects format
    /// ABA and sample-count changes, not just a different current format.
    target_pipeline_generation: u64,
    /// One receipt per field or contour series: the grid columns it read and the
    /// epochs they had. `column_sources` cannot serve this — a matrix declares
    /// thousands of columns, and an owned map entry each would put a per-frame
    /// allocation per column on the prepare path.
    grid_sources: Vec<Arc<GridSources>>,
}

#[derive(Debug)]
struct PreparedChartStamp {
    chart_id: ChartId,
    desired: RenderRevision,
    data: RenderRevision,
}

impl std::fmt::Debug for PreparedItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedItem")
            .field("series", &self.series.len())
            .finish_non_exhaustive()
    }
}

impl PreparedSeries {
    fn from_layers(
        layers: data_render::SeriesLayers<'_>,
        arc_charge: Option<crate::gpu_memory::SharedCharge>,
        style_charge: Option<crate::gpu_memory::SharedCharge>,
    ) -> Self {
        Self {
            _arc_charge: arc_charge,
            _column_charge: None,
            _style_charge: style_charge,
            _stream_style_charge: None,
            _stream_transform_charge: None,
            area: layers.area.map(|layer| PreparedAreaLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg.clone(),
                pool_buffer: layer.pool_buffer.clone(),
                x: layer.x,
                y: layer.y,
                base: layer.base,
                verts_per_instance: layer.verts_per_instance,
            }),
            field: layers.field.map(|layer| PreparedFieldLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg,
                field_bg: layer.field_bg,
                charge: layer.charge,
                drawable: layer.drawable,
            }),
            bar: layers.bar.map(|layer| PreparedBarLayer {
                envelope: layer.envelope,
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg.clone(),
                style_map_bg: layer.style_map_bg.cloned(),
                pool_buffer: layer.pool_buffer.clone(),
                edges: layer.edges,
                values: layer.values,
            }),
            vector: layers.vector.map(|layer| PreparedVectorLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg,
                vector_bg: layer.vector_bg,
                charge: layer.charge,
                count: layer.count,
            }),
            contour: layers.contour.map(|layer| PreparedContourLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg,
                field_bg: layer.field_bg,
                charge: layer.charge,
                drawable: layer.drawable,
                label: layer.label.map(|label| PreparedContourLabel {
                    pipeline: label.pipeline.clone(),
                    snapshot: label.snapshot,
                }),
            }),
            errorbar: layers.errorbar.map(|layer| PreparedErrorBarLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg.clone(),
                style_map_bg: layer.style_map_bg.cloned(),
                pool_buffer: layer.pool_buffer.clone(),
                x: layer.x,
                y: layer.y,
                err_y_lo: layer.err_y_lo,
                err_y_hi: layer.err_y_hi,
                err_x_lo: layer.err_x_lo,
                err_x_hi: layer.err_x_hi,
                style_index: layer.style_index,
            }),
            line: layers.line.map(|layer| PreparedLineLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg.clone(),
                pool_buffer: layer.pool_buffer.clone(),
                x: layer.x,
                y: layer.y,
                arc: layer.arc,
                verts_per_instance: layer.verts_per_instance,
                texture_bg: layer.texture_bg.cloned(),
            }),
            line_extra: layers.line_extra.map(|layer| PreparedStarLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg.clone(),
                texture_bg: layer.texture_bg.clone(),
                star_bg: layer.star_bg.clone(),
                indirect: layer.indirect.clone(),
            }),
            scatter: layers.scatter.map(|layer| PreparedScatterLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg.clone(),
                style_map_bg: layer.style_map_bg.cloned(),
                value_map_bg: layer.value_map_bg.cloned(),
                quad_vb: layer.quad_vb.clone(),
                pool_buffer: layer.pool_buffer.clone(),
                x: layer.x,
                y: layer.y,
                style_index: layer.style_index,
                color_value: layer.color_value,
                size_value: layer.size_value,
                texture_bg: layer.texture_bg.cloned(),
            }),
            selected_bars: layers
                .selected_bars
                .into_iter()
                .map(|layer| PreparedBarSelectionLayer {
                    pipeline: layer.pipeline.clone(),
                    transform_bg: layer.transform_bg.clone(),
                    style_bg: layer.style_bg,
                    selection_bg: layer.selection_bg,
                    selection_charge: layer.selection_charge,
                    pool_buffer: layer.pool_buffer.clone(),
                    edges: layer.edges,
                    values: layer.values,
                    instance: layer.instance,
                })
                .collect(),
            selected_fields: layers
                .selected_fields
                .into_iter()
                .map(|layer| PreparedFieldSelectionLayer {
                    pipeline: layer.pipeline.clone(),
                    transform_bg: layer.transform_bg.clone(),
                    selection_bg: layer.selection_bg,
                    selection_charge: layer.selection_charge,
                    field_bg: layer.field_bg,
                    charge: layer.charge,
                    drawable: layer.drawable,
                })
                .collect(),
            picked: layers
                .picked
                .into_iter()
                .map(|layer| PreparedPickRingLayer {
                    pipeline: layer.pipeline.clone(),
                    transform_bg: layer.transform_bg.clone(),
                    style_bg: layer.style_bg,
                    style_map_bg: layer.style_map_bg.cloned(),
                    value_map_bg: layer.value_map_bg.cloned(),
                    quad_vb: layer.quad_vb.clone(),
                    pool_buffer: layer.pool_buffer.clone(),
                    x: layer.x,
                    y: layer.y,
                    style_index: layer.style_index,
                    color_value: layer.color_value,
                    size_value: layer.size_value,
                    instance: layer.instance,
                })
                .collect(),
        }
    }

    fn layers(&self) -> data_render::SeriesLayers<'_> {
        data_render::SeriesLayers {
            area: self
                .area
                .as_ref()
                .map(|layer| data_render::ColumnAreaLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: &layer.style_bg,
                    pool_buffer: &layer.pool_buffer,
                    x: layer.x,
                    y: layer.y,
                    base: layer.base,
                    verts_per_instance: layer.verts_per_instance,
                }),
            field: self
                .field
                .as_ref()
                .map(|layer| data_render::ColumnFieldLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    field_bg: layer.field_bg.clone(),
                    charge: Arc::clone(&layer.charge),
                    drawable: layer.drawable,
                }),
            bar: self.bar.as_ref().map(|layer| data_render::ColumnBarLayer {
                envelope: layer.envelope.clone(),
                pipeline: &layer.pipeline,
                transform_bg: &layer.transform_bg,
                style_bg: &layer.style_bg,
                style_map_bg: layer.style_map_bg.as_ref(),
                pool_buffer: &layer.pool_buffer,
                edges: layer.edges,
                values: layer.values,
            }),
            vector: self
                .vector
                .as_ref()
                .map(|layer| data_render::vector::ColumnVectorLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    vector_bg: layer.vector_bg.clone(),
                    charge: Arc::clone(&layer.charge),
                    count: layer.count,
                }),
            contour: self
                .contour
                .as_ref()
                .map(|layer| data_render::ColumnContourLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    field_bg: layer.field_bg.clone(),
                    charge: Arc::clone(&layer.charge),
                    drawable: layer.drawable,
                    label: layer
                        .label
                        .as_ref()
                        .map(|label| data_render::ColumnContourLabel {
                            pipeline: &label.pipeline,
                            snapshot: label.snapshot.clone(),
                        }),
                }),
            errorbar: self.errorbar.as_ref().map(|layer| ColumnErrorBarDraw {
                pipeline: &layer.pipeline,
                transform_bg: &layer.transform_bg,
                style_bg: &layer.style_bg,
                style_map_bg: layer.style_map_bg.as_ref(),
                pool_buffer: &layer.pool_buffer,
                x: layer.x,
                y: layer.y,
                err_y_lo: layer.err_y_lo,
                err_y_hi: layer.err_y_hi,
                err_x_lo: layer.err_x_lo,
                err_x_hi: layer.err_x_hi,
                style_index: layer.style_index,
            }),
            line: self.line.as_ref().map(|layer| ColumnLineLayer {
                pipeline: &layer.pipeline,
                transform_bg: &layer.transform_bg,
                style_bg: &layer.style_bg,
                pool_buffer: &layer.pool_buffer,
                x: layer.x,
                y: layer.y,
                arc: layer.arc.clone(),
                verts_per_instance: layer.verts_per_instance,
                texture_bg: layer.texture_bg.as_ref(),
            }),
            line_extra: self
                .line_extra
                .as_ref()
                .map(|layer| data_render::ColumnStarLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: &layer.style_bg,
                    texture_bg: &layer.texture_bg,
                    star_bg: &layer.star_bg,
                    indirect: &layer.indirect,
                }),
            scatter: self.scatter.as_ref().map(|layer| ColumnScatterLayer {
                pipeline: &layer.pipeline,
                transform_bg: &layer.transform_bg,
                style_bg: &layer.style_bg,
                style_map_bg: layer.style_map_bg.as_ref(),
                value_map_bg: layer.value_map_bg.as_ref(),
                quad_vb: &layer.quad_vb,
                pool_buffer: &layer.pool_buffer,
                x: layer.x,
                y: layer.y,
                style_index: layer.style_index,
                color_value: layer.color_value,
                size_value: layer.size_value,
                texture_bg: layer.texture_bg.as_ref(),
            }),
            selected_bars: self
                .selected_bars
                .iter()
                .map(|layer| ColumnBarSelectionLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    selection_bg: layer.selection_bg.clone(),
                    selection_charge: Arc::clone(&layer.selection_charge),
                    pool_buffer: &layer.pool_buffer,
                    edges: layer.edges,
                    values: layer.values,
                    instance: layer.instance,
                })
                .collect(),
            selected_fields: self
                .selected_fields
                .iter()
                .map(|layer| ColumnFieldSelectionLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    selection_bg: layer.selection_bg.clone(),
                    selection_charge: Arc::clone(&layer.selection_charge),
                    field_bg: layer.field_bg.clone(),
                    charge: Arc::clone(&layer.charge),
                    drawable: layer.drawable,
                })
                .collect(),
            picked: self
                .picked
                .iter()
                .map(|layer| ColumnPickRingLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    style_map_bg: layer.style_map_bg.as_ref(),
                    value_map_bg: layer.value_map_bg.as_ref(),
                    quad_vb: &layer.quad_vb,
                    pool_buffer: &layer.pool_buffer,
                    x: layer.x,
                    y: layer.y,
                    style_index: layer.style_index,
                    color_value: layer.color_value,
                    size_value: layer.size_value,
                    instance: layer.instance,
                })
                .collect(),
        }
    }
}

/// A resident-to-streamed authority change that is deliberately withheld
/// until its replacement stream has rendered the complete chart closure.
/// While this value is live the resident pool and chart registry remain the
/// only published source of truth.
pub(super) struct ResidentStreamHandoff {
    pub(super) columns: Vec<crate::StreamColumn>,
    pub(super) allocation_epochs: Vec<(ColumnId, u64)>,
    pub(super) expected_stream_revisions: Vec<(ColumnId, u64)>,
    pub(super) expected_chart_revisions: Vec<(ChartId, RenderRevision, RenderRevision)>,
}

/// Cartesian renderer owning its column pool, views' shared resources and pipeline caches.
pub struct Renderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    caps: RendererDeviceCaps,
    pool: ColumnPool,
    // Disjoint from pool ids: host metadata only, never payloads or copied slots.
    streaming_sources: HashMap<ColumnId, RegisteredStreamColumn>,
    // Logical host revision authority survives streamed -> resident promotion.
    // Only a true logical-column removal clears an entry.
    stream_revision_lineage: HashMap<ColumnId, u64>,
    next_stream_source_fit_epoch: u64,
    stream_runtime: Option<streaming_runtime::StreamRuntime>,
    chart_states: HashMap<ChartId, ChartRenderState>,
    chart_order: Vec<ChartId>,
    next_chart_id: u64,
    visual_revision: RenderRevision,
    renderer_identity: u64,
    observed_font_generation: u64,
    pending_defrag: bool,
    picker: RendererPicker,
    errorbar_extent_engine: Option<crate::gpu_errorbar::GpuErrorbarExtentEngine>,
    target_sample_count: u32,
    target_pipeline_generation: u64,

    // Bind group layouts (exposed so callers can build per-panel bind groups).
    texture_bgl: wgpu::BindGroupLayout,
    transform_bgl: wgpu::BindGroupLayout,
    style_bgl: wgpu::BindGroupLayout,
    per_point_style_map_bgl: wgpu::BindGroupLayout,
    /// Group 3 of the mapped scatter pipelines: a scatter series' continuous
    /// value maps (`point_color_map` / `point_size_map`).
    point_value_map_bgl: wgpu::BindGroupLayout,
    /// The value-map group bound for a mapped scatter series that has no value
    /// map: every flag clear, so the shader reads nothing from it.
    neutral_point_value_map_bg: wgpu::BindGroup,
    _neutral_point_value_map_charge: crate::gpu_memory::SharedCharge,
    data_selection_bgl: wgpu::BindGroupLayout,
    /// Milkyway star-pass data layout (arc prefix + pool + offsets) —
    /// shared by the stars pipeline and every per-series star bind group.
    star_data_bgl: wgpu::BindGroupLayout,
    /// Field data layout (pool + grid table + levels + colourmap stops + lookup
    /// metadata + `FieldParams`) — shared by the field pipeline and every
    /// per-series field bind group.
    field_bgl: wgpu::BindGroupLayout,
    /// Vector data layout (pool + grid table + colourmap stops +
    /// `VectorParams`), group 2 of the assembled vector pipelines.
    vector_bgl: wgpu::BindGroupLayout,
    /// The padding tables every grid vector's cell-geometry field group shares.
    field_cell_tables: data_render::FieldCellTables,

    // Pipelines (precise + lazily-cached styled variants), all against
    // `surface_format`.
    pipelines: TargetPipelines,

    // Shared resources.
    sampler: wgpu::Sampler,
    /// Charged once per renderer; 32 bytes, but the gate counts objects too.
    quad_vb: TrackedBuffer,

    /// Per-series immutable GPU arc results, keyed by every input the scan or
    /// star-count kernel reads. A cache miss allocates and dispatches a new
    /// result; an old result is never rewritten, even after its token drops,
    /// because a host-owned command buffer may still reference it before a late
    /// submission. Mutated only in
    /// `&mut self` prepare-phase entry points (`prepare`/`paint`/export);
    /// the `&self` draw phase reads the token's owned snapshot, never this
    /// cache.
    arc_cache: HashMap<String, Vec<ArcResultEntry>>,
    /// One cached composite per qualifying resident panel; see
    /// [`ResidentDisplay`].
    resident_displays: Vec<ResidentDisplay>,
    /// Serial of the last composite drawn into any [`ResidentDisplay`].
    resident_display_serial: u64,
    /// Anchor compute pipelines plus the label draw's module and layouts.
    /// Compiled on the first frame that asks for a contour label; a chart with
    /// none never pays for the shader.
    contour_label_pipelines: Option<crate::gpu_contour::ContourLabelPipelines>,
    /// Per-contour-series group-2 state, keyed by `series_id`. Since design
    /// B.4.9 this holds the same tables a field's scratch does — the isolines are
    /// drawn from the grid by `fs_contour`, so there are no traced segments to
    /// cache — plus the immutable label atlas and its placement-slot pool when
    /// the series has labels.
    contour_cache: HashMap<String, ContourScratchEntry>,
    /// Per-field-series group-2 state, keyed by `series_id`. Rebuilt only when
    /// its [`FieldSignature`] changes; a live token holds its own refcounted
    /// clone of the bind group, so replacement disturbs nothing it can draw.
    field_cache: HashMap<String, FieldScratch>,
    /// Per-scatter-series value-map state (group 3), keyed by `series_id`.
    /// Rebuilt only when its [`ValueMapSignature`] changes; a live token holds
    /// its own refcounted clone of the bind group, like `field_cache`.
    value_map_cache: HashMap<String, ValueMapScratch>,
    /// Per-vector-series group-2 state, keyed by `series_id`. Rebuilt only
    /// when its [`VectorSignature`] changes, like `field_cache`.
    vector_cache: HashMap<String, VectorScratch>,
    arc_pipelines: Option<data_render::line_arc::ArcScanPipelines>,
    /// Test-only narrowing of the arc-scan chunk size so the multi-chunk
    /// carry path is exercisable with small `n`. Always `None` in release.
    #[cfg(test)]
    arc_chunk_override: Option<u32>,

    /// Constellation deep-space backdrop cache. The backdrop depends only on
    /// the panel size and the (nebula, dust, seed) options — NOT on axis
    /// ranges — so pan/zoom/`set_config` refreshes reuse the baked bytes
    /// instead of re-running the fBm lattice over the whole panel (~40 ms at
    /// 2000×1600). Single slot: multi-panel hosts with differing sizes fall
    /// back to re-baking per refresh, same as before the cache. Mutated only
    /// in `&mut self` entry points, like `arc_cache`.
    space_bg: Option<SpaceBgCache>,

    surface_format: wgpu::TextureFormat,

    /// Byte ledger for tracked GPU allocations outside the column pool. The
    /// pool reports its own bytes separately. This is not the whole device
    /// footprint; see [`Self::gpu_memory_usage`] for exclusions.
    ///
    /// Shared by `Arc` because final-owner `Drop` can retire resources from
    /// another thread, and `WindowedRenderer` owns its tracked MSAA target
    /// outside this struct. Credits return after submission completion.
    gpu_ledger: Arc<GpuLedger>,

    /// Host allocation policy for budget-checked paths, pool growth included.
    ///
    /// `None` (the default) means device limits are the only bound, which is
    /// the 0.9 behavior. When set, it rides into every pool allocation as
    /// `GpuAllocCtx::budget` together with the out-of-pool total read at that
    /// moment, so the pool refuses a growth that would cross the ceiling
    /// instead of discovering it as a device error.
    memory_budget: Option<u64>,

    /// Host policy for renderer-selected streamed -> resident transitions.
    /// `None` keeps native callers in explicit-control mode. This is separate
    /// from `memory_budget`: the latter governs checked allocation paths, while
    /// this cap decides whether one complete chart-column closure belongs in
    /// the resident representation.
    auto_resident_working_set_limit: Option<u64>,
}

/// Key + bytes of one baked constellation backdrop. `bake_gen` is a
/// monotonically increasing stamp; a `ChartView` whose grid texture already
/// holds this generation skips the (w·h·4)-byte re-upload too.
struct SpaceBgCache {
    key: SpaceBgKey,
    bake_gen: u64,
    rgba: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SpaceBgKey {
    w: u32,
    h: u32,
    nebula: u32,
    dust: u32,
    seed: u32,
}

// One descriptor fully defines each stylized render mode. The precise path is
// represented by `style_variant() == None` and never routes through this table.

fn picked_ref_matches_series(series: &SeriesConfig, picked: &PickedPointRef) -> bool {
    if picked.series_id != series.series_id {
        return false;
    }
    match picked.source_id.as_deref() {
        Some(source_id) => series.source_id.as_deref() == Some(source_id),
        None => true,
    }
}

fn picked_data_ref_matches_series(series: &SeriesConfig, picked: &PickedDataRef) -> bool {
    if picked.series_id() != series.series_id {
        return false;
    }
    match picked.source_id() {
        Some(source_id) => series.source_id.as_deref() == Some(source_id),
        None => true,
    }
}

/// Whether the line `series` draws under `draw_style` walks its arc prefix:
/// a dashed stroke takes its dash phase from it and a sketch stroke its
/// wobble. A Milkyway ribbon is never dashed and its stars are picked by
/// their envelope, so it needs none.
fn line_pick_walks_arc(draw_style: &DrawStyle, series: &SeriesConfig) -> bool {
    if !effective_series_primitives(draw_style, &series.render_type).line {
        return false;
    }
    let Some(line) = extract_line(&series.render_type) else {
        return false;
    };
    match draw_style {
        DrawStyle::Milkyway(_) => false,
        DrawStyle::Sketch(_) => true,
        DrawStyle::Precise | DrawStyle::Constellation(_) => {
            !matches!(line.line_style, LineStylePreset::Solid)
        }
    }
}

fn scatter_config_radius_px(
    scatter: &DataScatterStyleConfig,
    point_index: usize,
    use_style_mapping: bool,
) -> f32 {
    let mut radius = scatter.point_size;
    if use_style_mapping && scatter.point_style_index_column.is_none() {
        // A style table has no effect without an index column. Sparse
        // overrides are explicit by point index and can be resolved here.
        if let Some(overrides) = scatter.point_style_overrides.as_deref() {
            for ov in overrides {
                if ov.index == point_index
                    && let Some(size) = ov.style.point_size
                {
                    radius = size;
                }
            }
        }
    }
    radius.max(0.0)
}

impl Renderer {
    // Handle / accessor methods.

    // Column management (returns Result).

    // Standalone path — figgy owns instance / adapter / device / queue / surface.
    //
    // For hosts where figgy renders the window directly (e.g. winit). Caller
    // writes zero lines of wgpu setup code.

    /// Build base Cartesian resources and a surface from a window handle.
    /// The returned [`WindowedRenderer`] retains the instance, adapter and GPU
    /// handles, manages the surface and
    /// expects `draw` to be called once per frame.
    ///
    /// `target` can be anything convertible to `wgpu::SurfaceTarget`
    /// (`Arc<winit::Window>`, a raw-handle wrapper, a web canvas, …). The
    /// figgy crate does not depend on winit types itself. On wasm, drive this
    /// from the host event loop (e.g. `wasm_bindgen_futures::spawn_local`).
    ///
    /// In a browser this also awaits temporary JS render-pipeline warmup; those
    /// pipelines are discarded and do not populate the renderer's optional
    /// caches. Native construction has no JS warmup. Both leave resident optional
    /// caches lazy; use [`Self::prewarm_all`] or detached preparation explicitly.
    pub async fn for_window_async<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
    ) -> Result<WindowedRenderer<'w>> {
        let mut observer = |_| {};
        Self::for_window_async_observed(target, size, pool_capacity_bytes, &mut observer).await
    }

    /// Build a standalone renderer as in [`Self::for_window_async`], reporting
    /// timestamp-free startup stage boundaries to `observer`.
    // Keep the Arc-based RendererDevice API identical across targets. wgpu's
    // WebGPU handles are intentionally !Send/!Sync on the wasm main thread.
    #[cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]
    pub async fn for_window_async_observed<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<WindowedRenderer<'w>> {
        Self::for_window_prepared_async(target, size, pool_capacity_bytes, false, observer).await
    }

    /// Limit temporary browser render warmup to Basic entries during window setup.
    ///
    /// Native initialization is the same as [`Self::for_window_async_observed`].
    /// This does not install the retained Basic preparation group. A host gating
    /// first use must compile and install Basic and its other required groups via
    /// [`Self::begin_preparation`], or call [`Self::prewarm_all`]. Normal frame
    /// preparation can also create missing pipelines lazily.
    pub async fn for_window_minimal_async_observed<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<WindowedRenderer<'w>> {
        Self::for_window_prepared_async(target, size, pool_capacity_bytes, true, observer).await
    }

    async fn for_window_prepared_async<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
        minimal: bool,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<WindowedRenderer<'w>> {
        let instance = observe_value(observer, "window", "instance", data_render::create_instance);
        let surface = observe_result(observer, "window", "surface", || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                data_render::create_surface_for_window(&instance, target)
            }))
            .map_err(|_| FiggyError::SurfaceCreationFailed {
                reason:
                    "wgpu surface creation panicked; check platform window/canvas/thread constraints"
                        .into(),
            })?
            .map_err(|e| FiggyError::SurfaceCreationFailed {
                reason: format!("{e}"),
            })
        })?;

        started(observer, "window", "adapter");
        let adapter = data_render::request_adapter_for_surface_async(&instance, &surface)
            .await
            .map_err(|_| FiggyError::AdapterUnavailable)?;
        finished(observer, "window", "adapter");

        started(observer, "window", "device");
        let (device, queue) = data_render::request_device_async(&adapter)
            .await
            .map_err(|e| FiggyError::DeviceCreationFailed {
                reason: format!("{e}"),
            })?;
        finished(observer, "window", "device");
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let surface_config = observe_result(observer, "window", "configure", || {
            data_render::try_configure_surface(
                &surface,
                &adapter,
                &device,
                size.0.max(1),
                size.1.max(1),
            )
        })?;

        let caps = RendererDeviceCaps::from_device(&device);
        // Probing MSAA decides the sample count the renderer's pipelines are
        // compiled against, so it has to happen before the renderer exists.
        // The ledger is therefore created here and adopted by the renderer:
        // one ledger per device, this target included.
        // host-alloc: W1-a
        let gpu_ledger = Arc::new(GpuLedger::new());
        let preferred_sample_count = preferred_msaa_sample_count(caps, surface_config.format);
        let (target_sample_count, msaa_target) =
            observe_result(observer, "window", "figgy frame msaa target", || {
                match create_msaa_target(
                    &device,
                    &gpu_ledger,
                    caps,
                    "figgy frame msaa target",
                    surface_config.width,
                    surface_config.height,
                    surface_config.format,
                    preferred_sample_count,
                ) {
                    Ok(target) => Ok((preferred_sample_count, target)),
                    Err(_) if preferred_sample_count > 1 => Ok((1, None)),
                    Err(e) => Err(e),
                }
            })?;

        let inner = Renderer::try_new_with_sample_count_observed_async(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            surface_config.format,
            pool_capacity_bytes,
            target_sample_count,
            Arc::clone(&gpu_ledger),
            minimal,
            observer,
        )
        .await?;

        Ok(WindowedRenderer {
            inner,
            surface,
            surface_config,
            msaa_target,
            _instance: instance,
            _adapter: adapter,
        })
    }

    /// Blocking convenience wrapper around [`Self::for_window_async`]. Native
    /// only — on wasm, blocking the single thread would deadlock.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn for_window<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
    ) -> Result<WindowedRenderer<'w>> {
        pollster::block_on(Self::for_window_async(target, size, pool_capacity_bytes))
    }

    // Host-integration API (egui_wgpu / iced_wgpu / standalone).
    //
    // `Renderer` does NOT own the surface, swap chain, or event loop — the
    // host (egui, iced, a wgpu app) manages those. The frame is split to
    // match host callback shapes: every mutation lives in `prepare`
    // (`&mut self`), recording lives in `paint_prepared` (`&self`).
    // Per-frame flow:
    //   1. Host prepare stage (`&mut` access is available): data uploads
    //      (`add_column`), `ensure_target_format` on format change, and
    //      `refresh_axis` after consuming `chart.consume_raster_dirty()`.
    //      (`update_transform` is no longer needed per frame — `prepare`
    //      writes the transform uniform itself.)
    //   2. `let prepared = renderer.prepare(&items)?` — compiles missing
    //      styled pipelines, writes per-panel transforms, dispatches arc
    //      scans, returns an owned `'static` token. Store it wherever the
    //      host carries state into its paint callback (egui
    //      `CallbackResources`, iced `Storage`, or a local for winit).
    //   3. Host paint stage (only `&self` is available):
    //      `renderer.paint_prepared(pass, target_size, &prepared)`
    //      — pure recording, repeatable, no lock required. Returns
    //      `FiggyError::StalePreparedFrame` if an invalidating `&mut` call
    //      interleaved; recover by re-preparing next frame.
    //   4. After submitting every panel command buffer, call
    //      `renderer.end_gpu_frame()` once. Never call it per-panel during the
    //      prepare sweep: later panels can still own unsubmitted work.
    //
    // No `Mutex` is needed for the paint callback: `Renderer` is Send + Sync
    // and `paint_prepared` never mutates. Hosts that own the renderer
    // exclusively during their frame (winit loop, wasm wrapper) can keep
    // calling the one-shot `paint(&mut self, …)` facade, which runs both
    // phases back to back.
    //
    // egui pseudocode (`CallbackTrait`). Items are built by a FREE function
    // over the state's non-renderer fields so `s.renderer` stays free for
    // the `&mut` prepare call (disjoint field borrows) — a `&self` method
    // returning items would keep all of `*s` borrowed and fail to compile.
    // The token lives in a state field, crossing the callback boundary:
    // ```ignore
    // fn build_items<'a>(view: &'a ChartView, chart: &'a Chart, series: &'a [Series<'a>])
    //     -> Vec<ChartDrawItem<'a>> { /* prepare-only input */ }
    //
    // fn prepare(&self, _dev, _queue, _enc, res: &mut CallbackResources) -> Vec<CommandBuffer> {
    //     let s = res.get_mut::<FiggyState>().unwrap();
    //     if s.chart.consume_raster_dirty() {
    //         s.renderer.refresh_axis(&mut s.view, &s.chart, rect).unwrap();
    //     }
    //     let items = build_items(&s.view, &s.chart, &s.series);
    //     s.prepared = s.renderer.prepare(&items).ok(); // token into a state field
    //     vec![]
    // }
    // fn paint(&self, info, pass, res: &CallbackResources) {
    //     let s = res.get::<FiggyState>().unwrap();
    //     if let Some(prepared) = &s.prepared {
    //         let _ = s.renderer.paint_prepared(pass, target_size, prepared);
    //     }
    // }
    // ```
    // (One figgy widget per egui callback: with several callbacks sharing a
    // renderer, store one token per widget/panel — egui runs every
    // callback's prepare before any paint, so a single shared token would
    // be overwritten before the first paint. See examples/egui_embed.rs.)

    /// Build the per-panel GPU resources: grid + decoration textures and
    /// bind groups, plus the transform uniform buffer + bind group.
    /// `panel_rect` is the panel's pixel rect in surface coordinates.
    /// `chart.config_mut().chart_area` must match `panel_rect` before this
    /// call so the axis raster is drawn correctly.
    pub fn create_chart_view(&self, chart: &Chart, panel_rect: Rect) -> Result<ChartView> {
        let panels = expand_for_render(chart.config())?;
        let places = panel_places(chart.config(), &panels, panel_rect);
        let font_generation = crate::text_render::font_generation();
        let mut view = self.create_panel_view(&panels[0], panel_rect, false, font_generation)?;
        for (panel, place) in panels[1..].iter().zip(&places[1..]) {
            view.overlay_views.push(self.create_panel_view(
                panel,
                place.rect,
                !place.draws_backdrop,
                font_generation,
            )?);
        }
        // Annotation layer: two optional textures from one shared raster
        // pass. One layer per document, on the base view, anchored in the
        // parent's own axis space (the group data area).
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);
        let annotations = annotation_raster_for_chart(chart, None);
        view.annotation_underlay = annotations
            .underlay
            .as_deref()
            .map(|rgba| self.create_layer_texture(rgba, w, h))
            .transpose()?;
        view.annotation_overlay = annotations
            .overlay
            .as_deref()
            .map(|rgba| self.create_layer_texture(rgba, w, h))
            .transpose()?;
        view.annotation_map = annotations.map;
        view.tick_warnings = merged_render_warnings(chart.config(), &panels);
        view.rebuild_render_warnings();
        Ok(view)
    }

    /// One panel's grid, decoration and transform resources from one
    /// internal config. `overlay` panels never draw the Milkyway backdrop:
    /// the grid slot belongs to the first panel of their area. No panel carries annotation
    /// textures here; `create_chart_view` gives them to the base view.
    fn create_panel_view(
        &self,
        config: &Config,
        panel_rect: Rect,
        overlay: bool,
        font_generation: u64,
    ) -> Result<ChartView> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);

        // Grid layer (drawn below data): raster + texture + bind group.
        let grid_rgba = raster_grid_layer(config, overlay, w, h)?;
        let grid = self.create_layer_texture(&grid_rgba, w, h)?;

        // Decoration layer (drawn above data).
        let dec_rgba = axis_render::try_raster_chart_layer_to_rgba(
            config,
            axis_render::AxisLayerKind::Decoration,
        )?;
        let decoration = self.create_layer_texture(&dec_rgba, w, h)?;

        let t = data_render::scatter_transform_from_config(config);
        let transform_buffer = TrackedBuffer::new(
            &self.gpu_ledger,
            GpuResourceKind::Uniform,
            data_render::create_scatter_transform_uniform_buffer(&self.device, &t),
        );
        let transform_bg = data_render::create_scatter_transform_bind_group(
            &self.device,
            &self.transform_bgl,
            &transform_buffer,
        );

        Ok(ChartView {
            grid_texture: grid.texture,
            grid_bind_group: grid.bind_group,
            decoration_texture: decoration.texture,
            decoration_bind_group: decoration.bind_group,
            annotation_underlay: None,
            annotation_overlay: None,
            annotation_map: crate::annotation::AnnotationMap::empty(
                crate::annotation::pixel_space_for_config(config),
            ),
            annotation_selected: None,
            layer_stamps: ChartViewLayerStamps {
                grid: 1,
                decoration: 1,
                annotations: 1,
            },
            transform_buffer,
            transform_bg,
            content_revision: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            stream_revision: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            panel_rect,
            grid_space_gen: None,
            tick_warnings: axis_render::tick_warnings(config),
            render_warnings: Vec::new(),
            overlay_views: Vec::new(),
            raster_key: Some(PanelRasterKey::new(
                config,
                &[],
                font_generation,
                panel_rect,
            )),
            raster_generation: 1,
        })
    }

    /// Upload one panel-sized raster as a texture with its bind group.
    fn create_layer_texture(
        &self,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> Result<crate::annotation::LayerTexture> {
        let texture = TrackedTexture::new(
            &self.gpu_ledger,
            GpuResourceKind::PanelTexture,
            data_render::upload_rgba_texture(
                &self.device,
                &self.queue,
                self.caps.max_texture_dimension_2d,
                w,
                h,
                rgba,
            )?,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = data_render::create_texture_bind_group(
            &self.device,
            &self.texture_bgl,
            &view,
            &self.sampler,
        );
        Ok(crate::annotation::LayerTexture {
            texture,
            bind_group,
        })
    }

    /// Re-rasterize only the annotation layer of `view` from `chart`:
    /// the underlay and overlay textures, the annotation hit map and the
    /// annotation warnings. The grid and decoration textures are not
    /// touched, so an annotation edit never rewrites axis labels. `selected`
    /// is the annotation whose selection box the overlay draws; the view
    /// remembers it for the next `refresh_axis*`.
    ///
    /// Call it when [`ChartRenderStamp::needs_annotation_raster_since`] says
    /// the items changed or when the annotation selection changed. A panel
    /// rect that differs from the view's moves the anchor space as well, so
    /// that case refreshes every layer like [`Self::refresh_axis`].
    pub fn refresh_annotations(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<usize>,
    ) -> Result<()> {
        view.annotation_selected = selected;
        if view.panel_rect != panel_rect {
            return self.refresh_axis(view, chart, panel_rect);
        }
        view.advance_content_revision()?;
        self.refresh_annotation_layers(view, chart, panel_rect)
    }

    fn refresh_annotation_layers(
        &self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<()> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);
        let raster = annotation_raster_for_chart(chart, view.annotation_selected);
        self.refresh_optional_layer(
            &mut view.annotation_underlay,
            raster.underlay.as_deref(),
            w,
            h,
        )?;
        self.refresh_optional_layer(
            &mut view.annotation_overlay,
            raster.overlay.as_deref(),
            w,
            h,
        )?;
        view.annotation_map = raster.map;
        view.layer_stamps.annotations += 1;
        view.rebuild_render_warnings();
        Ok(())
    }

    fn refresh_optional_layer(
        &self,
        slot: &mut Option<crate::annotation::LayerTexture>,
        rgba: Option<&[u8]>,
        w: u32,
        h: u32,
    ) -> Result<()> {
        match (slot.as_mut(), rgba) {
            (Some(layer), Some(rgba)) => {
                self.refresh_one_layer(&mut layer.texture, &mut layer.bind_group, rgba, w, h)
            }
            (None, Some(rgba)) => {
                *slot = Some(self.create_layer_texture(rgba, w, h)?);
                Ok(())
            }
            (_, None) => {
                *slot = None;
                Ok(())
            }
        }
    }

    /// Write only the data transform uniform; leave axis/grid/decorations unchanged.
    ///
    /// An axis-range edit normally needs [`Self::refresh_axis`] as well to update
    /// tick labels and the grid. In the prepare/paint flow, refresh those raster
    /// layers and let [`Self::prepare`] write the transform from the same config;
    /// no separate call to this method is needed.
    pub fn update_transform(&mut self, view: &ChartView, chart: &Chart) -> Result<()> {
        let panels = expand_for_render(chart.config())?;
        let views = layer_views(view, panels.len())?;
        view.advance_stream_revision()?;
        view.advance_content_revision()?;
        for (panel, layer_view) in panels.iter().zip(views) {
            let t = data_render::scatter_transform_from_config(panel);
            data_render::update_scatter_transform(&self.queue, &layer_view.transform_buffer, &t);
        }
        Ok(())
    }

    /// Re-rasterize both grid and decoration textures. Updates in place via
    /// `write_texture` when the size matches; allocates new textures
    /// otherwise. The data transform uniform needs no separate call in the
    /// prepare/paint flow — [`Self::prepare`] (and the `paint`/`draw`
    /// facades) writes it every frame; [`Self::update_transform`] remains
    /// for imperative one-off updates outside that flow.
    ///
    /// `&mut self`: the constellation backdrop cache (`space_bg`) lives on
    /// the renderer and may be (re)baked here — same prepare-phase mutability
    /// rule as `paint`.
    pub fn refresh_axis(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<()> {
        self.refresh_axis_with_selection(view, chart, panel_rect, &[])
    }

    /// [`Self::refresh_axis`] plus selection highlight boxes composited into
    /// the decoration layer (selection draws above the data, never below).
    /// `selection` comes from `Selectable::selection_box` / `HitMap` and is in
    /// absolute surface coordinates.
    pub fn refresh_axis_with_selection(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selection: &[crate::select::SelectionBox],
    ) -> Result<()> {
        let panels = expand_for_render(chart.config())?;
        let places = panel_places(chart.config(), &panels, panel_rect);
        let font_generation = crate::text_render::font_generation();
        // The view follows the document's layer count: a layer added since
        // the last refresh gets its textures here, a removed one drops them.
        view.overlay_views.truncate(panels.len() - 1);
        while view.overlay_views.len() < panels.len() - 1 {
            let index = view.overlay_views.len() + 1;
            let place = &places[index];
            let layer = self.create_panel_view(
                &panels[index],
                place.rect,
                !place.draws_backdrop,
                font_generation,
            )?;
            view.overlay_views.push(layer);
        }
        // Each selection box is composited into the topmost panel of the
        // area it sits in, so it is above every layer's decoration there, as
        // it was above the single one.
        let routed = route_selection_boxes(&places, selection);
        for (index, panel) in panels.iter().enumerate() {
            let layer_view = if index == 0 {
                &mut *view
            } else {
                &mut view.overlay_views[index - 1]
            };
            self.refresh_panel_view(
                layer_view,
                panel,
                places[index].rect,
                &routed[index],
                !places[index].draws_backdrop,
                font_generation,
            )?;
        }
        view.panel_rect = panel_rect;
        view.tick_warnings = merged_render_warnings(chart.config(), &panels);
        // The anchor space follows the axes and the data area, which this
        // refresh may have moved, so the annotation layer is redone here as
        // well; its own revision covers item edits between axis refreshes.
        self.refresh_annotation_layers(view, chart, panel_rect)?;
        Ok(())
    }

    /// Refresh one panel's textures from its internal config, unless the
    /// textures already hold exactly this raster.
    fn refresh_panel_view(
        &mut self,
        view: &mut ChartView,
        config: &Config,
        panel_rect: Rect,
        selection: &[crate::select::SelectionBox],
        overlay: bool,
        font_generation: u64,
    ) -> Result<()> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);
        let key = PanelRasterKey::new(config, selection, font_generation, panel_rect);
        if view.raster_key.as_ref() == Some(&key) {
            // Same raster: the revisions still advance so a token prepared
            // before this call is retired, exactly as for a real rewrite.
            // The Milkyway backdrop keeps its own cache and stamp: a view
            // created with a direct grid raster adopts the shared bake here.
            view.advance_content_revision()?;
            if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style
                && !overlay
            {
                self.refresh_space_backdrop(view, config, c, w, h)?;
            }
            return Ok(());
        }

        let dec_rgba = axis_render::try_raster_chart_layer_to_rgba_with_selection(
            config,
            axis_render::AxisLayerKind::Decoration,
            selection,
        )?;
        if view.panel_rect != panel_rect {
            view.advance_stream_revision()?;
        }
        view.advance_content_revision()?;

        // Grid layer. The constellation backdrop is axis-range independent,
        // so it is cached by (size, nebula, dust, seed): a pan/zoom/config
        // commit re-rasters only the decoration layer, and a view whose grid
        // texture already holds the cached generation skips the upload too.
        // Overlay panels never draw the backdrop; the base panel owns it.
        if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style
            && !overlay
        {
            self.refresh_space_backdrop(view, config, c, w, h)?;
        } else {
            let grid_rgba = raster_grid_layer(config, overlay, w, h)?;
            self.refresh_one_layer(
                &mut view.grid_texture,
                &mut view.grid_bind_group,
                &grid_rgba,
                w,
                h,
            )?;
            view.grid_space_gen = None;
            view.layer_stamps.grid += 1;
        }

        self.refresh_one_layer(
            &mut view.decoration_texture,
            &mut view.decoration_bind_group,
            &dec_rgba,
            w,
            h,
        )?;
        view.layer_stamps.decoration += 1;
        view.panel_rect = panel_rect;
        view.tick_warnings = axis_render::tick_warnings(config);
        view.raster_key = Some(key);
        view.raster_generation = view.raster_generation.wrapping_add(1);

        Ok(())
    }

    /// Bake the Milkyway backdrop for `(w, h, nebula, dust, seed)` unless the
    /// renderer already holds it, and upload it into the view's grid texture
    /// unless the view already holds that generation.
    fn refresh_space_backdrop(
        &mut self,
        view: &mut ChartView,
        config: &Config,
        c: &crate::config::MilkywayOptions,
        w: u32,
        h: u32,
    ) -> Result<()> {
        let key = SpaceBgKey {
            w,
            h,
            nebula: c.nebula.to_bits(),
            dust: c.dust.to_bits(),
            seed: c.seed,
        };
        if !self.space_bg.as_ref().is_some_and(|s| s.key == key) {
            let rgba = axis_render::try_raster_chart_layer_to_rgba(
                config,
                axis_render::AxisLayerKind::Grid,
            )?;
            let bake_gen = self.space_bg.as_ref().map_or(1, |s| s.bake_gen + 1);
            self.space_bg = Some(SpaceBgCache {
                key,
                bake_gen,
                rgba,
            });
        }
        let cache = self.space_bg.as_ref().expect("space_bg baked above");
        if view.grid_space_gen != Some(cache.bake_gen) {
            self.refresh_one_layer(
                &mut view.grid_texture,
                &mut view.grid_bind_group,
                &cache.rgba,
                w,
                h,
            )?;
            view.grid_space_gen = Some(cache.bake_gen);
            view.layer_stamps.grid += 1;
        }
        Ok(())
    }

    /// Update one texture in place or by reallocation. Used for both
    /// the grid and decoration layers.
    fn refresh_one_layer(
        &self,
        tex: &mut TrackedTexture,
        bg: &mut wgpu::BindGroup,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> Result<()> {
        let same_size = tex.width() == w && tex.height() == h;
        if same_size {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        } else {
            // The replacement is charged before the assignment below drops
            // the old one, so a resize reads as `old + new` for one frame —
            // which is what the device is holding.
            let new_tex = TrackedTexture::new(
                &self.gpu_ledger,
                GpuResourceKind::PanelTexture,
                data_render::upload_rgba_texture(
                    &self.device,
                    &self.queue,
                    self.caps.max_texture_dimension_2d,
                    w,
                    h,
                    rgba,
                )?,
            );
            let new_view_t = new_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let new_bg = data_render::create_texture_bind_group(
                &self.device,
                &self.texture_bgl,
                &new_view_t,
                &self.sampler,
            );
            *tex = new_tex;
            *bg = new_bg;
        }
        Ok(())
    }

    /// Draw multiple chart panels into one RenderPass.
    ///
    /// Facade over [`Self::prepare`] + [`Self::paint_prepared`] for hosts
    /// that own the renderer exclusively during their frame (winit loop,
    /// wasm wrapper, export): both phases run back to back under one
    /// `&mut self`, so no interleaving is possible and no token handling is
    /// needed. Hosts whose paint callback only provides `&self` (egui's
    /// `CallbackTrait::paint`, iced's `shader::Primitive::draw`) should call
    /// the two phases separately instead of wrapping the renderer in a
    /// `Mutex` — see the host-integration notes above.
    ///
    /// Takes a pass already opened by the host via `begin_render_pass`.
    /// `Renderer` owns the pool buffer, pipelines, and bind groups, so no
    /// external resources are needed.
    ///
    /// Returns `FiggyError::UnknownColumn` if a series references an unknown
    /// column id (or a stale handle).
    ///
    /// `target_size` is the color attachment pixel size of the current pass;
    /// panel_rect / data_area are clamped to it because wgpu validation would
    /// otherwise abort. Pass the surface pixel size reported by the host
    /// (winit, egui CallbackInfo, iced shader::Primitive, …).
    pub fn paint(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        items: &[ChartDrawItem<'_>],
    ) -> Result<()> {
        let prepared = self.prepare(items)?;
        self.paint_prepared(pass, target_size, &prepared)
    }
    fn preparation_parts(&mut self) -> (&HashMap<ChartId, ChartRenderState>, PrepareContext<'_>) {
        (
            &self.chart_states,
            PrepareContext {
                device: &self.device,
                queue: &self.queue,
                caps: &self.caps,
                pool: &self.pool,
                gpu_ledger: &self.gpu_ledger,
                style_bgl: &self.style_bgl,
                field_bgl: &self.field_bgl,
                vector_bgl: &self.vector_bgl,
                field_cell_tables: &self.field_cell_tables,
                transform_bgl: &self.transform_bgl,
                per_point_style_map_bgl: &self.per_point_style_map_bgl,
                point_value_map_bgl: &self.point_value_map_bgl,
                neutral_point_value_map_bg: &self.neutral_point_value_map_bg,
                data_selection_bgl: &self.data_selection_bgl,
                star_data_bgl: &self.star_data_bgl,
                sampler: &self.sampler,
                quad_vb: &self.quad_vb,
                pipelines: &mut self.pipelines,
                field_cache: &mut self.field_cache,
                value_map_cache: &mut self.value_map_cache,
                vector_cache: &mut self.vector_cache,
                contour_cache: &mut self.contour_cache,
                arc_cache: &mut self.arc_cache,
                texture_bgl: &self.texture_bgl,
                target_sample_count: self.target_sample_count,
                resident_displays: &mut self.resident_displays,
                resident_display_serial: &mut self.resident_display_serial,
                contour_label_pipelines: &mut self.contour_label_pipelines,
                arc_pipelines: &mut self.arc_pipelines,
                surface_format: self.surface_format,
                renderer_identity: self.renderer_identity,
                target_pipeline_generation: self.target_pipeline_generation,
                #[cfg(test)]
                arc_chunk_override: self.arc_chunk_override,
            },
        )
    }
    /// Mutable half of the frame: finish every state change the draw needs,
    /// before any render pass opens.
    ///
    /// Does not rerasterize axes/decorations or rebuild caller-owned styles.
    /// Refresh a changed view with [`Self::refresh_axis`] (or its selection
    /// variant), and recreate affected styles with [`Self::create_style_for_series`]
    /// before calling this method. Unchanged inputs may be reused.
    ///
    /// Runs the styled-pipeline ensure (compiles pipeline sets the items
    /// need but the cache lacks), writes each panel's data→NDC transform
    /// uniform from `item.chart_config`, and (re)dispatches the per-series
    /// GPU arc-length scans — submitting their compute encoder on the queue,
    /// which orders it before the host's later pass submission. Returns an
    /// owned [`PreparedFrame`] token snapshotting everything the immutable
    /// draw phase will reference.
    ///
    /// Because the transform uniform and the arc dispatch are written here
    /// from the same config snapshot, they cannot desync — calling
    /// [`Self::update_transform`] per frame becomes unnecessary (though
    /// harmless) for callers of this API.
    ///
    /// Contract: call after this frame's data uploads (`add_column`),
    /// `ensure_target_format`, and `refresh_axis`; before the host records and
    /// submits the command buffer containing the render pass. Once a command
    /// buffer has been recorded from the token, submit it before any later
    /// mutation of the same [`ChartView`] or another `prepare` for that view.
    /// The returned token owns the resolved draw input. `paint_prepared`
    /// therefore receives no second
    /// copy of chart/view/series/style inputs. It rejects renderer resource
    /// changes that invalidate the captured GPU work: target-pipeline rebuild,
    /// pool layout change, replacement of any captured column allocation, or
    /// another transform/axis write to a captured `ChartView`.
    pub fn prepare(&mut self, items: &[ChartDrawItem<'_>]) -> Result<PreparedFrame> {
        for item in items {
            for series in item.series {
                self.require_resident_series(series.config)?;
            }
        }
        self.preparation_parts().1.prepare(items)
    }

    /// Prepare renderer-owned charts without copying their Config or ordered
    /// SeriesConfig values. GPU styles are cached derivatives of those series;
    /// chart identity is always the explicit id, never inferred from resources.
    ///
    /// Refresh each view's raster before this call as for [`Self::prepare`].
    /// Mutating or removing a captured chart makes its token stale. Changes to
    /// unrelated charts do not invalidate the captured chart stamps.
    pub fn prepare_registered(
        &mut self,
        items: &[RegisteredChartDrawItem<'_>],
    ) -> Result<PreparedFrame> {
        self.prepare_registered_mixed(items)
            .map_err(streaming_runtime::StreamRequestError::into_figgy)
    }

    fn prepare_registered_resident(
        &mut self,
        items: &[RegisteredChartDrawItem<'_>],
    ) -> Result<PreparedFrame> {
        let allocation =
            |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource: "registered chart prepare inputs",
                reason: error.to_string(),
            };
        // Resolve every id and fallible contour invariant before creating any
        // GPU styles or advancing the content revision of a supplied view.
        for item in items {
            let state = self
                .chart_states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            for series in &state.series {
                self.require_resident_series(series)?;
                validate_contour_level_count(&series.series_id, &series.render_type)?;
                validate_polar_series(&state.config, series)?;
                preflight_contour_label_spacing(
                    &series.series_id,
                    &series.render_type,
                    effective_series_primitives(&state.config.draw_style, &series.render_type)
                        .contour,
                    1.0,
                )?;
            }
        }
        let mut style_updates: Vec<(ChartId, RegisteredChartStyles)> = Vec::new();
        style_updates
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        for item in items {
            let state = self
                .chart_states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            if state
                .prepared_styles
                .as_ref()
                .is_some_and(|styles| styles.series_revision == state.revisions.series)
                || style_updates.iter().any(|(id, _)| *id == item.chart_id)
            {
                continue;
            }
            let mut styles = Vec::new();
            styles
                .try_reserve_exact(state.series.len())
                .map_err(allocation)?;
            for series in &state.series {
                styles.push(self.create_style_for_series_scaled_inner(series, 1.0)?);
            }
            style_updates.push((
                item.chart_id,
                RegisteredChartStyles {
                    series_revision: state.revisions.series,
                    styles,
                },
            ));
        }
        for (id, styles) in style_updates {
            self.chart_states
                .get_mut(&id)
                .ok_or(FiggyError::UnknownChart { id })?
                .prepared_styles = Some(styles);
        }
        let (states, mut preparation) = self.preparation_parts();
        let mut series_inputs = Vec::new();
        let mut stamps = Vec::new();
        series_inputs
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        stamps.try_reserve_exact(items.len()).map_err(allocation)?;
        for item in items {
            let state = states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            let styles = &state
                .prepared_styles
                .as_ref()
                .expect("registered styles were prepared")
                .styles;
            let mut series = Vec::new();
            series
                .try_reserve_exact(state.series.len())
                .map_err(allocation)?;
            series.extend(
                state
                    .series
                    .iter()
                    .zip(styles)
                    .map(|(config, style)| Series { config, style }),
            );
            series_inputs.push(series);
            stamps.push(PreparedChartStamp {
                chart_id: item.chart_id,
                desired: state.revisions.desired,
                data: state.revisions.data,
            });
        }
        let mut draws = Vec::new();
        draws.try_reserve_exact(items.len()).map_err(allocation)?;
        for (item, series) in items.iter().zip(&series_inputs) {
            let state = states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            draws.push(ChartDrawItem {
                view: item.view,
                chart_config: &state.config,
                series,
            });
        }
        let mut frame = preparation.prepare(&draws)?;
        frame.registered_charts = stamps;
        Ok(frame)
    }

    /// Select the current resident or owned streaming display for each chart
    /// from its explicit id. Paint receives only the prepared token.
    pub(crate) fn prepare_registered_mixed(
        &mut self,
        items: &[RegisteredChartDrawItem<'_>],
    ) -> streaming_runtime::StreamResult<PreparedFrame> {
        let allocation =
            |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource: "mixed chart prepare inputs",
                reason: error.to_string(),
            };
        let mut resident = Vec::new();
        let mut displays = Vec::new();
        let mut streamed = Vec::new();
        // Prepared items a resident chart contributes: one per panel of its
        // layered document (`expand_for_render`); zero for a streamed chart.
        let mut panel_counts = Vec::new();
        resident
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        displays
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        streamed
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        panel_counts
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        for item in items {
            let config = self.chart_config(item.chart_id)?;
            let active = self.chart_has_stream_surface(item.chart_id);
            panel_counts.push(if active {
                0
            } else {
                render_panel_count(config)
            });
            if active {
                self.preflight_chart_stream_display(item.chart_id, item.view)?;
            }
            if !active {
                resident.push(RegisteredChartDrawItem {
                    chart_id: item.chart_id,
                    view: item.view,
                });
            }
            streamed.push(active);
            displays.push(None);
        }
        let mut frame = self.prepare_registered_resident(&resident)?;
        for ((item, active), display) in items.iter().zip(streamed).zip(&mut displays) {
            if active {
                *display = Some(
                    self.prepare_chart_stream_display(item.chart_id, item.view)?
                        .ok_or(crate::streaming::StreamError::WrongState)?,
                );
            }
        }
        let mut resident_items = std::mem::take(&mut frame.items).into_iter();
        frame
            .items
            .try_reserve_exact(items.len() - resident.len() + panel_counts.iter().sum::<usize>())
            .map_err(allocation)?;
        frame
            .registered_charts
            .try_reserve_exact(items.len() - resident.len())
            .map_err(allocation)?;
        for (item, (display, count)) in items.iter().zip(displays.into_iter().zip(panel_counts)) {
            if let Some(display) = display {
                let state = self
                    .chart_states
                    .get(&item.chart_id)
                    .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
                frame.registered_charts.push(PreparedChartStamp {
                    chart_id: item.chart_id,
                    desired: state.revisions.desired,
                    data: state.revisions.data,
                });
                frame.items.push(PreparedItem {
                    view: PreparedView::capture(
                        item.view,
                        item.view
                            .content_revision
                            .load(std::sync::atomic::Ordering::Acquire),
                        state.config.annotations.visible,
                    ),
                    data_area: state
                        .config
                        .data_area()
                        .map(|area| area.0)
                        .unwrap_or(item.view.panel_rect),
                    axis_pipeline: self.pipelines.axis.clone(),
                    series: Vec::new(),
                    stream_display: Some(display),
                    resident_display: None,
                    column_fingerprint: 0,
                });
            } else {
                for _ in 0..count {
                    frame.items.push(resident_items.next().ok_or(
                        streaming_runtime::StreamRequestError::Scheduler(
                            crate::streaming::StreamError::WrongState,
                        ),
                    )?);
                }
            }
        }
        Ok(frame)
    }

    /// Immutable half of the frame: pure command recording against a
    /// [`PreparedFrame`] from [`Self::prepare`].
    ///
    /// `&self` — safe to call from host paint callbacks that only hand out
    /// shared access (egui `CallbackTrait::paint` via `&CallbackResources`,
    /// iced `shader::Primitive::draw` via `&Storage`), with no lock around
    /// the renderer. Repeatable: the same token may be recorded into more
    /// than one pass (egui re-record, iced mid-frame pass restart).
    ///
    /// All chart/view/series/style input comes from `prepared`; the host does
    /// not reconstruct or pass it again. Renderer-level stamps detect
    /// invalidating `&mut self` calls since `prepare`. On mismatch nothing is
    /// recorded and [`FiggyError::StalePreparedFrame`] is returned; recovery
    /// is a fresh `prepare`. A recorded command buffer is outside Rust's
    /// borrow graph, so the host must submit it before mutating or preparing
    /// the same [`ChartView`] again; validation can only run while recording.
    pub fn paint_prepared(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        prepared: &PreparedFrame,
    ) -> Result<()> {
        self.validate_prepared(prepared)?;
        self.paint_prepared_items(pass, target_size, &prepared.items)
    }

    /// Serial of the cached series bodies of `view`'s panel, `None` while it
    /// has none: the count of draws a test can read per panel.
    #[cfg(test)]
    pub(crate) fn resident_display_serial(&self, view: &ChartView) -> Option<u64> {
        let id = Arc::as_ptr(&view.content_revision) as usize;
        self.resident_displays
            .iter()
            .find(|display| display.view.as_ptr() as usize == id)
            .map(|display| display.serial)
    }

    #[cfg(test)]
    pub(crate) fn resident_display_count(&self) -> usize {
        self.resident_displays.len()
    }

    /// Stamp checks guarding `paint_prepared` — see [`PreparedFrame`] field
    /// docs for what each stamp catches. Kept separate from recording so a
    /// stale token fails before any pass state is touched.
    fn validate_prepared(&self, prepared: &PreparedFrame) -> Result<()> {
        let stale = |reason: String| FiggyError::StalePreparedFrame { reason };
        if prepared.renderer_identity != self.renderer_identity {
            return Err(stale(
                "prepared frame belongs to a different renderer".into(),
            ));
        }
        if prepared.target_pipeline_generation != self.target_pipeline_generation {
            return Err(stale(format!(
                "prepared at target pipeline generation {}, renderer is now at {} \
                 (the target format or sample count changed between prepare and paint_prepared)",
                prepared.target_pipeline_generation, self.target_pipeline_generation
            )));
        }
        for stamp in &prepared.registered_charts {
            let unchanged = self.chart_states.get(&stamp.chart_id).is_some_and(|state| {
                state.revisions.desired == stamp.desired && state.revisions.data == stamp.data
            });
            if !unchanged {
                return Err(stale(format!(
                    "registered chart {:?} changed or was removed between prepare and paint_prepared",
                    stamp.chart_id,
                )));
            }
        }
        if prepared.pool_layout_generation != self.pool.layout_generation() {
            return Err(stale(format!(
                "prepared at pool layout generation {}, pool is now at {} \
                 (the pool layout changed between prepare and paint_prepared)",
                prepared.pool_layout_generation,
                self.pool.layout_generation()
            )));
        }
        for (idx, item) in prepared.items.iter().enumerate() {
            let current = item
                .view
                .content_revision
                .load(std::sync::atomic::Ordering::Acquire);
            if current != item.view.expected_content_revision {
                return Err(stale(format!(
                    "panel {idx} view changed between prepare and paint_prepared"
                )));
            }
            if let Some(display) = &item.resident_display
                && !self.resident_displays.iter().any(|held| {
                    held.view.as_ptr() as usize == display.view && held.serial == display.serial
                })
            {
                return Err(stale(format!(
                    "panel {idx} cached composite was drawn again after prepare"
                )));
            }
            if let Some(display) = &item.stream_display
                && !self.stream_display_packet_is_current(display)
            {
                return Err(stale(format!(
                    "panel {idx} stream display changed or was cancelled after prepare"
                )));
            }
        }
        for (id, allocation_epoch) in &prepared.column_sources {
            if self.pool.allocation_epoch(id) != Some(*allocation_epoch) {
                return Err(stale(format!(
                    "column {:?} changed allocation between prepare and paint_prepared",
                    id
                )));
            }
        }
        // A field's grid columns, walked from the shared receipt: reads only, so
        // a 5000-column matrix costs no allocation to validate.
        for sources in &prepared.grid_sources {
            for (id, allocation_epoch) in sources.ids.iter().zip(&sources.epochs) {
                if self.pool.allocation_epoch(id) != Some(*allocation_epoch) {
                    return Err(stale(format!(
                        "grid column {:?} changed allocation between prepare and paint_prepared",
                        id
                    )));
                }
            }
        }
        Ok(())
    }

    /// Read the instance-count word from a prepared contour label's indirect
    /// draw arguments. This exists only for native unit tests and wasm browser
    /// conformance tests, where pixels cannot distinguish all 1024 instances.
    #[cfg(any(test, target_arch = "wasm32"))]
    #[doc(hidden)]
    pub async fn contour_label_instance_count_for_test(
        &self,
        prepared: &PreparedFrame,
        item_index: usize,
        series_index: usize,
    ) -> Result<Option<u32>> {
        self.validate_prepared(prepared)?;
        let Some(indirect) = prepared
            .items
            .get(item_index)
            .and_then(|item| item.series.get(series_index))
            .and_then(|series| series.contour.as_ref())
            .and_then(|contour| contour.label.as_ref())
            .map(|label| label.snapshot.indirect())
        else {
            return Ok(None);
        };

        // gpu-alloc: Readback
        let readback = TrackedBuffer::new(
            &self.gpu_ledger,
            GpuResourceKind::Readback,
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("figgy contour label indirect count readback"),
                size: 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy contour label indirect count copy"),
            });
        encoder.copy_buffer_to_buffer(indirect, 4, &readback, 0, 4);
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = futures_channel::oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        rx.await
            .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                resource: "contour label indirect count readback",
                reason: format!("map_async sender dropped: {error}"),
            })?
            .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                resource: "contour label indirect count readback",
                reason: format!("map_async: {error:?}"),
            })?;
        let mapped =
            slice
                .get_mapped_range()
                .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                    resource: "contour label indirect count readback",
                    reason: format!("get_mapped_range: {error:?}"),
                })?;
        let count = u32::from_le_bytes(mapped[..4].try_into().expect("four-byte readback"));
        drop(mapped);
        readback.unmap();
        Ok(Some(count))
    }

    /// Per-item body of the prepare phase (also reused by export). For each
    /// panel: computes ONE transform snapshot from `item.chart_config`,
    /// writes it to the panel's transform uniform, dispatches the arc-length
    /// scan of every line series that needs one with that same snapshot —
    /// dashed lines (dash phase) and every line of a style with
    /// `needs_arc_prefix` (sketch: the wobble is parameterized by arc
    /// length, so solid sketch lines need the prefix too). The immutable draw
    /// phase later reads the owned result, never the live `arc_cache`.
    /// Resolve every field series' group-2 state, reusing what has not changed.
    ///
    /// The prepare-phase `&mut self` half of the field path, next to
    /// `prepare_arc_items` and for the same reason: `build_series_layers` is
    /// `&self`, so anything that has to be built has to be built here. Returns
    /// the per-field source receipts in `items` order for the token to hold.
    fn ensure_field_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
    ) -> Result<Vec<Arc<GridSources>>> {
        self.preparation_parts().1.ensure_field_scratches(items)
    }

    fn ensure_value_map_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<()> {
        self.preparation_parts()
            .1
            .ensure_value_map_scratches(items, scale)
    }

    fn ensure_vector_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<Vec<Arc<GridSources>>> {
        self.preparation_parts()
            .1
            .ensure_vector_scratches(items, scale)
    }

    fn ensure_vector_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<Option<Arc<GridSources>>> {
        self.preparation_parts()
            .1
            .ensure_vector_scratch(chart_config, cfg, scale)
    }

    /// Build or reuse one field series' group-2 bind group.
    ///
    /// The signature is compared before anything is created, so a steady frame
    /// uploads nothing at all. When it does differ, the new buffers replace the
    /// old ones outright — a live `PreparedFrame` holds its own refcounted clone
    /// of the bind group, so nothing it can still draw with is disturbed. That is
    /// why this needs none of `arc_cache`'s immutable keyed results: the field
    /// buffers themselves are already immutable snapshots.
    fn ensure_field_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
    ) -> Result<Arc<GridSources>> {
        self.preparation_parts()
            .1
            .ensure_field_scratch(chart_config, cfg)
    }

    /// Prepare every contour series' reusable GPU state.
    ///
    /// Returns the grid receipts for the token, the same way
    /// `ensure_field_scratches` does. Automatic label anchors are the only
    /// contour compute dispatch and remain frame-dependent.
    /// `scale` is the export DPI multiplier — the same one
    /// `create_style_for_series_scaled` applies to every other pixel dimension.
    /// It rides in the signature, so switching between a window frame and an
    /// export rebuilds the level styles rather than drawing one at the other's
    /// stroke width.
    fn ensure_contour_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
        contour_spacings: &[Vec<Option<f32>>],
    ) -> Result<(Vec<PreparedContourItem>, Vec<Arc<GridSources>>)> {
        self.preparation_parts()
            .1
            .ensure_contour_scratches(items, scale, contour_spacings)
    }

    /// Build or reuse one contour series' GPU state.
    ///
    /// Since design B.4.9 this resolves the same group-2 tables a field does: the
    /// isolines are drawn from the grid itself by `fs_contour`, so there is no
    /// trace to dispatch and nothing here depends on the panel transform. The
    /// label anchors do — `ensure_contour_scratches` captures one placement
    /// snapshot per panel/item + series occurrence.
    ///
    /// `scale` is the export DPI multiplier, applied to the stroke width and the
    /// label font exactly as `create_style_for_series_scaled` applies it
    /// everywhere else.
    fn ensure_contour_scratch(
        &mut self,
        cfg: &SeriesConfig,
        chart_config: &Config,
        scale: f32,
    ) -> Result<Arc<GridSources>> {
        self.preparation_parts()
            .1
            .ensure_contour_scratch(cfg, chart_config, scale)
    }

    /// Compile the label shader's module and layouts if any item wants labels.
    ///
    /// Separate from `ensure_precise_variants_for_items` because that function
    /// may only *use* them: it is the single site allowed to compile a target
    /// pipeline (design B.6), and it has no `&mut self` to create the module
    /// with.
    fn ensure_contour_label_pipelines(&mut self, items: &[ChartDrawItem<'_>]) {
        self.preparation_parts()
            .1
            .ensure_contour_label_pipelines(items)
    }

    fn prepare_arc_items(&mut self, items: &[ChartDrawItem<'_>]) -> Result<Vec<PreparedArcItem>> {
        self.preparation_parts().1.prepare_arc_items(items)
    }

    fn resolve_prepared_items(
        &mut self,
        items: &[ChartDrawItem<'_>],
        prepared_arcs: &[PreparedArcItem],
        prepared_contours: &[PreparedContourItem],
        pipelines: &TargetPipelines,
    ) -> Result<(Vec<PreparedItem>, HashMap<ColumnId, u64>)> {
        self.preparation_parts().1.resolve_prepared_items(
            items,
            prepared_arcs,
            prepared_contours,
            pipelines,
        )
    }

    fn paint_prepared_items<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        prepared: &'a [PreparedItem],
    ) -> Result<()> {
        for item in prepared {
            if let Some(display) = &item.stream_display
                && display.size != target_size
            {
                return Err(FiggyError::StalePreparedFrame {
                    reason: "stream display size differs from paint target".into(),
                });
            }
        }
        for item in prepared {
            if let Some(display) = &item.stream_display {
                if let Some(panel) =
                    data_render::clamp_rect_to_target(item.view.panel_rect, target_size)
                {
                    // D covers the whole target, unlike a panel-sized axis texture.
                    pass.set_viewport(
                        0.0,
                        0.0,
                        target_size.0 as f32,
                        target_size.1 as f32,
                        0.0,
                        1.0,
                    );
                    pass.set_scissor_rect(panel.x, panel.y, panel.width, panel.height);
                    pass.set_pipeline(&item.axis_pipeline);
                    pass.set_bind_group(0, &display.bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                continue;
            }
            let panel_rect = item.view.panel_rect;
            let series_list: Vec<_> = item.series.iter().map(PreparedSeries::layers).collect();
            let (annotation_underlay, annotation_overlay) =
                item.view.annotation_layers(&item.axis_pipeline);

            // The single style decision per panel: the chart's `DrawStyle`
            // resolves to a cached styled pipeline set (compiled by the
            // prepare phase) or `None` for precise — the precise path stays
            // untouched.
            // Bundle every series's primitives for the panel into one call.

            // Cached bodies cover the whole panel; a panel the target clips
            // draws them directly, as before.
            let cached = item
                .resident_display
                .as_ref()
                .filter(|_| {
                    panel_rect.x.checked_add(panel_rect.width) <= Some(target_size.0)
                        && panel_rect.y.checked_add(panel_rect.height) <= Some(target_size.1)
                })
                .map(|display| AxisLayer {
                    pipeline: &item.axis_pipeline,
                    bind_group: &display.bind_group,
                });
            data_render::draw_chart_panel_with_bodies(
                pass,
                target_size,
                panel_rect,
                item.data_area,
                AxisLayer {
                    pipeline: &item.axis_pipeline,
                    bind_group: &item.view.grid_bind_group,
                },
                annotation_underlay,
                &series_list,
                cached,
                AxisLayer {
                    pipeline: &item.axis_pipeline,
                    bind_group: &item.view.decoration_bind_group,
                },
                annotation_overlay,
            );
        }
        Ok(())
    }

    /// Ensure the GPU arc-length prefix for one dashed line series and return
    /// the token-side snapshot: the prefix buffer slice, the owned star-pass
    /// handles (when the scratch has one), and the source-column layout the
    /// scan ran over. The whole computation runs on the GPU
    /// (`line_arc.wgsl` compute scan over the pool columns) — the data never
    /// returns to the CPU, keeping the pool's no-CPU-copy contract intact.
    /// Series longer than one chunk's dispatch capacity scan as sequential
    /// chunks linked by a carry buffer, so any pool-resident `n` is fine.
    ///
    /// A distinct scan input is dispatched once into new immutable buffers.
    /// An exact key hit reuses that result without traversing the columns. An
    /// old result is never rewritten: after `paint_prepared` records a buffer,
    /// the host may drop its [`PreparedFrame`] before submitting the command
    /// buffer, and the late submission must still see the recorded generation.
    ///
    /// `star_pitch`: `Some` adds the arc-driven star pass to the scratch
    /// (indirect args + VS bind group) and runs its kernel after the scan.
    /// Prepare passes `Some` only for Milkyway lines; `DrawStyle::Constellation`
    /// has no arc-prefix star pass.
    #[cfg(test)]
    fn ensure_arc_prefix(
        &mut self,
        series_id: &str,
        x_id: &str,
        y_id: &str,
        t: &data_render::ScatterTransform,
        star_pitch: Option<f32>,
        interpolation: LineInterpolation,
    ) -> Option<PreparedArc> {
        self.preparation_parts().1.ensure_arc_prefix(
            series_id,
            x_id,
            y_id,
            t,
            star_pitch,
            interpolation,
        )
    }

    // Headless PNG export.
}

// Export result type, scale clamping, PNG encoding.

// Public auxiliary types.

/// Per-panel GPU resources: separate grid + decoration textures (for layered
/// compositing) plus the transform uniform.
///
/// Grid (below data) and decoration (axis lines, labels — above data) are
/// rasterized to two textures so the GPU can composite in order:
/// `grid → data → decoration`.
pub struct ChartView {
    grid_texture: TrackedTexture,
    grid_bind_group: wgpu::BindGroup,
    decoration_texture: TrackedTexture,
    decoration_bind_group: wgpu::BindGroup,
    /// The annotation layer: spans below the data, everything else above
    /// every decoration. `None` where the layer drew nothing on that side.
    /// Refreshed by `refresh_axis*` (the anchor space may have moved) and by
    /// `refresh_annotations` (the items changed); never by anything else.
    annotation_underlay: Option<crate::annotation::LayerTexture>,
    annotation_overlay: Option<crate::annotation::LayerTexture>,
    annotation_map: crate::annotation::AnnotationMap,
    /// The annotation whose selection box the overlay carries.
    annotation_selected: Option<usize>,
    layer_stamps: ChartViewLayerStamps,
    transform_buffer: TrackedBuffer,
    transform_bg: wgpu::BindGroup,
    content_revision: Arc<std::sync::atomic::AtomicU64>,
    /// Changes only when the streamed data prefix's coordinate space changes.
    /// Decoration texture rewrites deliberately leave this revision untouched.
    stream_revision: Arc<std::sync::atomic::AtomicU64>,
    /// Panel pixel rect in surface coordinates.
    panel_rect: Rect,
    /// Generation of the renderer's cached constellation backdrop currently
    /// uploaded into `grid_texture` (`None` = non-cached content). Lets
    /// `refresh_axis` skip the full-panel texture write on cache hits.
    grid_space_gen: Option<u64>,
    /// Tick-guard substitutions in the current axis raster, derived from the
    /// chart that `create_chart_view` or the last successful `refresh_axis*`
    /// rasterized. On a layered view this is the merged list of every panel,
    /// overlay warnings tagged `WarningAxis::OverlayY(index)`.
    tick_warnings: Vec<crate::RenderWarning>,
    /// `tick_warnings` followed by the annotation layer's skipped items.
    render_warnings: Vec<crate::RenderWarning>,
    /// One nested single-panel view per overlay layer, in `Config::overlays`
    /// order. Each holds its own grid and decoration textures and transform
    /// uniform; none nests further (one level of layering).
    overlay_views: Vec<ChartView>,
    /// What this panel's textures hold: the internal panel config, the
    /// selection boxes composited into its decoration texture, the font
    /// generation and the panel rect of the last raster. A refresh that
    /// arrives with the same key uploads nothing, which is what keeps the
    /// other layers' textures untouched when one layer is edited.
    raster_key: Option<PanelRasterKey>,
    /// Counts the rasters uploaded into this panel's textures.
    raster_generation: u64,
}

/// Cache key of one panel's grid and decoration rasters.
#[derive(Clone, PartialEq)]
struct PanelRasterKey {
    config: Config,
    selection: Vec<crate::select::SelectionBox>,
    font_generation: u64,
    panel_rect: Rect,
}

impl PanelRasterKey {
    /// The annotation layer has its own rasters and revision, so it is not
    /// part of the axis key: an annotation edit keeps every panel's cache.
    fn new(
        config: &Config,
        selection: &[crate::select::SelectionBox],
        font_generation: u64,
        panel_rect: Rect,
    ) -> Self {
        let mut config = config.clone();
        config.annotations = crate::AnnotationLayer::default();
        Self {
            config,
            selection: selection.to_vec(),
            font_generation,
            panel_rect,
        }
    }
}

/// How many times each raster layer of a [`ChartView`] has been written,
/// starting at 1 for the view's creation. Lets a host or a test prove that a
/// refresh touched only the layer it was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChartViewLayerStamps {
    pub grid: u64,
    pub decoration: u64,
    pub annotations: u64,
}

impl ChartView {
    pub fn panel_rect(&self) -> Rect {
        self.panel_rect
    }

    /// The number of overlay layers this view holds textures for, which is
    /// the overlay count of the config it was created or last refreshed with.
    pub fn overlay_count(&self) -> usize {
        self.overlay_views.len()
    }

    /// How many rasters have been uploaded into each panel's textures, base
    /// panel first and then one entry per overlay. An edit of one layer
    /// advances only that layer's entry (and the top panel's, when the
    /// selection boxes it carries change); a frame that reuses a panel's
    /// cached raster leaves its entry alone.
    pub fn layer_raster_generations(&self) -> Vec<u64> {
        std::iter::once(self.raster_generation)
            .chain(self.overlay_views.iter().map(|view| view.raster_generation))
            .collect()
    }

    /// Renderer-measured annotation hit map of the current annotation
    /// raster, in the chart's own coordinates (the chart area's frame).
    /// Rebuilt by `refresh_axis*` and `refresh_annotations`; a `HitId` from
    /// it is valid until the next of those calls.
    pub fn annotation_map(&self) -> &crate::annotation::AnnotationMap {
        &self.annotation_map
    }

    /// The pointer route of the active layer at `position`: while
    /// `config.active_layer()` is `Annotations` (and the layer is shown) the
    /// annotation items of this view sit on top of `hit_map`, otherwise only
    /// `hit_map` answers and clicks pass through the items. `config` is the
    /// config the view was last refreshed with (the display config of a
    /// scaled view) and `hit_map` its [`crate::HitMap::for_active_layer`] map, so
    /// both maps measure in the same pixels.
    pub fn hit_test_active_layer(
        &self,
        config: &Config,
        hit_map: &crate::HitMap,
        position: [f32; 2],
    ) -> Option<crate::annotation::LayerHit> {
        self.annotation_map
            .route(config.active_layer(), &config.annotations, position, || {
                hit_map.hit_test(
                    config,
                    &crate::CpuTextMeasure::for_style(&config.draw_style),
                    position[0],
                    position[1],
                )
            })
    }

    /// The annotation whose selection box the overlay currently carries, as
    /// last passed to [`Renderer::refresh_annotations`].
    pub fn annotation_selected(&self) -> Option<usize> {
        self.annotation_selected
    }

    pub fn layer_stamps(&self) -> ChartViewLayerStamps {
        self.layer_stamps
    }

    fn rebuild_render_warnings(&mut self) {
        self.render_warnings = self
            .tick_warnings
            .iter()
            .chain(self.annotation_map.warnings())
            .cloned()
            .collect();
    }

    /// Bind groups of the annotation textures for the stream display
    /// composition, which draws underlay and overlay itself.
    pub(crate) fn annotation_bind_groups(
        &self,
        visible: bool,
    ) -> (Option<&wgpu::BindGroup>, Option<&wgpu::BindGroup>) {
        if !visible {
            return (None, None);
        }
        (
            self.annotation_underlay.as_ref().map(|l| &l.bind_group),
            self.annotation_overlay.as_ref().map(|l| &l.bind_group),
        )
    }

    /// The tick settings this view's current axis raster drew differently
    /// from how its chart's config wrote them: an impossible `major_spacing`
    /// drawn at automatic spacing, a clamped `minor_count`, an adjusted log
    /// decade step. Empty when every setting is drawn as written.
    ///
    /// [`Renderer::create_chart_view`] sets it, and every successful
    /// [`Renderer::refresh_axis`] / [`Renderer::refresh_axis_with_selection`]
    /// re-derives it from the chart it rasterized; nothing else changes it.
    /// A host that refreshes only when its dirty flag or
    /// [`ChartRenderStamp::needs_raster_since`] says the raster changed reads
    /// it right after that refresh. Compare with the previous list to report
    /// changes only; a NaN `requested` never compares equal.
    ///
    /// The tick warnings are followed by the annotation layer's
    /// `AnnotationSkipped` warnings, re-derived by `refresh_annotations` and
    /// by every `refresh_axis*` (the latter redoes the annotation raster).
    pub fn render_warnings(&self) -> &[crate::RenderWarning] {
        &self.render_warnings
    }

    fn advance_content_revision(&self) -> Result<u64> {
        self.content_revision
            .try_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |current| current.checked_add(1),
            )
            .map(|previous| previous + 1)
            .map_err(|_| FiggyError::CounterExhausted {
                counter: "chart view content revision",
            })
    }

    fn advance_stream_revision(&self) -> Result<u64> {
        self.stream_revision
            .try_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |current| current.checked_add(1),
            )
            .map(|previous| previous + 1)
            .map_err(|_| FiggyError::CounterExhausted {
                counter: "chart view stream revision",
            })
    }
}

// Disjoint GPU borrows let registered preparation read the registry in place.
// This context never owns chart Config or SeriesConfig values.
struct PrepareContext<'a> {
    device: &'a Arc<wgpu::Device>,
    queue: &'a Arc<wgpu::Queue>,
    caps: &'a RendererDeviceCaps,
    pool: &'a ColumnPool,
    gpu_ledger: &'a Arc<GpuLedger>,
    style_bgl: &'a wgpu::BindGroupLayout,
    field_bgl: &'a wgpu::BindGroupLayout,
    vector_bgl: &'a wgpu::BindGroupLayout,
    field_cell_tables: &'a data_render::FieldCellTables,
    transform_bgl: &'a wgpu::BindGroupLayout,
    per_point_style_map_bgl: &'a wgpu::BindGroupLayout,
    point_value_map_bgl: &'a wgpu::BindGroupLayout,
    neutral_point_value_map_bg: &'a wgpu::BindGroup,
    data_selection_bgl: &'a wgpu::BindGroupLayout,
    star_data_bgl: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
    quad_vb: &'a TrackedBuffer,
    pipelines: &'a mut TargetPipelines,
    field_cache: &'a mut HashMap<String, FieldScratch>,
    value_map_cache: &'a mut HashMap<String, ValueMapScratch>,
    vector_cache: &'a mut HashMap<String, VectorScratch>,
    contour_cache: &'a mut HashMap<String, ContourScratchEntry>,
    arc_cache: &'a mut HashMap<String, Vec<ArcResultEntry>>,
    texture_bgl: &'a wgpu::BindGroupLayout,
    target_sample_count: u32,
    resident_displays: &'a mut Vec<ResidentDisplay>,
    resident_display_serial: &'a mut u64,
    contour_label_pipelines: &'a mut Option<crate::gpu_contour::ContourLabelPipelines>,
    arc_pipelines: &'a mut Option<data_render::line_arc::ArcScanPipelines>,
    surface_format: wgpu::TextureFormat,
    renderer_identity: u64,
    target_pipeline_generation: u64,
    #[cfg(test)]
    arc_chunk_override: Option<u32>,
}

impl PrepareContext<'_> {
    fn prepare(&mut self, items: &[ChartDrawItem<'_>]) -> Result<PreparedFrame> {
        // One expansion path: a layered document becomes one item per panel
        // here, so everything below sees single-layer panels only.
        // A frame without layered items is drawn from the caller's slice as
        // it is, so it pays no allocation for expansion.
        let layered = items.iter().any(|item| is_layered(item.chart_config));
        let storage = if layered {
            Some(PanelDrawStorage::expand(items)?)
        } else {
            None
        };
        let group_sizes = storage
            .as_ref()
            .map(|storage| storage.group_sizes(items))
            .unwrap_or_default();
        let expanded = storage.as_ref().map(|storage| storage.items(items));
        let items: &[ChartDrawItem<'_>] = match &expanded {
            Some(expanded) => expanded.as_slice(),
            None => items,
        };
        let mut contour_spacings = Vec::with_capacity(items.len());
        for item in items {
            let mut per_series = Vec::with_capacity(item.series.len());
            for series in item.series {
                validate_contour_level_count(
                    series.config.series_id.as_str(),
                    &series.config.render_type,
                )?;
                validate_line_fill(
                    self.pool,
                    item.series.iter().map(|series| series.config),
                    series.config,
                )?;
                let contour_enabled = effective_series_primitives(
                    &item.chart_config.draw_style,
                    &series.config.render_type,
                )
                .contour;
                per_series.push(preflight_contour_label_spacing(
                    series.config.series_id.as_str(),
                    &series.config.render_type,
                    contour_enabled,
                    1.0,
                )?);
            }
            contour_spacings.push(per_series);
        }
        // The label draw's module and group-1 layout live on the renderer, and
        // the target pipeline is compiled from them — so they have to exist
        // before the one place allowed to compile new pipelines runs.
        self.ensure_contour_label_pipelines(items);
        self.pipelines.ensure_precise_variants_for_items(
            &self.device,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            self.point_value_map_bgl,
            &self.data_selection_bgl,
            &self.field_bgl,
            self.vector_bgl,
            self.contour_label_pipelines.as_ref(),
            self.surface_format,
            items,
        );
        self.pipelines.ensure_styles_for_items(
            &self.device,
            &self.queue,
            &self.transform_bgl,
            &self.style_bgl,
            &self.star_data_bgl,
            self.surface_format,
            items,
        );
        let mut grid_sources = self.ensure_field_scratches(items)?;
        self.ensure_value_map_scratches(items, 1.0)?;
        grid_sources.append(&mut self.ensure_vector_scratches(items, 1.0)?);
        let (prepared_contours, mut contour_sources) =
            self.ensure_contour_scratches(items, 1.0, &contour_spacings)?;
        grid_sources.append(&mut contour_sources);
        let prepared_arcs = self.prepare_arc_items(items)?;
        let (mut prepared, column_sources) = self.resolve_prepared_items(
            items,
            &prepared_arcs,
            &prepared_contours,
            &self.pipelines,
        )?;
        if layered {
            lift_annotation_overlays(&mut prepared, &group_sizes);
        }
        self.attach_resident_displays(items, &mut prepared);
        Ok(PreparedFrame {
            renderer_identity: self.renderer_identity,
            items: prepared,
            column_sources,
            pool_layout_generation: self.pool.layout_generation(),
            target_pipeline_generation: self.target_pipeline_generation,
            grid_sources,
            registered_charts: Vec::new(),
        })
    }

    /// Give every qualifying resident panel its cached composite, drawing it
    /// again only where its [`ResidentDisplayKey`] changed. `inputs` and
    /// `prepared` are the same panels in the same order. All composites drawn
    /// here go out in one submission, ordered before the host's pass.
    fn attach_resident_displays(
        &mut self,
        inputs: &[ChartDrawItem<'_>],
        prepared: &mut [PreparedItem],
    ) {
        self.resident_displays
            .retain(|display| display.view.strong_count() > 0);
        let max = self.caps.max_texture_dimension_2d;
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        for (input, item) in inputs.iter().zip(prepared.iter_mut()) {
            if item.stream_display.is_some()
                || item.series.is_empty()
                || !input.chart_config.draw_style.is_precise()
            {
                continue;
            }
            let panel = item.view.panel_rect;
            let (w, h) = (panel.width, panel.height);
            let (Some(data_x), Some(data_y)) = (
                item.data_area.x.checked_sub(panel.x),
                item.data_area.y.checked_sub(panel.y),
            ) else {
                continue;
            };
            if w == 0 || h == 0 || w > max || h > max || data_x >= w || data_y >= h {
                continue;
            }
            let data_area = Rect {
                x: data_x,
                y: data_y,
                width: item.data_area.width,
                height: item.data_area.height,
            };
            let probe = ResidentDisplayProbe {
                size: (w, h),
                data_area,
                transform: data_render::scatter_transform_from_config(input.chart_config),
                chart_config: input.chart_config,
                series: input.series,
                font_generation: crate::text_render::font_generation(),
                columns: self.panel_columns(item.column_fingerprint, input.series),
                pool_layout_generation: self.pool.layout_generation(),
                target_pipeline_generation: self.target_pipeline_generation,
            };
            let view_id = Arc::as_ptr(&item.view.content_revision) as usize;
            let slot = self
                .resident_displays
                .iter()
                .position(|display| display.view.as_ptr() as usize == view_id);
            let current = slot.filter(|&i| self.resident_displays[i].key.matches(&probe));
            let index = match current {
                Some(i) => i,
                None => {
                    let Some(serial) = self.resident_display_serial.checked_add(1) else {
                        continue;
                    };
                    *self.resident_display_serial = serial;
                    let key = ResidentDisplayKey::from_probe(&probe);
                    let reusable = slot.filter(|&i| {
                        let display = &self.resident_displays[i];
                        display.texture.width() == w
                            && display.texture.height() == h
                            && display.format == self.surface_format
                            && display.sample_count == self.target_sample_count
                    });
                    let i = match reusable {
                        Some(i) => {
                            let display = &mut self.resident_displays[i];
                            display.key = key;
                            display.serial = serial;
                            i
                        }
                        None => {
                            let display = self.create_resident_display(
                                Arc::downgrade(&item.view.content_revision),
                                key,
                                serial,
                                w,
                                h,
                            );
                            match slot {
                                Some(i) => {
                                    self.resident_displays[i] = display;
                                    i
                                }
                                None => {
                                    self.resident_displays.push(display);
                                    self.resident_displays.len() - 1
                                }
                            }
                        }
                    };
                    let display = &self.resident_displays[i];
                    let target = display.texture.create_view(&Default::default());
                    let msaa = display
                        .msaa
                        .as_ref()
                        .map(|texture| texture.create_view(&Default::default()));
                    let encoder = encoder.get_or_insert_with(|| {
                        self.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("resident panel composite"),
                            })
                    });
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("resident panel composite"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: msaa.as_ref().unwrap_or(&target),
                            depth_slice: None,
                            resolve_target: msaa.as_ref().map(|_| &target),
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: if msaa.is_some() {
                                    wgpu::StoreOp::Discard
                                } else {
                                    wgpu::StoreOp::Store
                                },
                            },
                        })],
                        ..Default::default()
                    });
                    let series_list: Vec<_> =
                        item.series.iter().map(PreparedSeries::layers).collect();
                    pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
                    pass.set_scissor_rect(
                        data_area.x,
                        data_area.y,
                        data_area.width.min(w - data_area.x),
                        data_area.height.min(h - data_area.y),
                    );
                    data_render::issue_series_bodies(&mut pass, &series_list);
                    i
                }
            };
            let display = &self.resident_displays[index];
            item.resident_display = Some(ResidentDisplayPacket {
                view: view_id,
                serial: display.serial,
                bind_group: display.bind_group.clone(),
                _charge: display.texture.shared_charge(),
            });
        }
        if let Some(encoder) = encoder {
            self.queue.submit([encoder.finish()]);
        }
    }

    /// `looked_up` plus the grid columns of the panel's field, contour and
    /// vector series, from the receipts their scratches were built against.
    fn panel_columns(&self, looked_up: u64, series: &[Series<'_>]) -> u64 {
        let receipt = |sources: &GridSources| {
            column_fingerprint(
                sources
                    .ids
                    .iter()
                    .zip(&sources.epochs)
                    .map(|(id, epoch)| (id.as_str(), *epoch)),
            )
        };
        series.iter().fold(looked_up, |sum, series| {
            let id = series.config.series_id.as_str();
            [
                self.field_cache.get(id).map(|scratch| &*scratch.sources),
                self.vector_cache.get(id).map(|scratch| &*scratch.sources),
                self.contour_cache.get(id).map(|entry| &*entry.sources),
            ]
            .into_iter()
            .flatten()
            .fold(sum, |sum, sources| sum.wrapping_add(receipt(sources)))
        })
    }

    /// A panel-sized composite target in the surface format and sample
    /// count, bound for the blit like a raster layer.
    fn create_resident_display(
        &self,
        view: std::sync::Weak<std::sync::atomic::AtomicU64>,
        key: ResidentDisplayKey,
        serial: u64,
        w: u32,
        h: u32,
    ) -> ResidentDisplay {
        let target = |sample_count: u32, usage: wgpu::TextureUsages| {
            TrackedTexture::new(
                self.gpu_ledger,
                if sample_count > 1 {
                    GpuResourceKind::MsaaTarget
                } else {
                    GpuResourceKind::PanelTexture
                },
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("resident panel composite"),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.surface_format,
                    usage,
                    view_formats: &[],
                }),
            )
        };
        let texture = target(
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let msaa = (self.target_sample_count > 1).then(|| {
            target(
                self.target_sample_count,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let bind_group = data_render::create_texture_bind_group(
            self.device,
            self.texture_bgl,
            &texture.create_view(&Default::default()),
            self.sampler,
        );
        ResidentDisplay {
            view,
            key,
            serial,
            format: self.surface_format,
            sample_count: self.target_sample_count,
            texture,
            msaa,
            bind_group,
        }
    }

    /// Per-item body of the prepare phase (also reused by export). For each
    /// panel: computes ONE transform snapshot from `item.chart_config`,
    /// writes it to the panel's transform uniform, dispatches the arc-length
    /// scan of every line series that needs one with that same snapshot —
    /// dashed lines (dash phase) and every line of a style with
    /// `needs_arc_prefix` (sketch: the wobble is parameterized by arc
    /// length, so solid sketch lines need the prefix too). The immutable draw
    /// phase later reads the owned result, never the live `arc_cache`.
    /// Resolve every field series' group-2 state, reusing what has not changed.
    ///
    /// The prepare-phase `&mut self` half of the field path, next to
    /// `prepare_arc_items` and for the same reason: `build_series_layers` is
    /// `&self`, so anything that has to be built has to be built here. Returns
    /// the per-field source receipts in `items` order for the token to hold.
    fn ensure_value_map_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<()> {
        for item in items {
            if !item.chart_config.draw_style.is_precise() {
                // Stylized draw styles ignore per-point style mappings, value
                // maps included, exactly as the style table is ignored.
                continue;
            }
            for series in item.series {
                self.ensure_value_map_scratch(item.chart_config, series.config, scale)?;
            }
        }
        Ok(())
    }

    /// Build or reuse one scatter series' value-map bind group (group 3).
    ///
    /// The value-map twin of `ensure_field_scratch`: the colourbar lives in
    /// the chart `Config`, not in the series, so it has to be read here where
    /// both are in hand, and `build_series_layers` is `&self`. Everything the
    /// shader derives per point — the colour from the colourbar ramp, the radius
    /// from the size domain — is computed on every draw from the value columns;
    /// this uploads only the parameters and the ramp's control points, and only
    /// when the signature changed. `scale` is the export DPI multiplier applied
    /// to the size map's pixel range.
    fn ensure_value_map_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<()> {
        let Some(scatter) = extract_scatter(&cfg.render_type) else {
            return Ok(());
        };
        if scatter.point_color_map.is_none() && scatter.point_size_map.is_none() {
            return Ok(());
        }
        let pair = |value: f64| {
            let (hi, lo) = crate::data::split_f64_to_f32_pair(value);
            [hi, lo]
        };
        let mut params = data_render::PointValueMapParamsGpu::NEUTRAL;
        let mut colormap = None;
        if scatter.point_color_map.is_some() {
            let bar =
                chart_config
                    .colorbar
                    .as_ref()
                    .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                        series_id: cfg.series_id.clone(),
                        reason: "a colour-mapped scatter needs Config.colorbar: it owns the z \
                                 range and colormap"
                            .into(),
                    })?;
            let colorbar = colorbar_gpu(bar);
            params.z_min = colorbar.z_min;
            params.z_max = colorbar.z_max;
            params.flags |= data_render::VALUE_MAP_COLOR;
            if colorbar.log_z {
                params.flags |= data_render::VALUE_MAP_LOG_Z;
            }
            params.stop_count = u32::try_from(bar.colormap.stops().len()).unwrap_or(u32::MAX);
            params.nan_color_premul = colorbar.nan_color_premul;
            colormap = Some(bar.colormap.clone());
        }
        if let Some(map) = scatter.point_size_map.as_ref() {
            map.validate()
                .map_err(|reason| FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    reason: reason.into(),
                })?;
            params.size_domain_min = pair(map.domain_min);
            params.size_domain_max = pair(map.domain_max);
            params.size_px = [map.min_px * scale, map.max_px * scale];
            params.flags |= data_render::VALUE_MAP_SIZE;
            if matches!(map.scale, SizeScale::Area) {
                params.flags |= data_render::VALUE_MAP_SIZE_AREA;
            }
        }
        let signature = ValueMapSignature { params, colormap };
        if let Some(scratch) = self.value_map_cache.get(cfg.series_id.as_str())
            && scratch.signature == signature
        {
            return Ok(());
        }
        let stops: &[Color] = signature
            .colormap
            .as_ref()
            .map_or(&[], |colormap| colormap.stops());
        let (bind_group, charge) = data_render::create_point_value_map_bind_group(
            &self.device,
            &self.gpu_ledger,
            &self.point_value_map_bgl,
            &signature.params,
            stops,
        );
        evict_if_full(
            &mut self.value_map_cache,
            cfg.series_id.as_str(),
            VALUE_MAP_SCRATCH_CACHE_LIMIT,
        );
        self.value_map_cache
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "point value map scratch cache",
                reason: error.to_string(),
            })?;
        self.value_map_cache.insert(
            cfg.series_id.clone(),
            ValueMapScratch {
                signature,
                bind_group,
                _charge: charge,
            },
        );
        Ok(())
    }

    /// Prepare every vector series' group-2 state, returning the column
    /// receipts for the token the way `ensure_field_scratches` does.
    fn ensure_vector_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<Vec<Arc<GridSources>>> {
        let mut receipts = Vec::new();
        for item in items {
            for series in item.series {
                let cfg = series.config;
                if !effective_series_primitives(&item.chart_config.draw_style, &cfg.render_type)
                    .vector
                {
                    continue;
                }
                let Some(sources) = self.ensure_vector_scratch(item.chart_config, cfg, scale)?
                else {
                    continue;
                };
                receipts
                    .try_reserve(1)
                    .map_err(|error| FiggyError::StateAllocationFailed {
                        resource: "vector source receipts",
                        reason: error.to_string(),
                    })?;
                receipts.push(sources);
            }
        }
        Ok(receipts)
    }

    /// Build or reuse one vector series' group-2 bind group (and, for the grid
    /// form, the field bind group its cell highlight uses).
    ///
    /// Same contract as `ensure_field_scratch`: the signature is compared
    /// before anything is created, so a steady frame uploads nothing, and a
    /// live token holds its own refcounted clones. `scale` is the export DPI
    /// multiplier applied to every pixel size, like `point_radius_px`.
    fn ensure_vector_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<Option<Arc<GridSources>>> {
        use crate::data_config::{VectorColorMap, VectorComponents, VectorPivot, VectorScale};
        use data_render::vector as vector_render;
        let Some((components, style)) = extract_vector(&cfg.render_type) else {
            return Ok(None);
        };
        let invalid = |reason: String| FiggyError::InvalidSeriesConfig {
            series_id: cfg.series_id.clone(),
            reason,
        };
        style.validate().map_err(|reason| invalid(reason.into()))?;
        let alloc = |resource: &'static str| {
            move |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource,
                reason: error.to_string(),
            }
        };
        let handle = |id: &str| {
            self.pool
                .handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })
        };
        let epoch = |id: &str| {
            self.pool
                .allocation_epoch(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })
        };
        let lane = |handle: &ColumnHandle| {
            u32::try_from(handle.offset / 4)
                .map_err(|_| invalid("column offset exceeds a 32-bit lane index".into()))
        };
        let len32 = |handle: &ColumnHandle| u32::try_from(handle.len_values).unwrap_or(u32::MAX);

        let x = handle(&cfg.x_column)?;
        let y = handle(&cfg.y_column)?;
        let mut params = vector_render::VectorParamsGpu {
            x_base: lane(&x)?,
            y_base: lane(&y)?,
            x_len: len32(&x),
            y_len: len32(&y),
            u_base: 0,
            u_len: 0,
            v_base: 0,
            v_len: 0,
            c_base: 0,
            c_len: 0,
            cols: 0,
            rows: 0,
            flags: 0,
            count: 0,
            stop_count: 0,
            pivot: match style.pivot {
                VectorPivot::Tail => vector_render::VECTOR_PIVOT_TAIL,
                VectorPivot::Middle => vector_render::VECTOR_PIVOT_MIDDLE,
                VectorPivot::Tip => vector_render::VECTOR_PIVOT_TIP,
            },
            scale: 0.0,
            width_px: style.width_px * scale,
            head_length_px: style.head_length_px * scale,
            head_width_px: style.head_width_px * scale,
            min_length_px: style.min_length_px * scale,
            aa_px: scale,
            pad0: 0.0,
            pad1: 0.0,
            z_min: [0.0; 2],
            z_max: [0.0; 2],
            nan_color_premul: [0.0; 4],
        };
        match style.scale {
            VectorScale::Data { factor } => params.scale = factor as f32,
            VectorScale::Pixels { px_per_unit } => {
                params.scale = px_per_unit as f32 * scale;
                params.flags |= vector_render::VECTOR_FLAG_SCALE_PIXELS;
            }
        }

        let mut grid = Vec::new();
        let mut ids = Vec::new();
        let mut epochs = Vec::new();
        // Grid form: the `u` grid's field params and its table length, for
        // the field bind group the selection reuses.
        let mut field_params = None;
        let input = match components {
            VectorComponents::Points { u, v } => {
                let u_h = handle(u)?;
                let v_h = handle(v)?;
                params.u_base = lane(&u_h)?;
                params.u_len = len32(&u_h);
                params.v_base = lane(&v_h)?;
                params.v_len = len32(&v_h);
                let mut count = x
                    .len_values
                    .min(y.len_values)
                    .min(u_h.len_values)
                    .min(v_h.len_values);
                ids.try_reserve_exact(3)
                    .map_err(alloc("vector source ids"))?;
                epochs
                    .try_reserve_exact(3)
                    .map_err(alloc("vector source epochs"))?;
                ids.push(u.clone());
                epochs.push(epoch(u)?);
                ids.push(v.clone());
                epochs.push(epoch(v)?);
                if let Some(VectorColorMap::Column(column)) = style.color_map.as_ref() {
                    let c_h = handle(column)?;
                    params.c_base = lane(&c_h)?;
                    params.c_len = len32(&c_h);
                    count = count.min(c_h.len_values);
                    ids.push(column.clone());
                    epochs.push(epoch(column)?);
                }
                params.count = u32::try_from(count).unwrap_or(u32::MAX);
                // A storage binding cannot be zero-sized: one padding entry.
                grid.push(data_render::GridColumnGpu { base: 0, len: 0 });
                vector_render::VectorInput::Points
            }
            VectorComponents::Grid { u, v } => {
                let mut extent: Option<(usize, usize)> = None;
                let mut append = |matrix: &crate::data_config::MatrixRef,
                                  grid: &mut Vec<data_render::GridColumnGpu>,
                                  ids: &mut Vec<ColumnId>,
                                  epochs: &mut Vec<u64>|
                 -> Result<(u32, u32, ResolvedGrid)> {
                    let resolved = self.resolve_grid(cfg, matrix)?;
                    let base = u32::try_from(grid.len()).unwrap_or(u32::MAX);
                    let entries = u32::try_from(resolved.grid.len()).unwrap_or(u32::MAX);
                    grid.try_reserve(resolved.grid.len())
                        .map_err(alloc("vector grid table"))?;
                    grid.extend_from_slice(&resolved.grid);
                    ids.try_reserve(matrix.columns.len())
                        .map_err(alloc("vector source ids"))?;
                    ids.extend(matrix.columns.iter().cloned());
                    epochs
                        .try_reserve(resolved.epochs.len())
                        .map_err(alloc("vector source epochs"))?;
                    epochs.extend_from_slice(&resolved.epochs);
                    let cells = (resolved.extent.cols, resolved.extent.rows);
                    extent = Some(match extent {
                        None => cells,
                        Some((cols, rows)) => (cols.min(cells.0), rows.min(cells.1)),
                    });
                    Ok((base, entries, resolved))
                };
                let (u_base, u_len, u_grid) = append(u, &mut grid, &mut ids, &mut epochs)?;
                params.u_base = u_base;
                params.u_len = u_len;
                let (v_base, v_len, _) = append(v, &mut grid, &mut ids, &mut epochs)?;
                params.v_base = v_base;
                params.v_len = v_len;
                if let Some(VectorColorMap::Grid(matrix)) = style.color_map.as_ref() {
                    let (c_base, c_len, _) = append(matrix, &mut grid, &mut ids, &mut epochs)?;
                    params.c_base = c_base;
                    params.c_len = c_len;
                }
                let (cols, rows) = extent.unwrap_or((0, 0));
                params.cols = u32::try_from(cols).unwrap_or(u32::MAX);
                params.rows = u32::try_from(rows).unwrap_or(u32::MAX);
                params.count = cols
                    .checked_mul(rows)
                    .and_then(|count| u32::try_from(count).ok())
                    .ok_or_else(|| {
                        invalid("vector grid cell count exceeds the u32 instance space".into())
                    })?;
                params.flags |= vector_render::VECTOR_FLAG_GRID;
                let mut field_flags = 0u32;
                if u_grid.columns_are_y {
                    params.flags |= vector_render::VECTOR_FLAG_COLUMNS_ARE_Y;
                    field_flags |= data_render::FIELD_FLAG_COLUMNS_ARE_Y;
                }
                if u_grid.centers {
                    params.flags |= vector_render::VECTOR_FLAG_CENTERS;
                    field_flags |= data_render::FIELD_FLAG_CENTERS;
                }
                // The `u` grid's lattice as a field, for the selected-cell
                // outline; that pass reads only the lattice.
                field_params = Some(data_render::FieldParamsGpu::cell_geometry(
                    params.x_base,
                    params.y_base,
                    params.x_len,
                    params.y_len,
                    params.cols,
                    params.rows,
                    field_flags,
                ));
                vector_render::VectorInput::Grid
            }
        };

        let mut stops = Vec::new();
        if style.color_map.is_some() {
            let bar = chart_config.colorbar.as_ref().ok_or_else(|| {
                invalid(
                    "a colour-mapped vector needs Config.colorbar: it owns the z range and \
                     colormap"
                        .into(),
                )
            })?;
            let colorbar = colorbar_gpu(bar);
            params.z_min = colorbar.z_min;
            params.z_max = colorbar.z_max;
            params.flags |= vector_render::VECTOR_FLAG_COLOR_MAP;
            if colorbar.log_z {
                params.flags |= vector_render::VECTOR_FLAG_LOG_Z;
            }
            params.nan_color_premul = colorbar.nan_color_premul;
            let table = colorbar_stops("vector colourmap stops", bar)?;
            stops = table.stops;
            params.stop_count = table.stop_count;
        }
        if stops.is_empty() {
            // No colour map: one padding entry keeps the binding non-empty.
            stops.push([0.0; 4]);
        }

        let signature = VectorSignature {
            pool_layout_generation: self.pool.layout_generation(),
            params,
            grid,
            epochs,
            stops,
            color: [
                style.color.r.to_bits(),
                style.color.g.to_bits(),
                style.color.b.to_bits(),
                style.color.a.to_bits(),
            ],
        };
        if let Some(scratch) = self.vector_cache.get(cfg.series_id.as_str())
            && scratch.signature == signature
        {
            return Ok(Some(Arc::clone(&scratch.sources)));
        }

        let sources = Arc::new(GridSources {
            ids,
            epochs: signature.epochs.clone(),
        });
        let style_buf = data_render::create_style_uniform_buffer(
            &self.device,
            &PrimitiveStyle::from_color_with_width(style.color, style.width_px * scale),
        );
        let style_bg =
            data_render::create_style_bind_group(&self.device, &self.style_bgl, &style_buf);
        let vector_render::VectorDataBindGroup {
            bind_group: vector_bg,
            grid: grid_buffer,
            charge,
        } = vector_render::create_vector_data_bind_group(
            &self.device,
            &self.gpu_ledger,
            &self.vector_bgl,
            self.pool.buffer(),
            vector_render::VectorTables {
                grid: &signature.grid,
                stops: &signature.stops,
                params: &signature.params,
            },
        );
        // The grid table starts with the `u` grid's columns, which is all the
        // field lattice indexes; the same buffer serves both groups.
        let field = field_params.map(|field_params| {
            data_render::create_field_cell_bind_group(
                &self.device,
                &self.gpu_ledger,
                &self.field_bgl,
                self.pool.buffer(),
                &grid_buffer,
                self.field_cell_tables,
                &field_params,
            )
        });

        evict_if_full(
            &mut self.vector_cache,
            cfg.series_id.as_str(),
            VECTOR_SCRATCH_CACHE_LIMIT,
        );
        self.vector_cache
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "vector scratch cache",
                reason: error.to_string(),
            })?;
        let receipt = Arc::clone(&sources);
        let count = signature.params.count;
        self.vector_cache.insert(
            cfg.series_id.clone(),
            VectorScratch {
                signature,
                input,
                style_bg,
                vector_bg,
                charge,
                field,
                sources,
                count,
            },
        );
        Ok(Some(receipt))
    }

    fn ensure_field_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
    ) -> Result<Vec<Arc<GridSources>>> {
        let mut receipts = Vec::new();
        for item in items {
            for series in item.series {
                let cfg = series.config;
                if !effective_series_primitives(&item.chart_config.draw_style, &cfg.render_type)
                    .field
                {
                    continue;
                }
                let sources = self.ensure_field_scratch(item.chart_config, cfg)?;
                receipts
                    .try_reserve(1)
                    .map_err(|error| FiggyError::StateAllocationFailed {
                        resource: "field source receipts",
                        reason: error.to_string(),
                    })?;
                receipts.push(sources);
            }
        }
        Ok(receipts)
    }

    /// A grid declaration resolved against the pool: the coordinate handles, the
    /// `(base, len)` table, the per-column allocation epochs, and the extent.
    ///
    /// Shared by the field and the contour paths so that "what does this
    /// declaration point at right now" has one answer. Both need every part of
    /// it, and a second copy of the resolution is a second place for the two to
    /// disagree about which cell holds which value.
    fn resolve_grid(
        &self,
        cfg: &SeriesConfig,
        matrix: &crate::data_config::MatrixRef,
    ) -> Result<ResolvedGrid> {
        let alloc = |resource: &'static str| {
            move |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource,
                reason: error.to_string(),
            }
        };
        let x = self
            .pool
            .handle_for(&cfg.x_column)
            .ok_or_else(|| FiggyError::UnknownColumn {
                id: cfg.x_column.clone(),
            })?;
        let y = self
            .pool
            .handle_for(&cfg.y_column)
            .ok_or_else(|| FiggyError::UnknownColumn {
                id: cfg.y_column.clone(),
            })?;

        let mut grid = Vec::new();
        grid.try_reserve_exact(matrix.columns.len().max(1))
            .map_err(alloc("field grid table"))?;
        let mut epochs = Vec::new();
        epochs
            .try_reserve_exact(matrix.columns.len())
            .map_err(alloc("field grid epochs"))?;
        let mut lengths = Vec::new();
        lengths
            .try_reserve_exact(matrix.columns.len())
            .map_err(alloc("field grid column lengths"))?;
        for id in &matrix.columns {
            let handle = self
                .pool
                .handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.clone() })?;
            let epoch = self
                .pool
                .allocation_epoch(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.clone() })?;
            lengths.push(handle.len_values);
            epochs.push(epoch);
            grid.push(data_render::GridColumnGpu {
                base: u32::try_from(handle.offset / 4).map_err(|_| {
                    FiggyError::InvalidSeriesConfig {
                        series_id: cfg.series_id.clone(),
                        reason: "grid column offset exceeds a 32-bit lane index".into(),
                    }
                })?,
                len: u32::try_from(handle.len_values).unwrap_or(u32::MAX),
            });
        }
        let mut index = 0usize;
        let extent = matrix_extent(matrix, x.len_values, y.len_values, |_| {
            let len = lengths.get(index).copied().unwrap_or(0);
            index += 1;
            len
        });
        // A storage binding cannot be zero-sized, so an empty table is padded.
        // The real count travels in the params uniform.
        if grid.is_empty() {
            grid.push(data_render::GridColumnGpu { base: 0, len: 0 });
        }
        Ok(ResolvedGrid {
            x,
            y,
            grid,
            epochs,
            extent,
            columns_are_y: matches!(
                matrix.orientation,
                crate::data_config::MatrixOrientation::ColumnsAreY
            ),
            centers: matches!(matrix.grid_layout, crate::data_config::GridLayout::Centers),
        })
    }

    /// The grid columns a resolved declaration read, as a shared receipt.
    fn grid_receipt(
        matrix: &crate::data_config::MatrixRef,
        epochs: &[u64],
    ) -> Result<Arc<GridSources>> {
        let alloc = |resource: &'static str| {
            move |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource,
                reason: error.to_string(),
            }
        };
        let mut ids = Vec::new();
        ids.try_reserve_exact(matrix.columns.len())
            .map_err(alloc("grid source ids"))?;
        ids.extend(matrix.columns.iter().cloned());
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(epochs.len())
            .map_err(alloc("grid source epochs"))?;
        owned.extend_from_slice(epochs);
        Ok(Arc::new(GridSources { ids, epochs: owned }))
    }

    /// Build or reuse one field series' group-2 bind group.
    ///
    /// The signature is compared before anything is created, so a steady frame
    /// uploads nothing at all. When it does differ, the new buffers replace the
    /// old ones outright — a live `PreparedFrame` holds its own refcounted clone
    /// of the bind group, so nothing it can still draw with is disturbed. That is
    /// why this needs none of `arc_cache`'s immutable keyed results: the field
    /// buffers themselves are already immutable snapshots.
    fn ensure_field_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
    ) -> Result<Arc<GridSources>> {
        let rt = &cfg.render_type;
        let matrix = extract_matrix(rt).expect("a field series has a matrix");
        let fill = extract_field_fill(rt).expect("a field series has a fill config");
        validate_contour_level_count(cfg.series_id.as_str(), rt)?;
        let bar =
            chart_config
                .colorbar
                .as_ref()
                .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    reason: "a field series needs Config.colorbar".into(),
                })?;
        let resolved = self.resolve_grid(cfg, matrix)?;

        let mut flags = 0u32;
        if resolved.columns_are_y {
            flags |= data_render::FIELD_FLAG_COLUMNS_ARE_Y;
        }
        if resolved.centers {
            flags |= data_render::FIELD_FLAG_CENTERS;
        }
        let interpolated = matches!(fill.shading, crate::data_config::Shading::Interpolated);
        if interpolated {
            flags |= data_render::FIELD_FLAG_INTERPOLATED;
        }
        if matches!(fill.mode, crate::data_config::FillMode::Bands) {
            flags |= data_render::FIELD_FLAG_BANDS;
        }
        let colorbar = colorbar_gpu(bar);
        if colorbar.log_z {
            flags |= data_render::FIELD_FLAG_LOG_Z;
        }

        let levels = contour_levels(rt)?;
        let level_count = u32::try_from(declared_level_count(rt)).unwrap_or(u32::MAX);
        let ColorbarStops { stops, stop_count } = colorbar_stops("field colourmap stops", bar)?;

        let params = data_render::FieldParamsGpu {
            x_base: u32::try_from(resolved.x.offset / 4).unwrap_or(0),
            y_base: u32::try_from(resolved.y.offset / 4).unwrap_or(0),
            x_len: u32::try_from(resolved.x.len_values).unwrap_or(u32::MAX),
            y_len: u32::try_from(resolved.y.len_values).unwrap_or(u32::MAX),
            cols: u32::try_from(resolved.extent.cols).unwrap_or(u32::MAX),
            rows: u32::try_from(resolved.extent.rows).unwrap_or(u32::MAX),
            level_count,
            stop_count,
            flags,
            opacity: fill.opacity.clamp(0.0, 1.0),
            // Contour-only; a fill draws no strokes and reads no level colours.
            line_width_px: 0.0,
            level_color_count: 0,
            z_min: colorbar.z_min,
            z_max: colorbar.z_max,
        };
        let nan = bar.nan_color;
        let signature = FieldSignature {
            pool_layout_generation: self.pool.layout_generation(),
            params,
            grid: resolved.grid,
            epochs: resolved.epochs,
            levels,
            stops,
            // Bits, not floats: the signature has to be comparable, and two
            // NaN alphas that differ in bits are two different configs.
            nan_color: [
                nan.r.to_bits(),
                nan.g.to_bits(),
                nan.b.to_bits(),
                nan.a.to_bits(),
            ],
        };

        if let Some(scratch) = self.field_cache.get(cfg.series_id.as_str())
            && scratch.signature == signature
        {
            return Ok(Arc::clone(&scratch.sources));
        }

        // Interpolated shading spans point-to-point, so it needs two samples
        // along each axis before it has a quad to draw.
        let minimum = if interpolated { 2 } else { 1 };
        let drawable = resolved.extent.cols >= minimum && resolved.extent.rows >= minimum;
        let sources = Self::grid_receipt(matrix, &signature.epochs)?;

        let nan_buf = data_render::create_style_uniform_buffer(
            &self.device,
            &PrimitiveStyle::from_color(bar.nan_color),
        );
        let style_bg =
            data_render::create_style_bind_group(&self.device, &self.style_bgl, &nan_buf);
        let lookup_tables = data_render::build_field_lookup_tables(
            &signature.stops,
            &signature.levels[..level_count as usize],
        )?;
        let (field_bg, charge) = data_render::create_field_data_bind_group(
            &self.device,
            &self.gpu_ledger,
            &self.field_bgl,
            self.pool.buffer(),
            data_render::FieldTables {
                grid: &signature.grid,
                levels: &signature.levels,
                stops: &lookup_tables.stops,
                // Fill-only bind group: the stroke colour table is padding.
                level_colors: &[[0.0; 4]],
                lookup_metadata: &lookup_tables.metadata,
                params: &signature.params,
            },
        );

        // Bounded like `arc_cache`: a host cycling through thousands of series
        // ids must not grow this without limit.
        evict_if_full(
            &mut self.field_cache,
            cfg.series_id.as_str(),
            FIELD_SCRATCH_CACHE_LIMIT,
        );
        self.field_cache
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "field scratch cache",
                reason: error.to_string(),
            })?;
        let receipt = Arc::clone(&sources);
        self.field_cache.insert(
            cfg.series_id.clone(),
            FieldScratch {
                signature,
                style_bg,
                field_bg,
                charge,
                sources,
                drawable,
            },
        );
        Ok(receipt)
    }

    /// Prepare every contour series' reusable GPU state.
    ///
    /// Returns the grid receipts for the token, the same way
    /// `ensure_field_scratches` does. Automatic label anchors are the only
    /// contour compute dispatch and remain frame-dependent.
    /// `scale` is the export DPI multiplier — the same one
    /// `create_style_for_series_scaled` applies to every other pixel dimension.
    /// It rides in the signature, so switching between a window frame and an
    /// export rebuilds the level styles rather than drawing one at the other's
    /// stroke width.
    fn ensure_contour_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
        contour_spacings: &[Vec<Option<f32>>],
    ) -> Result<(Vec<PreparedContourItem>, Vec<Arc<GridSources>>)> {
        let mut prepared = Vec::with_capacity(items.len());
        let mut receipts = Vec::new();
        debug_assert_eq!(items.len(), contour_spacings.len());
        for (item, item_spacings) in items.iter().zip(contour_spacings) {
            debug_assert_eq!(item.series.len(), item_spacings.len());
            let transform = data_render::scatter_transform_from_config(item.chart_config);
            let ca = item.chart_config.chart_area.0;
            let da = item
                .chart_config
                .data_area()
                .map(|area| area.0)
                .unwrap_or(ca);
            let mut encoder = None;
            let mut per_series = Vec::with_capacity(item.series.len());
            for (series, effective_spacing_px) in item.series.iter().zip(item_spacings) {
                let cfg = series.config;
                if !effective_series_primitives(&item.chart_config.draw_style, &cfg.render_type)
                    .contour
                {
                    per_series.push(PreparedContourSeries { scratch: None });
                    continue;
                }
                let sources = self.ensure_contour_scratch(cfg, item.chart_config, scale)?;
                receipts
                    .try_reserve(1)
                    .map_err(|error| FiggyError::StateAllocationFailed {
                        resource: "contour source receipts",
                        reason: error.to_string(),
                    })?;
                receipts.push(sources);

                let automatic = self
                    .contour_cache
                    .get(cfg.series_id.as_str())
                    .and_then(|entry| entry.label.as_ref())
                    .is_some_and(crate::gpu_contour::ContourLabelState::is_automatic);
                if automatic && encoder.is_none() {
                    encoder = Some(self.device.create_command_encoder(
                        &wgpu::CommandEncoderDescriptor {
                            label: Some("figgy contour label encoder"),
                        },
                    ));
                }

                let label = self.contour_label_snapshot(
                    cfg.series_id.as_str(),
                    &mut encoder,
                    &transform,
                    (ca, da),
                    *effective_spacing_px,
                );
                let entry = self
                    .contour_cache
                    .get(cfg.series_id.as_str())
                    .expect("contour scratch was just ensured");
                per_series.push(PreparedContourSeries {
                    scratch: Some(PreparedContourScratch {
                        style_bg: entry.style_bg.clone(),
                        field_bg: entry.field_bg.clone(),
                        charge: Arc::clone(&entry.charge),
                        drawable: entry.drawable,
                        label,
                    }),
                });
            }
            if let Some(encoder) = encoder {
                self.queue.submit(std::iter::once(encoder.finish()));
            }
            prepared.push(PreparedContourItem { series: per_series });
        }
        Ok((prepared, receipts))
    }

    /// The label snapshot a contour series draws with at `transform`: the
    /// explicit placement, or the automatic one for this exact transform and
    /// anchor plan (dispatched into `encoder` on a miss, reused on a hit).
    /// `None` without labels. Shared by the draw and the contour pick, which
    /// leaves the same label gaps unpicked.
    fn contour_label_snapshot(
        &mut self,
        series_id: &str,
        encoder: &mut Option<wgpu::CommandEncoder>,
        transform: &data_render::ScatterTransform,
        (ca, da): (Rect, Rect),
        effective_spacing_px: Option<f32>,
    ) -> Option<crate::gpu_contour::ContourLabelSnapshot> {
        let entry = self.contour_cache.get_mut(series_id)?;
        let label = entry.label.as_mut()?;
        if !label.is_automatic() {
            return label.explicit_snapshot();
        }
        let effective_spacing_px =
            effective_spacing_px.expect("automatic contour spacing was preflighted");
        let params = &entry.signature.params;
        let plan = crate::gpu_contour::ContourLabelState::anchor_plan(
            da.width as f32,
            da.height as f32,
            effective_spacing_px,
            params.level_count,
        );
        let encoder = encoder.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy contour label encoder"),
                })
        });
        Some(
            label.dispatch_automatic(
                &self.device,
                &self.queue,
                &self.gpu_ledger,
                encoder,
                self.contour_label_pipelines
                    .as_ref()
                    .expect("contour label pipelines ensured"),
                transform,
                crate::gpu_contour::AnchorParamsGpu {
                    lattice_x: plan.lattice_x,
                    lattice_y: plan.lattice_y,
                    level_count: plan.levels,
                    kept_capacity: crate::gpu_contour::MAX_CONTOUR_LABELS_TOTAL,
                    area_x: ca.x as f32,
                    area_y: ca.y as f32,
                    area_w: ca.width as f32,
                    area_h: ca.height as f32,
                    clip_x: da.x as f32,
                    clip_y: da.y as f32,
                    clip_w: da.width as f32,
                    clip_h: da.height as f32,
                    spacing_px: effective_spacing_px,
                    label_h_px: label.cell_h_px(),
                    label_vertices: crate::gpu_contour::LABEL_VERTICES,
                    _pad0: 0,
                },
                &entry.field_bg,
            ),
        )
    }

    /// The label snapshot a contour pick of `cfg` sees: the one the draw at
    /// `chart_config` (the pick's display configuration) and `scale` paints
    /// its label gaps from, through [`Self::contour_label_snapshot`]. Any
    /// anchor dispatch is submitted before the pick that reads it.
    fn contour_pick_label(
        &mut self,
        cfg: &SeriesConfig,
        chart_config: &Config,
        scale: f32,
    ) -> Result<Option<crate::gpu_contour::ContourLabelSnapshot>> {
        if !series_wants_contour_labels(&cfg.render_type) {
            return Ok(None);
        }
        let spacing =
            preflight_contour_label_spacing(cfg.series_id.as_str(), &cfg.render_type, true, scale)?;
        if self.contour_label_pipelines.is_none() {
            *self.contour_label_pipelines = Some(crate::gpu_contour::ContourLabelPipelines::new(
                &self.device,
                &self.field_bgl,
            ));
        }
        let transform = data_render::scatter_transform_from_config(chart_config);
        let ca = chart_config.chart_area.0;
        let da = chart_config.data_area().map(|area| area.0).unwrap_or(ca);
        let mut encoder = None;
        let label = self.contour_label_snapshot(
            cfg.series_id.as_str(),
            &mut encoder,
            &transform,
            (ca, da),
            spacing,
        );
        if let Some(encoder) = encoder {
            self.queue.submit(std::iter::once(encoder.finish()));
        }
        Ok(label)
    }

    /// Build or reuse one contour series' GPU state.
    ///
    /// Since design B.4.9 this resolves the same group-2 tables a field does: the
    /// isolines are drawn from the grid itself by `fs_contour`, so there is no
    /// trace to dispatch and nothing here depends on the panel transform. The
    /// label anchors do — `ensure_contour_scratches` captures one placement
    /// snapshot per panel/item + series occurrence.
    ///
    /// `scale` is the export DPI multiplier, applied to the stroke width and the
    /// label font exactly as `create_style_for_series_scaled` applies it
    /// everywhere else.
    fn ensure_contour_scratch(
        &mut self,
        cfg: &SeriesConfig,
        chart_config: &Config,
        scale: f32,
    ) -> Result<Arc<GridSources>> {
        let rt = &cfg.render_type;
        let matrix = extract_matrix(rt).expect("a contour series has a matrix");
        let contour = extract_contour(rt).expect("a contour series has a contour config");
        validate_contour_level_count(cfg.series_id.as_str(), rt)?;
        let resolved = self.resolve_grid(cfg, matrix)?;
        let levels = contour_levels(rt)?;
        let level_count = u32::try_from(declared_level_count(rt)).unwrap_or(u32::MAX);

        let mut flags = 0u32;
        if resolved.columns_are_y {
            flags |= data_render::FIELD_FLAG_COLUMNS_ARE_Y;
        }
        if resolved.centers {
            flags |= data_render::FIELD_FLAG_CENTERS;
        }
        // The contour's lattice is the sample-point lattice — that is where z
        // lives, and an isoline does not move because a fill decided to draw flat
        // cells. `fs_contour` asks for `LATTICE_SAMPLES` explicitly, so this flag
        // is redundant for it; set anyway so this bind group's params describe the
        // lattice its shader actually walks.
        flags |= data_render::FIELD_FLAG_INTERPOLATED;

        // Per-level stroke: `per_level_color` when given, the line's own colour
        // otherwise. Colours are not derived from the colormap here — a helper
        // that wants that writes them into the config (design B.3).
        //
        // Premultiplied, because that is what the shader composites and what the
        // blend state expects; storing it as drawn also makes a signature
        // comparison a comparison of pixels.
        let level_colors: Vec<[f32; 4]> = crate::gpu_contour::try_collect(
            "contour level colours",
            (0..level_count as usize).map(|index| {
                let color = contour
                    .per_level_color
                    .as_ref()
                    .and_then(|colors| colors.get(index))
                    .copied()
                    .unwrap_or(contour.line.line_color);
                let a = color.a.clamp(0.0, 1.0);
                [color.r * a, color.g * a, color.b * a, a]
            }),
        )?;

        let params = data_render::FieldParamsGpu {
            x_base: u32::try_from(resolved.x.offset / 4).unwrap_or(0),
            y_base: u32::try_from(resolved.y.offset / 4).unwrap_or(0),
            x_len: u32::try_from(resolved.x.len_values).unwrap_or(u32::MAX),
            y_len: u32::try_from(resolved.y.len_values).unwrap_or(u32::MAX),
            cols: u32::try_from(resolved.extent.cols).unwrap_or(u32::MAX),
            rows: u32::try_from(resolved.extent.rows).unwrap_or(u32::MAX),
            level_count,
            // One padding stop: `fs_contour` never samples the ramp, and a
            // zero-sized storage binding is invalid.
            stop_count: 1,
            flags,
            // Fill-only; the contour pass never reads it.
            opacity: 1.0,
            line_width_px: contour.line.line_width * scale,
            level_color_count: u32::try_from(level_colors.len()).unwrap_or(u32::MAX),
            // Colourbar bounds are the *fill*'s business. Levels are in data
            // units (design B.5), so the lines need no z range at all.
            z_min: [0.0, 0.0],
            z_max: [0.0, 0.0],
        };

        // What the labels are baked from. `spacing_px` is not in here: it is a
        // screen distance the anchor pass reads every frame, so changing it moves
        // the labels without rebaking a single glyph.
        let mut label_sig = None;
        if let Some(labels) = contour.labels.as_ref()
            && labels.visible
            && labels.font_size > 0.0
            && level_count > 0
        {
            let mut anchors = crate::gpu_contour::try_collect(
                "contour label anchor overrides",
                labels.anchors.iter().map(|anchor| {
                    let (x_hi, x_lo) = crate::data::split_f64_to_f32_pair(anchor.x);
                    let (y_hi, y_lo) = crate::data::split_f64_to_f32_pair(anchor.y);
                    crate::gpu_contour::LabelAnchorGpu {
                        x: [x_hi, x_lo],
                        y: [y_hi, y_lo],
                        // A direction, so it needs no pair split.
                        dir: [anchor.tx as f32, anchor.ty as f32],
                        level: anchor.level_index,
                        // Resolved from the baked atlas immediately before the
                        // explicit anchors are uploaded.
                        width_px: 0.0,
                    }
                }),
            )?;
            // An anchor naming a level the declaration no longer has is stale,
            // not fatal: it draws nothing and the rest still do.
            anchors.retain(|anchor| anchor.level < level_count);
            anchors.truncate(crate::gpu_contour::MAX_CONTOUR_LABELS_TOTAL as usize);
            label_sig = Some(ContourLabelSignature {
                texts: axis_render::contour_label_texts(chart_config, labels, &contour.levels),
                font_bits: (labels.font_size * scale).to_bits(),
                color: [
                    labels.color.r.to_bits(),
                    labels.color.g.to_bits(),
                    labels.color.b.to_bits(),
                    labels.color.a.to_bits(),
                ],
                policy: crate::text_render::FontPolicy::for_style(&chart_config.draw_style),
                family: chart_config.bottom_x.label_style.label_font.clone(),
                bg: labels
                    .bg_color
                    .as_ref()
                    .map(|c| [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()]),
                bg_padding_bits: labels.bg_padding_px.to_bits(),
                font_generation: crate::text_render::font_generation(),
                anchors,
            });
        }

        let signature = ContourSignature {
            pool_layout_generation: self.pool.layout_generation(),
            params,
            grid: resolved.grid,
            epochs: resolved.epochs,
            levels,
            level_colors,
            label: label_sig,
        };
        if let Some(entry) = self.contour_cache.get(cfg.series_id.as_str())
            && entry.signature == signature
        {
            return Ok(Arc::clone(&entry.sources));
        }

        // A contour interpolates point-to-point, so it needs two samples along
        // each axis before it has a cell to place a line in.
        let drawable = resolved.extent.cols >= 2 && resolved.extent.rows >= 2;
        let sources = Self::grid_receipt(matrix, &signature.epochs)?;

        // `fs_contour` does not read `Style`, but the layout is shared with the
        // fill and group 1 has to be bound. Transparent, so a future entry point
        // that did read it would draw nothing rather than something arbitrary.
        let style_buf = data_render::create_style_uniform_buffer(
            &self.device,
            &PrimitiveStyle::from_color(Color::new(0.0, 0.0, 0.0, 0.0)),
        );
        let style_bg =
            data_render::create_style_bind_group(&self.device, &self.style_bgl, &style_buf);
        let lookup_tables = data_render::build_field_lookup_tables(
            &[[0.0; 4]],
            &signature.levels[..level_count as usize],
        )?;
        let (field_bg, charge) = data_render::create_field_data_bind_group(
            &self.device,
            &self.gpu_ledger,
            &self.field_bgl,
            self.pool.buffer(),
            data_render::FieldTables {
                grid: &signature.grid,
                levels: &signature.levels,
                stops: &lookup_tables.stops,
                level_colors: if signature.level_colors.is_empty() {
                    &[[0.0; 4]]
                } else {
                    &signature.level_colors
                },
                lookup_metadata: &lookup_tables.metadata,
                params: &signature.params,
            },
        );

        let label = match signature.label.as_ref() {
            Some(label_sig) => self.build_contour_label(
                cfg.series_id.as_str(),
                chart_config,
                contour,
                label_sig,
                scale,
            )?,
            None => None,
        };

        evict_if_full(
            &mut self.contour_cache,
            cfg.series_id.as_str(),
            CONTOUR_SCRATCH_CACHE_LIMIT,
        );
        self.contour_cache
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "contour scratch cache",
                reason: error.to_string(),
            })?;
        let receipt = Arc::clone(&sources);
        self.contour_cache.insert(
            cfg.series_id.clone(),
            ContourScratchEntry {
                signature,
                style_bg,
                field_bg,
                charge,
                sources,
                drawable,
                label,
            },
        );
        Ok(receipt)
    }

    /// Bake one contour series' label atlas and allocate its anchor state.
    ///
    /// `None` when nothing is bakeable because every string is empty. A requested
    /// atlas that cannot fit the device limit is an explicit series error rather
    /// than silently dropping labels.
    fn build_contour_label(
        &mut self,
        series_id: &str,
        chart_config: &Config,
        contour: &crate::data_config::ContourConfig,
        label_sig: &ContourLabelSignature,
        scale: f32,
    ) -> Result<Option<crate::gpu_contour::ContourLabelState>> {
        let Some(labels) = contour.labels.as_ref() else {
            return Ok(None);
        };
        let Some(baked) = axis_render::bake_contour_label_atlas(
            chart_config,
            labels,
            &label_sig.texts,
            scale,
            self.caps.max_texture_dimension_2d,
        )
        .map_err(|error| FiggyError::InvalidSeriesConfig {
            series_id: series_id.to_owned(),
            reason: error.to_string(),
        })?
        else {
            return Ok(None);
        };
        let cell_w = crate::gpu_contour::try_collect(
            "contour label cell widths",
            baked.cells.iter().map(|cell| cell.width),
        )?;
        if self.contour_label_pipelines.is_none() {
            *self.contour_label_pipelines = Some(crate::gpu_contour::ContourLabelPipelines::new(
                &self.device,
                &self.field_bgl,
            ));
        }
        let pipelines = self
            .contour_label_pipelines
            .as_ref()
            .expect("contour label pipelines ensured");
        let state = crate::gpu_contour::ContourLabelState::build(
            &self.device,
            &self.queue,
            &self.gpu_ledger,
            pipelines,
            &self.sampler,
            self.caps.max_texture_dimension_2d,
            crate::gpu_contour::LabelAtlas {
                rgba: &baked.rgba,
                width: baked.width,
                height: baked.height,
                cell_h: baked.cell_h,
                cell_stride_w: baked.cell_stride_w,
                cell_stride_h: baked.cell_stride_h,
                columns: baked.columns,
                rows: baked.rows,
                gutter: baked.gutter,
                cell_w: &cell_w,
            },
            (!label_sig.anchors.is_empty()).then_some(label_sig.anchors.as_slice()),
        )?;
        Ok(Some(state))
    }

    /// Compile the label shader's module and layouts if any item wants labels.
    ///
    /// Separate from `ensure_precise_variants_for_items` because that function
    /// may only *use* them: it is the single site allowed to compile a target
    /// pipeline (design B.6), and it has no `&mut self` to create the module
    /// with.
    fn ensure_contour_label_pipelines(&mut self, items: &[ChartDrawItem<'_>]) {
        if self.contour_label_pipelines.is_some() {
            return;
        }
        let wanted = items.iter().any(|item| {
            item.series.iter().any(|series| {
                effective_series_primitives(
                    &item.chart_config.draw_style,
                    &series.config.render_type,
                )
                .contour
                    && series_wants_contour_labels(&series.config.render_type)
            })
        });
        if wanted {
            *self.contour_label_pipelines = Some(crate::gpu_contour::ContourLabelPipelines::new(
                &self.device,
                &self.field_bgl,
            ));
        }
    }

    fn prepare_arc_items(&mut self, items: &[ChartDrawItem<'_>]) -> Result<Vec<PreparedArcItem>> {
        let mut prepared = Vec::with_capacity(items.len());
        for item in items {
            let t = data_render::scatter_transform_from_config(item.chart_config);
            // Same snapshot for both GPU consumers of the transform: the
            // uniform the vertex shaders read and the arc dispatch below.
            // Written here (not via `update_transform`) so they cannot
            // desync within a frame.
            item.view.advance_stream_revision()?;
            let view_revision = item.view.advance_content_revision()?;
            data_render::update_scatter_transform(&self.queue, &item.view.transform_buffer, &t);
            let variant = style_variant(&item.chart_config.draw_style);
            // Milkyway lines also get the arc-driven star pass: the
            // candidate-slot pitch the indirect kernel sizes the dispatch
            // with (the star VS derives the same value from the transform).
            let star_pitch = match &item.chart_config.draw_style {
                crate::config::DrawStyle::Milkyway(c) => Some(
                    (data_render::line_arc::STAR_SLOT_PITCH_FACTOR * c.structure_scale).max(1e-3),
                ),
                _ => None,
            };
            let mut per_series = Vec::with_capacity(item.series.len());
            for series in item.series {
                let cfg = series.config;
                let line =
                    effective_series_primitives(&item.chart_config.draw_style, &cfg.render_type)
                        .line;
                let dashed = line
                    && extract_line(&cfg.render_type)
                        .is_some_and(|l| !matches!(l.line_style, LineStylePreset::Solid));
                let arc_wanted = dashed || (variant.is_some_and(|v| v.needs_arc_prefix) && line);
                let arc = if arc_wanted {
                    self.ensure_arc_prefix(
                        &cfg.series_id,
                        &cfg.x_column,
                        &cfg.y_column,
                        &t,
                        if line { star_pitch } else { None },
                        extract_line(&cfg.render_type)
                            .map_or(LineInterpolation::Linear, |l| l.interpolation),
                    )
                } else {
                    None
                };
                per_series.push(PreparedArcSeries { arc });
            }
            prepared.push(PreparedArcItem {
                view_revision,
                series: per_series,
            });
        }
        Ok(prepared)
    }

    fn resolve_prepared_items(
        &self,
        items: &[ChartDrawItem<'_>],
        prepared_arcs: &[PreparedArcItem],
        prepared_contours: &[PreparedContourItem],
        pipelines: &TargetPipelines,
    ) -> Result<(Vec<PreparedItem>, HashMap<ColumnId, u64>)> {
        let mut prepared = Vec::with_capacity(items.len());
        let mut column_sources = HashMap::new();

        for ((item, prepared_arc), prepared_contour) in
            items.iter().zip(prepared_arcs).zip(prepared_contours)
        {
            let styled = pipelines.style_set(&item.chart_config.draw_style);
            let products = PreparedSeriesProducts {
                arcs: &prepared_arc.series,
                contours: &prepared_contour.series,
            };
            let (layers, sources) = self.build_series_layers(
                item.view,
                item.chart_config,
                item.series,
                pipelines,
                products,
                styled,
            )?;

            let column_fingerprint =
                column_fingerprint(sources.iter().map(|(id, epoch)| (id.as_str(), *epoch)));
            for (id, allocation_epoch) in sources {
                column_sources.entry(id).or_insert(allocation_epoch);
            }

            prepared.push(PreparedItem {
                view: PreparedView::capture(
                    item.view,
                    prepared_arc.view_revision,
                    item.chart_config.annotations.visible,
                ),
                data_area: item
                    .chart_config
                    .data_area()
                    .map(|area| area.0)
                    .unwrap_or(item.view.panel_rect),
                axis_pipeline: pipelines.axis.clone(),
                series: layers
                    .into_iter()
                    .zip(&prepared_arc.series)
                    .zip(item.series)
                    .map(|((layers, arc), series)| {
                        PreparedSeries::from_layers(
                            layers,
                            arc.arc.as_ref().map(|arc| Arc::clone(&arc.charge)),
                            Some(Arc::clone(&series.style._charge)),
                        )
                    })
                    .collect(),
                stream_display: None,
                resident_display: None,
                column_fingerprint,
            });
        }
        Ok((prepared, column_sources))
    }

    /// Convert one panel's `Series` list into `SeriesLayers` ready for
    /// drawing. The `config.render_type` enum variant decides which of
    /// line/scatter/errorbar each series needs; column ids are resolved to
    /// handles via the pool. `prepared` supplies prepare-time arc and star
    /// products without a live `arc_cache` read. `styled` is the panel's
    /// resolved [`StyleSet`] — it selects the stylized pipeline variants
    /// (and the line strip's vertex count); `None` (precise mode, or the
    /// theoretically-impossible cache miss) selects the precise pipelines.
    /// Everything else is identical between the modes.
    fn build_series_layers<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series_specs: &[Series<'a>],
        pipelines: &'a TargetPipelines,
        products: PreparedSeriesProducts<'a>,
        styled: Option<&'a StyleSet>,
    ) -> Result<(Vec<data_render::SeriesLayers<'a>>, HashMap<ColumnId, u64>)> {
        let pool = &self.pool;
        let mut column_sources = HashMap::new();
        let mut lookup = |id: &str| -> Result<ColumnHandle> {
            let handle = pool
                .handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
            let allocation_epoch = pool
                .allocation_epoch(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
            column_sources.entry(id.into()).or_insert(allocation_epoch);
            Ok(handle)
        };

        let layers = self.build_series_layers_with_columns(
            view,
            chart_config,
            series_specs,
            pipelines,
            products,
            styled,
            pool.buffer(),
            &mut lookup,
            None,
        )?;
        Ok((layers, column_sources))
    }

    /// A single exact pass; composite series and global-context primitives must
    /// be scheduled separately rather than repeated once per source chunk.
    fn validate_stream_series(config: &Config, series: &SeriesConfig) -> Result<()> {
        let unsupported = || FiggyError::InvalidSeriesConfig {
            series_id: series.series_id.clone(),
            reason: "stream pass does not yet support contours or arc-driven stars".into(),
        };
        let primitives = effective_series_primitives(&config.draw_style, &series.render_type);
        if primitives.contour
            || (primitives.field && !streaming_runtime::is_stream_heatmap(&series.render_type))
            || (primitives.line && matches!(config.draw_style, DrawStyle::Milkyway(_)))
        {
            return Err(unsupported());
        }
        // Streaming is not extended to polar charts.
        if config.polar.is_some() {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "polar charts require GPU-resident columns; streamed sources are \
                         not supported"
                    .into(),
            });
        }
        // A fill between two series needs index-aligned chunks of two streams,
        // which nonresident streaming cannot guarantee, and a stepped line's
        // arc prefix is not part of the streamed canonical scan. Both are
        // resident-only by design, rejected here rather than drawn wrong.
        if primitives.line
            && let Some(line) = extract_line(&series.render_type)
            && (line.fill.is_some() || !line.interpolation.is_linear())
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "line fill and step interpolation require GPU-resident columns; \
                         streamed sources are not supported"
                    .into(),
            });
        }
        // Streaming is not extended to vector series: their shader reads the
        // resident pool as storage.
        if has_vector(&series.render_type) {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "vector series require GPU-resident columns; streamed sources are \
                         not supported"
                    .into(),
            });
        }
        // The value maps read index-aligned columns on every draw; streaming
        // is not extended to them.
        if let Some(scatter) = extract_scatter(&series.render_type)
            && (scatter.point_color_map.is_some() || scatter.point_size_map.is_some())
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "point colour and size maps require GPU-resident columns; streamed \
                         sources are not supported"
                    .into(),
            });
        }
        Ok(())
    }

    #[cfg(test)]
    fn build_stream_series(
        &self,
        view: &ChartView,
        config: &Config,
        series: &Series<'_>,
        chunk: &crate::streaming_upload::RecordedChunk,
        columns: &[crate::streaming::ColumnRange],
        phase: streaming_runtime::StreamDrawPhase,
    ) -> Result<PreparedSeries> {
        self.build_stream_series_with_pipelines(
            view,
            config,
            series,
            chunk,
            columns,
            phase,
            self.pipelines,
        )
    }

    fn build_stream_arc_series(
        &self,
        view: &ChartView,
        config: &Config,
        series: &Series<'_>,
        pool: &wgpu::Buffer,
        x: ColumnHandle,
        y: ColumnHandle,
        arc: ArcPrefix,
        charge: crate::gpu_memory::SharedCharge,
        pipelines: &TargetPipelines,
    ) -> Result<PreparedSeries> {
        Self::validate_stream_series(config, series.config)?;
        let specs = [Series {
            config: series.config,
            style: series.style,
        }];
        let mut layers = self.build_series_layers_with_columns(
            view,
            config,
            &specs,
            pipelines,
            PreparedSeriesProducts {
                arcs: &[],
                contours: &[],
            },
            pipelines.style_set(&config.draw_style),
            pool,
            |id| {
                if id == series.config.x_column {
                    Ok(x)
                } else if id == series.config.y_column {
                    Ok(y)
                } else {
                    Err(FiggyError::UnknownColumn { id: id.into() })
                }
            },
            Some(streaming_runtime::StreamDrawPhase::Line),
        )?;
        let mut packet = PreparedSeries::from_layers(
            layers.remove(0),
            Some(charge),
            Some(Arc::clone(&series.style._charge)),
        );
        if let Some(line) = &mut packet.line {
            line.arc = Some(arc);
        }
        Ok(packet)
    }

    fn build_stream_series_with_pipelines(
        &self,
        view: &ChartView,
        config: &Config,
        series: &Series<'_>,
        chunk: &crate::streaming_upload::RecordedChunk,
        columns: &[crate::streaming::ColumnRange],
        phase: streaming_runtime::StreamDrawPhase,
        pipelines: &TargetPipelines,
    ) -> Result<PreparedSeries> {
        Self::validate_stream_series(config, series.config)?;
        if phase == streaming_runtime::StreamDrawPhase::Field {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed field requires the bounded tile executor".into(),
            });
        }
        if phase == streaming_runtime::StreamDrawPhase::Line
            && extract_line(&series.config.render_type).is_some_and(|line| {
                !matches!(line.line_style, LineStylePreset::Solid)
                    || style_variant(&config.draw_style)
                        .is_some_and(|variant| variant.needs_arc_prefix)
            })
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed line requires a prepared canonical arc prefix".into(),
            });
        }
        let expected = streaming_runtime::stream_phase_columns(series.config, phase);
        let first = columns
            .first()
            .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed primitive has no supplied columns".into(),
            })?;
        let bins = if phase == streaming_runtime::StreamDrawPhase::Histogram {
            if streaming_runtime::histogram_edge_column(series.config)
                == Some(series.config.x_column.as_str())
            {
                first.len.saturating_sub(1)
            } else {
                first.len
            }
        } else {
            first.len.saturating_sub(phase.halo())
        };
        if columns.len() != expected.count
            || columns.iter().enumerate().any(|(index, column)| {
                column.column != index as u64
                    || column.offset != first.offset
                    || column.len
                        != if phase == streaming_runtime::StreamDrawPhase::Histogram {
                            bins.saturating_add(u64::from(
                                streaming_runtime::histogram_edge_column(series.config)
                                    == Some(expected.ids[index]),
                            ))
                        } else {
                            bins.saturating_add(phase.halo())
                        }
            })
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed primitive ranges must describe the same global rows".into(),
            });
        }
        let specs = [Series {
            config: series.config,
            style: series.style,
        }];
        let mut layers = self.build_series_layers_with_columns(
            view,
            config,
            &specs,
            pipelines,
            PreparedSeriesProducts {
                arcs: &[],
                contours: &[],
            },
            pipelines.style_set(&config.draw_style),
            &chunk.work,
            |id| {
                let index = expected.ids[..expected.count]
                    .iter()
                    .position(|column| *column == id)
                    .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
                let range = columns[index];
                chunk
                    .column_handle(range)
                    .map_err(|error| FiggyError::InvalidSeriesConfig {
                        series_id: series.config.series_id.clone(),
                        reason: format!("invalid streamed column span: {error:?}"),
                    })
            },
            Some(phase),
        )?;
        let mut prepared = PreparedSeries::from_layers(
            layers.remove(0),
            None,
            Some(Arc::clone(&series.style._charge)),
        );
        prepared._column_charge = Some(chunk.work.shared_charge());
        Ok(prepared)
    }

    /// The value columns and group-3 bind group a mapped scatter draw binds.
    ///
    /// `None` everywhere when the series draws through the plain pipeline. With
    /// a style map but no value map the renderer's neutral group is bound; with
    /// a value map the columns are looked up and length-checked the way the
    /// style-index column is, and the bind group is the one
    /// `ensure_value_map_scratch` resolved.
    #[allow(clippy::type_complexity)]
    pub(super) fn scatter_value_map_columns<'a>(
        &'a self,
        cfg: &SeriesConfig,
        mapped: bool,
        count: usize,
        lookup: &mut impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<(
        Option<ColumnHandle>,
        Option<ColumnHandle>,
        Option<&'a wgpu::BindGroup>,
    )> {
        if !mapped {
            return Ok((None, None, None));
        }
        let scatter = extract_scatter(&cfg.render_type).expect("a mapped series has points");
        if scatter.point_color_map.is_none() && scatter.point_size_map.is_none() {
            return Ok((None, None, Some(self.neutral_point_value_map_bg)));
        }
        let mut value_column = |column: &ColumnId, what: &str| -> Result<ColumnHandle> {
            let handle = lookup(column)?;
            if handle.len_values < count {
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    reason: format!(
                        "{what} column {column:?} has {} values, but scatter uses {count}",
                        handle.len_values
                    ),
                });
            }
            Ok(handle)
        };
        let color_value = scatter
            .point_color_map
            .as_ref()
            .map(|map| value_column(&map.column, "point colour map"))
            .transpose()?;
        let size_value = scatter
            .point_size_map
            .as_ref()
            .map(|map| value_column(&map.column, "point size map"))
            .transpose()?;
        let scratch = self
            .value_map_cache
            .get(cfg.series_id.as_str())
            .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                series_id: cfg.series_id.clone(),
                reason: "point value maps require GPU-resident columns; streamed sources are \
                         not supported"
                    .into(),
            })?;
        Ok((color_value, size_value, Some(&scratch.bind_group)))
    }

    /// Column locations and their lifetime authority are resolved by the caller;
    /// geometry and style interpretation remain shared with resident rendering.
    #[allow(clippy::too_many_arguments)]
    fn build_series_layers_with_columns<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series_specs: &[Series<'a>],
        pipelines: &'a TargetPipelines,
        products: PreparedSeriesProducts<'a>,
        styled: Option<&'a StyleSet>,
        column_buffer: &'a wgpu::Buffer,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
        phase: Option<streaming_runtime::StreamDrawPhase>,
    ) -> Result<Vec<data_render::SeriesLayers<'a>>> {
        // Resolve the style set into per-primitive picks once. `stars` is the
        // constellation's arc-driven indirect star pass over the same polyline.
        #[derive(Clone, Copy)]
        struct LinePick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            verts: u32,
            texture_bg: Option<&'p wgpu::BindGroup>,
        }
        #[derive(Clone, Copy)]
        struct StarsPick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            texture_bg: &'p wgpu::BindGroup,
        }
        #[derive(Clone, Copy)]
        struct ScatterPick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            texture_bg: Option<&'p wgpu::BindGroup>,
        }
        let (line_pick, stars_pick, scatter_pick, errorbar_pipe) = match styled {
            None => (
                pipelines.line.as_ref().map(|pipeline| LinePick {
                    pipeline,
                    verts: 4,
                    texture_bg: None,
                }),
                None,
                pipelines.scatter.as_ref().map(|pipeline| ScatterPick {
                    pipeline,
                    texture_bg: None,
                }),
                pipelines.errorbar.as_ref(),
            ),
            Some(StyleSet::Sketch {
                line,
                scatter,
                errorbar,
                line_verts,
            }) => (
                Some(LinePick {
                    pipeline: line,
                    verts: *line_verts,
                    texture_bg: None,
                }),
                None,
                Some(ScatterPick {
                    pipeline: scatter,
                    texture_bg: None,
                }),
                Some(errorbar),
            ),
            // Milkyway: line = ribbon + star pass, scatter = ringed
            // planets (premultiplied — they occlude the star field),
            // errorbar = bipolar jets with terminal shock knots.
            Some(StyleSet::Milkyway(c)) => (
                Some(LinePick {
                    pipeline: &c.ribbon,
                    verts: data_render::MILKYWAY_RIBBON_VERTICES,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                Some(StarsPick {
                    pipeline: &c.stars,
                    texture_bg: &c.star_tex_bg,
                }),
                Some(ScatterPick {
                    pipeline: &c.planets,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                Some(&c.jets),
            ),
            // Constellation: only ScatterLine is supported. The line is the
            // regular columnar stroke with style-level alpha; the scatter
            // layer renders PSF stars at the data points.
            Some(StyleSet::Constellation(c)) => (
                Some(LinePick {
                    pipeline: &c.line,
                    verts: 4,
                    texture_bg: None,
                }),
                None,
                Some(ScatterPick {
                    pipeline: &c.stars,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                None,
            ),
        };

        let mut out = Vec::with_capacity(series_specs.len());
        for (idx, series) in series_specs.iter().enumerate() {
            let cfg = series.config;
            let rt = &cfg.render_type;
            let constellation_only = matches!(styled, Some(StyleSet::Constellation(_)));
            let primitives = effective_series_primitives(&chart_config.draw_style, rt);
            let x_h = lookup(&cfg.x_column)?;
            let y_h = lookup(&cfg.y_column)?;

            // A field: the group-2 state was resolved in `ensure_field_scratches`
            // (the prepare phase's `&mut self` half) and is reused whole while
            // nothing it depends on changes.
            let field = if primitives.field {
                let scratch = self
                    .field_cache
                    .get(cfg.series_id.as_str())
                    .expect("prepare ensured this field's scratch");
                Some(data_render::ColumnFieldLayer {
                    pipeline: pipelines
                        .field
                        .as_ref()
                        .expect("prepare ensured field pipeline"),
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    field_bg: scratch.field_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    drawable: scratch.drawable,
                })
            } else {
                None
            };

            // Contour lines: the same quad the field draws, through
            // `fs_contour`. The group-2 tables were resolved in
            // `ensure_contour_scratches`; the isolines come out of the grid
            // itself, so there is nothing per-level about the draw call.
            let contour = if primitives.contour {
                let scratch = products
                    .contours
                    .get(idx)
                    .and_then(|series| series.scratch.as_ref())
                    .expect("prepare captured this contour occurrence");
                let label = scratch
                    .label
                    .as_ref()
                    .zip(pipelines.contour_label.as_ref())
                    .map(|(snapshot, pipeline)| data_render::ColumnContourLabel {
                        pipeline,
                        snapshot: snapshot.clone(),
                    });
                let pipeline = if label.is_some() {
                    pipelines
                        .contour_labelled
                        .as_ref()
                        .expect("prepare ensured labelled contour pipeline")
                } else {
                    pipelines
                        .contour
                        .as_ref()
                        .expect("prepare ensured contour pipeline")
                };
                Some(data_render::ColumnContourLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    field_bg: scratch.field_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    drawable: scratch.drawable,
                    // The label pipeline is compiled only when some series wants
                    // labels, so a `None` here means this series' labels are not
                    // drawable this frame — never that the state is missing.
                    label,
                })
            } else {
                None
            };

            // Arrows: the group-2 state was resolved in `ensure_vector_scratches`
            // and is reused whole while nothing it depends on changes.
            let vector = if primitives.vector && phase.is_none() {
                let scratch = self
                    .vector_cache
                    .get(cfg.series_id.as_str())
                    .expect("prepare ensured this vector's scratch");
                let pipeline = match scratch.input {
                    data_render::vector::VectorInput::Points => pipelines.vector_points.as_ref(),
                    data_render::vector::VectorInput::Grid => pipelines.vector_grid.as_ref(),
                }
                .expect("prepare ensured vector pipeline");
                Some(data_render::vector::ColumnVectorLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    vector_bg: scratch.vector_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    count: scratch.count,
                })
            } else {
                None
            };

            // Bars: the edge column and the value column are picked from the
            // bar's own orientation — never from the column lengths (design
            // §B.3). `Vertical` reads x as edges, `Horizontal` reads y.
            let bar = if primitives.bar
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Histogram)
            {
                let bar_cfg = extract_bar(rt).expect("primitives.bar implies a bar config");
                let (edges, values) = match bar_cfg.orientation {
                    crate::data_config::BarOrientation::Vertical => (x_h, y_h),
                    crate::data_config::BarOrientation::Horizontal => (y_h, x_h),
                };
                let style_map = series.style.bar_map.as_ref();
                Some(data_render::ColumnBarLayer {
                    envelope: phase.is_none().then(|| {
                        pipelines
                            .bar_envelope
                            .as_ref()
                            .expect("prepare ensured histogram envelope pipeline")
                            .prepare(
                                &self.device,
                                &self.queue,
                                &self.gpu_ledger,
                                column_buffer,
                                edges,
                                values,
                                match bar_cfg.orientation {
                                    crate::data_config::BarOrientation::Vertical => {
                                        view.panel_rect.width
                                    }
                                    crate::data_config::BarOrientation::Horizontal => {
                                        view.panel_rect.height
                                    }
                                },
                                &view.transform_bg,
                                &series.style.bar_bg,
                                style_map.map(|map| &map.bind_group),
                            )
                    }),
                    pipeline: style_map.map_or_else(
                        || {
                            pipelines
                                .bar
                                .as_ref()
                                .expect("prepare ensured bar pipeline")
                        },
                        |_| {
                            pipelines
                                .bar_mapped
                                .as_ref()
                                .expect("prepare ensured mapped bar pipeline")
                        },
                    ),
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.bar_bg,
                    style_map_bg: style_map.map(|map| &map.bind_group),
                    pool_buffer: column_buffer,
                    edges,
                    values,
                })
            } else {
                None
            };

            let interpolation =
                extract_line(rt).map_or(LineInterpolation::Linear, |line| line.interpolation);
            let line = if primitives.line
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Line)
            {
                let line_pick = line_pick.expect("prepare ensured line pipeline");
                let arc = products
                    .arcs
                    .get(idx)
                    .and_then(|series| series.arc.as_ref())
                    .map(|arc| arc.prefix.clone());
                Some(ColumnLineLayer {
                    pipeline: line_pick.pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.line_bg,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    arc,
                    // One strip per step piece (or polar arc piece), joined
                    // per `strip_slot`.
                    verts_per_instance: crate::line_step::strip_vertex_count(
                        line_pick.verts,
                        crate::line_step::mode_word(
                            interpolation,
                            crate::line_step::arc_pieces(chart_config.polar.as_ref()),
                        ),
                    ),
                    texture_bg: line_pick.texture_bg,
                })
            } else {
                None
            };

            // The area fill under / beside the line. Resident only: the fill
            // between two series needs index-aligned columns that no streamed
            // chunking can guarantee, and `validate_stream_series` has already
            // rejected a streamed fill before this point. The second y column
            // is the referenced series' own, resolved from the same series
            // list; `validate_line_fill` checked the reference and its x
            // column before any GPU work.
            let area = if line.is_some()
                && phase.is_none()
                && let Some(fill) = extract_line(rt).and_then(|line| line.fill.as_ref())
            {
                let base = match fill {
                    LineFill::ToBaseline(_) => y_h,
                    LineFill::ToSeries(target) => {
                        let target = series_specs
                            .iter()
                            .map(|other| other.config)
                            .find(|other| other.series_id == *target)
                            .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: format!(
                                    "line fill references series `{target}`, which is not in \
                                     this chart"
                                ),
                            })?;
                        if target.x_column != cfg.x_column {
                            return Err(FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: format!(
                                    "line fill to series `{}` requires the same x column: \
                                     `{}` vs `{}`",
                                    target.series_id, cfg.x_column, target.x_column
                                ),
                            });
                        }
                        lookup(&target.y_column)?
                    }
                };
                Some(data_render::ColumnAreaLayer {
                    pipeline: pipelines
                        .area
                        .as_ref()
                        .expect("prepare ensured line area pipeline"),
                    transform_bg: &view.transform_bg,
                    style_bg: series.style.area_bg.as_ref().ok_or_else(|| {
                        FiggyError::InvalidSeriesConfig {
                            series_id: cfg.series_id.clone(),
                            reason: "series style was created without its line fill; \
                                     recreate the style for the current series config"
                                .into(),
                        }
                    })?,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    base,
                    verts_per_instance: crate::line_step::strip_vertex_count(
                        data_render::LINE_PRECISE_VERTICES_PER_INSTANCE,
                        crate::line_step::mode_word(
                            interpolation,
                            crate::line_step::arc_pieces(chart_config.polar.as_ref()),
                        ),
                    ),
                })
            } else {
                None
            };

            // Star pass: per-series GPU state is the owned snapshot the
            // prepare phase cloned out of the arc scratch (indirect args +
            // VS bind group) — read from the token, never from `arc_cache`,
            // so an eviction/rebuild between prepare and paint cannot drop
            // or desync it. A missing star (n < 2, or a
            // theoretically-impossible prepare miss) just skips the stars —
            // the ribbon still draws.
            let line_extra = match (&line, &stars_pick) {
                (Some(_), Some(sp)) => products
                    .arcs
                    .get(idx)
                    .and_then(|series| series.arc.as_ref())
                    .and_then(|a| a.star.as_ref())
                    .map(|star| data_render::ColumnStarLayer {
                        pipeline: sp.pipeline,
                        transform_bg: &view.transform_bg,
                        style_bg: &series.style.line_bg,
                        texture_bg: sp.texture_bg,
                        star_bg: &star.vs_bg,
                        indirect: &star.indirect,
                    }),
                _ => None,
            };

            let scatter = if primitives.scatter
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Scatter)
            {
                let precise_style_map = if styled.is_none() {
                    series.style.scatter_map.as_ref()
                } else {
                    None
                };
                let style_index = match (precise_style_map, extract_scatter(rt)) {
                    (Some(map), Some(sc)) if map.has_index => {
                        let Some(column) = sc.point_style_index_column.as_ref() else {
                            return Err(FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: "scatter style map expects an index column".into(),
                            });
                        };
                        let h = lookup(column)?;
                        let count = x_h.len_values.min(y_h.len_values);
                        if h.len_values < count {
                            return Err(FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: format!(
                                    "style index column {column:?} has {} values, but scatter uses {count}",
                                    h.len_values
                                ),
                            });
                        }
                        Some(h)
                    }
                    _ => None,
                };
                let (color_value, size_value, value_map_bg) = self.scatter_value_map_columns(
                    cfg,
                    precise_style_map.is_some(),
                    x_h.len_values.min(y_h.len_values),
                    &mut lookup,
                )?;
                let scatter_pick = scatter_pick.expect("prepare ensured scatter pipeline");
                Some(ColumnScatterLayer {
                    pipeline: precise_style_map.map_or(scatter_pick.pipeline, |_| {
                        pipelines
                            .scatter_mapped
                            .as_ref()
                            .expect("prepare ensured mapped scatter pipeline")
                    }),
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.scatter_bg,
                    style_map_bg: precise_style_map.map(|m| &m.bind_group),
                    value_map_bg,
                    quad_vb: &self.quad_vb,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    style_index,
                    color_value,
                    size_value,
                    texture_bg: scatter_pick.texture_bg,
                })
            } else {
                None
            };

            let errorbar = if constellation_only
                || phase.is_some_and(|phase| phase != streaming_runtime::StreamDrawPhase::Errorbar)
            {
                None
            } else {
                match (extract_err_y(rt), extract_err_x(rt)) {
                    (None, None) => None,
                    (ey_opt, ex_opt) => {
                        let (ey_lo, ey_hi) = match ey_opt {
                            Some(ErrorRef::Symmetric { column }) => {
                                let h = lookup(column)?;
                                (h, h)
                            }
                            Some(ErrorRef::Asymmetric { lower, upper }) => {
                                (lookup(lower)?, lookup(upper)?)
                            }
                            // The direction flag disables these attributes
                            // before the shader reads them. Reuse the anchor
                            // column so the bound slice still covers every draw.
                            None => (y_h, y_h),
                        };
                        let (ex_lo, ex_hi) = match ex_opt {
                            Some(ErrorRef::Symmetric { column }) => {
                                let h = lookup(column)?;
                                (h, h)
                            }
                            Some(ErrorRef::Asymmetric { lower, upper }) => {
                                (lookup(lower)?, lookup(upper)?)
                            }
                            None => (x_h, x_h),
                        };
                        let precise_errorbar_style_map = if styled.is_none() {
                            series.style.errorbar_map.as_ref()
                        } else {
                            None
                        };
                        let errorbar_style_index = match (
                            precise_errorbar_style_map,
                            extract_errorbar_style(rt),
                        ) {
                            (Some(map), Some(eb)) if map.has_index => {
                                let Some(column) = eb.error_bar_style_index_column.as_ref() else {
                                    return Err(FiggyError::InvalidSeriesConfig {
                                        series_id: cfg.series_id.clone(),
                                        reason: "errorbar style map expects an index column".into(),
                                    });
                                };
                                let h = lookup(column)?;
                                let count = [
                                    x_h.len_values,
                                    y_h.len_values,
                                    ey_lo.len_values,
                                    ey_hi.len_values,
                                    ex_lo.len_values,
                                    ex_hi.len_values,
                                ]
                                .into_iter()
                                .min()
                                .unwrap_or(0);
                                if h.len_values < count {
                                    return Err(FiggyError::InvalidSeriesConfig {
                                        series_id: cfg.series_id.clone(),
                                        reason: format!(
                                            "errorbar style index column {column:?} has {} values, but errorbar uses {count}",
                                            h.len_values
                                        ),
                                    });
                                }
                                Some(h)
                            }
                            _ => None,
                        };
                        Some(ColumnErrorBarDraw {
                            pipeline: precise_errorbar_style_map.map_or(
                                errorbar_pipe.expect("prepare ensured errorbar pipeline"),
                                |_| {
                                    pipelines
                                        .errorbar_mapped
                                        .as_ref()
                                        .expect("prepare ensured mapped errorbar pipeline")
                                },
                            ),
                            transform_bg: &view.transform_bg,
                            style_bg: &series.style.errorbar_bg,
                            style_map_bg: precise_errorbar_style_map.map(|m| &m.bind_group),
                            pool_buffer: column_buffer,
                            x: x_h,
                            y: y_h,
                            err_y_lo: ey_lo,
                            err_y_hi: ey_hi,
                            err_x_lo: ex_lo,
                            err_x_hi: ex_hi,
                            style_index: errorbar_style_index,
                        })
                    }
                }
            };

            let selection_prepare::SelectionLayers {
                picked,
                selected_bars,
                selected_fields,
            } = if phase.is_some() {
                selection_prepare::SelectionLayers {
                    picked: Vec::new(),
                    selected_bars: Vec::new(),
                    selected_fields: Vec::new(),
                }
            } else {
                self.build_selection_layers(
                    view,
                    chart_config,
                    series,
                    pipelines,
                    selection_prepare::SelectionColumns {
                        buffer: column_buffer,
                        x: x_h,
                        y: y_h,
                        bar: bar.as_ref().map(|layer| (layer.edges, layer.values)),
                        rows: selection_prepare::SelectionRows::RESIDENT,
                        mapped_point_bg: None,
                    },
                    products
                        .contours
                        .get(idx)
                        .and_then(|series| series.scratch.as_ref()),
                    styled.is_none(),
                    &mut lookup,
                )?
            };

            out.push(data_render::SeriesLayers {
                area,
                field,
                bar,
                contour,
                vector,
                errorbar,
                line,
                line_extra,
                scatter,
                selected_bars,
                selected_fields,
                picked,
            });
        }
        Ok(out)
    }

    /// The `(x_base, y_base, n)` source layout for an arc scan over these
    /// columns right now — `None` when a column is missing or shorter than
    /// two points.
    fn arc_source_layout(&self, x_id: &str, y_id: &str) -> Option<(u32, u32, u32)> {
        let (x_offset, x_len) = {
            let s = self.pool.slot(x_id)?;
            (s.offset, s.len_values)
        };
        let (y_offset, y_len) = {
            let s = self.pool.slot(y_id)?;
            (s.offset, s.len_values)
        };
        let n = x_len.min(y_len);
        if n < 2 {
            return None;
        }
        let n = u32::try_from(n).ok()?;
        // Pool offsets are 256-aligned bytes → exact f32 element indices.
        // Arc WGSL views the pool as f32 lanes; each logical value occupies
        // two lanes `(hi, lo)`, so `point_px(i)` applies the `i * 2` stride.
        let x_base = u32::try_from(x_offset / 4).ok()?;
        let y_base = u32::try_from(y_offset / 4).ok()?;
        Some((x_base, y_base, n))
    }

    /// Ensure the GPU arc-length prefix for one dashed line series and return
    /// the token-side snapshot: the prefix buffer slice, the owned star-pass
    /// handles (when the scratch has one), and the source-column layout the
    /// scan ran over. The whole computation runs on the GPU
    /// (`line_arc.wgsl` compute scan over the pool columns) — the data never
    /// returns to the CPU, keeping the pool's no-CPU-copy contract intact.
    /// Series longer than one chunk's dispatch capacity scan as sequential
    /// chunks linked by a carry buffer, so any pool-resident `n` is fine.
    ///
    /// A distinct scan input is dispatched once into new immutable buffers.
    /// An exact key hit reuses that result without traversing the columns. An
    /// old result is never rewritten: after `paint_prepared` records a buffer,
    /// the host may drop its [`PreparedFrame`] before submitting the command
    /// buffer, and the late submission must still see the recorded generation.
    ///
    /// `star_pitch`: `Some` adds the arc-driven star pass to the scratch
    /// (indirect args + VS bind group) and runs its kernel after the scan.
    /// Prepare passes `Some` only for Milkyway lines; `DrawStyle::Constellation`
    /// has no arc-prefix star pass.
    fn ensure_arc_prefix(
        &mut self,
        series_id: &str,
        x_id: &str,
        y_id: &str,
        t: &data_render::ScatterTransform,
        star_pitch: Option<f32>,
        interpolation: LineInterpolation,
    ) -> Option<PreparedArc> {
        // The mode word: the step mode and, on a polar chart, the arc pieces
        // the transform carries.
        let step_mode = crate::line_step::mode_word_for(interpolation, t);
        if self.arc_pipelines.is_none() {
            *self.arc_pipelines = Some(data_render::line_arc::create_arc_scan_pipelines(
                &self.device,
            ));
        }
        let (x_base, y_base, n) = self.arc_source_layout(x_id, y_id)?;
        let layout_generation = self.pool.layout_generation();
        let x_allocation_epoch = self.pool.allocation_epoch(x_id)?;
        let y_allocation_epoch = self.pool.allocation_epoch(y_id)?;
        let transform_bits: [u32; std::mem::size_of::<data_render::ScatterTransform>() / 4] =
            bytemuck::cast(*t);
        let mut geometry_transform_bits = [0u32; 16];
        geometry_transform_bits.copy_from_slice(&transform_bits[..16]);
        let mut polar_transform_bits = [0u32; 8];
        polar_transform_bits.copy_from_slice(&transform_bits[28..]);
        let key = ArcResultKey {
            n,
            x_base,
            y_base,
            pool_layout_generation: layout_generation,
            x_allocation_epoch,
            y_allocation_epoch,
            geometry_transform_bits,
            polar_transform_bits,
            star_pitch_bits: star_pitch.map(f32::to_bits),
            step_mode,
        };

        // Runaway-churn backstop: ids of long-removed series would otherwise
        // pin GPU memory forever. Exact-key results are immutable; new-key
        // inserts evict only through this bounded cache policy.
        let has_cached_series = self.arc_cache.contains_key(series_id);
        if self.arc_cache.len() > ARC_PREFIX_CACHE_LIMIT
            || (!has_cached_series && self.arc_cache.len() >= ARC_PREFIX_CACHE_LIMIT)
        {
            self.arc_cache.clear();
        }
        let results = self.arc_cache.entry(series_id.to_string()).or_default();
        if let Some(hit) = results.iter().find(|entry| entry.key == key) {
            let star = hit.scratch.star.as_ref().map(|star| PreparedStar {
                indirect: star.indirect.clone(),
                vs_bg: star.vs_bg.clone(),
            });
            return Some(PreparedArc {
                prefix: (Arc::clone(&hit.scratch.arc), u64::from(n) * 4),
                star,
                charge: hit.scratch.charge(),
            });
        }

        #[cfg(test)]
        let chunk_override = self.arc_chunk_override;
        #[cfg(not(test))]
        let chunk_override = None;
        let scratch = data_render::line_arc::ArcScratch::build(
            &self.device,
            &self.gpu_ledger,
            self.arc_pipelines.as_ref().expect("arc pipelines ensured"),
            self.pool.buffer(),
            n,
            x_base,
            y_base,
            self.caps.max_compute_workgroups_per_dimension,
            chunk_override,
            star_pitch.is_some().then_some(&self.star_data_bgl),
            step_mode,
        )?; // None only on a zero-dispatch-limit adapter; try_new rejects those.

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy line arc encoder"),
            });
        scratch.dispatch(
            &self.queue,
            &mut encoder,
            self.arc_pipelines.as_ref().expect("arc pipelines ensured"),
            t,
            star_pitch,
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        // Snapshot after submission. The cache never mutates this scratch;
        // dropping a prepared token therefore cannot make a future prepare
        // overwrite a host command buffer that already recorded these handles.
        let star = scratch.star.as_ref().map(|s| PreparedStar {
            indirect: s.indirect.clone(),
            vs_bg: s.vs_bg.clone(),
        });
        let prepared = PreparedArc {
            prefix: (Arc::clone(&scratch.arc), u64::from(n) * 4),
            star,
            charge: scratch.charge(),
        };
        if results.len() >= ARC_RESULT_CACHE_LIMIT_PER_SERIES {
            results.remove(0);
        }
        results.push(ArcResultEntry { key, scratch });
        Some(prepared)
    }
}

/// Whether `config` draws as more than one panel: it has overlays or
/// subplots.
fn is_layered(config: &Config) -> bool {
    !config.overlays.is_empty() || !config.subplots.is_empty()
}

/// The panel that draws `layer` among the `count` panels `config` drew as
/// (`Config::panel_layers` order). A layer past them — after the fallback
/// to the base panel — is drawn by the base panel.
fn panel_index(config: &Config, count: usize, layer: crate::LayerRef) -> usize {
    config
        .panel_layers()
        .iter()
        .take(count)
        .position(|panel| *panel == layer)
        .unwrap_or(0)
}

/// The internal panels a document draws as, base first. A document without
/// overlays and subplots is drawn exactly as it is written (no expansion, no
/// data-area requirement), so every existing single-layer chart is
/// unchanged; a layered document goes through `Config::expand`.
fn expand_for_render(config: &Config) -> Result<Vec<Config>> {
    validate_polar_config(config)?;
    for index in 0..config.subplots.len() {
        if let Some(subplot) = config.subplot_config(index) {
            validate_polar_config(&subplot)?;
        }
    }
    if !is_layered(config) {
        let mut panel = config.clone();
        panel.active_layer = crate::LayerRef::Base;
        return Ok(vec![panel]);
    }
    if config.validate_overlays().is_err() {
        return Err(FiggyError::InvalidConfig {
            field: "overlays",
            reason: "a series id may belong to at most one overlay layer",
        });
    }
    if let Err(reason) = config.validate_subplots() {
        return Err(FiggyError::InvalidConfig {
            field: "subplots",
            reason,
        });
    }
    match config.expand() {
        Ok(expansion) => Ok(expansion
            .panels
            .into_iter()
            .map(|panel| panel.config)
            .collect()),
        // No data area at this size (a preview squeezed below its margins):
        // a single-layer chart draws its chrome and no data there, so a
        // layered one draws its base panel the same way instead of failing
        // the frame. The next larger size expands normally again.
        Err(_) => {
            let mut panel = config.clone();
            panel.overlays.clear();
            panel.subplots.clear();
            panel.active_layer = crate::LayerRef::Base;
            Ok(vec![panel])
        }
    }
}

/// The view of each panel, base first. A view created or refreshed with
/// fewer overlays than the document now has cannot draw it; refresh first.
fn layer_views(view: &ChartView, panels: usize) -> Result<Vec<&ChartView>> {
    if view.overlay_views.len() + 1 < panels {
        return Err(FiggyError::StaleStateToken {
            reason: format!(
                "the chart view holds {} overlay layers but the document has {}; \
                 refresh the view before drawing",
                view.overlay_views.len(),
                panels - 1
            ),
        });
    }
    Ok(std::iter::once(view)
        .chain(view.overlay_views.iter())
        .take(panels)
        .collect())
}

/// The grid texture of one panel. An overlay panel in the Milkyway style gets
/// a transparent texture: the backdrop is the base panel's.
fn raster_grid_layer(config: &Config, overlay: bool, w: u32, h: u32) -> Result<Vec<u8>> {
    if overlay && matches!(config.draw_style, DrawStyle::Milkyway(_)) {
        return Ok(vec![0; (w as usize) * (h as usize) * 4]);
    }
    axis_render::try_raster_chart_layer_to_rgba(config, axis_render::AxisLayerKind::Grid)
}

/// Every panel's tick warnings as one list. An overlay panel reports only
/// its own Y axis (right, or left for the first layer of an area of its
/// own), tagged with its layer; its other axes are copies of the base
/// panel's and would repeat the base warnings.
fn merged_render_warnings(config: &Config, panels: &[Config]) -> Vec<crate::RenderWarning> {
    let layers = config.panel_layers();
    let mut merged = Vec::new();
    for (panel, layer) in panels.iter().zip(layers) {
        for warning in axis_render::tick_warnings(panel) {
            let axis = warning.axis();
            let tagged = match (layer, axis) {
                (crate::LayerRef::Base, _) => Some(warning),
                (crate::LayerRef::Overlay(index), Some(crate::WarningAxis::RightY)) => {
                    Some(warning.with_axis(crate::WarningAxis::OverlayY(index)))
                }
                // A subplot is a chart of its own: all of its own warnings,
                // tagged with the subplot.
                (crate::LayerRef::Subplot(index), Some(axis)) => {
                    crate::SubplotAxis::from_panel(axis)
                        .map(|axis| warning.with_axis(crate::WarningAxis::Subplot { index, axis }))
                }
                (
                    crate::LayerRef::SubplotOverlay(index, overlay),
                    Some(crate::WarningAxis::RightY),
                ) => Some(warning.with_axis(crate::WarningAxis::Subplot {
                    index,
                    axis: crate::SubplotAxis::OverlayY(overlay),
                })),
                _ => None,
            };
            merged.extend(tagged);
        }
    }
    merged
}

/// Where one internal panel is drawn on the surface.
struct PanelPlace {
    /// The panel's rectangle on the surface: the host's panel rect for the
    /// parent's panels, else the subplot's own area moved with the parent.
    rect: Rect,
    /// The first panel of a chart: it owns the grid slot and the Milkyway
    /// backdrop there.
    draws_backdrop: bool,
}

/// The place of every panel of `config` (as `expand_for_render` returned
/// them) when the parent's area is drawn at `panel_rect`.
fn panel_places(config: &Config, panels: &[Config], panel_rect: Rect) -> Vec<PanelPlace> {
    let parent = config.chart_area.0;
    let dx = i64::from(panel_rect.x) - i64::from(parent.x);
    let dy = i64::from(panel_rect.y) - i64::from(parent.y);
    let shift = |value: u32, by: i64| (i64::from(value) + by).clamp(0, i64::from(u32::MAX)) as u32;
    panels
        .iter()
        .zip(config.panel_layers())
        .map(|(panel, layer)| match layer {
            crate::LayerRef::Subplot(_) | crate::LayerRef::SubplotOverlay(..) => {
                let area = panel.chart_area.0;
                PanelPlace {
                    rect: Rect {
                        x: shift(area.x, dx),
                        y: shift(area.y, dy),
                        width: area.width,
                        height: area.height,
                    },
                    draws_backdrop: matches!(layer, crate::LayerRef::Subplot(_)),
                }
            }
            layer => PanelPlace {
                rect: panel_rect,
                draws_backdrop: layer == crate::LayerRef::Base,
            },
        })
        .collect()
}

/// Each selection box goes to the topmost panel whose rectangle holds the
/// box's centre; a box outside every panel goes to the topmost panel.
fn route_selection_boxes(
    places: &[PanelPlace],
    selection: &[crate::select::SelectionBox],
) -> Vec<Vec<crate::select::SelectionBox>> {
    let mut routed = vec![Vec::new(); places.len()];
    for selection_box in selection {
        let cx = selection_box.rect.x + selection_box.rect.width * 0.5;
        let cy = selection_box.rect.y + selection_box.rect.height * 0.5;
        let index = places
            .iter()
            .rposition(|place| crate::layout::RectF::from_rect(&place.rect).contains(cx, cy))
            .unwrap_or(places.len() - 1);
        routed[index].push(selection_box.clone());
    }
    routed
}

/// The document's annotation layer is rasterized once, on the base view; its
/// underlay stays under the base panel's data and its overlay moves to the
/// topmost panel drawn in the parent's area, so it sits above every layer's
/// decoration there. Panels in areas of their own are not its place: the
/// annotation textures cover the parent's area only.
fn lift_annotation_overlays(items: &mut [PreparedItem], group_sizes: &[usize]) {
    let mut start = 0;
    for &size in group_sizes {
        if size > 1 && start + size <= items.len() {
            let base_rect = items[start].view.panel_rect;
            let top = (start..start + size)
                .rev()
                .find(|&index| items[index].view.panel_rect == base_rect)
                .unwrap_or(start);
            if top != start {
                let overlay = items[start].view.annotation_overlay_bind_group.take();
                let visible = items[start].view.annotations_visible;
                let top = &mut items[top].view;
                top.annotation_overlay_bind_group = overlay;
                top.annotations_visible = visible;
            }
        }
        start += size;
    }
}

/// How many panels `config` draws as; see [`expand_for_render`].
fn render_panel_count(config: &Config) -> usize {
    if !is_layered(config) {
        1
    } else {
        expand_for_render(config).map_or(1, |panels| panels.len())
    }
}

/// One chart panel to draw: view + chart config + series slice.
pub struct ChartDrawItem<'a> {
    pub view: &'a ChartView,
    pub chart_config: &'a crate::config::Config,
    pub series: &'a [Series<'a>],
}

/// Prepare-only resources for an explicitly identified renderer-owned chart.
/// Config and ordered series are borrowed from the Renderer registry.
pub struct RegisteredChartDrawItem<'a> {
    pub chart_id: ChartId,
    pub view: &'a ChartView,
}

// DataRenderType branching helpers — which layers are needed and which
// sub-style to extract.

// Which primitives a render type draws.
//
// Derived from the extractors rather than listed again, following `has_errorbar`
// below. An exhaustive `match` in each predicate would make the compiler stop on
// a new variant — but it would only force the arms to be *written*, not to agree
// with the config lookup. Someone answering `Bar { .. } => true` in `has_scatter`
// while `extract_scatter` correctly says `None` reproduces the original bug
// exactly, past a compiler that saw nothing wrong. Asking the extractor makes the
// two impossible to disagree, and leaves exactly one exhaustive match per
// primitive as the place a new variant has to be decided.
//
// The bug this shape closes: `has_scatter` was
// `!matches!(rt, Line { .. })` — "anything that is not a bare line has points" —
// which answers *yes* for a variant that draws bars or a filled field. Nothing
// panics on that answer, which is what makes it dangerous:
// `ensure_precise_variants_for_items` and `build_series_layers` read this same
// predicate, so they agree, compile the scatter pipeline, and draw a
// `ColumnScatterLayer` for the series. With no scatter config to read,
// `create_style_for_series_scaled` falls back to
// `PrimitiveStyle::from_color(BLACK)` — a 4 px filled circle — so the new series
// silently gains black dots at every data point instead of crashing.
fn has_line(rt: &DataRenderType) -> bool {
    extract_line(rt).is_some()
}

fn has_scatter(rt: &DataRenderType) -> bool {
    extract_scatter(rt).is_some()
}

fn has_errorbar(rt: &DataRenderType) -> bool {
    extract_err_x(rt).is_some() || extract_err_y(rt).is_some()
}

fn extract_line(rt: &DataRenderType) -> Option<&DataLineStyleConfig> {
    match rt {
        DataRenderType::Line { line }
        | DataRenderType::ScatterLine { line, .. }
        | DataRenderType::LineScatterErrorbarX { line, .. }
        | DataRenderType::LineScatterErrorbarY { line, .. }
        | DataRenderType::LineScatterErrorbarXY { line, .. } => Some(line),
        DataRenderType::Scatter { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        // A contour's `contour.line` is not this line. This one is the polyline
        // through `(x_column, y_column)`; a contour is an implicit level set
        // drawn directly from the matrix. Returning its style here would make `has_line`
        // true and draw a polyline through the grid's coordinate columns.
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. } => None,
    }
}

fn extract_scatter(rt: &DataRenderType) -> Option<&DataScatterStyleConfig> {
    match rt {
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterLine { scatter, .. }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. }
        | DataRenderType::LineScatterErrorbarX { scatter, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, .. } => Some(scatter),
        // None of these draw points. This is the predicate `has_scatter` is
        // derived from, so answering otherwise is what put black 4 px dots at
        // every data point — see the note above.
        DataRenderType::Line { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        // Arrows are not markers: they have their own pipeline, pick radius
        // and legend mark (`extract_vector`).
        | DataRenderType::Vector { .. } => None,
    }
}

/// What one series would draw right now, and whether the data forced a smaller
/// extent than the declaration asked for.
///
/// Column lengths are facts that arrive with the data, not SSoT — so a mismatch
/// never errors and never stops the draw: the smallest common extent is drawn
/// and reported here. This is the **series-common** window onto
/// that: a histogram's `edges` / `counts` truncation, a line's `min(x, y)`, and
/// (from W6) a matrix' grid extent all come out of the same query, so no
/// primitive ends up without a reporting path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeriesDrawInfo {
    /// Primitive instances the paint phase issues for this series: points,
    /// line segments, errorbars, or bars.
    pub drawn_count: u64,
    /// Grid columns actually used, for a matrix-backed series. `None` for every
    /// other render type.
    pub cols: Option<u64>,
    /// Grid rows actually used, for a matrix-backed series. `None` otherwise.
    pub rows: Option<u64>,
    /// True when some role column carries more values than `drawn_count` can
    /// consume — the declaration asked for more than the data supports, and the
    /// surplus is not drawn.
    pub truncated: bool,
}

/// The bar appearance of a bar-backed render type.
///
/// The single exhaustive match that decides whether a variant draws bars;
/// `has_bar` is derived from it so the predicate and the config lookup cannot
/// disagree, following `has_line` / `has_scatter`.
fn extract_bar(rt: &DataRenderType) -> Option<&crate::data_config::DataBarStyleConfig> {
    match rt {
        DataRenderType::Histogram { bar } => Some(bar),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. } => None,
    }
}

fn has_bar(rt: &DataRenderType) -> bool {
    extract_bar(rt).is_some()
}

/// The fill appearance of a field-backed render type.
///
/// `Contour` is deliberately absent: contour lines over an unfilled field is a
/// distinct look, and giving it a default fill would paint something the config
/// does not ask for. The single exhaustive match that decides whether a variant
/// paints cells; `has_field` is derived from it.
fn extract_field_fill(rt: &DataRenderType) -> Option<&crate::data_config::FieldFillConfig> {
    match rt {
        DataRenderType::Heatmap { fill, .. } | DataRenderType::HeatmapContour { fill, .. } => {
            Some(fill)
        }
        DataRenderType::Contour { .. }
        | DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Vector { .. } => None,
    }
}

fn has_field(rt: &DataRenderType) -> bool {
    extract_field_fill(rt).is_some()
}

/// Whether this render type draws implicit contour lines. A `Heatmap` with no contour
/// declaration does not, even though its banded fill can quantize into levels.
fn has_contour(rt: &DataRenderType) -> bool {
    extract_contour(rt).is_some()
}

/// The contour declaration of a render type that draws contour lines. Its levels
/// also divide the bands of a quantized fill.
pub(crate) fn extract_contour(rt: &DataRenderType) -> Option<&crate::data_config::ContourConfig> {
    match rt {
        DataRenderType::Contour { contour, .. }
        | DataRenderType::HeatmapContour { contour, .. } => Some(contour),
        DataRenderType::Heatmap { .. }
        | DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Vector { .. } => None,
    }
}

/// The components and style of a vector series.
///
/// The single exhaustive match that decides whether a variant draws arrows;
/// `has_vector` is derived from it, following `extract_bar` / `extract_scatter`.
fn extract_vector(
    rt: &DataRenderType,
) -> Option<(
    &crate::data_config::VectorComponents,
    &crate::data_config::VectorStyle,
)> {
    match rt {
        DataRenderType::Vector { components, style } => Some((components, style)),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. } => None,
    }
}

fn has_vector(rt: &DataRenderType) -> bool {
    extract_vector(rt).is_some()
}

/// `SeriesDrawInfo` for one series against current logical column lengths.
///
/// Exhaustive over the render types so a new primitive cannot ship without
/// deciding how its extent is reported. An unregistered column counts
/// as zero: nothing draws, and that is reported rather than raised — checking
/// registration is `validate_renderer_series`' job.
fn series_draw_info_from_pool(
    pool: &impl ColumnRegistration,
    series: &SeriesConfig,
) -> SeriesDrawInfo {
    let len = |id: &str| -> u64 { pool.column_len(id).unwrap_or(0) };
    let x = len(&series.x_column);
    let y = len(&series.y_column);
    // Index-aligned roles: the shortest decides, and anything longer is surplus.
    let paired = |lengths: &[u64]| -> (u64, bool) {
        let low = lengths.iter().copied().min().unwrap_or(0);
        (low, lengths.iter().any(|value| *value > low))
    };

    match &series.render_type {
        // A polyline draws one segment between adjacent points.
        DataRenderType::Line { .. } => {
            let (count, truncated) = paired(&[x, y]);
            SeriesDrawInfo {
                drawn_count: count.saturating_sub(1),
                cols: None,
                rows: None,
                truncated,
            }
        }
        DataRenderType::Scatter { .. } | DataRenderType::ScatterLine { .. } => {
            let (count, truncated) = paired(&[x, y]);
            SeriesDrawInfo {
                drawn_count: count,
                cols: None,
                rows: None,
                truncated,
            }
        }
        DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. } => {
            // x, y, and up to two lo/hi pairs — the same six roles the
            // errorbar draw call minimizes over, on the stack.
            let mut lengths = [x, y, 0, 0, 0, 0];
            let mut used = 2;
            for error in [
                extract_err_x(&series.render_type),
                extract_err_y(&series.render_type),
            ]
            .into_iter()
            .flatten()
            {
                match error {
                    ErrorRef::Symmetric { column } => {
                        lengths[used] = len(column);
                        used += 1;
                    }
                    ErrorRef::Asymmetric { lower, upper } => {
                        lengths[used] = len(lower);
                        lengths[used + 1] = len(upper);
                        used += 2;
                    }
                }
            }
            let (count, truncated) = paired(&lengths[..used]);
            SeriesDrawInfo {
                drawn_count: count,
                cols: None,
                rows: None,
                truncated,
            }
        }
        // `n` bars need `n + 1` edges. Orientation decides which column is
        // which — never the lengths.
        DataRenderType::Histogram { bar } => {
            let (edges, values) = match bar.orientation {
                crate::data_config::BarOrientation::Vertical => (x, y),
                crate::data_config::BarOrientation::Horizontal => (y, x),
            };
            let bins = edges.saturating_sub(1);
            SeriesDrawInfo {
                drawn_count: bins.min(values),
                cols: None,
                rows: None,
                truncated: bins != values,
            }
        }
        // A field's primitive count is its cells. `cols` / `rows` report the
        // grid the declaration and the data agreed on, which is the only way a
        // host can tell a 10x10 grid drawn as 10x9 from one drawn whole.
        DataRenderType::Heatmap { matrix, .. }
        | DataRenderType::Contour { matrix, .. }
        | DataRenderType::HeatmapContour { matrix, .. } => {
            let extent = matrix_extent(matrix, x as usize, y as usize, |id| len(id) as usize);
            SeriesDrawInfo {
                drawn_count: (extent.cols as u64).saturating_mul(extent.rows as u64),
                cols: Some(extent.cols as u64),
                rows: Some(extent.rows as u64),
                truncated: extent.truncated,
            }
        }
        // Point vectors: one arrow per index-aligned row of x, y, u, v and the
        // colour column. Grid vectors: one arrow per cell of the smallest
        // common extent of the u, v and colour grids.
        DataRenderType::Vector { components, style } => {
            let color_column = match style.color_map.as_ref() {
                Some(crate::data_config::VectorColorMap::Column(column)) => Some(len(column)),
                _ => None,
            };
            match components {
                crate::data_config::VectorComponents::Points { u, v } => {
                    let mut lengths = [x, y, len(u), len(v), 0];
                    let used = if let Some(color) = color_column {
                        lengths[4] = color;
                        5
                    } else {
                        4
                    };
                    let (count, truncated) = paired(&lengths[..used]);
                    SeriesDrawInfo {
                        drawn_count: count,
                        cols: None,
                        rows: None,
                        truncated,
                    }
                }
                crate::data_config::VectorComponents::Grid { u, v } => {
                    let extent = vector_grid_extent(u, v, style, x as usize, y as usize, |id| {
                        len(id) as usize
                    });
                    SeriesDrawInfo {
                        drawn_count: (extent.cols as u64).saturating_mul(extent.rows as u64),
                        cols: Some(extent.cols as u64),
                        rows: Some(extent.rows as u64),
                        truncated: extent.truncated,
                    }
                }
            }
        }
    }
}

/// The grid declaration of a matrix-backed render type.
///
/// The single exhaustive match that decides whether a variant is a field: both
/// "which columns does it reference" and "does it need a z scale" are derived
/// from this one answer instead of being listed again, so they cannot disagree
/// about a variant.
fn extract_matrix(rt: &DataRenderType) -> Option<&crate::data_config::MatrixRef> {
    match rt {
        DataRenderType::Heatmap { matrix, .. }
        | DataRenderType::Contour { matrix, .. }
        | DataRenderType::HeatmapContour { matrix, .. } => Some(matrix),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        // A vector's grids are not "the matrix": they are components, not z.
        // `fit_matrix` reaches the `u` grid for the coordinate lattice.
        | DataRenderType::Vector { .. } => None,
    }
}

/// Whether this render type cannot be drawn without `Config.colorbar`.
///
/// A field's z range and colormap live nowhere else, so without the colourbar
/// there is no value to paint — not a missing decoration, a missing input.
/// Derived from `extract_matrix` rather than matched again.
fn requires_colorbar(rt: &DataRenderType) -> bool {
    extract_matrix(rt).is_some()
}

fn extract_errorbar_style(rt: &DataRenderType) -> Option<&DataErrorBarStyleConfig> {
    match rt {
        DataRenderType::ScatterErrorbarX { err_style, .. }
        | DataRenderType::ScatterErrorbarY { err_style, .. }
        | DataRenderType::ScatterErrorbarXY { err_style, .. }
        | DataRenderType::LineScatterErrorbarX { err_style, .. }
        | DataRenderType::LineScatterErrorbarY { err_style, .. }
        | DataRenderType::LineScatterErrorbarXY { err_style, .. } => Some(err_style),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. } => None,
    }
}

fn extract_err_y(rt: &DataRenderType) -> Option<&ErrorRef> {
    match rt {
        DataRenderType::ScatterErrorbarY { err_y, .. }
        | DataRenderType::ScatterErrorbarXY { err_y, .. }
        | DataRenderType::LineScatterErrorbarY { err_y, .. }
        | DataRenderType::LineScatterErrorbarXY { err_y, .. } => Some(err_y),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. } => None,
    }
}

fn extract_err_x(rt: &DataRenderType) -> Option<&ErrorRef> {
    match rt {
        DataRenderType::ScatterErrorbarX { err_x, .. }
        | DataRenderType::ScatterErrorbarXY { err_x, .. }
        | DataRenderType::LineScatterErrorbarX { err_x, .. }
        | DataRenderType::LineScatterErrorbarXY { err_x, .. } => Some(err_x),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. } => None,
    }
}

fn errorbar_direction_flags(rt: &DataRenderType) -> u32 {
    (u32::from(extract_err_y(rt).is_some()) * ERRORBAR_HAS_Y)
        | (u32::from(extract_err_x(rt).is_some()) * ERRORBAR_HAS_X)
}

// WindowedRenderer — figgy owns the surface and swap chain.

/// Returned by `Renderer::for_window`. The caller never touches the surface,
/// surface config, encoder, or render pass — just calls `draw` per frame.
///
/// Immutable renderer access is available through `Deref<Target = Renderer>`.
/// Mutations use explicit forwarding methods so surface-sensitive operations
/// cannot bypass this owner.
pub struct WindowedRenderer<'w> {
    inner: Renderer,
    surface: wgpu::Surface<'w>,
    surface_config: wgpu::SurfaceConfiguration,
    msaa_target: Option<MsaaTarget>,
    /// Instance and adapter must outlive the surface; keep them pinned here.
    _instance: wgpu::Instance,
    _adapter: wgpu::Adapter,
}

impl<'w> std::ops::Deref for WindowedRenderer<'w> {
    type Target = Renderer;
    fn deref(&self) -> &Renderer {
        &self.inner
    }
}

impl Drop for WindowedRenderer<'_> {
    fn drop(&mut self) {
        self.inner.wait_idle();
    }
}

impl<'w> WindowedRenderer<'w> {
    /// Publish an owned preparation without retaining a mutable borrow while it compiles.
    pub fn install_preparation(&mut self, prepared: PreparedPipelines) -> Result<()> {
        self.inner.install_preparation(prepared)
    }

    /// Allocate a budget-checked style for this surface's renderer.
    pub fn create_style_for_series(&mut self, cfg: &SeriesConfig) -> Result<ChartStyle> {
        self.inner.create_style_for_series(cfg)
    }

    /// High-DPI variant of [`Self::create_style_for_series`].
    pub fn create_style_for_series_scaled(
        &mut self,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<ChartStyle> {
        self.inner.create_style_for_series_scaled(cfg, scale)
    }

    /// Build a standalone renderer while reporting timestamp-free startup
    /// stage boundaries to `observer`.
    pub async fn for_window_async_observed(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self> {
        Renderer::for_window_async_observed(target, size, pool_capacity_bytes, observer).await
    }

    pub fn surface_size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    pub async fn ensure_errorbar_extent_engine(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_errorbar::GpuErrorbarError> {
        self.inner.ensure_errorbar_extent_engine().await
    }

    pub async fn prewarm_all_observed(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<()> {
        self.inner.prewarm_all_observed(observer).await
    }

    pub async fn prewarm_all(&mut self) -> Result<()> {
        self.inner.prewarm_all().await
    }

    pub fn add_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        self.inner.add_column(id, source)
    }

    pub fn add_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        self.inner.add_hilo_column(id, source)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn begin_load_demo<F>(
        &mut self,
        chart_id: ChartId,
        demo_x: &dyn ColumnSource,
        demo_sin: &dyn ColumnSource,
        demo_t: &dyn ColumnSource,
        demo_rc: &dyn ColumnSource,
        prepare_state: F,
    ) -> Result<RendererLoadDemo<'_>>
    where
        F: FnOnce(&ColumnPool) -> Result<(Config, Vec<SeriesConfig>)>,
    {
        self.inner
            .begin_load_demo(chart_id, demo_x, demo_sin, demo_t, demo_rc, prepare_state)
    }

    pub fn add_columns(&mut self, columns: &[(&str, &dyn ColumnSource)]) -> Result<()> {
        self.inner.add_columns(columns)
    }

    pub fn register_streamed_columns(&mut self, columns: Vec<crate::StreamColumn>) -> Result<()> {
        self.inner.register_streamed_columns(columns)
    }

    pub fn replace_streamed_columns(&mut self, columns: Vec<crate::StreamColumn>) -> Result<()> {
        self.inner.replace_streamed_columns(columns)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resident_admission_for_streamed_columns(
        &self,
        sources: &[crate::StreamSourceBinding<'_>],
        already_resident_working_set_bytes: u64,
        derived_buffer_sizes: &[u64],
        transition_headroom_bytes: u64,
        working_set_limit_bytes: u64,
    ) -> Result<ResidentAdmission> {
        self.inner.resident_admission_for_streamed_columns(
            sources,
            already_resident_working_set_bytes,
            derived_buffer_sizes,
            transition_headroom_bytes,
            working_set_limit_bytes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn promote_streamed_columns_to_resident(
        &mut self,
        sources: &[crate::StreamSourceBinding<'_>],
        already_resident_working_set_bytes: u64,
        derived_buffer_sizes: &[u64],
        transition_headroom_bytes: u64,
        working_set_limit_bytes: u64,
    ) -> Result<ResidentAdmission> {
        self.inner.promote_streamed_columns_to_resident(
            sources,
            already_resident_working_set_bytes,
            derived_buffer_sizes,
            transition_headroom_bytes,
            working_set_limit_bytes,
        )
    }

    pub fn demote_resident_columns_to_streamed(
        &mut self,
        changed_ids: &[&str],
        columns: Vec<crate::StreamColumn>,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<()> {
        self.inner
            .demote_resident_columns_to_streamed(changed_ids, columns, sources)
    }

    pub fn configure_streaming(&mut self, limits: crate::StreamingLimits) -> Result<()> {
        self.inner.configure_streaming(limits)
    }

    pub fn begin_streaming_chart(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        options: crate::StreamingChartOptions,
    ) -> Result<()> {
        self.inner.begin_streaming_chart(chart, view, options)
    }

    pub fn request_auto_streaming_chart(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        self.inner
            .request_auto_streaming_chart(chart, view, options)
    }

    pub fn request_auto_streaming_chart_with_config(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        render_config: Config,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        self.inner
            .request_auto_streaming_chart_with_config(chart, view, render_config, options)
    }

    pub fn request_resident_stream_handoff_with_config(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        render_config: Config,
        columns: Vec<crate::StreamColumn>,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        self.inner.request_resident_stream_handoff_with_config(
            chart,
            view,
            render_config,
            columns,
            options,
        )
    }

    pub fn request_auto_streaming_chart_with_display_scale(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        render_config: Config,
        display_scale: f32,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        self.inner.request_auto_streaming_chart_with_display_scale(
            chart,
            view,
            render_config,
            display_scale,
            options,
        )
    }

    pub fn request_resident_stream_handoff_with_display_scale(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        render_config: Config,
        display_scale: f32,
        columns: Vec<crate::StreamColumn>,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        self.inner
            .request_resident_stream_handoff_with_display_scale(
                chart,
                view,
                render_config,
                display_scale,
                columns,
                options,
            )
    }

    pub fn auto_stream_chart_step(
        &mut self,
        chart: ChartId,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<crate::AutoStreamingProgress> {
        self.inner.auto_stream_chart_step(chart, sources)
    }

    pub fn auto_stream_chart_request_ranges(
        &mut self,
        chart: ChartId,
    ) -> Result<crate::AutoStreamingRangeRequest> {
        self.inner.auto_stream_chart_request_ranges(chart)
    }

    pub fn auto_stream_chart_submit_ranges(
        &mut self,
        chart: ChartId,
        sources: &[crate::StreamRangeSourceBinding<'_>],
    ) -> Result<crate::AutoStreamingProgress> {
        self.inner.auto_stream_chart_submit_ranges(chart, sources)
    }

    pub fn interrupt_render(&mut self, chart: ChartId) -> Result<crate::RenderInterruptStatus> {
        self.inner.interrupt_render(chart)
    }

    pub fn stream_chart_step(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<crate::StreamingProgress> {
        self.inner.stream_chart_step(chart, view, sources)
    }

    pub fn cancel_streaming_chart(&mut self, chart: ChartId) -> Result<()> {
        self.inner.cancel_streaming_chart(chart)
    }

    pub fn set_stream_chunk_budget(&mut self, chart: ChartId, primitives: u64) -> Result<()> {
        self.inner.set_stream_chunk_budget(chart, primitives)
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn cancel_streaming_chart_and_wait(&mut self, chart: ChartId) -> Result<()> {
        self.inner.cancel_streaming_chart_and_wait(chart).await
    }

    pub fn begin_upsert_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        self.inner.begin_upsert_column(id, source)
    }

    pub fn begin_upsert_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        self.inner.begin_upsert_hilo_column(id, source)
    }

    pub fn upsert_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        self.inner.upsert_column(id, source)
    }

    pub fn upsert_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        self.inner.upsert_hilo_column(id, source)
    }

    pub fn remove_column(&mut self, id: &str) -> Result<bool> {
        self.inner.remove_column(id)
    }

    pub fn remove_column_with_chart_config(
        &mut self,
        id: &str,
        chart_id: ChartId,
        next_config: Config,
    ) -> Result<bool> {
        self.inner
            .remove_column_with_chart_config(id, chart_id, next_config)
    }

    pub fn defragment(&mut self) -> Result<bool> {
        self.inner.defragment()
    }

    /// Set the allocation policy described by [`Renderer::set_memory_budget`],
    /// including what the budget does and does
    /// not govern and what a returned total means.
    ///
    /// The window's MSAA target is charged to the same ledger as the
    /// renderer's tracked resources, and the immutable [`Renderer::gpu_memory_usage`]
    /// reached through `Deref` reports it. This does not account for all backend
    /// surface/swapchain resources or enforce a device-wide ceiling.
    #[must_use = "a returned total means the new ceiling is already exceeded"]
    pub fn set_memory_budget(&mut self, budget: Option<u64>) -> Option<u64> {
        self.inner.set_memory_budget(budget)
    }

    pub fn set_auto_resident_working_set_limit(&mut self, limit: Option<u64>) -> Option<u64> {
        self.inner.set_auto_resident_working_set_limit(limit)
    }

    /// Submission boundary — see [`Renderer::end_gpu_frame`]. `draw` calls it
    /// after queue submission; hosts that record their own command buffers call
    /// it after submitting every panel in the frame.
    pub fn end_gpu_frame(&mut self) {
        self.inner.end_gpu_frame();
    }

    pub fn process_pending_maintenance(&mut self) -> Result<bool> {
        self.inner.process_pending_maintenance()
    }

    /// See [`Renderer::release_unused_pool_memory`].
    pub async fn release_unused_pool_memory(&mut self, minimum_capacity: u64) -> Result<()> {
        self.inner
            .release_unused_pool_memory(minimum_capacity)
            .await
    }

    pub fn set_defrag_policy(&mut self, policy: DefragPolicy) {
        self.inner.set_defrag_policy(policy);
    }

    /// See [`Renderer::set_pool_growth_policy`].
    pub fn set_pool_growth_policy(&mut self, policy: data_render::GrowthPolicy) {
        self.inner.set_pool_growth_policy(policy);
    }

    pub fn enable_gpu_picking(&mut self) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.inner.enable_gpu_picking()
    }

    pub fn enable_gpu_picking_observed(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.inner.enable_gpu_picking_observed(observer)
    }

    pub async fn enable_gpu_picking_async(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.inner.enable_gpu_picking_async().await
    }

    pub async fn enable_gpu_picking_observed_async(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.inner.enable_gpu_picking_observed_async(observer).await
    }

    pub fn prepare_gpu_picking_for_chart(
        &mut self,
        id: ChartId,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.inner.prepare_gpu_picking_for_chart(id)
    }

    /// Pick one chart using this window's current physical surface size.
    pub fn pick_chart_at(
        &mut self,
        id: ChartId,
        canvas_position_px: [f32; 2],
        max_distance_px: f32,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        let config = self
            .inner
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        let logical = config.config.document_area();
        let (display_scale, display_panel_px) = fit_display_panel(
            (logical.width, logical.height),
            (self.surface_config.width, self.surface_config.height),
        );
        self.inner.pick_chart(
            id,
            GpuPickRequest {
                canvas_position_px,
                display_panel_px,
                display_scale,
                max_distance_px,
            },
        )
    }

    /// Typed data pick using this window's current physical surface size.
    pub fn pick_chart_data_at(
        &mut self,
        id: ChartId,
        canvas_position_px: [f32; 2],
        max_distance_px: f32,
    ) -> Result<crate::GpuDataPickTicket> {
        let config = self
            .inner
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let logical = config.config.document_area();
        let (display_scale, display_panel_px) = fit_display_panel(
            (logical.width, logical.height),
            (self.surface_config.width, self.surface_config.height),
        );
        self.inner.pick_chart_data(
            id,
            GpuPickRequest {
                canvas_position_px,
                display_panel_px,
                display_scale,
                max_distance_px,
            },
        )
    }

    pub fn register_chart(&mut self, config: Config, series: Vec<SeriesConfig>) -> Result<ChartId> {
        self.inner.register_chart(config, series)
    }

    pub fn remove_chart(&mut self, id: ChartId) -> Result<()> {
        self.inner.remove_chart(id)
    }

    pub fn set_chart_config(&mut self, id: ChartId, config: Config) -> Result<()> {
        self.inner.set_chart_config(id, config)
    }

    pub fn set_chart_series(&mut self, id: ChartId, series: Vec<SeriesConfig>) -> Result<()> {
        self.inner.set_chart_series(id, series)
    }

    pub fn set_chart_state(
        &mut self,
        id: ChartId,
        config: Config,
        series: Vec<SeriesConfig>,
    ) -> Result<()> {
        self.inner.set_chart_state(id, config, series)
    }

    pub fn set_chart_view_state(&mut self, id: ChartId, view: ChartViewState) -> Result<()> {
        self.inner.set_chart_view_state(id, view)
    }

    pub fn set_chart_selection(&mut self, id: ChartId, selected: Option<HitId>) -> Result<()> {
        self.inner.set_chart_selection(id, selected)
    }

    pub fn set_active_layer(
        &mut self,
        id: ChartId,
        layer: crate::LayerRef,
    ) -> Result<crate::LayerRef> {
        self.inner.set_active_layer(id, layer)
    }

    pub fn active_layer(&self, id: ChartId) -> Result<crate::LayerRef> {
        self.inner.active_layer(id)
    }

    pub fn sync_external_invalidations(&mut self) -> Result<bool> {
        self.inner.sync_external_invalidations()
    }

    pub fn request_stream_auto_fit(&mut self, id: ChartId, padding: f64) -> Result<()> {
        self.inner.request_stream_auto_fit(id, padding)
    }

    pub fn commit_auto_fit_all_if_current(
        &mut self,
        token: &FitCommitToken,
        x_extent: &crate::chart::FitExtent,
        y_extent: &crate::chart::FitExtent,
        padding: f64,
    ) -> Result<()> {
        self.inner
            .commit_auto_fit_all_if_current(token, x_extent, y_extent, padding)
    }

    pub fn refresh_axis(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<()> {
        self.inner.refresh_axis(view, chart, panel_rect)
    }

    pub fn refresh_axis_with_selection(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selection: &[crate::select::SelectionBox],
    ) -> Result<()> {
        self.inner
            .refresh_axis_with_selection(view, chart, panel_rect, selection)
    }

    pub fn refresh_annotations(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<usize>,
    ) -> Result<()> {
        self.inner
            .refresh_annotations(view, chart, panel_rect, selected)
    }

    pub fn update_transform(&mut self, view: &ChartView, chart: &Chart) -> Result<()> {
        self.inner.update_transform(view, chart)
    }

    pub fn prepare(&mut self, items: &[ChartDrawItem<'_>]) -> Result<PreparedFrame> {
        self.inner.prepare(items)
    }

    pub fn prepare_registered(
        &mut self,
        items: &[RegisteredChartDrawItem<'_>],
    ) -> Result<PreparedFrame> {
        self.inner.prepare_registered(items)
    }

    pub async fn export_panel_rgba_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        self.inner
            .export_panel_rgba_async(chart, series, scale)
            .await
    }

    pub async fn export_panel_rgba_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        self.inner
            .export_panel_rgba_with_clear_async(chart, series, scale, clear)
            .await
    }

    pub async fn export_panel_png_bytes_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        self.inner
            .export_panel_png_bytes_async(chart, series, scale)
            .await
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_rgba(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        self.inner.export_panel_rgba(chart, series, scale)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_png_bytes(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        self.inner.export_panel_png_bytes(chart, series, scale)
    }

    pub async fn export_panel_png_bytes_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<Vec<u8>> {
        self.inner
            .export_panel_png_bytes_with_clear_async(chart, series, scale, clear)
            .await
    }

    pub async fn export_document_rgba_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        self.inner
            .export_document_rgba_async(chart, series, scale)
            .await
    }

    pub async fn export_document_rgba_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        self.inner
            .export_document_rgba_with_clear_async(chart, series, scale, clear)
            .await
    }

    pub async fn export_document_png_bytes_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        self.inner
            .export_document_png_bytes_async(chart, series, scale)
            .await
    }

    pub async fn export_document_png_bytes_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<Vec<u8>> {
        self.inner
            .export_document_png_bytes_with_clear_async(chart, series, scale, clear)
            .await
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_document_rgba(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        self.inner.export_document_rgba(chart, series, scale)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_document_png_bytes(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        self.inner.export_document_png_bytes(chart, series, scale)
    }

    /// Reconfigure the swap chain after a window resize. The caller is then
    /// responsible for updating each chart's `chart_area` — figgy doesn't
    /// know your panel layout policy.
    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        data_render::reconfigure_surface(
            &self.surface,
            &self._adapter,
            self.inner.device(),
            &mut self.surface_config,
            w.max(1),
            h.max(1),
        )?;
        let preferred_sample_count =
            preferred_msaa_sample_count(self.inner.caps, self.surface_config.format);
        let (target_sample_count, msaa_target) = match create_msaa_target(
            self.inner.device(),
            self.inner.gpu_ledger(),
            self.inner.caps,
            "figgy frame msaa target",
            self.surface_config.width,
            self.surface_config.height,
            self.surface_config.format,
            preferred_sample_count,
        ) {
            Ok(target) => (preferred_sample_count, target),
            Err(_) if preferred_sample_count > 1 => (1, None),
            Err(e) => return Err(e),
        };
        self.inner
            .ensure_target(self.surface_config.format, target_sample_count)?;
        self.msaa_target = msaa_target;
        Ok(())
    }

    /// Draw one frame.
    ///
    /// Runs the mutable prepare phase (`prepare(items)`), then acquires the
    /// current swap-chain texture, builds an encoder + render pass, records
    /// via `paint_prepared`, and submits and presents. The caller is only responsible for processing per-panel
    /// dirty flags (`chart.consume_*_dirty()` → `refresh_axis`).
    pub fn draw(&mut self, clear: crate::color::Color, items: &[ChartDrawItem<'_>]) -> Result<()> {
        let prepared = self.inner.prepare(items)?;
        self.draw_prepared(clear, &prepared)
    }

    /// Draw renderer-owned charts, selecting resident data or an active exact
    /// streaming display independently for each chart.
    pub fn draw_registered(
        &mut self,
        clear: crate::color::Color,
        items: &[RegisteredChartDrawItem<'_>],
    ) -> Result<()> {
        let prepared = self.inner.prepare_registered(items)?;
        self.draw_prepared(clear, &prepared)
    }

    /// Draw one frame using a prepared data token owned by the host.
    ///
    /// This still records the surface composition pass (grid, annotation
    /// underlay, data, decorations, annotation overlay),
    /// but it does not rerun `prepare`: no transform uniform write, styled
    /// pipeline ensure, or arc-prefix compute unless the host prepared a new
    /// token first.
    pub fn draw_prepared(
        &mut self,
        clear: crate::color::Color,
        prepared: &PreparedFrame,
    ) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err(FiggyError::SurfaceAcquireFailed {
                    reason: "lost".into(),
                });
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                return Err(FiggyError::SurfaceAcquireFailed {
                    reason: "outdated".into(),
                });
            }
            other => {
                return Err(FiggyError::SurfaceAcquireFailed {
                    reason: format!("{other:?}"),
                });
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let (color_view, resolve_target, store) = match self.msaa_target.as_ref() {
            Some(msaa) => (&msaa.view, Some(&target), wgpu::StoreOp::Discard),
            None => (&target, None, wgpu::StoreOp::Store),
        };

        let mut encoder =
            self.inner
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy frame encoder"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy frame pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.inner.paint_prepared(
                &mut pass,
                (self.surface_config.width, self.surface_config.height),
                prepared,
            )?;
        }
        self.inner.queue().submit(std::iter::once(encoder.finish()));
        self.inner.end_gpu_frame();
        self.inner.queue().present(frame);
        Ok(())
    }

    /// Submit one frame and wait until that GPU work has finished executing.
    ///
    /// Dawn / native drivers often compile the used pipelines on this first
    /// real submit rather than at `createRenderPipeline`. Hosts should keep
    /// a loading state up until this future resolves. It does not remove a
    /// main-thread freeze; it only makes that freeze observable as "not
    /// ready yet" instead of a black canvas.
    pub async fn first_frame_ready(
        &mut self,
        clear: crate::color::Color,
        items: &[ChartDrawItem<'_>],
    ) -> Result<()> {
        self.draw(clear, items)?;
        #[cfg(target_arch = "wasm32")]
        crate::init::wait_browser_submitted_work(self.inner.queue())
            .await
            .map_err(|reason| FiggyError::GpuResourceAllocationFailed {
                resource: "figgy first frame completion",
                reason,
            })?;
        #[cfg(not(target_arch = "wasm32"))]
        self.inner.wait_submitted_work().await;
        Ok(())
    }

    /// Alias for [`Self::first_frame_ready`].
    pub async fn warm_up(
        &mut self,
        clear: crate::color::Color,
        items: &[ChartDrawItem<'_>],
    ) -> Result<()> {
        self.first_frame_ready(clear, items).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wait until every retired GPU charge of `renderer` has completed. A
    /// completion callback can run on another test's polling thread on the
    /// shared device, so one `poll(Wait)` does not guarantee it has run yet.
    fn settle_gpu_retirement(renderer: &mut Renderer) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while renderer.gpu_memory_usage().retired_bytes() != 0 {
            renderer.service_gpu_completions().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "retirement callback timeout"
            );
            std::thread::yield_now();
        }
    }
    use crate::data::Column;
    use crate::data_config::{DataErrorBarPointStyleOverride, DataScatterPointStyleOverride};

    #[test]
    fn every_line_preset_packs_without_gpu_dash_truncation() {
        for preset in [
            LineStylePreset::Solid,
            LineStylePreset::Dash,
            LineStylePreset::Dot,
            LineStylePreset::DashDot,
            LineStylePreset::DashDotDot,
            LineStylePreset::ShortDash,
            LineStylePreset::ShortDot,
            LineStylePreset::ShortDashDot,
            LineStylePreset::LongDash,
            LineStylePreset::LongDashDot,
            LineStylePreset::LongDashDotDot,
        ] {
            let mut style = PrimitiveStyle::from_color(Color::BLACK);
            style.pack_dash_pattern(preset.pattern(), 1.5);
            let packed: Vec<f32> = style
                .dash
                .iter()
                .flatten()
                .copied()
                .take(style.dash_len as usize)
                .collect();
            let expected: Vec<f32> = preset.pattern().iter().map(|value| value * 1.5).collect();
            assert_eq!(
                style.dash_len as usize,
                preset.pattern().len(),
                "{preset:?}"
            );
            assert_eq!(packed, expected, "{preset:?}");
        }
    }

    fn col_f64(data: Vec<f64>) -> Column<f64> {
        let min = data.iter().copied().fold(f64::INFINITY, f64::min);
        let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Column { data, min, max }
    }

    fn demo_test_columns() -> [Column<f64>; 4] {
        [
            col_f64(vec![0.0, 1.0, 2.0]),
            col_f64(vec![20.0, 50.0, 80.0]),
            col_f64(vec![0.0, 2.5, 5.0]),
            col_f64(vec![0.0, 4.0, 5.0]),
        ]
    }

    fn revision_snapshot(renderer: &Renderer, id: ChartId) -> [RenderRevision; 7] {
        let revisions = renderer.chart_states[&id].revisions;
        [
            revisions.desired,
            revisions.config,
            revisions.series,
            revisions.data,
            revisions.view,
            revisions.selection,
            revisions.raster,
        ]
    }

    fn test_scatter_style() -> DataScatterStyleConfig {
        DataScatterStyleConfig {
            point_color: Color::new(0.0, 0.0, 0.0, 1.0),
            point_shape: crate::data_config::ScatterShape::CircleFilled,
            point_size: 5.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        }
    }

    fn test_errorbar_style() -> DataErrorBarStyleConfig {
        DataErrorBarStyleConfig {
            error_bar_color: Color::new(1.0, 0.0, 0.0, 1.0),
            error_bar_width: 1.0,
            error_bar_cap_size: 4.0,
            cap_width: 1.0,
            error_bar_style_table: None,
            error_bar_style_index_column: None,
            error_bar_style_overrides: None,
        }
    }

    fn state_test_renderer_with_capacity(pool_capacity: u64) -> Option<Renderer> {
        let (device, queue) = crate::data_render::shared_device()?;
        Some(
            Renderer::try_new(
                RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
                wgpu::TextureFormat::Rgba8Unorm,
                pool_capacity,
            )
            .expect("required GPU test: renderer initialization failed"),
        )
    }

    fn state_test_renderer() -> Option<Renderer> {
        state_test_renderer_with_capacity(1024 * 1024)
    }

    fn state_test_stream(id: &str, revision: u64) -> crate::StreamColumn {
        crate::StreamColumn {
            id: id.into(),
            len: 100,
            encoding: crate::StreamEncoding::HiLoF32,
            replay: crate::StreamReplay::Sequential,
            revision,
            statistics: crate::StreamStatistics::Unknown,
        }
    }

    struct FailingPromotionSource {
        len: usize,
    }

    impl ColumnSource for FailingPromotionSource {
        fn len(&self) -> usize {
            self.len
        }

        fn min(&self) -> f64 {
            0.0
        }

        fn max(&self) -> f64 {
            1.0
        }

        fn write_f32_le_into(&self, _dst: &mut [u8]) {
            unreachable!("promotion uses the range capability")
        }

        fn write_f32_pair_le_into_with_stats(
            &self,
            _dst: crate::data::ColumnPairWriter<'_>,
        ) -> crate::data::ColumnUploadStats {
            unreachable!("promotion uses the range capability")
        }

        fn write_f32_pair_range_into_with_stats(
            &self,
            _start: u64,
            _dst: crate::data::ColumnPairWriter<'_>,
        ) -> std::result::Result<Option<crate::StreamBounds>, crate::data::ColumnRangeWriteError>
        {
            Err(crate::data::ColumnRangeWriteError::SourceFailed)
        }
    }

    struct MutableLenPromotionSource {
        calls: std::cell::Cell<usize>,
    }

    impl ColumnSource for MutableLenPromotionSource {
        fn len(&self) -> usize {
            let call = self.calls.get();
            self.calls.set(call + 1);
            if call == 0 { 1 } else { usize::MAX }
        }

        fn min(&self) -> f64 {
            1.0
        }

        fn max(&self) -> f64 {
            1.0
        }

        fn write_f32_le_into(&self, _dst: &mut [u8]) {}

        fn write_f32_pair_le_into_with_stats(
            &self,
            _dst: crate::data::ColumnPairWriter<'_>,
        ) -> crate::data::ColumnUploadStats {
            crate::data::ColumnUploadStats {
                min_positive: Some(1.0),
            }
        }

        fn write_f32_pair_range_into_with_stats(
            &self,
            _start: u64,
            mut dst: crate::data::ColumnPairWriter<'_>,
        ) -> std::result::Result<Option<crate::StreamBounds>, crate::data::ColumnRangeWriteError>
        {
            assert_eq!(dst.len(), 1, "promotion must stage the frozen length");
            dst.write_pair(0, 1.0, 0.0);
            Ok(Some(crate::StreamBounds {
                min: 1.0,
                max: 1.0,
                min_positive: Some(1.0),
            }))
        }
    }

    struct PanickingPromotionSource;

    impl HiLoColumnSource for PanickingPromotionSource {
        fn len(&self) -> usize {
            1
        }

        fn min(&self) -> f64 {
            1.0
        }

        fn max(&self) -> f64 {
            1.0
        }

        fn write_f32_pair_le_into(&self, _dst: &mut [u8]) {}

        fn write_f32_pair_le_into_with_stats(
            &self,
            _dst: crate::data::ColumnPairWriter<'_>,
        ) -> crate::data::ColumnUploadStats {
            crate::data::ColumnUploadStats {
                min_positive: Some(1.0),
            }
        }

        fn write_f32_pair_range_into_with_stats(
            &self,
            _start: u64,
            _dst: crate::data::ColumnPairWriter<'_>,
        ) -> std::result::Result<Option<crate::StreamBounds>, crate::data::ColumnRangeWriteError>
        {
            panic!("injected promotion writer panic")
        }
    }

    struct InvalidBoundsPromotionSource;

    impl ColumnSource for InvalidBoundsPromotionSource {
        fn len(&self) -> usize {
            1
        }

        fn min(&self) -> f64 {
            1.0
        }

        fn max(&self) -> f64 {
            1.0
        }

        fn write_f32_le_into(&self, _dst: &mut [u8]) {}

        fn write_f32_pair_le_into_with_stats(
            &self,
            _dst: crate::data::ColumnPairWriter<'_>,
        ) -> crate::data::ColumnUploadStats {
            crate::data::ColumnUploadStats {
                min_positive: Some(1.0),
            }
        }

        fn write_f32_pair_range_into_with_stats(
            &self,
            _start: u64,
            mut dst: crate::data::ColumnPairWriter<'_>,
        ) -> std::result::Result<Option<crate::StreamBounds>, crate::data::ColumnRangeWriteError>
        {
            dst.write_pair(0, 1.0, 0.0);
            Ok(Some(crate::StreamBounds {
                min: 2.0,
                max: 1.0,
                min_positive: None,
            }))
        }
    }

    #[test]
    fn streamed_promotion_freezes_len_and_contains_source_failures() {
        fn setup(id: &str, encoding: crate::StreamEncoding) -> Option<(Renderer, ChartId)> {
            let mut renderer = state_test_renderer_with_capacity(4 * 1024)?;
            let _ = renderer.set_memory_budget(Some(u64::MAX));
            renderer
                .register_streamed_columns(vec![crate::StreamColumn {
                    id: id.into(),
                    len: 1,
                    encoding,
                    replay: crate::StreamReplay::RandomAccess,
                    revision: 1,
                    statistics: crate::StreamStatistics::Unknown,
                }])
                .unwrap();
            let chart = renderer
                .register_chart(
                    crate::default::default_config(),
                    vec![state_test_line("temporary", id, id)],
                )
                .unwrap();
            Some((renderer, chart))
        }

        let Some((mut renderer, chart)) = setup("mutable-len", crate::StreamEncoding::ScalarF32)
        else {
            return;
        };
        let source = MutableLenPromotionSource {
            calls: std::cell::Cell::new(0),
        };
        let before = revision_snapshot(&renderer, chart);
        let pending_before = renderer.pending_defrag;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            renderer.promote_streamed_columns_to_resident(
                &[crate::StreamSourceBinding {
                    id: "mutable-len",
                    revision: 1,
                    source: crate::StreamColumnSource::Scalar(&source),
                }],
                0,
                &[],
                0,
                u64::MAX,
            )
        }));
        let report = result
            .expect("mutable len must not panic")
            .expect("the first validated length must drive the complete promotion");
        assert!(report.is_admissible());
        assert_eq!(
            source.calls.get(),
            1,
            "source length must be read exactly once"
        );
        let slot = renderer.pool.slot("mutable-len").unwrap();
        assert_eq!(slot.len_values, 1);
        assert_eq!(
            (slot.min, slot.max, slot.min_positive),
            (1.0, 1.0, Some(1.0))
        );
        assert!(!renderer.streaming_sources.contains_key("mutable-len"));
        assert_eq!(renderer.stream_revision_lineage["mutable-len"], 1);
        assert_ne!(revision_snapshot(&renderer, chart), before);
        assert_eq!(renderer.pending_defrag, pending_before);

        let Some((mut renderer, chart)) = setup("panicking-writer", crate::StreamEncoding::HiLoF32)
        else {
            return;
        };
        let before = revision_snapshot(&renderer, chart);
        let used_before = renderer.pool.used_bytes();
        let layout_before = renderer.pool.layout_generation();
        let pending_before = renderer.pending_defrag;
        let picker_before = renderer.picker.active.is_some();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            renderer.promote_streamed_columns_to_resident(
                &[crate::StreamSourceBinding {
                    id: "panicking-writer",
                    revision: 1,
                    source: crate::StreamColumnSource::HiLo(&PanickingPromotionSource),
                }],
                0,
                &[],
                0,
                u64::MAX,
            )
        }));
        assert!(matches!(
            result.expect("writer panic must be contained"),
            Err(FiggyError::Pool(
                data_render::AllocError::AllocationFailed {
                    resource: "streamed promotion source",
                    ..
                }
            ))
        ));
        assert!(renderer.pool.slot("panicking-writer").is_none());
        assert!(renderer.streaming_sources.contains_key("panicking-writer"));
        assert_eq!(renderer.stream_revision_lineage["panicking-writer"], 1);
        assert_eq!(renderer.pool.used_bytes(), used_before);
        assert_eq!(renderer.pool.layout_generation(), layout_before);
        assert_eq!(revision_snapshot(&renderer, chart), before);
        assert_eq!(renderer.pending_defrag, pending_before);
        assert_eq!(renderer.picker.active.is_some(), picker_before);

        let Some((mut renderer, chart)) = setup("invalid-bounds", crate::StreamEncoding::ScalarF32)
        else {
            return;
        };
        let before = revision_snapshot(&renderer, chart);
        let used_before = renderer.pool.used_bytes();
        let layout_before = renderer.pool.layout_generation();
        let pending_before = renderer.pending_defrag;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            renderer.promote_streamed_columns_to_resident(
                &[crate::StreamSourceBinding {
                    id: "invalid-bounds",
                    revision: 1,
                    source: crate::StreamColumnSource::Scalar(&InvalidBoundsPromotionSource),
                }],
                0,
                &[],
                0,
                u64::MAX,
            )
        }));
        assert!(matches!(
            result.expect("invalid statistics must not panic"),
            Err(FiggyError::Pool(
                data_render::AllocError::AllocationFailed {
                    resource: "streamed promotion source",
                    ..
                }
            ))
        ));
        assert!(renderer.pool.slot("invalid-bounds").is_none());
        assert!(renderer.streaming_sources.contains_key("invalid-bounds"));
        assert_eq!(renderer.stream_revision_lineage["invalid-bounds"], 1);
        assert_eq!(renderer.pool.used_bytes(), used_before);
        assert_eq!(renderer.pool.layout_generation(), layout_before);
        assert_eq!(revision_snapshot(&renderer, chart), before);
        assert_eq!(renderer.pending_defrag, pending_before);
    }

    #[test]
    fn stream_statistics_cache_coalesces_coverage_and_encoded_bounds() {
        let mut cache = StreamStatisticsCache::default();
        cache.insert_measured(
            4..8,
            Some(crate::StreamBounds {
                min: -99.0,
                max: 99.0,
                min_positive: Some(99.0),
            }),
        );
        assert_eq!(cache.covered, vec![4..8]);
        assert_eq!(cache.uncovered(0..12).unwrap(), vec![0..4, 8..12]);
        assert_eq!(cache.bounds.unwrap().max, 99.0);
        cache.insert_measured(
            0..4,
            Some(crate::StreamBounds {
                min: -2.0,
                max: 7.0,
                min_positive: Some(0.5),
            }),
        );
        assert!(cache.covers(5..7));
        assert!(!cache.covers(0..12));
        assert_eq!(cache.covered, vec![0..8]);
        cache.insert_measured(
            8..12,
            Some(crate::StreamBounds {
                min: 1.0,
                max: 9.0,
                min_positive: Some(1.0),
            }),
        );
        assert_eq!(cache.covered, vec![0..12]);
        assert_eq!(
            cache.bounds,
            Some(crate::StreamBounds {
                min: -99.0,
                max: 99.0,
                min_positive: Some(0.5),
            })
        );

        let fragmented = StreamStatisticsCache {
            covered: (0..MAX_STREAM_STATISTICS_RANGES as u64)
                .map(|index| index * 2..index * 2 + 1)
                .collect(),
            bounds: None,
        };
        assert_eq!(
            fragmented.coverage_len_after_insert(
                &(MAX_STREAM_STATISTICS_RANGES as u64 * 2
                    ..MAX_STREAM_STATISTICS_RANGES as u64 * 2 + 1)
            ),
            MAX_STREAM_STATISTICS_RANGES + 1
        );
        assert_eq!(
            fragmented.coverage_len_after_insert(&(1..MAX_STREAM_STATISTICS_RANGES as u64 * 2)),
            1,
            "a bridge is allowed to collapse even a full fragmented cache"
        );
    }

    #[test]
    fn streamed_statistics_and_provisional_fit_preserve_revision_contract() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .register_streamed_columns(vec![state_test_stream("s", 1)])
            .unwrap();
        let series = state_test_line("streamed", "s", "s");
        let chart = renderer
            .register_chart(crate::default::default_config(), vec![series.clone()])
            .unwrap();
        assert!(matches!(
            renderer.begin_errorbar_extent("s", "s", "s"),
            Err(crate::GpuErrorbarError::ColumnNotResident { .. })
        ));
        let columns = crate::GpuSeriesExtentColumnIds {
            x: "s",
            y: "s",
            x_lower: None,
            x_upper: None,
            y_lower: None,
            y_upper: None,
        };
        let old = renderer.chart_states[&chart].revisions.desired;
        {
            let guard = renderer
                .begin_upsert_column("resident", &col_f64(vec![1.0, 2.0]))
                .unwrap();
            assert!(matches!(
                guard.web_derived_snapshot(chart),
                Err(FiggyError::ColumnNotResident { .. })
            ));
            assert!(matches!(
                guard.begin_series_fit_extent(&series),
                Err(crate::GpuErrorbarError::ColumnNotResident { .. })
            ));
            assert!(matches!(
                guard.begin_errorbar_extent("s", "s", "s"),
                Err(crate::GpuErrorbarError::ColumnNotResident { .. })
            ));
            assert!(matches!(
                guard.begin_series_extent(crate::GpuSeriesExtentMode::Line, columns),
                Err(crate::GpuErrorbarError::ColumnNotResident { .. })
            ));
        }
        assert!(renderer.logical_column("resident").is_none());
        assert_eq!(renderer.chart_states[&chart].revisions.desired, old);
        let bounds = Some(crate::StreamBounds {
            min: -1.0,
            max: 2.0,
            min_positive: Some(0.5),
        });
        assert!(matches!(
            renderer.commit_streamed_statistics("s", 2, bounds),
            Err(FiggyError::StaleStateToken { .. })
        ));
        assert_eq!(
            renderer.streaming_sources["s"].statistics,
            crate::StreamStatistics::Unknown
        );
        let fit_epoch = renderer.next_stream_source_fit_epoch;
        renderer.next_stream_source_fit_epoch = u64::MAX;
        assert!(matches!(
            renderer.commit_streamed_statistics("s", 1, bounds),
            Err(FiggyError::CounterExhausted { .. })
        ));
        assert_eq!(
            renderer.streaming_sources["s"].statistics,
            crate::StreamStatistics::Unknown
        );
        renderer.next_stream_source_fit_epoch = fit_epoch;
        renderer.commit_streamed_statistics("s", 1, bounds).unwrap();
        assert_eq!(
            renderer.streaming_sources["s"].statistics,
            crate::StreamStatistics::Known(bounds)
        );
        let committed = renderer.chart_render_stamp(chart).unwrap();
        renderer.commit_streamed_statistics("s", 1, bounds).unwrap();
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), committed);
        assert!(renderer.commit_streamed_statistics("s", 1, None).is_err());
    }

    #[test]
    fn streamed_fit_token_uses_live_source_revision_and_registration_epoch() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .register_streamed_columns(vec![state_test_stream("s", 1)])
            .unwrap();
        let series = state_test_line("streamed", "s", "s");
        let chart = renderer
            .register_chart(crate::default::default_config(), vec![series.clone()])
            .unwrap();
        assert_eq!(
            renderer.logical_column("s").unwrap().statistics(),
            crate::StreamStatistics::Unknown
        );
        let before_statistics = renderer.begin_fit_commit(chart).unwrap();
        let bounds = crate::StreamBounds {
            min: 1.0,
            max: 3.0,
            min_positive: Some(1.0),
        };
        renderer
            .commit_streamed_statistics("s", 1, Some(bounds))
            .unwrap();
        assert_eq!(
            renderer.logical_column("s").unwrap().statistics(),
            crate::StreamStatistics::Known(Some(bounds))
        );
        let extent = crate::FitExtent {
            min: bounds.min,
            max: bounds.max,
            min_positive: bounds.min_positive,
        };
        assert!(matches!(
            renderer.commit_auto_fit_all_if_current(&before_statistics, &extent, &extent, 0.0),
            Err(FiggyError::StaleStateToken { .. })
        ));
        let known = renderer.begin_fit_commit(chart).unwrap();
        renderer
            .commit_auto_fit_all_if_current(&known, &extent, &extent, 0.0)
            .unwrap();
        assert_eq!(renderer.chart_config(chart).unwrap().bottom_x.min, 1.0);
        assert_eq!(renderer.chart_config(chart).unwrap().bottom_x.max, 3.0);
        let current = renderer.begin_fit_commit(chart).unwrap();
        renderer
            .replace_streamed_columns(vec![state_test_stream("s", 2)])
            .unwrap();
        assert!(matches!(
            renderer.commit_auto_fit_all_if_current(&current, &extent, &extent, 0.0),
            Err(FiggyError::StaleStateToken { .. })
        ));

        let before_reuse = renderer.begin_fit_commit(chart).unwrap();
        let config = renderer.chart_config(chart).unwrap().clone();
        assert!(renderer.remove_column("s").unwrap());
        renderer
            .register_streamed_columns(vec![state_test_stream("s", 2)])
            .unwrap();
        renderer
            .set_chart_state(chart, config, vec![series])
            .unwrap();
        assert!(matches!(
            renderer.commit_auto_fit_all_if_current(&before_reuse, &extent, &extent, 0.0),
            Err(FiggyError::StaleStateToken { .. })
        ));
    }

    #[test]
    fn streamed_source_revision_failure_and_remove_are_chart_atomic() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .register_streamed_columns(vec![state_test_stream("s", 1), state_test_stream("t", 1)])
            .unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("s", "s", "s")],
            )
            .unwrap();
        let other = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("t", "t", "t")],
            )
            .unwrap();
        let other_stamp = renderer.chart_render_stamp(other).unwrap();
        let old = renderer.chart_states[&chart].revisions.desired;
        renderer
            .chart_states
            .get_mut(&chart)
            .unwrap()
            .revisions
            .desired
            .sequence = u64::MAX;
        assert!(matches!(
            renderer.replace_streamed_columns(vec![state_test_stream("s", 2)]),
            Err(FiggyError::CounterExhausted { .. })
        ));
        assert_eq!(renderer.streaming_sources["s"].revision, 1);
        assert!(renderer.remove_column("s").is_err());
        assert!(renderer.logical_column("s").is_some());
        assert_eq!(renderer.chart_states[&chart].series.len(), 1);
        renderer
            .chart_states
            .get_mut(&chart)
            .unwrap()
            .revisions
            .desired = old;
        renderer
            .replace_streamed_columns(vec![state_test_stream("s", 2)])
            .unwrap();
        assert_ne!(renderer.chart_states[&chart].revisions.desired, old);
        assert_eq!(renderer.chart_render_stamp(other).unwrap(), other_stamp);
        let fit_token = renderer.begin_fit_commit(chart).unwrap();
        assert!(renderer.remove_column("s").unwrap());
        assert!(renderer.chart_states[&chart].series.is_empty());
        assert!(renderer.logical_column("s").is_none());
        assert!(matches!(
            renderer.commit_auto_fit_all_if_current(
                &fit_token,
                &crate::FitExtent::EMPTY,
                &crate::FitExtent::EMPTY,
                0.0,
            ),
            Err(FiggyError::StaleStateToken { .. })
        ));
        assert_eq!(renderer.chart_render_stamp(other).unwrap(), other_stamp);
    }

    #[test]
    fn streamed_series_set_detaches_only_affected_picker_after_success() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![1.0, 2.0]);
        renderer
            .register_streamed_columns(vec![state_test_stream("s", 1)])
            .unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("resident", "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let unrelated = renderer
            .register_chart(crate::default::default_config(), vec![])
            .unwrap();
        renderer
            .set_chart_series(unrelated, vec![state_test_line("streamed", "s", "s")])
            .unwrap();
        assert_eq!(active_picker(&renderer).chart_id, chart);
        let old = renderer.chart_states[&chart].revisions.desired;
        renderer
            .chart_states
            .get_mut(&chart)
            .unwrap()
            .revisions
            .desired
            .sequence = u64::MAX;
        assert!(
            renderer
                .set_chart_series(chart, vec![state_test_line("streamed", "s", "s")])
                .is_err()
        );
        assert_eq!(active_picker(&renderer).chart_id, chart);
        renderer
            .chart_states
            .get_mut(&chart)
            .unwrap()
            .revisions
            .desired = old;
        renderer
            .set_chart_series(chart, vec![state_test_line("streamed", "s", "s")])
            .unwrap();
        assert!(renderer.picker.active.is_none());
        assert!(matches!(
            renderer.prepare_gpu_picking_for_chart(chart),
            Err(crate::GpuPickError::ColumnNotResident(_))
        ));
        renderer
            .set_chart_series(chart, vec![state_test_line("resident", "x", "y")])
            .unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        assert_eq!(active_picker(&renderer).chart_id, chart);
    }

    fn state_test_line(id: &str, x: &str, y: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.to_string(),
            source_id: None,
            label: None,
            x_column: x.to_string(),
            y_column: y.to_string(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::new(0.1, 0.2, 0.3, 1.0),
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }
    }

    fn state_test_scatter_line(id: &str, source: Option<&str>, x: &str, y: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.to_string(),
            source_id: source.map(str::to_string),
            label: None,
            x_column: x.to_string(),
            y_column: y.to_string(),
            render_type: DataRenderType::ScatterLine {
                scatter: test_scatter_style(),
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::new(0.1, 0.2, 0.3, 1.0),
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }
    }

    fn state_test_scatter(id: &str, source: Option<&str>, x: &str, y: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.to_string(),
            source_id: source.map(str::to_string),
            label: None,
            x_column: x.to_string(),
            y_column: y.to_string(),
            render_type: DataRenderType::Scatter {
                scatter: test_scatter_style(),
            },
        }
    }

    fn add_state_test_xy(renderer: &mut Renderer, x: &str, y: &str, values: Vec<f64>) {
        renderer.add_column(x, &col_f64(values.clone())).unwrap();
        renderer.add_column(y, &col_f64(values)).unwrap();
    }

    fn active_picker(renderer: &Renderer) -> &ActivePickRegistry {
        renderer
            .picker
            .active
            .as_ref()
            .expect("test prepared an active picker registry")
    }

    fn basic_errorbar_chart() -> Chart {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 4.0);
        chart.set_y_range(0.0, 5.0);
        chart
    }

    fn caps_with_limits(storage_binding: u64, buffer_size: u64) -> RendererDeviceCaps {
        RendererDeviceCaps {
            features: wgpu::Features::empty(),
            max_texture_dimension_2d: 8192,
            max_buffer_size: buffer_size,
            max_storage_buffer_binding_size: storage_binding,
            max_compute_workgroups_per_dimension: 65_535,
            max_compute_invocations_per_workgroup: 256,
        }
    }

    /// The pool must be clamped to whatever the arc-scan / star-pass storage
    /// binding can address, so an oversized request degrades gracefully
    /// instead of panicking on the first dashed line. No GPU needed — this
    /// pins the clamp arithmetic `try_new` relies on.
    #[test]
    fn pool_capacity_clamps_to_storage_and_buffer_limits() {
        let mb = 1024 * 1024;
        let caps = caps_with_limits(128 * mb, 1024 * mb);
        // Comfortably under both limits — untouched.
        assert_eq!(effective_pool_capacity(64 * mb, caps), 64 * mb);
        // Exactly at the storage-binding limit — untouched.
        assert_eq!(effective_pool_capacity(128 * mb, caps), 128 * mb);
        // Over the storage-binding limit (wgpu default) — clamped to it,
        // even though max_buffer_size is far higher. This is the 256 MB pool
        // that used to panic on the first dashed/sketch/milkyway line.
        assert_eq!(effective_pool_capacity(256 * mb, caps), 128 * mb);
        // When max_buffer_size is the tighter of the two, it wins.
        let caps_small_buffer = caps_with_limits(u64::from(u32::MAX), 32 * mb);
        assert_eq!(
            effective_pool_capacity(256 * mb, caps_small_buffer),
            32 * mb
        );
        // A device with a generous binding limit keeps a large pool intact.
        let caps_big = caps_with_limits(u64::from(u32::MAX), 4 * 1024 * mb);
        assert_eq!(effective_pool_capacity(512 * mb, caps_big), 512 * mb);
    }

    #[test]
    /// Enabling picking creates the gate and reduction pipelines and the
    /// typed data pipelines once; the exact point pipelines wait for the first
    /// pick that needs each (`PickExactPipelines`).
    fn renderer_gpu_picker_activation_creates_six_pipelines_once() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let mut events = Vec::new();
        renderer
            .enable_gpu_picking_observed(&mut |event| events.push(event))
            .unwrap();
        renderer
            .enable_gpu_picking_observed(&mut |event| events.push(event))
            .unwrap();

        let pipeline_starts = events
            .iter()
            .filter(|event| {
                event.scope == "renderer.gpu_pick"
                    && event.phase == crate::init::InitPhase::Started
                    && matches!(
                        event.stage,
                        "pick_gate_x"
                            | "pick_gate_y"
                            | "pick_exact_candidates"
                            | "pick_reduce_candidates"
                    )
            })
            .count();
        let setup_starts = events
            .iter()
            .filter(|event| {
                event.scope == "renderer.gpu_pick"
                    && event.phase == crate::init::InitPhase::Started
                    && event.stage == "setup"
            })
            .count();
        let data_pipeline_starts = events
            .iter()
            .filter(|event| {
                event.scope == "renderer.gpu_data_pick"
                    && event.phase == crate::init::InitPhase::Started
                    && matches!(
                        event.stage,
                        "pick_histogram_bin" | "pick_field_data" | "pick_vector_cell"
                    )
            })
            .count();
        let data_setup_starts = events
            .iter()
            .filter(|event| {
                event.scope == "renderer.gpu_data_pick"
                    && event.phase == crate::init::InitPhase::Started
                    && event.stage == "setup"
            })
            .count();
        assert_eq!(pipeline_starts, 3);
        assert_eq!(setup_starts, 1);
        assert_eq!(data_pipeline_starts, 3);
        assert_eq!(data_setup_starts, 1);
        assert!(renderer.picker.active.is_none());
    }

    fn typed_pick_config(x: (f64, f64), y: (f64, f64)) -> Config {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 480,
            height: 360,
        });
        config.legend.visible = false;
        for axis in [&mut config.bottom_x, &mut config.top_x] {
            axis.min = x.0;
            axis.max = x.1;
        }
        for axis in [&mut config.left_y, &mut config.right_y] {
            axis.min = y.0;
            axis.max = y.1;
        }
        config
    }

    fn typed_pick_position(config: &Config, x: f64, y: f64) -> [f32; 2] {
        let area = config.data_area().unwrap().0;
        let tx = ((x - config.bottom_x.min) / (config.bottom_x.max - config.bottom_x.min)) as f32;
        let ty = ((y - config.left_y.min) / (config.left_y.max - config.left_y.min)) as f32;
        [
            area.x as f32 + tx * area.width as f32,
            area.y as f32 + (1.0 - ty) * area.height as f32,
        ]
    }

    fn typed_pick_request(config: &Config, position: [f32; 2]) -> GpuPickRequest {
        GpuPickRequest {
            canvas_position_px: position,
            display_panel_px: config.chart_area.0,
            display_scale: 1.0,
            max_distance_px: 0.0,
        }
    }

    #[test]
    fn typed_gpu_pick_returns_histogram_bin_without_cpu_bar_bounds() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("edges", &col_f64(vec![0.0, 1.0, 2.0]))
            .unwrap();
        renderer
            .add_column("counts", &col_f64(vec![5.0, 8.0]))
            .unwrap();
        let config = typed_pick_config((0.0, 2.0), (0.0, 10.0));
        let position = typed_pick_position(&config, 1.5, 4.0);
        let series = SeriesConfig {
            series_id: "hist".into(),
            source_id: Some("source".into()),
            label: None,
            x_column: "edges".into(),
            y_column: "counts".into(),
            render_type: DataRenderType::Histogram {
                bar: crate::data_config::DataBarStyleConfig {
                    fill_color: Color::BLACK,
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    baseline: 0.0,
                    gap_px: 2.0,
                    width_ratio: 1.0,
                    orientation: crate::data_config::BarOrientation::Vertical,
                    bar_style_overrides: None,
                },
            },
        };
        let chart = renderer
            .register_chart(config.clone(), vec![series])
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        let picked = pollster::block_on(
            renderer
                .pick_chart_data(chart, typed_pick_request(&config, position))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .expect("histogram bin hit");
        assert_eq!(
            picked,
            crate::pick::PickedData::HistogramBin {
                source_id: Some("source".into()),
                series_id: "hist".into(),
                bin_index: 1,
                distance_px: 0.0,
            }
        );
    }

    #[test]
    fn typed_gpu_pick_uses_the_overridden_histogram_width() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("edges", &col_f64(vec![0.0, 1.0, 2.0]))
            .unwrap();
        renderer
            .add_column("counts", &col_f64(vec![5.0, 8.0]))
            .unwrap();
        let config = typed_pick_config((0.0, 2.0), (0.0, 10.0));
        let series = SeriesConfig {
            series_id: "hist".into(),
            source_id: None,
            label: None,
            x_column: "edges".into(),
            y_column: "counts".into(),
            render_type: DataRenderType::Histogram {
                bar: DataBarStyleConfig {
                    fill_color: Color::BLACK,
                    border_color: Color::BLACK,
                    border_width: 0.0,
                    baseline: 0.0,
                    gap_px: 0.0,
                    width_ratio: 1.0,
                    orientation: crate::data_config::BarOrientation::Vertical,
                    bar_style_overrides: Some(vec![crate::data_config::DataBarStyleOverride {
                        index: 1,
                        style: DataBarBinStyleConfig {
                            width_ratio: Some(0.2),
                            ..Default::default()
                        },
                    }]),
                },
            },
        };
        let chart = renderer
            .register_chart(config.clone(), vec![series])
            .unwrap();
        renderer.enable_gpu_picking().unwrap();

        // Raw bin 1 spans x=1..2, but its overridden bar spans x=1.4..1.6.
        let outside = pollster::block_on(
            renderer
                .pick_chart_data(
                    chart,
                    typed_pick_request(&config, typed_pick_position(&config, 1.1, 4.0)),
                )
                .unwrap()
                .resolve(),
        )
        .unwrap();
        assert_eq!(
            outside, None,
            "raw bin area outside the narrow bar was picked"
        );

        let inside = pollster::block_on(
            renderer
                .pick_chart_data(
                    chart,
                    typed_pick_request(&config, typed_pick_position(&config, 1.5, 4.0)),
                )
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .expect("narrow overridden bar hit");
        assert!(matches!(
            inside,
            crate::pick::PickedData::HistogramBin { bin_index: 1, .. }
        ));
    }

    #[test]
    fn typed_gpu_pick_returns_canonical_heatmap_cell_indices() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        for (id, values) in [
            ("x", vec![0.0, 1.0, 2.0]),
            ("y", vec![0.0, 1.0, 2.0]),
            ("z0", vec![1.0, 2.0]),
            ("z1", vec![3.0, 4.0]),
        ] {
            renderer.add_column(id, &col_f64(values)).unwrap();
        }
        let mut config = typed_pick_config((0.0, 2.0), (0.0, 2.0));
        config.colorbar = Some(crate::default::default_colorbar_options());
        let position = typed_pick_position(&config, 1.5, 0.5);
        let series = SeriesConfig {
            series_id: "heat".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Heatmap {
                matrix: crate::data_config::MatrixRef {
                    columns: vec!["z0".into(), "z1".into()],
                    orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                    grid_layout: crate::data_config::GridLayout::Edges,
                },
                fill: crate::data_config::FieldFillConfig {
                    mode: crate::data_config::FillMode::Continuous,
                    shading: crate::data_config::Shading::Flat,
                    opacity: 1.0,
                },
            },
        };
        let chart = renderer
            .register_chart(config.clone(), vec![series])
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        let picked = pollster::block_on(
            renderer
                .pick_chart_data(chart, typed_pick_request(&config, position))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .expect("heatmap cell hit");
        assert_eq!(
            picked,
            crate::pick::PickedData::MatrixCell {
                source_id: None,
                series_id: "heat".into(),
                x_index: 1,
                y_index: 0,
                distance_px: 0.0,
            }
        );
    }

    #[test]
    fn typed_gpu_pick_prefers_heatmap_contour_level_over_containing_cell() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        for (id, values) in [
            ("x", vec![0.0, 1.0]),
            ("y", vec![0.0, 1.0]),
            ("z0", vec![0.0, 0.0]),
            ("z1", vec![1.0, 1.0]),
        ] {
            renderer.add_column(id, &col_f64(values)).unwrap();
        }
        let mut config = typed_pick_config((-0.5, 1.5), (-0.5, 1.5));
        config.colorbar = Some(crate::default::default_colorbar_options());
        let position = typed_pick_position(&config, 0.5, 0.5);
        let series = SeriesConfig {
            series_id: "field".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::HeatmapContour {
                matrix: crate::data_config::MatrixRef {
                    columns: vec!["z0".into(), "z1".into()],
                    orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                    grid_layout: crate::data_config::GridLayout::Centers,
                },
                fill: crate::data_config::FieldFillConfig {
                    mode: crate::data_config::FillMode::Continuous,
                    shading: crate::data_config::Shading::Interpolated,
                    opacity: 1.0,
                },
                contour: crate::data_config::ContourConfig {
                    levels: vec![0.5],
                    line: DataLineStyleConfig {
                        line_style: LineStylePreset::Solid,
                        line_color: Color::BLACK,
                        line_width: 2.0,
                        fill: None,
                        fill_color: None,
                        interpolation: crate::data_config::LineInterpolation::Linear,
                    },
                    per_level_color: None,
                    labels: None,
                },
            },
        };
        let chart = renderer
            .register_chart(config.clone(), vec![series])
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        let picked = pollster::block_on(
            renderer
                .pick_chart_data(chart, typed_pick_request(&config, position))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .expect("contour hit");
        assert_eq!(
            picked,
            crate::pick::PickedData::ContourLevel {
                source_id: None,
                series_id: "field".into(),
                level_index: 0,
                x_index: 0,
                y_index: 0,
                distance_px: 0.0,
            }
        );
    }

    #[test]
    fn renderer_gpu_picker_activation_failure_is_sticky_without_retrying_creation() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .picker
            .fail_activation_once(crate::gpu_pick::GpuPickError::DeviceLimit {
                resource: "injected picker limit",
                requested: 17,
                limit: 9,
            });
        let mut events = Vec::new();
        for _ in 0..2 {
            let error = renderer
                .enable_gpu_picking_observed(&mut |event| events.push(event))
                .unwrap_err();
            assert!(matches!(
                error,
                crate::gpu_pick::GpuPickError::DeviceLimit {
                    resource: "injected picker limit",
                    requested: 17,
                    limit: 9,
                }
            ));
        }
        assert!(events.is_empty());
        assert!(matches!(
            &renderer.picker.pipeline_state,
            PickerPipelineState::Failed(crate::gpu_pick::GpuPickError::DeviceLimit {
                resource: "injected picker limit",
                requested: 17,
                limit: 9,
            })
        ));
    }

    #[test]
    fn active_picker_ignores_unrelated_mutations_and_reuses_source_only_changes() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let config = crate::default::default_config();
        let active_chart = renderer
            .register_chart(
                config.clone(),
                vec![state_test_scatter_line("series", Some("old"), "x", "y")],
            )
            .unwrap();
        let inactive_chart = renderer
            .register_chart(
                config.clone(),
                vec![state_test_scatter_line("other", None, "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer
            .prepare_gpu_picking_for_chart(active_chart)
            .unwrap();

        let generation = active_picker(&renderer).engine.registry_generation();
        let query_buffer = active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap();

        let mut inactive_series = renderer.chart_series(inactive_chart).unwrap()[0].clone();
        let DataRenderType::ScatterLine { line, .. } = &mut inactive_series.render_type else {
            unreachable!();
        };
        line.line_width = 8.0;
        renderer
            .set_chart_series(inactive_chart, vec![inactive_series])
            .unwrap();
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            generation
        );

        let mut decoration_only = config.clone();
        decoration_only.chart_title.text.segments =
            crate::text::rich_segments_from_text("new title");
        decoration_only.bottom_x.min = -10.0;
        decoration_only.bottom_x.max = 10.0;
        renderer
            .set_chart_config(active_chart, decoration_only)
            .unwrap();
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            generation
        );
        assert_eq!(
            active_picker(&renderer)
                .engine
                .query_params_buffer(0)
                .unwrap(),
            query_buffer
        );

        let mut identity_only = renderer.chart_series(active_chart).unwrap()[0].clone();
        identity_only.source_id = Some("new".into());
        renderer
            .set_chart_series(active_chart, vec![identity_only])
            .unwrap();
        assert!(active_picker(&renderer).engine.registry_generation() > generation);
        assert_eq!(
            active_picker(&renderer)
                .engine
                .query_params_buffer(0)
                .unwrap(),
            query_buffer
        );
        assert_eq!(
            active_picker(&renderer).signature.slots[0]
                .source_id
                .as_deref(),
            Some("new")
        );
    }

    #[test]
    fn precise_mode_without_point_mapping_reuses_stylized_picker_resources() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let config = crate::default::default_config();
        let chart = renderer
            .register_chart(config, vec![state_test_scatter("scatter", None, "x", "y")])
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let generation = active_picker(&renderer).engine.registry_generation();
        let query_buffer = active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap();
        assert!(matches!(
            active_picker(&renderer).signature.slots[0]
                .resource
                .scatter
                .as_ref()
                .unwrap()
                .style_mapping,
            PickStyleMappingSignature::Inactive
        ));

        let mut sketch = renderer.chart_config(chart).unwrap().clone();
        sketch.draw_style = DrawStyle::Sketch(Default::default());
        renderer.set_chart_config(chart, sketch).unwrap();

        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            generation
        );
        assert_eq!(
            active_picker(&renderer)
                .engine
                .query_params_buffer(0)
                .unwrap(),
            query_buffer
        );
        assert_eq!(
            active_picker(&renderer).signature.slots[0]
                .resource
                .primitives,
            EffectiveSeriesPrimitives {
                field: false,
                contour: false,
                line: false,
                scatter: true,
                bar: false,
                vector: false,
            }
        );
    }

    #[test]
    fn constellation_picker_support_matches_rendered_scatter_line_only() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let mut config = crate::default::default_config();
        config.draw_style = DrawStyle::Constellation(Default::default());
        let chart = renderer
            .register_chart(
                config,
                vec![
                    state_test_line("line", "x", "y"),
                    state_test_scatter("scatter", None, "x", "y"),
                    state_test_scatter_line("scatter-line", None, "x", "y"),
                ],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();

        let active = active_picker(&renderer);
        assert_eq!(active.engine.registered_series_count(), 1);
        assert_eq!(active.signature.slots.len(), 1);
        assert_eq!(active.signature.slots[0].series_id, "scatter-line");

        let mut precise = renderer.chart_config(chart).unwrap().clone();
        precise.draw_style = DrawStyle::Precise;
        renderer.set_chart_config(chart, precise).unwrap();
        assert_eq!(active_picker(&renderer).engine.registered_series_count(), 3);
    }

    #[test]
    fn picker_switches_charts_without_recreating_equal_series_resources() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let config = crate::default::default_config();
        let first = renderer
            .register_chart(
                config.clone(),
                vec![state_test_scatter("same", Some("first"), "x", "y")],
            )
            .unwrap();
        let second = renderer
            .register_chart(
                config,
                vec![state_test_scatter("same", Some("second"), "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(first).unwrap();
        let first_buffer = active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap();

        renderer.prepare_gpu_picking_for_chart(second).unwrap();
        assert_eq!(active_picker(&renderer).chart_id, second);
        assert_eq!(
            active_picker(&renderer)
                .engine
                .query_params_buffer(0)
                .unwrap(),
            first_buffer
        );
        assert_eq!(
            active_picker(&renderer).signature.slots[0]
                .source_id
                .as_deref(),
            Some("second")
        );

        renderer.prepare_gpu_picking_for_chart(first).unwrap();
        assert_eq!(active_picker(&renderer).chart_id, first);
        assert_eq!(
            active_picker(&renderer)
                .engine
                .query_params_buffer(0)
                .unwrap(),
            first_buffer
        );
    }

    #[test]
    fn picker_reorder_reuses_gpu_slots_and_publishes_new_identity_order() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let initial = vec![
            state_test_scatter("a", Some("source-a0"), "x", "y"),
            state_test_scatter("b", Some("source-b0"), "x", "y"),
        ];
        let chart = renderer
            .register_chart(crate::default::default_config(), initial)
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let a_buffer = active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap();
        let b_buffer = active_picker(&renderer)
            .engine
            .query_params_buffer(1)
            .unwrap();

        renderer
            .set_chart_series(
                chart,
                vec![
                    state_test_scatter("b", Some("source-b1"), "x", "y"),
                    state_test_scatter("a", Some("source-a1"), "x", "y"),
                ],
            )
            .unwrap();

        let active = active_picker(&renderer);
        assert_eq!(active.engine.query_params_buffer(0).unwrap(), b_buffer);
        assert_eq!(active.engine.query_params_buffer(1).unwrap(), a_buffer);
        assert_eq!(
            active.engine.identity_snapshot(),
            vec![
                (Some("source-b1".into()), "b".into()),
                (Some("source-a1".into()), "a".into()),
            ]
        );
        assert_eq!(
            active
                .signature
                .slots
                .iter()
                .map(|slot| slot.series_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
    }

    #[test]
    fn composite_picker_prepare_failure_preserves_all_state_and_retry_succeeds() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let initial_config = crate::default::default_config();
        let initial_series = vec![state_test_line("line", "x", "y")];
        let chart = renderer
            .register_chart(initial_config.clone(), initial_series.clone())
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let generation = active_picker(&renderer).engine.registry_generation();
        let signature = active_picker(&renderer).signature.clone();
        let render_stamp = renderer.chart_render_stamp(chart).unwrap();
        let visual_revision = renderer.visual_revision();
        active_picker(&renderer).engine.fail_next_registry_prepare();

        let mut changed_series = initial_series.clone();
        let DataRenderType::Line { line } = &mut changed_series[0].render_type else {
            unreachable!();
        };
        line.line_width = 9.0;
        let mut changed_config = initial_config.clone();
        changed_config.chart_title.text.segments =
            crate::text::rich_segments_from_text("prepared but not published");
        assert!(matches!(
            renderer.set_chart_state(chart, changed_config.clone(), changed_series.clone()),
            Err(FiggyError::GpuPick(
                crate::gpu_pick::GpuPickError::AllocationFailed {
                    resource: "injected registry prepare failure"
                }
            ))
        ));
        assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
        assert_eq!(
            renderer.chart_series(chart).unwrap(),
            initial_series.as_slice()
        );
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
        assert_eq!(renderer.visual_revision(), visual_revision);
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            generation
        );
        assert_eq!(active_picker(&renderer).signature, signature);

        renderer
            .set_chart_state(chart, changed_config.clone(), changed_series.clone())
            .unwrap();
        assert_eq!(renderer.chart_config(chart).unwrap(), &changed_config);
        assert_eq!(
            renderer.chart_series(chart).unwrap(),
            changed_series.as_slice()
        );
        assert_ne!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
        assert_ne!(renderer.visual_revision(), visual_revision);
        assert!(active_picker(&renderer).engine.registry_generation() > generation);
        assert_ne!(active_picker(&renderer).signature, signature);
    }

    #[test]
    fn picker_ticket_resolves_after_chart_registry_pool_and_renderer_drop() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![0.0])).unwrap();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        config.bottom_x.min = -1.0;
        config.bottom_x.max = 1.0;
        config.left_y.min = -1.0;
        config.left_y.max = 1.0;
        let chart = renderer
            .register_chart(
                config.clone(),
                vec![state_test_scatter("point", Some("source"), "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        let (display, panel, scale) = display_config_for_surface(&config, (320, 240));
        let area = display.data_area().unwrap().0;
        let ticket = renderer
            .pick_chart(
                chart,
                GpuPickRequest {
                    canvas_position_px: [
                        area.x as f32 + area.width as f32 * 0.5,
                        area.y as f32 + area.height as f32 * 0.5,
                    ],
                    display_panel_px: panel,
                    display_scale: scale,
                    max_distance_px: 20.0,
                },
            )
            .unwrap();
        renderer.remove_chart(chart).unwrap();
        drop(renderer);

        let picked = pollster::block_on(ticket.resolve()).unwrap().unwrap();
        assert_eq!(picked.source_id.as_deref(), Some("source"));
        assert_eq!(picked.series_id, "point");
        assert_eq!(picked.point_index, 0);
    }

    #[test]
    fn renderer_owned_chart_ids_are_ordered_and_never_reused() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let first = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        let second = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        assert_eq!(renderer.chart_order(), &[first, second]);

        renderer.remove_chart(first).unwrap();
        let third = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        assert_ne!(first, third);
        assert_eq!(renderer.chart_order(), &[second, third]);
        assert!(matches!(
            renderer.chart_config(first),
            Err(FiggyError::UnknownChart { .. })
        ));

        let Some(mut other_renderer) = state_test_renderer() else {
            return;
        };
        let other_first = other_renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        assert_ne!(first, other_first);
        assert_ne!(renderer.visual_revision(), other_renderer.visual_revision());
        assert_ne!(renderer.visual_stamp(), other_renderer.visual_stamp());
        assert_eq!(
            renderer.visual_stamp().font_generation,
            crate::text_render::font_generation()
        );
        assert!(matches!(
            other_renderer.chart_config(first),
            Err(FiggyError::UnknownChart { .. })
        ));
    }

    #[test]
    fn renderer_chart_id_exhaustion_is_failure_atomic() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.next_chart_id = u64::MAX;
        let before_visual = renderer.visual_revision();

        assert!(matches!(
            renderer.register_chart(crate::default::default_config(), Vec::new()),
            Err(FiggyError::CounterExhausted {
                counter: "chart id"
            })
        ));
        assert!(renderer.chart_order().is_empty());
        assert!(renderer.chart_states.is_empty());
        assert_eq!(renderer.visual_revision(), before_visual);
    }

    fn state_test_heatmap(id: &str, x: &str, y: &str, z: &[&str]) -> SeriesConfig {
        SeriesConfig {
            series_id: id.to_string(),
            source_id: None,
            label: None,
            x_column: x.to_string(),
            y_column: y.to_string(),
            render_type: DataRenderType::Heatmap {
                matrix: crate::data_config::MatrixRef {
                    columns: z.iter().map(|c| c.to_string()).collect(),
                    orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                    grid_layout: crate::data_config::GridLayout::Centers,
                },
                fill: crate::data_config::FieldFillConfig {
                    mode: crate::data_config::FillMode::Continuous,
                    shading: crate::data_config::Shading::Flat,
                    opacity: 1.0,
                },
            },
        }
    }

    fn state_test_contour(id: &str, x: &str, y: &str, z: &[&str], levels: usize) -> SeriesConfig {
        let mut series = state_test_heatmap(id, x, y, z);
        let DataRenderType::Heatmap { matrix, .. } = series.render_type else {
            unreachable!();
        };
        series.render_type = DataRenderType::Contour {
            matrix,
            contour: state_test_contour_config(levels),
        };
        series
    }

    fn state_test_heatmap_contour(
        id: &str,
        x: &str,
        y: &str,
        z: &[&str],
        levels: usize,
    ) -> SeriesConfig {
        let mut series = state_test_heatmap(id, x, y, z);
        let DataRenderType::Heatmap { matrix, fill } = series.render_type else {
            unreachable!();
        };
        series.render_type = DataRenderType::HeatmapContour {
            matrix,
            fill,
            contour: state_test_contour_config(levels),
        };
        series
    }

    fn state_test_contour_config(levels: usize) -> crate::data_config::ContourConfig {
        crate::data_config::ContourConfig {
            levels: (0..levels).map(|level| level as f64).collect(),
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: None,
            labels: None,
        }
    }

    fn state_test_labelled_contour(
        spacing_px: f32,
        anchors: Vec<crate::data_config::ContourLabelAnchor>,
    ) -> SeriesConfig {
        let mut series =
            state_test_contour("label-cache", "clx", "cly", &["clz0", "clz1", "clz2"], 1);
        let DataRenderType::Contour { contour, .. } = &mut series.render_type else {
            unreachable!();
        };
        contour.levels = vec![2.0];
        contour.line.line_color = Color::new(0.0, 0.2, 0.8, 1.0);
        contour.labels = Some(crate::data_config::ContourLabelConfig {
            visible: true,
            font_size: 18.0,
            color: Color::BLACK,
            format: crate::format::LabelFormat::Decimal,
            significant_digits: 3,
            spacing_px,
            anchors,
            bg_color: Some(Color::new(1.0, 0.0, 1.0, 1.0)),
            bg_padding_px: 2.0,
        });
        series
    }

    fn state_test_contour_labels_mut(
        series: &mut SeriesConfig,
    ) -> &mut crate::data_config::ContourLabelConfig {
        let (DataRenderType::Contour { contour, .. }
        | DataRenderType::HeatmapContour { contour, .. }) = &mut series.render_type
        else {
            panic!("expected contour series")
        };
        contour.labels.as_mut().expect("labelled contour")
    }

    fn assert_contour_spacing_error(result: Result<impl Sized>, product: bool) {
        let Err(FiggyError::InvalidSeriesConfig { reason, .. }) = result else {
            panic!("invalid contour spacing was accepted or returned the wrong error")
        };
        let expected = if product {
            "automatic contour label spacing_px * scale must be finite and greater than zero"
        } else {
            crate::data_config::ContourLabelConfig::INVALID_SPACING_REASON
        };
        assert_eq!(reason, expected);
    }

    fn add_state_test_contour_grid(renderer: &mut Renderer) {
        renderer
            .add_column("clx", &col_f64(vec![0.0, 0.5, 1.0]))
            .unwrap();
        renderer
            .add_column("cly", &col_f64(vec![0.0, 0.5, 1.0]))
            .unwrap();
        renderer
            .add_column("clz0", &col_f64(vec![0.0, 1.0, 2.0]))
            .unwrap();
        renderer
            .add_column("clz1", &col_f64(vec![1.0, 2.0, 3.0]))
            .unwrap();
        renderer
            .add_column("clz2", &col_f64(vec![2.0, 3.0, 4.0]))
            .unwrap();
    }

    fn state_test_contour_chart(rect: Rect, x_range: (f64, f64)) -> Chart {
        let mut config = panel_config(rect.width, rect.height);
        config.chart_area = crate::layout::ChartArea(rect);
        config.colorbar = Some(crate::default::default_colorbar_options());
        config.legend.visible = false;
        config.bottom_x.major_spacing = 0.25;
        config.left_y.major_spacing = 0.25;
        let mut chart = Chart::new(config);
        chart.set_x_range(x_range.0, x_range.1);
        chart.set_y_range(0.0, 1.0);
        chart.config_mut().bottom_x.major_spacing = 0.25;
        chart.config_mut().left_y.major_spacing = 0.25;
        chart
    }

    fn state_test_contour_variant(
        heatmap: bool,
        id: &str,
        x: &str,
        y: &str,
        z: &[&str],
        levels: usize,
    ) -> SeriesConfig {
        if heatmap {
            state_test_heatmap_contour(id, x, y, z, levels)
        } else {
            state_test_contour(id, x, y, z, levels)
        }
    }

    fn assert_contour_limit_error(result: Result<impl Sized>) {
        let Err(FiggyError::InvalidSeriesConfig { reason, .. }) = result else {
            panic!("oversized contour was accepted or returned the wrong error")
        };
        assert_eq!(
            reason,
            "contour level count 1025 exceeds the supported maximum 1024"
        );
    }

    #[test]
    fn renderer_owned_series_boundaries_reject_spacing_without_state_changes() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_contour_grid(&mut renderer);
        let mut config = crate::default::default_config();
        config.colorbar = Some(crate::default::default_colorbar_options());
        let chart = renderer.register_chart(config.clone(), Vec::new()).unwrap();

        let invalid = |spacing_px: f32, visible: bool, explicit: bool| {
            let anchors = explicit
                .then_some(crate::data_config::ContourLabelAnchor {
                    level_index: 0,
                    x: 0.5,
                    y: 0.5,
                    tx: 1.0,
                    ty: 0.0,
                })
                .into_iter()
                .collect();
            let mut series = state_test_labelled_contour(spacing_px, anchors);
            state_test_contour_labels_mut(&mut series).visible = visible;
            series
        };

        for series in [invalid(0.0, false, false), invalid(-1.0, true, true)] {
            let before_series = renderer.chart_series(chart).unwrap().to_vec();
            let before_visual = renderer.visual_revision();
            assert_contour_spacing_error(
                renderer.set_chart_series(chart, vec![series.clone()]),
                false,
            );
            assert_eq!(renderer.chart_series(chart).unwrap(), before_series);
            assert_eq!(renderer.visual_revision(), before_visual);

            assert_contour_spacing_error(
                renderer.set_chart_state(chart, config.clone(), vec![series.clone()]),
                false,
            );
            assert_eq!(renderer.chart_series(chart).unwrap(), before_series);
            assert_eq!(renderer.visual_revision(), before_visual);

            let before_order = renderer.chart_order().to_vec();
            assert_contour_spacing_error(
                renderer.register_chart(config.clone(), vec![series]),
                false,
            );
            assert_eq!(renderer.chart_order(), before_order);
            assert_eq!(renderer.visual_revision(), before_visual);
        }
    }

    #[test]
    fn demo_load_rejects_invalid_spacing_without_publishing_candidate_state() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let chart = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        let column = col_f64(vec![0.0, 1.0, 2.0]);
        let before_visual = renderer.visual_revision();
        let before_generation = renderer.pool().generation();
        let result = renderer.begin_load_demo(chart, &column, &column, &column, &column, |_| {
            let mut config = crate::default::default_config();
            config.colorbar = Some(crate::default::default_colorbar_options());
            let mut series = state_test_contour(
                "demo-label",
                DEMO_COLUMN_IDS[0],
                DEMO_COLUMN_IDS[1],
                &[DEMO_COLUMN_IDS[2]],
                1,
            );
            let DataRenderType::Contour { contour, .. } = &mut series.render_type else {
                unreachable!()
            };
            contour.labels = Some(crate::data_config::ContourLabelConfig {
                visible: false,
                font_size: 12.0,
                color: Color::BLACK,
                format: crate::format::LabelFormat::Decimal,
                significant_digits: 3,
                spacing_px: f32::NAN,
                anchors: Vec::new(),
                bg_color: None,
                bg_padding_px: 0.0,
            });
            Ok((config, vec![series]))
        });
        assert_contour_spacing_error(result, false);
        assert!(renderer.chart_series(chart).unwrap().is_empty());
        assert_eq!(renderer.visual_revision(), before_visual);
        assert_eq!(renderer.pool().generation(), before_generation);
    }

    #[test]
    fn prepare_preflights_hidden_invalid_spacing_before_gpu_mutation() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_contour_grid(&mut renderer);
        let chart = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: 240,
                height: 180,
            },
            (0.0, 1.0),
        );
        let mut config = state_test_labelled_contour(0.0, Vec::new());
        state_test_contour_labels_mut(&mut config).visible = false;
        let style = renderer.create_style_for_series(&config).unwrap();
        let series = [Series {
            config: &config,
            style: &style,
        }];
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        let before = renderer
            .gpu_memory_usage()
            .creations_of(GpuResourceKind::ContourScratch);
        assert_contour_spacing_error(renderer.prepare(&items), false);
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .creations_of(GpuResourceKind::ContourScratch),
            before
        );
        assert!(renderer.contour_label_pipelines.is_none());
    }

    #[test]
    fn export_checks_clamped_automatic_spacing_product_only_for_automatic_labels() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_contour_grid(&mut renderer);
        let chart = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: 240,
                height: 180,
            },
            (0.0, 1.0),
        );
        let huge = f32::MAX;
        let automatic = [state_test_labelled_contour(huge, Vec::new())];
        let before = renderer
            .gpu_memory_usage()
            .creations_of(GpuResourceKind::ContourScratch);
        assert_contour_spacing_error(
            renderer.export_panel_rgba(&chart, &automatic, f32::INFINITY),
            true,
        );
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .creations_of(GpuResourceKind::ContourScratch),
            before
        );

        let anchor = crate::data_config::ContourLabelAnchor {
            level_index: 0,
            x: 0.5,
            y: 0.5,
            tx: 1.0,
            ty: 0.0,
        };
        let explicit = [state_test_labelled_contour(huge, vec![anchor])];
        renderer
            .export_panel_rgba(&chart, &explicit, f32::INFINITY)
            .expect("explicit placement must not multiply spacing by export scale");

        let mut hidden = state_test_labelled_contour(huge, Vec::new());
        state_test_contour_labels_mut(&mut hidden).visible = false;
        renderer
            .export_panel_rgba(&chart, &[hidden], f32::INFINITY)
            .expect("hidden labels must not multiply spacing by export scale");
    }

    #[test]
    fn contour_label_font_generation_drift_forces_an_atlas_cache_miss() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_contour_grid(&mut renderer);
        let chart = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: 240,
                height: 180,
            },
            (0.0, 1.0),
        );
        let config = state_test_labelled_contour(90.0, Vec::new());
        let style = renderer.create_style_for_series(&config).unwrap();
        let series = [Series {
            config: &config,
            style: &style,
        }];
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];

        let first = renderer.prepare(&items).unwrap();
        let before = renderer.contour_cache["label-cache"]
            .label
            .as_ref()
            .unwrap()
            .atlas_identity();
        renderer
            .contour_cache
            .get_mut("label-cache")
            .unwrap()
            .signature
            .label
            .as_mut()
            .unwrap()
            .font_generation = crate::text_render::font_generation().wrapping_add(1);

        let second = renderer.prepare(&items).unwrap();
        let entry = &renderer.contour_cache["label-cache"];
        let after = entry.label.as_ref().unwrap().atlas_identity();
        assert_ne!(before, after, "font generation drift reused a stale atlas");
        assert_eq!(
            entry.signature.label.as_ref().unwrap().font_generation,
            crate::text_render::font_generation()
        );
        drop((first, second));
    }

    #[test]
    fn register_and_set_cover_both_contour_variants_at_the_level_boundary() {
        for heatmap in [false, true] {
            let Some(mut renderer) = state_test_renderer() else {
                return;
            };
            renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
            renderer.add_column("y", &col_f64(vec![0.0, 1.0])).unwrap();
            renderer.add_column("z0", &col_f64(vec![0.0, 1.0])).unwrap();
            let mut config = crate::default::default_config();
            config.colorbar = Some(crate::default::default_colorbar_options());

            let chart = renderer
                .register_chart(
                    config.clone(),
                    vec![state_test_contour_variant(
                        heatmap,
                        "accepted",
                        "x",
                        "y",
                        &["z0"],
                        crate::data_config::MAX_CONTOUR_LEVELS,
                    )],
                )
                .unwrap();
            renderer
                .set_chart_series(
                    chart,
                    vec![state_test_contour_variant(
                        heatmap,
                        "set-accepted",
                        "x",
                        "y",
                        &["z0"],
                        crate::data_config::MAX_CONTOUR_LEVELS,
                    )],
                )
                .unwrap();
            let before_revision = renderer.visual_revision();
            assert_contour_limit_error(renderer.set_chart_series(
                chart,
                vec![state_test_contour_variant(
                    heatmap,
                    "rejected",
                    "x",
                    "y",
                    &["z0"],
                    crate::data_config::MAX_CONTOUR_LEVELS + 1,
                )],
            ));
            assert_eq!(
                declared_level_count(&renderer.chart_series(chart).unwrap()[0].render_type),
                crate::data_config::MAX_CONTOUR_LEVELS
            );
            assert_eq!(renderer.visual_revision(), before_revision);

            let chart_count = renderer.chart_order().len();
            let before_revision = renderer.visual_revision();
            assert_contour_limit_error(renderer.register_chart(
                config,
                vec![state_test_contour_variant(
                    heatmap,
                    "register-rejected",
                    "x",
                    "y",
                    &["z0"],
                    crate::data_config::MAX_CONTOUR_LEVELS + 1,
                )],
            ));
            assert_eq!(renderer.chart_order().len(), chart_count);
            assert_eq!(renderer.visual_revision(), before_revision);
        }
    }

    #[test]
    fn contour_level_validation_is_identical_for_contour_and_heatmap_contour() {
        let allowed = [
            state_test_contour(
                "contour",
                "x",
                "y",
                &["z0"],
                crate::data_config::MAX_CONTOUR_LEVELS,
            ),
            state_test_heatmap_contour(
                "heatmap-contour",
                "x",
                "y",
                &["z0"],
                crate::data_config::MAX_CONTOUR_LEVELS,
            ),
        ];
        for series in &allowed {
            validate_contour_level_count(&series.series_id, &series.render_type).unwrap();
        }

        let rejected = [
            state_test_contour(
                "contour",
                "x",
                "y",
                &["z0"],
                crate::data_config::MAX_CONTOUR_LEVELS + 1,
            ),
            state_test_heatmap_contour(
                "heatmap-contour",
                "x",
                "y",
                &["z0"],
                crate::data_config::MAX_CONTOUR_LEVELS + 1,
            ),
        ];
        let reasons: Vec<String> = rejected
            .iter()
            .map(|series| {
                let FiggyError::InvalidSeriesConfig { reason, .. } =
                    validate_contour_level_count(&series.series_id, &series.render_type)
                        .unwrap_err()
                else {
                    panic!("wrong contour limit error")
                };
                reason
            })
            .collect();
        assert_eq!(reasons[0], reasons[1]);
    }

    #[test]
    fn direct_prepare_covers_both_contour_variants_at_the_level_boundary() {
        for heatmap in [false, true] {
            let Some((mut renderer, chart, _)) = field_prepare_fixture() else {
                return;
            };
            let view = renderer
                .create_chart_view(&chart, chart.config().chart_area.0)
                .unwrap();
            let accepted = state_test_contour_variant(
                heatmap,
                "field",
                "fx",
                "fy",
                &["fz0", "fz1"],
                crate::data_config::MAX_CONTOUR_LEVELS,
            );
            let style = renderer.create_style_for_series(&accepted).unwrap();
            let series = [Series {
                config: &accepted,
                style: &style,
            }];
            let items = [ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &series,
            }];
            drop(renderer.prepare(&items).unwrap());
            let creations = renderer
                .gpu_memory_usage()
                .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable);
            let field_cache_len = renderer.field_cache.len();
            let contour_cache_len = renderer.contour_cache.len();

            let rejected = state_test_contour_variant(
                heatmap,
                "rejected",
                "fx",
                "fy",
                &["fz0", "fz1"],
                crate::data_config::MAX_CONTOUR_LEVELS + 1,
            );
            let style = renderer.create_style_for_series(&rejected).unwrap();
            let series = [Series {
                config: &rejected,
                style: &style,
            }];
            let items = [ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &series,
            }];
            assert_contour_limit_error(renderer.prepare(&items));
            assert_eq!(
                renderer
                    .gpu_memory_usage()
                    .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable),
                creations
            );
            assert_eq!(renderer.field_cache.len(), field_cache_len);
            assert_eq!(renderer.contour_cache.len(), contour_cache_len);
        }
    }

    #[test]
    fn direct_export_covers_both_contour_variants_at_the_level_boundary() {
        for heatmap in [false, true] {
            let Some((mut renderer, chart, _)) = field_prepare_fixture() else {
                return;
            };
            let accepted = [state_test_contour_variant(
                heatmap,
                "field",
                "fx",
                "fy",
                &["fz0", "fz1"],
                crate::data_config::MAX_CONTOUR_LEVELS,
            )];
            let image = renderer.export_panel_rgba(&chart, &accepted, 1.0).unwrap();
            assert_eq!(image.width, chart.config().chart_area.0.width);
            assert_eq!(image.height, chart.config().chart_area.0.height);

            let creations = renderer
                .gpu_memory_usage()
                .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable);
            let rejected = [state_test_contour_variant(
                heatmap,
                "rejected",
                "fx",
                "fy",
                &["fz0", "fz1"],
                crate::data_config::MAX_CONTOUR_LEVELS + 1,
            )];
            assert_contour_limit_error(renderer.export_panel_rgba(&chart, &rejected, 1.0));
            assert_eq!(
                renderer
                    .gpu_memory_usage()
                    .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable),
                creations
            );
        }
    }

    #[test]
    fn demo_load_covers_both_contour_variants_at_the_level_boundary() {
        for heatmap in [false, true] {
            let Some(mut renderer) = state_test_renderer() else {
                return;
            };
            let chart = renderer
                .register_chart(crate::default::default_config(), Vec::new())
                .unwrap();
            let column = col_f64(vec![0.0, 1.0]);
            let mut config = crate::default::default_config();
            config.colorbar = Some(crate::default::default_colorbar_options());
            renderer
                .begin_load_demo(chart, &column, &column, &column, &column, |_| {
                    Ok((
                        config,
                        vec![state_test_contour_variant(
                            heatmap,
                            "accepted",
                            DEMO_COLUMN_IDS[0],
                            DEMO_COLUMN_IDS[1],
                            &[DEMO_COLUMN_IDS[2]],
                            crate::data_config::MAX_CONTOUR_LEVELS,
                        )],
                    ))
                })
                .unwrap()
                .commit();
            assert_eq!(
                declared_level_count(&renderer.chart_series(chart).unwrap()[0].render_type),
                crate::data_config::MAX_CONTOUR_LEVELS
            );

            let before_revision = renderer.visual_revision();
            let before_series = renderer.chart_series(chart).unwrap().to_vec();
            let before_generation = renderer.pool().generation();
            let before_layout_generation = renderer.pool().layout_generation();
            let before_handles = DEMO_COLUMN_IDS.map(|id| renderer.pool().handle_for(id).unwrap());
            let before_epochs =
                DEMO_COLUMN_IDS.map(|id| renderer.pool().allocation_epoch(id).unwrap());
            let mut rejected_config = crate::default::default_config();
            rejected_config.colorbar = Some(crate::default::default_colorbar_options());
            let result =
                renderer.begin_load_demo(chart, &column, &column, &column, &column, |_| {
                    Ok((
                        rejected_config,
                        vec![state_test_contour_variant(
                            heatmap,
                            "rejected",
                            DEMO_COLUMN_IDS[0],
                            DEMO_COLUMN_IDS[1],
                            &[DEMO_COLUMN_IDS[2]],
                            crate::data_config::MAX_CONTOUR_LEVELS + 1,
                        )],
                    ))
                });
            assert_contour_limit_error(result);
            assert_eq!(renderer.chart_series(chart).unwrap(), before_series);
            assert_eq!(renderer.visual_revision(), before_revision);
            assert_eq!(renderer.pool().generation(), before_generation);
            assert_eq!(
                renderer.pool().layout_generation(),
                before_layout_generation
            );
            for (id, before) in DEMO_COLUMN_IDS.into_iter().zip(before_handles) {
                let after = renderer.pool().handle_for(id).unwrap();
                assert_eq!(after.generation, before.generation);
                assert_eq!(after.offset, before.offset);
                assert_eq!(after.byte_size, before.byte_size);
                assert_eq!(after.len_values, before.len_values);
            }
            assert_eq!(
                DEMO_COLUMN_IDS.map(|id| renderer.pool().allocation_epoch(id).unwrap()),
                before_epochs
            );
        }
    }

    // A field series without `Config.colorbar` is not a chart missing a
    // decoration — it is a chart with no z range and no colormap, so there is no
    // value to draw. Rejected, and rejected atomically.
    #[test]
    fn a_field_series_without_a_colorbar_is_rejected() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let chart = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("z0", &col_f64(vec![1.0, 2.0])).unwrap();
        let before = renderer.visual_revision();

        let field = vec![state_test_heatmap("field", "x", "y", &["z0"])];
        assert!(matches!(
            renderer.set_chart_series(chart, field.clone()),
            Err(FiggyError::InvalidSeriesConfig { .. })
        ));
        assert!(renderer.chart_series(chart).unwrap().is_empty());
        assert_eq!(renderer.visual_revision(), before);

        // With a colourbar it is accepted — including a hidden one, which is how
        // a field is drawn without the bar beside it.
        let mut config = crate::default::default_config();
        let mut bar = crate::default::default_colorbar_options();
        bar.visible = false;
        config.colorbar = Some(bar);
        renderer.set_chart_state(chart, config, field).unwrap();
        assert_eq!(renderer.chart_series(chart).unwrap().len(), 1);
    }

    // The grid's constituent columns are referenced columns: they must exist,
    // and the reference must be reachable by everything that walks a series'
    // columns (validation, the remove-column cascade, derived identity).
    #[test]
    fn a_matrix_column_must_be_registered_and_is_a_referenced_column() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let mut config = crate::default::default_config();
        config.colorbar = Some(crate::default::default_colorbar_options());
        let chart = renderer.register_chart(config, Vec::new()).unwrap();
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("z0", &col_f64(vec![1.0, 2.0])).unwrap();

        assert!(matches!(
            renderer.set_chart_series(
                chart,
                vec![state_test_heatmap("field", "x", "y", &["z0", "z_missing"])]
            ),
            Err(FiggyError::UnknownColumn { .. })
        ));

        let series = state_test_heatmap("field", "x", "y", &["z0"]);
        renderer
            .set_chart_series(chart, vec![series.clone()])
            .unwrap();
        assert!(series_references_column(&series, "z0"));
        assert!(!series_references_column(&series, "z_other"));

        // A declaration that does not line up with the data is not an error —
        // so a grid declaring more columns than exist in the pool is only
        // rejected for the missing *registration*, never for the count.
        let mut visited = Vec::new();
        visit_series_columns(&series, &mut |column| visited.push(column.to_string()));
        assert!(visited.iter().any(|column| column == "z0"));
    }

    // The paired reducer's domain is index-aligned pairs, which the field and bar
    // types are not. Fields use the distinct lattice mode; histograms stay on
    // metadata. Their stamps remain in the fit token in either case.
    #[test]
    fn field_and_bar_types_are_not_submitted_to_the_paired_extent_reducer() {
        use crate::gpu_errorbar::GpuSeriesExtentMode;

        let matrix = crate::data_config::MatrixRef {
            columns: vec!["z0".into()],
            orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
            grid_layout: crate::data_config::GridLayout::Centers,
        };
        let fill = crate::data_config::FieldFillConfig {
            mode: crate::data_config::FillMode::Continuous,
            shading: crate::data_config::Shading::Flat,
            opacity: 1.0,
        };
        let contour = crate::data_config::ContourConfig {
            levels: vec![1.0],
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: None,
            labels: None,
        };
        let unpaired = [
            DataRenderType::Histogram {
                bar: crate::data_config::DataBarStyleConfig {
                    fill_color: Color::BLACK,
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    baseline: 0.0,
                    gap_px: 1.0,
                    width_ratio: 1.0,
                    orientation: crate::data_config::BarOrientation::Vertical,
                    bar_style_overrides: None,
                },
            },
            DataRenderType::Heatmap {
                matrix: matrix.clone(),
                fill: fill.clone(),
            },
            DataRenderType::Contour {
                matrix: matrix.clone(),
                contour: contour.clone(),
            },
            DataRenderType::HeatmapContour {
                matrix,
                fill,
                contour,
            },
        ];
        for render_type in &unpaired {
            assert_eq!(
                GpuSeriesExtentMode::from_render_type(render_type),
                None,
                "{render_type:?} must not go through the paired reducer"
            );
            // And they draw neither of the paired primitives, so nothing
            // downstream compiles a pipeline or a layer for them yet.
            assert!(!has_line(render_type));
            assert!(!has_scatter(render_type));
            assert!(!has_errorbar(render_type));
            assert!(!constellation_supports(render_type));
        }
        // The paired types still are.
        assert_eq!(
            GpuSeriesExtentMode::from_render_type(&DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            }),
            Some(GpuSeriesExtentMode::Line)
        );
    }

    #[test]
    fn draw_style_supports_matches_the_primitives_each_style_draws() {
        let line = DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 1.0,
            fill: None,
            fill_color: None,
            interpolation: crate::data_config::LineInterpolation::Linear,
        };
        let scatter = crate::data_config::DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: crate::data_config::ScatterShape::Circle,
            point_size: 4.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        };
        let matrix = crate::data_config::MatrixRef {
            columns: vec!["z0".into()],
            orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
            grid_layout: crate::data_config::GridLayout::Centers,
        };
        let fill = crate::data_config::FieldFillConfig {
            mode: crate::data_config::FillMode::Continuous,
            shading: crate::data_config::Shading::Flat,
            opacity: 1.0,
        };
        let contour = crate::data_config::ContourConfig {
            levels: vec![1.0],
            line: line.clone(),
            per_level_color: None,
            labels: None,
        };
        let line_rt = DataRenderType::Line { line: line.clone() };
        let scatter_rt = DataRenderType::Scatter {
            scatter: scatter.clone(),
        };
        let marker_line_rt = DataRenderType::ScatterLine {
            scatter: scatter.clone(),
            line: line.clone(),
        };
        let precise_only = [
            DataRenderType::Histogram {
                bar: crate::data_config::DataBarStyleConfig {
                    fill_color: Color::BLACK,
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    baseline: 0.0,
                    gap_px: 1.0,
                    width_ratio: 1.0,
                    orientation: crate::data_config::BarOrientation::Vertical,
                    bar_style_overrides: None,
                },
            },
            DataRenderType::Heatmap {
                matrix: matrix.clone(),
                fill: fill.clone(),
            },
            DataRenderType::Contour {
                matrix: matrix.clone(),
                contour: contour.clone(),
            },
            DataRenderType::HeatmapContour {
                matrix,
                fill,
                contour,
            },
        ];
        let err = crate::data_config::DataErrorBarStyleConfig {
            error_bar_color: Color::BLACK,
            error_bar_width: 1.0,
            error_bar_cap_size: 4.0,
            cap_width: 1.0,
            error_bar_style_table: None,
            error_bar_style_index_column: None,
            error_bar_style_overrides: None,
        };
        let sigma = || crate::data_config::ErrorRef::Symmetric { column: "e".into() };
        let errorbar_types = [
            DataRenderType::ScatterErrorbarX {
                scatter: scatter.clone(),
                err_x: sigma(),
                err_style: err.clone(),
            },
            DataRenderType::ScatterErrorbarY {
                scatter: scatter.clone(),
                err_y: sigma(),
                err_style: err.clone(),
            },
            DataRenderType::ScatterErrorbarXY {
                scatter: scatter.clone(),
                err_x: sigma(),
                err_y: sigma(),
                err_style: err.clone(),
            },
            DataRenderType::LineScatterErrorbarX {
                scatter: scatter.clone(),
                line: line.clone(),
                err_x: sigma(),
                err_style: err.clone(),
            },
            DataRenderType::LineScatterErrorbarY {
                scatter: scatter.clone(),
                line: line.clone(),
                err_y: sigma(),
                err_style: err.clone(),
            },
            DataRenderType::LineScatterErrorbarXY {
                scatter: scatter.clone(),
                line: line.clone(),
                err_x: sigma(),
                err_y: sigma(),
                err_style: err,
            },
        ];
        let sketch = DrawStyle::Sketch(Default::default());
        let milkyway = DrawStyle::Milkyway(Default::default());
        let constellation = DrawStyle::Constellation(Default::default());
        // Errorbars follow the series' scatter in Sketch and Milkyway; the
        // constellation style drops errorbar series entirely.
        for rt in &errorbar_types {
            assert!(draw_style_supports(&DrawStyle::Precise, rt), "{rt:?}");
            assert!(draw_style_supports(&sketch, rt), "sketch {rt:?}");
            assert!(draw_style_supports(&milkyway, rt), "milkyway {rt:?}");
            assert!(
                !draw_style_supports(&constellation, rt),
                "constellation {rt:?}"
            );
        }

        for rt in [&line_rt, &scatter_rt, &marker_line_rt]
            .into_iter()
            .chain(&precise_only)
        {
            assert!(draw_style_supports(&DrawStyle::Precise, rt), "{rt:?}");
        }
        for rt in [&line_rt, &scatter_rt, &marker_line_rt] {
            assert!(draw_style_supports(&sketch, rt), "sketch {rt:?}");
            assert!(draw_style_supports(&milkyway, rt), "milkyway {rt:?}");
        }
        for rt in &precise_only {
            assert!(!draw_style_supports(&sketch, rt), "sketch {rt:?}");
            assert!(!draw_style_supports(&milkyway, rt), "milkyway {rt:?}");
            assert!(
                !draw_style_supports(&constellation, rt),
                "constellation {rt:?}"
            );
        }
        assert!(draw_style_supports(&constellation, &marker_line_rt));
        assert!(!draw_style_supports(&constellation, &line_rt));
        assert!(!draw_style_supports(&constellation, &scatter_rt));
    }

    #[test]
    fn renderer_field_fit_resolves_the_same_sample_endpoints_as_contour_draw() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("fit-gx", &col_f64(vec![-3.1, -2.9, 2.9, 3.1]))
            .unwrap();
        renderer
            .add_column("fit-gy", &col_f64(vec![-2.5, -2.3, 2.3, 2.5]))
            .unwrap();
        for (id, values) in [
            ("fit-z0", vec![0.0, 1.0, 2.0]),
            ("fit-z1", vec![1.0, 2.0, 3.0]),
            ("fit-z2", vec![2.0, 3.0, 4.0]),
        ] {
            renderer.add_column(id, &col_f64(values)).unwrap();
        }
        let mut series = state_test_contour(
            "field-fit",
            "fit-gx",
            "fit-gy",
            &["fit-z0", "fit-z1", "fit-z2"],
            1,
        );
        let DataRenderType::Contour { matrix, .. } = &mut series.render_type else {
            unreachable!()
        };
        matrix.grid_layout = crate::data_config::GridLayout::Edges;

        pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();
        let extent = pollster::block_on(
            renderer
                .begin_series_fit_extent(&series)
                .unwrap()
                .expect("contour uses a GPU field-fit ticket")
                .resolve(),
        )
        .unwrap()
        .expect("the 3x3 contour lattice has an extent");
        assert!((extent.x.min + 3.0).abs() < 1.0e-6);
        assert!((extent.x.max - 3.0).abs() < 1.0e-6);
        assert!((extent.y.min + 2.4).abs() < 1.0e-6);
        assert!((extent.y.max - 2.4).abs() < 1.0e-6);
    }

    #[test]
    fn renderer_owned_series_validation_is_failure_atomic() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let chart = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        let before_revision = renderer.visual_revision();

        let missing = vec![state_test_line("missing", "x", "y")];
        assert!(matches!(
            renderer.set_chart_series(chart, missing),
            Err(FiggyError::UnknownColumn { .. })
        ));
        assert!(renderer.chart_series(chart).unwrap().is_empty());
        assert_eq!(renderer.visual_revision(), before_revision);

        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let duplicate = vec![
            state_test_line("same", "x", "y"),
            state_test_line("same", "x", "y"),
        ];
        assert!(matches!(
            renderer.set_chart_series(chart, duplicate),
            Err(FiggyError::InvalidSeriesConfig { .. })
        ));
        assert!(renderer.chart_series(chart).unwrap().is_empty());
        assert_eq!(renderer.visual_revision(), before_revision);
    }

    #[test]
    fn renderer_owned_combined_state_validation_is_failure_atomic() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let initial_config = crate::default::default_config();
        let initial_series = vec![state_test_line("line", "x", "y")];
        let chart = renderer
            .register_chart(initial_config.clone(), initial_series.clone())
            .unwrap();
        let initial_stamp = renderer.chart_render_stamp(chart).unwrap();

        let mut changed_config = initial_config.clone();
        changed_config.chart_title.text.segments =
            crate::text::rich_segments_from_text("must not publish");
        let invalid_series = vec![state_test_line("missing", "x", "missing")];
        assert!(matches!(
            renderer.set_chart_state(chart, changed_config, invalid_series),
            Err(FiggyError::UnknownColumn { .. })
        ));
        assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
        assert_eq!(renderer.chart_series(chart).unwrap(), initial_series);
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), initial_stamp);

        let mut invalid_config = initial_config.clone();
        invalid_config.bottom_x.max = invalid_config.bottom_x.min;
        assert!(matches!(
            renderer.set_chart_state(chart, invalid_config, Vec::new()),
            Err(FiggyError::InvalidConfig { .. })
        ));
        assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
        assert_eq!(renderer.chart_series(chart).unwrap(), initial_series);
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), initial_stamp);
    }

    #[test]
    fn renderer_owned_config_and_view_validation_are_failure_atomic() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let mut invalid = crate::default::default_config();
        invalid.bottom_x.max = invalid.bottom_x.min;
        let visual_before = renderer.visual_revision();
        assert!(matches!(
            renderer.register_chart(invalid, Vec::new()),
            Err(FiggyError::InvalidConfig {
                field: "bottom_x",
                ..
            })
        ));
        assert!(renderer.chart_order().is_empty());
        assert_eq!(renderer.visual_revision(), visual_before);

        let mut invalid_layout = crate::default::default_config();
        invalid_layout.left_y.out_margin = invalid_layout.chart_area.0.width as f32;
        assert!(matches!(
            renderer.register_chart(invalid_layout, Vec::new()),
            Err(FiggyError::InvalidConfig {
                field: "layout",
                ..
            })
        ));
        assert!(renderer.chart_order().is_empty());
        assert_eq!(renderer.visual_revision(), visual_before);

        let chart = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        let config_before = renderer.chart_config(chart).unwrap().clone();
        let view_before = renderer.chart_view_state(chart).unwrap();
        let visual_before = renderer.visual_revision();
        let mut invalid_view = view_before.clone();
        invalid_view.left_y.min = f64::NAN;
        assert!(matches!(
            renderer.set_chart_view_state(chart, invalid_view),
            Err(FiggyError::InvalidConfig {
                field: "left_y",
                ..
            })
        ));
        assert_eq!(renderer.chart_config(chart).unwrap(), &config_before);
        assert_eq!(renderer.chart_view_state(chart).unwrap(), view_before);
        assert_eq!(renderer.visual_revision(), visual_before);

        let mut invalid_layout_view = view_before.clone();
        invalid_layout_view.chart_area.width = 1;
        invalid_layout_view.chart_area.height = 1;
        assert!(matches!(
            renderer.set_chart_view_state(chart, invalid_layout_view),
            Err(FiggyError::InvalidConfig {
                field: "layout",
                ..
            })
        ));
        assert_eq!(renderer.chart_config(chart).unwrap(), &config_before);
        assert_eq!(renderer.chart_view_state(chart).unwrap(), view_before);
        assert_eq!(renderer.visual_revision(), visual_before);
    }

    #[test]
    fn set_chart_config_rejects_removing_required_colorbar_without_publishing() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        for id in ["x", "y", "z"] {
            renderer.add_column(id, &col_f64(vec![1.0, 2.0])).unwrap();
        }
        let mut config = crate::default::default_config();
        config.colorbar = Some(crate::default::default_colorbar_options());
        let series = SeriesConfig {
            series_id: "heat".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Heatmap {
                matrix: crate::data_config::MatrixRef {
                    columns: vec!["z".into()],
                    orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                    grid_layout: crate::data_config::GridLayout::Centers,
                },
                fill: crate::data_config::FieldFillConfig {
                    mode: crate::data_config::FillMode::Continuous,
                    shading: crate::data_config::Shading::Flat,
                    opacity: 1.0,
                },
            },
        };
        let chart = renderer
            .register_chart(config.clone(), vec![series])
            .unwrap();
        let stamp = renderer.chart_render_stamp(chart).unwrap();
        let visual = renderer.visual_revision();
        let mut invalid = config.clone();
        invalid.colorbar = None;
        assert!(matches!(
            renderer.set_chart_config(chart, invalid),
            Err(FiggyError::InvalidSeriesConfig { .. })
        ));
        assert_eq!(renderer.chart_config(chart).unwrap(), &config);
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), stamp);
        assert_eq!(renderer.visual_revision(), visual);
    }

    #[test]
    fn style_map_and_uniforms_are_admitted_and_charged_together() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let mut series = state_test_scatter("mapped", None, "x", "y");
        let DataRenderType::Scatter { scatter } = &mut series.render_type else {
            unreachable!();
        };
        scatter.point_style_table = Some(vec![DataScatterPointStyleConfig {
            point_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
            ..Default::default()
        }]);
        let bytes = renderer.style_allocation_bytes(&series).unwrap();
        assert_eq!(
            bytes,
            4 * std::mem::size_of::<PrimitiveStyle>() as u64
                + std::mem::size_of::<data_render::ScatterStyleSlotGpu>() as u64
                + std::mem::size_of::<data_render::ScatterStyleOverrideGpu>() as u64
                + std::mem::size_of::<data_render::ScatterStyleMapMeta>() as u64
        );
        let before = renderer.gpu_memory_usage();
        let _ = renderer.set_memory_budget(Some(before.total_bytes() + bytes - 1));
        assert!(matches!(
            renderer.create_style_for_series(&series),
            Err(FiggyError::GpuResourceLimit {
                resource: "chart style budget",
                ..
            })
        ));
        let rejected = renderer.gpu_memory_usage();
        assert_eq!(rejected.total_bytes(), before.total_bytes());
        assert_eq!(
            rejected.creations_of(GpuResourceKind::Uniform),
            before.creations_of(GpuResourceKind::Uniform)
        );

        let _ = renderer.set_memory_budget(Some(before.total_bytes() + bytes));
        let style = renderer.create_style_for_series(&series).unwrap();
        assert!(style.scatter_map.is_some());
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .live_bytes_of(GpuResourceKind::Uniform),
            before.live_bytes_of(GpuResourceKind::Uniform) + bytes
        );
        drop(style);
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .retired_bytes_of(GpuResourceKind::Uniform),
            before.retired_bytes_of(GpuResourceKind::Uniform) + bytes
        );
    }

    #[test]
    fn prepared_frame_keeps_style_charge_after_style_owner_is_dropped() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let cfg = state_test_scatter("point", None, "x", "y");
        let chart = Chart::new(crate::default::default_config());
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let style_bytes = renderer.style_allocation_bytes(&cfg).unwrap();
        let style = renderer.create_style_for_series(&cfg).unwrap();
        let frame = {
            let series = [Series {
                config: &cfg,
                style: &style,
            }];
            renderer
                .prepare(&[ChartDrawItem {
                    view: &view,
                    chart_config: chart.config(),
                    series: &series,
                }])
                .unwrap()
        };
        let live_with_frame = renderer
            .gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::Uniform);
        drop(style);
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .live_bytes_of(GpuResourceKind::Uniform),
            live_with_frame
        );
        drop(frame);
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .live_bytes_of(GpuResourceKind::Uniform),
            live_with_frame - style_bytes
        );
    }

    #[test]
    fn renderer_owned_explicit_setters_always_issue_checked_revisions() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let config = crate::default::default_config();
        let chart = renderer.register_chart(config.clone(), Vec::new()).unwrap();

        let after_register = renderer.visual_revision();
        renderer.set_chart_config(chart, config.clone()).unwrap();
        let after_config = renderer.visual_revision();
        assert_ne!(after_register, after_config);
        let raster_before_series = renderer.chart_states[&chart].revisions.raster;
        renderer.set_chart_series(chart, Vec::new()).unwrap();
        let after_series = renderer.visual_revision();
        assert_ne!(after_config, after_series);
        assert_ne!(
            renderer.chart_states[&chart].revisions.raster,
            raster_before_series
        );
        renderer.set_chart_selection(chart, None).unwrap();
        assert_ne!(after_series, renderer.visual_revision());

        let before_config = renderer.chart_config(chart).unwrap().clone();
        let before_visual = renderer.visual_revision();
        renderer.visual_revision = RenderRevision {
            renderer_identity: renderer.renderer_identity,
            sequence: u64::MAX,
        };
        assert!(matches!(
            renderer.set_chart_config(chart, config),
            Err(FiggyError::CounterExhausted { .. })
        ));
        assert_eq!(renderer.chart_config(chart).unwrap(), &before_config);
        renderer.visual_revision = before_visual;
    }

    #[test]
    fn chart_render_stamp_classifies_renderer_owned_dirty_state() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("unused", &col_f64(vec![9.0, 9.0]))
            .unwrap();
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();

        let initial = renderer.chart_render_stamp(chart).unwrap();
        assert!(initial.needs_draw_since(None));
        assert!(initial.needs_raster_since(None));

        renderer.remove_column("unused").unwrap();
        assert!(renderer.defragment().unwrap());
        let after_maintenance = renderer.chart_render_stamp(chart).unwrap();
        assert_eq!(after_maintenance, initial);

        // Replacing the config with an identical one is not an edit.
        let same_config = renderer.chart_config(chart).unwrap().clone();
        renderer.set_chart_config(chart, same_config).unwrap();
        let unchanged = renderer.chart_render_stamp(chart).unwrap();
        assert_eq!(unchanged, initial);

        // A chrome edit draws and re-rasterizes the axis layers.
        let mut chrome_config = renderer.chart_config(chart).unwrap().clone();
        chrome_config.chart_title.visible = !chrome_config.chart_title.visible;
        renderer.set_chart_config(chart, chrome_config).unwrap();
        let decoration = renderer.chart_render_stamp(chart).unwrap();
        assert!(decoration.needs_draw_since(Some(&initial)));
        assert!(decoration.needs_raster_since(Some(&initial)));

        // Layer-only and selection-only edits draw without touching the raster.
        let mut layer_config = renderer.chart_config(chart).unwrap().clone();
        layer_config.active_layer = crate::LayerRef::Annotations;
        layer_config.annotations.visible = false;
        renderer.set_chart_config(chart, layer_config).unwrap();
        let layer = renderer.chart_render_stamp(chart).unwrap();
        assert!(layer.needs_draw_since(Some(&decoration)));
        assert!(!layer.needs_raster_since(Some(&decoration)));
        let mut picked_config = renderer.chart_config(chart).unwrap().clone();
        picked_config.picked_points = Some(crate::config::PickedPointsConfig::default());
        renderer.set_chart_config(chart, picked_config).unwrap();
        let picked = renderer.chart_render_stamp(chart).unwrap();
        assert!(picked.needs_draw_since(Some(&layer)));
        assert!(!picked.needs_raster_since(Some(&layer)));

        let mut view_config = renderer.chart_config(chart).unwrap().clone();
        view_config.bottom_x.min -= 1.0;
        renderer.set_chart_config(chart, view_config).unwrap();
        let view = renderer.chart_render_stamp(chart).unwrap();
        assert!(view.needs_draw_since(Some(&picked)));
        assert!(view.needs_raster_since(Some(&picked)));

        renderer
            .upsert_column("x", &col_f64(vec![10.0, 11.0]))
            .unwrap();
        let data = renderer.chart_render_stamp(chart).unwrap();
        assert!(data.needs_draw_since(Some(&view)));
        assert!(!data.needs_raster_since(Some(&view)));
    }

    #[test]
    fn renderer_font_generation_sync_is_atomic_and_idempotent() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let chart = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        let generation = crate::text_render::font_generation();
        renderer.observed_font_generation = if generation == 0 {
            u64::MAX
        } else {
            generation - 1
        };
        let before = renderer.visual_revision();
        assert!(renderer.sync_external_invalidations().unwrap());
        assert_ne!(renderer.visual_revision(), before);
        assert!(!renderer.sync_external_invalidations().unwrap());
        assert!(renderer.chart_config(chart).is_ok());
    }

    #[test]
    fn renderer_column_upsert_invalidates_only_referencing_charts() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        renderer.add_column("z", &col_f64(vec![2.0, 3.0])).unwrap();
        let affected = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("affected", "x", "y")],
            )
            .unwrap();
        let unrelated = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("unrelated", "z", "y")],
            )
            .unwrap();
        let affected_before = renderer.chart_states[&affected].revisions.desired;
        let unrelated_before = renderer.chart_states[&unrelated].revisions.desired;
        let visual_before = renderer.visual_revision();

        renderer
            .upsert_column("x", &col_f64(vec![10.0, 11.0]))
            .unwrap();

        assert_ne!(
            renderer.chart_states[&affected].revisions.desired,
            affected_before
        );
        assert_eq!(
            renderer.chart_states[&unrelated].revisions.desired,
            unrelated_before
        );
        assert_ne!(renderer.visual_revision(), visual_before);
    }

    #[test]
    fn load_demo_guard_drop_preserves_renderer_authority_and_retry_commits() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let old = col_f64(vec![1.0, 2.0, 3.0]);
        for id in DEMO_COLUMN_IDS {
            renderer.add_column(id, &old).unwrap();
        }
        let initial_config = crate::default::default_config();
        let initial_series = vec![state_test_scatter(
            "old-demo",
            Some("old"),
            "demo_x",
            "demo_sin",
        )];
        let chart = renderer
            .register_chart(initial_config.clone(), initial_series.clone())
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();

        let revisions = revision_snapshot(&renderer, chart);
        let visual = renderer.visual_revision();
        let generation = renderer.pool().generation();
        let layout_generation = renderer.pool().layout_generation();
        let epochs = DEMO_COLUMN_IDS.map(|id| renderer.pool().allocation_epoch(id).unwrap());
        let handles = DEMO_COLUMN_IDS.map(|id| renderer.pool().handle_for(id).unwrap());
        let picker_generation = active_picker(&renderer).engine.registry_generation();
        let picker_signature = active_picker(&renderer).signature.clone();
        let columns = demo_test_columns();
        let mut final_config = initial_config.clone();
        final_config.chart_title.text.segments =
            crate::text::rich_segments_from_text("atomic demo");
        let final_series = vec![state_test_scatter(
            "new-demo",
            Some("new"),
            "demo_t",
            "demo_rc",
        )];
        {
            let pending = renderer
                .begin_load_demo(
                    chart,
                    &columns[0],
                    &columns[1],
                    &columns[2],
                    &columns[3],
                    |_| Ok((final_config.clone(), final_series.clone())),
                )
                .unwrap();
            drop(pending);
        }

        assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
        assert_eq!(renderer.chart_series(chart).unwrap(), initial_series);
        assert_eq!(revision_snapshot(&renderer, chart), revisions);
        assert_eq!(renderer.visual_revision(), visual);
        assert_eq!(renderer.pool().generation(), generation);
        assert_eq!(renderer.pool().layout_generation(), layout_generation);
        assert_eq!(
            DEMO_COLUMN_IDS.map(|id| renderer.pool().allocation_epoch(id).unwrap()),
            epochs
        );
        for (id, handle) in DEMO_COLUMN_IDS.into_iter().zip(handles) {
            let current = renderer.pool().handle_for(id).unwrap();
            assert_eq!(current.generation, handle.generation);
            assert_eq!(current.offset, handle.offset);
            assert_eq!(current.byte_size, handle.byte_size);
            assert_eq!(current.len_values, handle.len_values);
        }
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            picker_generation
        );
        assert_eq!(active_picker(&renderer).signature, picker_signature);

        renderer
            .begin_load_demo(
                chart,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
                |_| Ok((final_config.clone(), final_series.clone())),
            )
            .unwrap()
            .commit();
        assert_eq!(renderer.chart_config(chart).unwrap(), &final_config);
        assert_eq!(renderer.chart_series(chart).unwrap(), final_series);
        assert_ne!(revision_snapshot(&renderer, chart), revisions);
        assert_ne!(renderer.visual_revision(), visual);
        assert_eq!(renderer.pool().generation(), generation + 1);
        assert_eq!(renderer.pool().layout_generation(), layout_generation + 1);
        assert!(
            DEMO_COLUMN_IDS
                .into_iter()
                .zip(epochs)
                .all(|(id, epoch)| renderer.pool().allocation_epoch(id) != Some(epoch))
        );
        assert!(active_picker(&renderer).engine.registry_generation() > picker_generation);
        assert!(!renderer.has_pending_maintenance());
    }

    #[test]
    fn load_demo_rebuilds_changed_slots_for_a_different_active_chart() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let old = col_f64(vec![1.0, 2.0, 3.0]);
        for id in DEMO_COLUMN_IDS {
            renderer.add_column(id, &old).unwrap();
        }
        add_state_test_xy(&mut renderer, "stable-x", "stable-y", vec![1.0, 2.0, 3.0]);
        let target = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        let other_series = vec![
            state_test_scatter("changed", None, "demo_x", "demo_sin"),
            state_test_scatter("stable", None, "stable-x", "stable-y"),
        ];
        let other = renderer
            .register_chart(crate::default::default_config(), other_series.clone())
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(other).unwrap();
        let changed_buffer = active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap();
        let stable_buffer = active_picker(&renderer)
            .engine
            .query_params_buffer(1)
            .unwrap();
        let other_revisions = revision_snapshot(&renderer, other);
        let columns = demo_test_columns();
        let target_series = vec![state_test_line("demo", "demo_t", "demo_rc")];

        renderer
            .begin_load_demo(
                target,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
                |_| Ok((crate::default::default_config(), target_series.clone())),
            )
            .unwrap()
            .commit();

        assert_eq!(active_picker(&renderer).chart_id, other);
        assert_ne!(
            active_picker(&renderer)
                .engine
                .query_params_buffer(0)
                .unwrap(),
            changed_buffer
        );
        assert_eq!(
            active_picker(&renderer)
                .engine
                .query_params_buffer(1)
                .unwrap(),
            stable_buffer
        );
        assert_eq!(renderer.chart_series(other).unwrap(), other_series);
        assert_ne!(revision_snapshot(&renderer, other), other_revisions);
        assert_eq!(renderer.chart_series(target).unwrap(), target_series);
    }

    #[test]
    fn load_demo_picker_failure_preserves_pool_chart_and_picker_then_retries() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let old = col_f64(vec![1.0, 2.0]);
        for id in DEMO_COLUMN_IDS {
            renderer.add_column(id, &old).unwrap();
        }
        let initial_series = vec![state_test_scatter("demo", None, "demo_x", "demo_sin")];
        let chart = renderer
            .register_chart(crate::default::default_config(), initial_series.clone())
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let generation = renderer.pool().generation();
        let layout_generation = renderer.pool().layout_generation();
        let revisions = revision_snapshot(&renderer, chart);
        let visual = renderer.visual_revision();
        let picker_generation = active_picker(&renderer).engine.registry_generation();
        active_picker(&renderer).engine.fail_next_registry_prepare();
        let columns = demo_test_columns();

        assert!(matches!(
            renderer.begin_load_demo(
                chart,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
                |_| Ok((crate::default::default_config(), initial_series.clone())),
            ),
            Err(FiggyError::GpuPick(
                crate::gpu_pick::GpuPickError::AllocationFailed {
                    resource: "injected registry prepare failure"
                }
            ))
        ));
        assert_eq!(renderer.pool().generation(), generation);
        assert_eq!(renderer.pool().layout_generation(), layout_generation);
        assert_eq!(revision_snapshot(&renderer, chart), revisions);
        assert_eq!(renderer.visual_revision(), visual);
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            picker_generation
        );

        renderer
            .begin_load_demo(
                chart,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
                |_| Ok((crate::default::default_config(), initial_series.clone())),
            )
            .unwrap()
            .commit();
        assert_eq!(renderer.pool().generation(), generation + 1);
    }

    #[test]
    fn load_demo_revision_failure_happens_before_publication() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let chart = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        let original = renderer.chart_states[&chart].revisions.desired;
        renderer
            .chart_states
            .get_mut(&chart)
            .unwrap()
            .revisions
            .desired = RenderRevision {
            renderer_identity: original.renderer_identity,
            sequence: u64::MAX,
        };
        let generation = renderer.pool().generation();
        let layout_generation = renderer.pool().layout_generation();
        let visual = renderer.visual_revision();
        let columns = demo_test_columns();

        assert!(matches!(
            renderer.begin_load_demo(
                chart,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
                |_| Ok((crate::default::default_config(), Vec::new())),
            ),
            Err(FiggyError::CounterExhausted {
                counter: "chart desired revision"
            })
        ));
        assert_eq!(renderer.pool().generation(), generation);
        assert_eq!(renderer.pool().layout_generation(), layout_generation);
        assert_eq!(renderer.visual_revision(), visual);
        assert_eq!(
            renderer.chart_states[&chart].revisions.desired.sequence,
            u64::MAX
        );
    }

    #[test]
    fn active_picker_upsert_failure_rolls_back_every_authority_and_retry_succeeds() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_scatter("scatter", None, "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();

        let pool_generation = renderer.pool().generation();
        let layout_generation = renderer.pool().layout_generation();
        let picker_generation = active_picker(&renderer).engine.registry_generation();
        let picker_buffer = active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap();
        let render_stamp = renderer.chart_render_stamp(chart).unwrap();
        let visual_revision = renderer.visual_revision();
        let old_min = renderer.pool().slot("x").unwrap().min;
        active_picker(&renderer).engine.fail_next_registry_prepare();

        assert!(matches!(
            renderer.upsert_column("x", &col_f64(vec![10.0, 11.0])),
            Err(FiggyError::GpuPick(
                crate::gpu_pick::GpuPickError::AllocationFailed {
                    resource: "injected registry prepare failure"
                }
            ))
        ));
        assert_eq!(renderer.pool().slot("x").unwrap().min, old_min);
        assert_eq!(renderer.pool().generation(), pool_generation);
        assert_eq!(renderer.pool().layout_generation(), layout_generation);
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
        assert_eq!(renderer.visual_revision(), visual_revision);
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            picker_generation
        );
        assert_eq!(
            active_picker(&renderer)
                .engine
                .query_params_buffer(0)
                .unwrap(),
            picker_buffer
        );
        assert!(!renderer.has_pending_maintenance());

        renderer
            .upsert_column("x", &col_f64(vec![10.0, 11.0]))
            .unwrap();
        assert_eq!(renderer.pool().slot("x").unwrap().min, 10.0);
        assert_ne!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
        assert!(active_picker(&renderer).engine.registry_generation() > picker_generation);
        assert_ne!(
            active_picker(&renderer)
                .engine
                .query_params_buffer(0)
                .unwrap(),
            picker_buffer
        );
    }

    #[test]
    fn active_picker_removal_failure_rolls_back_pool_chart_and_registry() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_scatter("scatter", None, "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();

        let pool_generation = renderer.pool().generation();
        let picker_generation = active_picker(&renderer).engine.registry_generation();
        let render_stamp = renderer.chart_render_stamp(chart).unwrap();
        active_picker(&renderer).engine.fail_next_registry_prepare();

        assert!(matches!(
            renderer.remove_column("x"),
            Err(FiggyError::GpuPick(
                crate::gpu_pick::GpuPickError::AllocationFailed {
                    resource: "injected registry prepare failure"
                }
            ))
        ));
        assert!(renderer.pool().slot("x").is_some());
        assert_eq!(renderer.pool().generation(), pool_generation);
        assert_eq!(renderer.chart_series(chart).unwrap().len(), 1);
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            picker_generation
        );
        assert_eq!(active_picker(&renderer).engine.registered_series_count(), 1);
        assert!(!renderer.has_pending_maintenance());

        assert!(renderer.remove_column("x").unwrap());
        assert!(renderer.pool().slot("x").is_none());
        assert!(renderer.chart_series(chart).unwrap().is_empty());
        assert_eq!(active_picker(&renderer).engine.registered_series_count(), 0);
        assert!(renderer.has_pending_maintenance());
    }

    #[test]
    fn column_removal_with_chart_config_picker_failure_preserves_all_authority() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let initial_config = crate::default::default_config();
        let initial_series = vec![state_test_scatter("scatter", None, "x", "y")];
        let chart = renderer
            .register_chart(initial_config.clone(), initial_series.clone())
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();

        let mut next_config = initial_config.clone();
        next_config.chart_title.text.segments =
            crate::text::rich_segments_from_text("atomic legend replacement");
        let pool_generation = renderer.pool().generation();
        let render_stamp = renderer.chart_render_stamp(chart).unwrap();
        let visual_revision = renderer.visual_revision();
        let picker_generation = active_picker(&renderer).engine.registry_generation();
        let picker_signature = active_picker(&renderer).signature.clone();
        active_picker(&renderer).engine.fail_next_registry_prepare();

        assert!(matches!(
            renderer.remove_column_with_chart_config("x", chart, next_config.clone()),
            Err(FiggyError::GpuPick(
                crate::gpu_pick::GpuPickError::AllocationFailed {
                    resource: "injected registry prepare failure"
                }
            ))
        ));
        assert!(renderer.pool().slot("x").is_some());
        assert_eq!(renderer.pool().generation(), pool_generation);
        assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
        assert_eq!(
            renderer.chart_series(chart).unwrap(),
            initial_series.as_slice()
        );
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
        assert_eq!(renderer.visual_revision(), visual_revision);
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            picker_generation
        );
        assert_eq!(active_picker(&renderer).signature, picker_signature);
        assert!(!renderer.has_pending_maintenance());

        assert!(
            renderer
                .remove_column_with_chart_config("x", chart, next_config.clone())
                .unwrap()
        );
        assert!(renderer.pool().slot("x").is_none());
        assert_eq!(renderer.chart_config(chart).unwrap(), &next_config);
        assert!(renderer.chart_series(chart).unwrap().is_empty());
        assert_eq!(active_picker(&renderer).engine.registered_series_count(), 0);
        assert!(renderer.has_pending_maintenance());
    }

    #[test]
    fn column_removal_with_chart_config_counter_failure_preserves_all_authority() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let initial_config = crate::default::default_config();
        let initial_series = vec![state_test_scatter("scatter", None, "x", "y")];
        let chart = renderer
            .register_chart(initial_config.clone(), initial_series.clone())
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let renderer_identity = renderer.renderer_identity;
        renderer
            .chart_states
            .get_mut(&chart)
            .unwrap()
            .revisions
            .config = RenderRevision {
            renderer_identity,
            sequence: u64::MAX,
        };

        let mut next_config = initial_config.clone();
        next_config.chart_title.text.segments =
            crate::text::rich_segments_from_text("must not publish");
        let pool_generation = renderer.pool().generation();
        let visual_revision = renderer.visual_revision();
        let picker_generation = active_picker(&renderer).engine.registry_generation();
        let picker_signature = active_picker(&renderer).signature.clone();
        let pending_maintenance = renderer.has_pending_maintenance();

        assert!(matches!(
            renderer.remove_column_with_chart_config("x", chart, next_config),
            Err(FiggyError::CounterExhausted {
                counter: "chart config revision"
            })
        ));
        assert!(renderer.pool().slot("x").is_some());
        assert_eq!(renderer.pool().generation(), pool_generation);
        assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
        assert_eq!(
            renderer.chart_series(chart).unwrap(),
            initial_series.as_slice()
        );
        assert_eq!(renderer.visual_revision(), visual_revision);
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            picker_generation
        );
        assert_eq!(active_picker(&renderer).signature, picker_signature);
        assert_eq!(renderer.has_pending_maintenance(), pending_maintenance);
    }

    #[test]
    fn column_removal_with_invalid_chart_config_preserves_all_authority() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let initial_config = crate::default::default_config();
        let initial_series = vec![state_test_scatter("scatter", None, "x", "y")];
        let chart = renderer
            .register_chart(initial_config.clone(), initial_series.clone())
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();

        let mut invalid_config = initial_config.clone();
        invalid_config.bottom_x.max = invalid_config.bottom_x.min;
        let pool_generation = renderer.pool().generation();
        let render_stamp = renderer.chart_render_stamp(chart).unwrap();
        let visual_revision = renderer.visual_revision();
        let picker_generation = active_picker(&renderer).engine.registry_generation();
        let picker_signature = active_picker(&renderer).signature.clone();

        assert!(matches!(
            renderer.remove_column_with_chart_config("x", chart, invalid_config),
            Err(FiggyError::InvalidConfig {
                field: "bottom_x",
                ..
            })
        ));
        assert!(renderer.pool().slot("x").is_some());
        assert_eq!(renderer.pool().generation(), pool_generation);
        assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
        assert_eq!(
            renderer.chart_series(chart).unwrap(),
            initial_series.as_slice()
        );
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
        assert_eq!(renderer.visual_revision(), visual_revision);
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            picker_generation
        );
        assert_eq!(active_picker(&renderer).signature, picker_signature);
        assert!(!renderer.has_pending_maintenance());
    }

    #[test]
    fn missing_column_does_not_publish_chart_config_override() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let initial_config = crate::default::default_config();
        let chart = renderer
            .register_chart(initial_config.clone(), Vec::new())
            .unwrap();
        let render_stamp = renderer.chart_render_stamp(chart).unwrap();
        let visual_revision = renderer.visual_revision();
        let mut next_config = initial_config.clone();
        next_config.chart_title.text.segments =
            crate::text::rich_segments_from_text("missing column must not publish");

        assert!(
            !renderer
                .remove_column_with_chart_config("missing", chart, next_config)
                .unwrap()
        );
        assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
        assert_eq!(renderer.visual_revision(), visual_revision);
        assert!(!renderer.has_pending_maintenance());
    }

    #[test]
    fn removing_errorbar_column_removes_active_scatter_picker_slot() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        renderer
            .add_column("y_err", &col_f64(vec![0.1, 0.2]))
            .unwrap();
        let series = SeriesConfig {
            series_id: "scatter-errors".into(),
            source_id: Some("source".into()),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: test_scatter_style(),
                err_y: ErrorRef::Symmetric {
                    column: "y_err".into(),
                },
                err_style: test_errorbar_style(),
            },
        };
        let chart = renderer
            .register_chart(crate::default::default_config(), vec![series])
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        assert_eq!(active_picker(&renderer).engine.registered_series_count(), 1);

        assert!(renderer.remove_column("y_err").unwrap());
        assert!(renderer.chart_series(chart).unwrap().is_empty());
        assert_eq!(active_picker(&renderer).engine.registered_series_count(), 0);
    }

    #[test]
    fn active_picker_defrag_failure_rolls_back_and_noop_keeps_registry_generation() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("hole", &col_f64(vec![9.0, 9.0]))
            .unwrap();
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_scatter("scatter", None, "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        assert!(renderer.remove_column("hole").unwrap());

        let old_x = renderer.pool().handle_for("x").unwrap();
        let pool_generation = renderer.pool().generation();
        let layout_generation = renderer.pool().layout_generation();
        let picker_generation = active_picker(&renderer).engine.registry_generation();
        active_picker(&renderer).engine.fail_next_registry_prepare();

        assert!(matches!(
            renderer.defragment(),
            Err(FiggyError::GpuPick(
                crate::gpu_pick::GpuPickError::AllocationFailed {
                    resource: "injected registry prepare failure"
                }
            ))
        ));
        let rolled_back_x = renderer.pool().handle_for("x").unwrap();
        assert_eq!(rolled_back_x.generation, old_x.generation);
        assert_eq!(rolled_back_x.offset, old_x.offset);
        assert_eq!(rolled_back_x.byte_size, old_x.byte_size);
        assert_eq!(rolled_back_x.len_values, old_x.len_values);
        assert_eq!(renderer.pool().generation(), pool_generation);
        assert_eq!(renderer.pool().layout_generation(), layout_generation);
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            picker_generation
        );
        assert!(renderer.has_pending_maintenance());

        assert!(renderer.defragment().unwrap());
        assert_ne!(
            renderer.pool().handle_for("x").unwrap().offset,
            old_x.offset
        );
        let rebound_generation = active_picker(&renderer).engine.registry_generation();
        assert!(rebound_generation > picker_generation);
        assert!(!renderer.has_pending_maintenance());

        assert!(!renderer.defragment().unwrap());
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            rebound_generation
        );
    }

    #[test]
    fn on_alloc_failure_compaction_rebinds_active_picker_transactionally() {
        let Some(mut renderer) = state_test_renderer_with_capacity(1024) else {
            return;
        };
        let block = || col_f64(vec![1.0; 32]);
        renderer.add_column("x", &block()).unwrap();
        renderer.add_column("hole-a", &block()).unwrap();
        renderer.add_column("y", &block()).unwrap();
        renderer.add_column("hole-b", &block()).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_scatter("scatter", None, "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let picker_generation = active_picker(&renderer).engine.registry_generation();
        let picker_buffer = active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap();
        let old_y_offset = renderer.pool().handle_for("y").unwrap().offset;

        assert!(renderer.remove_column("hole-a").unwrap());
        assert!(renderer.remove_column("hole-b").unwrap());
        renderer.set_defrag_policy(DefragPolicy::OnAllocFailure);
        renderer.add_column("new", &col_f64(vec![2.0; 64])).unwrap();

        assert_ne!(
            renderer.pool().handle_for("y").unwrap().offset,
            old_y_offset
        );
        assert!(active_picker(&renderer).engine.registry_generation() > picker_generation);
        assert_eq!(
            active_picker(&renderer)
                .engine
                .query_params_buffer(0)
                .unwrap(),
            picker_buffer
        );
        assert!(!renderer.has_pending_maintenance());
    }

    #[test]
    fn dropped_renderer_column_upsert_rolls_back_pool_and_chart_state() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let visual_before = renderer.visual_revision();
        let desired_before = renderer.chart_states[&chart].revisions.desired;
        let min_before = renderer.pool().slot("x").unwrap().min;

        {
            let guard = renderer
                .begin_upsert_column("x", &col_f64(vec![20.0, 21.0]))
                .unwrap();
            assert_eq!(guard.pool().slot("x").unwrap().min, 20.0);
        }

        assert_eq!(renderer.pool().slot("x").unwrap().min, min_before);
        assert_eq!(renderer.visual_revision(), visual_before);
        assert_eq!(
            renderer.chart_states[&chart].revisions.desired,
            desired_before
        );
    }

    #[test]
    fn provisional_upsert_snapshot_matches_commit_and_disappears_on_drop() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let before = renderer.web_derived_snapshot(chart).unwrap();

        {
            let guard = renderer
                .begin_upsert_column("x", &col_f64(vec![10.0, 11.0]))
                .unwrap();
            let prospective = guard.web_derived_snapshot(chart).unwrap();
            assert_ne!(prospective.stamp(), before.stamp());
            assert_ne!(prospective.fit_token(), before.fit_token());
        }
        assert_eq!(
            renderer.web_derived_snapshot(chart).unwrap().stamp(),
            before.stamp()
        );
        assert_eq!(
            renderer.web_derived_snapshot(chart).unwrap().fit_token(),
            before.fit_token()
        );

        let (prospective_stamp, prospective_fit_token) = {
            let guard = renderer
                .begin_upsert_column("x", &col_f64(vec![20.0, 21.0]))
                .unwrap();
            let snapshot = guard.web_derived_snapshot(chart).unwrap();
            let stamp = snapshot.stamp().clone();
            let fit_token = snapshot.fit_token().clone();
            guard.commit();
            (stamp, fit_token)
        };
        assert_eq!(
            renderer.web_derived_snapshot(chart).unwrap().stamp(),
            &prospective_stamp
        );
        assert_eq!(
            renderer.web_derived_snapshot(chart).unwrap().fit_token(),
            &prospective_fit_token
        );
    }

    #[test]
    fn renderer_column_remove_cascades_across_registered_charts() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        for (id, values) in [
            ("x", vec![0.0, 1.0]),
            ("y", vec![1.0, 2.0]),
            ("z", vec![2.0, 3.0]),
        ] {
            renderer.add_column(id, &col_f64(values)).unwrap();
        }
        let first = renderer
            .register_chart(
                crate::default::default_config(),
                vec![
                    state_test_line("removed", "x", "y"),
                    state_test_line("kept", "z", "y"),
                ],
            )
            .unwrap();
        let second = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("also-removed", "x", "y")],
            )
            .unwrap();
        let first_raster = renderer.chart_states[&first].revisions.raster;
        let second_raster = renderer.chart_states[&second].revisions.raster;
        let second_config = renderer.chart_config(second).unwrap().clone();
        let mut next_first_config = renderer.chart_config(first).unwrap().clone();
        next_first_config.chart_title.text.segments =
            crate::text::rich_segments_from_text("first chart replacement");

        assert!(
            renderer
                .remove_column_with_chart_config("x", first, next_first_config.clone())
                .unwrap()
        );
        assert!(renderer.pool().slot("x").is_none());
        assert_eq!(renderer.chart_config(first).unwrap(), &next_first_config);
        assert_eq!(renderer.chart_config(second).unwrap(), &second_config);
        assert_eq!(renderer.chart_series(first).unwrap().len(), 1);
        assert_eq!(renderer.chart_series(first).unwrap()[0].series_id, "kept");
        assert!(renderer.chart_series(second).unwrap().is_empty());
        assert_ne!(renderer.chart_states[&first].revisions.raster, first_raster);
        assert_ne!(
            renderer.chart_states[&second].revisions.raster,
            second_raster
        );
        assert!(renderer.has_pending_maintenance());
    }

    #[test]
    fn renderer_column_revision_exhaustion_preflights_before_pool_mutation() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let renderer_identity = renderer.renderer_identity;
        renderer
            .chart_states
            .get_mut(&chart)
            .unwrap()
            .revisions
            .desired = RenderRevision {
            renderer_identity,
            sequence: u64::MAX,
        };
        let min_before = renderer.pool().slot("x").unwrap().min;

        let error = renderer
            .begin_upsert_column("x", &col_f64(vec![30.0, 31.0]))
            .err()
            .expect("revision exhaustion must reject before the pool changes");
        assert!(matches!(error, FiggyError::CounterExhausted { .. }));
        assert_eq!(renderer.pool().slot("x").unwrap().min, min_before);
    }

    #[test]
    fn web_derived_stamp_tracks_layout_while_fit_token_survives_defrag_only() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("hole", &col_f64(vec![9.0, 9.0]))
            .unwrap();
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let before = renderer.web_derived_snapshot(chart).unwrap();
        let token = renderer.begin_fit_commit(chart).unwrap();

        assert!(renderer.remove_column("hole").unwrap());
        assert!(renderer.defragment().unwrap());
        let after = renderer.web_derived_snapshot(chart).unwrap();
        assert_ne!(before.stamp(), after.stamp());

        let x_extent = crate::chart::FitExtent {
            min: 10.0,
            max: 20.0,
            min_positive: Some(10.0),
        };
        let y_extent = crate::chart::FitExtent {
            min: -5.0,
            max: 5.0,
            min_positive: Some(5.0),
        };
        renderer
            .commit_auto_fit_all_if_current(&token, &x_extent, &y_extent, 0.0)
            .unwrap();
        let config = renderer.chart_config(chart).unwrap();
        assert_eq!((config.bottom_x.min, config.bottom_x.max), (10.0, 20.0));
        assert_eq!((config.left_y.min, config.left_y.max), (-5.0, 5.0));
    }

    #[test]
    fn web_derived_stamp_tracks_picker_source_identity() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let before = renderer.web_derived_snapshot(chart).unwrap();
        let mut series = renderer.chart_series(chart).unwrap()[0].clone();
        series.source_id = Some("replacement-source".to_string());

        renderer.set_chart_series(chart, vec![series]).unwrap();

        let after = renderer.web_derived_snapshot(chart).unwrap();
        assert_ne!(before.stamp(), after.stamp());
    }

    #[test]
    fn resident_fit_token_survives_source_identity_only_edit() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let token = renderer.begin_fit_commit(chart).unwrap();
        let mut series = renderer.chart_series(chart).unwrap()[0].clone();
        series.source_id = Some("replacement-source".to_string());
        renderer.set_chart_series(chart, vec![series]).unwrap();
        let extent = crate::FitExtent {
            min: 10.0,
            max: 20.0,
            min_positive: Some(10.0),
        };
        renderer
            .commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0)
            .unwrap();
        assert_eq!(
            renderer.chart_series(chart).unwrap()[0]
                .source_id
                .as_deref(),
            Some("replacement-source")
        );
    }

    #[test]
    fn stale_fit_token_rejects_without_partially_updating_axes() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let token = renderer.begin_fit_commit(chart).unwrap();
        let mut changed = renderer.chart_config(chart).unwrap().clone();
        changed.bottom_x.min = -25.0;
        changed.bottom_x.max = 25.0;
        changed.left_y.min = -50.0;
        changed.left_y.max = 50.0;
        renderer.set_chart_config(chart, changed.clone()).unwrap();

        let extent = crate::chart::FitExtent {
            min: 100.0,
            max: 200.0,
            min_positive: Some(100.0),
        };
        assert!(matches!(
            renderer.commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0),
            Err(FiggyError::StaleStateToken { .. })
        ));
        assert_eq!(renderer.chart_config(chart).unwrap(), &changed);
    }

    #[test]
    fn fit_token_tracks_referenced_column_content() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let token = renderer.begin_fit_commit(chart).unwrap();
        renderer
            .upsert_column("x", &col_f64(vec![10.0, 11.0]))
            .unwrap();

        let extent = crate::chart::FitExtent {
            min: 10.0,
            max: 11.0,
            min_positive: Some(10.0),
        };
        assert!(matches!(
            renderer.commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0),
            Err(FiggyError::StaleStateToken { .. })
        ));
    }

    #[test]
    fn fit_token_tracks_normalized_series_extent_mode() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("series", "x", "y")],
            )
            .unwrap();
        let line_token = renderer.begin_fit_commit(chart).unwrap();
        let mut scatter = state_test_line("series", "x", "y");
        scatter.render_type = DataRenderType::Scatter {
            scatter: test_scatter_style(),
        };
        renderer.set_chart_series(chart, vec![scatter]).unwrap();

        let extent = crate::chart::FitExtent {
            min: 10.0,
            max: 20.0,
            min_positive: Some(10.0),
        };
        assert!(matches!(
            renderer.commit_auto_fit_all_if_current(&line_token, &extent, &extent, 0.0),
            Err(FiggyError::StaleStateToken { .. })
        ));
    }

    #[test]
    fn fit_token_ignores_title_only_config_change_and_preserves_it_on_commit() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let token = renderer.begin_fit_commit(chart).unwrap();
        let mut config = renderer.chart_config(chart).unwrap().clone();
        config.chart_title.visible = true;
        config.chart_title.text.segments = crate::text::rich_segments_from_text("newer title");
        renderer.set_chart_config(chart, config).unwrap();

        let extent = crate::chart::FitExtent {
            min: 10.0,
            max: 20.0,
            min_positive: Some(10.0),
        };
        renderer
            .commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0)
            .unwrap();
        let committed = renderer.chart_config(chart).unwrap();
        assert_eq!(
            committed
                .chart_title
                .text
                .segments
                .iter()
                .map(|segment| segment.text)
                .collect::<String>(),
            "newer title"
        );
        assert_eq!(
            (committed.bottom_x.min, committed.bottom_x.max),
            (10.0, 20.0)
        );
    }

    #[test]
    fn foreign_or_removed_fit_token_is_stale_not_an_unknown_chart_alias() {
        let Some(mut first) = state_test_renderer() else {
            return;
        };
        let Some(mut second) = state_test_renderer() else {
            return;
        };
        for renderer in [&mut first, &mut second] {
            renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
            renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
        }
        let first_chart = first
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let second_chart = second
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("line", "x", "y")],
            )
            .unwrap();
        let token = first.begin_fit_commit(first_chart).unwrap();
        let extent = crate::chart::FitExtent {
            min: 10.0,
            max: 20.0,
            min_positive: Some(10.0),
        };
        assert!(matches!(
            second.commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0),
            Err(FiggyError::StaleStateToken { .. })
        ));

        first.remove_chart(first_chart).unwrap();
        assert!(matches!(
            first.commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0),
            Err(FiggyError::StaleStateToken { .. })
        ));
        assert!(second.chart_config(second_chart).is_ok());
    }

    fn typecheck_windowed_load_demo_forwarding<'surface>(
        renderer: &mut WindowedRenderer<'surface>,
        chart_id: ChartId,
        source: &dyn ColumnSource,
    ) -> Result<()> {
        let guard = renderer.begin_load_demo(chart_id, source, source, source, source, |_| {
            Ok((crate::default::default_config(), Vec::new()))
        })?;
        drop(guard);
        Ok(())
    }

    #[test]
    fn windowed_load_demo_forwarding_compile_contract() {
        // Runtime construction needs a live surface. Compiling this helper
        // verifies the forwarding call and guard lifetime without fabricating one.
        fn assert_compiles<T>(_value: T) {}
        assert_compiles(typecheck_windowed_load_demo_forwarding);
    }

    #[test]
    fn renderer_begins_errorbar_extent_from_current_column_ids() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut renderer = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        renderer
            .add_column("fit-value", &col_f64(vec![0.0, 10.0]))
            .unwrap();
        renderer
            .add_column("fit-lower", &col_f64(vec![1.0, 5.0]))
            .unwrap();
        renderer
            .add_column("fit-upper", &col_f64(vec![1.0, 5.0]))
            .unwrap();
        pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();

        match renderer.begin_errorbar_extent("fit-value", "missing", "fit-upper") {
            Err(crate::gpu_errorbar::GpuErrorbarError::UnknownColumn { role, id }) => {
                assert_eq!(role, "lower error");
                assert_eq!(id, "missing");
            }
            _ => panic!("missing lower error column must be reported before submission"),
        }

        let ticket = renderer
            .begin_errorbar_extent("fit-value", "fit-lower", "fit-upper")
            .unwrap();
        let extent = pollster::block_on(ticket.resolve()).unwrap().unwrap();
        assert_eq!(extent.min, -1.0);
        assert_eq!(extent.max, 15.0);
        assert_eq!(extent.min_positive, Some(1.0));
    }

    #[test]
    fn renderer_series_extent_uses_role_checked_provisional_mappings() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut renderer = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        renderer
            .add_column("series-fit-x", &col_f64(vec![1.0, 2.0]))
            .unwrap();
        renderer
            .add_column("series-fit-y", &col_f64(vec![10.0, 20.0]))
            .unwrap();
        pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();
        let ids = crate::gpu_errorbar::GpuSeriesExtentColumnIds {
            x: "series-fit-x",
            y: "series-fit-y",
            x_lower: None,
            x_upper: None,
            y_lower: None,
            y_upper: None,
        };

        match renderer.begin_series_extent(
            crate::gpu_errorbar::GpuSeriesExtentMode::Points,
            crate::gpu_errorbar::GpuSeriesExtentColumnIds {
                x: "missing",
                ..ids
            },
        ) {
            Err(crate::gpu_errorbar::GpuErrorbarError::UnknownColumn { role, id }) => {
                assert_eq!(role, "x");
                assert_eq!(id, "missing");
            }
            _ => panic!("missing x column must be reported before submission"),
        }
        match renderer.begin_series_extent(crate::gpu_errorbar::GpuSeriesExtentMode::PointsX, ids) {
            Err(crate::gpu_errorbar::GpuErrorbarError::MissingSeriesColumn { role, .. }) => {
                assert_eq!(role, "x lower error");
            }
            _ => panic!("active x errors require role-labelled lower and upper columns"),
        }

        let rollback_ticket = {
            let upsert = renderer
                .begin_upsert_hilo_column("series-fit-x", &col_f64(vec![100.0, 200.0]))
                .unwrap();
            upsert
                .begin_series_extent(crate::gpu_errorbar::GpuSeriesExtentMode::Points, ids)
                .unwrap()
        };
        let provisional = pollster::block_on(rollback_ticket.resolve())
            .unwrap()
            .unwrap();
        assert_eq!((provisional.x.min, provisional.x.max), (100.0, 200.0));
        let rolled_back = pollster::block_on(
            renderer
                .begin_series_extent(crate::gpu_errorbar::GpuSeriesExtentMode::Points, ids)
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!((rolled_back.x.min, rolled_back.x.max), (1.0, 2.0));

        let committed_ticket = {
            let upsert = renderer
                .begin_upsert_hilo_column("series-fit-x", &col_f64(vec![300.0, 400.0]))
                .unwrap();
            let ticket = upsert
                .begin_series_extent(crate::gpu_errorbar::GpuSeriesExtentMode::Points, ids)
                .unwrap();
            upsert.commit();
            ticket
        };
        let committed = pollster::block_on(committed_ticket.resolve())
            .unwrap()
            .unwrap();
        assert_eq!((committed.x.min, committed.x.max), (300.0, 400.0));
    }

    fn empty_chart_items<'a>(
        renderer: &Renderer,
        chart: &'a Chart,
        view: &'a ChartView,
    ) -> [ChartDrawItem<'a>; 1] {
        let _ = renderer;
        [ChartDrawItem {
            view,
            chart_config: chart.config(),
            series: &[],
        }]
    }

    #[test]
    fn empty_prepare_compiles_only_axis_and_leaves_extent_engine_idle() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let chart = Chart::new(config);
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let items = empty_chart_items(&renderer, &chart, &view);
        renderer.prepare(&items).unwrap();

        assert!(renderer.line_pipeline().is_none());
        assert!(renderer.scatter_pipeline().is_none());
        assert!(renderer.errorbar_pipeline().is_none());
        assert!(renderer.bar_pipeline().is_none());
        assert!(renderer.pipelines.scatter_mapped.is_none());
        assert!(renderer.pipelines.errorbar_mapped.is_none());
        assert!(renderer.pipelines.pick_ring_mapped.is_none());
        assert!(!renderer.errorbar_extent_engine_ready());
        pollster::block_on(renderer.wait_submitted_work());
    }

    #[test]
    fn full_prewarm_materializes_every_deferred_pipeline() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let mut events = Vec::new();
        pollster::block_on(renderer.prewarm_all_observed(&mut |event| events.push(event))).unwrap();

        assert!(renderer.pipelines.line.is_some());
        assert!(renderer.pipelines.scatter.is_some());
        assert!(renderer.pipelines.scatter_mapped.is_some());
        assert!(renderer.pipelines.pick_ring.is_some());
        assert!(renderer.pipelines.pick_ring_mapped.is_some());
        assert!(renderer.pipelines.errorbar.is_some());
        assert!(renderer.pipelines.errorbar_mapped.is_some());
        assert!(renderer.pipelines.bar.is_some());
        assert!(renderer.pipelines.bar_selection.is_some());
        assert!(renderer.pipelines.field.is_some());
        assert!(renderer.pipelines.field_selection.is_some());
        assert!(renderer.pipelines.contour.is_some());
        assert!(renderer.pipelines.contour_label.is_some());
        assert!(renderer.pipelines.styled.contains_key(&StyleKey::Sketch));
        assert!(renderer.pipelines.styled.contains_key(&StyleKey::Milkyway));
        assert!(
            renderer
                .pipelines
                .styled
                .contains_key(&StyleKey::Constellation)
        );
        assert!(renderer.contour_label_pipelines.is_some());
        assert!(renderer.arc_pipelines.is_some());
        assert!(renderer.errorbar_extent_engine_ready());
        assert!(events.iter().any(|event| {
            event.scope == "renderer.prewarm"
                && event.stage == "point picking compute"
                && event.phase == crate::init::InitPhase::Finished
        }));
    }

    // ── Histogram (W5) ─────────────────────────────────────────────────────

    fn state_test_histogram(
        id: &str,
        x: &str,
        y: &str,
        orientation: crate::data_config::BarOrientation,
    ) -> SeriesConfig {
        SeriesConfig {
            series_id: id.to_string(),
            source_id: None,
            label: None,
            x_column: x.to_string(),
            y_column: y.to_string(),
            render_type: DataRenderType::Histogram {
                bar: crate::data_config::DataBarStyleConfig {
                    fill_color: Color::new(0.2, 0.5, 0.8, 1.0),
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    baseline: 0.0,
                    gap_px: 1.0,
                    width_ratio: 1.0,
                    orientation,
                    bar_style_overrides: None,
                },
            },
        }
    }

    /// A histogram compiles the bar pipeline and nothing else. Eager creation
    /// would break the other isolation tests in this module, and would put a
    /// pipeline on every chart that never draws a bar.
    #[test]
    fn histogram_prepare_compiles_only_the_bar_pipeline() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        // 4 bins: 5 edges, 4 counts.
        renderer
            .add_column("edges", &col_f64(vec![0.0, 1.0, 2.0, 3.0, 4.0]))
            .unwrap();
        renderer
            .add_column("counts", &col_f64(vec![3.0, 7.0, 5.0, 2.0]))
            .unwrap();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let chart = Chart::new(config);
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let series_cfg = state_test_histogram(
            "hist",
            "edges",
            "counts",
            crate::data_config::BarOrientation::Vertical,
        );
        let style = renderer.create_style_for_series(&series_cfg).unwrap();
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        renderer.prepare(&items).unwrap();

        assert!(renderer.bar_pipeline().is_some());
        assert!(renderer.line_pipeline().is_none());
        assert!(renderer.scatter_pipeline().is_none());
        assert!(renderer.errorbar_pipeline().is_none());
        assert!(renderer.pipelines.scatter_mapped.is_none());
        assert!(renderer.pipelines.errorbar_mapped.is_none());
        assert!(!renderer.errorbar_extent_engine_ready());
    }

    /// A bar is not a line: the line draw call issues `min(x, y) - 1`
    /// instances, which for a correctly shaped `(n + 1, n)` histogram drops the
    /// last bar. The bar count is `min(edges - 1, counts)`.
    #[test]
    fn bar_instance_count_keeps_the_last_bin() {
        use crate::data_render::bar_instance_count;

        // Well-formed: 11 edges bound 10 bins.
        assert_eq!(bar_instance_count(11, 10), 10);
        // What the line rule would have given for the same columns: one bar
        // short, the last bin silently gone.
        let line_rule = |a: usize, b: usize| a.min(b).saturating_sub(1);
        assert_eq!(line_rule(11, 10), 9);

        // Mismatches truncate to the smallest common extent, never error.
        assert_eq!(bar_instance_count(11, 9), 9);
        assert_eq!(bar_instance_count(5, 10), 4);
        assert_eq!(bar_instance_count(1, 10), 0);
        assert_eq!(bar_instance_count(0, 10), 0);
        assert_eq!(bar_instance_count(11, 0), 0);
    }

    /// The smallest-common-extent reporting window. A mismatch is drawn small and *reported* — the
    /// one place a host can learn its declaration outran its data.
    #[test]
    fn series_draw_info_reports_the_truncation_it_drew() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("edges", &col_f64(vec![0.0, 1.0, 2.0, 3.0, 4.0]))
            .unwrap();
        renderer
            .add_column("counts4", &col_f64(vec![3.0, 7.0, 5.0, 2.0]))
            .unwrap();
        renderer
            .add_column("counts2", &col_f64(vec![3.0, 7.0]))
            .unwrap();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let chart = renderer.register_chart(config, Vec::new()).unwrap();

        // 5 edges + 4 counts: exactly 4 bars, nothing left over.
        renderer
            .set_chart_series(
                chart,
                vec![state_test_histogram(
                    "hist",
                    "edges",
                    "counts4",
                    crate::data_config::BarOrientation::Vertical,
                )],
            )
            .unwrap();
        assert_eq!(
            renderer.series_draw_info(chart, "hist").unwrap(),
            SeriesDrawInfo {
                drawn_count: 4,
                cols: None,
                rows: None,
                truncated: false,
            }
        );

        // 5 edges + 2 counts: two bars, and the surplus edges are reported.
        renderer
            .set_chart_series(
                chart,
                vec![state_test_histogram(
                    "hist",
                    "edges",
                    "counts2",
                    crate::data_config::BarOrientation::Vertical,
                )],
            )
            .unwrap();
        let info = renderer.series_draw_info(chart, "hist").unwrap();
        assert_eq!(info.drawn_count, 2);
        assert!(info.truncated);

        // Orientation alone decides the roles. Same two columns in the same
        // slots, `Horizontal` instead of `Vertical`: now `y` (2 values) is the
        // edge column, bounding a single bin, against 5 values in `x`.
        renderer
            .set_chart_series(
                chart,
                vec![state_test_histogram(
                    "hist",
                    "edges",
                    "counts2",
                    crate::data_config::BarOrientation::Horizontal,
                )],
            )
            .unwrap();
        let info = renderer.series_draw_info(chart, "hist").unwrap();
        assert_eq!(
            info.drawn_count, 1,
            "horizontal reads y as edges: 2 edges bound 1 bin"
        );
        assert!(info.truncated);

        // The same window explains the existing primitives' `min` truncation.
        renderer
            .add_column("lx", &col_f64(vec![0.0, 1.0, 2.0]))
            .unwrap();
        renderer.add_column("ly", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer
            .set_chart_series(chart, vec![state_test_line("line", "lx", "ly")])
            .unwrap();
        let info = renderer.series_draw_info(chart, "line").unwrap();
        assert_eq!(info.drawn_count, 1, "two usable points make one segment");
        assert!(info.truncated);

        // And an unknown series is an error, not a zero.
        assert!(renderer.series_draw_info(chart, "nope").is_err());
    }

    #[test]
    fn line_prepare_compiles_only_the_line_pipeline() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("lx", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("ly", &col_f64(vec![0.0, 1.0])).unwrap();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let chart = Chart::new(config);
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let series_cfg = state_test_line("line", "lx", "ly");
        let style = renderer.create_style_for_series(&series_cfg).unwrap();
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        renderer.prepare(&items).unwrap();
        assert!(renderer.line_pipeline().is_some());
        assert!(renderer.scatter_pipeline().is_none());
        assert!(renderer.errorbar_pipeline().is_none());
        assert!(renderer.bar_pipeline().is_none());
        assert!(renderer.pipelines.scatter_mapped.is_none());
        assert!(renderer.pipelines.errorbar_mapped.is_none());
        assert!(renderer.pipelines.pick_ring_mapped.is_none());
        assert!(!renderer.errorbar_extent_engine_ready());
    }

    #[test]
    fn scatter_prepare_compiles_only_the_scatter_pipeline() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("sx", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("sy", &col_f64(vec![0.0, 1.0])).unwrap();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let chart = Chart::new(config);
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let series_cfg = SeriesConfig {
            series_id: "scatter".into(),
            source_id: None,
            label: None,
            x_column: "sx".into(),
            y_column: "sy".into(),
            render_type: DataRenderType::Scatter {
                scatter: test_scatter_style(),
            },
        };
        let style = renderer.create_style_for_series(&series_cfg).unwrap();
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        renderer.prepare(&items).unwrap();
        assert!(renderer.line_pipeline().is_none());
        assert!(renderer.scatter_pipeline().is_some());
        assert!(renderer.errorbar_pipeline().is_none());
        assert!(
            renderer.pipelines.scatter_mapped.is_none(),
            "unmapped scatter must not compile the mapped pipeline"
        );
        assert!(renderer.pipelines.errorbar_mapped.is_none());
        assert!(renderer.pipelines.pick_ring_mapped.is_none());
    }

    #[test]
    fn errorbar_prepare_compiles_scatter_and_errorbar_pipelines() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("ex", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("ey", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("ee", &col_f64(vec![0.1, 0.1])).unwrap();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let chart = Chart::new(config);
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let series_cfg = SeriesConfig {
            series_id: "errorbar".into(),
            source_id: None,
            label: None,
            x_column: "ex".into(),
            y_column: "ey".into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: test_scatter_style(),
                err_y: ErrorRef::Symmetric {
                    column: "ee".into(),
                },
                err_style: test_errorbar_style(),
            },
        };
        let style = renderer.create_style_for_series(&series_cfg).unwrap();
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        renderer.prepare(&items).unwrap();
        assert!(renderer.line_pipeline().is_none());
        assert!(renderer.scatter_pipeline().is_some());
        assert!(renderer.errorbar_pipeline().is_some());
        assert!(
            renderer.pipelines.scatter_mapped.is_none(),
            "errorbar without a style map must not compile mapped pipelines"
        );
        assert!(renderer.pipelines.errorbar_mapped.is_none());
        assert!(renderer.pipelines.pick_ring_mapped.is_none());
        assert!(!renderer.errorbar_extent_engine_ready());
    }

    #[test]
    fn shared_extent_job_requires_explicit_engine_preparation() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("fit-value", &col_f64(vec![0.0, 10.0]))
            .unwrap();
        renderer
            .add_column("fit-lower", &col_f64(vec![1.0, 5.0]))
            .unwrap();
        renderer
            .add_column("fit-upper", &col_f64(vec![1.0, 5.0]))
            .unwrap();
        assert!(!renderer.errorbar_extent_engine_ready());
        assert!(matches!(
            renderer.begin_errorbar_extent("fit-value", "fit-lower", "fit-upper"),
            Err(crate::gpu_errorbar::GpuErrorbarError::EngineNotReady)
        ));
        pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();
        assert!(renderer.errorbar_extent_engine_ready());
        renderer
            .begin_errorbar_extent("fit-value", "fit-lower", "fit-upper")
            .unwrap();
    }

    #[test]
    fn failed_extent_prewarm_leaves_engine_unpublished() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        // Native warmup is a no-op and the public API intentionally has no
        // failure injection, so exercise the private post-warm publication
        // boundary with the same error returned by a failed wasm warmup.
        let error = crate::gpu_errorbar::GpuErrorbarError::AsyncCompileFailed(
            "injected prewarm failure".into(),
        );
        assert_eq!(
            renderer.finish_errorbar_extent_engine_preparation(Err(error.clone())),
            Err(error)
        );
        assert!(!renderer.errorbar_extent_engine_ready());
        assert!(matches!(
            renderer.begin_series_extent(
                crate::gpu_errorbar::GpuSeriesExtentMode::Points,
                crate::gpu_errorbar::GpuSeriesExtentColumnIds {
                    x: "missing-x",
                    y: "missing-y",
                    x_lower: None,
                    x_upper: None,
                    y_lower: None,
                    y_upper: None,
                },
            ),
            Err(crate::gpu_errorbar::GpuErrorbarError::EngineNotReady)
        ));
    }

    #[test]
    fn point_constellation_scatterline_exports() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        r.add_column("pc_x", &col_f64(vec![0.0, 0.25, 0.5, 0.75, 1.0]))
            .unwrap();
        r.add_column("pc_y", &col_f64(vec![0.2, 0.75, 0.35, 0.9, 0.5]))
            .unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        config.legend.visible = false;
        config.draw_style =
            crate::config::DrawStyle::Constellation(crate::config::ConstellationOptions::default());
        let mut chart = Chart::new(config);
        chart.set_x_range(-0.05, 1.05);
        chart.set_y_range(0.0, 1.0);

        let series = [SeriesConfig {
            series_id: "pc".into(),
            source_id: None,
            label: None,
            x_column: "pc_x".into(),
            y_column: "pc_y".into(),
            render_type: DataRenderType::ScatterLine {
                scatter: DataScatterStyleConfig {
                    point_color: Color::new(1.0, 1.0, 1.0, 1.0),
                    point_shape: crate::data_config::ScatterShape::CircleFilled,
                    point_size: 5.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
                line: DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(0.4, 0.7, 1.0, 1.0),
                    line_width: 2.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }];
        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let lit = img.rgba.chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(
            lit > 100,
            "point constellation export produced too little ink: {lit}"
        );
    }

    #[test]
    fn all_scatter_shapes_export_visible_markers() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();

        let shapes = [
            crate::data_config::ScatterShape::Circle,
            crate::data_config::ScatterShape::Square,
            crate::data_config::ScatterShape::Triangle,
            crate::data_config::ScatterShape::Diamond,
            crate::data_config::ScatterShape::Cross,
            crate::data_config::ScatterShape::CircleFilled,
            crate::data_config::ScatterShape::SquareFilled,
            crate::data_config::ScatterShape::TriangleFilled,
            crate::data_config::ScatterShape::DiamondFilled,
            crate::data_config::ScatterShape::TriangleDown,
            crate::data_config::ScatterShape::TriangleLeft,
            crate::data_config::ScatterShape::TriangleRight,
            crate::data_config::ScatterShape::Plus,
            crate::data_config::ScatterShape::Pentagon,
            crate::data_config::ScatterShape::Hexagon,
            crate::data_config::ScatterShape::Octagon,
            crate::data_config::ScatterShape::Star,
            crate::data_config::ScatterShape::TriangleDownFilled,
            crate::data_config::ScatterShape::TriangleLeftFilled,
            crate::data_config::ScatterShape::TriangleRightFilled,
            crate::data_config::ScatterShape::PlusFilled,
            crate::data_config::ScatterShape::CrossFilled,
            crate::data_config::ScatterShape::PentagonFilled,
            crate::data_config::ScatterShape::HexagonFilled,
            crate::data_config::ScatterShape::OctagonFilled,
            crate::data_config::ScatterShape::StarFilled,
        ];

        let mut series = Vec::new();
        for (i, shape) in shapes.iter().enumerate() {
            let x = (i % 7) as f64;
            let y = (i / 7) as f64;
            let x_id = format!("shape_x_{i}");
            let y_id = format!("shape_y_{i}");
            r.add_column(&x_id, &col_f64(vec![x])).unwrap();
            r.add_column(&y_id, &col_f64(vec![y])).unwrap();
            series.push(SeriesConfig {
                series_id: format!("shape_{i}"),
                source_id: None,
                label: None,
                x_column: x_id,
                y_column: y_id,
                render_type: DataRenderType::Scatter {
                    scatter: DataScatterStyleConfig {
                        point_color: Color::new(1.0, 0.0, 0.0, 1.0),
                        point_shape: shape.clone(),
                        point_size: 8.0,
                        point_style_table: None,
                        point_style_index_column: None,
                        point_style_overrides: None,
                        point_color_map: None,
                        point_size_map: None,
                    },
                },
            });
        }

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 700,
            height: 400,
        });
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(-0.5, 6.5);
        chart.set_y_range(-0.5, 3.8);

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let red_ink = img
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0] > 150 && p[1] < 80 && p[2] < 80 && p[3] > 100)
            .count();
        assert!(
            red_ink > 600,
            "scatter shape export produced too little red ink: {red_ink}"
        );
    }

    #[test]
    fn xy_errorbar_does_not_require_zero_column() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        r.add_column("x", &col_f64(vec![1.0, 2.0, 3.0])).unwrap();
        r.add_column("y", &col_f64(vec![1.0, 3.0, 2.0])).unwrap();
        r.add_column("ex", &col_f64(vec![0.1, 0.2, 0.1])).unwrap();
        r.add_column("ey", &col_f64(vec![0.3, 0.1, 0.2])).unwrap();

        let chart = basic_errorbar_chart();
        let series = [SeriesConfig {
            series_id: "xy".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::ScatterErrorbarXY {
                scatter: test_scatter_style(),
                err_x: ErrorRef::Symmetric {
                    column: "ex".into(),
                },
                err_y: ErrorRef::Symmetric {
                    column: "ey".into(),
                },
                err_style: test_errorbar_style(),
            },
        }];

        r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    }

    /// A zero-size marker draws a dot and picks (`pick_draw_agreement`), so a
    /// picked one takes the ring like any other marker.
    #[test]
    fn picked_overlay_rings_a_zero_size_scatter_errorbar_anchor() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        r.add_column("x", &col_f64(vec![2.0])).unwrap();
        r.add_column("y", &col_f64(vec![2.5])).unwrap();
        r.add_column("ey", &col_f64(vec![1.0])).unwrap();

        let mut chart = basic_errorbar_chart();
        chart.config_mut().picked_points = Some(crate::config::PickedPointsConfig {
            visible: true,
            refs: vec![crate::config::PickedPointRef {
                source_id: None,
                series_id: "err".into(),
                point_index: 0,
            }],
            ring_color: Color::new(0.0, 1.0, 0.0, 1.0),
            ring_width_px: 3.0,
            radius_extra_px: 8.0,
        });
        let mut scatter = test_scatter_style();
        scatter.point_size = 0.0;
        let series = [SeriesConfig {
            series_id: "err".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter,
                err_y: ErrorRef::Symmetric {
                    column: "ey".into(),
                },
                err_style: test_errorbar_style(),
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let green_ring_ink = img
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0] < 80 && p[1] > 160 && p[2] < 80 && p[3] > 80)
            .count();

        assert!(
            green_ring_ink > 20,
            "a picked zero-size marker over an errorbar takes the ring: {green_ring_ink}"
        );
    }

    #[test]
    fn errorbar_style_index_colors_points_independently() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        r.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
        r.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
        r.add_column("ey", &col_f64(vec![1.0, 1.0])).unwrap();
        r.add_column("err_style", &col_f64(vec![0.0, 1.0])).unwrap();

        let chart = basic_errorbar_chart();
        let mut scatter = test_scatter_style();
        scatter.point_size = 0.0;
        let series = [SeriesConfig {
            series_id: "mapped-errorbar".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter,
                err_y: ErrorRef::Symmetric {
                    column: "ey".into(),
                },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: Color::BLACK,
                    error_bar_width: 2.0,
                    error_bar_cap_size: 7.0,
                    cap_width: 2.0,
                    error_bar_style_table: Some(vec![
                        DataErrorBarPointStyleConfig {
                            error_bar_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
                            ..Default::default()
                        },
                        DataErrorBarPointStyleConfig {
                            error_bar_color: Some(Color::new(0.0, 0.0, 1.0, 1.0)),
                            ..Default::default()
                        },
                    ]),
                    error_bar_style_index_column: Some("err_style".into()),
                    error_bar_style_overrides: None,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let green = img
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0] < 80 && p[1] > 150 && p[2] < 80 && p[3] > 100)
            .count();
        let blue = img
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0] < 80 && p[1] < 80 && p[2] > 150 && p[3] > 100)
            .count();

        assert!(green > 20, "mapped errorbar style produced no green ink");
        assert!(blue > 20, "mapped errorbar style produced no blue ink");
    }

    fn count_rgba_ink(rgba: &[u8], pred: impl Fn([u8; 4]) -> bool) -> usize {
        rgba.chunks_exact(4)
            .filter(|p| pred([p[0], p[1], p[2], p[3]]))
            .count()
    }

    fn is_saturated_red(p: [u8; 4]) -> bool {
        p[0] > 150 && p[1] < 80 && p[2] < 80 && p[3] > 100
    }

    fn is_saturated_green(p: [u8; 4]) -> bool {
        p[0] < 80 && p[1] > 150 && p[2] < 80 && p[3] > 100
    }

    fn is_saturated_blue(p: [u8; 4]) -> bool {
        p[0] < 80 && p[1] < 80 && p[2] > 150 && p[3] > 100
    }

    struct RecordedPreparedRgba {
        command_buffer: wgpu::CommandBuffer,
        readback: wgpu::Buffer,
        unpadded: u32,
        padded: u32,
        height: u32,
    }

    fn record_prepared_rgba(
        renderer: &Renderer,
        prepared: &PreparedFrame,
        w: u32,
        h: u32,
    ) -> RecordedPreparedRgba {
        let device = renderer.device.as_ref();
        let target_desc = wgpu::TextureDescriptor {
            label: Some("mapped style regression target"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let tex = device.create_texture(&target_desc);
        let tv = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let unpadded = w * 4;
        let padded = unpadded.div_ceil(256) * 256;
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            renderer
                .paint_prepared(&mut pass, (w, h), prepared)
                .expect("paint_prepared after mapped-style prepare");
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            target_desc.size,
        );
        RecordedPreparedRgba {
            command_buffer: enc.finish(),
            readback,
            unpadded,
            padded,
            height: h,
        }
    }

    fn submit_recorded_rgba(renderer: &Renderer, recorded: RecordedPreparedRgba) -> Vec<u8> {
        renderer
            .queue
            .submit(std::iter::once(recorded.command_buffer));
        let slice = recorded.readback.slice(..);
        let (mapped_tx, mapped_rx) = std::sync::mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = mapped_tx.send(result);
        });
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("offscreen GPU work completes before readback");
        // On the shared test device the callback can run on another test's
        // polling thread, after this poll returns.
        mapped_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("offscreen mapping callback runs after GPU completion")
            .expect("offscreen readback mapping succeeds");
        let mapped = slice
            .get_mapped_range()
            .expect("offscreen readback is mapped");
        if recorded.padded == recorded.unpadded {
            mapped.to_vec()
        } else {
            let mut out = Vec::with_capacity((recorded.unpadded * recorded.height) as usize);
            for row in 0..recorded.height {
                let start = (row * recorded.padded) as usize;
                out.extend_from_slice(&mapped[start..start + recorded.unpadded as usize]);
            }
            out
        }
    }

    fn paint_prepared_rgba(
        renderer: &Renderer,
        prepared: &PreparedFrame,
        w: u32,
        h: u32,
    ) -> Vec<u8> {
        submit_recorded_rgba(renderer, record_prepared_rgba(renderer, prepared, w, h))
    }

    struct MappedStyleRender {
        styles: Vec<ChartStyle>,
        screen: Vec<u8>,
        export: Vec<u8>,
        scatter_mapped: bool,
        errorbar_mapped: bool,
        pick_ring_mapped: bool,
    }

    /// On-screen path = `prepare` + `paint_prepared` (same as `WindowedRenderer::draw`).
    /// PNG/export path = `export_panel_rgba` (own pipeline cache, same ensure).
    fn render_mapped_style_case(
        renderer: &mut Renderer,
        chart: &Chart,
        series_cfgs: &[SeriesConfig],
    ) -> MappedStyleRender {
        let styles: Vec<ChartStyle> = series_cfgs
            .iter()
            .map(|cfg| renderer.create_style_for_series(cfg).unwrap())
            .collect();
        let w = chart.config().chart_area.0.width;
        let h = chart.config().chart_area.0.height;
        let (screen, scatter_mapped, errorbar_mapped, pick_ring_mapped) = {
            let view = renderer
                .create_chart_view(chart, chart.config().chart_area.0)
                .unwrap();
            let series: Vec<Series<'_>> = series_cfgs
                .iter()
                .zip(styles.iter())
                .map(|(cfg, style)| Series { config: cfg, style })
                .collect();
            let items = [ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &series,
            }];
            let prepared = renderer.prepare(&items).unwrap();
            let pipeline_state = (
                renderer.pipelines.scatter_mapped.is_some(),
                renderer.pipelines.errorbar_mapped.is_some(),
                renderer.pipelines.pick_ring_mapped.is_some(),
            );
            (
                paint_prepared_rgba(renderer, &prepared, w, h),
                pipeline_state.0,
                pipeline_state.1,
                pipeline_state.2,
            )
        };
        let export = renderer
            .export_panel_rgba(chart, series_cfgs, 1.0)
            .expect("export must not panic when a style map exists without an index column");
        MappedStyleRender {
            styles,
            screen,
            export: export.rgba,
            scatter_mapped,
            errorbar_mapped,
            pick_ring_mapped,
        }
    }

    fn assert_ink_on_screen_and_export(
        rendered: &MappedStyleRender,
        label: &str,
        pred: fn([u8; 4]) -> bool,
        min: usize,
    ) {
        let screen = count_rgba_ink(&rendered.screen, pred);
        let export = count_rgba_ink(&rendered.export, pred);
        assert!(
            screen >= min,
            "{label} missing on-screen ({screen} px, need {min})"
        );
        assert!(
            export >= min,
            "{label} missing on PNG/export ({export} px, need {min})"
        );
    }

    fn override_only_scatter_series(id: &str, x: &str, y: &str) -> SeriesConfig {
        let mut scatter = test_scatter_style();
        scatter.point_color = Color::new(0.0, 0.0, 0.0, 1.0);
        scatter.point_size = 6.0;
        scatter.point_style_table = None;
        scatter.point_style_index_column = None;
        scatter.point_style_overrides = Some(vec![DataScatterPointStyleOverride {
            index: 0,
            style: DataScatterPointStyleConfig {
                point_color: Some(Color::new(1.0, 0.0, 0.0, 1.0)),
                point_size: Some(16.0),
                ..Default::default()
            },
        }]);
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Scatter { scatter },
        }
    }

    /// `point_style_overrides` without `point_style_index_column` still builds
    /// a style map (`has_index = false`). Prepare must compile the mapped
    /// scatter pipeline that draw/export both require.
    #[test]
    fn override_only_scatter_paints_on_screen_and_export() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();

        let chart = basic_errorbar_chart();
        let series = [override_only_scatter_series("ov-scatter", "x", "y")];
        let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

        let map = rendered.styles[0]
            .scatter_map
            .as_ref()
            .expect("overrides-only scatter must build a style map");
        assert!(
            !map.has_index,
            "overrides-only scatter must not set has_index"
        );
        assert!(
            rendered.styles[0].errorbar_map.is_none(),
            "scatter-only series must not build an errorbar map"
        );
        assert!(
            rendered.scatter_mapped,
            "prepare must compile mapped scatter when scatter_map is Some"
        );
        assert!(!rendered.errorbar_mapped);
        assert!(!rendered.pick_ring_mapped);

        assert_ink_on_screen_and_export(
            &rendered,
            "override-only red scatter",
            is_saturated_red,
            20,
        );
    }

    /// Same override-only map plus a selected-point ring. Draw selects the
    /// mapped pick-ring pipeline whenever `scatter_map` is Some.
    #[test]
    fn override_only_scatter_picked_ring_paints_on_screen_and_export() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();

        let mut chart = basic_errorbar_chart();
        chart.config_mut().picked_points = Some(crate::config::PickedPointsConfig {
            visible: true,
            refs: vec![crate::config::PickedPointRef {
                source_id: None,
                series_id: "ov-scatter".into(),
                point_index: 0,
            }],
            ring_color: Color::new(0.0, 1.0, 0.0, 1.0),
            ring_width_px: 3.0,
            radius_extra_px: 8.0,
        });
        let series = [override_only_scatter_series("ov-scatter", "x", "y")];
        let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

        assert!(
            rendered.styles[0]
                .scatter_map
                .as_ref()
                .is_some_and(|map| !map.has_index)
        );
        assert!(
            rendered.scatter_mapped,
            "picked override-only scatter still needs mapped scatter"
        );
        assert!(
            rendered.pick_ring_mapped,
            "picked override-only scatter must compile mapped pick ring"
        );

        assert_ink_on_screen_and_export(
            &rendered,
            "override-only red scatter",
            is_saturated_red,
            20,
        );
        assert_ink_on_screen_and_export(&rendered, "mapped pick ring", is_saturated_green, 10);
    }

    // ── Continuous point value maps (#24) ───────────────────────────────────

    fn is_saturated_purple(p: [u8; 4]) -> bool {
        p[0] > 90 && p[1] < 60 && p[2] > 90 && p[3] > 100
    }

    fn two_stop_colorbar(scale: AxisScale, min: f64, max: f64) -> crate::config::ColorBarOptions {
        let mut bar = crate::default::default_colorbar_options();
        // Hidden, so the plot keeps the basic chart's geometry; the options
        // still own the z range and the ramp.
        bar.visible = false;
        bar.colormap = crate::colormap::ColorMap::Custom {
            stops: vec![
                Color::new(1.0, 0.0, 0.0, 1.0),
                Color::new(0.0, 0.0, 1.0, 1.0),
            ],
        };
        bar.nan_color = Color::new(0.0, 1.0, 0.0, 1.0);
        bar.axis.scale = scale;
        bar.axis.min = min;
        bar.axis.max = max;
        bar
    }

    fn value_mapped_scatter_series(
        id: &str,
        color_map: Option<&str>,
        size_map: Option<(&str, SizeScale)>,
    ) -> SeriesConfig {
        let mut scatter = test_scatter_style();
        scatter.point_color = Color::new(1.0, 0.0, 0.0, 1.0);
        scatter.point_size = 8.0;
        scatter.point_color_map = color_map.map(|column| crate::data_config::PointColorMap {
            column: column.into(),
        });
        scatter.point_size_map = size_map.map(|(column, scale)| crate::data_config::PointSizeMap {
            column: column.into(),
            domain_min: 0.0,
            domain_max: 10.0,
            min_px: 2.0,
            max_px: 12.0,
            scale,
        });
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Scatter { scatter },
        }
    }

    /// Colour comes from the colourbar ramp at the point's value: the ends
    /// take the end stops, the middle the interpolated colour, and a
    /// non-finite value the colourbar's `nan_color`. On screen and on export.
    #[test]
    fn color_mapped_scatter_paints_the_colorbar_ramp_and_nan_color() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("x", &col_f64(vec![0.5, 1.5, 2.5, 3.5]))
            .unwrap();
        renderer
            .add_column("y", &col_f64(vec![2.5, 2.5, 2.5, 2.5]))
            .unwrap();
        renderer
            .add_column("c", &col_f64(vec![0.0, 10.0, f64::NAN, 5.0]))
            .unwrap();

        let mut chart = basic_errorbar_chart();
        chart.config_mut().colorbar = Some(two_stop_colorbar(AxisScale::Linear, 0.0, 10.0));
        let series = [value_mapped_scatter_series("cmap", Some("c"), None)];
        let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

        assert!(
            rendered.styles[0].scatter_map.is_some(),
            "a colour map must select the mapped scatter path"
        );
        assert!(rendered.scatter_mapped);
        assert_ink_on_screen_and_export(&rendered, "z = min (red stop)", is_saturated_red, 20);
        assert_ink_on_screen_and_export(&rendered, "z = max (blue stop)", is_saturated_blue, 20);
        assert_ink_on_screen_and_export(&rendered, "z = NaN (nan_color)", is_saturated_green, 20);
        assert_ink_on_screen_and_export(&rendered, "z = mid (ramp)", is_saturated_purple, 20);
    }

    /// A logarithmic colourbar cannot place a non-positive value: it draws in
    /// `nan_color`, as a heatmap cell would, while 10 on a 1..100 bar is the
    /// ramp's middle.
    #[test]
    fn log_colorbar_maps_nonpositive_values_to_nan_color() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("x", &col_f64(vec![0.7, 2.0, 3.3]))
            .unwrap();
        renderer
            .add_column("y", &col_f64(vec![2.5, 2.5, 2.5]))
            .unwrap();
        renderer
            .add_column("c", &col_f64(vec![0.0, -3.0, 10.0]))
            .unwrap();

        let mut chart = basic_errorbar_chart();
        chart.config_mut().colorbar = Some(two_stop_colorbar(AxisScale::Logarithmic, 1.0, 100.0));
        let series = [value_mapped_scatter_series("cmap-log", Some("c"), None)];
        let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

        assert_ink_on_screen_and_export(&rendered, "non-positive z", is_saturated_green, 40);
        assert_ink_on_screen_and_export(&rendered, "log mid", is_saturated_purple, 20);
        assert_eq!(count_rgba_ink(&rendered.screen, is_saturated_red), 0);
        assert_eq!(count_rgba_ink(&rendered.screen, is_saturated_blue), 0);
    }

    /// One point at `value` on its own renderer, so each case uploads fresh
    /// columns under the same ids.
    fn single_point_ink(value: f64, scale: SizeScale) -> Option<usize> {
        let mut renderer = state_test_renderer()?;
        renderer.add_column("x", &col_f64(vec![2.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5])).unwrap();
        renderer.add_column("s", &col_f64(vec![value])).unwrap();
        let chart = basic_errorbar_chart();
        let series = [value_mapped_scatter_series(
            "smap",
            None,
            Some(("s", scale)),
        )];
        let rendered = render_mapped_style_case(&mut renderer, &chart, &series);
        let screen = count_rgba_ink(&rendered.screen, is_saturated_red);
        let export = count_rgba_ink(&rendered.export, is_saturated_red);
        // Export multisamples, so the exact counts differ; visibility must not.
        assert_eq!(
            screen == 0,
            export == 0,
            "screen ({screen} px) and export ({export} px) disagree for value {value}"
        );
        Some(screen)
    }

    /// The radius follows the size map: the domain's ends map to `min_px` and
    /// `max_px`, area scaling grows faster than radius scaling in the middle,
    /// a value past the domain clamps, and a non-finite value hides the point.
    #[test]
    fn size_mapped_scatter_scales_markers_by_value_and_hides_non_finite_values() {
        let Some(smallest) = single_point_ink(0.0, SizeScale::Radius) else {
            return;
        };
        let largest = single_point_ink(10.0, SizeScale::Radius).unwrap();
        let clamped = single_point_ink(1.0e9, SizeScale::Radius).unwrap();
        let mid_radius = single_point_ink(5.0, SizeScale::Radius).unwrap();
        let mid_area = single_point_ink(5.0, SizeScale::Area).unwrap();
        let hidden = single_point_ink(f64::NAN, SizeScale::Radius).unwrap();

        // 2 px vs 12 px radii: a 36x area ratio before antialiasing.
        assert!(smallest > 0, "min_px marker missing ({smallest} px)");
        assert!(
            largest > smallest * 8,
            "max_px marker ({largest} px) is not much larger than min_px ({smallest} px)"
        );
        assert_eq!(
            clamped, largest,
            "a value past the domain must clamp to max_px"
        );
        // Radius scaling: 7 px. Area scaling: sqrt((4 + 144) / 2) ≈ 8.6 px.
        assert!(
            mid_area > mid_radius * 13 / 10,
            "area scaling ({mid_area} px) must outgrow radius scaling ({mid_radius} px)"
        );
        assert_eq!(hidden, 0, "a non-finite size value must hide the point");
    }

    /// Export scales the mapped pixel range with the DPI multiplier like
    /// `point_size`: a 2x export draws about four times the area.
    #[test]
    fn size_mapped_scatter_scales_with_the_export_multiplier() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![2.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5])).unwrap();
        renderer.add_column("s", &col_f64(vec![10.0])).unwrap();
        let chart = basic_errorbar_chart();
        let series = [value_mapped_scatter_series(
            "smap",
            None,
            Some(("s", SizeScale::Radius)),
        )];
        let one = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let two = renderer.export_panel_rgba(&chart, &series, 2.0).unwrap();
        let one_px = count_rgba_ink(&one.rgba, is_saturated_red);
        let two_px = count_rgba_ink(&two.rgba, is_saturated_red);
        assert!(one_px > 0);
        assert!(
            two_px > one_px * 3 && two_px < one_px * 5,
            "2x export drew {two_px} px against {one_px} px at 1x"
        );
        let scratch = renderer
            .value_map_cache
            .get("smap")
            .expect("value map scratch");
        assert_eq!(scratch.signature.params.size_px, [4.0, 24.0]);
    }

    /// Per channel, a sparse override beats the value map: an override colour
    /// paints over the ramp, and an override size makes a hidden point
    /// visible again.
    #[test]
    fn overrides_take_precedence_over_value_maps() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
        renderer
            .add_column("c", &col_f64(vec![10.0, 10.0]))
            .unwrap();
        renderer
            .add_column("s", &col_f64(vec![f64::NAN, f64::NAN]))
            .unwrap();

        let mut chart = basic_errorbar_chart();
        chart.config_mut().colorbar = Some(two_stop_colorbar(AxisScale::Linear, 0.0, 10.0));
        let mut series =
            value_mapped_scatter_series("ov", Some("c"), Some(("s", SizeScale::Radius)));
        let DataRenderType::Scatter { scatter } = &mut series.render_type else {
            unreachable!()
        };
        scatter.point_style_overrides = Some(vec![DataScatterPointStyleOverride {
            index: 0,
            style: DataScatterPointStyleConfig {
                point_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
                point_shape: None,
                point_size: Some(10.0),
            },
        }]);
        let rendered = render_mapped_style_case(&mut renderer, &chart, &[series]);

        // Point 0: overridden colour and size. Point 1: hidden by its size.
        assert_ink_on_screen_and_export(&rendered, "override colour", is_saturated_green, 40);
        assert_eq!(
            count_rgba_ink(&rendered.screen, is_saturated_blue),
            0,
            "the ramp colour must not paint under an override"
        );
    }

    /// The picked ring follows the mapped radius and is absent on a point the
    /// size map hides.
    #[test]
    fn picked_ring_follows_the_size_map() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
        renderer
            .add_column("s", &col_f64(vec![10.0, f64::NAN]))
            .unwrap();
        let series = [value_mapped_scatter_series(
            "ring",
            None,
            Some(("s", SizeScale::Radius)),
        )];
        let picked = |point_index: usize| crate::config::PickedPointsConfig {
            visible: true,
            refs: vec![crate::config::PickedPointRef {
                source_id: None,
                series_id: "ring".into(),
                point_index,
            }],
            ring_color: Color::new(0.0, 1.0, 0.0, 1.0),
            ring_width_px: 3.0,
            radius_extra_px: 4.0,
        };

        let mut chart = basic_errorbar_chart();
        chart.config_mut().picked_points = Some(picked(0));
        let visible = render_mapped_style_case(&mut renderer, &chart, &series);
        assert!(visible.pick_ring_mapped);
        assert_ink_on_screen_and_export(
            &visible,
            "ring on the mapped point",
            is_saturated_green,
            40,
        );

        chart.config_mut().picked_points = Some(picked(1));
        let hidden = render_mapped_style_case(&mut renderer, &chart, &series);
        assert_eq!(
            count_rgba_ink(&hidden.screen, is_saturated_green),
            0,
            "a hidden point draws no ring"
        );
    }

    /// Registration rejects a colour map without a colourbar, an invalid size
    /// domain and an unregistered value column.
    #[test]
    fn value_maps_are_validated_at_registration() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
        renderer.add_column("c", &col_f64(vec![1.0, 2.0])).unwrap();
        let config = basic_errorbar_chart().config().clone();

        let no_colorbar = vec![value_mapped_scatter_series("v", Some("c"), None)];
        assert!(matches!(
            renderer.register_chart(config.clone(), no_colorbar),
            Err(FiggyError::InvalidSeriesConfig { reason, .. }) if reason.contains("colorbar")
        ));

        let mut bad_domain = value_mapped_scatter_series("v", None, Some(("c", SizeScale::Area)));
        let DataRenderType::Scatter { scatter } = &mut bad_domain.render_type else {
            unreachable!()
        };
        scatter.point_size_map.as_mut().unwrap().domain_max = 0.0;
        assert!(matches!(
            renderer.register_chart(config.clone(), vec![bad_domain]),
            Err(FiggyError::InvalidSeriesConfig { reason, .. })
                if reason == crate::data_config::PointSizeMap::INVALID_DOMAIN_REASON
        ));

        let missing = vec![value_mapped_scatter_series(
            "v",
            None,
            Some(("nope", SizeScale::Area)),
        )];
        assert!(matches!(
            renderer.register_chart(config, missing),
            Err(FiggyError::UnknownColumn { id }) if id == "nope"
        ));
    }

    /// A value column shorter than the series is a prepare error, never a
    /// read past the column.
    #[test]
    fn value_map_columns_must_cover_every_point() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("x", &col_f64(vec![1.0, 2.0, 3.0]))
            .unwrap();
        renderer
            .add_column("y", &col_f64(vec![2.5, 2.5, 2.5]))
            .unwrap();
        renderer.add_column("s", &col_f64(vec![1.0])).unwrap();
        let chart = basic_errorbar_chart();
        let series = [value_mapped_scatter_series(
            "short",
            None,
            Some(("s", SizeScale::Area)),
        )];
        let style = renderer.create_style_for_series(&series[0]).unwrap();
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let drawn = [Series {
            config: &series[0],
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &drawn,
        }];
        assert!(matches!(
            renderer.prepare(&items),
            Err(FiggyError::InvalidSeriesConfig { reason, .. }) if reason.contains("size map")
        ));
    }

    /// Streaming is not extended to the value maps: a streamed series with one
    /// is rejected with an explicit reason, like a streamed line fill.
    #[test]
    fn streamed_series_reject_value_maps() {
        let config = basic_errorbar_chart().config().clone();
        let colour = value_mapped_scatter_series("stream", Some("c"), None);
        let size = value_mapped_scatter_series("stream", None, Some(("s", SizeScale::Area)));
        for series in [colour, size] {
            assert!(matches!(
                PrepareContext::validate_stream_series(&config, &series),
                Err(FiggyError::InvalidSeriesConfig { reason, .. })
                    if reason.contains("streamed sources are not supported")
            ));
        }
        let plain = test_scatter_style();
        let plain = SeriesConfig {
            series_id: "plain".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Scatter { scatter: plain },
        };
        assert!(PrepareContext::validate_stream_series(&config, &plain).is_ok());
    }

    /// The group-3 state is rebuilt only when its inputs change: two prepares
    /// of the same chart share one entry, and editing the colourbar's range
    /// (SSOT config) replaces it.
    #[test]
    fn value_map_scratch_is_reused_until_its_inputs_change() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
        renderer.add_column("c", &col_f64(vec![1.0, 2.0])).unwrap();
        let mut chart = basic_errorbar_chart();
        chart.config_mut().colorbar = Some(two_stop_colorbar(AxisScale::Linear, 0.0, 10.0));
        let series = [value_mapped_scatter_series("reuse", Some("c"), None)];

        let first = render_mapped_style_case(&mut renderer, &chart, &series);
        assert_eq!(renderer.value_map_cache.len(), 1);
        let before = renderer.value_map_cache["reuse"].signature.params;
        let second = render_mapped_style_case(&mut renderer, &chart, &series);
        assert_eq!(renderer.value_map_cache.len(), 1);
        assert_eq!(renderer.value_map_cache["reuse"].signature.params, before);
        assert_eq!(
            first.screen, second.screen,
            "an unchanged chart repaints identically"
        );

        chart.config_mut().colorbar.as_mut().unwrap().axis.max = 2.0;
        let third = render_mapped_style_case(&mut renderer, &chart, &series);
        let after = renderer.value_map_cache["reuse"].signature.params;
        assert_ne!(
            after.z_max, before.z_max,
            "a colourbar edit rebuilds the params"
        );
        assert_ne!(
            third.screen, first.screen,
            "the narrower range changes the mapped colours"
        );
    }

    // ── Vector fields (#25) ─────────────────────────────────────────────────

    fn vector_points_series(id: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Vector {
                components: crate::data_config::VectorComponents::Points {
                    u: "u".into(),
                    v: "v".into(),
                },
                style: crate::data_config::VectorStyle {
                    color: Color::new(1.0, 0.0, 0.0, 1.0),
                    width_px: 4.0,
                    scale: crate::data_config::VectorScale::Pixels { px_per_unit: 1.0 },
                    pivot: crate::data_config::VectorPivot::Tail,
                    head_length_px: 8.0,
                    head_width_px: 8.0,
                    min_length_px: 0.0,
                    color_map: None,
                },
            },
        }
    }

    /// Streaming is not extended to vector series: a streamed one is rejected
    /// with an explicit reason, like a streamed value map.
    #[test]
    fn streamed_series_reject_vectors() {
        let config = basic_errorbar_chart().config().clone();
        assert!(matches!(
            PrepareContext::validate_stream_series(&config, &vector_points_series("stream")),
            Err(FiggyError::InvalidSeriesConfig { reason, .. })
                if reason.contains("streamed sources are not supported")
        ));
    }

    /// Streaming is not extended to polar charts: a streamed series on one
    /// is rejected with an explicit reason.
    #[test]
    fn streamed_series_reject_polar_charts() {
        let mut config = basic_errorbar_chart().config().clone();
        config.polar = Some(crate::polar::PolarOptions::default());
        let series = SeriesConfig {
            series_id: "stream".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Scatter {
                scatter: DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: crate::data_config::ScatterShape::CircleFilled,
                    point_size: 4.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
            },
        };
        assert!(matches!(
            PrepareContext::validate_stream_series(&config, &series),
            Err(FiggyError::InvalidSeriesConfig { reason, .. })
                if reason.contains("polar") && reason.contains("streamed sources are not supported")
        ));
        config.polar = None;
        assert!(PrepareContext::validate_stream_series(&config, &series).is_ok());
    }

    /// The vector scratch (params, grid table, bind groups) is rebuilt only
    /// when its inputs change: two prepares of the same chart share one
    /// entry, a style edit replaces its params, and replacing a component
    /// column replaces its source epochs.
    #[test]
    fn vector_scratch_is_reused_until_its_inputs_change() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
        renderer
            .add_column("u", &col_f64(vec![30.0, 30.0]))
            .unwrap();
        renderer.add_column("v", &col_f64(vec![0.0, 0.0])).unwrap();
        let chart = basic_errorbar_chart();
        let mut series = [vector_points_series("reuse")];

        let first = render_mapped_style_case(&mut renderer, &chart, &series);
        assert!(
            count_rgba_ink(&first.screen, is_saturated_red) > 50,
            "the arrows draw on screen"
        );
        assert!(
            count_rgba_ink(&first.export, is_saturated_red) > 50,
            "the arrows draw on export"
        );
        assert_eq!(renderer.vector_cache.len(), 1);
        let before = renderer.vector_cache["reuse"].signature.params;
        let epochs = renderer.vector_cache["reuse"].signature.epochs.clone();
        let second = render_mapped_style_case(&mut renderer, &chart, &series);
        assert_eq!(renderer.vector_cache.len(), 1);
        assert_eq!(renderer.vector_cache["reuse"].signature.params, before);
        assert_eq!(
            first.screen, second.screen,
            "an unchanged chart repaints identically"
        );

        let DataRenderType::Vector { style, .. } = &mut series[0].render_type else {
            unreachable!()
        };
        style.width_px = 12.0;
        let third = render_mapped_style_case(&mut renderer, &chart, &series);
        let after = renderer.vector_cache["reuse"].signature.params;
        assert_ne!(
            after.width_px, before.width_px,
            "a style edit rebuilds the params"
        );
        assert!(
            count_rgba_ink(&third.screen, is_saturated_red)
                > count_rgba_ink(&first.screen, is_saturated_red),
            "the wider shaft draws more"
        );

        renderer.remove_column("u").unwrap();
        renderer
            .add_column("u", &col_f64(vec![-30.0, -30.0]))
            .unwrap();
        let fourth = render_mapped_style_case(&mut renderer, &chart, &series);
        assert_ne!(
            renderer.vector_cache["reuse"].signature.epochs, epochs,
            "a replaced component column rebuilds the sources"
        );
        assert_ne!(fourth.screen, third.screen, "the arrows now point left");
    }

    /// `error_bar_style_overrides` without an index column is the errorbar
    /// counterpart of a single-point edit.
    #[test]
    fn override_only_errorbar_paints_on_screen_and_export() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
        renderer.add_column("ey", &col_f64(vec![1.0, 1.0])).unwrap();

        let chart = basic_errorbar_chart();
        let mut scatter = test_scatter_style();
        scatter.point_size = 0.0;
        let series = [SeriesConfig {
            series_id: "ov-errorbar".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter,
                err_y: ErrorRef::Symmetric {
                    column: "ey".into(),
                },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: Color::BLACK,
                    error_bar_width: 2.0,
                    error_bar_cap_size: 7.0,
                    cap_width: 2.0,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: Some(vec![
                        DataErrorBarPointStyleOverride {
                            index: 0,
                            style: DataErrorBarPointStyleConfig {
                                error_bar_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
                                ..Default::default()
                            },
                        },
                        DataErrorBarPointStyleOverride {
                            index: 1,
                            style: DataErrorBarPointStyleConfig {
                                error_bar_color: Some(Color::new(0.0, 0.0, 1.0, 1.0)),
                                ..Default::default()
                            },
                        },
                    ]),
                },
            },
        }];
        let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

        let map = rendered.styles[0]
            .errorbar_map
            .as_ref()
            .expect("overrides-only errorbar must build a style map");
        assert!(!map.has_index);
        assert!(
            rendered.errorbar_mapped,
            "prepare must compile mapped errorbar when errorbar_map is Some"
        );
        assert!(
            !rendered.scatter_mapped,
            "hidden unmapped scatter must not compile mapped scatter"
        );

        assert_ink_on_screen_and_export(
            &rendered,
            "override-only green errorbar",
            is_saturated_green,
            20,
        );
        assert_ink_on_screen_and_export(
            &rendered,
            "override-only blue errorbar",
            is_saturated_blue,
            20,
        );
    }

    /// Existing table + index-column mapped path, scatter and errorbar, both
    /// on-screen paint and PNG/export.
    #[test]
    fn table_and_index_mapped_scatter_and_errorbar_paints_on_screen_and_export() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("sx", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("sy", &col_f64(vec![3.5, 3.5])).unwrap();
        renderer
            .add_column("s_style", &col_f64(vec![0.0, 1.0]))
            .unwrap();
        renderer.add_column("ex", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("ey", &col_f64(vec![1.2, 1.2])).unwrap();
        renderer.add_column("ee", &col_f64(vec![0.7, 0.7])).unwrap();
        renderer
            .add_column("e_style", &col_f64(vec![0.0, 1.0]))
            .unwrap();

        let chart = basic_errorbar_chart();
        let mut hidden_scatter = test_scatter_style();
        hidden_scatter.point_size = 0.0;
        let series = [
            SeriesConfig {
                series_id: "mapped-scatter".into(),
                source_id: None,
                label: None,
                x_column: "sx".into(),
                y_column: "sy".into(),
                render_type: DataRenderType::Scatter {
                    scatter: DataScatterStyleConfig {
                        point_color: Color::BLACK,
                        point_shape: crate::data_config::ScatterShape::CircleFilled,
                        point_size: 10.0,
                        point_style_table: Some(vec![
                            DataScatterPointStyleConfig {
                                point_color: Some(Color::new(1.0, 0.0, 0.0, 1.0)),
                                ..Default::default()
                            },
                            DataScatterPointStyleConfig {
                                point_color: Some(Color::new(0.0, 0.0, 1.0, 1.0)),
                                ..Default::default()
                            },
                        ]),
                        point_style_index_column: Some("s_style".into()),
                        point_style_overrides: None,
                        point_color_map: None,
                        point_size_map: None,
                    },
                },
            },
            SeriesConfig {
                series_id: "mapped-errorbar".into(),
                source_id: None,
                label: None,
                x_column: "ex".into(),
                y_column: "ey".into(),
                render_type: DataRenderType::ScatterErrorbarY {
                    scatter: hidden_scatter,
                    err_y: ErrorRef::Symmetric {
                        column: "ee".into(),
                    },
                    err_style: DataErrorBarStyleConfig {
                        error_bar_color: Color::BLACK,
                        error_bar_width: 2.0,
                        error_bar_cap_size: 7.0,
                        cap_width: 2.0,
                        error_bar_style_table: Some(vec![
                            DataErrorBarPointStyleConfig {
                                error_bar_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
                                ..Default::default()
                            },
                            DataErrorBarPointStyleConfig {
                                error_bar_color: Some(Color::new(0.0, 0.0, 1.0, 1.0)),
                                ..Default::default()
                            },
                        ]),
                        error_bar_style_index_column: Some("e_style".into()),
                        error_bar_style_overrides: None,
                    },
                },
            },
        ];
        let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

        let scatter_map = rendered.styles[0]
            .scatter_map
            .as_ref()
            .expect("table+index scatter must build a style map");
        assert!(scatter_map.has_index);
        let errorbar_map = rendered.styles[1]
            .errorbar_map
            .as_ref()
            .expect("table+index errorbar must build a style map");
        assert!(errorbar_map.has_index);
        assert!(rendered.scatter_mapped);
        assert!(rendered.errorbar_mapped);

        assert_ink_on_screen_and_export(&rendered, "table+index red scatter", is_saturated_red, 20);
        assert_ink_on_screen_and_export(
            &rendered,
            "table+index green errorbar",
            is_saturated_green,
            20,
        );
        assert_ink_on_screen_and_export(
            &rendered,
            "table+index blue mapped marks",
            is_saturated_blue,
            20,
        );
    }

    /// A style table with no index column and no overrides still produces a
    /// style map (`has_index = false`). Draw uses the mapped pipeline, so
    /// prepare must compile it even though the table cannot be indexed.
    #[test]
    fn style_table_without_index_still_ensures_mapped_scatter_pipeline() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();

        let chart = basic_errorbar_chart();
        let series = [SeriesConfig {
            series_id: "table-only".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Scatter {
                scatter: DataScatterStyleConfig {
                    point_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    point_shape: crate::data_config::ScatterShape::CircleFilled,
                    point_size: 10.0,
                    point_style_table: Some(vec![DataScatterPointStyleConfig {
                        point_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
                        ..Default::default()
                    }]),
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
            },
        }];
        let rendered = render_mapped_style_case(&mut renderer, &chart, &series);
        let map = rendered.styles[0]
            .scatter_map
            .as_ref()
            .expect("a non-empty style table still builds a map");
        assert!(!map.has_index);
        assert!(
            rendered.scatter_mapped,
            "table-only map must compile mapped scatter"
        );
        // Table rows are ignored without an index column; base red must paint.
        assert_ink_on_screen_and_export(
            &rendered,
            "table-only base red scatter",
            is_saturated_red,
            20,
        );
    }

    #[test]
    fn single_direction_errorbar_needs_no_internal_filler_column() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        r.add_column("x", &col_f64(vec![1.0, 2.0, 3.0])).unwrap();
        r.add_column("y", &col_f64(vec![1.0, 3.0, 2.0])).unwrap();
        r.add_column("ey", &col_f64(vec![0.3, 0.1, 0.2])).unwrap();

        let chart = basic_errorbar_chart();
        let series_configs = [SeriesConfig {
            series_id: "y".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: test_scatter_style(),
                err_y: ErrorRef::Symmetric {
                    column: "ey".into(),
                },
                err_style: test_errorbar_style(),
            },
        }];

        {
            let style = r.create_style_for_series(&series_configs[0]).unwrap();
            let drawable = [Series {
                config: &series_configs[0],
                style: &style,
            }];
            let view = r
                .create_chart_view(&chart, chart.config().chart_area.0)
                .unwrap();
            let items = [ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &drawable,
            }];
            r.prepare(&items)
                .expect("single-direction errorbar prepare");
        }
        assert!(r.pool().slot("__zero").is_none());

        let mut export_renderer = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        export_renderer
            .add_column("x", &col_f64(vec![1.0, 2.0, 3.0]))
            .unwrap();
        export_renderer
            .add_column("y", &col_f64(vec![1.0, 3.0, 2.0]))
            .unwrap();
        export_renderer
            .add_column("ey", &col_f64(vec![0.3, 0.1, 0.2]))
            .unwrap();
        let image = export_renderer
            .export_panel_rgba(&chart, &series_configs, 1.0)
            .expect("single-direction errorbar export");
        assert!(export_renderer.pool().slot("__zero").is_none());
        assert!(image.rgba.chunks_exact(4).any(|pixel| pixel[3] > 0));
    }

    #[test]
    fn differently_sized_single_direction_panels_keep_both_prepared_frames_valid() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer
            .add_column("short_x", &col_f64(vec![1.0, 2.0]))
            .unwrap();
        renderer
            .add_column("short_y", &col_f64(vec![2.0, 3.0]))
            .unwrap();
        renderer
            .add_column("short_ey", &col_f64(vec![0.2, 0.3]))
            .unwrap();
        renderer
            .add_column("long_x", &col_f64((0..9).map(f64::from).collect()))
            .unwrap();
        renderer
            .add_column(
                "long_y",
                &col_f64((0..9).map(|value| f64::from(value) * 0.5).collect()),
            )
            .unwrap();
        renderer
            .add_column("long_ex", &col_f64(vec![0.25; 9]))
            .unwrap();

        let chart_a = basic_errorbar_chart();
        let chart_b = basic_errorbar_chart();
        let config_a = SeriesConfig {
            series_id: "short-y-error".into(),
            source_id: None,
            label: None,
            x_column: "short_x".into(),
            y_column: "short_y".into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: test_scatter_style(),
                err_y: ErrorRef::Symmetric {
                    column: "short_ey".into(),
                },
                err_style: test_errorbar_style(),
            },
        };
        let config_b = SeriesConfig {
            series_id: "long-x-error".into(),
            source_id: None,
            label: None,
            x_column: "long_x".into(),
            y_column: "long_y".into(),
            render_type: DataRenderType::ScatterErrorbarX {
                scatter: test_scatter_style(),
                err_x: ErrorRef::Symmetric {
                    column: "long_ex".into(),
                },
                err_style: test_errorbar_style(),
            },
        };
        let style_a = renderer.create_style_for_series(&config_a).unwrap();
        let style_b = renderer.create_style_for_series(&config_b).unwrap();
        assert_eq!(
            errorbar_direction_flags(&config_a.render_type),
            ERRORBAR_HAS_Y
        );
        assert_eq!(
            errorbar_direction_flags(&config_b.render_type),
            ERRORBAR_HAS_X
        );
        let series_a = [Series {
            config: &config_a,
            style: &style_a,
        }];
        let series_b = [Series {
            config: &config_b,
            style: &style_b,
        }];
        let view_a = renderer
            .create_chart_view(&chart_a, chart_a.config().chart_area.0)
            .unwrap();
        let view_b = renderer
            .create_chart_view(&chart_b, chart_b.config().chart_area.0)
            .unwrap();
        let prepared_a = renderer
            .prepare(&[ChartDrawItem {
                view: &view_a,
                chart_config: chart_a.config(),
                series: &series_a,
            }])
            .unwrap();
        let prepared_b = renderer
            .prepare(&[ChartDrawItem {
                view: &view_b,
                chart_config: chart_b.config(),
                series: &series_b,
            }])
            .unwrap();

        renderer.validate_prepared(&prepared_a).unwrap();
        renderer.validate_prepared(&prepared_b).unwrap();
        assert!(renderer.pool().slot("__zero").is_none());
    }

    /// Two line series on distinct column pairs must BOTH render — this
    /// mirrors the web demo (sine on x/y + rc on t/v) and guards the
    /// second-series draw path end to end via headless export.
    #[test]
    fn two_line_series_both_render() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .unwrap();

        let n = 512;
        let period = 628.0_f64 / 100.0;
        let xs: Vec<f64> = (0..n).map(|i| i as f64 * period / n as f64).collect();
        let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
        let ts: Vec<f64> = (0..n).map(|i| i as f64 * 5.0 / n as f64).collect();
        let vs: Vec<f64> = ts.iter().map(|t| 1.0 - (-t).exp()).collect();
        r.add_column("x", &col_f64(xs)).unwrap();
        r.add_column("y", &col_f64(ys)).unwrap();
        r.add_column("t", &col_f64(ts)).unwrap();
        r.add_column("v", &col_f64(vs)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 6.5);
        chart.set_y_range(-1.2, 1.2);

        let line_cfg = |id: &str, x: &str, y: &str, color: Color| SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: color,
                    line_width: 2.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        };
        let series = [
            line_cfg("sine", "x", "y", Color::BLACK),
            line_cfg("rc", "t", "v", Color::new(1.0, 0.0, 0.0, 1.0)),
        ];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (mut red, mut black) = (0usize, 0usize);
        for px in img.rgba.chunks_exact(4) {
            if px[3] > 16 {
                if px[0] > 120 && px[1] < 90 && px[2] < 90 {
                    red += 1;
                }
                if px[0] < 90 && px[1] < 90 && px[2] < 90 {
                    black += 1;
                }
            }
        }
        assert!(black > 300, "sine (black) curve missing: {black} px");
        assert!(red > 300, "rc (red) curve missing: {red} px");
    }

    /// Milkyway arc-star density must be a property of the polyline's ARC, not of how
    /// densely the data samples it — the field-reported failure mode of the
    /// old per-segment quad budget, where a sparse polyline saturated at 24
    /// stars per segment and then ignored the density knob entirely. The
    /// same curve sampled at 8 vs 480 points must produce comparable star
    /// ink (the indirect pass budgets by total arc; the 8-point chord arc is
    /// only a few % shorter).
    #[test]
    fn milkyway_arc_star_density_is_sampling_invariant() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .unwrap();

        let curve = |n: usize| -> (Vec<f64>, Vec<f64>) {
            let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
            let ys: Vec<f64> = xs.iter().map(|x| 50.0 + 30.0 * (x * 4.2).sin()).collect();
            (xs, ys)
        };
        let (sx, sy) = curve(8);
        let (dx, dy) = curve(480);
        r.add_column("inv_sx", &col_f64(sx)).unwrap();
        r.add_column("inv_sy", &col_f64(sy)).unwrap();
        r.add_column("inv_dx", &col_f64(dx)).unwrap();
        r.add_column("inv_dy", &col_f64(dy)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 400,
        });
        // Stars only: ribbon/backdrop/glow off so the ink count measures the
        // star field alone. Density 60 needs ~540 stars over this arc — the
        // old per-segment budget would cap the 8-point series at 7·24 = 168.
        config.draw_style = crate::config::DrawStyle::Milkyway(crate::config::MilkywayOptions {
            star_density: 60.0,
            ribbon_intensity: 0.0,
            nebula: 0.0,
            dust: 0.0,
            glow: 0.0,
            ..Default::default()
        });
        let mut chart = Chart::new(config);
        chart.set_x_range(-0.05, 1.05);
        chart.set_y_range(0.0, 100.0);

        let line_cfg = |id: &str, x: &str, y: &str| SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                    line_width: 2.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        };
        let mut star_ink = |id: &str, x: &str, y: &str| -> usize {
            let series = [line_cfg(id, x, y)];
            let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
            img.rgba
                .chunks_exact(4)
                .filter(|p| p[3] > 0 && (p[0] > 40 || p[1] > 40 || p[2] > 40))
                .count()
        };
        // Same series id on purpose: identical salt isolates sampling as the
        // only variable.
        let sparse = star_ink("inv", "inv_sx", "inv_sy");
        let dense = star_ink("inv", "inv_dx", "inv_dy");

        assert!(sparse > 1_000, "sparse star field missing ({sparse} px)");
        assert!(dense > 1_000, "dense star field missing ({dense} px)");
        let ratio = sparse as f64 / dense as f64;
        assert!(
            (0.65..=1.55).contains(&ratio),
            "star ink must not depend on sampling density: sparse {sparse} px \
             vs dense {dense} px (ratio {ratio:.2})"
        );
    }

    /// The constellation backdrop cache must rebake only when its key
    /// (panel size, nebula, dust, seed) changes: range-only refreshes hit
    /// the cache, and a view already holding the baked generation skips the
    /// texture re-upload (`grid_space_gen` stamp).
    #[test]
    fn space_background_rebakes_only_on_key_change() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();

        let rect = Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 200,
        };
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(rect);
        config.draw_style =
            crate::config::DrawStyle::Milkyway(crate::config::MilkywayOptions::default());
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(0.0, 1.0);

        let mut view = r.create_chart_view(&chart, rect).unwrap();
        assert_eq!(view.grid_space_gen, None, "fresh view starts unstamped");

        r.refresh_axis(&mut view, &chart, rect).unwrap();
        assert_eq!(r.space_bg.as_ref().map(|s| s.bake_gen), Some(1));
        assert_eq!(view.grid_space_gen, Some(1));

        // Range-only change (pan/zoom): cache hit — no rebake, no restamp.
        chart.set_x_range(0.0, 2.0);
        r.refresh_axis(&mut view, &chart, rect).unwrap();
        assert_eq!(
            r.space_bg.as_ref().map(|s| s.bake_gen),
            Some(1),
            "range-only refresh must not rebake the backdrop"
        );
        assert_eq!(view.grid_space_gen, Some(1));

        // Backdrop parameter change: rebake + re-upload.
        if let crate::config::DrawStyle::Milkyway(c) = &mut chart.config_mut().draw_style {
            c.seed = 7;
        }
        r.refresh_axis(&mut view, &chart, rect).unwrap();
        assert_eq!(
            r.space_bg.as_ref().map(|s| s.bake_gen),
            Some(2),
            "seed change must rebake the backdrop"
        );
        assert_eq!(view.grid_space_gen, Some(2));

        // Leaving the style un-stamps the view; the slot itself is retained.
        chart.config_mut().draw_style = crate::config::DrawStyle::Precise;
        r.refresh_axis(&mut view, &chart, rect).unwrap();
        assert_eq!(view.grid_space_gen, None);
        assert_eq!(r.space_bg.as_ref().map(|s| s.bake_gen), Some(2));
    }

    #[test]
    fn milkyway_export_zero_dust_zero_nebula_has_plain_background() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 96,
            height: 64,
        });
        config.chart_title.visible = false;
        config.legend.visible = false;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.label_style.visible = false;
            axis.title_option.visible = false;
            axis.tick = crate::config::TickVisibility::None;
            axis.line_visible = false;
        }
        config.draw_style = crate::config::DrawStyle::Milkyway(crate::config::MilkywayOptions {
            nebula: 0.0,
            dust: 0.0,
            glow: 0.0,
            ..Default::default()
        });
        let chart = Chart::new(config);

        for scale in [1.0, 2.0] {
            let img = r.export_panel_rgba(&chart, &[], scale).unwrap();
            let off_base = img
                .rgba
                .chunks_exact(4)
                .filter(|p| {
                    (p[0] as i16 - 11).abs() > 1
                        || (p[1] as i16 - 15).abs() > 1
                        || (p[2] as i16 - 23).abs() > 1
                        || p[3] != 255
                })
                .count();
            assert_eq!(
                off_base, 0,
                "dust=0 and nebula=0 must export a plain space background at {scale}x"
            );
        }
    }

    /// The GPU arc-length scan must equal a sequential CPU reference for
    /// every dispatch shape: single block (n ≤ 256), one sums level, two
    /// sums levels (n > 65 536), and the sequential multi-chunk path (forced
    /// via a narrowed chunk size) that makes n unlimited — both an
    /// exact-multiple boundary and a ragged tail. This is the correctness
    /// proof that lets the pool keep NO CPU copy of column data.
    #[test]
    fn gpu_arc_prefix_matches_cpu_reference() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 900,
            height: 600,
        });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(-1.2, 1.2);

        for (case, n, chunk) in [
            ("tiny", 3usize, None),
            ("one-level", 1000, None),
            ("two-level", 70_000, None),
            // Forced multi-chunk shapes: an exact-multiple boundary and a
            // ragged tail both exercise the sequential carry chain.
            ("chunked-even", 2_000, Some(1_000)),
            ("chunked-ragged", 2_500, Some(1_000)),
        ] {
            r.arc_chunk_override = chunk;
            let xs: Vec<f64> = (0..n).map(|i| i as f64 / n as f64).collect();
            let ys: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin()).collect();
            let xid = format!("ax_{case}");
            let yid = format!("ay_{case}");
            r.add_column(&xid, &col_f64(xs.clone())).unwrap();
            r.add_column(&yid, &col_f64(ys.clone())).unwrap();

            let t = data_render::scatter_transform_from_config(chart.config());
            let (arc_buf, len_bytes) = r
                .ensure_arc_prefix(
                    &format!("s_{case}"),
                    &xid,
                    &yid,
                    &t,
                    None,
                    LineInterpolation::Linear,
                )
                .expect("arc prefix")
                .prefix;

            // Read the GPU result back.
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: len_bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(&arc_buf, 0, &readback, 0, len_bytes);
            queue.submit(std::iter::once(enc.finish()));
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            // Bounded so a stalled submission fails the test instead of
            // spinning forever (see `data_render::shared_device`).
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            });
            let gpu: Vec<f32> = bytemuck::cast_slice(
                &slice
                    .get_mapped_range()
                    .expect("arc prefix readback is mapped"),
            )
            .to_vec();

            // Sequential CPU reference mirroring the shader math.
            let px = |x: f64, y: f64| -> (f32, f32) {
                let axis_ndc = |value: f64, axis: usize| {
                    let (hi, lo) = crate::data::split_f64_to_f32_pair(value);
                    let numerator = (hi - t.data_min[axis]) + (lo - t.data_min_lo[axis]);
                    let range = (t.data_max[axis] - t.data_min[axis])
                        + (t.data_max_lo[axis] - t.data_min_lo[axis]);
                    let data_t = numerator / range;
                    let panel_t =
                        t.data_to_panel_offset[axis] + data_t * t.data_to_panel_scale[axis];
                    panel_t * 2.0 - 1.0
                };
                let nx = axis_ndc(x, 0);
                let ny = axis_ndc(y, 1);
                (nx / t.pixel_to_ndc[0], ny / t.pixel_to_ndc[1])
            };
            let mut acc = 0.0f32;
            let mut reference = vec![0.0f32];
            for i in 1..n {
                let a = px(xs[i - 1], ys[i - 1]);
                let b = px(xs[i], ys[i]);
                acc += ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
                reference.push(acc);
            }

            assert_eq!(gpu.len(), n, "{case}: length");
            let total = reference.last().copied().unwrap_or(0.0).max(1.0);
            for (i, (g, c)) in gpu.iter().zip(reference.iter()).enumerate() {
                assert!(
                    (g - c).abs() <= total * 2e-4 + 0.05,
                    "{case}: arc[{i}] gpu={g} cpu={c} (total {total})"
                );
            }
        }
    }

    /// A stepped line's arc prefix sums its axis-aligned pieces in pixel
    /// space — the same corners the vertex shader draws and the picker walks
    /// — so the dash phase advances along the stepped path. Each mode is a
    /// distinct cache key: the prefix of one mode is never reused for
    /// another, and the linear prefix stays what it was.
    #[test]
    fn gpu_arc_prefix_follows_the_stepped_path_per_interpolation_mode() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .unwrap();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 900,
            height: 600,
        });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(-1.2, 1.2);
        let t = data_render::scatter_transform_from_config(chart.config());

        // Uneven x spacing and a NaN gap: the gap's segments contribute 0 to
        // every mode, and the gap stays exactly where the line breaks.
        let n = 300usize;
        let xs: Vec<f64> = (0..n).map(|i| (i as f64 / n as f64).powf(1.3)).collect();
        let ys: Vec<f64> = (0..n)
            .map(|i| {
                if i == 150 {
                    f64::NAN
                } else {
                    (i as f64 * 0.37).sin()
                }
            })
            .collect();
        r.add_column("step_ax", &col_f64(xs.clone())).unwrap();
        r.add_column("step_ay", &col_f64(ys.clone())).unwrap();

        let read = |device: &wgpu::Device,
                    queue: &wgpu::Queue,
                    arc_buf: &wgpu::Buffer,
                    len_bytes: u64|
         -> Vec<f32> {
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: len_bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(arc_buf, 0, &readback, 0, len_bytes);
            queue.submit(std::iter::once(enc.finish()));
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            });
            bytemuck::cast_slice(&slice.get_mapped_range().expect("arc readback mapped")).to_vec()
        };
        // Pixel position of a (hi, lo) pair point, mirroring the shader.
        let px = |x: (f32, f32), y: (f32, f32)| -> (f32, f32) {
            let axis_ndc = |(hi, lo): (f32, f32), axis: usize| {
                let numerator = (hi - t.data_min[axis]) + (lo - t.data_min_lo[axis]);
                let range = (t.data_max[axis] - t.data_min[axis])
                    + (t.data_max_lo[axis] - t.data_min_lo[axis]);
                let data_t = numerator / range;
                let panel_t = t.data_to_panel_offset[axis] + data_t * t.data_to_panel_scale[axis];
                panel_t * 2.0 - 1.0
            };
            (
                axis_ndc(x, 0) / t.pixel_to_ndc[0],
                axis_ndc(y, 1) / t.pixel_to_ndc[1],
            )
        };
        let pair = crate::data::split_f64_to_f32_pair;
        let mut prefixes = Vec::new();
        for mode in [
            LineInterpolation::Linear,
            LineInterpolation::StepPre,
            LineInterpolation::StepPost,
            LineInterpolation::StepMid,
        ] {
            let (arc_buf, len_bytes) = r
                .ensure_arc_prefix("stepped", "step_ax", "step_ay", &t, None, mode)
                .expect("arc prefix")
                .prefix;
            let gpu = read(&device, &queue, &arc_buf, len_bytes);
            assert_eq!(gpu.len(), n, "{mode:?}: length");

            // Sequential CPU reference over the stepped pieces.
            let mut acc = 0.0f32;
            let mut reference = vec![0.0f32];
            for i in 1..n {
                if !ys[i - 1].is_finite() || !ys[i].is_finite() {
                    reference.push(acc);
                    continue;
                }
                let (xa, ya, xb, yb) = (pair(xs[i - 1]), pair(ys[i - 1]), pair(xs[i]), pair(ys[i]));
                let xm = ((xa.0 + xb.0) * 0.5, (xa.1 + xb.1) * 0.5);
                let pieces: Vec<((f32, f32), (f32, f32), (f32, f32), (f32, f32))> = match mode {
                    LineInterpolation::Linear => vec![(xa, ya, xb, yb)],
                    LineInterpolation::StepPre => vec![(xa, ya, xa, yb), (xa, yb, xb, yb)],
                    LineInterpolation::StepPost => vec![(xa, ya, xb, ya), (xb, ya, xb, yb)],
                    LineInterpolation::StepMid => {
                        vec![(xa, ya, xm, ya), (xm, ya, xm, yb), (xm, yb, xb, yb)]
                    }
                };
                for (x0, y0, x1, y1) in pieces {
                    let a = px(x0, y0);
                    let b = px(x1, y1);
                    acc += ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
                }
                reference.push(acc);
            }
            let total = reference.last().copied().unwrap_or(0.0).max(1.0);
            for (i, (g, c)) in gpu.iter().zip(reference.iter()).enumerate() {
                assert!(
                    (g - c).abs() <= total * 2e-4 + 0.05,
                    "{mode:?}: arc[{i}] gpu={g} cpu={c} (total {total})"
                );
            }
            prefixes.push((mode, gpu));
        }
        // A stepped path is longer than the chord, and every mode has its own
        // cached result for the same columns and transform.
        let linear_total = prefixes[0].1[n - 1];
        for (mode, gpu) in &prefixes[1..] {
            assert!(
                gpu[n - 1] > linear_total * 1.05,
                "{mode:?}: stepped total {} should exceed the linear {linear_total}",
                gpu[n - 1]
            );
        }
        assert_eq!(
            r.arc_cache["stepped"].len(),
            4,
            "one cached scan per interpolation mode"
        );
    }

    #[test]
    fn arc_prefix_cache_never_exceeds_limit_on_new_series_churn() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        r.add_column("arc_cache_x", &col_f64(vec![0.0, 0.5, 1.0]))
            .unwrap();
        r.add_column("arc_cache_y", &col_f64(vec![0.0, 1.0, 0.25]))
            .unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(0.0, 1.0);

        let t = data_render::scatter_transform_from_config(chart.config());
        for i in 0..=ARC_PREFIX_CACHE_LIMIT {
            let series_id = format!("arc-cache-series-{i}");
            assert!(
                r.ensure_arc_prefix(
                    &series_id,
                    "arc_cache_x",
                    "arc_cache_y",
                    &t,
                    None,
                    LineInterpolation::Linear,
                )
                .is_some()
            );
            assert!(
                r.arc_cache.len() <= ARC_PREFIX_CACHE_LIMIT,
                "arc cache exceeded limit after inserting {series_id}: {}",
                r.arc_cache.len()
            );
        }
    }

    /// Arc results are immutable by their exact compute input. Equal inputs
    /// share one result without another traversal; a geometry/data generation
    /// change gets a different buffer even after every prepared token drops.
    #[test]
    fn arc_results_reuse_only_exact_compute_keys() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        r.add_column("cache_x", &col_f64(vec![0.0, 0.5, 1.0]))
            .unwrap();
        r.add_column("cache_y", &col_f64(vec![0.0, 1.0, 0.25]))
            .unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(0.0, 1.0);

        let series_cfg = SeriesConfig {
            series_id: "arc-cache".into(),
            source_id: None,
            label: None,
            x_column: "cache_x".into(),
            y_column: "cache_y".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Dash,
                    line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                    line_width: 2.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        };
        let style = r.create_style_for_series(&series_cfg).unwrap();
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let view_a = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let view_b = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let items_a = [ChartDrawItem {
            view: &view_a,
            chart_config: chart.config(),
            series: &series,
        }];
        let items_b = [ChartDrawItem {
            view: &view_b,
            chart_config: chart.config(),
            series: &series,
        }];

        let arc_of = |p: &PreparedFrame| -> Arc<wgpu::Buffer> {
            Arc::clone(
                &p.items[0].series[0]
                    .line
                    .as_ref()
                    .expect("dashed series has a line layer")
                    .arc
                    .as_ref()
                    .expect("dashed series has an arc prefix")
                    .0,
            )
        };

        let p1 = r.prepare(&items_a).unwrap();
        let a1 = arc_of(&p1);
        let creations = r.gpu_memory_usage().creations_of(GpuResourceKind::ArcScan);
        let p2 = r.prepare(&items_b).unwrap();
        let a2 = arc_of(&p2);
        assert!(
            Arc::ptr_eq(&a1, &a2),
            "an identical compute key should reuse its immutable arc result"
        );
        assert_eq!(
            r.gpu_memory_usage().creations_of(GpuResourceKind::ArcScan),
            creations,
            "an identical key must not retraverse or allocate"
        );
        drop((p1, a1, p2, a2));

        let base_transform = data_render::scatter_transform_from_config(chart.config());
        let mut changed_transform = base_transform;
        changed_transform.pixel_to_ndc[0] *= 0.5;
        let changed = r
            .ensure_arc_prefix(
                "arc-cache",
                "cache_x",
                "cache_y",
                &changed_transform,
                None,
                LineInterpolation::Linear,
            )
            .expect("changed geometry arc result");
        let changed_arc = Arc::clone(&changed.prefix.0);
        assert!(
            !Arc::ptr_eq(&arc_of(&r.prepare(&items_a).unwrap()), &changed_arc),
            "a changed geometry transform must not overwrite the old result"
        );
        drop(changed);

        let changed_again = r
            .ensure_arc_prefix(
                "arc-cache",
                "cache_x",
                "cache_y",
                &changed_transform,
                None,
                LineInterpolation::Linear,
            )
            .expect("cached changed geometry arc result");
        assert!(
            Arc::ptr_eq(&changed_arc, &changed_again.prefix.0),
            "an exact changed key should reuse its immutable result"
        );

        let mut style_only = changed_transform;
        style_only.style_params[0][0] = 123.0;
        let style_only_result = r
            .ensure_arc_prefix(
                "arc-cache",
                "cache_x",
                "cache_y",
                &style_only,
                None,
                LineInterpolation::Linear,
            )
            .expect("style-only arc result");
        assert!(
            Arc::ptr_eq(&changed_arc, &style_only_result.prefix.0),
            "style-only changes must not retraverse geometry columns"
        );

        r.upsert_column("cache_y", &col_f64(vec![0.0, 0.8, 0.25]))
            .unwrap();
        let changed_data = r
            .ensure_arc_prefix(
                "arc-cache",
                "cache_x",
                "cache_y",
                &base_transform,
                None,
                LineInterpolation::Linear,
            )
            .expect("changed data generation arc result");
        assert!(
            !Arc::ptr_eq(&changed_arc, &changed_data.prefix.0),
            "a new source generation must not reuse an old prefix"
        );
        assert_eq!(
            r.arc_cache.get("arc-cache").map(Vec::len),
            Some(3),
            "the cache should hold base, changed-transform, and changed-data results"
        );
    }

    #[test]
    fn contour_projection_matches_fixed_four_step_gpu_oracle() {
        for case in [
            "linear",
            "pan",
            "inverted",
            "log",
            "large-offset",
            "edges",
            "transposed",
            "nan",
            "infinity",
            "flat",
        ] {
            let (x, y, x_range) = match case {
                "pan" => (vec![0.0, 0.5, 1.0], vec![0.0, 0.5, 1.0], (-0.2, 1.2)),
                "inverted" => (vec![0.0, 0.5, 1.0], vec![0.0, 0.5, 1.0], (1.0, 0.0)),
                "log" => (vec![1.0, 10.0, 100.0], vec![0.0, 0.5, 1.0], (1.0, 100.0)),
                "large-offset" => (
                    vec![1.0e9, 1.0e9 + 0.5, 1.0e9 + 1.0],
                    vec![0.0, 0.5, 1.0],
                    (1.0e9, 1.0e9 + 1.0),
                ),
                "edges" => (
                    vec![0.0, 0.3, 0.7, 1.0],
                    vec![0.0, 0.3, 0.7, 1.0],
                    (0.0, 1.0),
                ),
                _ => (vec![0.0, 0.5, 1.0], vec![0.0, 0.5, 1.0], (0.0, 1.0)),
            };
            // Non-affine fields require repeated Newton corrections. Invalid
            // samples and zero gradients must retain their rejection behavior.
            let mut z = [
                vec![0.0, 0.0, 0.0],
                vec![0.0, 1.0, 2.0],
                vec![0.0, 2.0, 4.0],
            ];
            match case {
                "transposed" => z[1][2] = 3.0,
                "nan" => z[1][1] = f64::NAN,
                "infinity" => z[1][1] = f64::INFINITY,
                "flat" => z = [vec![2.0; 3], vec![2.0; 3], vec![2.0; 3]],
                _ => {}
            }
            let mut images = Vec::new();
            for legacy in [false, true] {
                let mut renderer = state_test_renderer().expect("required GPU renderer");
                // Scalar upload intentionally rounds to f32. This fixture
                // must exercise the explicit hi/lo contract to retain 0.5
                // increments at an offset of 1e9.
                if case == "large-offset" {
                    renderer
                        .add_hilo_column("clx", &col_f64(x.clone()))
                        .unwrap();
                } else {
                    renderer.add_column("clx", &col_f64(x.clone())).unwrap();
                }
                renderer.add_column("cly", &col_f64(y.clone())).unwrap();
                for (id, values) in ["clz0", "clz1", "clz2"].into_iter().zip(z.iter()) {
                    renderer.add_column(id, &col_f64(values.clone())).unwrap();
                }
                if legacy {
                    renderer.contour_label_pipelines = Some(
                        crate::gpu_contour::ContourLabelPipelines::legacy_projection_for_test(
                            &renderer.device,
                            &renderer.field_bgl,
                        ),
                    );
                }
                let mut chart = state_test_contour_chart(
                    Rect {
                        x: 0,
                        y: 0,
                        width: 320,
                        height: 240,
                    },
                    x_range,
                );
                if case == "log" {
                    chart.config_mut().bottom_x.scale = AxisScale::Logarithmic;
                    chart.config_mut().bottom_x.major_spacing = 1.0;
                }
                let mut config = state_test_labelled_contour(45.0, Vec::new());
                let DataRenderType::Contour { matrix, .. } = &mut config.render_type else {
                    unreachable!()
                };
                if case == "edges" {
                    matrix.grid_layout = crate::data_config::GridLayout::Edges;
                }
                if case == "transposed" {
                    matrix.orientation = crate::data_config::MatrixOrientation::ColumnsAreY;
                }
                let style = renderer.create_style_for_series(&config).unwrap();
                let series = [Series {
                    config: &config,
                    style: &style,
                }];
                let view = renderer
                    .create_chart_view(&chart, chart.config().chart_area.0)
                    .unwrap();
                let items = [ChartDrawItem {
                    view: &view,
                    chart_config: chart.config(),
                    series: &series,
                }];
                let prepared = renderer.prepare(&items).unwrap();
                let pixels = paint_prepared_rgba(&renderer, &prepared, 320, 240);
                let has_labels = pixels
                    .chunks_exact(4)
                    .any(|p| p[0] > 200 && p[1] < 100 && p[2] > 200);
                if !matches!(case, "nan" | "infinity" | "flat") {
                    assert!(
                        has_labels,
                        "oracle fixture must produce automatic labels: {case}, legacy={legacy}"
                    );
                } else if case == "flat" {
                    assert!(
                        !has_labels,
                        "a zero gradient must not produce an automatic label"
                    );
                }
                images.push(pixels);
            }
            // The old fused loop and four dispatches round intermediate f32
            // values at different compiler boundaries. On Metal this can move
            // antialiasing across one 8-bit quantization threshold (45 pixels
            // in the edge fixture), without changing the projection algorithm.
            // Bound every channel, including alpha; a displaced/missing contour
            // or label still fails. Do not dump two 300 KB vectors on failure.
            assert_eq!(images[0].len(), images[1].len());
            let max_error = images[0]
                .iter()
                .zip(&images[1])
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(
                max_error <= 1,
                "four Newton corrections changed the image: {case}, max channel error {max_error}"
            );
        }
    }

    /// Paint one contour-only series with automatic labels on a 320×240 panel
    /// and return the RGBA frame. Labels carry an opaque magenta background, a
    /// colour nothing else in this chart uses.
    fn render_labelled_contour_for_test(
        columns: &[(&str, Vec<f64>)],
        x: &str,
        y: &str,
        z: &[&str],
        levels: Vec<f64>,
        spacing_px: f32,
    ) -> Vec<u8> {
        let mut renderer = state_test_renderer().expect("required GPU renderer");
        for (id, values) in columns {
            renderer.add_column(*id, &col_f64(values.clone())).unwrap();
        }
        let chart = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            },
            (0.0, 1.0),
        );
        let mut config = state_test_labelled_contour(spacing_px, Vec::new());
        let DataRenderType::Contour { matrix, contour } = &mut config.render_type else {
            unreachable!()
        };
        matrix.columns = z.iter().map(|id| (*id).into()).collect();
        contour.levels = levels;
        config.x_column = x.into();
        config.y_column = y.into();
        let style = renderer.create_style_for_series(&config).unwrap();
        let series = [Series {
            config: &config,
            style: &style,
        }];
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        let prepared = renderer.prepare(&items).unwrap();
        paint_prepared_rgba(&renderer, &prepared, 320, 240)
    }

    fn label_background_pixels(rgba: &[u8]) -> usize {
        rgba.chunks_exact(4)
            .filter(|p| p[0] > 200 && p[1] < 100 && p[2] > 200)
            .count()
    }

    // Issue #19. An automatic anchor is only valid where its level is actually
    // crossed inside the cell it lands in, which is the same test the stroke
    // uses. A strictly positive field with a shallow minimum just below the
    // projection tolerance used to accept a level-0 anchor at that minimum,
    // where no isoline exists.
    #[test]
    fn automatic_contour_labels_require_a_level_crossing_in_their_cell() {
        let grid: Vec<f64> = (0..24).map(|i| i as f64 / 23.0).collect();
        let field = |x: f64, y: f64| 4.0e-4 + 0.05 * ((x - 0.3).powi(2) + (y - 0.4).powi(2));
        let ids: Vec<String> = (0..grid.len()).map(|i| format!("nfz{i}")).collect();
        let mut columns = vec![("nfx", grid.clone()), ("nfy", grid.clone())];
        for (i, id) in ids.iter().enumerate() {
            columns.push((
                id.as_str(),
                grid.iter().map(|y| field(grid[i], *y)).collect(),
            ));
        }
        let z: Vec<&str> = ids.iter().map(String::as_str).collect();
        // A level the field does cross still gets labels in this setup, so the
        // assertion below cannot pass merely because nothing is drawn.
        let crossed =
            render_labelled_contour_for_test(&columns, "nfx", "nfy", &z, vec![0.01], 40.0);
        assert!(
            label_background_pixels(&crossed) > 0,
            "a crossed level must still be labelled"
        );
        let uncrossed =
            render_labelled_contour_for_test(&columns, "nfx", "nfy", &z, vec![0.0], 40.0);
        assert_eq!(
            label_background_pixels(&uncrossed),
            0,
            "a level the field never crosses must not be labelled"
        );
    }

    // Issue #20. A rotated label quad used to end exactly at the edge of its
    // baked background box, so the box edge was the rasterized quad edge and
    // stair-stepped. The quad now reaches into the transparent gutter and the
    // texture filter antialiases the box edge like the glyphs inside it.
    #[test]
    fn rotated_contour_label_background_edges_are_antialiased() {
        let columns = [
            ("clx", vec![0.0, 0.5, 1.0]),
            ("cly", vec![0.0, 0.5, 1.0]),
            ("clz0", vec![0.0, 1.0, 2.0]),
            ("clz1", vec![1.0, 2.0, 3.0]),
            ("clz2", vec![2.0, 3.0, 4.0]),
        ];
        // z = i + j: the level-2 isoline is the panel's diagonal, so the one
        // label is rotated away from both axes.
        let rgba = render_labelled_contour_for_test(
            &columns,
            "clx",
            "cly",
            &["clz0", "clz1", "clz2"],
            vec![2.0],
            400.0,
        );
        // The panel clears to transparent black, so the frame is premultiplied
        // magenta over nothing: an opaque box pixel is (255, 0, 255, 255) and a
        // partially covered edge pixel is (a, 0, a, a). Glyphs are drawn over the
        // opaque box and stay opaque, so they are never counted as edge.
        let solid = rgba
            .chunks_exact(4)
            .filter(|p| p[0] >= 250 && p[1] <= 2 && p[2] >= 250 && p[3] == 255)
            .count();
        let edge = rgba
            .chunks_exact(4)
            .filter(|p| {
                let premultiplied_magenta =
                    p[1] <= 2 && p[0].abs_diff(p[2]) <= 2 && p[0].abs_diff(p[3]) <= 2;
                premultiplied_magenta && (8..=247).contains(&p[3])
            })
            .count();
        assert!(solid > 100, "the label background must be drawn: {solid}");
        assert!(
            edge >= 40,
            "a rotated label background needs antialiased edges, found {edge} partial pixels"
        );
    }

    #[test]
    fn contour_label_placements_reuse_only_exact_dispatch_keys() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_contour_grid(&mut renderer);
        let chart_a = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            },
            (0.0, 1.0),
        );
        let chart_b = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            },
            (-0.5, 1.5),
        );
        let automatic_a = state_test_labelled_contour(70.0, Vec::new());
        let automatic_b = state_test_labelled_contour(260.0, Vec::new());
        let style_a = renderer.create_style_for_series(&automatic_a).unwrap();
        let style_b = renderer.create_style_for_series(&automatic_b).unwrap();
        let series_a = [Series {
            config: &automatic_a,
            style: &style_a,
        }];
        let series_b = [Series {
            config: &automatic_b,
            style: &style_b,
        }];
        let view_a = renderer
            .create_chart_view(&chart_a, chart_a.config().chart_area.0)
            .unwrap();
        let view_b = renderer
            .create_chart_view(&chart_b, chart_b.config().chart_area.0)
            .unwrap();
        let items_a = [ChartDrawItem {
            view: &view_a,
            chart_config: chart_a.config(),
            series: &series_a,
        }];
        let items_b = [ChartDrawItem {
            view: &view_b,
            chart_config: chart_b.config(),
            series: &series_b,
        }];
        let placement_of = |prepared: &PreparedFrame| {
            prepared.items[0].series[0]
                .contour
                .as_ref()
                .and_then(|contour| contour.label.as_ref())
                .expect("labelled contour snapshot")
                .snapshot
                .placement_identity()
        };

        let first = renderer.prepare(&items_a).unwrap();
        let first_id = placement_of(&first);
        let second = renderer.prepare(&items_b).unwrap();
        let second_id = placement_of(&second);
        assert_ne!(
            first_id, second_id,
            "different dispatch keys reused one automatic placement result"
        );
        assert!(renderer.validate_prepared(&first).is_ok());
        assert!(renderer.validate_prepared(&second).is_ok());
        assert_eq!(
            renderer.contour_cache["label-cache"]
                .label
                .as_ref()
                .unwrap()
                .automatic_result_count(),
            2
        );

        let creations = renderer
            .gpu_memory_usage()
            .creations_of(GpuResourceKind::ContourScratch);
        drop((first, second));
        let reused = renderer.prepare(&items_a).unwrap();
        let reused_id = placement_of(&reused);
        assert_eq!(
            reused_id, first_id,
            "the exact A input must reuse A's immutable result"
        );
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .creations_of(GpuResourceKind::ContourScratch),
            creations,
            "an exact automatic placement key should not allocate or dispatch again"
        );
        drop(reused);

        // Record A without submitting it, release the Rust token, then cross
        // the eight-result cache limit with distinct transforms. The delayed
        // command buffer must still read A's immutable anchors and indirect
        // count, while its evicted charge remains retired until submission.
        let baseline_a = {
            let prepared = renderer.prepare(&items_a).unwrap();
            paint_prepared_rgba(&renderer, &prepared, 320, 240)
        };
        let delayed_a = {
            let prepared = renderer.prepare(&items_a).unwrap();
            record_prepared_rgba(&renderer, &prepared, 320, 240)
        };
        let mut pixels_c = None;
        for key in 0..9 {
            let min = -1.0 - f64::from(key) * 0.125;
            let chart_c = state_test_contour_chart(
                Rect {
                    x: 0,
                    y: 0,
                    width: 320,
                    height: 240,
                },
                (min, 2.0 + f64::from(key) * 0.25),
            );
            let view_c = renderer
                .create_chart_view(&chart_c, chart_c.config().chart_area.0)
                .unwrap();
            let items_c = [ChartDrawItem {
                view: &view_c,
                chart_config: chart_c.config(),
                series: &series_a,
            }];
            let prepared = renderer.prepare(&items_c).unwrap();
            pixels_c = Some(paint_prepared_rgba(&renderer, &prepared, 320, 240));
        }
        assert_eq!(
            renderer.contour_cache["label-cache"]
                .label
                .as_ref()
                .unwrap()
                .automatic_result_count(),
            crate::gpu_contour::AUTOMATIC_PLACEMENT_CACHE_LIMIT
        );
        assert!(
            renderer
                .gpu_memory_usage()
                .retired_bytes_of(GpuResourceKind::ContourScratch)
                > 0,
            "evicted recorded contour resources must remain budgeted before submission"
        );
        let delayed_pixels_a = submit_recorded_rgba(&renderer, delayed_a);
        assert_eq!(
            delayed_pixels_a, baseline_a,
            "a later contour dispatch changed an already-recorded A command buffer"
        );
        assert_ne!(
            baseline_a,
            pixels_c.unwrap(),
            "the delayed-submit oracle needs visibly different A and C frames"
        );
        renderer.end_gpu_frame();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("retired GPU work completes");
        settle_gpu_retirement(&mut renderer);
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .retired_bytes_of(GpuResourceKind::ContourScratch),
            0
        );

        let explicit_anchor = crate::data_config::ContourLabelAnchor {
            level_index: 0,
            x: 0.5,
            y: 0.5,
            tx: 1.0,
            ty: -1.0,
        };
        let explicit = state_test_labelled_contour(70.0, vec![explicit_anchor]);
        let explicit_style = renderer.create_style_for_series(&explicit).unwrap();
        let explicit_series = [Series {
            config: &explicit,
            style: &explicit_style,
        }];
        let explicit_a = [ChartDrawItem {
            view: &view_a,
            chart_config: chart_a.config(),
            series: &explicit_series,
        }];
        let explicit_b = [ChartDrawItem {
            view: &view_b,
            chart_config: chart_b.config(),
            series: &explicit_series,
        }];
        let first = renderer.prepare(&explicit_a).unwrap();
        let second = renderer.prepare(&explicit_b).unwrap();
        assert_eq!(
            placement_of(&first),
            placement_of(&second),
            "explicit placement must remain one immutable shared snapshot"
        );
        assert_eq!(
            renderer.contour_cache["label-cache"]
                .label
                .as_ref()
                .unwrap()
                .automatic_result_count(),
            0
        );
    }

    #[test]
    fn automatic_contour_label_indirect_count_is_exactly_1024() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_contour_grid(&mut renderer);
        let chart = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            },
            (0.0, 1.0),
        );
        let mut config = state_test_labelled_contour(2000.0, Vec::new());
        let DataRenderType::Contour { contour, .. } = &mut config.render_type else {
            unreachable!();
        };
        contour.levels = vec![2.0; crate::data_config::MAX_CONTOUR_LEVELS];
        let style = renderer.create_style_for_series(&config).unwrap();
        let series = [Series {
            config: &config,
            style: &style,
        }];
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];

        let prepared = renderer.prepare(&items).unwrap();
        let count =
            pollster::block_on(renderer.contour_label_instance_count_for_test(&prepared, 0, 0))
                .unwrap()
                .expect("automatic contour label snapshot");
        assert_eq!(count, 1024, "native indirect label count must be exact");
    }

    #[test]
    fn contour_label_snapshot_survives_cache_replacement_until_token_drop() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_contour_grid(&mut renderer);
        let chart = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            },
            (0.0, 1.0),
        );
        let old_config = state_test_labelled_contour(90.0, Vec::new());
        let old_style = renderer.create_style_for_series(&old_config).unwrap();
        let old_series = [Series {
            config: &old_config,
            style: &old_style,
        }];
        let old_view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let old_items = [ChartDrawItem {
            view: &old_view,
            chart_config: chart.config(),
            series: &old_series,
        }];
        let placement_of = |prepared: &PreparedFrame| {
            prepared.items[0].series[0]
                .contour
                .as_ref()
                .and_then(|contour| contour.label.as_ref())
                .expect("labelled contour snapshot")
                .snapshot
                .placement_identity()
        };

        let old_prepared = renderer.prepare(&old_items).unwrap();
        let old_atlas = renderer.contour_cache["label-cache"]
            .label
            .as_ref()
            .unwrap()
            .atlas_identity();
        let old_placement = placement_of(&old_prepared);
        let old_live = renderer
            .gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::ContourScratch);
        assert!(old_live > 0, "old atlas and placement slot must be charged");

        let mut replacement_config = state_test_labelled_contour(90.0, Vec::new());
        state_test_contour_labels_mut(&mut replacement_config).font_size = 20.0;
        let replacement_style = renderer
            .create_style_for_series(&replacement_config)
            .unwrap();
        let replacement_series = [Series {
            config: &replacement_config,
            style: &replacement_style,
        }];
        let replacement_view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let replacement_items = [ChartDrawItem {
            view: &replacement_view,
            chart_config: chart.config(),
            series: &replacement_series,
        }];
        let replacement_prepared = renderer.prepare(&replacement_items).unwrap();
        let replacement_atlas = renderer.contour_cache["label-cache"]
            .label
            .as_ref()
            .unwrap()
            .atlas_identity();
        let replacement_placement = placement_of(&replacement_prepared);
        assert_ne!(old_atlas, replacement_atlas, "cache kept the old atlas");
        assert_ne!(
            old_placement, replacement_placement,
            "replacement reused the live token's placement slot"
        );
        assert!(renderer.validate_prepared(&old_prepared).is_ok());
        assert!(renderer.validate_prepared(&replacement_prepared).is_ok());

        let combined_live = renderer
            .gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::ContourScratch);
        assert!(
            combined_live > old_live,
            "replacement must add a separately charged atlas and placement"
        );
        drop(old_prepared);
        let after_old_drop = renderer.gpu_memory_usage();
        let replacement_live = after_old_drop.live_bytes_of(GpuResourceKind::ContourScratch);
        assert_eq!(
            combined_live,
            old_live + replacement_live,
            "the old token's exact charge must remain live across replacement"
        );
        assert_eq!(
            after_old_drop.retired_bytes_of(GpuResourceKind::ContourScratch),
            old_live,
            "dropping the old token must retire its atlas and placement charge"
        );

        drop(replacement_prepared);
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .live_bytes_of(GpuResourceKind::ContourScratch),
            replacement_live,
            "the replacement cache must retain its atlas and placement charge"
        );
        renderer.contour_cache.clear();
        let retired = renderer.gpu_memory_usage();
        assert_eq!(retired.live_bytes_of(GpuResourceKind::ContourScratch), 0);
        assert_eq!(
            retired.retired_bytes_of(GpuResourceKind::ContourScratch),
            combined_live,
            "cache removal must retire the replacement charge too"
        );
        renderer.end_gpu_frame();
        // The completion callback can run on another test's polling thread
        // on the shared device, so wait for it rather than for one poll.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while renderer
            .gpu_memory_usage()
            .bytes_of(GpuResourceKind::ContourScratch)
            != 0
        {
            renderer.service_gpu_completions().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "retirement callback timeout"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn multi_panel_contour_labels_survive_interleaved_prepare_and_repeat_paint() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_contour_grid(&mut renderer);
        let (panel_w, panel_h) = (320, 240);
        let target_w = panel_w * 2;
        let chart_a = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: panel_w,
                height: panel_h,
            },
            (0.0, 1.0),
        );
        let chart_b = state_test_contour_chart(
            Rect {
                x: panel_w,
                y: 0,
                width: panel_w,
                height: panel_h,
            },
            (-0.5, 1.5),
        );
        let chart_c = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: panel_w,
                height: panel_h,
            },
            (0.2, 0.8),
        );
        let config_a = state_test_labelled_contour(70.0, Vec::new());
        let config_b = state_test_labelled_contour(420.0, Vec::new());
        let config_c = state_test_labelled_contour(140.0, Vec::new());
        let style_a = renderer.create_style_for_series(&config_a).unwrap();
        let style_b = renderer.create_style_for_series(&config_b).unwrap();
        let style_c = renderer.create_style_for_series(&config_c).unwrap();
        let series_a = [Series {
            config: &config_a,
            style: &style_a,
        }];
        let series_b = [Series {
            config: &config_b,
            style: &style_b,
        }];
        let series_c = [Series {
            config: &config_c,
            style: &style_c,
        }];
        let view_a = renderer
            .create_chart_view(&chart_a, chart_a.config().chart_area.0)
            .unwrap();
        let view_b = renderer
            .create_chart_view(&chart_b, chart_b.config().chart_area.0)
            .unwrap();
        let view_c = renderer
            .create_chart_view(&chart_c, chart_c.config().chart_area.0)
            .unwrap();
        let multi_items = [
            ChartDrawItem {
                view: &view_a,
                chart_config: chart_a.config(),
                series: &series_a,
            },
            ChartDrawItem {
                view: &view_b,
                chart_config: chart_b.config(),
                series: &series_b,
            },
        ];
        let interleaved_items = [ChartDrawItem {
            view: &view_c,
            chart_config: chart_c.config(),
            series: &series_c,
        }];

        let multi = renderer.prepare(&multi_items).unwrap();
        let interleaved = renderer.prepare(&interleaved_items).unwrap();
        assert!(renderer.validate_prepared(&multi).is_ok());
        let first = paint_prepared_rgba(&renderer, &multi, target_w, panel_h);
        let repeated = paint_prepared_rgba(&renderer, &multi, target_w, panel_h);
        assert_eq!(first, repeated, "repeat paint changed a prepared token");
        drop((interleaved, multi));

        let only_a = [ChartDrawItem {
            view: &view_a,
            chart_config: chart_a.config(),
            series: &series_a,
        }];
        let prepared_a = renderer.prepare(&only_a).unwrap();
        let baseline_a = paint_prepared_rgba(&renderer, &prepared_a, target_w, panel_h);
        drop(prepared_a);
        let only_b = [ChartDrawItem {
            view: &view_b,
            chart_config: chart_b.config(),
            series: &series_b,
        }];
        let prepared_b = renderer.prepare(&only_b).unwrap();
        let baseline_b = paint_prepared_rgba(&renderer, &prepared_b, target_w, panel_h);

        let crop = |rgba: &[u8], x: u32| {
            let mut out = Vec::with_capacity((panel_w * panel_h * 4) as usize);
            for row in 0..panel_h {
                let start = ((row * target_w + x) * 4) as usize;
                out.extend_from_slice(&rgba[start..start + (panel_w * 4) as usize]);
            }
            out
        };
        let multi_a = crop(&first, 0);
        let multi_b = crop(&first, panel_w);
        assert_eq!(multi_a, crop(&baseline_a, 0));
        assert_eq!(multi_b, crop(&baseline_b, panel_w));
        let magenta = |rgba: &[u8]| {
            rgba.chunks_exact(4)
                .filter(|p| p[0] > 150 && p[1] < 80 && p[2] > 150 && p[3] > 100)
                .count()
        };
        assert!(
            magenta(&multi_a) > 10,
            "left panel drew no label background"
        );
        assert!(
            magenta(&multi_b) > 10,
            "right panel drew no label background"
        );
        assert_ne!(
            multi_a, multi_b,
            "different transforms and spacing produced the same panel baseline"
        );
    }

    #[test]
    fn stream_chunk_uses_shared_preparation_and_retains_work_charge() {
        use crate::streaming::{ColumnInput, ColumnRange, SourceEncoding};
        use crate::streaming_upload::{ChunkUploadBudget, record_chunk};
        let (device, queue) = crate::data_render::shared_device().expect("stream GPU required");
        let mut r = Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap();
        let values = [0.1f32, 0.5, 0.9];
        for id in ["rx", "ry"] {
            r.add_column(
                id,
                &Column {
                    data: values.to_vec(),
                    min: 0.1,
                    max: 0.9,
                },
            )
            .unwrap();
        }
        r.register_streamed_columns(
            ["sx", "sy"]
                .map(|id| crate::StreamColumn {
                    id: id.into(),
                    len: 3,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    revision: 1,
                    statistics: crate::StreamStatistics::Unknown,
                })
                .to_vec(),
        )
        .unwrap();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(0.0, 1.0);
        let resident_series = SeriesConfig {
            series_id: "s".into(),
            source_id: None,
            label: None,
            x_column: "rx".into(),
            y_column: "ry".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 0.5),
                    line_width: 5.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        };
        let mut stream_series = resident_series.clone();
        stream_series.x_column = "sx".into();
        stream_series.y_column = "sy".into();
        let resident_id = r
            .register_chart(chart.config().clone(), vec![resident_series])
            .unwrap();
        let streamed_id = r
            .register_chart(chart.config().clone(), vec![stream_series])
            .unwrap();
        let view = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let mut frame = r
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: resident_id,
                view: &view,
            }])
            .unwrap();
        let expected = paint_prepared_rgba(&r, &frame, 320, 240);
        let x = ColumnRange {
            column: 0,
            revision: 1,
            source_len: 3,
            offset: 0,
            len: 3,
            encoding: SourceEncoding::ScalarF32,
        };
        let y = ColumnRange { column: 1, ..x };
        let requested = [x, y];
        let mut encoder = r.device.create_command_encoder(&Default::default());
        let chunk = record_chunk(
            &r.device,
            &mut encoder,
            &r.gpu_ledger,
            ChunkUploadBudget {
                max_columns: 2,
                max_input_bytes: 24,
                max_work_buffer_bytes: 48,
                max_upload_bytes: 96,
                renderer_budget_bytes: u64::MAX,
                pool_bytes: r.gpu_memory_usage().pool_bytes(),
            },
            &[
                ColumnInput {
                    range: x,
                    bytes: bytemuck::cast_slice(&values),
                },
                ColumnInput {
                    range: y,
                    bytes: bytemuck::cast_slice(&values),
                },
            ],
        )
        .unwrap();
        r.queue.submit([encoder.finish()]);
        let cfg = r.chart_series(streamed_id).unwrap()[0].clone();
        let style = r.create_style_for_series(&cfg).unwrap();
        let stream_packet = {
            let (states, preparation) = r.preparation_parts();
            let state = &states[&streamed_id];
            let series = Series {
                config: &state.series[0],
                style: &style,
            };
            assert!(
                preparation
                    .build_stream_series(
                        &view,
                        &state.config,
                        &series,
                        &chunk,
                        &[
                            requested[0],
                            ColumnRange {
                                offset: 1,
                                len: 2,
                                ..requested[1]
                            },
                        ],
                        streaming_runtime::StreamDrawPhase::Line,
                    )
                    .is_err()
            );
            let mut dashed = state.series[0].clone();
            if let DataRenderType::Line { line } = &mut dashed.render_type {
                line.line_style = LineStylePreset::Dash;
            }
            assert!(
                preparation
                    .build_stream_series(
                        &view,
                        &state.config,
                        &Series {
                            config: &dashed,
                            style: &style
                        },
                        &chunk,
                        &requested,
                        streaming_runtime::StreamDrawPhase::Line,
                    )
                    .is_err()
            );
            let mut sketch = state.config.clone();
            sketch.draw_style = DrawStyle::Sketch(Default::default());
            assert!(
                preparation
                    .build_stream_series(
                        &view,
                        &sketch,
                        &series,
                        &chunk,
                        &requested,
                        streaming_runtime::StreamDrawPhase::Line,
                    )
                    .is_err()
            );
            preparation
                .build_stream_series(
                    &view,
                    &state.config,
                    &series,
                    &chunk,
                    &requested,
                    streaming_runtime::StreamDrawPhase::Line,
                )
                .unwrap()
        };
        frame.items[0].series = vec![stream_packet];
        frame.column_sources.clear();
        let state = &r.chart_states[&streamed_id];
        frame.registered_charts = vec![PreparedChartStamp {
            chart_id: streamed_id,
            desired: state.revisions.desired,
            data: state.revisions.data,
        }];
        // The prepared packet, not the temporary upload record, keeps work live.
        drop(chunk);
        r.end_gpu_frame();
        r.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        // The shared device may dispatch completion callbacks on another
        // test's poll thread. Wait for retirement itself, not only queue work.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while r
            .gpu_memory_usage()
            .retired_bytes_of(crate::GpuResourceKind::StreamingUpload)
            != 0
        {
            assert!(
                std::time::Instant::now() < deadline,
                "upload retirement timed out"
            );
            r.service_gpu_completions().unwrap();
            std::thread::yield_now();
        }
        assert_eq!(
            r.gpu_memory_usage()
                .bytes_of(crate::GpuResourceKind::StreamingUpload),
            48
        );
        assert_eq!(paint_prepared_rgba(&r, &frame, 320, 240), expected);
        assert!(r.pool.slot("sx").is_none());
        let mut replacement = match r.logical_column("sx").unwrap() {
            crate::LogicalColumn::Streamed(source) => source.clone(),
            _ => unreachable!(),
        };
        replacement.revision = 2;
        r.replace_streamed_columns(vec![replacement]).unwrap();
        assert!(r.validate_prepared(&frame).is_err());
        drop(frame);
        r.end_gpu_frame();
        r.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        // As above: the release can complete on another test's poll thread.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while r
            .gpu_memory_usage()
            .bytes_of(crate::GpuResourceKind::StreamingUpload)
            != 0
        {
            assert!(
                std::time::Instant::now() < deadline,
                "upload release timed out"
            );
            r.service_gpu_completions().unwrap();
            std::thread::yield_now();
        }
    }

    /// The token owns host inputs after prepare and rejects only renderer
    /// resource changes that invalidate its captured GPU work.
    #[test]
    fn registered_prepare_preserves_chart_identity_and_ssot_styles() {
        let (device, queue) = crate::data_render::shared_device()
            .expect("registered preparation regression requires a GPU adapter");
        let mut r = Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            1024 * 1024,
        )
        .unwrap();
        for id in ["registered_x", "registered_y"] {
            r.add_column(id, &col_f64(vec![0.1, 0.5, 0.9])).unwrap();
        }
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(0.0, 1.0);
        let series = SeriesConfig {
            series_id: "same-series-id".into(),
            source_id: None,
            label: None,
            x_column: "registered_x".into(),
            y_column: "registered_y".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 5.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        };
        // Equal configs, series ids and source columns cannot identify a chart.
        let a = r
            .register_chart(chart.config().clone(), vec![series.clone()])
            .unwrap();
        let b = r
            .register_chart(chart.config().clone(), vec![series.clone()])
            .unwrap();
        let view_a = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let view_b = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let view_b_only = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let both = r
            .prepare_registered(&[
                RegisteredChartDrawItem {
                    chart_id: a,
                    view: &view_a,
                },
                RegisteredChartDrawItem {
                    chart_id: b,
                    view: &view_b,
                },
            ])
            .unwrap();
        assert_eq!(both.registered_charts[0].chart_id, a);
        assert_eq!(both.registered_charts[1].chart_id, b);
        let before_a = paint_prepared_rgba(&r, &both, 320, 240);
        let b_only = r
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: b,
                view: &view_b_only,
            }])
            .unwrap();
        let before_b = paint_prepared_rgba(&r, &b_only, 320, 240);
        let b_style_revision = r.chart_states[&b]
            .prepared_styles
            .as_ref()
            .unwrap()
            .series_revision;
        let b_style_buffer = r.chart_states[&b].prepared_styles.as_ref().unwrap().styles[0]
            .line_bg
            .clone();
        r.validate_prepared(&both).unwrap();

        let mut changed = series.clone();
        if let DataRenderType::Line { line } = &mut changed.render_type {
            line.line_color = Color::new(0.0, 1.0, 0.0, 1.0);
        }
        r.set_chart_series(a, vec![changed]).unwrap();
        assert!(matches!(
            r.validate_prepared(&both),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
        r.validate_prepared(&b_only).unwrap();
        let next_a = r
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: a,
                view: &view_a,
            }])
            .unwrap();
        let after_a = paint_prepared_rgba(&r, &next_a, 320, 240);
        assert_ne!(
            before_a, after_a,
            "registered style must be regenerated from SSOT"
        );
        let b_again = r
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: b,
                view: &view_b,
            }])
            .unwrap();
        assert_eq!(before_b, paint_prepared_rgba(&r, &b_again, 320, 240));
        assert_eq!(before_b, paint_prepared_rgba(&r, &b_only, 320, 240));
        assert_eq!(
            r.chart_states[&b]
                .prepared_styles
                .as_ref()
                .unwrap()
                .series_revision,
            b_style_revision
        );
        assert_eq!(
            r.chart_states[&b].prepared_styles.as_ref().unwrap().styles[0].line_bg,
            b_style_buffer
        );

        let mut next_config = r.chart_config(a).unwrap().clone();
        next_config.bottom_x.max = 2.0;
        r.set_chart_config(a, next_config).unwrap();
        assert!(matches!(
            r.validate_prepared(&next_a),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
        r.validate_prepared(&b_only).unwrap();
        let removal_token = r
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: a,
                view: &view_a,
            }])
            .unwrap();
        r.remove_chart(a).unwrap();
        assert!(matches!(
            r.validate_prepared(&removal_token),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
        r.validate_prepared(&b_only).unwrap();
        assert_eq!(before_b, paint_prepared_rgba(&r, &b_only, 320, 240));

        let source_token = r
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: b,
                view: &view_b_only,
            }])
            .unwrap();
        r.upsert_column("registered_y", &col_f64(vec![0.9, 0.5, 0.1]))
            .unwrap();
        assert!(matches!(
            r.validate_prepared(&source_token),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
    }

    #[test]
    fn registered_prepare_preflight_failure_preserves_live_token() {
        let (device, queue) = crate::data_render::shared_device()
            .expect("registered preparation regression requires a GPU adapter");
        let mut r = Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            1024 * 1024,
        )
        .unwrap();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let chart = Chart::new(config);
        let a = r.register_chart(chart.config().clone(), vec![]).unwrap();
        let removed = r.register_chart(chart.config().clone(), vec![]).unwrap();
        r.remove_chart(removed).unwrap();
        let view = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let token = r
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: a,
                view: &view,
            }])
            .unwrap();
        let unprepared = r.register_chart(chart.config().clone(), vec![]).unwrap();
        let stamp = r.chart_render_stamp(a).unwrap();
        let revision = view
            .content_revision
            .load(std::sync::atomic::Ordering::Acquire);
        assert!(matches!(
            r.prepare_registered(&[
                RegisteredChartDrawItem {
                    chart_id: a,
                    view: &view
                },
                RegisteredChartDrawItem {
                    chart_id: unprepared,
                    view: &view
                },
                RegisteredChartDrawItem {
                    chart_id: removed,
                    view: &view
                },
            ]),
            Err(FiggyError::UnknownChart { .. })
        ));
        assert!(r.chart_states[&unprepared].prepared_styles.is_none());
        assert_eq!(r.chart_render_stamp(a).unwrap(), stamp);
        assert_eq!(
            view.content_revision
                .load(std::sync::atomic::Ordering::Acquire),
            revision
        );
        r.validate_prepared(&token).unwrap();
        let mut invalid = r.chart_config(a).unwrap().clone();
        invalid.bottom_x.min = f64::NAN;
        assert!(r.set_chart_config(a, invalid).is_err());
        r.validate_prepared(&token).unwrap();
        paint_prepared_rgba(&r, &token, 320, 240);
    }

    #[test]
    fn prepared_frame_owns_inputs_and_rejects_resource_staleness() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        r.add_column("st_x", &col_f64(vec![0.0, 0.5, 1.0])).unwrap();
        r.add_column("st_y", &col_f64(vec![0.0, 1.0, 0.25]))
            .unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(0.0, 1.0);

        let mk_cfg = |id: &str| SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "st_x".into(),
            y_column: "st_y".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        };
        let (cfg_a, cfg_b) = (mk_cfg("st_a"), mk_cfg("st_b"));
        let style = r.create_style_for_series(&cfg_a).unwrap();
        let view = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let series_ab = [
            Series {
                config: &cfg_a,
                style: &style,
            },
            Series {
                config: &cfg_b,
                style: &style,
            },
        ];
        let prepared = {
            let items = [ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &series_ab,
            }];
            let p = r.prepare(&items).unwrap();
            assert!(r.validate_prepared(&p).is_ok(), "fresh token validates");
            p
        };

        // (1) Reordered series → identity stamp.
        // (2) Chart config edited between prepare and paint → transform stamp.
        assert_eq!(prepared.items[0].series.len(), 2);
        chart.set_x_range(0.0, 2.0);
        assert!(r.validate_prepared(&prepared).is_ok());
        chart.set_x_range(0.0, 1.0);

        let mut foreign = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        foreign
            .add_column("st_x", &col_f64(vec![0.0, 0.5, 1.0]))
            .unwrap();
        foreign
            .add_column("st_y", &col_f64(vec![0.0, 1.0, 0.25]))
            .unwrap();
        assert!(matches!(
            foreign.validate_prepared(&prepared),
            Err(FiggyError::StalePreparedFrame { .. })
        ));

        let mut view_reuse = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let first_view_token = {
            let items = [ChartDrawItem {
                view: &view_reuse,
                chart_config: chart.config(),
                series: &series_ab,
            }];
            r.prepare(&items).unwrap()
        };
        let second_view_token = {
            let items = [ChartDrawItem {
                view: &view_reuse,
                chart_config: chart.config(),
                series: &series_ab,
            }];
            r.prepare(&items).unwrap()
        };
        assert!(matches!(
            r.validate_prepared(&first_view_token),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
        assert!(r.validate_prepared(&second_view_token).is_ok());
        r.refresh_axis(&mut view_reuse, &chart, chart.config().chart_area.0)
            .unwrap();
        assert!(matches!(
            r.validate_prepared(&second_view_token),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
        view_reuse
            .content_revision
            .store(u64::MAX, std::sync::atomic::Ordering::Release);
        assert!(matches!(
            r.update_transform(&view_reuse, &chart),
            Err(FiggyError::CounterExhausted {
                counter: "chart view content revision"
            })
        ));

        // (3) Remove + re-add an arc source at the same physical layout.
        // Layout generation stays put, so the source allocation epoch must
        // invalidate the retained arc snapshot.
        let cfg_dash = SeriesConfig {
            series_id: "st_dash".into(),
            source_id: None,
            label: None,
            x_column: "st_x".into(),
            y_column: "st_y".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Dash,
                    line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        };
        let style_dash = r.create_style_for_series(&cfg_dash).unwrap();
        let series_dash = [Series {
            config: &cfg_dash,
            style: &style_dash,
        }];
        r.add_column("st_unrelated", &col_f64(vec![9.0])).unwrap();
        let prepared_dash = {
            let items = [ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &series_dash,
            }];
            r.prepare(&items).unwrap()
        };
        assert!(r.remove_column("st_unrelated").unwrap());
        assert!(r.validate_prepared(&prepared_dash).is_ok());
        let old_y_handle = r.handle_for("st_y").unwrap();
        let old_y_epoch = r.pool().allocation_epoch("st_y").unwrap();
        assert!(r.remove_column("st_y").unwrap());
        let new_y_handle = r
            .add_column("st_y", &col_f64(vec![0.0, 1.0, 0.25]))
            .unwrap();
        assert_eq!(new_y_handle.offset, old_y_handle.offset);
        assert_eq!(new_y_handle.byte_size, old_y_handle.byte_size);
        assert_eq!(new_y_handle.len_values, old_y_handle.len_values);
        assert_ne!(r.pool().allocation_epoch("st_y"), Some(old_y_epoch));
        assert!(matches!(
            r.validate_prepared(&prepared_dash),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
        assert!(matches!(
            r.validate_prepared(&prepared),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
        let prepared_target = {
            let items = [ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &series_ab,
            }];
            r.prepare(&items).unwrap()
        };

        // (4) Target-format switch rebuilds the pipeline cache → format stamp.
        assert!(
            r.ensure_target_format(wgpu::TextureFormat::Rgba8Unorm)
                .unwrap()
        );
        assert!(
            r.ensure_target_format(wgpu::TextureFormat::Bgra8Unorm)
                .unwrap()
        );
        assert!(matches!(
            r.validate_prepared(&prepared_target),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
    }

    /// End-to-end split under the adversarial host schedule: record A, drop its
    /// token without submitting, dispatch and draw a different B transform,
    /// then submit A. The recorded arc/star resources must remain A's exact
    /// immutable result.
    #[test]
    fn recorded_arc_star_frame_survives_later_compute_before_submit() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Rgba8Unorm,
            4 * 1024 * 1024,
        )
        .unwrap();
        r.add_column("mw_x", &col_f64(vec![0.0, 0.25, 0.5, 0.75, 1.0]))
            .unwrap();
        r.add_column("mw_y", &col_f64(vec![0.2, 0.75, 0.35, 0.9, 0.5]))
            .unwrap();

        let (w, h) = (320u32, 240u32);
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        });
        config.legend.visible = false;
        config.draw_style =
            crate::config::DrawStyle::Milkyway(crate::config::MilkywayOptions::default());
        let mut chart_a = Chart::new(config.clone());
        chart_a.set_x_range(-0.05, 1.05);
        chart_a.set_y_range(0.0, 1.0);
        let mut chart_b = Chart::new(config);
        chart_b.set_x_range(-1.0, 2.0);
        chart_b.set_y_range(-0.5, 1.5);

        let series_cfg = SeriesConfig {
            series_id: "mw".into(),
            source_id: None,
            label: None,
            x_column: "mw_x".into(),
            y_column: "mw_y".into(),
            render_type: DataRenderType::ScatterLine {
                scatter: test_scatter_style(),
                line: DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(0.4, 0.7, 1.0, 1.0),
                    line_width: 2.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        };
        let style = r.create_style_for_series(&series_cfg).unwrap();
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let view_a = r
            .create_chart_view(&chart_a, chart_a.config().chart_area.0)
            .unwrap();
        let view_b = r
            .create_chart_view(&chart_b, chart_b.config().chart_area.0)
            .unwrap();
        let items_a = [ChartDrawItem {
            view: &view_a,
            chart_config: chart_a.config(),
            series: &series,
        }];
        let items_b = [ChartDrawItem {
            view: &view_b,
            chart_config: chart_b.config(),
            series: &series,
        }];

        let baseline_a = {
            let prepared = r.prepare(&items_a).unwrap();
            assert!(
                prepared.items[0].series[0].line_extra.is_some(),
                "milkyway line series must carry a star-pass snapshot"
            );
            paint_prepared_rgba(&r, &prepared, w, h)
        };
        let delayed_a = {
            let prepared = r.prepare(&items_a).unwrap();
            record_prepared_rgba(&r, &prepared, w, h)
        };
        let pixels_b = {
            let prepared = r.prepare(&items_b).unwrap();
            paint_prepared_rgba(&r, &prepared, w, h)
        };
        for key in 0..8 {
            let mut chart = Chart::new(chart_a.config().clone());
            chart.set_x_range(-2.0 - f64::from(key) * 0.25, 2.5 + f64::from(key) * 0.5);
            chart.set_y_range(-1.0 - f64::from(key) * 0.125, 1.75 + f64::from(key) * 0.25);
            let view = r
                .create_chart_view(&chart, chart.config().chart_area.0)
                .unwrap();
            let items = [ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &series,
            }];
            let prepared = r.prepare(&items).unwrap();
            let _ = paint_prepared_rgba(&r, &prepared, w, h);
        }
        assert_eq!(
            r.arc_cache.get("mw").map(Vec::len),
            Some(ARC_RESULT_CACHE_LIMIT_PER_SERIES)
        );
        assert!(
            r.gpu_memory_usage()
                .retired_bytes_of(GpuResourceKind::ArcScan)
                > 0,
            "evicted recorded arc resources must remain budgeted before submission"
        );
        let delayed_pixels_a = submit_recorded_rgba(&r, delayed_a);
        assert_eq!(
            delayed_pixels_a, baseline_a,
            "a later arc/star dispatch changed an already-recorded A command buffer"
        );
        assert_ne!(
            baseline_a, pixels_b,
            "the delayed-submit oracle needs visibly different A and B frames"
        );
        let lit = delayed_pixels_a
            .chunks_exact(4)
            .filter(|pixel| pixel[3] > 0)
            .count();
        assert!(
            lit > 100,
            "delayed milkyway frame produced too little ink: {lit}"
        );
        r.end_gpu_frame();
        r.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("retired GPU work completes");
        settle_gpu_retirement(&mut r);
        assert_eq!(
            r.gpu_memory_usage()
                .retired_bytes_of(GpuResourceKind::ArcScan),
            0
        );
    }

    /// Log-axis auto-fit with zeros in the data must clamp the lower bound
    /// to the smallest POSITIVE uploaded value instead of
    /// feeding log10 a non-positive min.
    #[test]
    fn log_auto_fit_clamps_to_smallest_positive_uploaded_value() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        // Scalar stats follow the recorded f32 pair, not the source f64.
        let vals = vec![0.0, 0.04, 1.0, 50.0, 1000.0];
        r.add_column("v", &col_f64(vals)).unwrap();

        let mut config = crate::default::default_config();
        config.left_y.scale = crate::config::AxisScale::Logarithmic;
        let mut chart = Chart::new(config);
        chart.auto_fit_y_union(r.pool(), &["v"], 0.0).unwrap();

        let min = chart.config().left_y.min;
        let uploaded_min = (0.04_f64 as f32) as f64;
        assert_eq!(min, uploaded_min);
        assert_ne!(uploaded_min, 0.04_f64);
        assert!(chart.config().left_y.max >= 1000.0);
    }

    /// `auto_fit_*` must leave a UNIFORM margin fraction on all four sides —
    /// measured in pixels, from the user's point of view. Data is a triangle
    /// wave touching its exact min/max so the ink bbox equals the data bbox.
    #[test]
    fn auto_fit_margins_are_uniform_on_all_sides() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .unwrap();

        let n = 400;
        let xs: Vec<f64> = (0..n).map(|i| i as f64).collect();
        // Triangle wave between exactly 10 and 90.
        let ys: Vec<f64> = (0..n)
            .map(|i| {
                let t = (i % 80) as f64 / 80.0;
                let tri = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
                10.0 + tri * 80.0
            })
            .collect();
        r.add_column("x", &col_f64(xs)).unwrap();
        r.add_column("y", &col_f64(ys)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        });
        config.legend.visible = false;
        // Bare data area: no grid/tick ink inside to pollute the bbox.
        config.grid.show_major_x = false;
        config.grid.show_major_y = false;
        config.grid.show_minor_x = false;
        config.grid.show_minor_y = false;
        let mut chart = Chart::new(config);
        let pad = 0.05;
        chart.auto_fit_x_union(r.pool(), &["x"], pad).unwrap();
        chart.auto_fit_y_union(r.pool(), &["y"], pad).unwrap();

        let series = [SeriesConfig {
            series_id: "s".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (w, h) = (img.width as usize, img.height as usize);
        let da = chart.config().data_area().unwrap().0;

        let (mut min_x, mut max_x, mut min_y, mut max_y) = (usize::MAX, 0usize, usize::MAX, 0usize);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let p = &img.rgba[i..i + 4];
                if p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90 {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        assert!(min_x < max_x, "no data ink found");

        // Expected margin fraction of the data_area extent on every side:
        // pad / (1 + 2·pad) of the full padded span.
        let expect = pad / (1.0 + 2.0 * pad);
        let close = |frac: f64, side: &str| {
            assert!(
                (frac - expect).abs() < 0.015,
                "{side} margin {frac:.3}, expected {expect:.3}"
            );
        };
        let daw = da.width as f64;
        let dah = da.height as f64;
        close((min_x as f64 - da.x as f64) / daw, "left");
        close((da.x as f64 + daw - 1.0 - max_x as f64) / daw, "right");
        close((min_y as f64 - da.y as f64) / dah, "top");
        close((da.y as f64 + dah - 1.0 - max_y as f64) / dah, "bottom");
    }

    /// Hide the four axis lines for a data-layer pixel test.
    ///
    /// The decoration layer is composited **over** the data by design, so an axis
    /// line sitting on the data area's boundary legitimately covers the first or
    /// last column of whatever the data layer drew there. A continuity assertion
    /// that scans every column between the first and last inked one would then
    /// report that covered column as a break in the *curve*, which is not what it
    /// is measuring.
    ///
    /// This is not a tolerance: the axis chrome is simply not part of the question
    /// "does the stroke ever break". Whether it lands opaque on that column or
    /// leaks a little colour through its antialiasing is a rasterizer detail — on
    /// llvmpipe it lands opaque, which is what made these two tests look
    /// environment-dependent.
    fn hide_axis_lines(config: &mut crate::Config) {
        config.bottom_x.line_visible = false;
        config.top_x.line_visible = false;
        config.left_y.line_visible = false;
        config.right_y.line_visible = false;
    }

    /// Offscreen endpoints still contribute their connecting segment on the
    /// first draw, including after a very deep zoom or a small axis change.
    #[test]
    fn precise_zoom_keeps_connections_to_distant_endpoints() {
        let (device, queue) = crate::data_render::shared_device().expect("GPU required");
        let mut r = Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 400,
        });
        config.legend.visible = false;
        hide_axis_lines(&mut config);
        let series = [SeriesConfig {
            series_id: "crossing".into(),
            source_id: None,
            label: None,
            x_column: "zx".into(),
            y_column: "zy".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_width: 2.0,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }];
        for vertical in [false, true] {
            for reverse in [false, true] {
                for span in [2.0, 1e4, 1e6, 1e9] {
                    let mut along = vec![-span, 0.5, span];
                    let mut across = vec![0.1, 0.5, 0.9];
                    if reverse {
                        along.reverse();
                        across.reverse();
                    }
                    let (x, y) = if vertical {
                        (across, along)
                    } else {
                        (along, across)
                    };
                    let _ = r.remove_column("zx");
                    let _ = r.remove_column("zy");
                    r.add_column("zx", &col_f64(x)).unwrap();
                    r.add_column("zy", &col_f64(y)).unwrap();
                    for shift in [0.0, 0.001, 0.0] {
                        let mut chart = Chart::new(config.clone());
                        chart.set_x_range(shift, 1.0 + shift);
                        chart.set_y_range(shift, 1.0 + shift);
                        let image = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
                        let area = chart.config().data_area().unwrap().0;
                        let red = |x: u32, y: u32| {
                            let p = &image.rgba[((y * image.width + x) * 4) as usize..][..4];
                            p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
                        };
                        for fraction in [0.05, 0.25, 0.5, 0.75, 0.95] {
                            let present = if vertical {
                                let y = area.y + (area.height as f32 * fraction) as u32;
                                (area.x..area.x + area.width).any(|x| red(x, y))
                            } else {
                                let x = area.x + (area.width as f32 * fraction) as u32;
                                (area.y..area.y + area.height).any(|y| red(x, y))
                            };
                            assert!(
                                present,
                                "missing connection: vertical={vertical} reverse={reverse} span={span} shift={shift} fraction={fraction}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A solid line stays CONTINUOUS no matter how dense the data is. With
    /// sub-pixel segments (here ~0.04 px each) naive per-segment quads
    /// degenerate into disconnected slivers — every x column the curve
    /// crosses must contain ink.
    ///
    /// Axis lines off ([`hide_axis_lines`]): the y-axis line covers the data
    /// area's first column, and that is the decoration layer doing its job rather
    /// than the stroke breaking.
    #[test]
    fn dense_line_has_no_gaps() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .unwrap();

        let n = 50_000;
        let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
        // Decay curve like the report, plus measurement-style jitter: the
        // sub-pixel zig-zag flips the segment direction at every point,
        // which is what real noisy data does to the quad extrusion.
        let ys: Vec<f64> = xs
            .iter()
            .enumerate()
            .map(|(i, x)| (-6.0 * x).exp() + if i % 2 == 0 { 0.0008 } else { -0.0008 })
            .collect();
        r.add_column("dx", &col_f64(xs)).unwrap();
        r.add_column("dy", &col_f64(ys)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 400,
        });
        config.legend.visible = false;
        hide_axis_lines(&mut config);
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(-0.05, 1.05);

        let series = [SeriesConfig {
            series_id: "dense".into(),
            source_id: None,
            label: None,
            x_column: "dx".into(),
            y_column: "dy".into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 1.5,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (w, h) = (img.width as usize, img.height as usize);
        let col_has_red = |x: usize| {
            (0..h).any(|y| {
                let i = (y * w + x) * 4;
                let p = &img.rgba[i..i + 4];
                p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
            })
        };
        let first = (0..w)
            .find(|&x| col_has_red(x))
            .expect("curve drew nothing");
        let last = (0..w).rev().find(|&x| col_has_red(x)).unwrap();
        let gaps: Vec<usize> = (first..=last).filter(|&x| !col_has_red(x)).collect();
        assert!(
            gaps.is_empty(),
            "dense solid line has {} gap columns between x={first} and x={last}: {:?}…",
            gaps.len(),
            &gaps[..gaps.len().min(20)]
        );

        // Continuity alone is not enough — a stippled line can still touch
        // every column. A 1.5 px stroke must average ≥ 1 ink pixel per
        // column along a near-horizontal curve.
        let red_per_col = |x: usize| {
            (0..h)
                .filter(|&y| {
                    let i = (y * w + x) * 4;
                    let p = &img.rgba[i..i + 4];
                    p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
                })
                .count()
        };
        let total: usize = (first..=last).map(red_per_col).sum();
        let density = total as f64 / (last - first + 1) as f64;
        assert!(
            density >= 1.0,
            "dense solid line is stippled: {density:.2} ink px per column (expected ≥ 1.0)"
        );
    }

    /// Precise solid lines are rendered into a single-sample export target, so
    /// smooth edges must come from the line shader itself. A shallow 1 px line
    /// should therefore contain both strong stroke coverage and partial alpha
    /// fringe pixels.
    #[test]
    fn solid_line_exports_analytic_aa_fringe() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .unwrap();

        r.add_column("aa_x", &col_f64(vec![0.0, 10.0])).unwrap();
        r.add_column("aa_y", &col_f64(vec![4.8, 5.2])).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 220,
            height: 90,
        });
        config.legend.visible = false;
        config.grid.show_major_x = false;
        config.grid.show_major_y = false;
        config.grid.show_minor_x = false;
        config.grid.show_minor_y = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 10.0);
        chart.set_y_range(0.0, 10.0);

        let series = [SeriesConfig {
            series_id: "aa".into(),
            source_id: None,
            label: None,
            x_column: "aa_x".into(),
            y_column: "aa_y".into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let red_alphas: Vec<u8> = img
            .rgba
            .chunks_exact(4)
            .filter_map(|p| (p[0] > 200 && p[1] < 40 && p[2] < 40 && p[3] > 0).then_some(p[3]))
            .collect();

        assert!(!red_alphas.is_empty(), "line drew no red pixels");
        let max_alpha = red_alphas.iter().copied().max().unwrap_or(0);
        assert!(
            max_alpha >= 180,
            "line has no strong stroke coverage, max alpha {max_alpha}: {red_alphas:?}"
        );
        assert!(
            red_alphas.iter().any(|&a| (16..=239).contains(&a)),
            "line has no partial-alpha AA fringe pixels: {red_alphas:?}"
        );
    }

    /// The square-cap fix must NOT change line semantics: NaN values still
    /// break the line (the caps may narrow a gap by ~line_width, never close
    /// a real one), and nothing assumes monotonic x — a path that doubles
    /// back stays continuous.
    ///
    /// Axis lines off ([`hide_axis_lines`]) for the same reason as
    /// `dense_line_has_no_gaps`: the y-axis line covers the data area's first
    /// column, which would read as one pass of the doubled-back path missing.
    #[test]
    fn caps_preserve_nan_breaks_and_nonmonotonic_paths() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 400,
        });
        config.legend.visible = false;
        hide_axis_lines(&mut config);
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(-0.1, 1.1);

        let line_cfg = |id: &str, x: &str, y: &str| SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 1.5,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        };
        let red_cols = |img: &crate::RasterImage| -> Vec<usize> {
            let (w, h) = (img.width as usize, img.height as usize);
            (0..w)
                .filter(|&x| {
                    (0..h).any(|y| {
                        let i = (y * w + x) * 4;
                        let p = &img.rgba[i..i + 4];
                        p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
                    })
                })
                .collect()
        };

        // 1) Dense noisy line with a NaN band: x ∈ [0.40, 0.45] → ~36 px of
        //    data area. The break must survive (gap may shrink by ~1.5 px of
        //    caps, so probe the gap's middle).
        let n = 20_000;
        let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
        let ys: Vec<f64> = xs
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                if (0.40..0.45).contains(&x) {
                    f64::NAN
                } else {
                    0.5 + 0.3 * (x * 9.0).sin() + if i % 2 == 0 { 0.0008 } else { -0.0008 }
                }
            })
            .collect();
        r.add_column("gx", &col_f64(xs)).unwrap();
        r.add_column("gy", &col_f64(ys)).unwrap();

        let series = [line_cfg("gap", "gx", "gy")];
        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let cols = red_cols(&img);
        let da = chart.config().data_area().unwrap().0;
        let to_px = |v: f64| da.x as f64 + v * da.width as f64;
        let (gap_a, gap_b) = (to_px(0.40), to_px(0.45));
        let mid_a = (gap_a + 4.0) as usize;
        let mid_b = (gap_b - 4.0) as usize;
        let leaked: Vec<&usize> = cols.iter().filter(|&&x| x >= mid_a && x <= mid_b).collect();
        assert!(
            leaked.is_empty(),
            "NaN break was bridged: ink at columns {leaked:?} inside the gap ({mid_a}..{mid_b})"
        );
        // Both sides of the gap still drew.
        assert!(
            cols.iter().any(|&x| (x as f64) < gap_a - 2.0),
            "left of gap missing"
        );
        assert!(
            cols.iter().any(|&x| (x as f64) > gap_b + 2.0),
            "right of gap missing"
        );

        // 2) Non-monotonic path: x sweeps 0→1 then back 1→0 at a higher y.
        //    Every column the path covers must have ink in BOTH passes.
        let m = 30_000;
        let xs2: Vec<f64> = (0..m)
            .map(|i| {
                let t = i as f64 / (m - 1) as f64;
                if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }
            })
            .collect();
        let ys2: Vec<f64> = (0..m)
            .map(|i| {
                let t = i as f64 / (m - 1) as f64;
                let base = if t < 0.5 { 0.25 } else { 0.75 };
                base + if i % 2 == 0 { 0.0008 } else { -0.0008 }
            })
            .collect();
        r.add_column("mx", &col_f64(xs2)).unwrap();
        r.add_column("my", &col_f64(ys2)).unwrap();

        let series2 = [line_cfg("loop", "mx", "my")];
        let img2 = r.export_panel_rgba(&chart, &series2, 1.0).unwrap();
        let (w2, h2) = (img2.width as usize, img2.height as usize);
        let half = h2 / 2; // y=0.25 band renders below the midline, 0.75 above
        let band_has = |x: usize, top: bool| {
            let range = if top { 0..half } else { half..h2 };
            range.into_iter().any(|y| {
                let i = (y * w2 + x) * 4;
                let p = &img2.rgba[i..i + 4];
                p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
            })
        };
        let cols2 = red_cols(&img2);
        let (first2, last2) = (*cols2.first().unwrap(), *cols2.last().unwrap());
        let gaps2: Vec<usize> = (first2..=last2)
            .filter(|&x| !(band_has(x, true) && band_has(x, false)))
            .collect();
        assert!(
            gaps2.is_empty(),
            "non-monotonic path broke: {} columns missing one of the two passes: {:?}…",
            gaps2.len(),
            &gaps2[..gaps2.len().min(20)]
        );
    }

    /// Dash regularity on a CURVED line: the dash phase must advance with
    /// arc length, so on/off run lengths along the curve stay close to the
    /// pattern (Dash = [8, 4]). A phase that jumps at segment joints
    /// (per-segment restart, or screen-position projection) shatters the
    /// pattern into irregular fragments and fails the run statistics.
    #[test]
    fn dashed_curve_runs_match_pattern() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .unwrap();

        // Worst case for joint phase errors: a SPARSE sine (large direction
        // change per segment) on a WIDE panel (fragments far from the origin
        // amplify any screen-position-derived phase).
        let n = 64;
        let period = 628.0_f64 / 100.0;
        let ts: Vec<f64> = (0..n).map(|i| i as f64 * period / (n - 1) as f64).collect();
        let vs: Vec<f64> = ts.iter().map(|t| t.sin()).collect();
        r.add_column("t", &col_f64(ts)).unwrap();
        r.add_column("v", &col_f64(vs)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 1000,
            height: 600,
        });
        // Hide chrome that could add red-ish AA pixels; keep it minimal.
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, period);
        chart.set_y_range(-1.2, 1.2);

        let series = [SeriesConfig {
            series_id: "rc".into(),
            source_id: None,
            label: None,
            x_column: "t".into(),
            y_column: "v".into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Dash, // [8, 4]
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 2.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (w, h) = (img.width as usize, img.height as usize);
        let is_red = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            let p = &img.rgba[i..i + 4];
            // Dash run statistics should follow the solid core of the stroke,
            // not the intentionally-added analytic AA fringe.
            p[3] > 96 && p[0] > 120 && p[1] < 90 && p[2] < 90
        };

        // Trace the x-monotonic curve column by column: ink centroid y per
        // column (None = gap), arc step ds = √(1 + dy²) per 1-px x step.
        // On/off run lengths in ARC pixels must then match the [8, 4]
        // pattern regardless of local slope.
        let col_y: Vec<Option<f64>> = (0..w)
            .map(|x| {
                let ys: Vec<usize> = (0..h).filter(|&y| is_red(x, y)).collect();
                if ys.is_empty() {
                    None
                } else {
                    Some(ys.iter().sum::<usize>() as f64 / ys.len() as f64)
                }
            })
            .collect();
        let x0 = col_y
            .iter()
            .position(|c| c.is_some())
            .expect("dashed curve drew nothing");
        let x1 = col_y.iter().rposition(|c| c.is_some()).unwrap();

        let mut runs_on: Vec<f64> = Vec::new();
        let mut runs_off: Vec<f64> = Vec::new();
        let mut cur_on = true;
        let mut run = 0.0f64;
        let mut last_y = col_y[x0].unwrap();
        for value in &col_y[x0..=x1] {
            let (on, ds) = match *value {
                Some(y) => {
                    let ds = (1.0 + (y - last_y).powi(2)).sqrt();
                    last_y = y;
                    (true, ds)
                }
                // In a gap the curve continues invisibly — approximate the
                // arc step with the local slope we last saw.
                None => (false, 1.0),
            };
            if on == cur_on {
                run += ds;
            } else {
                if cur_on {
                    runs_on.push(run)
                } else {
                    runs_off.push(run)
                }
                cur_on = on;
                run = ds;
            }
        }
        if cur_on {
            runs_on.push(run)
        } else {
            runs_off.push(run)
        }

        let median = |v: &mut Vec<f64>| -> f64 {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if v.is_empty() { 0.0 } else { v[v.len() / 2] }
        };
        let needles = runs_on.iter().filter(|&&l| l <= 3.0).count();
        let n_on = runs_on.len();
        let med_on = median(&mut runs_on);
        let med_off = median(&mut runs_off);

        // Dash pattern is [8 on, 4 off] in arc px (line caps blur ±~2 px).
        assert!(
            (5.0..=12.0).contains(&med_on),
            "median on-run {med_on:.1} arc-px, expected ≈8 (runs: {runs_on:?})"
        );
        assert!(
            (2.0..=7.0).contains(&med_off),
            "median off-run {med_off:.1} arc-px, expected ≈4 (runs: {runs_off:?})"
        );
        assert!(
            needles * 5 <= n_on.max(1),
            "{needles}/{n_on} on-runs are needle fragments (≤3 arc-px): {runs_on:?}"
        );
    }

    #[test]
    fn renderer_init_and_add_column() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();

        let c = col_f64((0..100).map(|i| i as f64).collect());
        let h = r.add_column("x", &c).unwrap();
        assert_eq!(h.len_values, 100);

        let h2 = r.handle_for("x").unwrap();
        assert!(r.is_valid_handle(&h2));

        // Unknown id → UnknownColumn.
        let res = r.handle_for("nope");
        assert!(matches!(res, Err(FiggyError::UnknownColumn { .. })));
    }

    // GPU memory accounting. These live in-crate so they can compare the
    // report against the resources the renderer actually holds — a self-report
    // is only worth what an independent recount proves.

    fn panel_config(width: u32, height: u32) -> Config {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width,
            height,
        });
        config
    }

    /// Independent recount of the pool row: the buffers the pool holds, read
    /// back from wgpu rather than from any counter.
    fn pool_bytes_held(renderer: &Renderer) -> u64 {
        renderer.pool.buffer().size()
            + renderer
                .pool
                .backup_buffer_for_test()
                .map_or(0, |backup| backup.size())
    }

    #[test]
    fn pool_row_equals_the_buffers_the_pool_actually_holds() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let usage = renderer.gpu_memory_usage();
        assert_eq!(
            usage.pool_bytes(),
            pool_bytes_held(&renderer),
            "pool row disagrees with the buffers held\n{}",
            usage.report()
        );
        assert_eq!(usage.pool_bytes(), renderer.pool.capacity());

        // A defragmentation materializes the ping-pong backup, so the pool's
        // footprint becomes two buffers and the row must follow.
        let column = col_f64((0..64).map(|i| i as f64).collect());
        renderer.add_column("x", &column).unwrap();
        renderer.add_column("y", &column).unwrap();
        renderer.remove_column("x").unwrap();
        renderer.defragment().unwrap();

        let usage = renderer.gpu_memory_usage();
        assert_eq!(
            usage.live_bytes_of(GpuResourceKind::ColumnPool),
            pool_bytes_held(&renderer),
            "pool row missed the ping-pong backup\n{}",
            usage.report()
        );
        assert_eq!(
            usage.pool_bytes(),
            renderer.pool.capacity() * 2 + usage.retired_bytes_of(GpuResourceKind::ColumnPool)
        );
    }

    #[test]
    fn data_upload_moves_only_the_pool_row() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let before = renderer.gpu_memory_usage();
        let column = col_f64((0..4096).map(|i| i as f64).collect());
        renderer.add_column("x", &column).unwrap();
        renderer.end_gpu_frame();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("retired GPU work completes");
        settle_gpu_retirement(&mut renderer);
        let after = renderer.gpu_memory_usage();

        assert_eq!(
            before.external_bytes(),
            after.external_bytes(),
            "uploading data must not change out-of-pool bytes\nbefore:\n{}after:\n{}",
            before.report(),
            after.report()
        );
        assert_eq!(
            before.pool_bytes(),
            after.pool_bytes(),
            "the slab was already allocated; an upload fills it, it does not grow it"
        );
    }

    #[test]
    fn upload_peak_and_current_usage_include_submitted_staging() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let values = 4096usize;
        let column = col_f64((0..values).map(|i| i as f64).collect());
        renderer.pool.reset_peak_bytes();
        renderer.add_column("x", &column).unwrap();

        let staged = (values as u64) * crate::data::COLUMN_VALUE_BYTES as u64;
        let usage = renderer.gpu_memory_usage();
        assert_eq!(
            renderer.pool.peak_bytes(),
            usage.live_bytes_of(GpuResourceKind::ColumnPool) + staged,
            "the peak must include submitted staging"
        );
        assert_eq!(
            usage.retired_bytes_of(GpuResourceKind::ColumnPool),
            staged,
            "submitted staging remains budgeted until queue completion"
        );
    }

    #[test]
    fn panel_textures_are_charged_at_exactly_two_rgba_layers() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let (w, h) = (320u32, 240u32);
        let chart = Chart::new(panel_config(w, h));
        let before = renderer.gpu_memory_usage();
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let after = renderer.gpu_memory_usage();

        let charged = after.bytes_of(GpuResourceKind::PanelTexture)
            - before.bytes_of(GpuResourceKind::PanelTexture);
        let held = crate::gpu_memory::texture_bytes(&view.grid_texture)
            + crate::gpu_memory::texture_bytes(&view.decoration_texture);
        assert_eq!(charged, held, "charge disagrees with the textures held");
        assert_eq!(
            charged,
            u64::from(w) * u64::from(h) * 4 * 2,
            "grid + decoration, RGBA8, one sample\n{}",
            after.report()
        );

        // Dropping the panel credits the bytes back — as retired first,
        // because wgpu releases them when the queue drains, not at the drop.
        drop(view);
        let retired = renderer.gpu_memory_usage();
        assert_eq!(
            retired.retired_bytes_of(GpuResourceKind::PanelTexture),
            held
        );
        assert_eq!(
            retired.bytes_of(GpuResourceKind::PanelTexture),
            before.bytes_of(GpuResourceKind::PanelTexture) + held,
            "released bytes stay in the total until the submission boundary"
        );
        renderer.end_gpu_frame();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("retired GPU work completes");
        settle_gpu_retirement(&mut renderer);
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .bytes_of(GpuResourceKind::PanelTexture),
            before.bytes_of(GpuResourceKind::PanelTexture),
            "the submission boundary clears retirement"
        );
    }

    #[test]
    fn idle_nonblocking_service_completes_retirement_without_another_submission() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let chart = Chart::new(panel_config(320, 240));
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        drop(view);
        assert!(renderer.gpu_memory_usage().retired_bytes() > 0);
        renderer.end_gpu_frame();

        // No further submission or boundary call: an idle host services only
        // nonblocking callbacks until the already submitted work completes.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            renderer
                .service_gpu_completions()
                .expect("device remains available");
            if renderer.gpu_memory_usage().retired_bytes() == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "idle retirement did not complete"
            );
            std::thread::yield_now();
        }
        renderer
            .service_gpu_completions()
            .expect("empty poll is idempotent");
        assert_eq!(renderer.gpu_memory_usage().retired_bytes(), 0);
    }

    #[test]
    fn pool_cleanup_releases_capacity_after_delete_and_keeps_survivor() {
        let Some(mut renderer) = state_test_renderer_with_capacity(1024 * 1024) else {
            return;
        };
        renderer
            .add_column("keep", &col_f64(vec![1.0, 2.0, 3.0]))
            .unwrap();
        renderer
            .add_column("drop", &col_f64(vec![4.0, 5.0, 6.0]))
            .unwrap();
        renderer.remove_column("drop").unwrap();
        let old_handle = renderer.handle_for("keep").unwrap();
        let before = renderer.gpu_memory_usage().total_bytes();

        pollster::block_on(renderer.release_unused_pool_memory(16 * 1024)).unwrap();

        assert_eq!(renderer.pool().capacity(), 16 * 1024);
        assert_eq!(renderer.pool().backup_bytes(), 0);
        assert_eq!(renderer.pool().slot("keep").unwrap().len_values, 3);
        assert!(!renderer.is_valid_handle(&old_handle));
        assert!(renderer.gpu_memory_usage().total_bytes() < before);
        assert_eq!(renderer.pool().retired_bytes(), 0);
    }

    #[test]
    fn pool_cleanup_rebinds_active_picker_to_shrunk_primary() {
        let Some(mut renderer) = state_test_renderer_with_capacity(1024 * 1024) else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_scatter("scatter", None, "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let picker_generation = active_picker(&renderer).engine.registry_generation();

        pollster::block_on(renderer.release_unused_pool_memory(16 * 1024)).unwrap();

        assert_eq!(renderer.pool().capacity(), 16 * 1024);
        assert!(active_picker(&renderer).engine.registry_generation() > picker_generation);
        assert!(!renderer.has_pending_maintenance());
    }

    #[test]
    fn pool_cleanup_picker_failure_preserves_original_pool() {
        let Some(mut renderer) = state_test_renderer_with_capacity(1024 * 1024) else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_scatter("scatter", None, "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let handle = renderer.handle_for("x").unwrap();
        active_picker(&renderer).engine.fail_next_registry_prepare();

        assert!(pollster::block_on(renderer.release_unused_pool_memory(16 * 1024)).is_err());

        assert_eq!(renderer.pool().capacity(), 1024 * 1024);
        assert!(renderer.is_valid_handle(&handle));
        assert_eq!(renderer.pool().slot("x").unwrap().len_values, 2);
        assert_eq!(renderer.pool().retired_bytes(), 0);
    }

    #[test]
    fn prepared_view_keeps_panel_charges_after_chart_view_drop() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let chart = Chart::new(panel_config(320, 240));
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let panel_bytes = renderer
            .gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::PanelTexture);
        let series: [Series<'_>; 0] = [];
        let prepared = {
            let items = [ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &series,
            }];
            renderer.prepare(&items).unwrap()
        };
        drop(view);

        let held = renderer.gpu_memory_usage();
        assert_eq!(
            held.live_bytes_of(GpuResourceKind::PanelTexture),
            panel_bytes,
            "the prepared bind groups and their accounting lifetime must agree"
        );
        assert_eq!(held.retired_bytes_of(GpuResourceKind::PanelTexture), 0);

        drop(prepared);
        let retired = renderer.gpu_memory_usage();
        assert_eq!(retired.live_bytes_of(GpuResourceKind::PanelTexture), 0);
        assert_eq!(
            retired.retired_bytes_of(GpuResourceKind::PanelTexture),
            panel_bytes
        );
        renderer.end_gpu_frame();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("retired GPU work completes");
        settle_gpu_retirement(&mut renderer);
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .bytes_of(GpuResourceKind::PanelTexture),
            0
        );
    }

    #[test]
    fn panel_resize_charges_the_replacement_before_releasing_the_old_one() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let mut chart = Chart::new(panel_config(320, 240));
        let mut view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let small = renderer
            .gpu_memory_usage()
            .bytes_of(GpuResourceKind::PanelTexture);
        assert_eq!(small, 320 * 240 * 4 * 2);

        let big_rect = Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        };
        chart.config_mut().chart_area = crate::layout::ChartArea(big_rect);
        renderer.refresh_axis(&mut view, &chart, big_rect).unwrap();

        let during = renderer.gpu_memory_usage();
        let big = 640 * 480 * 4 * 2;
        assert_eq!(
            during.live_bytes_of(GpuResourceKind::PanelTexture),
            big,
            "the live rows are the new textures\n{}",
            during.report()
        );
        assert_eq!(
            during.bytes_of(GpuResourceKind::PanelTexture),
            big + small,
            "old and new coexist until the queue drains"
        );

        renderer.end_gpu_frame();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("retired GPU work completes");
        settle_gpu_retirement(&mut renderer);
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .bytes_of(GpuResourceKind::PanelTexture),
            big
        );
    }

    /// The batch is the matrix upload path, so what it has to deliver is: every
    /// declared column registered (or `validate_renderer_series` rejects the
    /// grid), and one staging buffer for the lot instead of one per column.
    #[test]
    fn a_renderer_batch_registers_every_grid_column_at_one_buffer_each_batch() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let x = col_f64(vec![0.0, 1.0, 2.0]);
        let y = col_f64(vec![0.0, 1.0]);
        let z0 = col_f64(vec![1.0, 2.0]);
        let z1 = col_f64(vec![-3.0, 4.0]);

        let before = renderer.pool.buffer_creations();
        renderer
            .add_columns(&[
                ("x", &x as &dyn ColumnSource),
                ("y", &y as &dyn ColumnSource),
                ("z0", &z0 as &dyn ColumnSource),
                ("z1", &z1 as &dyn ColumnSource),
            ])
            .expect("batch upload");
        assert_eq!(
            renderer.pool.buffer_creations() - before,
            1,
            "four columns, one staging buffer"
        );

        for (id, len) in [("x", 3usize), ("y", 2), ("z0", 2), ("z1", 2)] {
            let slot = renderer.pool.slot(id).expect("batch column is registered");
            assert_eq!(slot.len_values, len, "{id} length");
        }
        assert_eq!(renderer.pool.slot("z1").unwrap().min, -3.0);
        assert_eq!(renderer.pool.slot("z1").unwrap().max, 4.0);

        // The point of registration: the grid declaration now validates.
        let mut config = crate::default::default_config();
        config.colorbar = Some(crate::default::default_colorbar_options());
        renderer
            .register_chart(
                config,
                vec![state_test_heatmap("field", "x", "y", &["z0", "z1"])],
            )
            .expect("a grid over batched columns is drawable");
    }

    /// An empty batch is a no-op, and a repeated id is refused with nothing
    /// uploaded — the same two edges the single-column path has.
    #[test]
    fn a_renderer_batch_rejects_repeated_ids_and_accepts_an_empty_one() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let a = col_f64(vec![1.0, 2.0]);
        renderer.add_columns(&[]).expect("empty batch is a no-op");
        assert_eq!(renderer.pool.used_bytes(), 0);

        assert!(matches!(
            renderer.add_columns(&[
                ("dup", &a as &dyn ColumnSource),
                ("dup", &a as &dyn ColumnSource),
            ]),
            Err(FiggyError::Pool(data_render::AllocError::DuplicateId(_)))
        ));
        assert!(renderer.pool.slot("dup").is_none());
        assert_eq!(renderer.pool.used_bytes(), 0, "nothing was uploaded");

        renderer
            .add_columns(&[("live", &a as &dyn ColumnSource)])
            .expect("first insert");
        let after_first = renderer.pool.used_bytes();
        assert!(matches!(
            renderer.add_columns(&[("live", &a as &dyn ColumnSource)]),
            Err(FiggyError::Pool(data_render::AllocError::DuplicateId(_)))
        ));
        assert_eq!(
            renderer.pool.used_bytes(),
            after_first,
            "the second insert of a live id added nothing"
        );
    }

    /// A batch that has to grow the pool relocates every column, so the active
    /// picker is rebound in the same call and the maintenance bit is cleared —
    /// the same contract the single-column upsert has.
    #[test]
    fn a_renderer_batch_that_grows_the_pool_rebinds_the_active_picker() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        renderer.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_scatter("scatter", None, "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();

        let picker_generation = active_picker(&renderer).engine.registry_generation();
        let old_x = renderer.pool.handle_for("x").unwrap();
        let capacity_before = renderer.pool.capacity();
        let layout_before = renderer.pool.layout_generation();

        let big: Vec<f64> = (0..512).map(|i| i as f64).collect();
        let b0 = col_f64(big.clone());
        let b1 = col_f64(big);
        renderer
            .add_columns(&[
                ("b0", &b0 as &dyn ColumnSource),
                ("b1", &b1 as &dyn ColumnSource),
            ])
            .expect("the batch grew the pool to fit");

        assert!(
            renderer.pool.capacity() > capacity_before,
            "the batch was supposed to grow the pool"
        );
        assert!(renderer.pool.layout_generation() > layout_before);
        assert_ne!(
            renderer.pool.handle_for("x").unwrap().generation,
            old_x.generation,
            "growth republished every handle"
        );
        assert!(
            active_picker(&renderer).engine.registry_generation() > picker_generation,
            "the picker must be rebound against the new layout"
        );
        assert!(!renderer.has_pending_maintenance());
        for id in ["b0", "b1"] {
            assert_eq!(renderer.pool.slot(id).unwrap().len_values, 512, "{id}");
        }
    }

    /// The renderer half of the batch is fallible too. When the picker rebuild
    /// fails, the guard drops: no column is published and the region goes back.
    /// The relayout that made room is *not* undone — it is not part of the batch
    /// — and the picker reports `StaleColumn` until it is re-prepared, which is
    /// the same recovery a defragmentation asks for.
    #[test]
    fn a_renderer_batch_rolled_back_by_a_picker_failure_publishes_no_column() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        renderer.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_scatter("scatter", None, "x", "y")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();

        let picker_generation = active_picker(&renderer).engine.registry_generation();
        let used_before = renderer.pool.used_bytes();
        active_picker(&renderer).engine.fail_next_registry_prepare();

        let big: Vec<f64> = (0..512).map(|i| i as f64).collect();
        let b0 = col_f64(big.clone());
        let b1 = col_f64(big);
        assert!(matches!(
            renderer.add_columns(&[
                ("b0", &b0 as &dyn ColumnSource),
                ("b1", &b1 as &dyn ColumnSource),
            ]),
            Err(FiggyError::GpuPick(
                crate::gpu_pick::GpuPickError::AllocationFailed {
                    resource: "injected registry prepare failure"
                }
            ))
        ));

        assert!(renderer.pool.slot("b0").is_none(), "b0 was published");
        assert!(renderer.pool.slot("b1").is_none(), "b1 was published");
        assert_eq!(
            renderer.pool.used_bytes(),
            used_before,
            "the batch region went back to the free list"
        );
        assert_eq!(
            renderer.pool.free_bytes(),
            renderer.pool.capacity() - used_before,
            "and it coalesced back into the rest of the free space"
        );
        assert_eq!(
            active_picker(&renderer).engine.registry_generation(),
            picker_generation,
            "the failed prepare must not publish a registry"
        );

        // Retrying now succeeds: the region is allocatable again.
        renderer
            .add_columns(&[
                ("b0", &b0 as &dyn ColumnSource),
                ("b1", &b1 as &dyn ColumnSource),
            ])
            .expect("the retry lands where the abandoned batch was");
        assert_eq!(renderer.pool.slot("b1").unwrap().len_values, 512);
    }

    /// The field's group-2 buffers are the grid `(base, len)` table, the levels,
    /// the stops and `FieldParams`. At matrix scale the table alone is tens of
    /// kilobytes, so a steady frame must rebuild **none** of it.
    #[test]
    fn a_steady_field_frame_rebuilds_no_group_two_buffers() {
        let Some((mut renderer, chart, series_cfg)) = field_prepare_fixture() else {
            return;
        };
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let style = renderer.create_style_for_series(&series_cfg).unwrap();
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];

        let first = renderer.prepare(&items).unwrap();
        renderer.validate_prepared(&first).unwrap();
        let after_first = renderer
            .gpu_memory_usage()
            .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable);
        assert!(
            after_first > 0,
            "the first prepare must build the field tables"
        );
        let live = renderer
            .gpu_memory_usage()
            .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable);
        assert!(live > 0, "and charge them");
        assert_eq!(renderer.field_cache.len(), 1);

        let second = renderer.prepare(&items).unwrap();
        renderer.validate_prepared(&second).unwrap();
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            after_first,
            "nothing changed, so nothing may be rebuilt"
        );
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            live,
            "and the charge must not double"
        );
    }

    /// Replacing a grid column both stales the live token and rebuilds the
    /// scratch on the next prepare. The token check is the point: the captured
    /// `rows` came from that column's length, so drawing on with it would paint a
    /// grid the data no longer has.
    #[test]
    fn a_replaced_grid_column_stales_the_token_and_rebuilds_the_scratch() {
        let Some((mut renderer, chart, series_cfg)) = field_prepare_fixture() else {
            return;
        };
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let style = renderer.create_style_for_series(&series_cfg).unwrap();
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];

        let token = renderer.prepare(&items).unwrap();
        renderer.validate_prepared(&token).unwrap();
        let creations_before = renderer
            .gpu_memory_usage()
            .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable);

        // Same id, same length, new bytes: the layout does not move, so only the
        // per-column allocation epoch marks the change.
        renderer
            .upsert_column("fz0", &col_f64(vec![7.0, 8.0]))
            .unwrap();
        let stale = renderer
            .validate_prepared(&token)
            .expect_err("a replaced grid column must stale the token");
        match &stale {
            FiggyError::StalePreparedFrame { reason } => {
                assert!(
                    reason.contains("grid column"),
                    "expected the grid column to be named: {reason}"
                );
            }
            other => panic!("expected StalePreparedFrame, got {other:?}"),
        }

        let next = renderer.prepare(&items).unwrap();
        renderer.validate_prepared(&next).unwrap();
        assert!(
            renderer
                .gpu_memory_usage()
                .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable)
                > creations_before,
            "the scratch must be rebuilt against the new epoch"
        );
    }

    /// A chart, a colourbar and a two-column grid, ready to prepare.
    fn field_prepare_fixture() -> Option<(Renderer, Chart, SeriesConfig)> {
        let mut renderer = state_test_renderer()?;
        renderer.add_column("fx", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("fy", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer
            .add_column("fz0", &col_f64(vec![0.0, 1.0]))
            .unwrap();
        renderer
            .add_column("fz1", &col_f64(vec![2.0, 3.0]))
            .unwrap();
        let mut config = panel_config(320, 240);
        config.colorbar = Some(crate::default::default_colorbar_options());
        Some((
            renderer,
            Chart::new(config),
            state_test_heatmap("field", "fx", "fy", &["fz0", "fz1"]),
        ))
    }

    #[test]
    fn a_growth_makes_a_live_prepared_frame_stale_and_the_next_one_valid() {
        // Small pool so one oversized upload has to relayout.
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        renderer.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
        renderer.add_column("lx", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("ly", &col_f64(vec![0.0, 1.0])).unwrap();
        let chart = Chart::new(panel_config(320, 240));
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let series_cfg = state_test_line("line", "lx", "ly");
        let style = renderer.create_style_for_series(&series_cfg).unwrap();
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        let prepared = renderer.prepare(&items).unwrap();
        renderer
            .validate_prepared(&prepared)
            .expect("a fresh token is valid");
        let capacity_before = renderer.pool.capacity();

        // Force a growth: this relayouts the slab, which moves every column and
        // advances the pool's layout generation.
        let big: Vec<f64> = (0..2048).map(|i| i as f64).collect();
        renderer.add_column("big", &col_f64(big)).unwrap();
        assert!(
            renderer.pool.capacity() > capacity_before,
            "the upload was supposed to grow the pool"
        );

        // The token prepared against the old layout must be refused rather than
        // drawn: its vertex offsets point into a buffer that no longer exists.
        let stale = renderer
            .validate_prepared(&prepared)
            .expect_err("a token prepared before a growth must be stale");
        assert!(
            matches!(stale, FiggyError::StalePreparedFrame { .. }),
            "expected StalePreparedFrame, got {stale:?}"
        );

        // Recovery is a fresh prepare, exactly as it is after a defragmentation.
        let reprepared = renderer.prepare(&items).unwrap();
        renderer
            .validate_prepared(&reprepared)
            .expect("the next frame prepares normally after a growth");
    }

    #[test]
    fn the_report_tracks_the_pool_across_a_growth() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        renderer.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
        renderer.add_column("lx", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.end_gpu_frame();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("retired GPU work completes");
        settle_gpu_retirement(&mut renderer);

        let before = renderer.gpu_memory_usage();
        assert_eq!(before.pool_bytes(), pool_bytes_held(&renderer));
        let capacity_before = renderer.pool.capacity();

        let big: Vec<f64> = (0..2048).map(|i| i as f64).collect();
        renderer.add_column("big", &col_f64(big)).unwrap();
        let capacity_after = renderer.pool.capacity();
        assert!(capacity_after > capacity_before, "growth expected");

        // Immediately after the growth the old slab is retired but not yet
        // counted as gone, so the row is new + old.
        let during = renderer.gpu_memory_usage();
        assert_eq!(
            during.live_bytes_of(GpuResourceKind::ColumnPool),
            pool_bytes_held(&renderer),
            "live pool bytes must equal the buffers held\n{}",
            during.report()
        );
        assert_eq!(
            during.retired_bytes_of(GpuResourceKind::ColumnPool),
            capacity_before + 2048 * crate::data::COLUMN_VALUE_BYTES as u64,
            "the old slab and submitted staging stay retired until completion\n{}",
            during.report()
        );
        assert!(
            during.peak_bytes() >= capacity_before + capacity_after,
            "the peak must remember that both slabs existed across the copy: \
             {} < {} + {}\n{}",
            during.peak_bytes(),
            capacity_before,
            capacity_after,
            during.report()
        );

        renderer.end_gpu_frame();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("retired GPU work completes");
        settle_gpu_retirement(&mut renderer);
        let after = renderer.gpu_memory_usage();
        assert_eq!(after.retired_bytes(), 0);
        assert_eq!(after.pool_bytes(), pool_bytes_held(&renderer));
        assert_eq!(after.pool_bytes(), capacity_after);
    }

    #[test]
    fn an_undeclared_ceiling_carries_no_out_of_pool_total() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let chart = Chart::new(panel_config(320, 240));
        let _view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        renderer.end_gpu_frame();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("retired GPU work completes");
        settle_gpu_retirement(&mut renderer);

        // No ceiling: the context carries no budget at all, so there is no
        // out-of-pool total to compute and none to misread. `GpuBudget` fusing
        // the two is what makes that a type-level fact rather than a comment.
        let ctx = pool_alloc_ctx(
            renderer.device.as_ref(),
            renderer.queue.as_ref(),
            renderer.memory_budget,
            renderer.gpu_ledger.as_ref(),
        );
        assert!(
            ctx.budget.is_none(),
            "with no ceiling declared the pool must be handed no budget"
        );

        // With a ceiling, the total rides along and matches what the report
        // says is held outside the pool right now.
        let external = renderer.gpu_memory_usage().external_bytes();
        assert!(external > 0, "the panel textures should be charged by now");
        assert_eq!(renderer.set_memory_budget(Some(u64::MAX)), None);
        let ctx = pool_alloc_ctx(
            renderer.device.as_ref(),
            renderer.queue.as_ref(),
            renderer.memory_budget,
            renderer.gpu_ledger.as_ref(),
        );
        let budget = ctx.budget.expect("a declared ceiling reaches the pool");
        assert_eq!(budget.ceiling_bytes, u64::MAX);
        assert_eq!(
            budget.external_bytes, external,
            "the out-of-pool total must be the one read at this call"
        );
    }

    #[test]
    fn lowering_the_ceiling_below_the_current_total_reports_it_at_the_call() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let chart = Chart::new(panel_config(320, 240));
        let _view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        renderer.end_gpu_frame();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("retired GPU work completes");
        settle_gpu_retirement(&mut renderer);
        let total = renderer.gpu_memory_usage().total_bytes();

        // A ceiling with room says nothing.
        assert_eq!(renderer.set_memory_budget(Some(total + 1)), None);

        // A ceiling already crossed is still applied — a host lowering it on
        // purpose, about to release something, gets what it asked for — but it
        // hears about the conflict here instead of at some later growth.
        assert_eq!(
            renderer.set_memory_budget(Some(total - 1)),
            Some(total),
            "the setter must report the total that already exceeds the ceiling"
        );
        assert_eq!(
            renderer.memory_budget(),
            Some(total - 1),
            "and it must still have been applied"
        );

        assert_eq!(renderer.set_memory_budget(None), None);
        assert_eq!(renderer.memory_budget(), None);
    }

    #[test]
    fn resident_admission_counts_staging_derived_headroom_and_requires_a_budget() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let lengths = [1, 33]; // 256 + 512 bytes after per-column alignment.
        let derived = [128, 192];
        let request = ResidentAdmissionRequest {
            column_value_counts: &lengths,
            already_resident_working_set_bytes: 256,
            derived_buffer_sizes: &derived,
            transition_headroom_bytes: 128,
            working_set_limit_bytes: 1344,
        };
        let current = renderer.gpu_memory_usage().total_bytes();
        let unbounded = renderer.resident_admission(request);
        assert_eq!(unbounded.status, ResidentAdmissionStatus::MemoryBudgetUnset);
        assert_eq!(unbounded.resident_column_bytes, 768);
        assert_eq!(unbounded.derived_resident_bytes, 320);
        assert_eq!(unbounded.upload_staging_bytes, 768);
        assert_eq!(unbounded.resident_working_set_bytes, 1344);
        assert_eq!(unbounded.pool_transition_bytes, 0);
        assert_eq!(unbounded.transition_peak_bytes, current + 768 + 320 + 128);

        assert_eq!(
            renderer.set_memory_budget(Some(unbounded.transition_peak_bytes)),
            None
        );
        let accepted = renderer.resident_admission(request);
        assert!(accepted.is_admissible(), "{accepted:?}");

        assert_eq!(
            renderer.set_memory_budget(Some(accepted.transition_peak_bytes - 1)),
            None
        );
        assert_eq!(
            renderer.resident_admission(request).status,
            ResidentAdmissionStatus::MemoryBudgetExceeded
        );
        let capped = renderer.resident_admission(ResidentAdmissionRequest {
            working_set_limit_bytes: 1343,
            ..request
        });
        assert_eq!(
            capped.status,
            ResidentAdmissionStatus::WorkingSetLimitExceeded
        );
    }

    #[test]
    fn streamed_promotion_commits_mixed_encodings_and_only_invalidates_dependents() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let _ = renderer.set_memory_budget(Some(u64::MAX));
        let scalar = col_f64(vec![-2.0, 0.5, 4.0]);
        let hilo = col_f64(vec![
            1_700_000_000_000.125,
            1_700_000_000_000.25,
            1_700_000_000_000.5,
        ]);
        renderer
            .register_streamed_columns(vec![
                crate::StreamColumn {
                    id: "promote-x".into(),
                    len: 3,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    revision: 7,
                    statistics: crate::StreamStatistics::Unknown,
                },
                crate::StreamColumn {
                    id: "promote-y".into(),
                    len: 3,
                    encoding: crate::StreamEncoding::HiLoF32,
                    replay: crate::StreamReplay::RandomAccess,
                    revision: 9,
                    statistics: crate::StreamStatistics::Unknown,
                },
            ])
            .unwrap();
        let series = state_test_line("promoted", "promote-x", "promote-y");
        let dependent = renderer
            .register_chart(crate::default::default_config(), vec![series.clone()])
            .unwrap();
        let unrelated = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        let dependent_before = revision_snapshot(&renderer, dependent);
        let unrelated_before = revision_snapshot(&renderer, unrelated);
        let fit_before = renderer.begin_fit_commit(dependent).unwrap();
        let sources = [
            crate::StreamSourceBinding {
                id: "promote-x",
                revision: 7,
                source: crate::data::StreamColumnSource::Scalar(&scalar),
            },
            crate::StreamSourceBinding {
                id: "promote-y",
                revision: 9,
                source: crate::data::StreamColumnSource::HiLo(&hilo),
            },
        ];

        let report = renderer
            .promote_streamed_columns_to_resident(&sources, 0, &[], 0, u64::MAX)
            .unwrap();
        assert!(report.is_admissible(), "{report:?}");
        assert!(matches!(
            renderer.logical_column("promote-x"),
            Some(crate::LogicalColumn::Resident(_))
        ));
        assert!(matches!(
            renderer.logical_column("promote-y"),
            Some(crate::LogicalColumn::Resident(_))
        ));
        assert!(!renderer.streaming_sources.contains_key("promote-x"));
        assert!(!renderer.streaming_sources.contains_key("promote-y"));
        assert_eq!(renderer.stream_revision_lineage["promote-x"], 7);
        assert_eq!(renderer.stream_revision_lineage["promote-y"], 9);
        assert_eq!(renderer.chart_states[&dependent].series, vec![series]);
        let dependent_after = revision_snapshot(&renderer, dependent);
        assert_ne!(dependent_after[0], dependent_before[0]);
        assert_ne!(dependent_after[3], dependent_before[3]);
        assert_eq!(dependent_after[1], dependent_before[1]);
        assert_eq!(dependent_after[2], dependent_before[2]);
        assert_eq!(revision_snapshot(&renderer, unrelated), unrelated_before);
        assert_eq!(renderer.pool.slot("promote-x").unwrap().min, -2.0);
        assert_eq!(
            renderer.pool.slot("promote-y").unwrap().min_positive,
            Some(1_700_000_000_000.125)
        );
        assert!(matches!(
            renderer.commit_auto_fit_all_if_current(
                &fit_before,
                &crate::chart::FitExtent::EMPTY,
                &crate::chart::FitExtent::EMPTY,
                0.0,
            ),
            Err(FiggyError::StaleStateToken { .. })
        ));
    }

    #[test]
    fn streamed_promotion_validation_admission_and_source_failure_leave_stream_authority() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let source = FailingPromotionSource { len: 3 };
        renderer
            .register_streamed_columns(vec![crate::StreamColumn {
                id: "stay-streamed".into(),
                len: 3,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 4,
                statistics: crate::StreamStatistics::Unknown,
            }])
            .unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line(
                    "streamed",
                    "stay-streamed",
                    "stay-streamed",
                )],
            )
            .unwrap();
        let revisions = revision_snapshot(&renderer, chart);
        let used = renderer.pool.used_bytes();
        let layout = renderer.pool.layout_generation();
        let stale = [crate::StreamSourceBinding {
            id: "stay-streamed",
            revision: 3,
            source: crate::data::StreamColumnSource::Scalar(&source),
        }];
        assert!(matches!(
            renderer.resident_admission_for_streamed_columns(&stale, 0, &[], 0, u64::MAX),
            Err(FiggyError::InvalidStreamSource { .. })
        ));
        let short = FailingPromotionSource { len: 2 };
        let wrong_len = [crate::StreamSourceBinding {
            id: "stay-streamed",
            revision: 4,
            source: crate::data::StreamColumnSource::Scalar(&short),
        }];
        assert!(matches!(
            renderer.resident_admission_for_streamed_columns(&wrong_len, 0, &[], 0, u64::MAX),
            Err(FiggyError::InvalidStreamSource { .. })
        ));
        let wrong_encoding_values = col_f64(vec![1.0, 2.0, 3.0]);
        let wrong_encoding = [crate::StreamSourceBinding {
            id: "stay-streamed",
            revision: 4,
            source: crate::data::StreamColumnSource::HiLo(&wrong_encoding_values),
        }];
        assert!(matches!(
            renderer.resident_admission_for_streamed_columns(&wrong_encoding, 0, &[], 0, u64::MAX,),
            Err(FiggyError::InvalidStreamSource { .. })
        ));

        let current = renderer.gpu_memory_usage().checked_total_bytes().unwrap();
        let _ = renderer.set_memory_budget(Some(current));
        let exact = [crate::StreamSourceBinding {
            revision: 4,
            ..stale[0]
        }];
        let duplicate = [exact[0], exact[0]];
        assert!(matches!(
            renderer.resident_admission_for_streamed_columns(&duplicate, 0, &[], 0, u64::MAX),
            Err(FiggyError::InvalidStreamSource { .. })
        ));
        let rejected = renderer
            .promote_streamed_columns_to_resident(&exact, 0, &[], 0, u64::MAX)
            .unwrap();
        assert_eq!(
            rejected.status,
            ResidentAdmissionStatus::MemoryBudgetExceeded
        );
        let _ = renderer.set_memory_budget(Some(u64::MAX));
        assert!(matches!(
            renderer.promote_streamed_columns_to_resident(&exact, 0, &[], 0, u64::MAX),
            Err(FiggyError::Pool(
                data_render::AllocError::AllocationFailed {
                    resource: "streamed promotion source",
                    ..
                }
            ))
        ));
        assert!(matches!(
            renderer.logical_column("stay-streamed"),
            Some(crate::LogicalColumn::Streamed(_))
        ));
        assert!(renderer.pool.slot("stay-streamed").is_none());
        assert_eq!(renderer.stream_revision_lineage["stay-streamed"], 4);
        assert_eq!(renderer.pool.used_bytes(), used);
        assert_eq!(renderer.pool.layout_generation(), layout);
        assert_eq!(revision_snapshot(&renderer, chart), revisions);
    }

    #[test]
    fn resident_demotion_commits_mixed_metadata_and_preserves_chart_authority() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let demote_x = col_f64(vec![1.0, 2.0, 3.0]);
        let demote_y = col_f64(vec![
            1_700_000_000_000.0,
            1_700_000_000_000.25,
            1_700_000_000_000.5,
        ]);
        renderer.add_column("demote-x", &demote_x).unwrap();
        renderer.add_hilo_column("demote-y", &demote_y).unwrap();
        renderer
            .add_column("resident-z", &col_f64(vec![4.0, 5.0, 6.0]))
            .unwrap();
        let config = crate::default::default_config();
        let series = state_test_line("demoted", "demote-x", "demote-y");
        let dependent = renderer
            .register_chart(config.clone(), vec![series.clone()])
            .unwrap();
        let unrelated = renderer
            .register_chart(
                config.clone(),
                vec![state_test_line("resident", "resident-z", "resident-z")],
            )
            .unwrap();
        let dependent_before = revision_snapshot(&renderer, dependent);
        let unrelated_before = revision_snapshot(&renderer, unrelated);
        let fit_before = renderer.begin_fit_commit(dependent).unwrap();
        let chart = Chart::new(config.clone());
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let prepared = renderer
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: dependent,
                view: &view,
            }])
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(dependent).unwrap();
        assert_eq!(active_picker(&renderer).chart_id, dependent);

        let sources = [
            crate::StreamSourceBinding {
                id: "demote-x",
                revision: 31,
                source: crate::StreamColumnSource::Scalar(&demote_x),
            },
            crate::StreamSourceBinding {
                id: "demote-y",
                revision: 47,
                source: crate::StreamColumnSource::HiLo(&demote_y),
            },
        ];
        renderer
            .demote_resident_columns_to_streamed(
                &["demote-x"],
                vec![
                    crate::StreamColumn {
                        id: "demote-x".into(),
                        len: 3,
                        encoding: crate::StreamEncoding::ScalarF32,
                        replay: crate::StreamReplay::RandomAccess,
                        revision: 31,
                        statistics: crate::StreamStatistics::Unknown,
                    },
                    crate::StreamColumn {
                        id: "demote-y".into(),
                        len: 3,
                        encoding: crate::StreamEncoding::HiLoF32,
                        replay: crate::StreamReplay::Sequential,
                        revision: 47,
                        statistics: crate::StreamStatistics::Unknown,
                    },
                ],
                &sources,
            )
            .unwrap();

        assert!(matches!(
            renderer.logical_column("demote-x"),
            Some(crate::LogicalColumn::Streamed(source))
                if source.revision == 31
                    && source.encoding == crate::StreamEncoding::ScalarF32
        ));
        assert!(matches!(
            renderer.logical_column("demote-y"),
            Some(crate::LogicalColumn::Streamed(source))
                if source.revision == 47
                    && source.encoding == crate::StreamEncoding::HiLoF32
        ));
        assert!(renderer.pool.slot("demote-x").is_none());
        assert!(renderer.pool.slot("demote-y").is_none());
        assert!(renderer.pool.slot("resident-z").is_some());
        assert_eq!(renderer.stream_revision_lineage["demote-x"], 31);
        assert_eq!(renderer.stream_revision_lineage["demote-y"], 47);
        assert_eq!(renderer.chart_config(dependent).unwrap(), &config);
        assert_eq!(renderer.chart_series(dependent).unwrap(), &[series]);
        let dependent_after = revision_snapshot(&renderer, dependent);
        assert_ne!(dependent_after[0], dependent_before[0]);
        assert_ne!(dependent_after[3], dependent_before[3]);
        assert_eq!(dependent_after[1], dependent_before[1]);
        assert_eq!(dependent_after[2], dependent_before[2]);
        assert_eq!(revision_snapshot(&renderer, unrelated), unrelated_before);
        assert!(renderer.picker.active.is_none());
        assert!(matches!(
            renderer.validate_prepared(&prepared),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
        assert!(matches!(
            renderer.commit_auto_fit_all_if_current(
                &fit_before,
                &crate::FitExtent::EMPTY,
                &crate::FitExtent::EMPTY,
                0.0,
            ),
            Err(FiggyError::StaleStateToken { .. })
        ));
    }

    #[test]
    fn resident_demotion_rejects_the_whole_batch_before_any_authority_changes() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let resident_a = col_f64(vec![1.0, 2.0]);
        let resident_b = col_f64(vec![1.0, 2.0]);
        renderer.add_column("resident-a", &resident_a).unwrap();
        renderer.add_column("resident-b", &resident_b).unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("resident", "resident-a", "resident-b")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let revisions = revision_snapshot(&renderer, chart);
        let generation = renderer.pool.generation();
        let used = renderer.pool.used_bytes();
        let stream_count = renderer.streaming_sources.len();
        let fit_epoch = renderer.next_stream_source_fit_epoch;
        let lineage = renderer.stream_revision_lineage.clone();

        let metadata = |id: &str, len: u64| crate::StreamColumn {
            id: id.into(),
            len,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 5,
            statistics: crate::StreamStatistics::Unknown,
        };
        let sources = [
            crate::StreamSourceBinding {
                id: "resident-a",
                revision: 5,
                source: crate::StreamColumnSource::Scalar(&resident_a),
            },
            crate::StreamSourceBinding {
                id: "resident-b",
                revision: 5,
                source: crate::StreamColumnSource::Scalar(&resident_b),
            },
        ];
        assert!(matches!(
            renderer.demote_resident_columns_to_streamed(
                &["resident-a"],
                vec![metadata("resident-a", 2), metadata("resident-a", 2)],
                &sources,
            ),
            Err(FiggyError::Pool(data_render::AllocError::DuplicateId(_)))
        ));
        let short = col_f64(vec![1.0]);
        let wrong_companion = [
            sources[0],
            crate::StreamSourceBinding {
                id: "resident-b",
                revision: 5,
                source: crate::StreamColumnSource::Scalar(&short),
            },
        ];
        assert!(matches!(
            renderer.demote_resident_columns_to_streamed(
                &["resident-a"],
                vec![metadata("resident-a", 2), metadata("resident-b", 1)],
                &wrong_companion,
            ),
            Err(FiggyError::InvalidStreamSource { .. })
        ));
        let mut known = metadata("resident-a", 2);
        known.statistics = crate::StreamStatistics::Known(Some(crate::StreamBounds {
            min: 1.0,
            max: 2.0,
            min_positive: Some(1.0),
        }));
        assert!(matches!(
            renderer.demote_resident_columns_to_streamed(
                &["resident-a"],
                vec![known, metadata("resident-b", 2)],
                &sources,
            ),
            Err(FiggyError::InvalidStreamSource { .. })
        ));
        let missing_source = [crate::StreamSourceBinding {
            id: "missing",
            revision: 5,
            source: crate::StreamColumnSource::Scalar(&resident_a),
        }];
        assert!(matches!(
            renderer.demote_resident_columns_to_streamed(
                &["missing"],
                vec![metadata("missing", 2)],
                &missing_source,
            ),
            Err(FiggyError::UnknownColumn { .. })
        ));

        let old_desired = renderer.chart_states[&chart].revisions.desired;
        renderer
            .chart_states
            .get_mut(&chart)
            .unwrap()
            .revisions
            .desired = RenderRevision {
            renderer_identity: renderer.renderer_identity,
            sequence: u64::MAX,
        };
        assert!(matches!(
            renderer.demote_resident_columns_to_streamed(
                &["resident-a"],
                vec![metadata("resident-a", 2), metadata("resident-b", 2)],
                &sources,
            ),
            Err(FiggyError::CounterExhausted { .. })
        ));
        renderer
            .chart_states
            .get_mut(&chart)
            .unwrap()
            .revisions
            .desired = old_desired;

        assert_eq!(renderer.pool.generation(), generation);
        assert_eq!(renderer.pool.used_bytes(), used);
        assert!(renderer.pool.slot("resident-a").is_some());
        assert!(renderer.pool.slot("resident-b").is_some());
        assert_eq!(renderer.streaming_sources.len(), stream_count);
        assert_eq!(renderer.stream_revision_lineage, lineage);
        assert_eq!(renderer.next_stream_source_fit_epoch, fit_epoch);
        assert_eq!(revision_snapshot(&renderer, chart), revisions);
        assert_eq!(active_picker(&renderer).chart_id, chart);
    }

    #[test]
    fn resident_demotion_closes_over_shared_charts_and_accepts_new_root_length() {
        let Some(mut renderer) = state_test_renderer_with_capacity(8 * 1024) else {
            return;
        };
        let old_a = col_f64(vec![1.0, 2.0]);
        let new_a = col_f64(vec![1.0, 2.0, 3.0]);
        let b = col_f64(vec![4.0, 5.0]);
        let c = col_f64(vec![6.0, 7.0]);
        let d = col_f64(vec![8.0, 9.0]);
        for (id, source) in [
            ("closure-a", &old_a),
            ("closure-b", &b),
            ("closure-c", &c),
            ("closure-d", &d),
        ] {
            renderer.add_column(id, source).unwrap();
        }
        let first = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("first", "closure-a", "closure-b")],
            )
            .unwrap();
        let shared = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("shared", "closure-b", "closure-c")],
            )
            .unwrap();
        let unrelated = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("unrelated", "closure-d", "closure-d")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(shared).unwrap();

        let metadata = |id: &str, len: u64| crate::StreamColumn {
            id: id.into(),
            len,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 1,
            statistics: crate::StreamStatistics::Unknown,
        };
        let sources = [
            crate::StreamSourceBinding {
                id: "closure-a",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&new_a),
            },
            crate::StreamSourceBinding {
                id: "closure-b",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&b),
            },
            crate::StreamSourceBinding {
                id: "closure-c",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&c),
            },
        ];
        let first_before = revision_snapshot(&renderer, first);
        let shared_before = revision_snapshot(&renderer, shared);
        let unrelated_before = revision_snapshot(&renderer, unrelated);
        let generation = renderer.pool.generation();
        let used = renderer.pool.used_bytes();
        assert!(matches!(
            renderer.demote_resident_columns_to_streamed(
                &["closure-a"],
                vec![metadata("closure-a", 3), metadata("closure-b", 2)],
                &sources[..2],
            ),
            Err(FiggyError::InvalidStreamSource { .. })
        ));
        assert_eq!(renderer.pool.generation(), generation);
        assert_eq!(renderer.pool.used_bytes(), used);
        assert!(renderer.streaming_sources.is_empty());
        assert!(renderer.stream_revision_lineage.is_empty());
        assert_eq!(revision_snapshot(&renderer, first), first_before);
        assert_eq!(revision_snapshot(&renderer, shared), shared_before);
        assert_eq!(active_picker(&renderer).chart_id, shared);

        renderer
            .demote_resident_columns_to_streamed(
                &["closure-a"],
                vec![
                    metadata("closure-a", 3),
                    metadata("closure-b", 2),
                    metadata("closure-c", 2),
                ],
                &sources,
            )
            .unwrap();
        for id in ["closure-a", "closure-b", "closure-c"] {
            assert!(renderer.pool.slot(id).is_none());
            assert!(renderer.streaming_sources.contains_key(id));
        }
        assert_eq!(renderer.streaming_sources["closure-a"].len, 3);
        assert!(renderer.pool.slot("closure-d").is_some());
        for (before, after) in [
            (first_before, revision_snapshot(&renderer, first)),
            (shared_before, revision_snapshot(&renderer, shared)),
        ] {
            assert_eq!(after[0].sequence, before[0].sequence + 1);
            assert_eq!(after[3].sequence, before[3].sequence + 1);
            assert_eq!(after[1], before[1]);
            assert_eq!(after[2], before[2]);
            assert_eq!(after[4], before[4]);
            assert_eq!(after[5], before[5]);
            assert_eq!(after[6], before[6]);
        }
        assert_eq!(revision_snapshot(&renderer, unrelated), unrelated_before);
        assert!(renderer.picker.active.is_none());
        assert!(renderer.pending_defrag);

        let first_demoted = revision_snapshot(&renderer, first);
        let shared_demoted = revision_snapshot(&renderer, shared);
        let _ = renderer.set_memory_budget(Some(u64::MAX));
        assert!(matches!(
            renderer.promote_streamed_columns_to_resident(&sources[..1], 0, &[], 0, u64::MAX,),
            Err(FiggyError::InvalidStreamSource { .. })
        ));
        for id in ["closure-a", "closure-b", "closure-c"] {
            assert!(renderer.pool.slot(id).is_none());
            assert!(renderer.streaming_sources.contains_key(id));
        }
        let report = renderer
            .promote_streamed_columns_to_resident(&sources, 0, &[], 0, u64::MAX)
            .unwrap();
        assert!(report.is_admissible(), "{report:?}");
        for id in ["closure-a", "closure-b", "closure-c"] {
            assert!(renderer.pool.slot(id).is_some());
            assert!(!renderer.streaming_sources.contains_key(id));
            assert_eq!(renderer.stream_revision_lineage[id], 1);
        }
        for (before, after) in [
            (first_demoted, revision_snapshot(&renderer, first)),
            (shared_demoted, revision_snapshot(&renderer, shared)),
        ] {
            assert_eq!(after[0].sequence, before[0].sequence + 1);
            assert_eq!(after[3].sequence, before[3].sequence + 1);
            assert_eq!(after[1], before[1]);
            assert_eq!(after[2], before[2]);
            assert_eq!(after[4], before[4]);
            assert_eq!(after[5], before[5]);
            assert_eq!(after[6], before[6]);
        }
        assert_eq!(revision_snapshot(&renderer, unrelated), unrelated_before);
    }

    #[test]
    fn stream_revision_lineage_survives_promotion_and_true_remove_clears_it() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let values = col_f64(vec![1.0, 2.0]);
        let metadata = |revision| crate::StreamColumn {
            id: "lineage".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision,
            statistics: crate::StreamStatistics::Unknown,
        };
        renderer
            .register_streamed_columns(vec![metadata(5)])
            .unwrap();
        assert_eq!(renderer.stream_revision_lineage["lineage"], 5);
        let binding = |revision| crate::StreamSourceBinding {
            id: "lineage",
            revision,
            source: crate::StreamColumnSource::Scalar(&values),
        };
        let _ = renderer.set_memory_budget(Some(u64::MAX));
        renderer
            .promote_streamed_columns_to_resident(&[binding(5)], 0, &[], 0, u64::MAX)
            .unwrap();
        assert_eq!(renderer.stream_revision_lineage["lineage"], 5);
        assert!(matches!(
            renderer.demote_resident_columns_to_streamed(
                &["lineage"],
                vec![metadata(5)],
                &[binding(5)],
            ),
            Err(FiggyError::InvalidStreamSource { .. })
        ));
        assert!(renderer.pool.slot("lineage").is_some());
        assert!(!renderer.streaming_sources.contains_key("lineage"));
        assert_eq!(renderer.stream_revision_lineage["lineage"], 5);

        renderer
            .demote_resident_columns_to_streamed(&["lineage"], vec![metadata(6)], &[binding(6)])
            .unwrap();
        renderer
            .replace_streamed_columns(vec![metadata(7)])
            .unwrap();
        assert_eq!(renderer.stream_revision_lineage["lineage"], 7);
        assert!(renderer.remove_column("lineage").unwrap());
        assert!(!renderer.stream_revision_lineage.contains_key("lineage"));

        renderer.add_column("native-zero", &values).unwrap();
        let zero = crate::StreamColumn {
            id: "native-zero".into(),
            revision: 0,
            ..metadata(1)
        };
        let zero_binding = crate::StreamSourceBinding {
            id: "native-zero",
            revision: 0,
            source: crate::StreamColumnSource::Scalar(&values),
        };
        assert!(matches!(
            renderer.demote_resident_columns_to_streamed(
                &["native-zero"],
                vec![zero],
                &[zero_binding],
            ),
            Err(FiggyError::InvalidStreamSource { .. })
        ));
        assert!(renderer.pool.slot("native-zero").is_some());
    }

    #[test]
    fn resident_demotion_normalizes_a_mixed_chart_and_preserves_streamed_companion() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let resident = col_f64(vec![1.0, 2.0]);
        let streamed = col_f64(vec![3.0, 4.0]);
        renderer.add_column("mixed-resident", &resident).unwrap();
        renderer
            .register_streamed_columns(vec![crate::StreamColumn {
                id: "mixed-streamed".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 1,
                statistics: crate::StreamStatistics::Unknown,
            }])
            .unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("mixed", "mixed-resident", "mixed-streamed")],
            )
            .unwrap();
        let revisions = revision_snapshot(&renderer, chart);
        let streamed_fit_epoch = renderer.streaming_sources["mixed-streamed"].fit_epoch;
        let resident_metadata = crate::StreamColumn {
            id: "mixed-resident".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 1,
            statistics: crate::StreamStatistics::Unknown,
        };
        let streamed_metadata = crate::StreamColumn {
            id: "mixed-streamed".into(),
            ..resident_metadata.clone()
        };
        let bindings = [
            crate::StreamSourceBinding {
                id: "mixed-resident",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&resident),
            },
            crate::StreamSourceBinding {
                id: "mixed-streamed",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&streamed),
            },
        ];
        renderer
            .demote_resident_columns_to_streamed(
                &["mixed-resident"],
                vec![resident_metadata, streamed_metadata],
                &bindings,
            )
            .unwrap();
        assert!(renderer.pool.slot("mixed-resident").is_none());
        assert_eq!(renderer.streaming_sources["mixed-resident"].revision, 1);
        assert!(renderer.streaming_sources.contains_key("mixed-streamed"));
        assert_eq!(
            renderer.streaming_sources["mixed-streamed"].fit_epoch,
            streamed_fit_epoch
        );
        assert_eq!(renderer.stream_revision_lineage["mixed-resident"], 1);
        assert_eq!(renderer.stream_revision_lineage["mixed-streamed"], 1);
        let after = revision_snapshot(&renderer, chart);
        assert_eq!(after[0].sequence, revisions[0].sequence + 1);
        assert_eq!(after[3].sequence, revisions[3].sequence + 1);
        for index in [1, 2, 4, 5, 6] {
            assert_eq!(after[index], revisions[index]);
        }
    }

    #[test]
    fn mixed_normalization_closes_three_shared_charts_and_preserves_caches_and_picker() {
        let Some(mut renderer) = state_test_renderer_with_capacity(8 * 1024) else {
            return;
        };
        let a = col_f64(vec![1.0, 2.0]);
        let b = col_f64(vec![3.0, 4.0]);
        let c = col_f64(vec![5.0, 6.0]);
        let d = col_f64(vec![7.0, 8.0]);
        let picker_values = col_f64(vec![9.0, 10.0]);
        renderer.add_column("graph-a", &a).unwrap();
        renderer.add_column("graph-c", &c).unwrap();
        renderer.add_column("graph-picker", &picker_values).unwrap();
        renderer
            .register_streamed_columns(vec![
                crate::StreamColumn {
                    id: "graph-b".into(),
                    len: 2,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    revision: 10,
                    statistics: crate::StreamStatistics::Unknown,
                },
                crate::StreamColumn {
                    id: "graph-d".into(),
                    len: 2,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::Sequential,
                    revision: 20,
                    statistics: crate::StreamStatistics::Unknown,
                },
            ])
            .unwrap();
        let chart_ab = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("ab", "graph-a", "graph-b")],
            )
            .unwrap();
        let chart_bc = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("bc", "graph-b", "graph-c")],
            )
            .unwrap();
        let chart_cd = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("cd", "graph-c", "graph-d")],
            )
            .unwrap();
        let picker_chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("picker", "graph-picker", "graph-picker")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer
            .prepare_gpu_picking_for_chart(picker_chart)
            .unwrap();
        renderer
            .streaming_sources
            .get_mut("graph-b")
            .unwrap()
            .statistics_cache
            .covered
            .push(0..1);
        renderer
            .streaming_sources
            .get_mut("graph-d")
            .unwrap()
            .statistics_cache
            .covered
            .push(1..2);

        let chart_snapshots = [chart_ab, chart_bc, chart_cd].map(|chart| {
            (
                chart,
                renderer.chart_config(chart).unwrap().clone(),
                renderer.chart_series(chart).unwrap().to_vec(),
                revision_snapshot(&renderer, chart),
            )
        });
        let picker_revisions = revision_snapshot(&renderer, picker_chart);
        let b_fit = renderer.streaming_sources["graph-b"].fit_epoch;
        let d_fit = renderer.streaming_sources["graph-d"].fit_epoch;
        let metadata = vec![
            crate::StreamColumn {
                id: "graph-a".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 1,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "graph-b".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 10,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "graph-c".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 1,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "graph-d".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::Sequential,
                revision: 20,
                statistics: crate::StreamStatistics::Unknown,
            },
        ];
        let bindings = [
            crate::StreamSourceBinding {
                id: "graph-a",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&a),
            },
            crate::StreamSourceBinding {
                id: "graph-b",
                revision: 10,
                source: crate::StreamColumnSource::Scalar(&b),
            },
            crate::StreamSourceBinding {
                id: "graph-c",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&c),
            },
            crate::StreamSourceBinding {
                id: "graph-d",
                revision: 20,
                source: crate::StreamColumnSource::Scalar(&d),
            },
        ];
        renderer
            .demote_resident_columns_to_streamed(&["graph-a"], metadata, &bindings)
            .unwrap();

        for id in ["graph-a", "graph-b", "graph-c", "graph-d"] {
            assert!(renderer.pool.slot(id).is_none(), "{id} stayed resident");
            assert!(
                renderer.streaming_sources.contains_key(id),
                "{id} did not become streamed"
            );
        }
        assert_eq!(renderer.streaming_sources["graph-b"].fit_epoch, b_fit);
        assert_eq!(renderer.streaming_sources["graph-d"].fit_epoch, d_fit);
        assert_eq!(
            renderer.streaming_sources["graph-b"]
                .statistics_cache
                .covered,
            vec![0..1]
        );
        assert_eq!(
            renderer.streaming_sources["graph-d"]
                .statistics_cache
                .covered,
            vec![1..2]
        );
        for (chart, config, series, before) in chart_snapshots {
            assert_eq!(renderer.chart_config(chart).unwrap(), &config);
            assert_eq!(renderer.chart_series(chart).unwrap(), series);
            let after = revision_snapshot(&renderer, chart);
            assert_eq!(after[0].sequence, before[0].sequence + 1);
            assert_eq!(after[3].sequence, before[3].sequence + 1);
            for index in [1, 2, 4, 5, 6] {
                assert_eq!(after[index], before[index]);
            }
        }
        assert_eq!(revision_snapshot(&renderer, picker_chart), picker_revisions);
        assert_eq!(active_picker(&renderer).chart_id, picker_chart);
        assert!(renderer.pending_defrag);
    }

    #[test]
    fn mixed_normalize_promote_and_length_change_demote_preserve_lineage_monotonicity() {
        let Some(mut renderer) = state_test_renderer_with_capacity(8 * 1024) else {
            return;
        };
        let x_old = col_f64(vec![1.0, 2.0]);
        let x_new = col_f64(vec![1.0, 2.0, 3.0]);
        let y = col_f64(vec![4.0, 5.0]);
        renderer.add_column("round-x", &x_old).unwrap();
        renderer
            .register_streamed_columns(vec![crate::StreamColumn {
                id: "round-y".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 5,
                statistics: crate::StreamStatistics::Unknown,
            }])
            .unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("round", "round-x", "round-y")],
            )
            .unwrap();
        let initial_revisions = revision_snapshot(&renderer, chart);
        let initial_metadata = vec![
            crate::StreamColumn {
                id: "round-x".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 1,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "round-y".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 5,
                statistics: crate::StreamStatistics::Unknown,
            },
        ];
        let initial_bindings = [
            crate::StreamSourceBinding {
                id: "round-x",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&x_old),
            },
            crate::StreamSourceBinding {
                id: "round-y",
                revision: 5,
                source: crate::StreamColumnSource::Scalar(&y),
            },
        ];
        renderer
            .demote_resident_columns_to_streamed(&["round-x"], initial_metadata, &initial_bindings)
            .unwrap();
        assert_eq!(renderer.stream_revision_lineage["round-x"], 1);
        assert_eq!(renderer.stream_revision_lineage["round-y"], 5);

        let _ = renderer.set_memory_budget(Some(u64::MAX));
        let report = renderer
            .promote_streamed_columns_to_resident(&initial_bindings, 0, &[], 0, u64::MAX)
            .unwrap();
        assert!(report.is_admissible(), "{report:?}");
        assert!(renderer.pool.slot("round-x").is_some());
        assert!(renderer.pool.slot("round-y").is_some());
        assert_eq!(renderer.stream_revision_lineage["round-x"], 1);
        assert_eq!(renderer.stream_revision_lineage["round-y"], 5);

        let final_metadata = vec![
            crate::StreamColumn {
                id: "round-x".into(),
                len: 3,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::Sequential,
                revision: 2,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "round-y".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 6,
                statistics: crate::StreamStatistics::Unknown,
            },
        ];
        let final_bindings = [
            crate::StreamSourceBinding {
                id: "round-x",
                revision: 2,
                source: crate::StreamColumnSource::Scalar(&x_new),
            },
            crate::StreamSourceBinding {
                id: "round-y",
                revision: 6,
                source: crate::StreamColumnSource::Scalar(&y),
            },
        ];
        renderer
            .demote_resident_columns_to_streamed(&["round-x"], final_metadata, &final_bindings)
            .unwrap();
        assert_eq!(renderer.streaming_sources["round-x"].len, 3);
        assert_eq!(renderer.streaming_sources["round-x"].revision, 2);
        assert_eq!(renderer.streaming_sources["round-y"].revision, 6);
        assert_eq!(renderer.stream_revision_lineage["round-x"], 2);
        assert_eq!(renderer.stream_revision_lineage["round-y"], 6);
        let final_revisions = revision_snapshot(&renderer, chart);
        assert_eq!(
            final_revisions[0].sequence,
            initial_revisions[0].sequence + 3
        );
        assert_eq!(
            final_revisions[3].sequence,
            initial_revisions[3].sequence + 3
        );
        for index in [1, 2, 4, 5, 6] {
            assert_eq!(final_revisions[index], initial_revisions[index]);
        }
    }

    #[test]
    fn resident_demotion_replaces_a_changed_streamed_root_within_the_mixed_commit() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let resident = col_f64(vec![1.0, 2.0]);
        let new_stream = col_f64(vec![3.0, 4.0, 5.0]);
        renderer.add_column("replace-resident", &resident).unwrap();
        renderer
            .register_streamed_columns(vec![crate::StreamColumn {
                id: "replace-stream".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 5,
                statistics: crate::StreamStatistics::Unknown,
            }])
            .unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line(
                    "replace",
                    "replace-resident",
                    "replace-stream",
                )],
            )
            .unwrap();
        let before = revision_snapshot(&renderer, chart);
        let old_fit_epoch = renderer.streaming_sources["replace-stream"].fit_epoch;
        let metadata = vec![
            crate::StreamColumn {
                id: "replace-resident".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 1,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "replace-stream".into(),
                len: 3,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::Sequential,
                revision: 6,
                statistics: crate::StreamStatistics::Unknown,
            },
        ];
        let bindings = [
            crate::StreamSourceBinding {
                id: "replace-resident",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&resident),
            },
            crate::StreamSourceBinding {
                id: "replace-stream",
                revision: 6,
                source: crate::StreamColumnSource::Scalar(&new_stream),
            },
        ];
        renderer
            .demote_resident_columns_to_streamed(&["replace-stream"], metadata, &bindings)
            .unwrap();

        assert!(renderer.pool.slot("replace-resident").is_none());
        assert_eq!(renderer.streaming_sources["replace-resident"].revision, 1);
        let replaced = &renderer.streaming_sources["replace-stream"];
        assert_eq!(replaced.revision, 6);
        assert_eq!(replaced.len, 3);
        assert_eq!(replaced.replay, crate::StreamReplay::Sequential);
        assert!(replaced.fit_epoch > old_fit_epoch);
        assert_eq!(renderer.stream_revision_lineage["replace-stream"], 6);
        let after = revision_snapshot(&renderer, chart);
        assert_eq!(after[0].sequence, before[0].sequence + 1);
        assert_eq!(after[3].sequence, before[3].sequence + 1);
        for index in [1, 2, 4, 5, 6] {
            assert_eq!(after[index], before[index]);
        }
    }

    #[test]
    fn all_streamed_same_revision_normalization_is_a_true_noop() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let x = col_f64(vec![1.0, 2.0]);
        let y = col_f64(vec![3.0, 4.0]);
        let resident = col_f64(vec![5.0, 6.0]);
        renderer
            .register_streamed_columns(vec![
                crate::StreamColumn {
                    id: "noop-x".into(),
                    len: 2,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    revision: 3,
                    statistics: crate::StreamStatistics::Unknown,
                },
                crate::StreamColumn {
                    id: "noop-y".into(),
                    len: 2,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::Sequential,
                    revision: 4,
                    statistics: crate::StreamStatistics::Unknown,
                },
            ])
            .unwrap();
        let streamed_chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("noop", "noop-x", "noop-y")],
            )
            .unwrap();
        renderer.add_column("noop-resident", &resident).unwrap();
        let picker_chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("picker", "noop-resident", "noop-resident")],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer
            .prepare_gpu_picking_for_chart(picker_chart)
            .unwrap();
        renderer
            .streaming_sources
            .get_mut("noop-x")
            .unwrap()
            .statistics_cache
            .covered
            .push(0..1);
        renderer.pending_defrag = true;

        let revisions = revision_snapshot(&renderer, streamed_chart);
        let picker_revisions = revision_snapshot(&renderer, picker_chart);
        let visual = renderer.visual_revision;
        let generation = renderer.pool.generation();
        let used = renderer.pool.used_bytes();
        let next_fit_epoch = renderer.next_stream_source_fit_epoch;
        let x_fit_epoch = renderer.streaming_sources["noop-x"].fit_epoch;
        let y_fit_epoch = renderer.streaming_sources["noop-y"].fit_epoch;
        let lineage = renderer.stream_revision_lineage.clone();
        let metadata = || {
            vec![
                crate::StreamColumn {
                    id: "noop-x".into(),
                    len: 2,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    revision: 3,
                    statistics: crate::StreamStatistics::Unknown,
                },
                crate::StreamColumn {
                    id: "noop-y".into(),
                    len: 2,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::Sequential,
                    revision: 4,
                    statistics: crate::StreamStatistics::Unknown,
                },
            ]
        };
        let bindings = [
            crate::StreamSourceBinding {
                id: "noop-x",
                revision: 3,
                source: crate::StreamColumnSource::Scalar(&x),
            },
            crate::StreamSourceBinding {
                id: "noop-y",
                revision: 4,
                source: crate::StreamColumnSource::Scalar(&y),
            },
        ];
        for _ in 0..4 {
            renderer
                .demote_resident_columns_to_streamed(&["noop-x"], metadata(), &bindings)
                .unwrap();
        }

        assert_eq!(renderer.pool.generation(), generation);
        assert_eq!(renderer.pool.used_bytes(), used);
        assert_eq!(renderer.next_stream_source_fit_epoch, next_fit_epoch);
        assert_eq!(renderer.streaming_sources["noop-x"].fit_epoch, x_fit_epoch);
        assert_eq!(renderer.streaming_sources["noop-y"].fit_epoch, y_fit_epoch);
        assert_eq!(
            renderer.streaming_sources["noop-x"]
                .statistics_cache
                .covered,
            vec![0..1]
        );
        assert_eq!(renderer.stream_revision_lineage, lineage);
        assert_eq!(revision_snapshot(&renderer, streamed_chart), revisions);
        assert_eq!(revision_snapshot(&renderer, picker_chart), picker_revisions);
        assert_eq!(renderer.visual_revision, visual);
        assert_eq!(active_picker(&renderer).chart_id, picker_chart);
        assert!(renderer.pending_defrag);
    }

    #[test]
    fn live_normalization_supersedes_the_active_auto_snapshot_immediately() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let new_x = col_f64(vec![10.0, 20.0]);
        let y = col_f64(vec![3.0, 4.0]);
        renderer
            .register_streamed_columns(vec![
                crate::StreamColumn {
                    id: "auto-live-x".into(),
                    len: 2,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    revision: 1,
                    statistics: crate::StreamStatistics::Unknown,
                },
                crate::StreamColumn {
                    id: "auto-live-y".into(),
                    len: 2,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    revision: 1,
                    statistics: crate::StreamStatistics::Unknown,
                },
            ])
            .unwrap();
        let config = crate::default::default_config();
        let chart = renderer
            .register_chart(
                config.clone(),
                vec![state_test_line("auto-live", "auto-live-x", "auto-live-y")],
            )
            .unwrap();
        renderer
            .configure_streaming(crate::StreamingLimits {
                max_active_charts: 1,
                max_in_flight_chunks: 2,
                max_columns_per_chunk: 2,
                max_chunk_input_bytes: 4096,
                max_in_flight_gpu_bytes: 8192,
            })
            .unwrap();
        let view = renderer
            .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
            .unwrap();
        let options = crate::StreamingChartOptions {
            size: (config.chart_area.0.width, config.chart_area.0.height),
            clear_color: Color::WHITE,
            max_primitives_per_chunk: 1,
        };
        let crate::AutoStreamingRequest::Started {
            revision: active_revision,
            ..
        } = renderer
            .request_auto_streaming_chart(chart, &view, options)
            .unwrap()
        else {
            panic!("automatic stream must start")
        };
        let job = renderer.active_stream_job(chart).unwrap();
        assert_eq!(
            renderer.auto_stream_snapshot(job).unwrap().sources["auto-live-x"].revision,
            1
        );

        let metadata = vec![
            crate::StreamColumn {
                id: "auto-live-x".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 2,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "auto-live-y".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 1,
                statistics: crate::StreamStatistics::Unknown,
            },
        ];
        let live_bindings = [
            crate::StreamSourceBinding {
                id: "auto-live-x",
                revision: 2,
                source: crate::StreamColumnSource::Scalar(&new_x),
            },
            crate::StreamSourceBinding {
                id: "auto-live-y",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&y),
            },
        ];
        renderer
            .demote_resident_columns_to_streamed(&["auto-live-x"], metadata, &live_bindings)
            .unwrap();
        let latest = renderer.chart_states[&chart].revisions.desired;
        let crate::AutoStreamingRequest::Started {
            revision: replacement_revision,
            ..
        } = renderer
            .request_auto_streaming_chart(chart, &view, options)
            .unwrap()
        else {
            panic!("live source normalization must immediately start the latest execution")
        };
        assert_eq!(replacement_revision, latest);
        let replacement_job = renderer.active_stream_job(chart).unwrap();
        assert_ne!(replacement_job, job);
        let snapshot = renderer.auto_stream_snapshot(replacement_job).unwrap();
        assert_eq!(snapshot.sources["auto-live-x"].revision, 2);
        assert_eq!(snapshot.sources["auto-live-y"].revision, 1);
        assert!(renderer.request_chart_stream_draw(job).is_err());
        let streaming_runtime::StreamDrawRequestStatus::Ready(ticket) =
            renderer.request_chart_stream_draw(replacement_job).unwrap()
        else {
            panic!("replacement automatic stream must issue the latest source request")
        };
        let requested = renderer.stream_request_columns(ticket).unwrap();
        assert!(
            requested
                .iter()
                .any(|request| { request.column == "auto-live-x" && request.range.revision == 2 })
        );
        assert!(
            requested
                .iter()
                .any(|request| { request.column == "auto-live-y" && request.range.revision == 1 })
        );
        assert!(
            requested
                .iter()
                .all(|request| { request.column != "auto-live-x" || request.range.revision != 1 })
        );
        assert_ne!(active_revision, latest);
    }

    #[test]
    fn mixed_normalization_validation_failures_preserve_every_authority() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let resident = col_f64(vec![1.0, 2.0]);
        let streamed = col_f64(vec![3.0, 4.0]);
        let unrelated = col_f64(vec![5.0, 6.0]);
        let short = col_f64(vec![3.0]);
        renderer.add_column("fail-resident", &resident).unwrap();
        renderer.add_column("fail-unrelated", &unrelated).unwrap();
        renderer
            .register_streamed_columns(vec![crate::StreamColumn {
                id: "fail-stream".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 5,
                statistics: crate::StreamStatistics::Unknown,
            }])
            .unwrap();
        let affected = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("affected", "fail-resident", "fail-stream")],
            )
            .unwrap();
        let picker_chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line(
                    "unrelated",
                    "fail-unrelated",
                    "fail-unrelated",
                )],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        renderer
            .prepare_gpu_picking_for_chart(picker_chart)
            .unwrap();
        {
            let cache = &mut renderer
                .streaming_sources
                .get_mut("fail-stream")
                .unwrap()
                .statistics_cache;
            cache.covered.push(0..1);
            cache.bounds = Some(crate::StreamBounds {
                min: 3.0,
                max: 3.0,
                min_positive: Some(3.0),
            });
        }

        let generation = renderer.pool.generation();
        let used = renderer.pool.used_bytes();
        let resident_slot = renderer.pool.slot("fail-resident").unwrap().clone();
        let stream_column = renderer.streaming_sources["fail-stream"].column.clone();
        let stream_fit_epoch = renderer.streaming_sources["fail-stream"].fit_epoch;
        let stream_covered = renderer.streaming_sources["fail-stream"]
            .statistics_cache
            .covered
            .clone();
        let stream_bounds = renderer.streaming_sources["fail-stream"]
            .statistics_cache
            .bounds;
        let lineage = renderer.stream_revision_lineage.clone();
        let affected_revisions = revision_snapshot(&renderer, affected);
        let picker_revisions = revision_snapshot(&renderer, picker_chart);
        let visual = renderer.visual_revision;
        let fit_epoch = renderer.next_stream_source_fit_epoch;
        let pending_defrag = renderer.pending_defrag;
        let assert_unchanged = |renderer: &Renderer| {
            assert_eq!(renderer.pool.generation(), generation);
            assert_eq!(renderer.pool.used_bytes(), used);
            let slot = renderer.pool.slot("fail-resident").unwrap();
            assert_eq!(slot.offset, resident_slot.offset);
            assert_eq!(slot.len_values, resident_slot.len_values);
            let registered = &renderer.streaming_sources["fail-stream"];
            assert_eq!(registered.column, stream_column);
            assert_eq!(registered.fit_epoch, stream_fit_epoch);
            assert_eq!(registered.statistics_cache.covered, stream_covered);
            assert_eq!(registered.statistics_cache.bounds, stream_bounds);
            assert_eq!(renderer.stream_revision_lineage, lineage);
            assert_eq!(revision_snapshot(renderer, affected), affected_revisions);
            assert_eq!(revision_snapshot(renderer, picker_chart), picker_revisions);
            assert_eq!(renderer.visual_revision, visual);
            assert_eq!(renderer.next_stream_source_fit_epoch, fit_epoch);
            assert_eq!(renderer.pending_defrag, pending_defrag);
            assert_eq!(active_picker(renderer).chart_id, picker_chart);
        };
        let metadata = |id: &str, len: u64, revision: u64| crate::StreamColumn {
            id: id.into(),
            len,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision,
            statistics: crate::StreamStatistics::Unknown,
        };
        let resident_binding = crate::StreamSourceBinding {
            id: "fail-resident",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&resident),
        };
        let stream_binding = crate::StreamSourceBinding {
            id: "fail-stream",
            revision: 5,
            source: crate::StreamColumnSource::Scalar(&streamed),
        };

        assert!(
            renderer
                .demote_resident_columns_to_streamed(
                    &["fail-resident"],
                    vec![metadata("fail-resident", 2, 1)],
                    &[resident_binding],
                )
                .is_err()
        );
        assert_unchanged(&renderer);

        assert!(
            renderer
                .demote_resident_columns_to_streamed(
                    &["fail-resident"],
                    vec![
                        metadata("fail-resident", 2, 1),
                        metadata("fail-stream", 2, 5),
                        metadata("fail-stream", 2, 5),
                    ],
                    &[resident_binding, stream_binding],
                )
                .is_err()
        );
        assert_unchanged(&renderer);

        let extra_binding = crate::StreamSourceBinding {
            id: "fail-extra",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&resident),
        };
        assert!(
            renderer
                .demote_resident_columns_to_streamed(
                    &["fail-resident"],
                    vec![
                        metadata("fail-resident", 2, 1),
                        metadata("fail-stream", 2, 5),
                        metadata("fail-extra", 2, 1),
                    ],
                    &[resident_binding, stream_binding, extra_binding],
                )
                .is_err()
        );
        assert_unchanged(&renderer);

        let stale_binding = crate::StreamSourceBinding {
            id: "fail-stream",
            revision: 4,
            source: crate::StreamColumnSource::Scalar(&streamed),
        };
        assert!(
            renderer
                .demote_resident_columns_to_streamed(
                    &["fail-stream"],
                    vec![
                        metadata("fail-resident", 2, 1),
                        metadata("fail-stream", 2, 4),
                    ],
                    &[resident_binding, stale_binding],
                )
                .is_err()
        );
        assert_unchanged(&renderer);

        let short_binding = crate::StreamSourceBinding {
            id: "fail-stream",
            revision: 5,
            source: crate::StreamColumnSource::Scalar(&short),
        };
        assert!(
            renderer
                .demote_resident_columns_to_streamed(
                    &["fail-resident"],
                    vec![
                        metadata("fail-resident", 2, 1),
                        metadata("fail-stream", 1, 5),
                    ],
                    &[resident_binding, short_binding],
                )
                .is_err()
        );
        assert_unchanged(&renderer);

        let wrong_encoding = crate::StreamSourceBinding {
            id: "fail-stream",
            revision: 5,
            source: crate::StreamColumnSource::HiLo(&streamed),
        };
        assert!(
            renderer
                .demote_resident_columns_to_streamed(
                    &["fail-resident"],
                    vec![
                        metadata("fail-resident", 2, 1),
                        metadata("fail-stream", 2, 5),
                    ],
                    &[resident_binding, wrong_encoding],
                )
                .is_err()
        );
        assert_unchanged(&renderer);

        let newer_companion = crate::StreamSourceBinding {
            id: "fail-stream",
            revision: 6,
            source: crate::StreamColumnSource::Scalar(&streamed),
        };
        assert!(
            renderer
                .demote_resident_columns_to_streamed(
                    &["fail-resident"],
                    vec![
                        metadata("fail-resident", 2, 1),
                        metadata("fail-stream", 2, 6),
                    ],
                    &[resident_binding, newer_companion],
                )
                .is_err()
        );
        assert_unchanged(&renderer);

        renderer
            .demote_resident_columns_to_streamed(
                &["fail-resident"],
                vec![
                    metadata("fail-resident", 2, 1),
                    metadata("fail-stream", 2, 5),
                ],
                &[resident_binding, stream_binding],
            )
            .unwrap();
        assert!(renderer.pool.slot("fail-resident").is_none());
        assert_eq!(renderer.streaming_sources["fail-resident"].revision, 1);
        assert_eq!(
            renderer.streaming_sources["fail-stream"].column,
            stream_column
        );
        assert_eq!(
            renderer.streaming_sources["fail-stream"]
                .statistics_cache
                .covered,
            stream_covered
        );
        assert_eq!(
            renderer.streaming_sources["fail-stream"]
                .statistics_cache
                .bounds,
            stream_bounds
        );
        let after = revision_snapshot(&renderer, affected);
        assert_eq!(after[0].sequence, affected_revisions[0].sequence + 1);
        assert_eq!(after[3].sequence, affected_revisions[3].sequence + 1);
        for index in [1, 2, 4, 5, 6] {
            assert_eq!(after[index], affected_revisions[index]);
        }
        assert_eq!(revision_snapshot(&renderer, picker_chart), picker_revisions);
        assert_eq!(active_picker(&renderer).chart_id, picker_chart);
        assert!(renderer.pending_defrag);
    }

    #[test]
    fn resident_admission_counts_submitted_column_staging_until_completion() {
        let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
            return;
        };
        let before = renderer.gpu_memory_usage().checked_total_bytes().unwrap();
        renderer.add_column("staged", &col_f64(vec![1.0])).unwrap();
        let during = renderer.gpu_memory_usage();
        assert_eq!(
            during.retired_bytes_of(GpuResourceKind::ColumnPool),
            data_render::column_pool::ALIGN
        );
        let current = during.checked_total_bytes().unwrap();
        assert_eq!(current, before + data_render::column_pool::ALIGN);
        assert_eq!(renderer.set_memory_budget(Some(current)), None);
        let admitted = renderer.resident_admission(ResidentAdmissionRequest {
            column_value_counts: &[],
            already_resident_working_set_bytes: 0,
            derived_buffer_sizes: &[],
            transition_headroom_bytes: 0,
            working_set_limit_bytes: 0,
        });
        assert!(admitted.is_admissible(), "{admitted:?}");
        assert_eq!(admitted.current_gpu_bytes, current);

        renderer.end_gpu_frame();
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("submitted staging completes");
        assert_eq!(
            renderer.gpu_memory_usage().checked_total_bytes(),
            Some(before)
        );
    }

    #[test]
    fn resident_admission_mirrors_fixed_and_growing_pool_policies_without_mutation() {
        let Some(mut renderer) = state_test_renderer_with_capacity(data_render::column_pool::ALIGN)
        else {
            return;
        };
        let lengths = [64]; // 512 resident bytes, twice the initial pool.
        let request = ResidentAdmissionRequest {
            column_value_counts: &lengths,
            already_resident_working_set_bytes: 0,
            derived_buffer_sizes: &[],
            transition_headroom_bytes: 0,
            working_set_limit_bytes: 512,
        };
        let before_capacity = renderer.pool.capacity();
        assert_eq!(renderer.set_memory_budget(Some(u64::MAX)), None);
        let fixed = renderer.resident_admission(request);
        assert_eq!(
            fixed.status,
            ResidentAdmissionStatus::PoolCapacityUnavailable
        );
        assert_eq!(renderer.pool.capacity(), before_capacity);
        assert_eq!(renderer.pool.used_bytes(), 0);

        renderer.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
        let growing = renderer.resident_admission(request);
        assert!(growing.is_admissible(), "{growing:?}");
        assert_eq!(growing.pool_capacity_before, 256);
        assert_eq!(growing.pool_capacity_after, 512);
        assert_eq!(growing.pool_transition_bytes, 512);
        assert_eq!(growing.upload_staging_bytes, 512);
        assert_eq!(renderer.pool.capacity(), before_capacity);
        assert_eq!(renderer.pool.used_bytes(), 0);
    }

    #[test]
    fn resident_admission_fails_closed_on_index_and_byte_overflow() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        assert_eq!(renderer.set_memory_budget(Some(u64::MAX)), None);
        let over_index = [u64::from(u32::MAX) + 1];
        let rejected = renderer.resident_admission(ResidentAdmissionRequest {
            column_value_counts: &over_index,
            already_resident_working_set_bytes: 0,
            derived_buffer_sizes: &[],
            transition_headroom_bytes: 0,
            working_set_limit_bytes: u64::MAX,
        });
        assert_eq!(
            rejected.status,
            ResidentAdmissionStatus::ColumnLengthLimitExceeded {
                column: 0,
                length: u64::from(u32::MAX) + 1,
            }
        );

        let one = [1];
        let derived = [1];
        let overflow = renderer.resident_admission(ResidentAdmissionRequest {
            column_value_counts: &one,
            already_resident_working_set_bytes: u64::MAX,
            derived_buffer_sizes: &derived,
            transition_headroom_bytes: 0,
            working_set_limit_bytes: u64::MAX,
        });
        assert_eq!(overflow.status, ResidentAdmissionStatus::ArithmeticOverflow);

        let oversized = [renderer
            .caps
            .max_buffer_size
            .min(renderer.caps.max_storage_buffer_binding_size)
            + 1];
        let rejected = renderer.resident_admission(ResidentAdmissionRequest {
            column_value_counts: &[],
            already_resident_working_set_bytes: 0,
            derived_buffer_sizes: &oversized,
            transition_headroom_bytes: 0,
            working_set_limit_bytes: u64::MAX,
        });
        assert_eq!(
            rejected.status,
            ResidentAdmissionStatus::DerivedBufferLimitExceeded {
                buffer: 0,
                bytes: oversized[0],
            }
        );
    }

    #[test]
    fn registering_a_pick_series_charges_pick_scratch_and_releases_it() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_scatter("scatter", None, "x", "y")],
            )
            .unwrap();

        let before = renderer.gpu_memory_usage();
        renderer.enable_gpu_picking().unwrap();
        renderer.prepare_gpu_picking_for_chart(chart).unwrap();
        let after = renderer.gpu_memory_usage();
        assert!(
            after.live_bytes_of(GpuResourceKind::PickScratch)
                > before.live_bytes_of(GpuResourceKind::PickScratch),
            "pick registration allocates gate masks, candidates, style rows and \
             a scalar result — none of it was charged\n{}",
            after.report()
        );

        // Dropping the renderer must credit every charged byte back — the
        // ledger outlives it, so a leak shows up as a non-zero live row.
        let ledger = Arc::clone(renderer.gpu_ledger());
        drop(renderer);
        let released = ledger.snapshot();
        for kind in GpuResourceKind::ALL {
            assert_eq!(
                released.live_bytes_of(kind),
                0,
                "{} bytes still charged after the renderer was dropped\n{}",
                kind.label(),
                released.report()
            );
        }
        assert!(
            released.creations_of(GpuResourceKind::PickScratch) > 0,
            "the pick rows were never populated, so the check above proves nothing"
        );
    }

    #[test]
    fn out_of_pool_bytes_ride_into_the_pool_and_a_budget_refuses_growth() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.pool.growth_policy = data_render::GrowthPolicy::OnAllocFailure;
        let chart = Chart::new(panel_config(320, 240));
        let _view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();

        let usage = renderer.gpu_memory_usage();
        let external = usage.external_bytes();
        assert!(
            external >= 320 * 240 * 4 * 2,
            "the panel textures must be visible as out-of-pool bytes\n{}",
            usage.report()
        );

        // A ceiling that the settled state fits under but a growth cannot:
        // the pool would need old + new on top of `external`.
        assert_eq!(
            renderer.set_memory_budget(Some(external + usage.pool_bytes() + 1)),
            None,
            "this ceiling is above the settled total, so nothing is exceeded yet"
        );
        let capacity_before = renderer.pool.capacity();
        let oversized = col_f64((0..(capacity_before as usize)).map(|i| i as f64).collect());
        let error = renderer
            .add_column("too-big", &oversized)
            .expect_err("an upload larger than the pool must not fit under the budget");
        assert!(
            matches!(
                error,
                FiggyError::Pool(data_render::AllocError::BudgetExceeded { .. })
            ),
            "growth refused under budget must name the actual cause: {error:?}"
        );
        assert_eq!(
            renderer.pool.capacity(),
            capacity_before,
            "a refused growth leaves the pool exactly as it was"
        );
        assert!(renderer.pool.slot("too-big").is_none());

        // Lifting the ceiling lets the same upload through, which proves the
        // refusal came from the budget and not from the data.
        assert_eq!(
            renderer.set_memory_budget(None),
            None,
            "clearing the ceiling cannot conflict with anything"
        );
        renderer
            .add_column("too-big", &oversized)
            .expect("the same upload fits once the ceiling is lifted");
        assert!(renderer.pool.capacity() > capacity_before);
    }
}

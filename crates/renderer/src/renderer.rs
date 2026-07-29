//! Public-facing wgpu renderer.
//!
//! `Renderer::try_new` builds every figgy GPU resource (pool, pipelines,
//! bind-group layouts, sampler, quad VB) in one call so users don't have to
//! wire `data_render::create_*_pipeline` / `ColumnPool::new` manually.
//!
//! Ownership: holds `Arc<wgpu::Device>` and `Arc<wgpu::Queue>` so the same
//! device can be shared with the host (egui_wgpu, iced_wgpu, etc.) without
//! lifetime gymnastics.
//!
//! `try_new` returns `Result<_, FiggyError>` so unsupported target formats and
//! device resource limits are reported before wgpu validation can abort.

use std::collections::HashMap;
use std::sync::Arc;

use crate::axis_render;
use crate::chart::Chart;
use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, Config, DrawStyle, PickedPointRef};
use crate::data::{ColumnSource, HiLoColumnSource};
use crate::data_config::{
    DataErrorBarPointStyleConfig, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
    DataScatterPointStyleConfig, DataScatterStyleConfig, ErrorRef, ScatterShape, SeriesConfig,
};
use crate::data_render::{
    self, AxisLayer, ColumnErrorBarDraw, ColumnHandle, ColumnId, ColumnLineLayer,
    ColumnPickRingLayer, ColumnPool, ColumnScatterLayer, DefragPolicy, PrimitiveStyle,
};
use crate::error::{FiggyError, Result};
use crate::layout::Rect;
use crate::line::LineStylePreset;
use crate::select::HitId;

/// A wgpu device/queue pair used by figgy.
///
/// wgpu does not expose a public parent-device id on `Queue`, so figgy cannot
/// prove at runtime that arbitrary handles came from the same logical device.
/// Passing them as one value keeps the pair together throughout the renderer
/// API and avoids accepting loose device/queue arguments at every call site.
#[derive(Clone)]
pub struct RendererDevice {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl RendererDevice {
    /// Bundle a wgpu device and queue that came from the same `request_device`
    /// result or host render state.
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        Self { device, queue }
    }

    pub fn device(&self) -> &Arc<wgpu::Device> {
        &self.device
    }
    pub fn queue(&self) -> &Arc<wgpu::Queue> {
        &self.queue
    }
}

#[derive(Clone, Copy, Debug)]
struct RendererDeviceCaps {
    features: wgpu::Features,
    max_texture_dimension_2d: u32,
    max_buffer_size: u64,
    /// The arc-length scan and star pass bind the WHOLE pool as a single
    /// storage-buffer binding, so the pool can never exceed this. A request
    /// above it is clamped at construction (see [`effective_pool_capacity`])
    /// rather than deferred into a wgpu validation panic on the first
    /// dashed/sketch/milkyway line.
    max_storage_buffer_binding_size: u32,
    /// Per-dimension dispatch ceiling for the arc-length compute scan.
    max_compute_workgroups_per_dimension: u32,
    /// The arc scan needs 256-wide workgroups; downlevel (GL-class)
    /// adapters report smaller/zero compute limits.
    max_compute_invocations_per_workgroup: u32,
}

impl RendererDeviceCaps {
    fn from_device(device: &wgpu::Device) -> Self {
        let limits = device.limits();
        Self {
            features: device.features(),
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            max_buffer_size: limits.max_buffer_size,
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_compute_workgroups_per_dimension: limits.max_compute_workgroups_per_dimension,
            max_compute_invocations_per_workgroup: limits.max_compute_invocations_per_workgroup,
        }
    }
}

/// The largest pool the renderer can actually bind, given the device.
///
/// The arc-scan compute and the constellation star pass bind the entire pool
/// buffer as one storage binding, so the pool is capped by BOTH
/// `max_storage_buffer_binding_size` and `max_buffer_size`. `try_new` clamps
/// the caller's requested capacity to this instead of letting an oversized
/// pool become a runtime validation panic — a caller that needs a bigger
/// pool must request higher device limits when creating the device (wgpu's
/// defaults are only 128 MiB / 256 MiB). Callers can read the granted size
/// back via `renderer.pool().capacity()`.
fn effective_pool_capacity(requested: u64, caps: RendererDeviceCaps) -> u64 {
    requested
        .min(u64::from(caps.max_storage_buffer_binding_size))
        .min(caps.max_buffer_size)
}

fn issue_renderer_identity() -> Result<u64> {
    static NEXT_RENDERER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT_RENDERER_ID
        .fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |current| current.checked_add(1),
        )
        .map(|previous| previous + 1)
        .map_err(|_| FiggyError::CounterExhausted {
            counter: "renderer identity",
        })
}

/// A dashed series' GPU arc-length prefix: the buffer (bound as the line
/// pipeline's vertex slots 4/5) and its used byte length.
type ArcPrefix = (Arc<wgpu::Buffer>, u64);
const ARC_PREFIX_CACHE_LIMIT: usize = 256;
const INTERNAL_ZERO_COLUMN_ID: &str = "__zero";

#[derive(Clone, Copy, Debug)]
struct InternalZeroColumn {
    len: usize,
}

impl ColumnSource for InternalZeroColumn {
    fn len(&self) -> usize {
        self.len
    }

    fn min(&self) -> f64 {
        0.0
    }

    fn max(&self) -> f64 {
        0.0
    }

    fn write_f32_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.len * std::mem::size_of::<f32>());
        dst.fill(0);
    }

    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.len * crate::data::COLUMN_VALUE_BYTES);
        dst.fill(0);
    }
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
}

/// Logical data-to-screen state derived from the renderer-owned `Config`.
///
/// This value is never stored as a parallel truth source. The getter derives
/// it from `Config`; the setter applies it back to those same four axes.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartViewState {
    pub chart_area: Rect,
    pub top_x: AxisViewState,
    pub bottom_x: AxisViewState,
    pub left_y: AxisViewState,
    pub right_y: AxisViewState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AxisViewState {
    pub scale: AxisScale,
    pub min: f64,
    pub max: f64,
    pub inverted: bool,
}

impl AxisViewState {
    fn from_axis(axis: &AxisOptions) -> Self {
        Self {
            scale: axis.scale.clone(),
            min: axis.min,
            max: axis.max,
            inverted: axis.inverted,
        }
    }

    fn apply_to(&self, axis: &mut AxisOptions) {
        axis.scale = self.scale.clone();
        crate::chart::apply_axis_range(
            axis,
            self.min,
            self.max,
            matches!(&self.scale, AxisScale::Logarithmic),
        );
        axis.inverted = self.inverted;
    }
}

impl ChartViewState {
    fn from_config(config: &Config) -> Self {
        Self {
            chart_area: config.chart_area.0,
            top_x: AxisViewState::from_axis(&config.top_x),
            bottom_x: AxisViewState::from_axis(&config.bottom_x),
            left_y: AxisViewState::from_axis(&config.left_y),
            right_y: AxisViewState::from_axis(&config.right_y),
        }
    }

    fn apply_to(&self, config: &mut Config) {
        config.chart_area.0 = self.chart_area;
        self.top_x.apply_to(&mut config.top_x);
        self.bottom_x.apply_to(&mut config.bottom_x);
        self.left_y.apply_to(&mut config.left_y);
        self.right_y.apply_to(&mut config.right_y);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ColumnContentStamp {
    id: ColumnId,
    allocation_epoch: u64,
}

/// Atomic identity for all renderer state used by web-owned derived caches.
///
/// Fields stay private so the web wrapper cannot reconstruct or increment
/// renderer counters independently.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WebDerivedStamp {
    renderer_identity: u64,
    chart_id: ChartId,
    view: RenderRevision,
    data: RenderRevision,
    pool_layout_generation: u64,
    columns: Vec<ColumnContentStamp>,
}

/// One coherent, owned read of renderer state for browser-only derived work.
#[derive(Clone, Debug)]
pub struct WebDerivedSnapshot {
    stamp: WebDerivedStamp,
    fit_token: FitCommitToken,
    config: Config,
    series: Vec<SeriesConfig>,
}

impl WebDerivedSnapshot {
    pub fn stamp(&self) -> &WebDerivedStamp {
        &self.stamp
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn series(&self) -> &[SeriesConfig] {
        &self.series
    }

    pub fn fit_token(&self) -> &FitCommitToken {
        &self.fit_token
    }
}

/// Invocation identity for an atomic two-axis auto-fit commit.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FitCommitToken {
    renderer_identity: u64,
    chart_id: ChartId,
    axes: FitAxisStamp,
    series: Vec<FitSeriesStamp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FitAxisStamp {
    top_x: (u8, u64, u64),
    bottom_x: (u8, u64, u64),
    left_y: (u8, u64, u64),
    right_y: (u8, u64, u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FitSeriesStamp {
    series_id: String,
    mode: crate::gpu_errorbar::GpuSeriesExtentMode,
    columns: Vec<FitColumnStamp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FitColumnStamp {
    role: u8,
    id: ColumnId,
    allocation_epoch: u64,
}

#[derive(Clone, Copy, Debug)]
struct ChartRevisions {
    desired: RenderRevision,
    config: RenderRevision,
    series: RenderRevision,
    data: RenderRevision,
    view: RenderRevision,
    selection: RenderRevision,
    raster: RenderRevision,
}

impl ChartRevisions {
    fn initial(revision: RenderRevision) -> Self {
        Self {
            desired: revision,
            config: revision,
            series: revision,
            data: revision,
            view: revision,
            selection: revision,
            raster: revision,
        }
    }
}

struct ChartRenderState {
    config: Config,
    series: Vec<SeriesConfig>,
    selected: Option<HitId>,
    revisions: ChartRevisions,
}

struct ChartColumnInvalidation {
    id: ChartId,
    desired: RenderRevision,
    data: RenderRevision,
}

struct ColumnInvalidationPlan {
    charts: Vec<ChartColumnInvalidation>,
    visual: Option<RenderRevision>,
}

impl ColumnInvalidationPlan {
    fn prospective_data_revision(&self, id: ChartId, current: RenderRevision) -> RenderRevision {
        self.charts
            .iter()
            .find(|update| update.id == id)
            .map_or(current, |update| update.data)
    }

    fn publish(
        self,
        chart_states: &mut HashMap<ChartId, ChartRenderState>,
        visual_revision: &mut RenderRevision,
    ) {
        for update in self.charts {
            let state = chart_states
                .get_mut(&update.id)
                .expect("column invalidation chart remains registered");
            state.revisions.desired = update.desired;
            state.revisions.data = update.data;
        }
        if let Some(visual) = self.visual {
            *visual_revision = visual;
        }
    }
}

struct ChartSeriesRemoval {
    id: ChartId,
    desired: RenderRevision,
    series_revision: RenderRevision,
    data: RenderRevision,
    raster: RenderRevision,
}

struct ColumnRemovalPlan {
    charts: Vec<ChartSeriesRemoval>,
    visual: Option<RenderRevision>,
}

impl ColumnRemovalPlan {
    fn publish(
        self,
        chart_states: &mut HashMap<ChartId, ChartRenderState>,
        visual_revision: &mut RenderRevision,
        removed_column: &str,
    ) {
        for update in self.charts {
            let state = chart_states
                .get_mut(&update.id)
                .expect("column removal chart remains registered");
            state
                .series
                .retain(|series| !series_references_column(series, removed_column));
            state.revisions.desired = update.desired;
            state.revisions.series = update.series_revision;
            state.revisions.data = update.data;
            state.revisions.raster = update.raster;
        }
        if let Some(visual) = self.visual {
            *visual_revision = visual;
        }
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
}

struct PreparedArcSeries {
    arc: Option<PreparedArc>,
}

struct PreparedArcItem {
    view_revision: u64,
    series: Vec<PreparedArcSeries>,
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
    quad_vb: wgpu::Buffer,
    pool_buffer: wgpu::Buffer,
    x: ColumnHandle,
    y: ColumnHandle,
    style_index: Option<ColumnHandle>,
    texture_bg: Option<wgpu::BindGroup>,
}

struct PreparedPickRingLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    style_map_bg: Option<wgpu::BindGroup>,
    quad_vb: wgpu::Buffer,
    pool_buffer: wgpu::Buffer,
    x: ColumnHandle,
    y: ColumnHandle,
    style_index: Option<ColumnHandle>,
    instance: u32,
}

struct PreparedStarLayer {
    pipeline: wgpu::RenderPipeline,
    transform_bg: wgpu::BindGroup,
    style_bg: wgpu::BindGroup,
    texture_bg: wgpu::BindGroup,
    star_bg: wgpu::BindGroup,
    indirect: wgpu::Buffer,
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
    errorbar: Option<PreparedErrorBarLayer>,
    line: Option<PreparedLineLayer>,
    line_extra: Option<PreparedStarLayer>,
    scatter: Option<PreparedScatterLayer>,
    picked: Vec<PreparedPickRingLayer>,
}

/// Refcounted GPU view bindings captured by `prepare`. Bind groups keep their
/// backing textures/buffers alive, so paint needs no `ChartView` borrow.
struct PreparedView {
    grid_bind_group: wgpu::BindGroup,
    decoration_bind_group: wgpu::BindGroup,
    content_revision: Arc<std::sync::atomic::AtomicU64>,
    expected_content_revision: u64,
    panel_rect: Rect,
}

/// Complete owned render input for one panel.
struct PreparedItem {
    view: PreparedView,
    data_area: Rect,
    axis_pipeline: wgpu::RenderPipeline,
    series: Vec<PreparedSeries>,
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
    /// Pool layout stamp — defrag, clear, or a buffer-relocating upsert can
    /// move column bytes or replace the backing buffer. Per-column allocation
    /// epochs above catch same-layout replacement of any captured draw input
    /// without invalidating a token for an unrelated column mutation.
    pool_layout_generation: u64,
    /// Monotonic identity of the target pipeline set. This detects format
    /// ABA and sample-count changes, not just a different current format.
    target_pipeline_generation: u64,
}

impl std::fmt::Debug for PreparedItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedItem")
            .field("series", &self.series.len())
            .finish_non_exhaustive()
    }
}

impl PreparedSeries {
    fn from_layers(layers: data_render::SeriesLayers<'_>) -> Self {
        Self {
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
                quad_vb: layer.quad_vb.clone(),
                pool_buffer: layer.pool_buffer.clone(),
                x: layer.x,
                y: layer.y,
                style_index: layer.style_index,
                texture_bg: layer.texture_bg.cloned(),
            }),
            picked: layers
                .picked
                .into_iter()
                .map(|layer| PreparedPickRingLayer {
                    pipeline: layer.pipeline.clone(),
                    transform_bg: layer.transform_bg.clone(),
                    style_bg: layer.style_bg,
                    style_map_bg: layer.style_map_bg.cloned(),
                    quad_vb: layer.quad_vb.clone(),
                    pool_buffer: layer.pool_buffer.clone(),
                    x: layer.x,
                    y: layer.y,
                    style_index: layer.style_index,
                    instance: layer.instance,
                })
                .collect(),
        }
    }

    fn layers(&self) -> data_render::SeriesLayers<'_> {
        data_render::SeriesLayers {
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
                quad_vb: &layer.quad_vb,
                pool_buffer: &layer.pool_buffer,
                x: layer.x,
                y: layer.y,
                style_index: layer.style_index,
                texture_bg: layer.texture_bg.as_ref(),
            }),
            picked: self
                .picked
                .iter()
                .map(|layer| ColumnPickRingLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    style_map_bg: layer.style_map_bg.as_ref(),
                    quad_vb: &layer.quad_vb,
                    pool_buffer: &layer.pool_buffer,
                    x: layer.x,
                    y: layer.y,
                    style_index: layer.style_index,
                    instance: layer.instance,
                })
                .collect(),
        }
    }
}

/// Facade bundling every figgy GPU resource.
pub struct Renderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    caps: RendererDeviceCaps,
    pool: ColumnPool,
    chart_states: HashMap<ChartId, ChartRenderState>,
    chart_order: Vec<ChartId>,
    next_chart_id: u64,
    visual_revision: RenderRevision,
    renderer_identity: u64,
    observed_font_generation: u64,
    pending_defrag: bool,
    errorbar_extent_engine: crate::gpu_errorbar::GpuErrorbarExtentEngine,
    target_sample_count: u32,
    target_pipeline_generation: u64,

    // Bind group layouts (exposed so callers can build per-panel bind groups).
    texture_bgl: wgpu::BindGroupLayout,
    transform_bgl: wgpu::BindGroupLayout,
    style_bgl: wgpu::BindGroupLayout,
    per_point_style_map_bgl: wgpu::BindGroupLayout,
    /// Constellation star-pass data layout (arc prefix + pool + offsets) —
    /// shared by the stars pipeline and every per-series star bind group.
    star_data_bgl: wgpu::BindGroupLayout,

    // Pipelines (precise + lazily-cached styled variants), all against
    // `surface_format`.
    pipelines: TargetPipelines,

    // Shared resources.
    sampler: wgpu::Sampler,
    quad_vb: wgpu::Buffer,

    /// Per-series GPU arc-scan slot pool for dashed lines, keyed by series
    /// id. The prefix is re-dispatched on every prepare that uses it (it
    /// depends on the data→pixel transform); a slot is reused in place only
    /// while the series layout (length, column offsets, pool layout
    /// generation) is stable AND no live `PreparedFrame` still references it
    /// (copy-on-write
    /// — see `ensure_arc_prefix`). Retained-token hosts settle on two slots
    /// per series that alternate frame over frame. Mutated only in
    /// `&mut self` prepare-phase entry points (`prepare`/`paint`/export);
    /// the `&self` draw phase reads the token's owned snapshot, never this
    /// cache.
    arc_cache: HashMap<String, Vec<data_render::line_arc::ArcScratch>>,
    arc_pipelines: data_render::line_arc::ArcScanPipelines,
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
}

/// Renderer-level guard for an atomic scalar or hi/lo column upsert.
///
/// The provisional pool is available for picker/bind-group preparation.  A
/// dropped guard restores the previous column mapping; `commit` is infallible
/// and allocation-free.
#[must_use = "dropping a RendererColumnUpsert rolls the provisional pool state back"]
pub struct RendererColumnUpsert<'a> {
    inner: Option<data_render::column_pool::ColumnUpsert<'a>>,
    chart_states: &'a mut HashMap<ChartId, ChartRenderState>,
    visual_revision: &'a mut RenderRevision,
    pending_defrag: &'a mut bool,
    invalidation: Option<ColumnInvalidationPlan>,
    renderer_identity: u64,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    errorbar_extent_engine: &'a crate::gpu_errorbar::GpuErrorbarExtentEngine,
}

impl RendererColumnUpsert<'_> {
    fn inner(&self) -> &data_render::column_pool::ColumnUpsert<'_> {
        self.inner
            .as_ref()
            .expect("renderer column upsert remains live")
    }

    pub fn pool(&self) -> &ColumnPool {
        self.inner().pool()
    }

    pub fn handle(&self) -> ColumnHandle {
        self.inner().handle()
    }

    pub fn replaced_existing(&self) -> bool {
        self.inner().replaced_existing()
    }

    /// Coherent prospective snapshot for cache preparation before commit.
    ///
    /// The stamp reflects the provisional pool mapping and the checked data
    /// revision that `commit` will publish. Dropping this guard rolls the pool
    /// back, so the returned stamp never becomes current.
    pub fn web_derived_snapshot(&self, id: ChartId) -> Result<WebDerivedSnapshot> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let data_revision = self
            .invalidation
            .as_ref()
            .expect("renderer column invalidation plan remains live")
            .prospective_data_revision(id, state.revisions.data);
        build_web_derived_snapshot_from_state(
            self.pool(),
            self.renderer_identity,
            id,
            state,
            data_revision,
        )
    }

    /// Begin an extent job against the provisional mappings.  The column
    /// upload was submitted before this guard was returned, so queue ordering
    /// makes the new bytes visible to the reduction.
    pub fn begin_errorbar_extent(
        &self,
        value_id: &str,
        lower_id: &str,
        upper_id: &str,
    ) -> std::result::Result<
        crate::gpu_errorbar::GpuErrorbarExtentTicket,
        crate::gpu_errorbar::GpuErrorbarError,
    > {
        begin_errorbar_extent_from_pool(
            self.errorbar_extent_engine,
            self.device,
            self.queue,
            self.pool(),
            value_id,
            lower_id,
            upper_id,
        )
    }

    /// Begin an exact drawable-series extent job against the provisional
    /// mappings. Inactive error directions are `None`; active directions
    /// require both lower and upper ids.
    pub fn begin_series_extent(
        &self,
        mode: crate::gpu_errorbar::GpuSeriesExtentMode,
        columns: crate::gpu_errorbar::GpuSeriesExtentColumnIds<'_>,
    ) -> std::result::Result<
        crate::gpu_errorbar::GpuSeriesExtentTicket,
        crate::gpu_errorbar::GpuErrorbarError,
    > {
        begin_series_extent_from_pool(
            self.errorbar_extent_engine,
            self.device,
            self.queue,
            self.pool(),
            mode,
            columns,
        )
    }

    pub fn commit(mut self) -> ColumnHandle {
        let replaced_existing = self.replaced_existing();
        let handle = self
            .inner
            .take()
            .expect("renderer column upsert remains live")
            .commit();
        self.invalidation
            .take()
            .expect("renderer column invalidation plan remains live")
            .publish(self.chart_states, self.visual_revision);
        if replaced_existing {
            *self.pending_defrag = true;
        }
        handle
    }
}

fn current_extent_handle(
    pool: &ColumnPool,
    role: &'static str,
    id: &str,
) -> std::result::Result<ColumnHandle, crate::gpu_errorbar::GpuErrorbarError> {
    let handle = pool.handle_for(id).ok_or_else(|| {
        crate::gpu_errorbar::GpuErrorbarError::UnknownColumn {
            role,
            id: id.to_string(),
        }
    })?;
    if !pool.is_valid_handle(&handle) {
        return Err(crate::gpu_errorbar::GpuErrorbarError::StaleHandle {
            role,
            id: id.to_string(),
            generation: handle.generation,
            current: pool.generation(),
        });
    }
    Ok(handle)
}

fn begin_errorbar_extent_from_pool(
    engine: &crate::gpu_errorbar::GpuErrorbarExtentEngine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pool: &ColumnPool,
    value_id: &str,
    lower_id: &str,
    upper_id: &str,
) -> std::result::Result<
    crate::gpu_errorbar::GpuErrorbarExtentTicket,
    crate::gpu_errorbar::GpuErrorbarError,
> {
    engine.begin(
        device,
        queue,
        pool.buffer(),
        current_extent_handle(pool, "value", value_id)?,
        current_extent_handle(pool, "lower error", lower_id)?,
        current_extent_handle(pool, "upper error", upper_id)?,
    )
}

fn begin_series_extent_from_pool(
    engine: &crate::gpu_errorbar::GpuErrorbarExtentEngine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pool: &ColumnPool,
    mode: crate::gpu_errorbar::GpuSeriesExtentMode,
    ids: crate::gpu_errorbar::GpuSeriesExtentColumnIds<'_>,
) -> std::result::Result<
    crate::gpu_errorbar::GpuSeriesExtentTicket,
    crate::gpu_errorbar::GpuErrorbarError,
> {
    let optional_handle = |role, id: Option<&str>| {
        id.map(|id| current_extent_handle(pool, role, id))
            .transpose()
    };
    engine.begin_series(
        device,
        queue,
        pool.buffer(),
        mode,
        crate::gpu_errorbar::GpuSeriesExtentColumns {
            x: current_extent_handle(pool, "x", ids.x)?,
            y: current_extent_handle(pool, "y", ids.y)?,
            x_lower: optional_handle("x lower error", ids.x_lower)?,
            x_upper: optional_handle("x upper error", ids.x_upper)?,
            y_lower: optional_handle("y lower error", ids.y_lower)?,
            y_upper: optional_handle("y upper error", ids.y_upper)?,
        },
    )
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

fn validate_target_format(caps: RendererDeviceCaps, format: wgpu::TextureFormat) -> Result<()> {
    if !caps.features.contains(format.required_features()) {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format requires device features that were not enabled".into(),
        });
    }
    let features = format.guaranteed_format_features(caps.features);
    if !features
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format is not guaranteed to support RENDER_ATTACHMENT".into(),
        });
    }
    if !features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "figgy pipelines require alpha blending into the target".into(),
        });
    }
    Ok(())
}

// Style descriptor table (docs/STYLE_REGISTRY.md §3). One descriptor fully
// describes a stylized render mode; the precise path is the absence of one
// (`style_variant` → `None`) and never routes through this table.

fn validate_target_sample_count(
    caps: RendererDeviceCaps,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> Result<()> {
    validate_target_format(caps, format)?;
    if sample_count == 1 {
        return Ok(());
    }
    let features = format.guaranteed_format_features(caps.features);
    if !features.flags.sample_count_supported(sample_count) {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: format!("format does not support {sample_count}x MSAA"),
        });
    }
    if !features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format does not support MSAA resolve targets".into(),
        });
    }
    Ok(())
}

fn preferred_msaa_sample_count(caps: RendererDeviceCaps, format: wgpu::TextureFormat) -> u32 {
    let features = format.guaranteed_format_features(caps.features);
    let can_resolve = features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE);
    if can_resolve && features.flags.sample_count_supported(4) {
        4
    } else if can_resolve && features.flags.sample_count_supported(2) {
        2
    } else {
        1
    }
}

/// Discriminant for cached per-style pipeline sets.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum StyleKey {
    Sketch,
    Milkyway,
    Constellation,
}

/// Everything the renderer needs to know about one stylized mode up front;
/// the per-format GPU objects live in the [`StyleSet`] the key maps to.
pub(crate) struct StyleVariant {
    pub(crate) key: StyleKey,
    pub(crate) needs_arc_prefix: bool,
    /// `Transform.style_params` packing — all three vec4 slots, layout per
    /// SHADER_COMMON.md §1.
    pub(crate) pack_params: fn(&DrawStyle) -> [f32; 12],
}

/// sketch: `[0] = (amplitude_px, wavelength_px, seed, 0)`, rest 0.
fn pack_sketch_params(style: &DrawStyle) -> [f32; 12] {
    match style.sketch() {
        Some(s) => {
            let mut p = [0.0; 12];
            p[0] = s.amplitude_px;
            p[1] = s.wavelength_px;
            p[2] = s.seed as f32;
            p
        }
        None => [0.0; 12],
    }
}

/// milkyway: `[0] = (star_density, ribbon_width_px, ribbon_intensity,
/// seed)`, `[1] = (star_scale, spread_px, faint_bias, planet_rim)`,
/// `[2] = (structure_scale, star_brightness, 0, 0)`.
/// `glow`/`nebula`/`dust` are CPU-raster parameters and never reach the GPU.
fn pack_milkyway_params(style: &DrawStyle) -> [f32; 12] {
    match style.milkyway() {
        Some(c) => [
            c.star_density,
            c.ribbon_width_px,
            c.ribbon_intensity,
            c.seed as f32,
            c.star_scale,
            c.spread_px,
            c.faint_bias,
            c.planet_rim,
            c.structure_scale,
            c.star_brightness,
            0.0,
            0.0,
        ],
        None => [0.0; 12],
    }
}

/// constellation: `[0] = (star_opacity, line_opacity, 0, 0)`,
/// `[1]`/`[2]` = 0.
/// Only ScatterLine series use this style.
fn pack_constellation_params(style: &DrawStyle) -> [f32; 12] {
    match style.constellation() {
        Some(c) => [
            c.star_opacity,
            c.line_opacity,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        None => [0.0; 12],
    }
}

static SKETCH_VARIANT: StyleVariant = StyleVariant {
    key: StyleKey::Sketch,
    needs_arc_prefix: true,
    pack_params: pack_sketch_params,
};

static MILKYWAY_VARIANT: StyleVariant = StyleVariant {
    key: StyleKey::Milkyway,
    // Ribbon profile and every star attribute are arc-length parameterized.
    needs_arc_prefix: true,
    pack_params: pack_milkyway_params,
};

static CONSTELLATION_VARIANT: StyleVariant = StyleVariant {
    key: StyleKey::Constellation,
    needs_arc_prefix: false,
    pack_params: pack_constellation_params,
};

/// Style lookup. None for Precise.
pub(crate) fn style_variant(style: &DrawStyle) -> Option<&'static StyleVariant> {
    match style {
        DrawStyle::Precise => None,
        DrawStyle::Sketch(_) => Some(&SKETCH_VARIANT),
        DrawStyle::Milkyway(_) => Some(&MILKYWAY_VARIANT),
        DrawStyle::Constellation(_) => Some(&CONSTELLATION_VARIANT),
    }
}

/// One style's compiled GPU objects against one target format. Styles differ
/// structurally (sketch swaps entry points; milkyway draws additive star
/// passes and carries baked textures; constellation renders scatter stars
/// plus a connecting line), so the set is an enum and the draw side matches
/// once.
enum StyleSet {
    Sketch {
        line: wgpu::RenderPipeline,
        scatter: wgpu::RenderPipeline,
        errorbar: wgpu::RenderPipeline,
        line_verts: u32,
    },
    Milkyway(data_render::MilkywaySet),
    Constellation(data_render::PointConstellationSet),
}

/// Every pipeline compiled against one render-target format: the four
/// precise pipelines (created eagerly, as before) plus a lazy per-style
/// cache. A style's set is compiled the first time the prepare phase of
/// `paint`/export sees an item drawn in that style; charts that stay precise
/// never pay for any styled compile. Rebuilding for a new target format
/// starts from an empty cache — the next prepare phase recompiles on use.
struct TargetPipelines {
    axis: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    scatter: wgpu::RenderPipeline,
    scatter_mapped: wgpu::RenderPipeline,
    pick_ring: wgpu::RenderPipeline,
    pick_ring_mapped: wgpu::RenderPipeline,
    errorbar: wgpu::RenderPipeline,
    errorbar_mapped: wgpu::RenderPipeline,
    sample_count: u32,
    styled: HashMap<StyleKey, StyleSet>,
}

fn create_target_pipelines(
    device: &wgpu::Device,
    texture_bgl: &wgpu::BindGroupLayout,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    per_point_style_map_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
) -> TargetPipelines {
    TargetPipelines {
        axis: data_render::create_fullscreen_textured_pipeline_with_sample_count(
            device,
            texture_bgl,
            surface_format,
            sample_count,
        ),
        line: data_render::create_line_columnar_pipeline_with_sample_count(
            device,
            transform_bgl,
            style_bgl,
            surface_format,
            sample_count,
        ),
        scatter: data_render::create_scatter_columnar_pipeline_with_sample_count(
            device,
            transform_bgl,
            style_bgl,
            surface_format,
            sample_count,
        ),
        scatter_mapped: data_render::create_scatter_columnar_mapped_pipeline(
            device,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            surface_format,
            sample_count,
        ),
        pick_ring: data_render::create_scatter_columnar_pipeline_with_entries(
            device,
            transform_bgl,
            style_bgl,
            surface_format,
            sample_count,
            "vs_pick_ring",
            "fs_pick_ring",
            "figgy picked point ring pipeline",
        ),
        pick_ring_mapped: data_render::create_scatter_columnar_mapped_pipeline_with_entries(
            device,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            surface_format,
            sample_count,
            "vs_pick_ring_mapped",
            "fs_pick_ring",
            "figgy picked point mapped ring pipeline",
        ),
        errorbar: data_render::create_errorbar_columnar_pipeline_with_sample_count(
            device,
            transform_bgl,
            style_bgl,
            surface_format,
            sample_count,
        ),
        errorbar_mapped: data_render::create_errorbar_columnar_mapped_pipeline(
            device,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            surface_format,
            sample_count,
        ),
        sample_count,
        styled: HashMap::new(),
    }
}

impl TargetPipelines {
    /// Prepare-phase ensure (`&mut`): compile and cache the pipeline set of
    /// every style that `items` draw with and the cache doesn't hold yet.
    /// Run before pass recording so the draw phase (`&self`) only looks up.
    #[allow(clippy::too_many_arguments)]
    fn ensure_styles_for_items(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        star_data_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        items: &[ChartDrawItem<'_>],
    ) {
        for item in items {
            let Some(v) = style_variant(&item.chart_config.draw_style) else {
                continue;
            };
            self.styled.entry(v.key).or_insert_with(|| match v.key {
                StyleKey::Sketch => StyleSet::Sketch {
                    line: data_render::create_line_columnar_pipeline_with_entries(
                        device,
                        transform_bgl,
                        style_bgl,
                        surface_format,
                        self.sample_count,
                        "vs_sketch",
                        "fs_main",
                        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                        wgpu::PrimitiveTopology::TriangleStrip,
                        None,
                        "figgy line styled pipeline",
                    ),
                    scatter: data_render::create_scatter_columnar_pipeline_with_entries(
                        device,
                        transform_bgl,
                        style_bgl,
                        surface_format,
                        self.sample_count,
                        "vs_sketch",
                        "fs_sketch",
                        "figgy scatter styled pipeline",
                    ),
                    errorbar: data_render::create_errorbar_columnar_pipeline_with_entries(
                        device,
                        transform_bgl,
                        style_bgl,
                        surface_format,
                        self.sample_count,
                        "vs_sketch",
                        "figgy errorbar styled pipeline",
                    ),
                    line_verts: data_render::LINE_SKETCH_VERTICES_PER_INSTANCE,
                },
                // Bakes the milkyway PSF + blackbody textures (once, then cached with
                // the pipelines) — docs/CONSTELLATION_DESIGN.md §3c.
                StyleKey::Milkyway => StyleSet::Milkyway(data_render::create_milkyway_set(
                    device,
                    queue,
                    transform_bgl,
                    style_bgl,
                    star_data_bgl,
                    surface_format,
                    self.sample_count,
                )),
                StyleKey::Constellation => {
                    StyleSet::Constellation(data_render::create_point_constellation_set(
                        device,
                        queue,
                        transform_bgl,
                        style_bgl,
                        surface_format,
                        self.sample_count,
                    ))
                }
            });
        }
    }

    /// Draw-phase lookup: the cached set for the item's style, `None` for
    /// precise mode. A cache miss (impossible once the prepare phase ran)
    /// also yields `None` — callers then fall back to the precise pipelines
    /// instead of panicking.
    fn style_set(&self, style: &DrawStyle) -> Option<&StyleSet> {
        style_variant(style).and_then(|v| self.styled.get(&v.key))
    }
}

fn validate_texture_extent(
    caps: RendererDeviceCaps,
    resource: &'static str,
    width: u32,
    height: u32,
) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(FiggyError::InvalidChartArea { width, height });
    }
    let max_dim = width.max(height);
    if max_dim > caps.max_texture_dimension_2d {
        return Err(FiggyError::GpuResourceLimit {
            resource,
            requested: max_dim as u64,
            limit: caps.max_texture_dimension_2d as u64,
        });
    }
    Ok(())
}

fn validate_buffer_size(caps: RendererDeviceCaps, resource: &'static str, size: u64) -> Result<()> {
    if size > caps.max_buffer_size {
        return Err(FiggyError::GpuResourceLimit {
            resource,
            requested: size,
            limit: caps.max_buffer_size,
        });
    }
    Ok(())
}

fn create_texture_checked(
    device: &wgpu::Device,
    desc: &wgpu::TextureDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Texture> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_texture(desc))).map_err(
        |_| FiggyError::GpuResourceAllocationFailed {
            resource,
            reason: "wgpu Device::create_texture panicked".into(),
        },
    )
}

fn create_buffer_checked(
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Buffer> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_buffer(desc))).map_err(
        |_| FiggyError::GpuResourceAllocationFailed {
            resource,
            reason: "wgpu Device::create_buffer panicked".into(),
        },
    )
}

fn premul_rgba(c: Color) -> [f32; 4] {
    let a = c.a.clamp(0.0, 1.0);
    [c.r * a, c.g * a, c.b * a, a]
}

fn scatter_style_slot_gpu(
    slot: &DataScatterPointStyleConfig,
    scale: f32,
    mask_color: u32,
    mask_radius: u32,
    mask_shape: u32,
) -> data_render::ScatterStyleSlotGpu {
    let mut mask = 0u32;
    let mut color_premul = [0.0; 4];
    let mut radius_px = 0.0;
    let mut shape_id = 0.0;
    if let Some(c) = slot.point_color {
        mask |= mask_color;
        color_premul = premul_rgba(c);
    }
    if let Some(size) = slot.point_size {
        mask |= mask_radius;
        radius_px = size * scale;
    }
    if let Some(shape) = &slot.point_shape {
        mask |= mask_shape;
        shape_id = data_render::shape_id(shape) as f32;
    }
    data_render::ScatterStyleSlotGpu {
        color_premul,
        meta: [radius_px, shape_id, mask as f32, 0.0],
    }
}

fn errorbar_style_slot_gpu(
    slot: &DataErrorBarPointStyleConfig,
    scale: f32,
    mask_color: u32,
    mask_stem_width: u32,
    mask_cap_half: u32,
    mask_cap_width: u32,
) -> data_render::ErrorBarStyleSlotGpu {
    let mut mask = 0u32;
    let mut color_premul = [0.0; 4];
    let mut stem_width = 0.0;
    let mut cap_half = 0.0;
    let mut cap_width = 0.0;
    if let Some(c) = slot.error_bar_color {
        mask |= mask_color;
        color_premul = premul_rgba(c);
    }
    if let Some(width) = slot.error_bar_width {
        mask |= mask_stem_width;
        stem_width = width * scale;
    }
    if let Some(size) = slot.error_bar_cap_size {
        mask |= mask_cap_half;
        cap_half = size * scale;
    }
    if let Some(width) = slot.cap_width {
        mask |= mask_cap_width;
        cap_width = width * scale;
    }
    data_render::ErrorBarStyleSlotGpu {
        color_premul,
        meta: [stem_width, cap_half, mask as f32, cap_width],
    }
}

fn picked_ref_matches_series(series: &SeriesConfig, picked: &PickedPointRef) -> bool {
    if picked.series_id != series.series_id {
        return false;
    }
    match picked.source_id.as_deref() {
        Some(source_id) => series.source_id.as_deref() == Some(source_id),
        None => true,
    }
}

struct MsaaTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

fn create_msaa_target(
    device: &wgpu::Device,
    caps: RendererDeviceCaps,
    label: &'static str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> Result<Option<MsaaTarget>> {
    if sample_count <= 1 {
        return Ok(None);
    }
    validate_target_sample_count(caps, format, sample_count)?;
    validate_texture_extent(caps, label, width, height)?;
    let desc = wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    };
    let texture = create_texture_checked(device, &desc, label)?;
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(Some(MsaaTarget {
        _texture: texture,
        view,
    }))
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
                if ov.index == point_index {
                    if let Some(size) = ov.style.point_size {
                        radius = size;
                    }
                }
            }
        }
    }
    radius.max(0.0)
}

fn scatter_pick_anchor_may_be_visible(
    scatter: &DataScatterStyleConfig,
    point_index: usize,
    use_style_mapping: bool,
) -> bool {
    (use_style_mapping && scatter.point_style_index_column.is_some())
        || scatter_config_radius_px(scatter, point_index, use_style_mapping) > 0.0
}

fn validate_registered_column(pool: &ColumnPool, id: &str) -> Result<()> {
    validate_host_column_id(id)?;
    if pool.slot(id).is_some() {
        Ok(())
    } else {
        Err(FiggyError::UnknownColumn { id: id.to_owned() })
    }
}

fn validate_host_column_id(id: &str) -> Result<()> {
    if id == INTERNAL_ZERO_COLUMN_ID {
        Err(FiggyError::ReservedColumnId { id: id.to_owned() })
    } else {
        Ok(())
    }
}

fn validate_axis_view_state(field: &'static str, axis: &AxisViewState) -> Result<()> {
    if !axis.min.is_finite() || !axis.max.is_finite() {
        return Err(FiggyError::InvalidConfig {
            field,
            reason: "axis bounds must be finite",
        });
    }
    if axis.max <= axis.min {
        return Err(FiggyError::InvalidConfig {
            field,
            reason: "axis max must be greater than min",
        });
    }
    if matches!(axis.scale, AxisScale::Logarithmic) && axis.min <= 0.0 {
        return Err(FiggyError::InvalidConfig {
            field,
            reason: "logarithmic axis bounds must be positive",
        });
    }
    Ok(())
}

fn validate_chart_view_state(view: &ChartViewState) -> Result<()> {
    if view.chart_area.width == 0 || view.chart_area.height == 0 {
        return Err(FiggyError::InvalidChartArea {
            width: view.chart_area.width,
            height: view.chart_area.height,
        });
    }
    validate_axis_view_state("top_x", &view.top_x)?;
    validate_axis_view_state("bottom_x", &view.bottom_x)?;
    validate_axis_view_state("left_y", &view.left_y)?;
    validate_axis_view_state("right_y", &view.right_y)
}

fn validate_error_ref_columns(pool: &ColumnPool, error: &ErrorRef) -> Result<()> {
    match error {
        ErrorRef::Symmetric { column } => validate_registered_column(pool, column),
        ErrorRef::Asymmetric { lower, upper } => {
            validate_registered_column(pool, lower)?;
            validate_registered_column(pool, upper)
        }
    }
}

fn error_ref_references_column(error: &ErrorRef, id: &str) -> bool {
    match error {
        ErrorRef::Symmetric { column } => column == id,
        ErrorRef::Asymmetric { lower, upper } => lower == id || upper == id,
    }
}

fn series_references_column(series: &SeriesConfig, id: &str) -> bool {
    if series.x_column == id || series.y_column == id {
        return true;
    }
    if extract_scatter(&series.render_type)
        .and_then(|scatter| scatter.point_style_index_column.as_deref())
        == Some(id)
    {
        return true;
    }
    if extract_errorbar_style(&series.render_type)
        .and_then(|errorbar| errorbar.error_bar_style_index_column.as_deref())
        == Some(id)
    {
        return true;
    }
    extract_err_x(&series.render_type).is_some_and(|error| error_ref_references_column(error, id))
        || extract_err_y(&series.render_type)
            .is_some_and(|error| error_ref_references_column(error, id))
}

fn data_series_declarations_equal(left: &[SeriesConfig], right: &[SeriesConfig]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.series_id == right.series_id
                && left.source_id == right.source_id
                && left.x_column == right.x_column
                && left.y_column == right.y_column
                && left.render_type == right.render_type
        })
}

fn visit_error_ref_columns(error: &ErrorRef, visit: &mut impl FnMut(&str)) {
    match error {
        ErrorRef::Symmetric { column } => visit(column),
        ErrorRef::Asymmetric { lower, upper } => {
            visit(lower);
            visit(upper);
        }
    }
}

fn visit_series_columns(series: &SeriesConfig, visit: &mut impl FnMut(&str)) {
    visit(&series.x_column);
    visit(&series.y_column);
    if let Some(column) = extract_scatter(&series.render_type)
        .and_then(|scatter| scatter.point_style_index_column.as_deref())
    {
        visit(column);
    }
    if let Some(column) = extract_errorbar_style(&series.render_type)
        .and_then(|errorbar| errorbar.error_bar_style_index_column.as_deref())
    {
        visit(column);
    }
    if let Some(error) = extract_err_x(&series.render_type) {
        visit_error_ref_columns(error, visit);
    }
    if let Some(error) = extract_err_y(&series.render_type) {
        visit_error_ref_columns(error, visit);
    }
}

fn validate_renderer_series(pool: &ColumnPool, series: &[SeriesConfig]) -> Result<()> {
    for (index, item) in series.iter().enumerate() {
        if item.series_id.is_empty() {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: item.series_id.clone(),
                reason: "series_id must not be empty".to_string(),
            });
        }
        if series[..index]
            .iter()
            .any(|earlier| earlier.series_id == item.series_id)
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: item.series_id.clone(),
                reason: "series_id must be unique within a chart".to_string(),
            });
        }

        validate_registered_column(pool, &item.x_column)?;
        validate_registered_column(pool, &item.y_column)?;
        if let Some(scatter) = extract_scatter(&item.render_type)
            && let Some(column) = scatter.point_style_index_column.as_deref()
        {
            validate_registered_column(pool, column)?;
        }
        if let Some(errorbar) = extract_errorbar_style(&item.render_type)
            && let Some(column) = errorbar.error_bar_style_index_column.as_deref()
        {
            validate_registered_column(pool, column)?;
        }
        if let Some(error) = extract_err_x(&item.render_type) {
            validate_error_ref_columns(pool, error)?;
        }
        if let Some(error) = extract_err_y(&item.render_type) {
            validate_error_ref_columns(pool, error)?;
        }
    }
    Ok(())
}

fn axis_fit_stamp(axis: &AxisOptions) -> (u8, u64, u64) {
    let scale = match axis.scale {
        AxisScale::Linear => 0,
        AxisScale::Logarithmic => 1,
    };
    (scale, axis.min.to_bits(), axis.max.to_bits())
}

fn push_fit_column_stamp(
    pool: &ColumnPool,
    columns: &mut Vec<FitColumnStamp>,
    role: u8,
    id: &str,
) -> Result<()> {
    let allocation_epoch = pool
        .allocation_epoch(id)
        .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })?;
    columns.push(FitColumnStamp {
        role,
        id: id.to_string(),
        allocation_epoch,
    });
    Ok(())
}

fn push_fit_error_stamps(
    pool: &ColumnPool,
    columns: &mut Vec<FitColumnStamp>,
    error: &ErrorRef,
    lower_role: u8,
    upper_role: u8,
) -> Result<()> {
    match error {
        ErrorRef::Symmetric { column } => {
            push_fit_column_stamp(pool, columns, lower_role, column)?;
            push_fit_column_stamp(pool, columns, upper_role, column)
        }
        ErrorRef::Asymmetric { lower, upper } => {
            push_fit_column_stamp(pool, columns, lower_role, lower)?;
            push_fit_column_stamp(pool, columns, upper_role, upper)
        }
    }
}

fn build_fit_token_from_state(
    pool: &ColumnPool,
    renderer_identity: u64,
    chart_id: ChartId,
    state: &ChartRenderState,
) -> Result<FitCommitToken> {
    let mut series = Vec::new();
    series
        .try_reserve(state.series.len())
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "fit series stamps",
            reason: error.to_string(),
        })?;
    for item in &state.series {
        let mut columns = Vec::new();
        columns
            .try_reserve(6)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "fit role-column stamps",
                reason: error.to_string(),
            })?;
        push_fit_column_stamp(pool, &mut columns, 0, &item.x_column)?;
        push_fit_column_stamp(pool, &mut columns, 1, &item.y_column)?;
        if let Some(error) = extract_err_x(&item.render_type) {
            push_fit_error_stamps(pool, &mut columns, error, 2, 3)?;
        }
        if let Some(error) = extract_err_y(&item.render_type) {
            push_fit_error_stamps(pool, &mut columns, error, 4, 5)?;
        }
        series.push(FitSeriesStamp {
            series_id: item.series_id.clone(),
            mode: crate::gpu_errorbar::GpuSeriesExtentMode::from_render_type(&item.render_type),
            columns,
        });
    }

    Ok(FitCommitToken {
        renderer_identity,
        chart_id,
        axes: FitAxisStamp {
            top_x: axis_fit_stamp(&state.config.top_x),
            bottom_x: axis_fit_stamp(&state.config.bottom_x),
            left_y: axis_fit_stamp(&state.config.left_y),
            right_y: axis_fit_stamp(&state.config.right_y),
        },
        series,
    })
}

fn build_web_derived_stamp_from_state(
    pool: &ColumnPool,
    renderer_identity: u64,
    chart_id: ChartId,
    state: &ChartRenderState,
    data_revision: RenderRevision,
) -> Result<WebDerivedStamp> {
    let mut ids: Vec<String> = Vec::new();
    ids.try_reserve(state.series.len().saturating_mul(8))
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "derived column identities",
            reason: error.to_string(),
        })?;
    for series in &state.series {
        visit_series_columns(series, &mut |column| {
            if !ids.iter().any(|existing| existing == column) {
                ids.push(column.to_string());
            }
        });
    }

    let mut columns = Vec::new();
    columns
        .try_reserve(ids.len())
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "derived column stamps",
            reason: error.to_string(),
        })?;
    for column in ids {
        let allocation_epoch = pool
            .allocation_epoch(&column)
            .ok_or_else(|| FiggyError::UnknownColumn { id: column.clone() })?;
        columns.push(ColumnContentStamp {
            id: column,
            allocation_epoch,
        });
    }

    Ok(WebDerivedStamp {
        renderer_identity,
        chart_id,
        view: state.revisions.view,
        data: data_revision,
        pool_layout_generation: pool.layout_generation(),
        columns,
    })
}

fn build_web_derived_snapshot_from_state(
    pool: &ColumnPool,
    renderer_identity: u64,
    chart_id: ChartId,
    state: &ChartRenderState,
    data_revision: RenderRevision,
) -> Result<WebDerivedSnapshot> {
    let stamp = build_web_derived_stamp_from_state(
        pool,
        renderer_identity,
        chart_id,
        state,
        data_revision,
    )?;
    let fit_token = build_fit_token_from_state(pool, renderer_identity, chart_id, state)?;
    let mut series = Vec::new();
    series
        .try_reserve(state.series.len())
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "web derived series snapshot",
            reason: error.to_string(),
        })?;
    series.extend(state.series.iter().cloned());
    Ok(WebDerivedSnapshot {
        stamp,
        fit_token,
        config: state.config.clone(),
        series,
    })
}

impl Renderer {
    /// Initialize every figgy GPU resource against the given device/queue pair.
    ///
    /// `pool_capacity_bytes` is the requested total size of the GPU column
    /// pool (sum of all chart data). It is clamped to what the device can
    /// bind — the arc-scan compute binds the whole pool as one storage
    /// binding, so a request above `max_storage_buffer_binding_size` (or
    /// `max_buffer_size`) is reduced instead of panicking later; read the
    /// granted size back via [`Self::pool`]`().capacity()`. To get a larger
    /// pool, raise those limits when requesting the wgpu device (the defaults
    /// are only 128 MiB / 256 MiB). `surface_format` is the final render
    /// target color format; every graphics pipeline is compiled against it.
    pub fn try_new(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
    ) -> Result<Self> {
        Self::try_new_with_sample_count(gpu, surface_format, pool_capacity_bytes, 1)
    }

    fn try_new_with_sample_count(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
        target_sample_count: u32,
    ) -> Result<Self> {
        let RendererDevice { device, queue } = gpu;
        let caps = RendererDeviceCaps::from_device(&device);
        // Downlevel (GL-class) adapters report compute limits below what the
        // dashed-line arc scan needs; creating its pipelines there is a
        // validation panic. Refuse early with a real error instead.
        if caps.max_compute_invocations_per_workgroup < data_render::line_arc::WG
            || caps.max_compute_workgroups_per_dimension == 0
        {
            return Err(FiggyError::GpuResourceLimit {
                resource: "compute workgroup (figgy requires WebGPU-class compute; \
                           GL-downlevel adapters are not supported)",
                requested: u64::from(data_render::line_arc::WG),
                limit: u64::from(caps.max_compute_invocations_per_workgroup),
            });
        }
        validate_target_sample_count(caps, surface_format, target_sample_count)?;

        // Clamp the pool to what the arc-scan / star-pass storage bindings
        // can address — graceful fallback instead of a first-dashed-line
        // panic. The caller can read the granted size back via `pool()`.
        let pool_capacity = effective_pool_capacity(pool_capacity_bytes, caps);
        if pool_capacity < pool_capacity_bytes {
            eprintln!(
                "[figgy] pool capacity {pool_capacity_bytes} B exceeds device limits \
                 (max_storage_buffer_binding_size {}, max_buffer_size {}); \
                 clamped to {pool_capacity} B",
                caps.max_storage_buffer_binding_size, caps.max_buffer_size
            );
        }
        let pool = ColumnPool::new(&device, pool_capacity)?;
        let errorbar_extent_engine = crate::gpu_errorbar::GpuErrorbarExtentEngine::new(&device);

        let texture_bgl = data_render::create_texture_bind_group_layout(&device);
        let transform_bgl = data_render::create_scatter_transform_bind_group_layout(&device);
        let style_bgl = data_render::create_style_bind_group_layout(&device);
        let per_point_style_map_bgl =
            data_render::create_per_point_style_map_bind_group_layout(&device);
        let star_data_bgl = data_render::create_star_data_bind_group_layout(&device);

        let sampler = data_render::create_linear_sampler(&device);
        let quad_vb = data_render::create_unit_centered_quad_vertex_buffer(&device);

        // Precise pipelines compile eagerly; styled variants compile lazily
        // in the prepare phase of the first `paint`/export that uses them.
        let pipelines = create_target_pipelines(
            &device,
            &texture_bgl,
            &transform_bgl,
            &style_bgl,
            &per_point_style_map_bgl,
            surface_format,
            target_sample_count,
        );
        let arc_pipelines = data_render::line_arc::create_arc_scan_pipelines(&device);
        let renderer_identity = issue_renderer_identity()?;

        Ok(Self {
            device,
            queue,
            caps,
            pool,
            chart_states: HashMap::new(),
            chart_order: Vec::new(),
            next_chart_id: 0,
            visual_revision: RenderRevision::initial(renderer_identity),
            renderer_identity,
            observed_font_generation: crate::text_render::font_generation(),
            pending_defrag: false,
            errorbar_extent_engine,
            target_sample_count,
            target_pipeline_generation: 1,
            texture_bgl,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            star_data_bgl,
            pipelines,
            sampler,
            quad_vb,
            arc_cache: HashMap::new(),
            arc_pipelines,
            #[cfg(test)]
            arc_chunk_override: None,
            space_bg: None,
            surface_format,
        })
    }

    // Handle / accessor methods.

    pub fn device(&self) -> &Arc<wgpu::Device> {
        &self.device
    }
    pub fn queue(&self) -> &Arc<wgpu::Queue> {
        &self.queue
    }
    pub fn pool(&self) -> &ColumnPool {
        &self.pool
    }
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    /// Current renderer-owned visual state identity.
    pub fn visual_revision(&self) -> RenderRevision {
        self.visual_revision
    }

    pub fn visual_stamp(&self) -> RendererVisualStamp {
        RendererVisualStamp {
            revision: self.visual_revision,
            font_generation: crate::text_render::font_generation(),
        }
    }

    /// Chart composition order. This slice, never `HashMap` iteration order,
    /// defines panel ordering for renderer-owned composition.
    pub fn chart_order(&self) -> &[ChartId] {
        &self.chart_order
    }

    pub fn chart_config(&self, id: ChartId) -> Result<&Config> {
        self.chart_states
            .get(&id)
            .map(|state| &state.config)
            .ok_or(FiggyError::UnknownChart { id })
    }

    pub fn chart_series(&self, id: ChartId) -> Result<&[SeriesConfig]> {
        self.chart_states
            .get(&id)
            .map(|state| state.series.as_slice())
            .ok_or(FiggyError::UnknownChart { id })
    }

    pub fn chart_selection(&self, id: ChartId) -> Result<Option<HitId>> {
        self.chart_states
            .get(&id)
            .map(|state| state.selected)
            .ok_or(FiggyError::UnknownChart { id })
    }

    pub fn chart_render_stamp(&self, id: ChartId) -> Result<ChartRenderStamp> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        Ok(ChartRenderStamp {
            renderer_identity: self.renderer_identity,
            chart_id: id,
            desired: state.revisions.desired,
            raster: state.revisions.raster,
        })
    }

    pub fn chart_view_state(&self, id: ChartId) -> Result<ChartViewState> {
        self.chart_states
            .get(&id)
            .map(|state| ChartViewState::from_config(&state.config))
            .ok_or(FiggyError::UnknownChart { id })
    }

    /// Register one renderer-owned chart. The id issuer and visual revision
    /// are checked before any state is published.
    pub fn register_chart(&mut self, config: Config, series: Vec<SeriesConfig>) -> Result<ChartId> {
        crate::chart::validate_renderer_config(&config)?;
        validate_renderer_series(&self.pool, &series)?;
        let id_value = self
            .next_chart_id
            .checked_add(1)
            .ok_or(FiggyError::CounterExhausted {
                counter: "chart id",
            })?;
        let visual_revision = self.visual_revision.successor("renderer visual revision")?;
        let chart_revision =
            RenderRevision::initial(self.renderer_identity).successor("chart revision")?;
        self.chart_states
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "chart registry",
                reason: error.to_string(),
            })?;
        self.chart_order
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "chart order",
                reason: error.to_string(),
            })?;

        let id = ChartId {
            renderer_identity: self.renderer_identity,
            sequence: id_value,
        };
        let previous = self.chart_states.insert(
            id,
            ChartRenderState {
                config,
                series,
                selected: None,
                revisions: ChartRevisions::initial(chart_revision),
            },
        );
        debug_assert!(previous.is_none());
        self.chart_order.push(id);
        self.next_chart_id = id_value;
        self.visual_revision = visual_revision;
        Ok(id)
    }

    pub fn remove_chart(&mut self, id: ChartId) -> Result<()> {
        if !self.chart_states.contains_key(&id) {
            return Err(FiggyError::UnknownChart { id });
        }
        let visual_revision = self.visual_revision.successor("renderer visual revision")?;
        self.chart_states.remove(&id);
        self.chart_order.retain(|candidate| *candidate != id);
        self.visual_revision = visual_revision;
        Ok(())
    }

    pub fn set_chart_config(&mut self, id: ChartId, config: Config) -> Result<()> {
        crate::chart::validate_renderer_config(&config)?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let desired = state
            .revisions
            .desired
            .successor("chart desired revision")?;
        let config_revision = state.revisions.config.successor("chart config revision")?;
        let view = if ChartViewState::from_config(&state.config)
            != ChartViewState::from_config(&config)
            || state.config.draw_style != config.draw_style
        {
            state.revisions.view.successor("chart view revision")?
        } else {
            state.revisions.view
        };
        let raster = state.revisions.raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;

        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        state.config = config;
        state.revisions.desired = desired;
        state.revisions.config = config_revision;
        state.revisions.view = view;
        state.revisions.raster = raster;
        self.visual_revision = visual;
        Ok(())
    }

    pub fn set_chart_series(&mut self, id: ChartId, series: Vec<SeriesConfig>) -> Result<()> {
        validate_renderer_series(&self.pool, &series)?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let desired = state
            .revisions
            .desired
            .successor("chart desired revision")?;
        let series_revision = state.revisions.series.successor("chart series revision")?;
        let data = if data_series_declarations_equal(&state.series, &series) {
            state.revisions.data
        } else {
            state.revisions.data.successor("chart data revision")?
        };
        let raster = state.revisions.raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;

        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        state.series = series;
        state.revisions.desired = desired;
        state.revisions.series = series_revision;
        state.revisions.data = data;
        state.revisions.raster = raster;
        self.visual_revision = visual;
        Ok(())
    }

    /// Atomically replace the renderer-owned config and ordered series.
    ///
    /// This is the mutation boundary for hosts whose one logical edit affects
    /// both declarations, such as an auto-managed legend update. Validation
    /// and every revision successor complete before either value is replaced.
    pub fn set_chart_state(
        &mut self,
        id: ChartId,
        config: Config,
        series: Vec<SeriesConfig>,
    ) -> Result<()> {
        crate::chart::validate_renderer_config(&config)?;
        validate_renderer_series(&self.pool, &series)?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let desired = state
            .revisions
            .desired
            .successor("chart desired revision")?;
        let config_revision = state.revisions.config.successor("chart config revision")?;
        let series_revision = state.revisions.series.successor("chart series revision")?;
        let data = if data_series_declarations_equal(&state.series, &series) {
            state.revisions.data
        } else {
            state.revisions.data.successor("chart data revision")?
        };
        let view = if ChartViewState::from_config(&state.config)
            != ChartViewState::from_config(&config)
            || state.config.draw_style != config.draw_style
        {
            state.revisions.view.successor("chart view revision")?
        } else {
            state.revisions.view
        };
        let raster = state.revisions.raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;

        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        state.config = config;
        state.series = series;
        state.revisions.desired = desired;
        state.revisions.config = config_revision;
        state.revisions.series = series_revision;
        state.revisions.data = data;
        state.revisions.view = view;
        state.revisions.raster = raster;
        self.visual_revision = visual;
        Ok(())
    }

    pub fn set_chart_view_state(&mut self, id: ChartId, view: ChartViewState) -> Result<()> {
        validate_chart_view_state(&view)?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let mut prospective_config = state.config.clone();
        view.apply_to(&mut prospective_config);
        crate::chart::validate_renderer_config(&prospective_config)?;
        let desired = state
            .revisions
            .desired
            .successor("chart desired revision")?;
        let config = state.revisions.config.successor("chart config revision")?;
        let view_revision = state.revisions.view.successor("chart view revision")?;
        let raster = state.revisions.raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;

        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        view.apply_to(&mut state.config);
        state.revisions.desired = desired;
        state.revisions.config = config;
        state.revisions.view = view_revision;
        state.revisions.raster = raster;
        self.visual_revision = visual;
        Ok(())
    }

    pub fn set_chart_selection(&mut self, id: ChartId, selected: Option<HitId>) -> Result<()> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let desired = state
            .revisions
            .desired
            .successor("chart desired revision")?;
        let selection = state
            .revisions
            .selection
            .successor("chart selection revision")?;
        let raster = state.revisions.raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;

        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        state.selected = selected;
        state.revisions.desired = desired;
        state.revisions.selection = selection;
        state.revisions.raster = raster;
        self.visual_revision = visual;
        Ok(())
    }

    /// Observe global font registration without adding a lock to frame or
    /// raster hot paths. All chart revision successors are preflighted before
    /// any state is changed.
    pub fn sync_external_invalidations(&mut self) -> Result<bool> {
        let generation = crate::text_render::font_generation();
        if generation == self.observed_font_generation {
            return Ok(false);
        }
        if self.chart_order.is_empty() {
            self.observed_font_generation = generation;
            return Ok(false);
        }

        let visual = self.visual_revision.successor("renderer visual revision")?;
        let mut updates = Vec::new();
        updates
            .try_reserve(self.chart_order.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "font invalidation revisions",
                reason: error.to_string(),
            })?;
        for id in &self.chart_order {
            let state = self
                .chart_states
                .get(id)
                .ok_or(FiggyError::UnknownChart { id: *id })?;
            updates.push((
                *id,
                state
                    .revisions
                    .desired
                    .successor("chart desired revision")?,
                state.revisions.raster.successor("chart raster revision")?,
            ));
        }

        for (id, desired, raster) in updates {
            let state = self
                .chart_states
                .get_mut(&id)
                .ok_or(FiggyError::UnknownChart { id })?;
            state.revisions.desired = desired;
            state.revisions.raster = raster;
        }
        self.visual_revision = visual;
        self.observed_font_generation = generation;
        Ok(true)
    }

    pub fn web_derived_snapshot(&self, id: ChartId) -> Result<WebDerivedSnapshot> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        build_web_derived_snapshot_from_state(
            &self.pool,
            self.renderer_identity,
            id,
            state,
            state.revisions.data,
        )
    }

    pub fn begin_fit_commit(&self, id: ChartId) -> Result<FitCommitToken> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        build_fit_token_from_state(&self.pool, self.renderer_identity, id, state)
    }

    pub fn commit_auto_fit_all_if_current(
        &mut self,
        token: &FitCommitToken,
        x_extent: &crate::chart::FitExtent,
        y_extent: &crate::chart::FitExtent,
        padding: f64,
    ) -> Result<()> {
        if token.renderer_identity != self.renderer_identity
            || !self.chart_states.contains_key(&token.chart_id)
        {
            return Err(FiggyError::StaleStateToken {
                reason: "fit token belongs to another renderer or a removed chart".to_string(),
            });
        }
        let state = self
            .chart_states
            .get(&token.chart_id)
            .ok_or(FiggyError::UnknownChart { id: token.chart_id })?;
        let current_fit =
            build_fit_token_from_state(&self.pool, self.renderer_identity, token.chart_id, state)?;
        if current_fit != *token {
            return Err(FiggyError::StaleStateToken {
                reason: "axis range/scale, ordered fit geometry, or source column content changed"
                    .to_string(),
            });
        }

        let state = self
            .chart_states
            .get(&token.chart_id)
            .ok_or(FiggyError::UnknownChart { id: token.chart_id })?;
        let desired = state
            .revisions
            .desired
            .successor("chart desired revision")?;
        let config = state.revisions.config.successor("chart config revision")?;
        let view = state.revisions.view.successor("chart view revision")?;
        let raster = state.revisions.raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;

        let state = self
            .chart_states
            .get_mut(&token.chart_id)
            .ok_or(FiggyError::UnknownChart { id: token.chart_id })?;
        crate::chart::apply_auto_fit_all(&mut state.config, x_extent, y_extent, padding);
        state.revisions.desired = desired;
        state.revisions.config = config;
        state.revisions.view = view;
        state.revisions.raster = raster;
        self.visual_revision = visual;
        Ok(())
    }

    fn prepare_column_invalidation(&self, id: &str) -> Result<ColumnInvalidationPlan> {
        let mut charts = Vec::new();
        charts
            .try_reserve(self.chart_order.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "column invalidation revisions",
                reason: error.to_string(),
            })?;
        for chart_id in &self.chart_order {
            let state = self
                .chart_states
                .get(chart_id)
                .ok_or(FiggyError::UnknownChart { id: *chart_id })?;
            if state
                .series
                .iter()
                .any(|series| series_references_column(series, id))
            {
                charts.push(ChartColumnInvalidation {
                    id: *chart_id,
                    desired: state
                        .revisions
                        .desired
                        .successor("chart desired revision")?,
                    data: state.revisions.data.successor("chart data revision")?,
                });
            }
        }
        let visual = if charts.is_empty() {
            None
        } else {
            Some(self.visual_revision.successor("renderer visual revision")?)
        };
        Ok(ColumnInvalidationPlan { charts, visual })
    }

    fn prepare_column_removal(&self, id: &str) -> Result<ColumnRemovalPlan> {
        let mut charts = Vec::new();
        charts
            .try_reserve(self.chart_order.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "column removal candidates",
                reason: error.to_string(),
            })?;
        for chart_id in &self.chart_order {
            let state = self
                .chart_states
                .get(chart_id)
                .ok_or(FiggyError::UnknownChart { id: *chart_id })?;
            let removed_count = state
                .series
                .iter()
                .filter(|series| series_references_column(series, id))
                .count();
            if removed_count == 0 {
                continue;
            }

            let desired = state
                .revisions
                .desired
                .successor("chart desired revision")?;
            let series_revision = state.revisions.series.successor("chart series revision")?;
            let data = state.revisions.data.successor("chart data revision")?;
            let raster = state.revisions.raster.successor("chart raster revision")?;
            charts.push(ChartSeriesRemoval {
                id: *chart_id,
                desired,
                series_revision,
                data,
                raster,
            });
        }
        let visual = if charts.is_empty() {
            None
        } else {
            Some(self.visual_revision.successor("renderer visual revision")?)
        };
        Ok(ColumnRemovalPlan { charts, visual })
    }

    /// Rebuild render-target pipelines if the host swap-chain format changed.
    ///
    /// Column data, chart views, textures, bind groups, styles, and buffers do
    /// not depend on the final color attachment format, so embedders can call
    /// this instead of tearing down the whole renderer state.
    pub fn ensure_target_format(&mut self, surface_format: wgpu::TextureFormat) -> Result<bool> {
        self.ensure_target(surface_format, self.target_sample_count)
    }

    fn ensure_target(
        &mut self,
        surface_format: wgpu::TextureFormat,
        target_sample_count: u32,
    ) -> Result<bool> {
        if self.surface_format == surface_format && self.target_sample_count == target_sample_count
        {
            return Ok(false);
        }
        validate_target_sample_count(self.caps, surface_format, target_sample_count)?;
        let target_pipeline_generation =
            self.target_pipeline_generation
                .checked_add(1)
                .ok_or(FiggyError::CounterExhausted {
                    counter: "target pipeline generation",
                })?;
        // Fresh set with an empty styled cache — any styled pipelines the
        // next frame needs are recompiled by its prepare phase.
        self.pipelines = create_target_pipelines(
            &self.device,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            surface_format,
            target_sample_count,
        );
        self.surface_format = surface_format;
        self.target_sample_count = target_sample_count;
        self.target_pipeline_generation = target_pipeline_generation;
        Ok(true)
    }

    /// Block until submitted work on this device has completed.
    ///
    /// This is useful during shutdown in embedder examples, where dropping
    /// callback-owned GPU resources after the host surface/window has started
    /// tearing down can expose backend or driver lifetime bugs.
    ///
    /// No-op on wasm: the browser drives the device, blocking waits don't
    /// exist there, and the shutdown races this guards against are native
    /// driver concerns.
    pub fn wait_idle(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        }
    }

    pub fn texture_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.texture_bgl
    }
    pub fn transform_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.transform_bgl
    }
    pub fn style_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.style_bgl
    }

    pub fn axis_pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipelines.axis
    }
    pub fn line_pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipelines.line
    }
    pub fn scatter_pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipelines.scatter
    }
    pub fn errorbar_pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipelines.errorbar
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }
    pub fn quad_vertex_buffer(&self) -> &wgpu::Buffer {
        &self.quad_vb
    }

    // Column management (returns Result).

    /// Add a column to the pool. `AllocError` is converted to `FiggyError::Pool`.
    pub fn add_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        let id = id.into();
        validate_host_column_id(&id)?;
        Ok(self
            .pool
            .add_column(id, source, &self.device, &self.queue)?)
    }

    pub fn add_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        let id = id.into();
        validate_host_column_id(&id)?;
        Ok(self
            .pool
            .add_hilo_column(id, source, &self.device, &self.queue)?)
    }

    /// Begin a failure-atomic scalar insert or same-id replacement.
    pub fn begin_upsert_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        let id = id.into();
        validate_host_column_id(&id)?;
        let invalidation = self.prepare_column_invalidation(&id)?;
        let inner =
            self.pool
                .begin_upsert_column(id, source, self.device.as_ref(), self.queue.as_ref())?;
        Ok(RendererColumnUpsert {
            inner: Some(inner),
            chart_states: &mut self.chart_states,
            visual_revision: &mut self.visual_revision,
            pending_defrag: &mut self.pending_defrag,
            invalidation: Some(invalidation),
            renderer_identity: self.renderer_identity,
            device: self.device.as_ref(),
            queue: self.queue.as_ref(),
            errorbar_extent_engine: &self.errorbar_extent_engine,
        })
    }

    /// Begin a failure-atomic hi/lo insert or same-id replacement.
    pub fn begin_upsert_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        let id = id.into();
        validate_host_column_id(&id)?;
        let invalidation = self.prepare_column_invalidation(&id)?;
        let inner = self.pool.begin_upsert_hilo_column(
            id,
            source,
            self.device.as_ref(),
            self.queue.as_ref(),
        )?;
        Ok(RendererColumnUpsert {
            inner: Some(inner),
            chart_states: &mut self.chart_states,
            visual_revision: &mut self.visual_revision,
            pending_defrag: &mut self.pending_defrag,
            invalidation: Some(invalidation),
            renderer_identity: self.renderer_identity,
            device: self.device.as_ref(),
            queue: self.queue.as_ref(),
            errorbar_extent_engine: &self.errorbar_extent_engine,
        })
    }

    /// Failure-atomic scalar upsert without an intermediate batch-preparation
    /// step.  Web integrations that rebuild picker/errorbar state should use
    /// [`Self::begin_upsert_column`] and commit its guard explicitly.
    pub fn upsert_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        Ok(self.begin_upsert_column(id, source)?.commit())
    }

    /// Failure-atomic hi/lo upsert without an intermediate batch-preparation
    /// step.
    pub fn upsert_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        Ok(self.begin_upsert_hilo_column(id, source)?.commit())
    }

    /// Ensure the renderer-owned filler column can cover `len` points.
    ///
    /// The column grows only when necessary and is uploaded without a
    /// temporary scalar vector. It is maintenance data, not a public chart
    /// column or a second source of truth.
    pub fn ensure_internal_zero_column(&mut self, len: usize) -> Result<()> {
        if len == 0
            || self
                .pool
                .slot(INTERNAL_ZERO_COLUMN_ID)
                .is_some_and(|slot| slot.len_values >= len)
        {
            return Ok(());
        }
        let replaced_existing = self.pool.slot(INTERNAL_ZERO_COLUMN_ID).is_some();
        self.pool
            .begin_upsert_column(
                INTERNAL_ZERO_COLUMN_ID.to_string(),
                &InternalZeroColumn { len },
                self.device.as_ref(),
                self.queue.as_ref(),
            )?
            .commit();
        if replaced_existing {
            self.pending_defrag = true;
        }
        Ok(())
    }

    /// Remove a column and cascade-remove every renderer-owned series that
    /// references it, so no registered chart can retain a freed data id.
    ///
    /// The core renderer does not rewrite `Config::legend`; legend document
    /// policy belongs to the host. The web wrapper removes rows for its
    /// auto-managed legend and preserves freely edited legend text.
    pub fn remove_column(&mut self, id: &str) -> Result<bool> {
        validate_host_column_id(id)?;
        if self.pool.slot(id).is_none() {
            return Ok(false);
        }
        let removal = self.prepare_column_removal(id)?;
        if !self.pool.remove_column(id)? {
            return Ok(false);
        }
        removal.publish(&mut self.chart_states, &mut self.visual_revision, id);
        self.pending_defrag = true;
        Ok(true)
    }

    /// Compact every live column to the start of the pool. `true` iff
    /// anything actually moved.
    pub fn defragment(&mut self) -> Result<bool> {
        let moved = self.pool.defragment(&self.device, &self.queue)?;
        self.pending_defrag = false;
        Ok(moved)
    }

    pub fn has_pending_maintenance(&self) -> bool {
        self.pending_defrag
    }

    pub fn process_pending_maintenance(&mut self) -> Result<bool> {
        if !self.pending_defrag {
            return Ok(false);
        }
        let moved = self.pool.defragment(&self.device, &self.queue)?;
        self.pending_defrag = false;
        Ok(moved)
    }

    pub fn set_defrag_policy(&mut self, policy: DefragPolicy) {
        self.pool.defrag_policy = policy;
    }

    pub fn handle_for(&self, id: &str) -> Result<ColumnHandle> {
        self.pool
            .handle_for(id)
            .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })
    }

    pub fn is_valid_handle(&self, h: &ColumnHandle) -> bool {
        self.pool.is_valid_handle(h)
    }

    /// Submit one exact errorbar-inclusive extent reduction from the current
    /// GPU column-pool mappings. The returned ticket owns its readback and
    /// does not borrow this renderer.
    pub fn begin_errorbar_extent(
        &self,
        value_id: &str,
        lower_id: &str,
        upper_id: &str,
    ) -> std::result::Result<
        crate::gpu_errorbar::GpuErrorbarExtentTicket,
        crate::gpu_errorbar::GpuErrorbarError,
    > {
        begin_errorbar_extent_from_pool(
            &self.errorbar_extent_engine,
            &self.device,
            &self.queue,
            &self.pool,
            value_id,
            lower_id,
            upper_id,
        )
    }

    /// Submit one exact x/y extent reduction over the primitive rows that
    /// the current series mode can draw. The returned ticket owns its
    /// readback and does not borrow this renderer.
    pub fn begin_series_extent(
        &self,
        mode: crate::gpu_errorbar::GpuSeriesExtentMode,
        columns: crate::gpu_errorbar::GpuSeriesExtentColumnIds<'_>,
    ) -> std::result::Result<
        crate::gpu_errorbar::GpuSeriesExtentTicket,
        crate::gpu_errorbar::GpuErrorbarError,
    > {
        begin_series_extent_from_pool(
            &self.errorbar_extent_engine,
            &self.device,
            &self.queue,
            &self.pool,
            mode,
            columns,
        )
    }

    // Standalone path — figgy owns instance / adapter / device / queue / surface.
    //
    // For hosts where figgy renders the window directly (e.g. winit). Caller
    // writes zero lines of wgpu setup code.

    /// Build every GPU resource plus a surface and swap chain from a window
    /// handle. The returned [`WindowedRenderer`] manages the surface and
    /// expects `draw` to be called once per frame.
    ///
    /// `target` can be anything convertible to `wgpu::SurfaceTarget`
    /// (`Arc<winit::Window>`, a raw-handle wrapper, a web canvas, …). The
    /// figgy crate does not depend on winit types itself. On wasm, drive this
    /// from the host event loop (e.g. `wasm_bindgen_futures::spawn_local`).
    pub async fn for_window_async<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
    ) -> Result<WindowedRenderer<'w>> {
        let instance = data_render::create_instance();
        let surface = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            data_render::create_surface_for_window(&instance, target)
        }))
        .map_err(|_| FiggyError::SurfaceCreationFailed {
            reason:
                "wgpu surface creation panicked; check platform window/canvas/thread constraints"
                    .into(),
        })?
        .map_err(|e| FiggyError::SurfaceCreationFailed {
            reason: format!("{e}"),
        })?;
        let adapter = data_render::request_adapter_for_surface_async(&instance, &surface)
            .await
            .map_err(|_| FiggyError::AdapterUnavailable)?;
        let (device, queue) = data_render::request_device_async(&adapter)
            .await
            .map_err(|e| FiggyError::DeviceCreationFailed {
                reason: format!("{e}"),
            })?;
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let surface_config = data_render::try_configure_surface(
            &surface,
            &adapter,
            &device,
            size.0.max(1),
            size.1.max(1),
        )?;

        let caps = RendererDeviceCaps::from_device(&device);
        let preferred_sample_count = preferred_msaa_sample_count(caps, surface_config.format);
        let (target_sample_count, msaa_target) = match create_msaa_target(
            &device,
            caps,
            "figgy frame msaa target",
            surface_config.width,
            surface_config.height,
            surface_config.format,
            preferred_sample_count,
        ) {
            Ok(target) => (preferred_sample_count, target),
            Err(_) if preferred_sample_count > 1 => (1, None),
            Err(e) => return Err(e),
        };

        let inner = Renderer::try_new_with_sample_count(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            surface_config.format,
            pool_capacity_bytes,
            target_sample_count,
        )?;

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

    /// Build a `ChartStyle` from `cfg.render_type`'s line/scatter/errorbar
    /// styles. Missing components default to BLACK / 1 px.
    pub fn create_style_for_series(&self, cfg: &SeriesConfig) -> ChartStyle {
        self.create_style_for_series_scaled(cfg, 1.0)
    }

    /// Scaled variant of `create_style_for_series` — every pixel size (line
    /// width, dash lengths, point radius, errorbar widths/cap) is multiplied
    /// by `scale` (used by the high-DPI export path).
    pub fn create_style_for_series_scaled(&self, cfg: &SeriesConfig, scale: f32) -> ChartStyle {
        // Decorrelates per-series hash patterns in the styled shader entries
        // (star placement, sketch wobble). Derived from the series id, so the
        // result stays deterministic for a given config; renaming a series
        // re-rolls its pattern.
        let series_salt = crate::sketch::fnv1a(&cfg.series_id);
        let line = match extract_line(&cfg.render_type) {
            Some(l) => {
                let mut s =
                    PrimitiveStyle::from_color_with_width(l.line_color, l.line_width * scale);
                let pattern = l.line_style.pattern();
                // GPU dash capacity is 8 scalars (2 × vec4); presets max out at 6.
                for (i, len) in pattern.iter().take(8).enumerate() {
                    s.dash[i / 4][i % 4] = len * scale;
                }
                s.dash_len = pattern.len().min(8) as u32;
                s
            }
            None => PrimitiveStyle::from_color_with_width(Color::BLACK, 1.0),
        };
        let scatter = match extract_scatter(&cfg.render_type) {
            Some(sc) => {
                let mut s = PrimitiveStyle::from_color(sc.point_color);
                s.point_radius_px = sc.point_size * scale;
                s.shape_id = data_render::shape_id(&sc.point_shape);
                s
            }
            None => PrimitiveStyle::from_color(Color::BLACK),
        };
        let errorbar = match extract_errorbar_style(&cfg.render_type) {
            Some(e) => {
                let mut s = PrimitiveStyle::from_color_with_width(
                    e.error_bar_color,
                    e.error_bar_width * scale,
                );
                s.cap_half_px = e.error_bar_cap_size * scale;
                s.cap_width_px = e.cap_width * scale;
                s
            }
            None => PrimitiveStyle::from_color(Color::BLACK),
        };
        let (mut line, mut scatter, mut errorbar) = (line, scatter, errorbar);
        line.series_salt = series_salt;
        scatter.series_salt = series_salt;
        errorbar.series_salt = series_salt;
        let scatter_map = extract_scatter(&cfg.render_type)
            .and_then(|sc| self.create_scatter_style_map_for(sc, scale));
        let errorbar_map = extract_errorbar_style(&cfg.render_type)
            .and_then(|eb| self.create_errorbar_style_map_for(eb, scale));
        self.create_style_from_primitives(line, scatter, errorbar, scatter_map, errorbar_map)
    }

    /// Shared tail of `create_style_for_series*`: allocate the three style
    /// uniform buffers and bind groups from fully-built values.
    fn create_style_from_primitives(
        &self,
        line: PrimitiveStyle,
        scatter: PrimitiveStyle,
        errorbar: PrimitiveStyle,
        scatter_map: Option<data_render::ScatterStyleMap>,
        errorbar_map: Option<data_render::ErrorBarStyleMap>,
    ) -> ChartStyle {
        let dev = &self.device;
        let scatter_radius_px = scatter.point_radius_px;
        let line_buf = data_render::create_style_uniform_buffer(dev, &line);
        let sc_buf = data_render::create_style_uniform_buffer(dev, &scatter);
        let eb_buf = data_render::create_style_uniform_buffer(dev, &errorbar);
        ChartStyle {
            line_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &line_buf),
            scatter_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &sc_buf),
            errorbar_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &eb_buf),
            scatter_map,
            errorbar_map,
            scatter_radius_px,
        }
    }

    fn create_scatter_style_map_for(
        &self,
        scatter: &DataScatterStyleConfig,
        scale: f32,
    ) -> Option<data_render::ScatterStyleMap> {
        const MASK_COLOR: u32 = 1;
        const MASK_RADIUS: u32 = 2;
        const MASK_SHAPE: u32 = 4;

        let has_index = scatter.point_style_index_column.is_some();
        let style_slots: Vec<data_render::ScatterStyleSlotGpu> = scatter
            .point_style_table
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|slot| scatter_style_slot_gpu(slot, scale, MASK_COLOR, MASK_RADIUS, MASK_SHAPE))
            .collect();
        let overrides: Vec<data_render::ScatterStyleOverrideGpu> = scatter
            .point_style_overrides
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter_map(|ov| {
                let point_index = u32::try_from(ov.index).ok()?;
                let slot =
                    scatter_style_slot_gpu(&ov.style, scale, MASK_COLOR, MASK_RADIUS, MASK_SHAPE);
                Some(data_render::ScatterStyleOverrideGpu {
                    point_index,
                    _pad: [0; 3],
                    color_premul: slot.color_premul,
                    meta: slot.meta,
                })
            })
            .collect();

        if !has_index && style_slots.is_empty() && overrides.is_empty() {
            return None;
        }
        Some(data_render::create_scatter_style_map(
            &self.device,
            &self.per_point_style_map_bgl,
            &style_slots,
            &overrides,
            data_render::ScatterStyleMapMeta {
                style_count: style_slots.len().min(u32::MAX as usize) as u32,
                override_count: overrides.len().min(u32::MAX as usize) as u32,
                has_index: u32::from(has_index),
                _pad: 0,
            },
        ))
    }

    fn create_errorbar_style_map_for(
        &self,
        errorbar: &DataErrorBarStyleConfig,
        scale: f32,
    ) -> Option<data_render::ErrorBarStyleMap> {
        const MASK_COLOR: u32 = 1;
        const MASK_STEM_WIDTH: u32 = 2;
        const MASK_CAP_HALF: u32 = 4;
        const MASK_CAP_WIDTH: u32 = 8;

        let has_index = errorbar.error_bar_style_index_column.is_some();
        let style_slots: Vec<data_render::ErrorBarStyleSlotGpu> = errorbar
            .error_bar_style_table
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|slot| {
                errorbar_style_slot_gpu(
                    slot,
                    scale,
                    MASK_COLOR,
                    MASK_STEM_WIDTH,
                    MASK_CAP_HALF,
                    MASK_CAP_WIDTH,
                )
            })
            .collect();
        let overrides: Vec<data_render::ErrorBarStyleOverrideGpu> = errorbar
            .error_bar_style_overrides
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter_map(|ov| {
                let point_index = u32::try_from(ov.index).ok()?;
                let slot = errorbar_style_slot_gpu(
                    &ov.style,
                    scale,
                    MASK_COLOR,
                    MASK_STEM_WIDTH,
                    MASK_CAP_HALF,
                    MASK_CAP_WIDTH,
                );
                Some(data_render::ErrorBarStyleOverrideGpu {
                    point_index,
                    _pad: [0; 3],
                    color_premul: slot.color_premul,
                    meta: slot.meta,
                })
            })
            .collect();

        if !has_index && style_slots.is_empty() && overrides.is_empty() {
            return None;
        }
        Some(data_render::create_errorbar_style_map(
            &self.device,
            &self.per_point_style_map_bgl,
            &style_slots,
            &overrides,
            data_render::ErrorBarStyleMapMeta {
                style_count: style_slots.len().min(u32::MAX as usize) as u32,
                override_count: overrides.len().min(u32::MAX as usize) as u32,
                has_index: u32::from(has_index),
                _pad: 0,
            },
        ))
    }

    /// Build the per-panel GPU resources: grid + decoration textures and
    /// bind groups, plus the transform uniform buffer + bind group.
    /// `panel_rect` is the panel's pixel rect in surface coordinates.
    /// `chart.config_mut().chart_area` must match `panel_rect` before this
    /// call so the axis raster is drawn correctly.
    pub fn create_chart_view(&self, chart: &Chart, panel_rect: Rect) -> Result<ChartView> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);

        // Grid layer (drawn below data): raster + texture + bind group.
        let grid_rgba = axis_render::try_raster_chart_layer_to_rgba(
            chart.config(),
            axis_render::AxisLayerKind::Grid,
        )?;
        let grid_tex = data_render::upload_rgba_texture(
            &self.device,
            &self.queue,
            self.caps.max_texture_dimension_2d,
            w,
            h,
            &grid_rgba,
        )?;
        let grid_view_t = grid_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let grid_bg = data_render::create_texture_bind_group(
            &self.device,
            &self.texture_bgl,
            &grid_view_t,
            &self.sampler,
        );

        // Decoration layer (drawn above data).
        let dec_rgba = axis_render::try_raster_chart_layer_to_rgba(
            chart.config(),
            axis_render::AxisLayerKind::Decoration,
        )?;
        let dec_tex = data_render::upload_rgba_texture(
            &self.device,
            &self.queue,
            self.caps.max_texture_dimension_2d,
            w,
            h,
            &dec_rgba,
        )?;
        let dec_view_t = dec_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let dec_bg = data_render::create_texture_bind_group(
            &self.device,
            &self.texture_bgl,
            &dec_view_t,
            &self.sampler,
        );

        let t = data_render::scatter_transform_from_config(chart.config());
        let transform_buffer =
            data_render::create_scatter_transform_uniform_buffer(&self.device, &t);
        let transform_bg = data_render::create_scatter_transform_bind_group(
            &self.device,
            &self.transform_bgl,
            &transform_buffer,
        );

        Ok(ChartView {
            grid_texture: grid_tex,
            grid_bind_group: grid_bg,
            decoration_texture: dec_tex,
            decoration_bind_group: dec_bg,
            transform_buffer,
            transform_bg,
            content_revision: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            panel_rect,
            grid_space_gen: None,
        })
    }

    /// When only the axis range (`AxisOptions { scale, min, max }`) changes,
    /// only the transform uniform buffer needs to be rewritten. No CPU
    /// raster, no texture rebuild — one GPU write.
    pub fn update_transform(&mut self, view: &ChartView, chart: &Chart) -> Result<()> {
        let t = data_render::scatter_transform_from_config(chart.config());
        view.advance_content_revision()?;
        data_render::update_scatter_transform(&self.queue, &view.transform_buffer, &t);
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
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);

        let dec_rgba = axis_render::try_raster_chart_layer_to_rgba_with_selection(
            chart.config(),
            axis_render::AxisLayerKind::Decoration,
            selection,
        )?;
        view.advance_content_revision()?;

        // Grid layer. The constellation backdrop is axis-range independent,
        // so it is cached by (size, nebula, dust, seed): a pan/zoom/config
        // commit re-rasters only the decoration layer, and a view whose grid
        // texture already holds the cached generation skips the upload too.
        if let crate::config::DrawStyle::Milkyway(c) = &chart.config().draw_style {
            let key = SpaceBgKey {
                w,
                h,
                nebula: c.nebula.to_bits(),
                dust: c.dust.to_bits(),
                seed: c.seed,
            };
            if !self.space_bg.as_ref().is_some_and(|s| s.key == key) {
                let rgba = axis_render::try_raster_chart_layer_to_rgba(
                    chart.config(),
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
            }
        } else {
            let grid_rgba = axis_render::try_raster_chart_layer_to_rgba(
                chart.config(),
                axis_render::AxisLayerKind::Grid,
            )?;
            self.refresh_one_layer(
                &mut view.grid_texture,
                &mut view.grid_bind_group,
                &grid_rgba,
                w,
                h,
            )?;
            view.grid_space_gen = None;
        }

        self.refresh_one_layer(
            &mut view.decoration_texture,
            &mut view.decoration_bind_group,
            &dec_rgba,
            w,
            h,
        )?;
        view.panel_rect = panel_rect;

        Ok(())
    }

    /// Update one texture in place or by reallocation. Used for both
    /// the grid and decoration layers.
    fn refresh_one_layer(
        &self,
        tex: &mut wgpu::Texture,
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
            let new_tex = data_render::upload_rgba_texture(
                &self.device,
                &self.queue,
                self.caps.max_texture_dimension_2d,
                w,
                h,
                rgba,
            )?;
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

    /// Mutable half of the frame: finish every state change the draw needs,
    /// before any render pass opens.
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
    /// `ensure_target_format`, and `refresh_axis`; before the host submits
    /// the command buffer containing the render pass. The returned token owns
    /// the resolved draw input. `paint_prepared` therefore receives no second
    /// copy of chart/view/series/style inputs. It rejects renderer resource
    /// changes that invalidate the captured GPU work: target-pipeline rebuild,
    /// pool layout change, replacement of any captured column allocation, or
    /// another transform/axis write to a captured `ChartView`.
    pub fn prepare(&mut self, items: &[ChartDrawItem<'_>]) -> Result<PreparedFrame> {
        self.pipelines.ensure_styles_for_items(
            &self.device,
            &self.queue,
            &self.transform_bgl,
            &self.style_bgl,
            &self.star_data_bgl,
            self.surface_format,
            items,
        );
        let prepared_arcs = self.prepare_arc_items(items)?;
        let (items, column_sources) =
            self.resolve_prepared_items(items, &prepared_arcs, &self.pipelines)?;
        Ok(PreparedFrame {
            renderer_identity: self.renderer_identity,
            items,
            column_sources,
            pool_layout_generation: self.pool.layout_generation(),
            target_pipeline_generation: self.target_pipeline_generation,
        })
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
    /// is a fresh `prepare`.
    pub fn paint_prepared(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        prepared: &PreparedFrame,
    ) -> Result<()> {
        self.validate_prepared(prepared)?;
        self.paint_prepared_items(pass, target_size, &prepared.items)
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
        }
        for (id, allocation_epoch) in &prepared.column_sources {
            if self.pool.allocation_epoch(id) != Some(*allocation_epoch) {
                return Err(stale(format!(
                    "column {:?} changed allocation between prepare and paint_prepared",
                    id
                )));
            }
        }
        Ok(())
    }

    /// Per-item body of the prepare phase (also reused by export). For each
    /// panel: computes ONE transform snapshot from `item.chart_config`,
    /// writes it to the panel's transform uniform, dispatches the arc-length
    /// scan of every line series that needs one with that same snapshot —
    /// dashed lines (dash phase) and every line of a style with
    /// `needs_arc_prefix` (sketch: the wobble is parameterized by arc
    /// length, so solid sketch lines need the prefix too). The immutable draw
    /// phase later reads the owned result, never the live `arc_cache`.
    fn prepare_arc_items(&mut self, items: &[ChartDrawItem<'_>]) -> Result<Vec<PreparedArcItem>> {
        let mut prepared = Vec::with_capacity(items.len());
        for item in items {
            let t = data_render::scatter_transform_from_config(item.chart_config);
            // Same snapshot for both GPU consumers of the transform: the
            // uniform the vertex shaders read and the arc dispatch below.
            // Written here (not via `update_transform`) so they cannot
            // desync within a frame.
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
                let line = has_line(&cfg.render_type);
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
        pipelines: &TargetPipelines,
    ) -> Result<(Vec<PreparedItem>, HashMap<ColumnId, u64>)> {
        let mut prepared = Vec::with_capacity(items.len());
        let mut column_sources = HashMap::new();

        for (item, prepared_arc) in items.iter().zip(prepared_arcs) {
            let styled = pipelines.style_set(&item.chart_config.draw_style);
            let (layers, sources) = self.build_series_layers(
                item.view,
                item.chart_config,
                item.series,
                pipelines,
                &prepared_arc.series,
                styled,
            )?;

            for (id, allocation_epoch) in sources {
                column_sources.entry(id).or_insert(allocation_epoch);
            }

            prepared.push(PreparedItem {
                view: PreparedView {
                    grid_bind_group: item.view.grid_bind_group.clone(),
                    decoration_bind_group: item.view.decoration_bind_group.clone(),
                    content_revision: Arc::clone(&item.view.content_revision),
                    expected_content_revision: prepared_arc.view_revision,
                    panel_rect: item.view.panel_rect,
                },
                data_area: item
                    .chart_config
                    .data_area()
                    .map(|area| area.0)
                    .unwrap_or(item.view.panel_rect),
                axis_pipeline: pipelines.axis.clone(),
                series: layers
                    .into_iter()
                    .map(PreparedSeries::from_layers)
                    .collect(),
            });
        }
        Ok((prepared, column_sources))
    }

    fn paint_prepared_items<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        prepared: &'a [PreparedItem],
    ) -> Result<()> {
        for item in prepared {
            let panel_rect = item.view.panel_rect;
            let series_list: Vec<_> = item.series.iter().map(PreparedSeries::layers).collect();

            // The single style decision per panel: the chart's `DrawStyle`
            // resolves to a cached styled pipeline set (compiled by the
            // prepare phase) or `None` for precise — the precise path stays
            // untouched.
            // Bundle every series's primitives for the panel into one call.

            data_render::draw_chart_panel_columnar(
                pass,
                target_size,
                panel_rect,
                item.data_area,
                AxisLayer {
                    pipeline: &item.axis_pipeline,
                    bind_group: &item.view.grid_bind_group,
                },
                &series_list,
                AxisLayer {
                    pipeline: &item.axis_pipeline,
                    bind_group: &item.view.decoration_bind_group,
                },
            );
        }
        Ok(())
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
        prepared: &'a [PreparedArcSeries],
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

        // Resolve the style set into per-primitive picks once. `stars` is the
        // constellation's arc-driven indirect star pass over the same polyline.
        struct LinePick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            verts: u32,
            texture_bg: Option<&'p wgpu::BindGroup>,
        }
        struct StarsPick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            texture_bg: &'p wgpu::BindGroup,
        }
        struct ScatterPick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            texture_bg: Option<&'p wgpu::BindGroup>,
        }
        let (line_pick, stars_pick, scatter_pick, errorbar_pipe) = match styled {
            None => (
                LinePick {
                    pipeline: &pipelines.line,
                    verts: 4,
                    texture_bg: None,
                },
                None,
                ScatterPick {
                    pipeline: &pipelines.scatter,
                    texture_bg: None,
                },
                &pipelines.errorbar,
            ),
            Some(StyleSet::Sketch {
                line,
                scatter,
                errorbar,
                line_verts,
            }) => (
                LinePick {
                    pipeline: line,
                    verts: *line_verts,
                    texture_bg: None,
                },
                None,
                ScatterPick {
                    pipeline: scatter,
                    texture_bg: None,
                },
                errorbar,
            ),
            // Milkyway: line = ribbon + star pass, scatter = ringed
            // planets (premultiplied — they occlude the star field),
            // errorbar = bipolar jets with terminal shock knots.
            Some(StyleSet::Milkyway(c)) => (
                LinePick {
                    pipeline: &c.ribbon,
                    verts: data_render::MILKYWAY_RIBBON_VERTICES,
                    texture_bg: Some(&c.star_tex_bg),
                },
                Some(StarsPick {
                    pipeline: &c.stars,
                    texture_bg: &c.star_tex_bg,
                }),
                ScatterPick {
                    pipeline: &c.planets,
                    texture_bg: Some(&c.star_tex_bg),
                },
                &c.jets,
            ),
            // Constellation: only ScatterLine is supported. The line is the
            // regular columnar stroke with style-level alpha; the scatter
            // layer renders PSF stars at the data points.
            Some(StyleSet::Constellation(c)) => (
                LinePick {
                    pipeline: &c.line,
                    verts: 4,
                    texture_bg: None,
                },
                None,
                ScatterPick {
                    pipeline: &c.stars,
                    texture_bg: Some(&c.star_tex_bg),
                },
                &pipelines.errorbar,
            ),
        };

        let mut out = Vec::with_capacity(series_specs.len());
        for (idx, series) in series_specs.iter().enumerate() {
            let cfg = series.config;
            let rt = &cfg.render_type;
            let constellation_only = matches!(styled, Some(StyleSet::Constellation(_)));
            let constellation_supported = matches!(rt, DataRenderType::ScatterLine { .. });
            let x_h = lookup(&cfg.x_column)?;
            let y_h = lookup(&cfg.y_column)?;

            let line = if has_line(rt) && (!constellation_only || constellation_supported) {
                let arc = prepared
                    .get(idx)
                    .and_then(|series| series.arc.as_ref())
                    .map(|arc| arc.prefix.clone());
                Some(ColumnLineLayer {
                    pipeline: line_pick.pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.line_bg,
                    pool_buffer: pool.buffer(),
                    x: x_h,
                    y: y_h,
                    arc,
                    verts_per_instance: line_pick.verts,
                    texture_bg: line_pick.texture_bg,
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
                (Some(_), Some(sp)) => prepared
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

            let scatter = if has_scatter(rt) && (!constellation_only || constellation_supported) {
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
                Some(ColumnScatterLayer {
                    pipeline: precise_style_map
                        .map_or(scatter_pick.pipeline, |_| &pipelines.scatter_mapped),
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.scatter_bg,
                    style_map_bg: precise_style_map.map(|m| &m.bind_group),
                    quad_vb: &self.quad_vb,
                    pool_buffer: pool.buffer(),
                    x: x_h,
                    y: y_h,
                    style_index,
                    texture_bg: scatter_pick.texture_bg,
                })
            } else {
                None
            };

            let errorbar = if constellation_only {
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
                            None => {
                                let zero = lookup(INTERNAL_ZERO_COLUMN_ID)?;
                                (zero, zero)
                            }
                        };
                        let (ex_lo, ex_hi) = match ex_opt {
                            Some(ErrorRef::Symmetric { column }) => {
                                let h = lookup(column)?;
                                (h, h)
                            }
                            Some(ErrorRef::Asymmetric { lower, upper }) => {
                                (lookup(lower)?, lookup(upper)?)
                            }
                            None => {
                                let zero = lookup(INTERNAL_ZERO_COLUMN_ID)?;
                                (zero, zero)
                            }
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
                            pipeline: precise_errorbar_style_map
                                .map_or(errorbar_pipe, |_| &pipelines.errorbar_mapped),
                            transform_bg: &view.transform_bg,
                            style_bg: &series.style.errorbar_bg,
                            style_map_bg: precise_errorbar_style_map.map(|m| &m.bind_group),
                            pool_buffer: pool.buffer(),
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

            let mut picked = Vec::new();
            if let Some(picked_cfg) = chart_config
                .picked_points
                .as_ref()
                .filter(|cfg| cfg.visible && !cfg.refs.is_empty())
            {
                let visible_in_style = !constellation_only || constellation_supported;
                if visible_in_style {
                    for picked_ref in &picked_cfg.refs {
                        if !picked_ref_matches_series(cfg, picked_ref) {
                            continue;
                        }
                        if picked_ref.point_index >= x_h.len_values
                            || picked_ref.point_index >= y_h.len_values
                        {
                            continue;
                        }
                        let Some(instance) = u32::try_from(picked_ref.point_index).ok() else {
                            continue;
                        };

                        let scatter_cfg = extract_scatter(rt);
                        let has_line_anchor = extract_line(rt).is_some();
                        let use_scatter_style_mapping = styled.is_none();
                        let has_scatter_anchor = scatter_cfg.is_some_and(|scatter| {
                            scatter_pick_anchor_may_be_visible(
                                scatter,
                                picked_ref.point_index,
                                use_scatter_style_mapping,
                            )
                        });
                        if !has_line_anchor && !has_scatter_anchor {
                            continue;
                        }

                        let precise_pick_style_map = if styled.is_none() {
                            series.style.scatter_map.as_ref()
                        } else {
                            None
                        };
                        let style_index = match (precise_pick_style_map, scatter_cfg) {
                            (Some(map), Some(scatter)) if map.has_index => {
                                let Some(column) = scatter.point_style_index_column.as_ref() else {
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

                        let mut ring_style = PrimitiveStyle::from_color_with_width(
                            picked_cfg.ring_color,
                            picked_cfg.ring_width_px,
                        );
                        let uses_mapped_pick = precise_pick_style_map.is_some();
                        if uses_mapped_pick {
                            ring_style.point_radius_px = series.style.scatter_radius_px;
                            ring_style.cap_half_px = picked_cfg.radius_extra_px;
                        } else {
                            let scatter_radius = scatter_cfg
                                .map(|scatter| {
                                    scatter_config_radius_px(
                                        scatter,
                                        picked_ref.point_index,
                                        use_scatter_style_mapping,
                                    )
                                })
                                .unwrap_or(0.0);
                            ring_style.point_radius_px =
                                (scatter_radius + picked_cfg.radius_extra_px).max(0.0);
                        }
                        ring_style.shape_id = data_render::shape_id(&ScatterShape::Circle);
                        let ring_buf =
                            data_render::create_style_uniform_buffer(&self.device, &ring_style);
                        let style_bg = data_render::create_style_bind_group(
                            &self.device,
                            &self.style_bgl,
                            &ring_buf,
                        );
                        picked.push(ColumnPickRingLayer {
                            pipeline: if uses_mapped_pick {
                                &pipelines.pick_ring_mapped
                            } else {
                                &pipelines.pick_ring
                            },
                            transform_bg: &view.transform_bg,
                            style_bg,
                            style_map_bg: precise_pick_style_map.map(|map| &map.bind_group),
                            quad_vb: &self.quad_vb,
                            pool_buffer: pool.buffer(),
                            x: x_h,
                            y: y_h,
                            style_index,
                            instance,
                        });
                    }
                }
            }

            out.push(data_render::SeriesLayers {
                errorbar,
                line,
                line_extra,
                scatter,
                picked,
            });
        }
        Ok((out, column_sources))
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
    /// The scan is re-dispatched on every prepare that needs it (the prefix
    /// depends on the data→pixel transform `t`): a handful of tiny compute
    /// dispatches submitted before the host's render pass, which queue order
    /// then sequences correctly. Buffers/bind groups are cached per series
    /// and rebuilt only when the series layout changes — or when a live
    /// [`PreparedFrame`] still references the cached buffer (its refcount is
    /// > 1): `dispatch` rewrites contents in place, so instead of clobbering
    /// what an outstanding token will draw, the scratch is rebuilt with
    /// fresh buffers (copy-on-write). This makes overlapping prepares — a
    /// second panel with the same series, an export between a host's
    /// prepare and paint — safe by construction.
    /// `star_pitch`: `Some` adds the constellation star pass to the scratch
    /// (indirect args + VS bind group) and runs its kernel after the scan.
    fn ensure_arc_prefix(
        &mut self,
        series_id: &str,
        x_id: &str,
        y_id: &str,
        t: &data_render::ScatterTransform,
        star_pitch: Option<f32>,
    ) -> Option<PreparedArc> {
        let (x_base, y_base, n) = self.arc_source_layout(x_id, y_id)?;
        let layout_generation = self.pool.layout_generation();

        // Runaway-churn backstop: ids of long-removed series would otherwise
        // pin GPU memory forever. Existing-key rebuilds replace in place, but
        // new-key inserts must not let the cache grow past the cap.
        let has_cached_series = self.arc_cache.contains_key(series_id);
        if self.arc_cache.len() > ARC_PREFIX_CACHE_LIMIT
            || (!has_cached_series && self.arc_cache.len() >= ARC_PREFIX_CACHE_LIMIT)
        {
            self.arc_cache.clear();
        }
        let slots = self.arc_cache.entry(series_id.to_string()).or_default();
        // Slots whose layout no longer matches are useless for reuse; any
        // live token that still draws from one owns clones of its handles,
        // so dropping the slot here never invalidates a token.
        slots.retain(|s| {
            s.matches(n, x_base, y_base, layout_generation)
                && s.star.is_some() == star_pitch.is_some()
        });
        // Copy-on-write slot pool: `dispatch` rewrites buffer contents in
        // place, and a strong count > 1 means a live PreparedFrame (or an
        // in-flight sibling within this same prepare) still references the
        // slot's arc buffer — its contents must survive until that token's
        // pass is submitted. Reuse the first free slot; only when every slot
        // is pinned is a fresh one built. Retained-token hosts (replace last
        // frame's token after the next prepare returns) settle on two slots
        // that alternate, so the steady state allocates nothing. A transient
        // over-count from a token dropping on another thread only costs a
        // spurious slot, never a clobber.
        let idx = match slots.iter().position(|s| Arc::strong_count(&s.arc) == 1) {
            Some(idx) => idx,
            None => {
                #[cfg(test)]
                let chunk_override = self.arc_chunk_override;
                #[cfg(not(test))]
                let chunk_override = None;
                let scratch = data_render::line_arc::ArcScratch::build(
                    &self.device,
                    &self.arc_pipelines,
                    self.pool.buffer(),
                    n,
                    x_base,
                    y_base,
                    layout_generation,
                    self.caps.max_compute_workgroups_per_dimension,
                    chunk_override,
                    star_pitch.is_some().then_some(&self.star_data_bgl),
                )?; // None only on a zero-dispatch-limit adapter; try_new rejects those.
                slots.push(scratch);
                slots.len() - 1
            }
        };
        let scratch = &slots[idx];

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy line arc encoder"),
            });
        scratch.dispatch(
            &self.queue,
            &mut encoder,
            &self.arc_pipelines,
            t,
            star_pitch,
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        // Snapshot AFTER the dispatch is submitted: the token's handles are
        // owned clones (wgpu resources are refcounted), so the draw phase
        // never has to reach back into `arc_cache`.
        let star = scratch.star.as_ref().map(|s| PreparedStar {
            indirect: s.indirect.clone(),
            vs_bg: s.vs_bg.clone(),
        });
        Some(PreparedArc {
            prefix: (Arc::clone(&scratch.arc), u64::from(n) * 4),
            star,
        })
    }

    // Headless PNG export.

    /// Render one chart panel offscreen at `scale × original` pixel
    /// dimensions and return an RGBA buffer. Fonts, line widths, and margins
    /// are all scaled proportionally so the result is visually consistent.
    ///
    /// - `scale` is clamped to [`MIN_EXPORT_SCALE`] .. [`MAX_EXPORT_SCALE`].
    /// - Background is fully transparent (alpha 0).
    /// - Output size is `chart_area × scale`.
    /// - Encoding / saving the PNG is the caller's responsibility — use
    ///   [`encode_png`] or [`Self::export_panel_png_bytes_async`].
    ///
    /// Async because the GPU→CPU readback must yield to the browser on wasm;
    /// on native the await resolves immediately (the device is polled to
    /// completion inline). Native callers can use the blocking
    /// [`Self::export_panel_rgba`] wrapper instead. `&mut self` for the same
    /// reason as [`Self::paint`]: the prepare phase (arc prefixes; the
    /// export's styled pipelines live in a per-call set instead).
    pub async fn export_panel_rgba_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        self.export_panel_rgba_with_clear_async(
            chart,
            series,
            scale,
            crate::color::Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        )
        .await
    }

    /// Export a panel with an explicit clear color behind the chart.
    pub async fn export_panel_rgba_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        let scale = clamp_export_scale(scale);
        let orig = chart.config().chart_area.0;
        let w = ((orig.width as f32) * scale).round().max(1.0) as u32;
        let h = ((orig.height as f32) * scale).round().max(1.0) as u32;
        validate_texture_extent(self.caps, "figgy export target dimension", w, h)?;
        let export_format = wgpu::TextureFormat::Rgba8Unorm;
        let mut export_sample_count = preferred_msaa_sample_count(self.caps, export_format);
        validate_target_sample_count(self.caps, export_format, export_sample_count)?;
        let export_features = export_format.guaranteed_format_features(self.caps.features);
        if !export_features
            .allowed_usages
            .contains(wgpu::TextureUsages::COPY_SRC)
        {
            return Err(FiggyError::UnsupportedSurfaceFormat {
                format: export_format,
                reason: "figgy export target is not guaranteed to support COPY_SRC".into(),
            });
        }

        // 1) Scaled config with chart_area overridden to (0,0,w,h) → temp chart.
        let mut scaled_config = chart.config().scaled(scale);
        scaled_config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        });
        let scaled_chart = Chart::new(scaled_config);

        // 2) Temp ChartView with scaled axis textures.
        let view = self.create_chart_view(
            &scaled_chart,
            Rect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            },
        )?;

        // 3) Series styles — line_width also scales (extracted from render_type).
        let scaled_styles: Vec<ChartStyle> = series
            .iter()
            .map(|cfg| self.create_style_for_series_scaled(cfg, scale))
            .collect();
        let series_objs: Vec<Series<'_>> = series
            .iter()
            .zip(scaled_styles.iter())
            .map(|(cfg, st)| Series {
                config: cfg,
                style: st,
            })
            .collect();
        let items = [ChartDrawItem {
            view: &view,
            chart_config: scaled_chart.config(),
            series: &series_objs,
        }];

        // 3.5) Per-item prepare phase (transform uniform write + arc-prefix
        // dispatch) — submitted before the render pass below, so queue order
        // sequences them.
        // 4) Offscreen target.
        let target_desc = wgpu::TextureDescriptor {
            label: Some("figgy export target"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: export_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let target_tex = create_texture_checked(&self.device, &target_desc, "figgy export target")?;
        let target_view = target_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa_export_target = match create_msaa_target(
            &self.device,
            self.caps,
            "figgy export msaa target",
            w,
            h,
            export_format,
            export_sample_count,
        ) {
            Ok(target) => target,
            Err(_) if export_sample_count > 1 => {
                export_sample_count = 1;
                None
            }
            Err(e) => return Err(e),
        };
        let (render_view, resolve_target, store) = match msaa_export_target.as_ref() {
            Some(msaa) => (&msaa.view, Some(&target_view), wgpu::StoreOp::Discard),
            None => (&target_view, None, wgpu::StoreOp::Store),
        };
        // Per-call set against the export format. The same prepare-phase
        // ensure as `paint` fills the styled cache, so styled variants
        // compile only when this export actually draws with them.
        let mut export_target_pipelines = create_target_pipelines(
            &self.device,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            export_format,
            export_sample_count,
        );
        export_target_pipelines.ensure_styles_for_items(
            &self.device,
            &self.queue,
            &self.transform_bgl,
            &self.style_bgl,
            &self.star_data_bgl,
            export_format,
            &items,
        );
        let prepared_arcs = self.prepare_arc_items(&items)?;
        let (prepared_items, _) =
            self.resolve_prepared_items(&items, &prepared_arcs, &export_target_pipelines)?;

        // 5) Readback buffer. Allocate at most the hardware limit and read rows
        // sequentially if the full image would exceed it.
        let unpadded_bpr_u64 = u64::from(w) * 4;
        let align = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let padded_bpr_u64 = ((unpadded_bpr_u64 + align - 1) / align) * align;
        if padded_bpr_u64 > u64::from(u32::MAX) {
            return Err(FiggyError::GpuResourceLimit {
                resource: "figgy export bytes_per_row",
                requested: padded_bpr_u64,
                limit: u64::from(u32::MAX),
            });
        }
        let unpadded_bpr = unpadded_bpr_u64 as u32;
        let padded_bpr = padded_bpr_u64 as u32;
        let max_rows = (self.caps.max_buffer_size / padded_bpr_u64).min(u64::from(h));
        if max_rows == 0 {
            return Err(FiggyError::GpuResourceLimit {
                resource: "figgy export readback row",
                requested: padded_bpr_u64,
                limit: self.caps.max_buffer_size,
            });
        }
        let mut rows_per_chunk = max_rows.min(u64::from(u32::MAX)) as u32;
        let readback = loop {
            let size = padded_bpr_u64 * u64::from(rows_per_chunk);
            validate_buffer_size(self.caps, "figgy export readback buffer", size)?;
            let desc = wgpu::BufferDescriptor {
                label: Some("figgy export readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            };
            match create_buffer_checked(&self.device, &desc, "figgy export readback buffer") {
                Ok(buffer) => break buffer,
                Err(e) if rows_per_chunk > 1 => {
                    rows_per_chunk = (rows_per_chunk / 2).max(1);
                    let _ = e;
                }
                Err(e) => return Err(e),
            }
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy export encoder"),
            });

        // 6) Render pass — configured clear + a single paint.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy export pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: render_view,
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
            });
            self.paint_prepared_items(&mut pass, (w, h), &prepared_items)?;
        }
        self.queue.submit(std::iter::once(encoder.finish()));

        // 7) Texture → readback buffer in row chunks.
        let bgra = false;
        let rgba_len = u64::from(w)
            .checked_mul(u64::from(h))
            .and_then(|px| px.checked_mul(4))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(FiggyError::GpuResourceLimit {
                resource: "figgy export rgba output",
                requested: u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4),
                limit: usize::MAX as u64,
            })?;
        let mut rgba = vec![0u8; rgba_len];
        let mut y0 = 0;
        while y0 < h {
            let rows = rows_per_chunk.min(h - y0);
            let chunk_size = padded_bpr_u64 * u64::from(rows);
            let mut copy_encoder =
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("figgy export copy encoder"),
                    });
            copy_encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: y0, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bpr),
                        rows_per_image: Some(rows),
                    },
                },
                wgpu::Extent3d {
                    width: w,
                    height: rows,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.submit(std::iter::once(copy_encoder.finish()));

            let slice = readback.slice(..chunk_size);
            let (tx, rx) = futures_channel::oneshot::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            // Native: drive the device to completion so the await below
            // resolves immediately. On wasm the browser polls the device and
            // the await yields to the JS event loop instead.
            #[cfg(not(target_arch = "wasm32"))]
            let _ = self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            rx.await
                .map_err(|e| FiggyError::GpuResourceAllocationFailed {
                    resource: "figgy export readback mapping",
                    reason: format!("map_async sender dropped: {e}"),
                })?
                .map_err(|e| FiggyError::GpuResourceAllocationFailed {
                    resource: "figgy export readback mapping",
                    reason: format!("map_async: {e:?}"),
                })?;
            let mapped = slice.get_mapped_range();

            for local_y in 0..rows {
                let src_off = (local_y * padded_bpr) as usize;
                let dst_off = ((y0 + local_y) * unpadded_bpr) as usize;
                let row = &mapped[src_off..src_off + unpadded_bpr as usize];
                for i in 0..(w as usize) {
                    let p = i * 4;
                    let (b0, b1, b2, b3) = (row[p], row[p + 1], row[p + 2], row[p + 3]);
                    let (r, g, b, a) = if bgra {
                        (b2, b1, b0, b3)
                    } else {
                        (b0, b1, b2, b3)
                    };
                    let (or, og, ob) = if a == 0 || a == 255 {
                        (r, g, b)
                    } else {
                        let a_f = a as f32 / 255.0;
                        (
                            ((r as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                            ((g as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                            ((b as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                        )
                    };
                    rgba[dst_off + p] = or;
                    rgba[dst_off + p + 1] = og;
                    rgba[dst_off + p + 2] = ob;
                    rgba[dst_off + p + 3] = a;
                }
            }
            drop(mapped);
            readback.unmap();
            y0 += rows;
        }

        Ok(RasterImage {
            width: w,
            height: h,
            rgba,
        })
    }

    /// Convenience wrapper: export panel RGBA, then encode PNG bytes in
    /// memory. Saving the bytes to disk is up to the caller.
    pub async fn export_panel_png_bytes_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let img = self.export_panel_rgba_async(chart, series, scale).await?;
        encode_png(&img)
    }

    /// Convenience wrapper: export panel PNG with an explicit clear color.
    pub async fn export_panel_png_bytes_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<Vec<u8>> {
        let img = self
            .export_panel_rgba_with_clear_async(chart, series, scale, clear)
            .await?;
        encode_png(&img)
    }

    /// Blocking convenience wrapper around [`Self::export_panel_rgba_async`].
    /// Native only — on wasm, await the async variant from the host's event
    /// loop instead.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_rgba(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        pollster::block_on(self.export_panel_rgba_async(chart, series, scale))
    }

    /// Blocking convenience wrapper around
    /// [`Self::export_panel_png_bytes_async`]. Native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_png_bytes(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        pollster::block_on(self.export_panel_png_bytes_async(chart, series, scale))
    }
}

// Export result type, scale clamping, PNG encoding.

/// Lower bound for export scale. 1.0 is screen-equivalent (96 DPI baseline,
/// `scale = dpi / 96`); 0.25 corresponds to ~24 DPI (thumbnails).
pub const MIN_EXPORT_SCALE: f32 = 0.25;
/// Upper bound. 8.0 corresponds to ~768 DPI; we clamp instead of erroring
/// because anything bigger explodes memory and time
/// (an 8× export of a 1920×1080 panel is 15360×8640 RGBA ≈ 530 MB).
pub const MAX_EXPORT_SCALE: f32 = 8.0;

/// Convert DPI to a `scale` value relative to the 96 DPI baseline.
pub fn dpi_to_scale(dpi: f32) -> f32 {
    clamp_export_scale(dpi / 96.0)
}

/// Clamp `scale` to [`MIN_EXPORT_SCALE`] .. [`MAX_EXPORT_SCALE`].
pub fn clamp_export_scale(scale: f32) -> f32 {
    if scale.is_nan() || scale <= 0.0 {
        return MIN_EXPORT_SCALE;
    }
    scale.clamp(MIN_EXPORT_SCALE, MAX_EXPORT_SCALE)
}

/// In-memory RGBA8 image (straight alpha).
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, channel order R, G, B, A.
    pub rgba: Vec<u8>,
}

/// Encode `img` as PNG bytes in memory. Saving to disk is the caller's job.
pub fn encode_png(img: &RasterImage) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| FiggyError::RasterWrapFailed {
                reason: format!("png header: {e}"),
            })?;
        writer
            .write_image_data(&img.rgba)
            .map_err(|e| FiggyError::RasterWrapFailed {
                reason: format!("png write: {e}"),
            })?;
    }
    Ok(buf)
}

// Public auxiliary types.

/// Per-panel GPU resources: separate grid + decoration textures (for layered
/// compositing) plus the transform uniform.
///
/// Grid (below data) and decoration (axis lines, labels — above data) are
/// rasterized to two textures so the GPU can composite in order:
/// `grid → data → decoration`.
pub struct ChartView {
    grid_texture: wgpu::Texture,
    grid_bind_group: wgpu::BindGroup,
    decoration_texture: wgpu::Texture,
    decoration_bind_group: wgpu::BindGroup,
    transform_buffer: wgpu::Buffer,
    transform_bg: wgpu::BindGroup,
    content_revision: Arc<std::sync::atomic::AtomicU64>,
    /// Panel pixel rect in surface coordinates.
    panel_rect: Rect,
    /// Generation of the renderer's cached constellation backdrop currently
    /// uploaded into `grid_texture` (`None` = non-cached content). Lets
    /// `refresh_axis` skip the full-panel texture write on cache hits.
    grid_space_gen: Option<u64>,
}

impl ChartView {
    pub fn panel_rect(&self) -> Rect {
        self.panel_rect.clone()
    }

    fn advance_content_revision(&self) -> Result<u64> {
        self.content_revision
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |current| current.checked_add(1),
            )
            .map(|previous| previous + 1)
            .map_err(|_| FiggyError::CounterExhausted {
                counter: "chart view content revision",
            })
    }
}

/// Bundle of style bind groups for line, scatter, and errorbar primitives.
/// Built via `Renderer::create_style_for_series*`. Multiple series in one
/// chart can share a single `ChartStyle` (`Series.style: &ChartStyle`).
///
/// The style uniform buffers live inside the bind groups — wgpu keeps bound
/// resources alive, so no named buffer fields are needed.
pub struct ChartStyle {
    line_bg: wgpu::BindGroup,
    scatter_bg: wgpu::BindGroup,
    errorbar_bg: wgpu::BindGroup,
    scatter_map: Option<data_render::ScatterStyleMap>,
    errorbar_map: Option<data_render::ErrorBarStyleMap>,
    scatter_radius_px: f32,
}

/// One drawable series — `data_config::SeriesConfig` (declarative) plus a
/// `ChartStyle` (GPU uniform buffers). The unit of input to `Renderer::paint`.
pub struct Series<'a> {
    /// Column ids + render type + label. Pure declaration, no GPU state.
    pub config: &'a SeriesConfig,
    /// GPU style created by `Renderer::create_style_for_series`.
    pub style: &'a ChartStyle,
}

/// One chart panel to draw: view + chart config + series slice.
pub struct ChartDrawItem<'a> {
    pub view: &'a ChartView,
    pub chart_config: &'a crate::config::Config,
    pub series: &'a [Series<'a>],
}

// DataRenderType branching helpers — which layers are needed and which
// sub-style to extract.

fn has_line(rt: &DataRenderType) -> bool {
    matches!(
        rt,
        DataRenderType::Line { .. }
            | DataRenderType::ScatterLine { .. }
            | DataRenderType::LineScatterErrorbarX { .. }
            | DataRenderType::LineScatterErrorbarY { .. }
            | DataRenderType::LineScatterErrorbarXY { .. }
    )
}

fn has_scatter(rt: &DataRenderType) -> bool {
    !matches!(rt, DataRenderType::Line { .. })
}

fn extract_line(rt: &DataRenderType) -> Option<&DataLineStyleConfig> {
    match rt {
        DataRenderType::Line { line }
        | DataRenderType::ScatterLine { line, .. }
        | DataRenderType::LineScatterErrorbarX { line, .. }
        | DataRenderType::LineScatterErrorbarY { line, .. }
        | DataRenderType::LineScatterErrorbarXY { line, .. } => Some(line),
        _ => None,
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
        DataRenderType::Line { .. } => None,
    }
}

fn extract_errorbar_style(rt: &DataRenderType) -> Option<&DataErrorBarStyleConfig> {
    match rt {
        DataRenderType::ScatterErrorbarX { err_style, .. }
        | DataRenderType::ScatterErrorbarY { err_style, .. }
        | DataRenderType::ScatterErrorbarXY { err_style, .. }
        | DataRenderType::LineScatterErrorbarX { err_style, .. }
        | DataRenderType::LineScatterErrorbarY { err_style, .. }
        | DataRenderType::LineScatterErrorbarXY { err_style, .. } => Some(err_style),
        _ => None,
    }
}

fn extract_err_y(rt: &DataRenderType) -> Option<&ErrorRef> {
    match rt {
        DataRenderType::ScatterErrorbarY { err_y, .. }
        | DataRenderType::ScatterErrorbarXY { err_y, .. }
        | DataRenderType::LineScatterErrorbarY { err_y, .. }
        | DataRenderType::LineScatterErrorbarXY { err_y, .. } => Some(err_y),
        _ => None,
    }
}

fn extract_err_x(rt: &DataRenderType) -> Option<&ErrorRef> {
    match rt {
        DataRenderType::ScatterErrorbarX { err_x, .. }
        | DataRenderType::ScatterErrorbarXY { err_x, .. }
        | DataRenderType::LineScatterErrorbarX { err_x, .. }
        | DataRenderType::LineScatterErrorbarXY { err_x, .. } => Some(err_x),
        _ => None,
    }
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
    pub fn surface_size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
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

    pub fn defragment(&mut self) -> Result<bool> {
        self.inner.defragment()
    }

    pub fn process_pending_maintenance(&mut self) -> Result<bool> {
        self.inner.process_pending_maintenance()
    }

    pub fn ensure_internal_zero_column(&mut self, len: usize) -> Result<()> {
        self.inner.ensure_internal_zero_column(len)
    }

    pub fn set_defrag_policy(&mut self, policy: DefragPolicy) {
        self.inner.set_defrag_policy(policy);
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

    pub fn sync_external_invalidations(&mut self) -> Result<bool> {
        self.inner.sync_external_invalidations()
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

    pub fn update_transform(&mut self, view: &ChartView, chart: &Chart) -> Result<()> {
        self.inner.update_transform(view, chart)
    }

    pub fn prepare(&mut self, items: &[ChartDrawItem<'_>]) -> Result<PreparedFrame> {
        self.inner.prepare(items)
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
    /// Acquires the current swap-chain texture, runs the mutable prepare
    /// phase (`prepare(items)`) before any pass opens, then builds an
    /// encoder + render pass, records via `paint_prepared`, and submits and
    /// presents. The caller is only responsible for processing per-panel
    /// dirty flags (`chart.consume_*_dirty()` → `refresh_axis`).
    pub fn draw(&mut self, clear: crate::color::Color, items: &[ChartDrawItem<'_>]) -> Result<()> {
        let prepared = self.inner.prepare(items)?;
        self.draw_prepared(clear, &prepared)
    }

    /// Draw one frame using a prepared data token owned by the host.
    ///
    /// This still records the surface composition pass (`grid -> data -> decoration`),
    /// but it does not rerun `prepare`: no transform uniform write, styled
    /// pipeline ensure, or arc-prefix compute unless the host prepared a new
    /// token first.
    pub fn draw_prepared(
        &mut self,
        clear: crate::color::Color,
        prepared: &PreparedFrame,
    ) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(error @ (wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated)) => {
                // Let the caller decide whether to resize, retry, or exit.
                return Err(FiggyError::SurfaceAcquireFailed { error });
            }
            Err(error) => return Err(FiggyError::SurfaceAcquireFailed { error }),
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
            });
            self.inner.paint_prepared(
                &mut pass,
                (self.surface_config.width, self.surface_config.height),
                prepared,
            )?;
        }
        self.inner.queue().submit(std::iter::once(encoder.finish()));
        frame.present();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Column;

    fn col_f64(data: Vec<f64>) -> Column<f64> {
        let min = data.iter().copied().fold(f64::INFINITY, f64::min);
        let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Column { data, min, max }
    }

    fn test_scatter_style() -> DataScatterStyleConfig {
        DataScatterStyleConfig {
            point_color: Color::new(0.0, 0.0, 0.0, 1.0),
            point_shape: crate::data_config::ScatterShape::CircleFilled,
            point_size: 5.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
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

    fn state_test_renderer() -> Option<Renderer> {
        let (device, queue) = crate::data_render::shared_device()?;
        Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Rgba8Unorm,
            1024 * 1024,
        )
        .ok()
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
                },
            },
        }
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

    fn caps_with_limits(storage_binding: u32, buffer_size: u64) -> RendererDeviceCaps {
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
        let caps = caps_with_limits(128 * mb as u32, 1024 * mb);
        // Comfortably under both limits — untouched.
        assert_eq!(effective_pool_capacity(64 * mb, caps), 64 * mb);
        // Exactly at the storage-binding limit — untouched.
        assert_eq!(effective_pool_capacity(128 * mb, caps), 128 * mb);
        // Over the storage-binding limit (wgpu default) — clamped to it,
        // even though max_buffer_size is far higher. This is the 256 MB pool
        // that used to panic on the first dashed/sketch/milkyway line.
        assert_eq!(effective_pool_capacity(256 * mb, caps), 128 * mb);
        // When max_buffer_size is the tighter of the two, it wins.
        let caps_small_buffer = caps_with_limits(u32::MAX, 32 * mb);
        assert_eq!(
            effective_pool_capacity(256 * mb, caps_small_buffer),
            32 * mb
        );
        // A device with a generous binding limit keeps a large pool intact.
        let caps_big = caps_with_limits(u32::MAX, 4 * 1024 * mb);
        assert_eq!(effective_pool_capacity(512 * mb, caps_big), 512 * mb);
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

        let same_config = renderer.chart_config(chart).unwrap().clone();
        renderer.set_chart_config(chart, same_config).unwrap();
        let decoration = renderer.chart_render_stamp(chart).unwrap();
        assert!(decoration.needs_draw_since(Some(&initial)));
        assert!(decoration.needs_raster_since(Some(&initial)));

        let mut view_config = renderer.chart_config(chart).unwrap().clone();
        view_config.bottom_x.min -= 1.0;
        renderer.set_chart_config(chart, view_config).unwrap();
        let view = renderer.chart_render_stamp(chart).unwrap();
        assert!(view.needs_draw_since(Some(&decoration)));

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
        }
        assert_eq!(
            renderer.web_derived_snapshot(chart).unwrap().stamp(),
            before.stamp()
        );

        let prospective_stamp = {
            let guard = renderer
                .begin_upsert_column("x", &col_f64(vec![20.0, 21.0]))
                .unwrap();
            let stamp = guard.web_derived_snapshot(chart).unwrap().stamp().clone();
            guard.commit();
            stamp
        };
        assert_eq!(
            renderer.web_derived_snapshot(chart).unwrap().stamp(),
            &prospective_stamp
        );
    }

    #[test]
    fn renderer_internal_zero_column_is_not_host_mutable() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let nonzero = col_f64(vec![7.0, 7.0]);
        assert!(matches!(
            renderer.add_column(INTERNAL_ZERO_COLUMN_ID, &nonzero),
            Err(FiggyError::ReservedColumnId { .. })
        ));
        assert!(matches!(
            renderer.begin_upsert_column(INTERNAL_ZERO_COLUMN_ID, &nonzero),
            Err(FiggyError::ReservedColumnId { .. })
        ));

        renderer.ensure_internal_zero_column(2).unwrap();
        let slot = renderer.pool().slot(INTERNAL_ZERO_COLUMN_ID).unwrap();
        assert_eq!((slot.min, slot.max, slot.len_values), (0.0, 0.0, 2));
        assert!(matches!(
            renderer.remove_column(INTERNAL_ZERO_COLUMN_ID),
            Err(FiggyError::ReservedColumnId { .. })
        ));
        assert!(renderer.pool().slot(INTERNAL_ZERO_COLUMN_ID).is_some());
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

        assert!(renderer.remove_column("x").unwrap());
        assert!(renderer.pool().slot("x").is_none());
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

    #[test]
    fn internal_zero_column_grows_without_point_vector_or_visual_invalidation() {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let visual = renderer.visual_revision();
        renderer.ensure_internal_zero_column(4).unwrap();
        assert_eq!(
            renderer
                .pool()
                .slot(INTERNAL_ZERO_COLUMN_ID)
                .unwrap()
                .len_values,
            4
        );
        assert_eq!(renderer.visual_revision(), visual);

        renderer.ensure_internal_zero_column(2).unwrap();
        assert_eq!(
            renderer
                .pool()
                .slot(INTERNAL_ZERO_COLUMN_ID)
                .unwrap()
                .len_values,
            4
        );
        renderer.ensure_internal_zero_column(8).unwrap();
        assert_eq!(
            renderer
                .pool()
                .slot(INTERNAL_ZERO_COLUMN_ID)
                .unwrap()
                .len_values,
            8
        );
        assert_eq!(renderer.visual_revision(), visual);
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
                },
                line: DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(0.4, 0.7, 1.0, 1.0),
                    line_width: 2.0,
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

    #[test]
    fn picked_overlay_skips_zero_size_scatter_errorbar_anchor() {
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
        r.ensure_internal_zero_column(1).unwrap();

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

        assert_eq!(
            green_ring_ink, 0,
            "hidden scatter marker over errorbar must not draw picked overlay"
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
        r.ensure_internal_zero_column(2).unwrap();

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

    #[test]
    fn single_direction_errorbar_still_requires_zero_column() {
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
        let series = [SeriesConfig {
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

        let err = match r.export_panel_rgba(&chart, &series, 1.0) {
            Ok(_) => panic!("Y-only errorbar must still need __zero for the missing X side"),
            Err(err) => err,
        };
        match err {
            FiggyError::UnknownColumn { id } => assert!(id.contains("__zero"), "{id}"),
            other => panic!("unexpected error: {other:?}"),
        }
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
        let xs: Vec<f64> = (0..n).map(|i| i as f64 * 6.28 / n as f64).collect();
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
        config.chart_area = crate::layout::ChartArea(rect.clone());
        config.draw_style =
            crate::config::DrawStyle::Milkyway(crate::config::MilkywayOptions::default());
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(0.0, 1.0);

        let mut view = r.create_chart_view(&chart, rect.clone()).unwrap();
        assert_eq!(view.grid_space_gen, None, "fresh view starts unstamped");

        r.refresh_axis(&mut view, &chart, rect.clone()).unwrap();
        assert_eq!(r.space_bg.as_ref().map(|s| s.bake_gen), Some(1));
        assert_eq!(view.grid_space_gen, Some(1));

        // Range-only change (pan/zoom): cache hit — no rebake, no restamp.
        chart.set_x_range(0.0, 2.0);
        r.refresh_axis(&mut view, &chart, rect.clone()).unwrap();
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
        r.refresh_axis(&mut view, &chart, rect.clone()).unwrap();
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
                .ensure_arc_prefix(&format!("s_{case}"), &xid, &yid, &t, None)
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
            let gpu: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();

            // Sequential CPU reference mirroring the shader math.
            let px = |x: f64, y: f64| -> (f32, f32) {
                let nx = ((x as f32) - t.data_min[0]) / (t.data_max[0] - t.data_min[0]) * 2.0 - 1.0;
                let ny = ((y as f32) - t.data_min[1]) / (t.data_max[1] - t.data_min[1]) * 2.0 - 1.0;
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
                r.ensure_arc_prefix(&series_id, "arc_cache_x", "arc_cache_y", &t, None,)
                    .is_some()
            );
            assert!(
                r.arc_cache.len() <= ARC_PREFIX_CACHE_LIMIT,
                "arc cache exceeded limit after inserting {series_id}: {}",
                r.arc_cache.len()
            );
        }
    }

    /// Two overlapping prepares for the same dashed series (egui's
    /// all-prepares-before-any-paint schedule with a shared renderer, or an
    /// export interleaved between a host's prepare and paint) must not share
    /// an arc buffer: `dispatch` rewrites contents in place, so a buffer
    /// referenced by a live token is never reused (copy-on-write). Once no
    /// token holds the buffer, the cached scratch is reused again.
    #[test]
    fn overlapping_prepares_get_isolated_arc_buffers() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        r.add_column("cow_x", &col_f64(vec![0.0, 0.5, 1.0]))
            .unwrap();
        r.add_column("cow_y", &col_f64(vec![0.0, 1.0, 0.25]))
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
            series_id: "cow".into(),
            source_id: None,
            label: None,
            x_column: "cow_x".into(),
            y_column: "cow_y".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Dash,
                    line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                    line_width: 2.0,
                },
            },
        };
        let style = r.create_style_for_series(&series_cfg);
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let view_a = r
            .create_chart_view(&chart, chart.config().chart_area.0.clone())
            .unwrap();
        let view_b = r
            .create_chart_view(&chart, chart.config().chart_area.0.clone())
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
        let p2 = r.prepare(&items_b).unwrap();
        let a2 = arc_of(&p2);
        assert!(
            !Arc::ptr_eq(&a1, &a2),
            "a prepare must not rewrite the arc buffer a live token references"
        );

        // Both tokens stay individually paintable.
        assert!(r.validate_prepared(&p1).is_ok());
        assert!(r.validate_prepared(&p2).is_ok());

        // With no token left alive, an existing slot is reused in place.
        let a1_raw = Arc::as_ptr(&a1);
        let a2_raw = Arc::as_ptr(&a2);
        drop((p1, a1, p2, a2));
        let p3 = r.prepare(&items_a).unwrap();
        let a3_raw = Arc::as_ptr(&arc_of(&p3));
        assert!(
            a3_raw == a1_raw || a3_raw == a2_raw,
            "an unreferenced slot should be reused, not rebuilt"
        );
        drop(p3);

        // Retained-token steady state (hosts replace last frame's token only
        // AFTER the next prepare returns): the slot pool must settle on two
        // alternating slots — no per-frame rebuild, no growth.
        let mut held = r.prepare(&items_a).unwrap();
        let mut seen = std::collections::HashSet::new();
        seen.insert(Arc::as_ptr(&arc_of(&held)));
        for idx in 0..6 {
            let next = r
                .prepare(if idx % 2 == 0 { &items_b } else { &items_a })
                .unwrap();
            seen.insert(Arc::as_ptr(&arc_of(&next)));
            held = next;
        }
        drop(held);
        assert!(
            seen.len() <= 2,
            "retained-token steady state must alternate between two slots, saw {} distinct buffers",
            seen.len()
        );
        assert_eq!(
            r.arc_cache.get("cow").map(|slots| slots.len()),
            Some(2),
            "slot pool should hold exactly two slots after retained-token frames"
        );
    }

    /// The token owns host inputs after prepare and rejects only renderer
    /// resource changes that invalidate its captured GPU work.
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
                },
            },
        };
        let (cfg_a, cfg_b) = (mk_cfg("st_a"), mk_cfg("st_b"));
        let style = r.create_style_for_series(&cfg_a);
        let view = r
            .create_chart_view(&chart, chart.config().chart_area.0.clone())
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
            .create_chart_view(&chart, chart.config().chart_area.0.clone())
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
        r.refresh_axis(&mut view_reuse, &chart, chart.config().chart_area.0.clone())
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
                },
            },
        };
        let style_dash = r.create_style_for_series(&cfg_dash);
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

    /// End-to-end split under the adversarial host schedule: prepare, then
    /// an interleaved second prepare (which rebuilds the cache entry via
    /// copy-on-write), then record with the FIRST token into an offscreen
    /// pass. The token's owned arc/star snapshot must survive the
    /// interleaving and still put ink on the target.
    #[test]
    fn paint_prepared_draws_after_interleaved_prepare() {
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
        let mut chart = Chart::new(config);
        chart.set_x_range(-0.05, 1.05);
        chart.set_y_range(0.0, 1.0);

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
                },
            },
        };
        let style = r.create_style_for_series(&series_cfg);
        let series = [Series {
            config: &series_cfg,
            style: &style,
        }];
        let view_a = r
            .create_chart_view(&chart, chart.config().chart_area.0.clone())
            .unwrap();
        let view_b = r
            .create_chart_view(&chart, chart.config().chart_area.0.clone())
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

        // egui-style schedule: both prepares run before any paint. The
        // second one rebuilds the arc scratch (COW), so painting with `p1`
        // exercises the token's cache-independent snapshot.
        let p1 = r.prepare(&items_a).unwrap();
        assert!(
            p1.items[0].series[0].line_extra.is_some(),
            "milkyway line series must carry a star-pass snapshot"
        );
        let p2 = r.prepare(&items_b).unwrap();

        let target_desc = wgpu::TextureDescriptor {
            label: Some("prepare split test target"),
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

        let paint_once = |prepared: &PreparedFrame| -> Vec<u8> {
            let tex = device.create_texture(&target_desc);
            let tv = tex.create_view(&wgpu::TextureViewDescriptor::default());
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
                });
                r.paint_prepared(&mut pass, (w, h), prepared)
                    .expect("paint_prepared with a valid token");
            }
            // 320 * 4 = 1280 bytes/row, already 256-aligned.
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(w) * 4 * u64::from(h),
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
                        bytes_per_row: Some(w * 4),
                        rows_per_image: Some(h),
                    },
                },
                target_desc.size,
            );
            queue.submit(std::iter::once(enc.finish()));
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            // Bounded so a stalled submission fails the test instead of
            // spinning forever (see `data_render::shared_device`).
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            });
            let out = slice.get_mapped_range().to_vec();
            out
        };

        for (label, prepared) in [("stale-cache token p1", &p1), ("fresh token p2", &p2)] {
            let rgba = paint_once(prepared);
            let lit = rgba.chunks_exact(4).filter(|p| p[3] > 0).count();
            assert!(
                lit > 100,
                "{label}: milkyway split paint produced too little ink: {lit}"
            );
        }
    }

    /// Log-axis auto-fit with zeros in the data must clamp the lower bound
    /// to the smallest POSITIVE value (via the pool's CPU shadow) instead of
    /// feeding log10 a non-positive min.
    #[test]
    fn log_auto_fit_clamps_to_smallest_positive() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .unwrap();
        // Data spans 0..1000 with smallest positive 0.04.
        let vals = vec![0.0, 0.04, 1.0, 50.0, 1000.0];
        r.add_column("v", &col_f64(vals)).unwrap();

        let mut config = crate::default::default_config();
        config.left_y.scale = crate::config::AxisScale::Logarithmic;
        let mut chart = Chart::new(config);
        chart.auto_fit_y_union(r.pool(), &["v"], 0.0).unwrap();

        let min = chart.config().left_y.min;
        assert!(
            (min - 0.04).abs() < 1e-9,
            "log fit lower bound must be the smallest positive value, got {min}"
        );
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

    /// A solid line stays CONTINUOUS no matter how dense the data is. With
    /// sub-pixel segments (here ~0.04 px each) naive per-segment quads
    /// degenerate into disconnected slivers — every x column the curve
    /// crosses must contain ink.
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
        let ts: Vec<f64> = (0..n).map(|i| i as f64 * 6.28 / (n - 1) as f64).collect();
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
        chart.set_x_range(0.0, 6.28);
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
        for x in x0..=x1 {
            let (on, ds) = match col_y[x] {
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
}

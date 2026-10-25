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
mod series;
pub use series::*;
mod window;
pub use window::*;
mod composite;
pub use composite::draw_chart_panel_columnar;
mod frame;
pub use frame::*;
mod prepare;
use prepare::*;
mod preparation;
pub use preparation::{PipelinePreparation, PreparationFeature, PreparedPipelines};

use std::borrow::Cow;
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
    DefragPolicy, ERRORBAR_HAS_X, ERRORBAR_HAS_Y, PrimitiveStyle, RaggedId, RaggedSource, RecordId,
    RecordSource,
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
    /// Category bar data layout (pool + `CategoryBarParams`), group 3 of the
    /// category bar pipelines and pick.
    category_bar_data_bgl: wgpu::BindGroupLayout,
    /// Value label layout (uniform + atlas + sampler), group 1 of the value
    /// label pipelines of the bar module.
    value_label_bgl: wgpu::BindGroupLayout,
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
    /// the series has labels. A category bar, box plot or radar series with
    /// value labels holds its baked label atlas here too
    /// (`ScratchEntry::ValueLabels`), keyed as a contour's atlas is.
    contour_cache: HashMap<String, ScratchEntry>,
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

// One descriptor fully defines each stylized render mode. The precise path is
// represented by `style_variant() == None` and never routes through this table.

impl Renderer {
    // Handle / accessor methods.

    // Column management (returns Result).

    // Standalone path — figgy owns instance / adapter / device / queue / surface.
    //
    // For hosts where figgy renders the window directly (e.g. winit). Caller
    // writes zero lines of wgpu setup code.

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

    // Headless PNG export.
}

// Export result type, scale clamping, PNG encoding.

// Public auxiliary types.

// DataRenderType branching helpers — which layers are needed and which
// sub-style to extract.

// WindowedRenderer — figgy owns the surface and swap chain.

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "serde"))]
pub(crate) mod config_mutation;

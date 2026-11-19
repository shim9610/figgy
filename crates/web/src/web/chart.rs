//! `FiggyChart`: construction, lifecycle, the document (config and
//! series), and the frame loop.

use super::*;

mod documents;
mod frame;
mod init;
mod kernel;
mod lifecycle;
mod model_api;
mod state;

pub(super) use documents::*;
pub use init::*;
pub(super) use kernel::*;
pub use model_api::*;

// ------------------------------------------------------------------
// FiggyChart — low-level wasm kernel bound to one canvas.
// ------------------------------------------------------------------

#[wasm_bindgen]
pub struct FiggyChart {
    pub(super) renderer: WindowedRenderer<'static>,
    pub(super) chart_id: ChartId,
    pub(super) view: ChartView,
    /// Current WebGPU surface size in physical canvas pixels. This is a
    /// viewport property, not the exported document size.
    pub(super) surface_size: (u32, u32),
    /// True while the legend follows the wrapper's auto row layout.
    /// Direct `set_config` legend edits turn this off so later series
    /// changes preserve user-authored text instead of deleting rows.
    pub(super) legend_auto_managed: bool,
    /// Monotonic color assignment for newly registered series.
    pub(super) color_seq: usize,
    pub(super) hitmap: HitMap,
    /// The (active layer, overlay count, active subplot's overlay count,
    /// subplot count, active overlay's `link_x` / `link_y`) the hit map was
    /// registered for. Any change rebuilds it with
    /// `HitMap::for_active_layer`.
    pub(super) hitmap_layer: (
        renderer::LayerRef,
        usize,
        Option<usize>,
        usize,
        Option<(bool, bool)>,
    ),
    /// The polar category label counts, chart then subplots, the hit map
    /// was registered for (`Config::polar_category_label_counts`, #309):
    /// one entry per panel, compared in place and replaced only when they
    /// change.
    pub(super) hitmap_categories: Vec<usize>,
    pub(super) dragging: bool,
    /// The pointer in document coordinates while a chart element drags:
    /// the press point advanced by every move. Polar axes and tick
    /// labels move relative to it.
    pub(super) drag_pointer: [f32; 2],
    pub(super) resizing: Option<ModelResizeHandle>,
    pub(super) cycle: renderer::ColorCycle,
    pub(super) clear_color: Color,
    pub(super) view_dirty: bool,
    /// Host-owned surface state not represented by renderer chart
    /// revisions (currently the clear color). Starts true so the initial
    /// surface frame is never skipped.
    pub(super) redraw_pending: bool,
    /// Renderer-issued identity of the state last presented onscreen.
    pub(super) last_presented_stamp: Option<ChartRenderStamp>,
    /// The annotation whose selection box the overlay draws. Host state
    /// like the decoration selection; not part of the config JSON.
    pub(super) selected_annotation: Option<usize>,
    /// The annotation selection changed since the overlay was drawn.
    pub(super) annotation_selection_dirty: bool,
    /// The pointer press that started the current drag or handle drag
    /// landed on the selected annotation (the annotation layer's route),
    /// so `on_move` edits the annotation instead of a chart element.
    pub(super) annotation_pointer: bool,
    /// The current drag started on a value label (`press_value_label`), so
    /// `on_move` moves the selected label instead of a chart element.
    pub(super) value_label_pointer: bool,
    /// A subplot frame drag holds the display placement
    /// (`Renderer::hold_chart_display`, #205) until the pointer's release.
    pub(super) display_held: bool,
    /// The current drag started on a part of a Feynman series
    /// (`press_feynman`, #234): the series and what `on_move` moves.
    pub(super) feynman_pointer: Option<(String, renderer::FeynmanHit)>,
    /// The document a plot-area resize gesture proposes on a chart with
    /// monotonic series (#271), built up move by move and never applied:
    /// the resize takes the groups' sources, which the host passes.
    pub(super) plot_resize: Option<renderer::Config>,
    /// The plot rect the last such gesture proposed on release, until the
    /// element reads it (`take_plot_resize_request`).
    pub(super) plot_resize_request: Option<renderer::layout::Rect>,
    /// The monotonic chart the kernel shows instead of its general one
    /// (`set_monotonic_document`, #271), reached only through its handle.
    pub(super) monotonic: Option<renderer::MonotonicChartId>,
}

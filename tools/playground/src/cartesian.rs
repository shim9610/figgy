//! Cartesian scenes: one registered chart, drawn into the host surface.
//!
//! The registry (`Renderer::chart_config`) holds the logical document; the
//! window shows a scaled, letterboxed display copy of it, the same split the
//! WASM host uses. Pointer deltas are divided by the display scale before
//! they edit the document, so drags stay exact at any window size.

use renderer::config::{AxisOptions, AxisScale, Config, DrawStyle, MilkywayOptions, SketchOptions};
use renderer::data_config::{
    BarOrientation, DataRenderType, LineFill, LineInterpolation, PointColorMap, PointSizeMap,
    SeriesConfig, Shading, SizeScale, VectorColorMap, VectorComponents, VectorPivot, VectorScale,
    VectorStyle,
};
use renderer::layout::ChartArea;
use renderer::layout::{NudgeResult, Rect};
use renderer::legend::LegendCorner;
use renderer::text::{RichText, rich_segments_from_text};
use renderer::{
    Anchor, Annotation, AnnotationAxis, AnnotationKind, AnnotationLineStyle, AnnotationMap,
    ArrowStyle, AxisPreset, CellRef, Chart, ChartDrawItem, ChartId, ChartStyle, ChartView, Color,
    ColorCycle, ColorMap, CpuTextMeasure, DataSelectionsConfig, GpuPickRequest, HitId, HitMap,
    LayerHit, LayerRef, OverlayLayer, PickedData, PreparedFrame, RenderWarning, Renderer,
    ResizeHandle, ScatterShape, Series, Subplot, fit_display_panel, place_document,
};

use crate::HostCtx;
use crate::big_data::{self, BigData, FrameTime};
use crate::scenes::{self, SceneKind};
use crate::widgets::{self, choice, color, rich_text, section, slider};

const AXIS_PRESETS: [AxisPreset; 5] = [
    AxisPreset::BoxedInward,
    AxisPreset::BoxedOutward,
    AxisPreset::OpenOutward,
    AxisPreset::OpenInward,
    AxisPreset::Minimal,
];
const COLOR_CYCLES: [ColorCycle; 5] = [
    ColorCycle::Classic,
    ColorCycle::Vivid,
    ColorCycle::Balanced,
    ColorCycle::ColorblindSafe,
    ColorCycle::Monochrome,
];
const SHAPES: &[ScatterShape] = &[
    ScatterShape::Circle,
    ScatterShape::CircleFilled,
    ScatterShape::Square,
    ScatterShape::SquareFilled,
    ScatterShape::Triangle,
    ScatterShape::TriangleFilled,
    ScatterShape::Diamond,
    ScatterShape::DiamondFilled,
    ScatterShape::Cross,
    ScatterShape::Plus,
    ScatterShape::Star,
    ScatterShape::Hexagon,
];
const INTERPOLATIONS: &[LineInterpolation] = &[
    LineInterpolation::Linear,
    LineInterpolation::StepPre,
    LineInterpolation::StepPost,
    LineInterpolation::StepMid,
];
const COLORMAPS: &[ColorMap] = &[
    ColorMap::Viridis,
    ColorMap::Magma,
    ColorMap::Turbo,
    ColorMap::GrayScale,
    ColorMap::RdBu,
];
const SIZE_SCALES: &[SizeScale] = &[SizeScale::Area, SizeScale::Radius];
const CORNERS: &[LegendCorner] = &[
    LegendCorner::TopLeft,
    LegendCorner::TopRight,
    LegendCorner::BottomLeft,
    LegendCorner::BottomRight,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum PointerMode {
    Edit,
    Pick,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Element,
    Resize(ResizeHandle),
    Annotation,
    AnnotationHandle(ResizeHandle),
}

/// The document as currently displayed.
struct Display {
    config: Config,
    panel: Rect,
    document: Rect,
    scale: f32,
}

/// Which chart of the document a pan / zoom moves.
#[derive(Clone, Copy)]
enum ViewTarget {
    Parent { overlay: Option<usize> },
    Subplot(usize),
}

pub struct CartesianHost {
    kind: SceneKind,
    id: ChartId,
    initial: Config,
    initial_series: Vec<SeriesConfig>,
    view: Option<ChartView>,
    styles: Vec<ChartStyle>,
    /// Display scale the styles were built at; `None` forces a rebuild.
    styles_scale: Option<f32>,
    raster_dirty: bool,
    /// Central egui area in physical surface pixels.
    viewport_px: Rect,
    selected: Option<HitId>,
    selected_annotation: Option<usize>,
    drag: Drag,
    mode: PointerMode,
    picks: Vec<PickedData>,
    /// One line per pick: the picked point's coordinates and, for a colour-
    /// or size-mapped series, the values its colour and radius came from,
    /// read back from the GPU column the draw used.
    pick_notes: Vec<String>,
    cursor_data: Option<(f64, f64)>,
    warnings: Vec<RenderWarning>,
    error: Option<String>,
    color_cycle: usize,
    export_scale: f32,
    /// Size, mode and timings of the large-data scene; `None` elsewhere.
    big: Option<BigData>,
}

impl CartesianHost {
    pub fn new(r: &mut Renderer, kind: SceneKind) -> Result<Self, renderer::FiggyError> {
        let scene = scenes::build(r, kind, big_data::START_POINTS)?;
        let id = r.register_chart(scene.config.clone(), scene.series.clone())?;
        Ok(Self {
            kind,
            id,
            initial: scene.config,
            initial_series: scene.series,
            view: None,
            styles: Vec::new(),
            styles_scale: None,
            raster_dirty: true,
            viewport_px: Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            selected: None,
            selected_annotation: None,
            drag: Drag::None,
            mode: PointerMode::Edit,
            picks: Vec::new(),
            pick_notes: Vec::new(),
            cursor_data: None,
            warnings: Vec::new(),
            error: None,
            color_cycle: 0,
            export_scale: 2.0,
            big: (kind == SceneKind::BigData).then(BigData::new),
        })
    }

    pub fn remove(self, r: &mut Renderer) {
        drop(self.view);
        drop(self.styles);
        if let Err(e) = r.remove_chart(self.id) {
            eprintln!("[scene] remove chart: {e}");
        }
        if self.big.is_some() {
            big_data::release(r);
        }
    }

    /// Whether the host should wait for the GPU after each frame and report
    /// its time through [`Self::frame_timed`].
    pub fn wants_frame_timing(&self) -> bool {
        self.big.as_ref().is_some_and(BigData::wants_timing)
    }

    /// Record one frame's time. Returns whether another frame is needed now
    /// (a benchmark is running).
    pub fn frame_timed(
        &mut self,
        r: &mut Renderer,
        prepare: std::time::Duration,
        total: std::time::Duration,
    ) -> bool {
        let Some(big) = self.big.as_mut() else {
            return false;
        };
        if let Some((min, max)) = big.frame_done(FrameTime { prepare, total }) {
            let mut chart = Chart::new(self.config(r));
            chart.set_x_range(min, max);
            self.commit(r, chart.config().clone());
        }
        self.big
            .as_ref()
            .is_some_and(|b| b.benchmarking().is_some())
    }

    fn config(&self, r: &Renderer) -> Config {
        r.chart_config(self.id).expect("registered chart").clone()
    }

    fn series(&self, r: &Renderer) -> Vec<SeriesConfig> {
        r.chart_series(self.id).expect("registered chart").to_vec()
    }

    fn commit(&mut self, r: &mut Renderer, config: Config) -> bool {
        match r.set_chart_config(self.id, config) {
            Ok(()) => {
                self.raster_dirty = true;
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(format!("config rejected: {e}"));
                false
            }
        }
    }

    /// Replace the series, then recolour the legends to match.
    fn commit_series(&mut self, r: &mut Renderer, series: Vec<SeriesConfig>) {
        let previous = self.series(r);
        if let Err(e) = r.set_chart_series(self.id, series.clone()) {
            self.error = Some(format!("series rejected: {e}"));
            return;
        }
        let mut config = self.config(r);
        scenes::rebuild_legends(&mut config, &series);
        if !self.commit(r, config) {
            let _ = r.set_chart_series(self.id, previous);
        }
        self.styles_scale = None;
    }

    fn display(&self, r: &Renderer) -> Option<Display> {
        let cfg = r.chart_config(self.id).ok()?;
        let vp = self.viewport_px;
        if vp.width < 8 || vp.height < 8 {
            return None;
        }
        let doc = cfg.document_area();
        let (scale, mut document) =
            fit_display_panel((doc.width, doc.height), (vp.width, vp.height));
        document.x += vp.x;
        document.y += vp.y;
        let config = place_document(cfg, scale, document);
        let panel = config.chart_area.0;
        Some(Display {
            config,
            panel,
            document,
            scale,
        })
    }

    // ------------------------------------------------------------------
    // Frame
    // ------------------------------------------------------------------

    /// Refresh derived resources and prepare this frame's draw token.
    pub fn prepare(&mut self, r: &mut Renderer) -> Option<PreparedFrame> {
        if let Some((min, max)) = self.big.as_ref().and_then(BigData::bench_range) {
            let mut chart = Chart::new(self.config(r));
            chart.set_x_range(min, max);
            self.commit(r, chart.config().clone());
        }
        let d = self.display(r)?;
        let series = self.series(r);
        if self.styles_scale != Some(d.scale) {
            match series
                .iter()
                .map(|s| r.create_style_for_series_scaled(s, d.scale))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(styles) => {
                    self.styles = styles;
                    self.styles_scale = Some(d.scale);
                }
                Err(e) => {
                    self.error = Some(format!("style: {e}"));
                    return None;
                }
            }
        }
        let chart = Chart::new(d.config.clone());
        let view = match self.view.take() {
            Some(v) => v,
            None => match r.create_chart_view(&chart, d.panel) {
                Ok(v) => {
                    self.raster_dirty = true;
                    v
                }
                Err(e) => {
                    self.error = Some(format!("view: {e}"));
                    return None;
                }
            },
        };
        let mut view = view;
        if self.raster_dirty || view.panel_rect() != d.panel {
            let hitmap = HitMap::for_active_layer(&d.config);
            let selected = self
                .selected
                .and_then(|id| hitmap.get(id))
                .map(|el| el.element_id());
            if let Err(e) =
                r.refresh_axis_with_selection(&mut view, &chart, d.panel, selected.as_deref())
            {
                self.error = Some(format!("refresh: {e}"));
            }
            self.warnings = view.render_warnings().to_vec();
            self.raster_dirty = false;
        }
        if view.annotation_selected() != self.selected_annotation
            && let Err(e) =
                r.refresh_annotations(&mut view, &chart, d.panel, self.selected_annotation)
        {
            self.error = Some(format!("annotations: {e}"));
        }
        let draw: Vec<Series<'_>> = series
            .iter()
            .zip(&self.styles)
            .map(|(config, style)| Series { config, style })
            .collect();
        let prepared = r.prepare(&[ChartDrawItem {
            view: &view,
            chart_config: &d.config,
            series: &draw,
        }]);
        drop(draw);
        self.view = Some(view);
        match prepared {
            Ok(p) => Some(p),
            Err(e) => {
                self.error = Some(format!("prepare: {e}"));
                None
            }
        }
    }

    // ------------------------------------------------------------------
    // Pointer
    // ------------------------------------------------------------------

    pub fn viewport(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
        r: &mut Renderer,
        host: &mut HostCtx<'_>,
    ) {
        let ppp = host.pixels_per_point;
        self.viewport_px = Rect {
            x: (rect.min.x * ppp).round().max(0.0) as u32,
            y: (rect.min.y * ppp).round().max(0.0) as u32,
            width: (rect.width() * ppp).floor().max(1.0) as u32,
            height: (rect.height() * ppp).floor().max(1.0) as u32,
        };
        let Some(d) = self.display(r) else {
            return;
        };
        let (pressed, down, released, pan_down, delta, scroll, mods, pos, origin) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.secondary_down() || i.pointer.middle_down(),
                i.pointer.delta(),
                i.smooth_scroll_delta,
                i.modifiers,
                i.pointer.hover_pos(),
                i.pointer.press_origin(),
            )
        });
        let px = |p: egui::Pos2| [p.x * ppp, p.y * ppp];
        let inside = pos.is_some_and(|p| rect.contains(p));
        let mut changed = false;

        self.cursor_data = pos.filter(|_| inside).and_then(|p| self.data_at(&d, px(p)));

        if pressed && origin.is_some_and(|p| rect.contains(p)) {
            let p = px(origin.unwrap());
            match self.mode {
                PointerMode::Edit => self.on_press(r, &d, p, mods.shift, ppp),
                PointerMode::Pick => {
                    let picked = self.query_pick(r, &d, p, ppp);
                    self.select_pick(r, picked, mods.shift);
                }
            }
            *host.status = match self.picks.last() {
                Some(p) => format!("Picked: {}", describe_pick(p)),
                None => String::new(),
            };
            changed = true;
        }
        if down && self.drag != Drag::None && (delta.x != 0.0 || delta.y != 0.0) {
            changed |= self.on_move(r, &d, delta.x * ppp, delta.y * ppp);
        }
        if released {
            self.drag = Drag::None;
        }
        if pan_down && response.hovered() && (delta.x != 0.0 || delta.y != 0.0) {
            changed |= self.pan(r, &d, delta.x * ppp, delta.y * ppp);
        }
        if inside && scroll.y != 0.0 {
            let factor = (-scroll.y as f64 * 0.0025).exp();
            let (zx, zy) = if mods.shift {
                (true, false)
            } else if mods.command {
                (false, true)
            } else {
                (true, true)
            };
            changed |= self.zoom(r, &d, px(pos.unwrap()), factor, zx, zy);
        }
        if let Some((x, y)) = self.cursor_data {
            response
                .clone()
                .on_hover_text_at_pointer(format!("x = {x:.4}\ny = {y:.4}"));
        }
        if changed {
            ui.ctx().request_repaint();
        }
    }

    fn on_press(&mut self, r: &mut Renderer, d: &Display, p: [f32; 2], add: bool, ppp: f32) {
        let Some(view) = &self.view else { return };
        let hitmap = HitMap::for_active_layer(&d.config);
        let measure = CpuTextMeasure::for_style(&d.config.draw_style);
        let routes =
            AnnotationMap::receives_pointer(d.config.active_layer(), &d.config.annotations);
        let map = view.annotation_map();
        if routes
            && let Some(index) = self.selected_annotation
            && let Some(id) = map.find(index)
            && let Some(handle) = map.hit_resize_handle(id, p)
        {
            self.drag = Drag::AnnotationHandle(handle);
            return;
        }
        if let Some(LayerHit::Annotation { index, .. }) =
            view.hit_test_active_layer(&d.config, &hitmap, p)
        {
            if self.selected.take().is_some() {
                self.raster_dirty = true;
            }
            self.selected_annotation = Some(index);
            self.drag = Drag::Annotation;
            return;
        }
        if let Some(id) = self.selected
            && let Some(rz) = hitmap.get(id).and_then(|e| e.as_resizable())
            && let Some(handle) = rz.hit_resize_handle(&d.config, &measure, p[0], p[1])
        {
            self.drag = Drag::Resize(handle);
            return;
        }
        let hit = hitmap.hit_test(&d.config, &measure, p[0], p[1]);
        // Data wins over the plot area until the area itself is selected
        // (select it from an axis-free spot, e.g. "Deselect" then a click on
        // empty plot space), so a plain click on a point / bar / cell picks it.
        let on_area = hit
            .and_then(|id| hitmap.get(id))
            .is_some_and(|e| e.element_id().ends_with("data_area"));
        if (hit.is_none() || (on_area && hit != self.selected))
            && let Some(picked) = self.query_pick(r, d, p, ppp)
        {
            if self.selected.take().is_some() {
                self.raster_dirty = true;
            }
            self.selected_annotation = None;
            self.drag = Drag::None;
            self.select_pick(r, Some(picked), add);
            return;
        }
        if !add && !self.picks.is_empty() {
            self.select_pick(r, None, false);
        }
        let draggable =
            hit.is_some_and(|id| hitmap.get(id).is_some_and(|e| e.as_draggable().is_some()));
        if hit != self.selected {
            self.selected = hit;
            self.raster_dirty = true;
        }
        if hit.is_some() || routes {
            self.selected_annotation = None;
        }
        self.drag = if draggable { Drag::Element } else { Drag::None };
    }

    fn on_move(&mut self, r: &mut Renderer, d: &Display, dx: f32, dy: f32) -> bool {
        let mut config = self.config(r);
        let moved = match self.drag {
            Drag::None => return false,
            Drag::Annotation | Drag::AnnotationHandle(_) => {
                let (Some(view), Some(index)) = (&self.view, self.selected_annotation) else {
                    return false;
                };
                let map = view.annotation_map();
                let Some(id) = map.find(index) else {
                    return false;
                };
                let result = match self.drag {
                    Drag::AnnotationHandle(h) => {
                        map.resize_by(&mut config.annotations, id, h, dx, dy)
                    }
                    _ => map.drag_by(&mut config.annotations, id, dx, dy),
                };
                result == Some(NudgeResult::Moved)
            }
            Drag::Element | Drag::Resize(_) => {
                let Some(id) = self.selected else {
                    return false;
                };
                let (dx, dy) = (dx / d.scale, dy / d.scale);
                let hitmap = HitMap::for_active_layer(&d.config);
                let measure = CpuTextMeasure::for_style(&d.config.draw_style);
                let Some(el) = hitmap.get(id) else {
                    return false;
                };
                match self.drag {
                    Drag::Resize(h) => el.as_resizable().is_some_and(|rz| {
                        rz.resize_by(&mut config, &measure, h, dx, dy) == NudgeResult::Moved
                    }),
                    _ => el.as_draggable().is_some_and(|dr| {
                        dr.drag_by(&mut config, &measure, dx, dy) == NudgeResult::Moved
                    }),
                }
            }
        };
        moved && self.commit(r, config)
    }

    /// One GPU pick at a physical surface position; errors land in `self.error`.
    fn query_pick(
        &mut self,
        r: &mut Renderer,
        d: &Display,
        p: [f32; 2],
        ppp: f32,
    ) -> Option<PickedData> {
        let ticket = r.pick_chart_data(
            self.id,
            GpuPickRequest {
                canvas_position_px: p,
                display_panel_px: d.document,
                display_scale: d.scale,
                max_distance_px: 10.0 * ppp,
            },
        );
        match ticket {
            Ok(t) => match pollster::block_on(t.resolve()) {
                Ok(p) => p,
                Err(e) => {
                    self.error = Some(format!("pick resolve: {e}"));
                    None
                }
            },
            Err(e) => {
                self.error = Some(format!("pick: {e}"));
                None
            }
        }
    }

    /// Replace (or with `add`, extend) the data selection and highlight it.
    fn select_pick(&mut self, r: &mut Renderer, picked: Option<PickedData>, add: bool) {
        if !add {
            self.picks.clear();
            self.pick_notes.clear();
        }
        if let Some(p) = picked {
            let note = match pick_values(r, &self.series(r), &p) {
                Ok(note) => note,
                Err(e) => {
                    self.error = Some(format!("pick values: {e}"));
                    String::new()
                }
            };
            self.picks.push(p);
            self.pick_notes.push(note);
        }
        self.apply_picks(r);
    }

    fn apply_picks(&mut self, r: &mut Renderer) {
        let mut config = self.config(r);
        config.picked_data = (!self.picks.is_empty()).then(|| DataSelectionsConfig {
            visible: true,
            refs: self.picks.iter().map(PickedData::to_ref).collect(),
            // The default (3 px ring, gold) nearly vanishes on a 2 px line;
            // a test bench wants the hit obvious.
            highlight_color: Color::from_rgb8(230, 20, 120),
            outline_width_px: 3.0,
            point_radius_extra_px: 8.0,
            contour_width_extra_px: 4.0,
        });
        self.commit(r, config);
    }

    // ------------------------------------------------------------------
    // Pan / zoom
    // ------------------------------------------------------------------

    fn view_target(config: &Config) -> ViewTarget {
        match config.active_layer() {
            LayerRef::Overlay(i) => ViewTarget::Parent { overlay: Some(i) },
            LayerRef::Subplot(i) | LayerRef::SubplotOverlay(i, _) => ViewTarget::Subplot(i),
            _ => ViewTarget::Parent { overlay: None },
        }
    }

    /// Display data area plus the authored X / Y axes of the target chart.
    fn target_axes(
        &self,
        r: &Renderer,
        d: &Display,
    ) -> Option<(ViewTarget, Rect, AxisOptions, AxisOptions)> {
        let config = self.config(r);
        let target = Self::view_target(&config);
        Some(match target {
            ViewTarget::Parent { overlay } => {
                let area = d.config.data_area().ok()?.0;
                let y = match overlay {
                    Some(i) => config.overlays.get(i)?.y_axis.clone(),
                    None => config.left_y.clone(),
                };
                (target, area, config.bottom_x.clone(), y)
            }
            ViewTarget::Subplot(i) => {
                let area = d.config.subplot_config(i)?.data_area().ok()?.0;
                let sub = config.subplot_config(i)?;
                (target, area, sub.bottom_x.clone(), sub.left_y.clone())
            }
        })
    }

    fn data_at(&self, d: &Display, p: [f32; 2]) -> Option<(f64, f64)> {
        let area = d.config.data_area().ok()?.0;
        let fx = (p[0] - area.x as f32) / area.width as f32;
        let fy = 1.0 - (p[1] - area.y as f32) / area.height as f32;
        if !(0.0..=1.0).contains(&fx) || !(0.0..=1.0).contains(&fy) {
            return None;
        }
        let at = |axis: &AxisOptions, f: f32| {
            let f = if axis.inverted { 1.0 - f } else { f } as f64;
            let (a, b) = (to_space(axis, axis.min), to_space(axis, axis.max));
            from_space(axis, a + (b - a) * f)
        };
        Some((at(&d.config.bottom_x, fx), at(&d.config.left_y, fy)))
    }

    fn apply_ranges(
        &mut self,
        r: &mut Renderer,
        target: ViewTarget,
        x: Option<(f64, f64)>,
        y: Option<(f64, f64)>,
    ) -> bool {
        let ok = |v: &(f64, f64)| v.0.is_finite() && v.1.is_finite() && v.1 > v.0;
        let mut chart = Chart::new(self.config(r));
        match target {
            ViewTarget::Parent { overlay } => {
                if let Some(x) = x.filter(ok) {
                    chart.set_x_range(x.0, x.1);
                }
                if let Some(y) = y.filter(ok) {
                    match overlay {
                        Some(i) => chart.set_overlay_y_range(i, y.0, y.1),
                        None => chart.set_y_range(y.0, y.1),
                    }
                }
            }
            ViewTarget::Subplot(i) => {
                if let Some(x) = x.filter(ok) {
                    chart.set_subplot_x_range(i, x.0, x.1);
                }
                if let Some(y) = y.filter(ok) {
                    chart.set_subplot_y_range(i, y.0, y.1);
                }
            }
        }
        let config = chart.config().clone();
        self.commit(r, config)
    }

    fn pan(&mut self, r: &mut Renderer, d: &Display, dx: f32, dy: f32) -> bool {
        let Some((target, area, xa, ya)) = self.target_axes(r, d) else {
            return false;
        };
        let shift = |axis: &AxisOptions, frac: f64| {
            let frac = if axis.inverted { -frac } else { frac };
            let (a, b) = (to_space(axis, axis.min), to_space(axis, axis.max));
            let s = (b - a) * frac;
            (from_space(axis, a + s), from_space(axis, b + s))
        };
        let x = shift(&xa, -(dx as f64) / area.width as f64);
        let y = shift(&ya, dy as f64 / area.height as f64);
        self.apply_ranges(r, target, Some(x), Some(y))
    }

    fn zoom(
        &mut self,
        r: &mut Renderer,
        d: &Display,
        p: [f32; 2],
        factor: f64,
        zx: bool,
        zy: bool,
    ) -> bool {
        let Some((target, area, xa, ya)) = self.target_axes(r, d) else {
            return false;
        };
        let fx = ((p[0] - area.x as f32) / area.width as f32).clamp(0.0, 1.0) as f64;
        let fy = (1.0 - (p[1] - area.y as f32) / area.height as f32).clamp(0.0, 1.0) as f64;
        let scale = |axis: &AxisOptions, f: f64| {
            let f = if axis.inverted { 1.0 - f } else { f };
            let (a, b) = (to_space(axis, axis.min), to_space(axis, axis.max));
            let c = a + (b - a) * f;
            (
                from_space(axis, c - (c - a) * factor),
                from_space(axis, c + (b - c) * factor),
            )
        };
        self.apply_ranges(
            r,
            target,
            zx.then(|| scale(&xa, fx)),
            zy.then(|| scale(&ya, fy)),
        )
    }

    // ------------------------------------------------------------------
    // Control panel
    // ------------------------------------------------------------------

    pub fn controls(&mut self, ui: &mut egui::Ui, r: &mut Renderer, host: &mut HostCtx<'_>) {
        let mut config = self.config(r);
        let before = config.clone();
        let mut series = self.series(r);
        let series_before = series.clone();
        let big_action = self
            .big
            .as_mut()
            .and_then(|big| big_data_controls(ui, big, r, host));

        section(ui, "Interaction", true, |ui| {
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.mode, PointerMode::Edit, "Select / drag");
                ui.radio_value(&mut self.mode, PointerMode::Pick, "Pick data");
            });
            let mut layer = config.active_layer();
            let mut layers = vec![LayerRef::Base];
            layers.extend((0..config.overlays.len()).map(LayerRef::Overlay));
            for (i, sub) in config.subplots.iter().enumerate() {
                layers.push(LayerRef::Subplot(i));
                layers
                    .extend((0..sub.config.overlays.len()).map(|j| LayerRef::SubplotOverlay(i, j)));
            }
            layers.push(LayerRef::Annotations);
            if choice(ui, "Active layer", &mut layer, &layers) {
                config.set_active_layer(layer);
                self.selected = None;
                self.selected_annotation = None;
            }
            ui.horizontal(|ui| {
                if ui.button("Reset view").clicked() {
                    reset_ranges(&mut config, &self.initial);
                }
                if ui.button("Reset scene").clicked() {
                    config = self.initial.clone();
                    series = self.initial_series.clone();
                    self.picks.clear();
                    self.pick_notes.clear();
                }
                if ui.button("Deselect").clicked() {
                    self.selected = None;
                    self.selected_annotation = None;
                    self.raster_dirty = true;
                }
            });
            let hitmap = HitMap::for_active_layer(&config);
            let name = self
                .selected
                .and_then(|id| hitmap.get(id))
                .map(|e| e.element_id())
                .or_else(|| self.selected_annotation.map(|i| format!("annotation_{i}")))
                .unwrap_or_else(|| "nothing".into());
            ui.label(format!("Selected: {name}"));
            match self.cursor_data {
                Some((x, y)) => ui.label(format!("Cursor: x = {x:.4}, y = {y:.4}")),
                None => ui.label("Cursor: outside the plot"),
            };
            ui.label(
                egui::RichText::new("Wheel zooms (Shift: X, Ctrl: Y), right/middle drag pans.")
                    .weak()
                    .small(),
            );
        });

        section(ui, "Chart", true, |ui| {
            let mut mode = draw_style_index(&config.draw_style);
            let names = ["Precise", "Sketch", "Milkyway", "Constellation"];
            ui.horizontal(|ui| {
                ui.label("Draw style");
                egui::ComboBox::from_id_salt("draw_style")
                    .selected_text(names[mode])
                    .show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            ui.selectable_value(&mut mode, i, *n);
                        }
                    });
            });
            if mode != draw_style_index(&config.draw_style) {
                config.draw_style = match mode {
                    1 => DrawStyle::Sketch(SketchOptions::default()),
                    2 => DrawStyle::Milkyway(MilkywayOptions::default()),
                    3 => DrawStyle::Constellation(Default::default()),
                    _ => DrawStyle::Precise,
                };
                self.styles_scale = None;
            }
            if let DrawStyle::Sketch(o) = &mut config.draw_style {
                slider(ui, "amplitude px", &mut o.amplitude_px, 0.0..=6.0);
                slider(ui, "wavelength px", &mut o.wavelength_px, 10.0..=200.0);
                ui.add(egui::Slider::new(&mut o.seed, 0..=99).text("seed"));
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Axis preset:");
                for preset in AXIS_PRESETS {
                    if ui.small_button(format!("{preset:?}")).clicked() {
                        config.apply_axis_preset(preset);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Colour cycle");
                let mut idx = self.color_cycle;
                egui::ComboBox::from_id_salt("cycle")
                    .selected_text(format!("{:?}", COLOR_CYCLES[idx]))
                    .show_ui(ui, |ui| {
                        for (i, c) in COLOR_CYCLES.iter().enumerate() {
                            ui.selectable_value(&mut idx, i, format!("{c:?}"));
                        }
                    });
                if idx != self.color_cycle {
                    self.color_cycle = idx;
                    COLOR_CYCLES[idx].apply_to_all(&mut series);
                }
            });
            rich_text(ui, "Title", &mut config.chart_title.text);
            rich_text(ui, "X title", &mut config.bottom_x.title_option.text);
            rich_text(ui, "Y title", &mut config.left_y.title_option.text);
            ui.horizontal(|ui| {
                ui.checkbox(&mut config.grid.show_major_x, "major X");
                ui.checkbox(&mut config.grid.show_major_y, "major Y");
                ui.checkbox(&mut config.grid.show_minor_x, "minor X");
                ui.checkbox(&mut config.grid.show_minor_y, "minor Y");
            });
            ui.horizontal(|ui| {
                let mut log_x = config.bottom_x.scale == AxisScale::Logarithmic;
                let mut log_y = config.left_y.scale == AxisScale::Logarithmic;
                if ui.checkbox(&mut log_x, "log X").changed() {
                    set_log(&mut config, true, log_x);
                }
                if ui.checkbox(&mut log_y, "log Y").changed() {
                    set_log(&mut config, false, log_y);
                }
                let mut inv_x = config.bottom_x.inverted;
                let mut inv_y = config.left_y.inverted;
                if ui.checkbox(&mut inv_x, "invert X").changed() {
                    config.bottom_x.inverted = inv_x;
                    config.top_x.inverted = inv_x;
                }
                if ui.checkbox(&mut inv_y, "invert Y").changed() {
                    config.left_y.inverted = inv_y;
                    config.right_y.inverted = inv_y;
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut config.legend.visible, "Legend");
                choice(ui, "corner", &mut config.legend.corner, CORNERS);
            });
            if let Some(bar) = config.colorbar.as_mut() {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut bar.visible, "Colourbar");
                    choice(ui, "map", &mut bar.colormap, COLORMAPS);
                });
            }
        });

        section(ui, "Layers: overlays & subplots", true, |ui| {
            let initial_right = self.initial.right_y.clone();
            if let Some(msg) = layers_controls(ui, &mut config, &series, r, &initial_right) {
                // Legends list what each chart draws, so they follow the move.
                scenes::rebuild_legends(&mut config, &series);
                *host.status = msg;
                self.selected = None;
                self.selected_annotation = None;
            }
        });

        section(ui, "Series", false, |ui| {
            // Columns a value map may name: everything the scene's series
            // reference now or referenced when the scene started.
            let mut columns: Vec<String> = Vec::new();
            for s in series.iter().chain(&self.initial_series) {
                for c in series_columns(s) {
                    if !columns.contains(&c) {
                        columns.push(c);
                    }
                }
            }
            for (i, s) in series.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    egui::CollapsingHeader::new(format!(
                        "{} ({})",
                        s.series_id,
                        render_type_name(&s.render_type)
                    ))
                    .default_open(i == 0)
                    .show(ui, |ui| series_controls(ui, s, &mut config, &columns, r));
                });
            }
        });

        section(ui, "Picking", true, |ui| {
            ui.label("Click a point, bar, cell or isoline (Shift adds). “Pick data” mode never selects chrome.");
            for (p, note) in self.picks.iter().zip(&self.pick_notes) {
                ui.label(egui::RichText::new(describe_pick(p)).monospace());
                if !note.is_empty() {
                    ui.label(egui::RichText::new(note).monospace().small());
                }
            }
            if !self.picks.is_empty() && ui.button("Clear picks").clicked() {
                self.picks.clear();
                self.pick_notes.clear();
                config.picked_data = None;
            }
        });

        section(
            ui,
            "Annotations",
            self.kind == SceneKind::Annotations,
            |ui| {
                ui.checkbox(&mut config.annotations.visible, "Visible");
                ui.label(format!("{} items", config.annotations.items.len()));
                let (x0, x1) = (config.bottom_x.min, config.bottom_x.max);
                let (y0, y1) = (config.left_y.min, config.left_y.max);
                let (cx, cy) = (0.5 * (x0 + x1), 0.5 * (y0 + y1));
                let accent = Color::from_rgb8(200, 40, 120);
                let mut add = |kind| {
                    config.annotations.items.push(Annotation::new(kind));
                    config.set_active_layer(LayerRef::Annotations);
                };
                ui.horizontal_wrapped(|ui| {
                    if ui.button("+ V line").clicked() {
                        add(AnnotationKind::ReferenceLine {
                            axis: AnnotationAxis::X,
                            value: cx,
                            style: AnnotationLineStyle {
                                color: accent,
                                width: 1.5,
                                style: renderer::line::LineStylePreset::Solid,
                            },
                            label: Some(RichText::plain("x ref", accent, 13.0, "")),
                        });
                    }
                    if ui.button("+ H line").clicked() {
                        add(AnnotationKind::ReferenceLine {
                            axis: AnnotationAxis::Y,
                            value: cy,
                            style: AnnotationLineStyle {
                                color: accent,
                                width: 1.5,
                                style: renderer::line::LineStylePreset::Dash,
                            },
                            label: None,
                        });
                    }
                    if ui.button("+ Span").clicked() {
                        add(AnnotationKind::Span {
                            axis: AnnotationAxis::X,
                            from: x0 + 0.4 * (x1 - x0),
                            to: x0 + 0.6 * (x1 - x0),
                            fill: Color::from_rgba(0.3, 0.6, 1.0, 0.25),
                        });
                    }
                    if ui.button("+ Text").clicked() {
                        add(AnnotationKind::Text {
                            anchor: Anchor::Data { x: cx, y: cy },
                            text: RichText::plain("note", Color::BLACK, 15.0, ""),
                            box_style: Some(Default::default()),
                        });
                    }
                    if ui.button("+ Arrow").clicked() {
                        add(AnnotationKind::Arrow {
                            from: Anchor::Ratio { x: 0.3, y: 0.3 },
                            to: Anchor::Data { x: cx, y: cy },
                            style: ArrowStyle::default(),
                            text: Some(RichText::plain("here", Color::BLACK, 13.0, "")),
                        });
                    }
                });
                if let Some(i) = self.selected_annotation
                    && i < config.annotations.items.len()
                {
                    let item = &mut config.annotations.items[i];
                    ui.label(format!("Selected annotation {i}"));
                    ui.checkbox(&mut item.visible, "item visible");
                    match &mut item.kind {
                        AnnotationKind::Text { text, .. } => {
                            rich_text(ui, "text", text);
                        }
                        AnnotationKind::ReferenceLine { style, .. } => {
                            color(ui, "colour", &mut style.color);
                            slider(ui, "width", &mut style.width, 0.5..=6.0);
                        }
                        AnnotationKind::Span { fill, .. } => {
                            color(ui, "fill", fill);
                        }
                        AnnotationKind::Arrow { style, .. } => {
                            color(ui, "colour", &mut style.line.color);
                            slider(ui, "head length", &mut style.head_length, 2.0..=30.0);
                        }
                    }
                    if ui.button("Delete annotation").clicked() {
                        config.annotations.items.remove(i);
                        self.selected_annotation = None;
                    }
                }
            },
        );

        section(ui, "Export", false, |ui| {
            slider(ui, "scale", &mut self.export_scale, 1.0..=4.0);
            if ui.button("Export document PNG").clicked() {
                let chart = Chart::new(config.clone());
                match r.export_document_png_bytes(&chart, &series, self.export_scale) {
                    Ok(bytes) => {
                        let dir = std::path::Path::new("target/playground");
                        let path = dir.join(format!("{:?}.png", self.kind).to_lowercase());
                        let result = std::fs::create_dir_all(dir)
                            .and_then(|_| std::fs::write(&path, &bytes));
                        *host.status = match result {
                            Ok(()) => {
                                format!("Saved {} ({} KiB)", path.display(), bytes.len() / 1024)
                            }
                            Err(e) => format!("Write failed: {e}"),
                        };
                    }
                    Err(e) => *host.status = format!("Export failed: {e}"),
                }
            }
        });

        section(ui, "Diagnostics", false, |ui| {
            let usage = r.gpu_memory_usage();
            ui.label(
                egui::RichText::new(format!("{usage:#?}"))
                    .monospace()
                    .small(),
            );
            for w in &self.warnings {
                ui.colored_label(egui::Color32::from_rgb(170, 110, 0), w.to_string());
            }
        });
        if let Some(e) = &self.error {
            ui.colored_label(egui::Color32::RED, e);
        }

        if series != series_before {
            if config != before {
                self.commit(r, config);
            }
            self.commit_series(r, series);
        } else if config != before {
            self.commit(r, config);
        }
        if let Some(action) = big_action {
            self.big_data_action(r, action);
            ui.ctx().request_repaint();
        }
    }

    fn big_data_action(&mut self, r: &mut Renderer, action: big_data::Action) {
        let Some(big) = self.big.as_mut() else {
            return;
        };
        match action {
            big_data::Action::Upload => {
                self.styles_scale = None;
                self.selected = None;
                self.picks.clear();
                self.pick_notes.clear();
                let current = self.config(r);
                let Some(big) = self.big.as_mut() else {
                    return;
                };
                let (config, series) = big.upload(r, current);
                if let Err(e) = r.set_chart_series(self.id, series) {
                    self.error = Some(format!("series rejected: {e}"));
                } else {
                    self.commit(r, config);
                }
            }
            big_data::Action::Mode(mode) => {
                big.mode = mode;
                self.commit_series(r, big_data::series(mode));
            }
            big_data::Action::Bench => {
                let config = self.config(r);
                if let Some(big) = self.big.as_mut() {
                    big.start_bench(&config);
                }
            }
        }
    }
}

/// The large-data panel: size, mode, memory and timings. Actions are returned
/// and applied after the panel's other edits commit.
fn big_data_controls(
    ui: &mut egui::Ui,
    big: &mut BigData,
    r: &Renderer,
    host: &HostCtx<'_>,
) -> Option<big_data::Action> {
    const MIB: f64 = 1024.0 * 1024.0;
    let mut action = None;
    section(ui, "Large data (resident)", true, |ui| {
        ui.add(
            egui::Slider::new(
                &mut big.pending,
                big_data::MIN_POINTS..=big_data::MAX_POINTS,
            )
            .logarithmic(true)
            .text("points"),
        );
        // Two hi/lo columns, 8 bytes per value each.
        let bytes = big.pending as f64 * 16.0;
        ui.label(format!(
            "{:.0} MiB in the GPU pool (X and Y, hi/lo pairs)",
            bytes / MIB
        ));
        // The pool is one buffer bound whole as storage, so the smaller of the
        // two device limits caps it; the other scenes' columns share it.
        let limits = host.device.limits();
        let ceiling = limits
            .max_buffer_size
            .min(limits.max_storage_buffer_binding_size);
        let others = r.pool().used_bytes().saturating_sub(big.points as u64 * 16);
        let max_points = ceiling.saturating_sub(others) / 16;
        if big.pending as u64 > max_points {
            ui.colored_label(
                egui::Color32::from_rgb(170, 110, 0),
                format!(
                    "Over this device's pool ceiling ({:.0} MiB): about {} points fit",
                    ceiling as f64 / MIB,
                    big_data::group_digits(max_points as usize)
                ),
            );
        }
        let busy = big.benchmarking().is_some();
        if ui
            .add_enabled(
                !busy,
                egui::Button::new(format!(
                    "Upload {} points",
                    big_data::group_digits(big.pending)
                )),
            )
            .clicked()
        {
            action = Some(big_data::Action::Upload);
        }
        let upload = big
            .upload_time
            .map_or(String::new(), |t| format!(" in {:.2} s", t.as_secs_f64()));
        ui.label(format!(
            "Uploaded: {} points{upload}",
            big_data::group_digits(big.points)
        ));
        if let Some(e) = &big.upload_error {
            ui.colored_label(egui::Color32::RED, e);
        }
        ui.horizontal(|ui| {
            for mode in big_data::Mode::ALL {
                if ui
                    .add_enabled(
                        !busy,
                        egui::RadioButton::new(big.mode == mode, mode.label()),
                    )
                    .clicked()
                    && big.mode != mode
                {
                    action = Some(big_data::Action::Mode(mode));
                }
            }
        });
        ui.label(
            egui::RichText::new(format!(
                "Pool {:.0} / {:.0} MiB · device buffer limit {:.0} MiB, storage binding {:.0} MiB",
                r.pool().used_bytes() as f64 / MIB,
                r.pool().capacity() as f64 / MIB,
                limits.max_buffer_size as f64 / MIB,
                limits.max_storage_buffer_binding_size as f64 / MIB,
            ))
            .small(),
        );
        ui.separator();
        ui.checkbox(&mut big.time_frames, "Time every frame (waits for the GPU)");
        if let Some(t) = big.last {
            let total = t.total.as_secs_f64() * 1000.0;
            ui.label(
                egui::RichText::new(format!(
                    "Last frame {total:.2} ms ({}) · prepare {:.2} ms",
                    big_data::fps(total),
                    t.prepare.as_secs_f64() * 1000.0
                ))
                .monospace(),
            );
        }
        match big.benchmarking() {
            Some(frame) => {
                ui.label(format!(
                    "Panning… frame {frame} / {}",
                    big_data::BENCH_FRAMES
                ));
            }
            None => {
                if ui
                    .button(format!("Pan benchmark ({} frames)", big_data::BENCH_FRAMES))
                    .on_hover_text(
                        "Moves the X range every frame, as a drag does, and waits for the \
                         GPU after each frame",
                    )
                    .clicked()
                {
                    action = Some(big_data::Action::Bench);
                }
            }
        }
        if let Some(rep) = big.report {
            ui.label(
                egui::RichText::new(format!(
                    "{} points, {}:\nmean {:.2} ms ({}) · median {:.2}\np95 {:.2} · max {:.2} ms",
                    big_data::group_digits(rep.points),
                    rep.mode.label(),
                    rep.mean,
                    big_data::fps(rep.mean),
                    rep.median,
                    rep.p95,
                    rep.max,
                ))
                .monospace(),
            );
        }
    });
    action
}

fn to_space(axis: &AxisOptions, v: f64) -> f64 {
    match axis.scale {
        AxisScale::Logarithmic => v.max(f64::MIN_POSITIVE).log10(),
        AxisScale::Linear => v,
    }
}

fn from_space(axis: &AxisOptions, v: f64) -> f64 {
    match axis.scale {
        AxisScale::Logarithmic => 10f64.powf(v),
        AxisScale::Linear => v,
    }
}

fn set_log(config: &mut Config, x: bool, log: bool) {
    let scale = if log {
        AxisScale::Logarithmic
    } else {
        AxisScale::Linear
    };
    let (min, max) = {
        let axis = if x { &config.bottom_x } else { &config.left_y };
        (axis.min, axis.max)
    };
    let (min, max) = if log && min <= 0.0 {
        let max = if max > 0.0 { max } else { 1.0 };
        (max / 1000.0, max)
    } else {
        (min, max)
    };
    let mut chart = Chart::new(config.clone());
    chart.with_axis_range_change(|c| {
        let pair = if x {
            [&mut c.bottom_x, &mut c.top_x]
        } else {
            [&mut c.left_y, &mut c.right_y]
        };
        for axis in pair {
            axis.scale = scale.clone();
        }
    });
    if x {
        chart.set_x_range(min, max);
    } else {
        chart.set_y_range(min, max);
    }
    *config = chart.config().clone();
}

/// Restore axis ranges and tick settings, keep every other edit.
fn reset_ranges(config: &mut Config, initial: &Config) {
    fn copy(dst: &mut AxisOptions, src: &AxisOptions) {
        dst.min = src.min;
        dst.max = src.max;
        dst.scale = src.scale.clone();
        dst.major_spacing = src.major_spacing;
        dst.minor_count = src.minor_count;
    }
    copy(&mut config.bottom_x, &initial.bottom_x);
    copy(&mut config.top_x, &initial.top_x);
    copy(&mut config.left_y, &initial.left_y);
    copy(&mut config.right_y, &initial.right_y);
    for (o, i) in config.overlays.iter_mut().zip(&initial.overlays) {
        copy(&mut o.y_axis, &i.y_axis);
    }
    for (s, i) in config.subplots.iter_mut().zip(&initial.subplots) {
        copy(&mut s.config.bottom_x, &i.config.bottom_x);
        copy(&mut s.config.top_x, &i.config.top_x);
        copy(&mut s.config.left_y, &i.config.left_y);
        copy(&mut s.config.right_y, &i.config.right_y);
        for (o, i) in s.config.overlays.iter_mut().zip(&i.config.overlays) {
            copy(&mut o.y_axis, &i.y_axis);
        }
    }
}

fn draw_style_index(style: &DrawStyle) -> usize {
    match style {
        DrawStyle::Precise => 0,
        DrawStyle::Sketch(_) => 1,
        DrawStyle::Milkyway(_) => 2,
        DrawStyle::Constellation(_) => 3,
    }
}

fn render_type_name(t: &DataRenderType) -> &'static str {
    match t {
        DataRenderType::Scatter { .. } => "scatter",
        DataRenderType::Line { .. } => "line",
        DataRenderType::ScatterLine { .. } => "scatter+line",
        DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. } => "scatter+errors",
        DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. } => "line+scatter+errors",
        DataRenderType::Histogram { .. } => "histogram",
        DataRenderType::Heatmap { .. } => "heatmap",
        DataRenderType::Contour { .. } => "contour",
        DataRenderType::HeatmapContour { .. } => "heatmap+contour",
        DataRenderType::Vector { .. } => "vector",
    }
}

fn line_controls(ui: &mut egui::Ui, line: &mut renderer::DataLineStyleConfig) {
    color(ui, "line colour", &mut line.line_color);
    slider(ui, "line width", &mut line.line_width, 0.5..=10.0);
    choice(ui, "dash", &mut line.line_style, widgets::LINE_STYLES);
    choice(ui, "interpolation", &mut line.interpolation, INTERPOLATIONS);
    let mut filled = line.fill.is_some();
    if ui.checkbox(&mut filled, "fill").changed() {
        line.fill = filled.then_some(LineFill::ToBaseline(0.0));
    }
    if let Some(LineFill::ToBaseline(b)) = &mut line.fill {
        ui.add(egui::DragValue::new(b).speed(0.05).prefix("baseline "));
    }
}

/// A combo box over "none" and the chart's columns. `Some` when the user
/// picked an entry.
fn column_choice(
    ui: &mut egui::Ui,
    label: &str,
    current: Option<&str>,
    columns: &[String],
) -> Option<Option<String>> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .selected_text(current.unwrap_or("none"))
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), "none").clicked() {
                    picked = Some(None);
                }
                for c in columns {
                    if ui
                        .selectable_label(current == Some(c.as_str()), c)
                        .clicked()
                    {
                        picked = Some(Some(c.clone()));
                    }
                }
            });
    });
    picked
}

fn scatter_controls(
    ui: &mut egui::Ui,
    s: &mut renderer::DataScatterStyleConfig,
    columns: &[String],
    r: &Renderer,
) {
    color(ui, "marker colour", &mut s.point_color);
    slider(ui, "marker size", &mut s.point_size, 1.0..=24.0);
    choice(ui, "shape", &mut s.point_shape, SHAPES);

    // Value maps: the colour map needs the chart's colourbar (the registry
    // rejects it otherwise); the size map carries its own domain.
    let current = s.point_color_map.as_ref().map(|m| m.column.as_str());
    if let Some(pick) = column_choice(ui, "colour map column", current, columns) {
        s.point_color_map = pick.map(|column| PointColorMap { column });
    }
    let current = s.point_size_map.as_ref().map(|m| m.column.as_str());
    if let Some(pick) = column_choice(ui, "size map column", current, columns) {
        s.point_size_map = pick.map(|column| match s.point_size_map.take() {
            Some(mut map) => {
                map.column = column;
                map
            }
            None => PointSizeMap {
                column,
                domain_min: 0.0,
                domain_max: 1.0,
                min_px: 2.0,
                max_px: 16.0,
                scale: SizeScale::Area,
            },
        });
    }
    if let Some(map) = s.point_size_map.as_mut() {
        ui.horizontal(|ui| {
            ui.label("domain");
            ui.add(egui::DragValue::new(&mut map.domain_min).speed(0.05));
            ui.add(egui::DragValue::new(&mut map.domain_max).speed(0.05));
            if ui
                .button("fit")
                .on_hover_text("Domain from the column's resident statistics")
                .clicked()
                && let Ok(ext) = Chart::slot_extent(r.pool(), &map.column)
            {
                map.fit_domain(ext.min, ext.max);
            }
        });
        slider(ui, "min px", &mut map.min_px, 0.0..=12.0);
        slider(ui, "max px", &mut map.max_px, 1.0..=40.0);
        choice(ui, "size scale", &mut map.scale, SIZE_SCALES);
    }
}

/// The scatter style of a render type that draws markers.
fn scatter_of(t: &DataRenderType) -> Option<&renderer::DataScatterStyleConfig> {
    match t {
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterLine { scatter, .. }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. }
        | DataRenderType::LineScatterErrorbarX { scatter, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, .. } => Some(scatter),
        _ => None,
    }
}

/// The columns a series names: x, y and any value-map columns.
fn series_columns(s: &SeriesConfig) -> Vec<String> {
    let mut out = vec![s.x_column.clone(), s.y_column.clone()];
    if let Some(scatter) = scatter_of(&s.render_type) {
        out.extend(scatter.point_color_map.iter().map(|m| m.column.clone()));
        out.extend(scatter.point_size_map.iter().map(|m| m.column.clone()));
    }
    if let Some((u, v, color)) = vector_point_columns(&s.render_type) {
        out.push(u.clone());
        out.push(v.clone());
        out.extend(color.cloned());
    }
    out
}

/// The component and colour columns of a point vector series.
fn vector_point_columns(t: &DataRenderType) -> Option<(&String, &String, Option<&String>)> {
    let DataRenderType::Vector {
        components: VectorComponents::Points { u, v },
        style,
    } = t
    else {
        return None;
    };
    let color = match style.color_map.as_ref() {
        Some(VectorColorMap::Column(column)) => Some(column),
        _ => None,
    };
    Some((u, v, color))
}

/// Read `samples` (column, index) straight from the GPU column pool, the
/// same values the draw used. The CPU keeps no copy of column data.
fn read_samples(r: &Renderer, samples: &[(String, usize)]) -> Result<Vec<f64>, String> {
    let mut offsets = Vec::with_capacity(samples.len());
    for (column, index) in samples {
        let handle = r
            .pool()
            .handle_for(column)
            .ok_or_else(|| format!("column {column} is not resident"))?;
        if *index >= handle.len_values {
            return Err(format!("index {index} is outside {column}"));
        }
        offsets.push(handle.offset + *index as u64 * 8);
    }
    let buffer = r.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("playground picked values"),
        size: (offsets.len().max(1) * 8) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = r.device().create_command_encoder(&Default::default());
    for (i, offset) in offsets.iter().enumerate() {
        encoder.copy_buffer_to_buffer(r.pool().buffer(), *offset, &buffer, i as u64 * 8, 8);
    }
    r.queue().submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    r.device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| e.to_string())?;
    rx.recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let mapped = slice.get_mapped_range().map_err(|e| e.to_string())?;
    let values = mapped
        .as_chunks::<8>()
        .0
        .iter()
        .take(offsets.len())
        .map(|pair| {
            f64::from(f32::from_le_bytes(pair[..4].try_into().unwrap()))
                + f64::from(f32::from_le_bytes(pair[4..].try_into().unwrap()))
        })
        .collect();
    drop(mapped);
    buffer.unmap();
    Ok(values)
}

/// The picked point's coordinates and, when its series maps colour or size,
/// the values those came from.
fn pick_values(r: &Renderer, series: &[SeriesConfig], p: &PickedData) -> Result<String, String> {
    let PickedData::Point {
        series_id,
        point_index,
        ..
    } = p
    else {
        return Ok(String::new());
    };
    let series = series
        .iter()
        .find(|s| &s.series_id == series_id)
        .ok_or_else(|| format!("series {series_id} no longer exists"))?;
    let scatter = scatter_of(&series.render_type);
    let color_column = scatter.and_then(|s| s.point_color_map.as_ref().map(|m| &m.column));
    let size_column = scatter.and_then(|s| s.point_size_map.as_ref().map(|m| &m.column));
    let mut samples = vec![
        (series.x_column.clone(), *point_index),
        (series.y_column.clone(), *point_index),
    ];
    samples.extend(color_column.map(|c| (c.clone(), *point_index)));
    samples.extend(size_column.map(|c| (c.clone(), *point_index)));
    let vector = vector_point_columns(&series.render_type);
    if let Some((u, v, color)) = vector {
        samples.push((u.clone(), *point_index));
        samples.push((v.clone(), *point_index));
        samples.extend(color.map(|c| (c.clone(), *point_index)));
    }
    let values = read_samples(r, &samples)?;
    let mut note = format!("x = {:.4}, y = {:.4}", values[0], values[1]);
    let mut next = 2;
    if color_column.is_some() {
        note.push_str(&format!(", colour value = {:.4}", values[next]));
        next += 1;
    }
    if size_column.is_some() {
        let v = values[next];
        if v.is_finite() {
            note.push_str(&format!(", size value = {v:.4}"));
        } else {
            note.push_str(", size value = NaN (hidden)");
        }
        next += 1;
    }
    if let Some((_, _, color)) = vector {
        note.push_str(&format!(
            ", u = {:.4}, v = {:.4}",
            values[next],
            values[next + 1]
        ));
        if color.is_some() {
            note.push_str(&format!(", colour value = {:.4}", values[next + 2]));
        }
    }
    Ok(note)
}

fn error_controls(ui: &mut egui::Ui, e: &mut renderer::DataErrorBarStyleConfig) {
    color(ui, "error colour", &mut e.error_bar_color);
    slider(ui, "error width", &mut e.error_bar_width, 0.5..=6.0);
    slider(ui, "cap size", &mut e.error_bar_cap_size, 0.0..=20.0);
}

fn series_controls(
    ui: &mut egui::Ui,
    s: &mut SeriesConfig,
    config: &mut Config,
    columns: &[String],
    r: &Renderer,
) {
    match &mut s.render_type {
        DataRenderType::Line { line } => line_controls(ui, line),
        DataRenderType::Scatter { scatter } => scatter_controls(ui, scatter, columns, r),
        DataRenderType::ScatterLine { scatter, line } => {
            scatter_controls(ui, scatter, columns, r);
            line_controls(ui, line);
        }
        DataRenderType::ScatterErrorbarX {
            scatter, err_style, ..
        }
        | DataRenderType::ScatterErrorbarY {
            scatter, err_style, ..
        }
        | DataRenderType::ScatterErrorbarXY {
            scatter, err_style, ..
        } => {
            scatter_controls(ui, scatter, columns, r);
            error_controls(ui, err_style);
        }
        DataRenderType::LineScatterErrorbarX {
            scatter,
            line,
            err_style,
            ..
        }
        | DataRenderType::LineScatterErrorbarY {
            scatter,
            line,
            err_style,
            ..
        }
        | DataRenderType::LineScatterErrorbarXY {
            scatter,
            line,
            err_style,
            ..
        } => {
            scatter_controls(ui, scatter, columns, r);
            line_controls(ui, line);
            error_controls(ui, err_style);
        }
        DataRenderType::Histogram { bar } => {
            color(ui, "fill", &mut bar.fill_color);
            color(ui, "border", &mut bar.border_color);
            slider(ui, "border width", &mut bar.border_width, 0.0..=4.0);
            slider(ui, "width ratio", &mut bar.width_ratio, 0.1..=1.0);
            slider(ui, "gap px", &mut bar.gap_px, 0.0..=10.0);
            let mut horizontal = bar.orientation == BarOrientation::Horizontal;
            if ui.checkbox(&mut horizontal, "horizontal").changed() {
                // Orientation alone decides which column holds the edges, so
                // flipping it swaps the columns and the two axis ranges.
                bar.orientation = if horizontal {
                    BarOrientation::Horizontal
                } else {
                    BarOrientation::Vertical
                };
                std::mem::swap(&mut s.x_column, &mut s.y_column);
                let (x, y) = (
                    (config.bottom_x.min, config.bottom_x.max),
                    (config.left_y.min, config.left_y.max),
                );
                let mut chart = Chart::new(config.clone());
                chart.set_x_range(y.0, y.1);
                chart.set_y_range(x.0, x.1);
                *config = chart.config().clone();
            }
        }
        DataRenderType::Heatmap { fill, .. } => {
            slider(ui, "opacity", &mut fill.opacity, 0.0..=1.0);
            choice(
                ui,
                "shading",
                &mut fill.shading,
                &[Shading::Interpolated, Shading::Flat],
            );
        }
        DataRenderType::Contour { contour, .. } => contour_controls(ui, contour, config),
        DataRenderType::HeatmapContour { fill, contour, .. } => {
            slider(ui, "opacity", &mut fill.opacity, 0.0..=1.0);
            choice(
                ui,
                "shading",
                &mut fill.shading,
                &[Shading::Interpolated, Shading::Flat],
            );
            contour_controls(ui, contour, config);
        }
        DataRenderType::Vector { components, style } => {
            vector_controls(ui, components, style, columns);
        }
    }
}

fn vector_controls(
    ui: &mut egui::Ui,
    components: &VectorComponents,
    style: &mut VectorStyle,
    columns: &[String],
) {
    color(ui, "arrow colour", &mut style.color);
    slider(ui, "shaft width", &mut style.width_px, 0.5..=8.0);
    slider(ui, "head length", &mut style.head_length_px, 0.0..=24.0);
    slider(ui, "head width", &mut style.head_width_px, 0.0..=24.0);
    slider(ui, "min length", &mut style.min_length_px, 0.0..=20.0);
    choice(
        ui,
        "pivot",
        &mut style.pivot,
        &[VectorPivot::Tail, VectorPivot::Middle, VectorPivot::Tip],
    );
    // Scale: the mode and its factor. Switching modes keeps a usable
    // magnitude rather than the other mode's number.
    let mut mode = match style.scale {
        VectorScale::Pixels { .. } => "Pixels",
        VectorScale::Data { .. } => "Data",
    };
    if choice(ui, "scale", &mut mode, &["Pixels", "Data"]) {
        style.scale = match mode {
            "Data" => VectorScale::Data { factor: 0.3 },
            _ => VectorScale::Pixels { px_per_unit: 20.0 },
        };
    }
    match &mut style.scale {
        VectorScale::Pixels { px_per_unit } => {
            let mut value = *px_per_unit as f32;
            if slider(ui, "px per unit", &mut value, 1.0..=120.0) {
                *px_per_unit = value as f64;
            }
        }
        VectorScale::Data { factor } => {
            let mut value = *factor as f32;
            if slider(ui, "data factor", &mut value, 0.02..=2.0) {
                *factor = value as f64;
            }
        }
    }
    match components {
        VectorComponents::Points { .. } => {
            let current = match style.color_map.as_ref() {
                Some(VectorColorMap::Column(column)) => Some(column.as_str()),
                _ => None,
            };
            if let Some(pick) = column_choice(ui, "colour map column", current, columns) {
                style.color_map = pick.map(VectorColorMap::Column);
            }
        }
        VectorComponents::Grid { .. } => {
            let mut mapped = matches!(style.color_map, Some(VectorColorMap::Grid(_)));
            ui.add_enabled(
                mapped,
                egui::Checkbox::new(&mut mapped, "colour map grid (set by the scene)"),
            );
        }
    }
}

fn contour_controls(
    ui: &mut egui::Ui,
    c: &mut renderer::data_config::ContourConfig,
    config: &Config,
) {
    color(ui, "isoline colour", &mut c.line.line_color);
    slider(ui, "isoline width", &mut c.line.line_width, 0.5..=5.0);
    let mut n = c.levels.len();
    if ui
        .add(egui::Slider::new(&mut n, 1..=12).text("levels"))
        .changed()
    {
        let (lo, hi) = config
            .colorbar
            .as_ref()
            .map_or((0.0, 1.0), |b| (b.axis.min, b.axis.max));
        c.levels = (1..=n)
            .map(|i| lo + (hi - lo) * i as f64 / (n + 1) as f64)
            .collect();
    }
    if let Some(labels) = c.labels.as_mut() {
        ui.checkbox(&mut labels.visible, "labels");
        slider(ui, "label spacing px", &mut labels.spacing_px, 40.0..=400.0);
        slider(ui, "label font px", &mut labels.font_size, 6.0..=24.0);
        ui.add(egui::Slider::new(&mut labels.significant_digits, 1..=4).text("label digits"));
        let mut boxed = labels.bg_color.is_some();
        if ui.checkbox(&mut boxed, "label background box").changed() {
            labels.bg_color = boxed.then(|| Color::from_rgb8(250, 250, 250));
        }
        if labels.bg_color.is_some() {
            slider(ui, "box padding px", &mut labels.bg_padding_px, 0.0..=8.0);
        }
    }
    let mut zero = c.levels.contains(&0.0);
    if ui
        .checkbox(&mut zero, "include level 0 (flat region)")
        .changed()
    {
        if zero {
            c.levels.insert(0, 0.0);
        } else {
            c.levels.retain(|l| *l != 0.0);
        }
    }
}

fn describe_pick(p: &PickedData) -> String {
    match p {
        PickedData::Point {
            series_id,
            point_index,
            distance_px,
            ..
        } => {
            format!("{series_id} point #{point_index} ({distance_px:.1} px)")
        }
        other => format!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Layers: overlay layers (extra Y axis on the same plot) and subplots (whole
// charts of their own, optionally axis-linked). Pure Config edits.
// ---------------------------------------------------------------------------

fn axis_color(axis: &mut AxisOptions, c: Color) {
    axis.line_color = c;
    axis.label_style.color = c;
    axis.title_option.text.color = c;
}

fn layer_columns<'a>(
    config: &Config,
    series: &'a [SeriesConfig],
    layer: LayerRef,
) -> (Vec<&'a str>, Vec<&'a str>) {
    let ids: Vec<&SeriesConfig> = series
        .iter()
        .filter(|s| config.layer_of_series(&s.series_id) == layer)
        .filter(|s| {
            !matches!(
                s.render_type,
                DataRenderType::Heatmap { .. }
                    | DataRenderType::Contour { .. }
                    | DataRenderType::HeatmapContour { .. }
            )
        })
        .collect();
    (
        ids.iter().map(|s| s.x_column.as_str()).collect(),
        ids.iter().map(|s| s.y_column.as_str()).collect(),
    )
}

/// Padding ratio of both fit buttons, so "fit" and "fit group" differ only in
/// which series take part.
const PADDING: f64 = 0.05;

/// Fit the axes a layer owns to the series it draws. No-op for an empty layer.
fn fit_layer(config: &mut Config, series: &[SeriesConfig], r: &Renderer, layer: LayerRef) {
    let (xs, ys) = layer_columns(config, series, layer);
    if ys.is_empty() {
        return;
    }
    let pool = r.pool();
    let mut chart = Chart::new(config.clone());
    let result = match layer {
        LayerRef::Overlay(i) => chart.auto_fit_overlay_y_union(pool, i, &ys, PADDING),
        LayerRef::Subplot(j) => chart
            .auto_fit_subplot_x_union(pool, j, &xs, PADDING)
            .and_then(|_| chart.auto_fit_subplot_y_union(pool, j, &ys, PADDING)),
        LayerRef::SubplotOverlay(j, k) => {
            // No document-level helper: fit the subplot's own chart, copy the axis back.
            let Some(sub) = config.subplots.get(j) else {
                return;
            };
            let mut inner = Chart::new(sub.config.clone());
            let fitted = inner.auto_fit_overlay_y_union(pool, k, &ys, PADDING);
            if fitted.is_ok() {
                let axis = inner.config().overlays[k].y_axis.clone();
                chart.with_axis_range_change(|c| c.subplots[j].config.overlays[k].y_axis = axis);
            }
            fitted
        }
        _ => chart
            .auto_fit_x_union(pool, &xs, PADDING)
            .and_then(|_| chart.auto_fit_y_union(pool, &ys, PADDING)),
    };
    if result.is_ok() {
        *config = chart.config().clone();
    }
}

/// Fit a layer's axes to every series of its link groups, so a linked axis
/// keeps the data of every chart that shares it.
fn fit_link_group(config: &mut Config, series: &[SeriesConfig], r: &Renderer, layer: LayerRef) {
    let mut chart = Chart::new(config.clone());
    if chart
        .auto_fit_layer_union(r.pool(), layer, series, PADDING)
        .is_ok()
    {
        *config = chart.config().clone();
    }
}

fn new_overlay(n: usize) -> OverlayLayer {
    let c = ColorCycle::Vivid.color(n + 3);
    let mut o = OverlayLayer::default();
    o.y_axis.out_margin = 120.0;
    o.y_axis.title_option.visible = true;
    o.y_axis.title_option.text = RichText::plain(&format!("overlay {n}"), c, 16.0, "");
    axis_color(&mut o.y_axis, c);
    o
}

/// Lay the parent and every subplot out on a grid filling the document.
fn retile(config: &mut Config) {
    let n = 1 + config.subplots.len() as u32;
    let cols = (n as f64).sqrt().ceil() as u32;
    let rows = n.div_ceil(cols);
    let (w, h) = (scenes::DOC_W / cols, scenes::DOC_H / rows);
    let cell = |i: u32| {
        ChartArea(Rect {
            x: (i % cols) * w,
            y: (i / cols) * h,
            width: w,
            height: h,
        })
    };
    config.chart_area = cell(0);
    for (i, s) in config.subplots.iter_mut().enumerate() {
        s.config.chart_area = cell(i as u32 + 1);
    }
}

fn new_subplot(parent: &Config, n: usize) -> Subplot {
    let mut c = parent.clone();
    c.subplots.clear();
    c.overlays.clear();
    c.annotations = Default::default();
    c.picked_data = None;
    c.picked_points = None;
    c.legend.visible = false;
    c.chart_title.text.segments = rich_segments_from_text(&format!("Subplot {n}"));
    c.set_active_layer(LayerRef::Base);
    Subplot {
        config: c,
        series: Vec::new(),
        link_x: None,
        link_y: None,
    }
}

/// The series joined to `id` by a `LineFill::ToSeries` band, in either
/// direction. Both ends of a band must live in the same chart.
fn fill_partners(series: &[SeriesConfig], id: &str) -> Vec<String> {
    let fills_to = |s: &SeriesConfig| match &s.render_type {
        DataRenderType::Line { line } | DataRenderType::ScatterLine { line, .. } => {
            match &line.fill {
                Some(LineFill::ToSeries(other)) => Some(other.clone()),
                _ => None,
            }
        }
        _ => None,
    };
    let mut out: Vec<String> = series
        .iter()
        .filter(|s| s.series_id == id)
        .filter_map(fills_to)
        .collect();
    out.extend(
        series
            .iter()
            .filter(|s| fills_to(s).as_deref() == Some(id))
            .map(|s| s.series_id.clone()),
    );
    out
}

/// Move a series (and its fill partners) to `target`, dropping them from every
/// other layer list.
fn place_with_partners(
    config: &mut Config,
    series: &[SeriesConfig],
    id: &str,
    target: LayerRef,
) -> Vec<String> {
    let partners = fill_partners(series, id);
    place_series(config, id, target);
    for p in &partners {
        place_series(config, p, target);
    }
    partners
}

/// Move one series to `target`, dropping it from every other layer list.
fn place_series(config: &mut Config, id: &str, target: LayerRef) {
    let drop = |v: &mut Vec<String>| v.retain(|s| s != id);
    config.overlays.iter_mut().for_each(|o| drop(&mut o.series));
    for s in &mut config.subplots {
        drop(&mut s.series);
        s.config
            .overlays
            .iter_mut()
            .for_each(|o| drop(&mut o.series));
    }
    match target {
        LayerRef::Overlay(i) => config.overlays[i].series.push(id.into()),
        LayerRef::Subplot(j) => config.subplots[j].series.push(id.into()),
        LayerRef::SubplotOverlay(j, k) => {
            // A subplot lists every series it draws, its overlays' included.
            config.subplots[j].series.push(id.into());
            config.subplots[j].config.overlays[k].series.push(id.into());
        }
        _ => {}
    }
}

fn all_layers(config: &Config) -> Vec<LayerRef> {
    let mut v = vec![LayerRef::Base];
    v.extend((0..config.overlays.len()).map(LayerRef::Overlay));
    for (j, s) in config.subplots.iter().enumerate() {
        v.push(LayerRef::Subplot(j));
        v.extend((0..s.config.overlays.len()).map(|k| LayerRef::SubplotOverlay(j, k)));
    }
    v
}

fn legend_template() -> renderer::legend::Legend {
    let mut l = renderer::default::default_config().legend;
    l.visible = true;
    l.corner = LegendCorner::BottomRight;
    l
}

/// Returns a status line when the layer structure changed.
fn layers_controls(
    ui: &mut egui::Ui,
    config: &mut Config,
    series: &[SeriesConfig],
    r: &Renderer,
    initial_right: &AxisOptions,
) -> Option<String> {
    let mut status = None;

    // ---- parent overlays ----
    ui.label(egui::RichText::new("Overlays (extra Y axis on the same plot)").strong());
    let mut remove_overlay = None;
    let mut fit_overlay = None;
    for i in 0..config.overlays.len() {
        ui.push_id(("ov", i), |ui| {
            let o = &mut config.overlays[i];
            ui.horizontal(|ui| {
                ui.label(format!("Overlay {i}: {} series", o.series.len()));
                let mut c = o.y_axis.line_color;
                if color(ui, "", &mut c) {
                    axis_color(&mut o.y_axis, c);
                }
                if ui.small_button("fit Y").clicked() {
                    fit_overlay = Some(i);
                }
                if ui.small_button("remove").clicked() {
                    remove_overlay = Some(i);
                }
            });
            slider(ui, "axis margin", &mut o.y_axis.out_margin, 30.0..=200.0);
            let mut legend = o.legend.is_some();
            if ui.checkbox(&mut legend, "own legend").changed() {
                o.legend = legend.then(legend_template);
            }
        });
    }
    if let Some(i) = fit_overlay {
        fit_layer(config, series, r, LayerRef::Overlay(i));
    }
    if let Some(i) = remove_overlay {
        config.overlays.remove(i);
        if config.overlays.is_empty() {
            config.right_y.line_visible = initial_right.line_visible;
            config.right_y.tick = initial_right.tick.clone();
        }
        config.set_active_layer(LayerRef::Base);
        status = Some(format!(
            "Removed overlay {i}; its series went back to the base layer"
        ));
    }
    if ui.button("+ Add overlay").clicked() {
        let n = config.overlays.len();
        config.overlays.push(new_overlay(n));
        // The base right axis would sit under the overlay axis.
        config.right_y.line_visible = false;
        config.right_y.tick = renderer::config::TickVisibility::None;
        status = Some(format!(
            "Added overlay {n}: move a series into it under “Series placement”"
        ));
    }

    // ---- subplots ----
    ui.separator();
    ui.label(egui::RichText::new("Subplots (separate charts, optional axis links)").strong());
    let mut remove_subplot = None;
    let mut fit_subplot = None;
    for j in 0..config.subplots.len() {
        ui.push_id(("sp", j), |ui| {
            let sub = &mut config.subplots[j];
            ui.horizontal(|ui| {
                ui.label(format!("Subplot {j}: {} series", sub.series.len()));
                if ui
                    .small_button("fit")
                    .on_hover_text("This chart's series only")
                    .clicked()
                {
                    fit_subplot = Some((j, false));
                }
                if ui
                    .small_button("fit group")
                    .on_hover_text("Every series of its link groups (Chart::auto_fit_layer_union)")
                    .clicked()
                {
                    fit_subplot = Some((j, true));
                }
                if ui.small_button("+ overlay").clicked() {
                    let k = sub.config.overlays.len();
                    sub.config.overlays.push(new_overlay(k));
                    sub.config.right_y.line_visible = false;
                    sub.config.right_y.tick = renderer::config::TickVisibility::None;
                    status = Some(format!("Added overlay {k} to subplot {j}"));
                }
                if ui.small_button("remove").clicked() {
                    remove_subplot = Some(j);
                }
            });
            let mut targets: Vec<Option<CellRef>> = vec![None, Some(CellRef::Parent)];
            targets.extend((0..j).map(|t| Some(CellRef::Subplot(t))));
            choice(ui, "link X", &mut sub.link_x, &targets);
            choice(ui, "link Y", &mut sub.link_y, &targets);
            let mut remove_inner = None;
            for k in 0..sub.config.overlays.len() {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "    overlay {k}: {} series",
                        sub.config.overlays[k].series.len()
                    ));
                    if ui.small_button("remove").clicked() {
                        remove_inner = Some(k);
                    }
                });
            }
            if let Some(k) = remove_inner {
                let ids = sub.config.overlays.remove(k).series;
                sub.series.retain(|s| !ids.contains(s));
                status = Some(format!("Removed overlay {k} of subplot {j}"));
            }
        });
    }
    match fit_subplot {
        Some((j, false)) => fit_layer(config, series, r, LayerRef::Subplot(j)),
        Some((j, true)) => fit_link_group(config, series, r, LayerRef::Subplot(j)),
        None => {}
    }
    if let Some(j) = remove_subplot {
        config.subplots.remove(j);
        for s in &mut config.subplots[j..] {
            for link in [&mut s.link_x, &mut s.link_y] {
                *link = match *link {
                    Some(CellRef::Subplot(t)) if t == j => None,
                    Some(CellRef::Subplot(t)) if t > j => Some(CellRef::Subplot(t - 1)),
                    other => other,
                };
            }
        }
        retile(config);
        status = Some(format!(
            "Removed subplot {j}; its series went back to the parent"
        ));
    }
    ui.horizontal(|ui| {
        if ui.button("+ Add subplot").clicked() {
            let n = config.subplots.len();
            let sub = new_subplot(config, n);
            config.subplots.push(sub);
            retile(config);
            status = Some(format!(
                "Added subplot {n}: move a series into it under “Series placement”"
            ));
        }
        if !config.subplots.is_empty() && ui.button("Re-tile").clicked() {
            retile(config);
        }
    });
    if status.is_some() {
        // Layer indices may have shifted; Base is always valid.
        config.set_active_layer(LayerRef::Base);
    }

    // ---- series placement ----
    ui.separator();
    ui.label(egui::RichText::new("Series placement").strong());
    let layers = all_layers(config);
    for s in series {
        let current = config.layer_of_series(&s.series_id);
        let mut target = current;
        ui.push_id(("place", &s.series_id), |ui| {
            choice(ui, &s.series_id, &mut target, &layers);
        });
        if target != current {
            let partners = place_with_partners(config, series, &s.series_id, target);
            fit_layer(config, series, r, target);
            status = Some(if partners.is_empty() {
                format!("{} → {target:?}", s.series_id)
            } else {
                format!(
                    "{} → {target:?} (with fill partner {})",
                    s.series_id,
                    partners.join(", ")
                )
            });
        }
    }
    status
}

/// Headless check of the layer edits (`playground --selftest`): every step is
/// committed through the registry and exported, so a rejected config fails loudly.
pub fn selftest(r: &mut Renderer) -> Result<(), String> {
    let e = |e: renderer::FiggyError| e.to_string();
    let scene = scenes::build(r, SceneKind::Lines, big_data::START_POINTS).map_err(e)?;
    let id = r
        .register_chart(scene.config.clone(), scene.series.clone())
        .map_err(e)?;
    let series = scene.series.clone();
    let dir = std::path::Path::new("target/playground/selftest");
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut step = 0;
    let mut commit = |r: &mut Renderer, name: &str, config: Config| -> Result<(), String> {
        r.set_chart_config(id, config)
            .map_err(|e| format!("{name}: {e}"))?;
        let chart = Chart::new(r.chart_config(id).unwrap().clone());
        let png = r
            .export_document_png_bytes(&chart, &series, 1.0)
            .map_err(|e| format!("{name} export: {e}"))?;
        step += 1;
        let path = dir.join(format!("{step:02}_{name}.png"));
        std::fs::write(&path, png).map_err(|e| e.to_string())?;
        println!("ok  {name} -> {}", path.display());
        Ok(())
    };
    let mut c = r.chart_config(id).unwrap().clone();

    c.overlays.push(new_overlay(0));
    c.right_y.line_visible = false;
    commit(r, "add_overlay", c.clone())?;
    place_series(&mut c, "cos_dashed", LayerRef::Overlay(0));
    fit_layer(&mut c, &series, r, LayerRef::Overlay(0));
    scenes::rebuild_legends(&mut c, &series);
    commit(r, "cos_to_overlay", c.clone())?;

    c.subplots.push(new_subplot(&c, 0));
    retile(&mut c);
    commit(r, "add_subplot", c.clone())?;
    place_series(&mut c, "step_mid", LayerRef::Subplot(0));
    fit_layer(&mut c, &series, r, LayerRef::Subplot(0));
    commit(r, "step_to_subplot", c.clone())?;
    c.subplots[0].link_x = Some(CellRef::Parent);
    commit(r, "link_x", c.clone())?;
    c.subplots[0].config.overlays.push(new_overlay(0));
    let partners = place_with_partners(&mut c, &series, "band_lo", LayerRef::SubplotOverlay(0, 0));
    assert_eq!(partners, ["band_hi"]);
    fit_layer(&mut c, &series, r, LayerRef::SubplotOverlay(0, 0));
    commit(r, "band_lo_to_subplot_overlay", c.clone())?;
    c.subplots.push(new_subplot(&c, 1));
    retile(&mut c);
    place_series(&mut c, "sin_area", LayerRef::Subplot(1));
    fit_layer(&mut c, &series, r, LayerRef::Subplot(1));
    scenes::rebuild_legends(&mut c, &series);
    commit(r, "second_subplot", c.clone())?;

    c.overlays.remove(0);
    commit(r, "remove_overlay", c.clone())?;
    c.subplots.remove(0);
    retile(&mut c);
    commit(r, "remove_first_subplot", c.clone())?;
    c.subplots.clear();
    retile(&mut c);
    commit(r, "remove_all_subplots", c.clone())?;
    r.remove_chart(id).map_err(e)?;

    // The field scene with its label workarounds: no box, no level 0.
    {
        let mut scene = scenes::build(r, SceneKind::Field, big_data::START_POINTS).map_err(e)?;
        if let DataRenderType::HeatmapContour { contour, .. } = &mut scene.series[0].render_type {
            contour.levels.retain(|l| *l != 0.0);
            if let Some(l) = contour.labels.as_mut() {
                l.bg_color = None;
            }
        }
        let png = r
            .export_document_png_bytes(&Chart::new(scene.config), &scene.series, 2.0)
            .map_err(e)?;
        std::fs::write(dir.join("scene_field_clean_2x.png"), png).map_err(|e| e.to_string())?;
        println!("ok  field without box / level 0");
    }

    // Every Cartesian scene as it starts, at 1x and 2x.
    for kind in SceneKind::ALL.into_iter().filter(|k| !k.is_bounded()) {
        let scene = scenes::build(r, kind, big_data::START_POINTS).map_err(e)?;
        let chart = Chart::new(scene.config);
        for scale in [1.0f32, 2.0] {
            let png = r
                .export_document_png_bytes(&chart, &scene.series, scale)
                .map_err(e)?;
            let path = dir.join(format!("scene_{kind:?}_{scale}x.png").to_lowercase());
            std::fs::write(&path, png).map_err(|e| e.to_string())?;
        }
        println!("ok  scene {kind:?}");
    }
    Ok(())
}

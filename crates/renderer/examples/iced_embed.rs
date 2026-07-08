//! figgy + iced 0.14 GPU integration demo — uses `shader::Primitive` to put
//! renderer::Renderer on top of the wgpu 27 device/queue iced provides.
//!
//! Layout:
//!
//! - `FiggyPipeline` (impls `iced_wgpu::primitive::Pipeline`) — built once
//!   the first time iced sees it. Owns renderer::Renderer + 3 panels.
//! - `FiggyPrimitive` (impls `shader::Primitive`) — per-frame instance
//!   carrying which panel to draw + panel rect.
//!
//! Run with:
//! `cargo run -p renderer --example iced_embed --features iced_demo`

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use std::sync::atomic::AtomicU32;

use iced::widget::shader;
use iced::widget::{button, column as ic_column, container, row, text, text_input};
use iced::{Element, Length, Rectangle, Task, Theme};

/// 1-bit export-request signal between UI and `Primitive::prepare`.
/// Set to true on click; prepare swaps it back to false and runs the export.
static EXPORT_REQUESTED: AtomicBool = AtomicBool::new(false);
/// User-entered DPI (integer). 96 = screen 1:1, 192 = 2x, …
static EXPORT_DPI: AtomicU32 = AtomicU32::new(192);
/// Pending click from `shader::Program::update` → consumed by panel 0's
/// `Primitive::prepare`: (panel index, x, y) in the shader-bounds coordinate
/// space — the same space `FiggyPrimitive.panel_rect_px` / chart_area use.
static CLICK_EVENT: Mutex<Option<(usize, f32, f32)>> = Mutex::new(None);
/// Accumulated pointer drag delta (same coordinate space) since the last
/// frame. Applied to the currently selected element by panel 0's `prepare`.
static DRAG_EVENT: Mutex<Option<(f32, f32)>> = Mutex::new(None);
/// Left button released — ends any active resize.
static RELEASE_EVENT: AtomicBool = AtomicBool::new(false);
use iced_wgpu::graphics::Viewport;
use iced_wgpu::wgpu;

use renderer::color::Color;
use renderer::config::{AxisScale, LegendEntryKind};
use renderer::data::Column;
use renderer::default;
use renderer::demo;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, ChartDrawItem, ChartStyle, ChartView, CpuTextMeasure, DataLineStyleConfig,
    DataRenderType, HitId, HitMap, MAX_EXPORT_SCALE, MIN_EXPORT_SCALE, PreparedFrame, Renderer,
    RendererDevice, ResizeHandle, SelectionBox, Series, SeriesConfig, dpi_to_scale,
};

const POOL_CAPACITY: u64 = 16 * 1024 * 1024;
const N: usize = 1024;

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

// ============================================================================
// figgy state — built once, shared by every Primitive instance.
// ============================================================================

struct PanelEntry {
    chart: Chart,
    view: ChartView,
    series: Vec<SeriesConfig>,
    styles: Vec<ChartStyle>,
    /// Selectable elements of this panel (axes / titles / data area).
    hitmap: HitMap,
    /// Token from `Renderer::prepare` plus the surface physical size captured
    /// at prepare time (iced's `Primitive::draw` doesn't receive the
    /// viewport). Consumed by this panel's `Primitive::draw`; `None` until
    /// the first successful prepare.
    prepared: Option<(PreparedFrame, (u32, u32))>,
}

struct FiggyPipeline {
    /// No lock needed: iced's phases map directly onto figgy's split frame
    /// API. `shader::Primitive::prepare` hands out `&mut Pipeline` →
    /// `Renderer::prepare(&mut self)`, and `shader::Primitive::draw` hands
    /// out `&Pipeline` → `Renderer::paint_prepared(&self)`, so the
    /// `Mutex<Renderer>` wrapper the old `paint(&mut self)` facade required
    /// is gone.
    renderer: Option<Renderer>,
    panels: Vec<PanelEntry>,
    init_error: Option<String>,
    /// Currently selected element: (panel index, hit-map id).
    selected: Option<(usize, HitId)>,
    /// Resize in progress via one of the selected element's handles.
    active_resize: Option<ResizeHandle>,
}

impl FiggyPipeline {
    fn build(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let device = Arc::new(device.clone());
        let queue = Arc::new(queue.clone());
        let mut renderer =
            match Renderer::try_new(RendererDevice::new(device, queue), format, POOL_CAPACITY) {
                Ok(renderer) => renderer,
                Err(e) => {
                    let message = format!("Renderer init failed: {e}");
                    eprintln!("[figgy] {message}");
                    return Self {
                        renderer: None,
                        panels: Vec::new(),
                        init_error: Some(message),
                        selected: None,
                        active_resize: None,
                    };
                }
            };

        let placeholder = Rect {
            x: 0,
            y: 0,
            width: 480,
            height: 480,
        };
        let panels = vec![
            build_sine_panel(&mut renderer, placeholder),
            build_rc_panel(&mut renderer, placeholder),
            build_xs_panel(&mut renderer, placeholder),
        ];
        Self {
            renderer: Some(renderer),
            panels,
            init_error: None,
            selected: None,
            active_resize: None,
        }
    }

    fn shutdown(&mut self) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.wait_idle();
        }
        self.panels.clear();
    }
}

impl Drop for FiggyPipeline {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl iced_wgpu::primitive::Pipeline for FiggyPipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self::build(device, queue, format)
    }
}

// ============================================================================
// 3 panel builders — same as winit/egui.
// ============================================================================

fn build_sine_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    let (xs, ys) = demo::sine_data(N);
    renderer.add_column("sine_x", &col_f64(xs)).unwrap();
    renderer.add_column("sine_y", &col_f64(ys)).unwrap();
    let mut config = default::default_config();
    config.chart_area = ChartArea(rect);
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    let line_color = Color::from_rgb8(20, 110, 230);
    let line_w = 1.0;
    let mut chart = Chart::new(config)
        .with_title("Sine (1 px line, no grid)")
        .with_x_title("x [rad]")
        .with_y_title("y")
        .with_legend_entry("sin(x)", line_color, line_w, LegendEntryKind::Line);
    chart.auto_fit_x(renderer.pool(), "sine_x", 0.02).unwrap();
    chart.auto_fit_y(renderer.pool(), "sine_y", 0.10).unwrap();
    let view = renderer.create_chart_view(&chart, rect).unwrap();
    let cfg = SeriesConfig {
        series_id: "sin".into(),
        label: None,
        source_id: None,
        x_column: "sine_x".into(),
        y_column: "sine_y".into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color,
                line_width: line_w,
            },
        },
    };
    let style = renderer.create_style_for_series(&cfg);
    PanelEntry {
        chart,
        view,
        series: vec![cfg],
        styles: vec![style],
        hitmap: HitMap::standard_chart(),
        prepared: None,
    }
}

fn build_rc_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    let (ts, vs_charge) = demo::rc_data(N);
    let (_, vs_discharge) = demo::rc_discharge_data(N);
    renderer.add_column("rc_t", &col_f64(ts)).unwrap();
    renderer
        .add_column("rc_charge", &col_f64(vs_charge))
        .unwrap();
    renderer
        .add_column("rc_discharge", &col_f64(vs_discharge))
        .unwrap();
    let mut config = default::default_config();
    config.chart_area = ChartArea(rect);
    config.grid.show_major_x = true;
    config.grid.show_major_y = true;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;

    let charge_color = Color::from_rgb8(220, 90, 60);
    let discharge_color = Color::from_rgb8(60, 130, 210);
    let line_w = 2.0;
    let mut chart = Chart::new(config)
        .with_title("RC charge / discharge (2 px, major grid)")
        .with_x_title("t [s]")
        .with_y_title("V [V]")
        .with_legend_entry("Charging", charge_color, line_w, LegendEntryKind::Line)
        .with_legend_entry(
            "Discharging",
            discharge_color,
            line_w,
            LegendEntryKind::Line,
        );
    chart.auto_fit_x(renderer.pool(), "rc_t", 0.02).unwrap();
    chart
        .auto_fit_y_union(renderer.pool(), &["rc_charge", "rc_discharge"], 0.05)
        .unwrap();

    let view = renderer.create_chart_view(&chart, rect).unwrap();
    let mk = |id: &str, x: &str, y: &str, color: Color| SeriesConfig {
        series_id: id.into(),
        label: None,
        source_id: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: color,
                line_width: line_w,
            },
        },
    };
    let cfg_charge = mk("charge", "rc_t", "rc_charge", charge_color);
    let cfg_discharge = mk("discharge", "rc_t", "rc_discharge", discharge_color);
    let style_charge = renderer.create_style_for_series(&cfg_charge);
    let style_discharge = renderer.create_style_for_series(&cfg_discharge);
    PanelEntry {
        chart,
        view,
        series: vec![cfg_charge, cfg_discharge],
        styles: vec![style_charge, style_discharge],
        hitmap: HitMap::standard_chart(),
        prepared: None,
    }
}

fn build_xs_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    let (es, sigmas) = demo::cross_section_data(N);
    renderer.add_column("xs_e", &col_f64(es)).unwrap();
    renderer.add_column("xs_sigma", &col_f64(sigmas)).unwrap();
    let mut config = default::default_config();
    config.chart_area = ChartArea(rect);
    config.left_y.scale = AxisScale::Logarithmic;
    config.right_y.scale = AxisScale::Logarithmic;
    config.grid.show_major_x = true;
    config.grid.show_major_y = true;
    config.grid.show_minor_x = true;
    config.grid.show_minor_y = true;
    config.grid.minor_x_style = LineStylePreset::Dot;
    config.grid.minor_y_style = LineStylePreset::Dot;

    let line_color = Color::from_rgb8(60, 160, 90);
    let line_w = 3.5;
    let mut chart = Chart::new(config)
        .with_title("Cross-section σ(E) — 3.5 px, log + minor grid")
        .with_x_title("E [keV]")
        .with_y_title("σ [barn]")
        .with_legend_entry("σ(E)", line_color, line_w, LegendEntryKind::Line);
    chart.auto_fit_x(renderer.pool(), "xs_e", 0.0).unwrap();
    chart.auto_fit_y(renderer.pool(), "xs_sigma", 0.10).unwrap();
    let view = renderer.create_chart_view(&chart, rect).unwrap();
    let cfg = SeriesConfig {
        series_id: "sigma".into(),
        label: None,
        source_id: None,
        x_column: "xs_e".into(),
        y_column: "xs_sigma".into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color,
                line_width: line_w,
            },
        },
    };
    let style = renderer.create_style_for_series(&cfg);
    PanelEntry {
        chart,
        view,
        series: vec![cfg],
        styles: vec![style],
        hitmap: HitMap::standard_chart(),
        prepared: None,
    }
}

// ============================================================================
// Per-frame Primitive — which panel + screen pixel coords.
// ============================================================================

#[derive(Debug)]
struct FiggyPrimitive {
    panel_idx: usize,
    panel_rect_px: Rect,
}

impl shader::Primitive for FiggyPrimitive {
    type Pipeline = FiggyPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        // Pending pointer input → selection / resize / drag. Processed once
        // per frame from panel 0's prepare (panels render in order), before
        // the dirty checks below so changes show up the same frame.
        if self.panel_idx == 0 {
            if let Some((pi, x, y)) = CLICK_EVENT.lock().unwrap().take() {
                // Resize handles on the selected element win over plain
                // hit-testing.
                let on_handle =
                    pipeline
                        .selected
                        .filter(|(spi, _)| *spi == pi)
                        .and_then(|(spi, id)| {
                            let p = pipeline.panels.get(spi)?;
                            p.hitmap.get(id)?.as_resizable()?.hit_resize_handle(
                                p.chart.config(),
                                &CpuTextMeasure::for_style(&p.chart.config().draw_style),
                                x,
                                y,
                            )
                        });
                if let Some(handle) = on_handle {
                    pipeline.active_resize = Some(handle);
                } else {
                    pipeline.active_resize = None;
                    let new_sel = pipeline
                        .panels
                        .get(pi)
                        .and_then(|p| {
                            p.hitmap.hit_test(
                                p.chart.config(),
                                &CpuTextMeasure::for_style(&p.chart.config().draw_style),
                                x,
                                y,
                            )
                        })
                        .map(|id| (pi, id));
                    if new_sel != pipeline.selected {
                        for affected in [pipeline.selected, new_sel].into_iter().flatten() {
                            if let Some(p) = pipeline.panels.get_mut(affected.0) {
                                p.chart.with_decoration_change(|_| {});
                            }
                        }
                        pipeline.selected = new_sel;
                    }
                }
            }
            // Accumulated drag delta → resize when a handle is active,
            // otherwise the selected element's drag policy. `config_mut`
            // flags both dirty bits (axis drags / resizes move the data
            // area, so the transform refreshes along with the raster).
            if let Some((dx, dy)) = DRAG_EVENT.lock().unwrap().take()
                && let Some((pi, id)) = pipeline.selected
                && let Some(panel) = pipeline.panels.get_mut(pi)
            {
                if let Some(handle) = pipeline.active_resize {
                    if let Some(rz) = panel.hitmap.get(id).and_then(|el| el.as_resizable()) {
                        let _ = rz.resize_by(panel.chart.config_mut(), handle, dx, dy);
                    }
                } else if let Some(drag) = panel.hitmap.get(id).and_then(|el| el.as_draggable()) {
                    let _ = drag.drag_by(panel.chart.config_mut(), dx, dy);
                }
            }
            if RELEASE_EVENT.swap(false, Ordering::Relaxed) {
                pipeline.active_resize = None;
            }
        }
        let selected = pipeline.selected;

        let Some(renderer) = pipeline.renderer.as_mut() else {
            if self.panel_idx == 0 {
                if let Some(message) = pipeline.init_error.as_deref() {
                    eprintln!("[figgy] {message}");
                }
            }
            return;
        };
        let Some(panel) = pipeline.panels.get_mut(self.panel_idx) else {
            return;
        };
        // Cleared up-front so an early return below can't leave `draw` with
        // a token from an older frame; repopulated at the end on success.
        panel.prepared = None;
        let sel_boxes: Vec<SelectionBox> = match selected {
            Some((pi, id)) if pi == self.panel_idx => panel
                .hitmap
                .selection_box(
                    id,
                    panel.chart.config(),
                    &CpuTextMeasure::for_style(&panel.chart.config().draw_style),
                )
                .into_iter()
                .collect(),
            _ => Vec::new(),
        };
        let cur = panel.view.panel_rect();
        if cur != self.panel_rect_px {
            panel.chart.config_mut().chart_area = ChartArea(self.panel_rect_px);
            if let Err(e) = renderer.refresh_axis_with_selection(
                &mut panel.view,
                &panel.chart,
                self.panel_rect_px,
                &sel_boxes,
            ) {
                eprintln!("[figgy] refresh_axis failed: {e}");
                return;
            }
            let _ = panel.chart.consume_data_dirty();
            let _ = panel.chart.consume_raster_dirty();
        } else {
            let raster_dirty = panel.chart.consume_raster_dirty();
            // `Renderer::prepare` below rewrites the transform uniform from
            // the current config every frame, so a data-dirty flag needs no
            // separate `update_transform` call — just consume it.
            let _ = panel.chart.consume_data_dirty();
            if raster_dirty {
                if let Err(e) = renderer.refresh_axis_with_selection(
                    &mut panel.view,
                    &panel.chart,
                    cur,
                    &sel_boxes,
                ) {
                    eprintln!("[figgy] refresh_axis failed: {e}");
                    return;
                }
            }
        }

        // Export every panel separately, but only once — from panel_idx == 0's prepare.
        // The figgy API returns in-memory PNG bytes; the file write happens here.
        if self.panel_idx == 0 && EXPORT_REQUESTED.swap(false, Ordering::Relaxed) {
            let dpi = EXPORT_DPI.load(Ordering::Relaxed) as f32;
            let scale = dpi_to_scale(dpi);
            for (i, p) in pipeline.panels.iter().enumerate() {
                match renderer.export_panel_png_bytes(&p.chart, &p.series, scale) {
                    Ok(bytes) => {
                        let path = format!("/tmp/figgy_iced_panel_{i}.png");
                        match std::fs::write(&path, &bytes) {
                            Ok(_) => eprintln!(
                                "[export] saved {path} (DPI={dpi}, scale={scale:.3}, {} bytes)",
                                bytes.len(),
                            ),
                            Err(e) => eprintln!("[export] write {path} failed: {e}"),
                        }
                    }
                    Err(e) => eprintln!("[export] panel {i} failed: {e}"),
                }
            }
        }

        // Mutable half of the frame: build this panel's draw items and run
        // `Renderer::prepare`. `draw` rebuilds the identical items (same
        // order) and replays them through `paint_prepared(&self)`. Overlap
        // with the other panels' live tokens — or with the export above — is
        // safe: buffers referenced by a live token are copy-on-write.
        let Some(panel) = pipeline.panels.get_mut(self.panel_idx) else {
            return;
        };
        let series: Vec<Series<'_>> = panel
            .series
            .iter()
            .zip(panel.styles.iter())
            .map(|(cfg, style)| Series { config: cfg, style })
            .collect();
        let items = [ChartDrawItem {
            view: &panel.view,
            chart_config: panel.chart.config(),
            series: &series,
        }];
        match renderer.prepare(&items) {
            // `paint_prepared` clamps its viewport/scissor against the real
            // surface pixel size; `draw` doesn't receive the viewport, so
            // capture it here alongside the token.
            Ok(token) => {
                panel.prepared = Some((
                    token,
                    (viewport.physical_width(), viewport.physical_height()),
                ));
            }
            Err(e) => eprintln!("[figgy] prepare failed: {e}"),
        }
    }

    /// `draw` is the efficient path — pure command recording of the token
    /// `prepare` made, into the RenderPass iced already built. `&Pipeline`
    /// is all it needs: `paint_prepared` takes `&self`, so no renderer lock.
    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        let Some(renderer) = pipeline.renderer.as_ref() else {
            return false;
        };
        let Some(panel) = pipeline.panels.get(self.panel_idx) else {
            return false;
        };
        let Some((prepared, target_size)) = panel.prepared.as_ref() else {
            eprintln!(
                "[figgy] panel {}: draw without a prepared frame — skipping",
                self.panel_idx
            );
            return false;
        };
        // The same items, in the same order, as at `prepare`. iced has
        // already set viewport / scissor to the Primitive bounds, but figgy
        // sets its own from panel_rect, clamped to `target_size` — the real
        // surface pixel size captured at prepare time.
        let series: Vec<Series<'_>> = panel
            .series
            .iter()
            .zip(panel.styles.iter())
            .map(|(cfg, style)| Series { config: cfg, style })
            .collect();
        let items = [ChartDrawItem {
            view: &panel.view,
            chart_config: panel.chart.config(),
            series: &series,
        }];
        match renderer.paint_prepared(render_pass, *target_size, &items, prepared) {
            Ok(()) => true,
            Err(e) => {
                // Includes `StalePreparedFrame` if an invalidating `&mut`
                // call interleaved — skip this frame; the next `prepare`
                // recovers.
                eprintln!("[figgy] paint_prepared failed: {e}");
                false
            }
        }
    }
}

// ============================================================================
// shader::Program — issues the Primitive.
// ============================================================================

struct ChartCanvas {
    panel_idx: usize,
}

/// Per-widget pointer state: pressed flag + last cursor position, used to
/// turn CursorMoved events into drag deltas while the button is down.
#[derive(Default)]
struct CanvasState {
    pressed: bool,
    last: Option<iced::Point>,
}

impl<Message> shader::Program<Message> for ChartCanvas {
    type State = CanvasState;
    type Primitive = FiggyPrimitive;

    /// Left press → record (panel, cursor px) for `Primitive::prepare` to
    /// hit-test against the panel's `HitMap`; subsequent moves while pressed
    /// accumulate drag deltas. Coordinates are in the same space as the
    /// bounds `draw` converts to `panel_rect_px`.
    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        match event {
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_over(bounds) {
                    state.pressed = true;
                    state.last = Some(pos);
                    *CLICK_EVENT.lock().unwrap() = Some((self.panel_idx, pos.x, pos.y));
                    return Some(shader::Action::request_redraw());
                }
            }
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) if state.pressed => {
                if let Some(last) = state.last {
                    let (dx, dy) = (position.x - last.x, position.y - last.y);
                    if dx != 0.0 || dy != 0.0 {
                        let mut pending = DRAG_EVENT.lock().unwrap();
                        let (ax, ay) = pending.unwrap_or((0.0, 0.0));
                        *pending = Some((ax + dx, ay + dy));
                    }
                }
                state.last = Some(*position);
                return Some(shader::Action::request_redraw());
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                state.pressed = false;
                state.last = None;
                RELEASE_EVENT.store(true, Ordering::Relaxed);
            }
            _ => {}
        }
        None
    }

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: iced::mouse::Cursor,
        bounds: Rectangle,
    ) -> Self::Primitive {
        // bounds from iced (logical coords) → physical pixels. The point at which
        // iced applies the pixel ratio varies, but these bounds are already close
        // to pixels (iced_wgpu converts). Round and clamp to be safe.
        let panel_rect_px = Rect {
            x: bounds.x.round().max(0.0) as u32,
            y: bounds.y.round().max(0.0) as u32,
            width: bounds.width.round().max(1.0) as u32,
            height: bounds.height.round().max(1.0) as u32,
        };
        FiggyPrimitive {
            panel_idx: self.panel_idx,
            panel_rect_px,
        }
    }
}

// ============================================================================
// iced app.
// ============================================================================

struct App {
    dpi_text: String,
}

#[derive(Debug, Clone)]
enum Message {
    SavePng,
    DpiChanged(String),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        (
            App {
                dpi_text: "192".to_string(),
            },
            Task::none(),
        )
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::DpiChanged(s) => {
                // Only keep digits; non-digit input is ignored.
                if s.is_empty() || s.chars().all(|c| c.is_ascii_digit()) {
                    self.dpi_text = s;
                    if let Ok(d) = self.dpi_text.parse::<u32>() {
                        EXPORT_DPI.store(d, Ordering::Relaxed);
                    }
                }
            }
            Message::SavePng => {
                EXPORT_REQUESTED.store(true, Ordering::Relaxed);
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let chart = |idx: usize| {
            shader(ChartCanvas { panel_idx: idx })
                .width(Length::Fill)
                .height(Length::Fill)
        };
        let dpi_min = (MIN_EXPORT_SCALE * 96.0).round() as u32;
        let dpi_max = (MAX_EXPORT_SCALE * 96.0).round() as u32;
        container(ic_column![
            row![
                text("figgy + iced — sine / RC / cross-section").size(18),
                text(format!("DPI ({}-{}):", dpi_min, dpi_max)),
                text_input("96", &self.dpi_text)
                    .on_input(Message::DpiChanged)
                    .width(80),
                button("Save PNG").on_press(Message::SavePng),
            ]
            .spacing(12),
            row![
                container(chart(0)).width(Length::FillPortion(1)),
                container(chart(1)).width(Length::FillPortion(1)),
                container(chart(2)).width(Length::FillPortion(1)),
            ]
            .spacing(8)
            .height(Length::Fill),
        ])
        .padding(12)
        .into()
    }
}

fn theme_fn(_state: &App) -> Theme {
    Theme::Light
}

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("figgy iced demo")
        .theme(theme_fn)
        .window_size(iced::Size::new(1800.0, 500.0))
        .run()
}

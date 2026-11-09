//! figgy + egui_wgpu CallbackTrait integration — 3-panel demo.
//!
//! Stacks renderer::Renderer on top of eframe (winit + egui_wgpu) and draws
//! sine / RC / cross-section charts via paint callbacks.
//!
//! Flow:
//!
//! 1. Get device/queue/format from eframe's `wgpu_render_state`.
//! 2. On first update, build `renderer::Renderer`, register columns, set up
//!    Charts, and store the result in `CallbackResources` as a plain
//!    `FiggyState` — no `Mutex`; the split frame API never needs
//!    `&mut Renderer` in paint (see the struct doc).
//! 3. Each frame: allocate 3 panel regions in egui, register an
//!    `egui_wgpu::Callback` for each.
//! 4. Callback `prepare` (`&mut CallbackResources`): set the panel rect and
//!    the selected element on the chart, then
//!    `renderer::Renderer::prepare(items)` — which redraws the text and axis
//!    raster when the chart changed and writes the panel's data→NDC
//!    transform uniform — and stash the returned `PreparedFrame` token on
//!    the panel.
//! 5. Callback `paint` (`&CallbackResources`): record only the prepared token
//!    with `renderer::Renderer::paint_prepared(pass, target, token)`.
//!
//! Run with:
//! `cargo run -p figgy-renderer --example egui_embed --features egui_demo`

use eframe::egui_wgpu::{self, CallbackTrait};
use eframe::wgpu;

use renderer::color::Color;
use renderer::config::{AxisOptions, AxisScale, Config, GridOptions};
use renderer::default;
use renderer::demo;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, ChartDrawItem, ChartStyle, ChartView, CpuTextMeasure, HitId, HitMap, MIN_EXPORT_SCALE,
    PreparedFrame, Renderer, RendererDevice, ResizeHandle, Series, SeriesConfig, dpi_to_scale,
};

const POOL_CAPACITY: u64 = 16 * 1024 * 1024;
const N: usize = 1024;

// Use light foreground for axes / labels / titles so they read on dark egui backgrounds.
const FG: Color = Color {
    r: 0.92,
    g: 0.92,
    b: 0.92,
    a: 1.0,
};
const GRID_MAJOR: Color = Color {
    r: 0.40,
    g: 0.40,
    b: 0.45,
    a: 1.0,
};
const GRID_MINOR: Color = Color {
    r: 0.30,
    g: 0.30,
    b: 0.35,
    a: 1.0,
};
const EGUI_BG: egui::Color32 = egui::Color32::from_rgb(18, 18, 22);

fn force_dark_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = EGUI_BG;
    visuals.window_fill = EGUI_BG;
    visuals.faint_bg_color = egui::Color32::from_rgb(28, 28, 34);
    ctx.set_visuals(visuals);
}

fn paint_axis_dark(axis: &mut AxisOptions) {
    axis.line_color = FG;
    axis.label_style.color = FG;
    axis.title_option.text.color = FG;
}

fn paint_grid_dark(grid: &mut GridOptions) {
    grid.major_x_color = GRID_MAJOR;
    grid.major_y_color = GRID_MAJOR;
    grid.minor_x_color = GRID_MINOR;
    grid.minor_y_color = GRID_MINOR;
}

/// Default Config tweaked for dark themes.
fn dark_config() -> Config {
    let mut c = default::default_config();
    c.chart_title.text.color = FG;
    paint_axis_dark(&mut c.top_x);
    paint_axis_dark(&mut c.bottom_x);
    paint_axis_dark(&mut c.left_y);
    paint_axis_dark(&mut c.right_y);
    paint_grid_dark(&mut c.grid);
    c
}

// ============================================================================
// One panel bundle + overall figgy state.
// ============================================================================

struct PanelEntry {
    chart: Chart,
    view: ChartView,
    series: Vec<SeriesConfig>,
    styles: Vec<ChartStyle>,
    /// Selectable elements of this panel (axes / titles / data area).
    hitmap: HitMap,
    /// Frame token from `Renderer::prepare`, consumed by `paint_prepared`.
    /// Written by `CallbackTrait::prepare` (`&mut CallbackResources`), read
    /// by `CallbackTrait::paint` (`&CallbackResources`). Per-panel because
    /// egui runs every callback's `prepare` before any `paint` — a single
    /// shared token would be overwritten by the other panels' prepares.
    prepared: Option<PreparedFrame>,
}

/// All figgy state, stored directly in `CallbackResources` (no `Mutex`).
///
/// egui's `CallbackTrait::paint` only hands out `&CallbackResources`, and
/// the split frame API fits that exactly: `prepare` (called with
/// `&mut CallbackResources`) does every `&mut Renderer` step — pipeline
/// compiles, transform-uniform writes, arc-length dispatch — and stores a
/// [`PreparedFrame`] token per panel; `paint` then records commands through
/// `Renderer::paint_prepared(&self, ..)` with shared access only.
/// `Renderer` is `Send + Sync`, so no host-side locking is needed.
struct FiggyState {
    renderer: Renderer,
    panels: Vec<PanelEntry>,
    /// Currently selected element: (panel index, hit-map id).
    selected: Option<(usize, HitId)>,
    /// Resize in progress via one of the selected element's handles.
    active_resize: Option<ResizeHandle>,
}

impl FiggyState {
    /// Press in surface pixels → resize-handle check on the selected element
    /// first (handles overlay everything), then plain hit-test selection.
    /// Panels whose decoration layer changes are raster-flagged.
    fn handle_click(&mut self, panel_idx: usize, x: f32, y: f32) {
        if let Some((pi, id)) = self.selected
            && pi == panel_idx
            && let Some(panel) = self.panels.get(pi)
            && let Some(rz) = panel.hitmap.get(id).and_then(|el| el.as_resizable())
            && let Some(handle) = rz.hit_resize_handle(
                panel.chart.config(),
                &CpuTextMeasure::for_style(&panel.chart.config().draw_style),
                x,
                y,
            )
        {
            self.active_resize = Some(handle);
            return;
        }
        self.active_resize = None;

        let new_sel = self
            .panels
            .get(panel_idx)
            .and_then(|p| {
                p.hitmap.hit_test(
                    p.chart.config(),
                    &CpuTextMeasure::for_style(&p.chart.config().draw_style),
                    x,
                    y,
                )
            })
            .map(|id| (panel_idx, id));
        if new_sel == self.selected {
            return;
        }
        for affected in [self.selected, new_sel].into_iter().flatten() {
            if let Some(panel) = self.panels.get_mut(affected.0) {
                panel.chart.with_decoration_change(|_| {});
            }
        }
        self.selected = new_sel;
    }

    /// Pointer drag delta (surface pixels) → resize when a handle is active,
    /// otherwise the selected element's drag policy. `config_mut` flags both
    /// dirty bits — axis drags / resizes move the data area, so the transform
    /// refreshes along with the raster.
    fn handle_drag(&mut self, panel_idx: usize, dx: f32, dy: f32) {
        let Some((pi, id)) = self.selected else {
            return;
        };
        if pi != panel_idx {
            return;
        }
        let Some(panel) = self.panels.get_mut(pi) else {
            return;
        };
        let measure = CpuTextMeasure::for_style(&panel.chart.config().draw_style);
        if let Some(handle) = self.active_resize {
            if let Some(rz) = panel.hitmap.get(id).and_then(|el| el.as_resizable()) {
                let _ = rz.resize_by(panel.chart.config_mut(), &measure, handle, dx, dy);
            }
            return;
        }
        let Some(drag) = panel.hitmap.get(id).and_then(|el| el.as_draggable()) else {
            return;
        };
        let _ = drag.drag_by(panel.chart.config_mut(), &measure, dx, dy);
    }
}

impl FiggyState {
    fn shutdown(&mut self) {
        self.renderer.wait_idle().unwrap();
        self.panels.clear();
    }
}

impl Drop for FiggyState {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ============================================================================
// 3 panel builders — same pattern as winit_simple.
// ============================================================================

fn build_sine_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    let (xs, ys) = demo::sine_data(N);
    renderer.add_column("sine_x", &xs).expect("add x");
    renderer.add_column("sine_y", &ys).expect("add y");
    let mut config = dark_config();
    config.chart_area = ChartArea(rect);
    // Chart 1: grid off.
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;

    let series = vec![
        SeriesConfig::line("sin", "sine_x", "sine_y")
            .label("sin(x)")
            .color(Color::from_rgb8(100, 180, 255))
            .width(1.0),
    ];
    let mut chart = Chart::new(config)
        .with_title("Sine (1 px line, no grid)")
        .with_x_title("x [rad]")
        .with_y_title("y")
        .with_legend_from(&series);
    chart.auto_fit_x(renderer.pool(), "sine_x", 0.02).unwrap();
    chart.auto_fit_y(renderer.pool(), "sine_y", 0.10).unwrap();
    let view = renderer.create_chart_view(&chart, rect).unwrap();
    panel_entry(renderer, chart, view, series)
}

fn build_rc_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    let (ts, vs_charge) = demo::rc_data(N);
    let (_, vs_discharge) = demo::rc_discharge_data(N);
    renderer.add_column("rc_t", &ts).unwrap();
    renderer.add_column("rc_charge", &vs_charge).unwrap();
    renderer.add_column("rc_discharge", &vs_discharge).unwrap();

    let mut config = dark_config();
    config.chart_area = ChartArea(rect);
    config.grid.show_major_x = true;
    config.grid.show_major_y = true;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;

    let series = vec![
        SeriesConfig::line("charge", "rc_t", "rc_charge")
            .label("Charging")
            .color(Color::from_rgb8(255, 140, 100))
            .width(2.0),
        SeriesConfig::line("discharge", "rc_t", "rc_discharge")
            .label("Discharging")
            .color(Color::from_rgb8(120, 200, 255))
            .width(2.0),
    ];
    let mut chart = Chart::new(config)
        .with_title("RC charge / discharge (2 px, major grid)")
        .with_x_title("t [s]")
        .with_y_title("V [V]")
        .with_legend_from(&series);
    chart.auto_fit_x(renderer.pool(), "rc_t", 0.02).unwrap();
    chart
        .auto_fit_y_union(renderer.pool(), &["rc_charge", "rc_discharge"], 0.05)
        .unwrap();

    let view = renderer.create_chart_view(&chart, rect).unwrap();
    panel_entry(renderer, chart, view, series)
}

fn build_xs_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    let (es, sigmas) = demo::cross_section_data(N);
    renderer.add_column("xs_e", &es).unwrap();
    renderer.add_column("xs_sigma", &sigmas).unwrap();
    let mut config = dark_config();
    config.chart_area = ChartArea(rect);
    config.left_y.scale = AxisScale::Logarithmic;
    config.right_y.scale = AxisScale::Logarithmic;
    // Chart 3: major + minor grid, minor is dotted.
    config.grid.show_major_x = true;
    config.grid.show_major_y = true;
    config.grid.show_minor_x = true;
    config.grid.show_minor_y = true;
    config.grid.minor_x_style = LineStylePreset::Dot;
    config.grid.minor_y_style = LineStylePreset::Dot;

    let series = vec![
        SeriesConfig::line("sigma", "xs_e", "xs_sigma")
            .label("σ(E)")
            .color(Color::from_rgb8(120, 220, 140))
            .width(3.5),
    ];
    let mut chart = Chart::new(config)
        .with_title("Cross-section σ(E) — 3.5 px, log + minor grid")
        .with_x_title("E [keV]")
        .with_y_title("σ [barn]")
        .with_legend_from(&series);
    chart.auto_fit_x(renderer.pool(), "xs_e", 0.0).unwrap();
    chart.auto_fit_y(renderer.pool(), "xs_sigma", 0.10).unwrap();
    let view = renderer.create_chart_view(&chart, rect).unwrap();
    panel_entry(renderer, chart, view, series)
}

/// A panel over `series`, with one GPU style per series.
fn panel_entry(
    renderer: &mut Renderer,
    chart: Chart,
    view: ChartView,
    series: Vec<SeriesConfig>,
) -> PanelEntry {
    let styles = series
        .iter()
        .map(|cfg| renderer.create_style_for_series(cfg).unwrap())
        .collect();
    PanelEntry {
        chart,
        view,
        series,
        styles,
        hitmap: HitMap::standard_chart(),
        prepared: None,
    }
}

// ============================================================================
// CallbackTrait — bridge between egui_wgpu and figgy.
// ============================================================================

/// Series list used to build the prepare-only panel input.
fn panel_series<'a>(series: &'a [SeriesConfig], styles: &'a [ChartStyle]) -> Vec<Series<'a>> {
    series
        .iter()
        .zip(styles.iter())
        .map(|(cfg, style)| Series { config: cfg, style })
        .collect()
}

struct FiggyCallback {
    panel_idx: usize,
    panel_rect_px: Rect, // physical pixels (egui logical * pixels_per_point).
}

impl CallbackTrait for FiggyCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(state) = callback_resources.get_mut::<FiggyState>() else {
            return Vec::new();
        };
        let selected = state.selected;
        let Some(panel) = state.panels.get_mut(self.panel_idx) else {
            return Vec::new();
        };
        // The renderer draws the selection from the element's own placement;
        // the host only names the element.
        let selected_id: Option<String> = match selected {
            Some((pi, id)) if pi == self.panel_idx => {
                panel.hitmap.get(id).map(|el| el.element_id())
            }
            _ => None,
        };

        // The chart holds what the raster draws: its area and its selected
        // element. `Renderer::prepare` below redraws the text and axis
        // raster when either changed, and writes the data→NDC transform
        // every frame.
        if panel.chart.config().chart_area.0 != self.panel_rect_px {
            panel.chart.config_mut().chart_area = ChartArea(self.panel_rect_px);
        }
        panel.chart.select(selected_id.as_deref());

        // Mutable half of the frame: compile any missing pipelines, write
        // this panel's transform uniform, dispatch arc-length compute. The
        // returned token is what `paint` records against.
        let series = panel_series(&panel.series, &panel.styles);
        let mut items = [ChartDrawItem {
            view: &mut panel.view,
            chart: &panel.chart,
            series: &series,
        }];
        let prepared = state.renderer.prepare(&mut items);
        drop(items);
        panel.prepared = match prepared {
            Ok(token) => Some(token),
            Err(e) => {
                eprintln!("[figgy] prepare failed: {e}");
                None
            }
        };
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        // Shared access is enough: `paint_prepared` takes `&self`, so no
        // lock bridges egui's `&CallbackResources` to the renderer.
        let Some(state) = callback_resources.get::<FiggyState>() else {
            return;
        };
        let Some(panel) = state.panels.get(self.panel_idx) else {
            return;
        };
        let Some(prepared) = panel.prepared.as_ref() else {
            eprintln!(
                "[figgy] panel {}: no prepared frame; skipping paint",
                self.panel_idx
            );
            return;
        };
        // target = pixel size of the color attachment the current render pass draws into (egui's swap chain).
        let target_size = (info.screen_size_px[0], info.screen_size_px[1]);
        // A stale token (invalidating `&mut` call between prepare and paint)
        // is recoverable: skip this frame, the next prepare re-issues it.
        if let Err(e) = state
            .renderer
            .paint_prepared(render_pass, target_size, prepared)
        {
            eprintln!("[figgy] paint_prepared failed (skipping frame): {e}");
        }
    }
}

// ============================================================================
// eframe App
// ============================================================================

struct DemoApp {
    initialized: bool,
    /// User-entered DPI. figgy converts to scale = dpi/96 internally and clamps.
    export_dpi: u32,
    render_state: Option<egui_wgpu::RenderState>,
    renderer_error: Option<String>,
}

impl Default for DemoApp {
    fn default() -> Self {
        Self {
            initialized: false,
            export_dpi: 192,
            render_state: None,
            renderer_error: None,
        }
    }
}

impl DemoApp {
    fn show_renderer_error(ui: &mut egui::Ui, message: &str) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(EGUI_BG))
            .show(ui, |ui| {
                ui.label(message);
            });
    }

    fn cleanup_figgy_state(&mut self) {
        let Some(render_state) = self.render_state.take() else {
            self.initialized = false;
            return;
        };

        {
            let mut renderer_guard = render_state.renderer.write();
            if let Some(mut state) = renderer_guard.callback_resources.remove::<FiggyState>() {
                state.shutdown();
            }
        }

        let _ = render_state.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        self.initialized = false;
    }
}

impl Drop for DemoApp {
    fn drop(&mut self) {
        self.cleanup_figgy_state();
    }
}

impl eframe::App for DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        force_dark_theme(&ctx);

        // 1) Lazy init: build renderer::Renderer + register columns + Chart on the first frame.
        let render_state = match frame.wgpu_render_state() {
            Some(r) => r,
            None => {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default().fill(EGUI_BG))
                    .show(ui, |ui| {
                        ui.label("No eframe wgpu_render_state — wgpu backend must be enabled.");
                    });
                return;
            }
        };
        self.render_state = Some(render_state.clone());

        if !self.initialized {
            // wgpu::Device and wgpu::Queue are reference-counted handles, so the
            // clones share RenderState's device and queue.
            let device = render_state.device.clone();
            let queue = render_state.queue.clone();
            let format = render_state.target_format;

            let mut renderer = match Renderer::try_new(
                RendererDevice::new(device, queue).expect("figgy poll thread"),
                format,
                POOL_CAPACITY,
            ) {
                Ok(renderer) => renderer,
                Err(e) => {
                    let message = format!("Renderer init failed: {e}");
                    self.renderer_error = Some(message);
                    if let Some(message) = self.renderer_error.as_deref() {
                        Self::show_renderer_error(ui, message);
                    }
                    return;
                }
            };

            // Initial panel rect — guess for the first frame; prepare overwrites it
            // with the real egui rect.
            let placeholder = Rect {
                x: 0,
                y: 0,
                width: 400,
                height: 300,
            };
            let panels = vec![
                build_sine_panel(&mut renderer, placeholder),
                build_rc_panel(&mut renderer, placeholder),
                build_xs_panel(&mut renderer, placeholder),
            ];

            render_state
                .renderer
                .write()
                .callback_resources
                .insert(FiggyState {
                    renderer,
                    panels,
                    selected: None,
                    active_resize: None,
                });
            self.initialized = true;
            self.renderer_error = None;
        } else {
            let mut renderer_guard = render_state.renderer.write();
            if let Some(state) = renderer_guard.callback_resources.get_mut::<FiggyState>() {
                if let Err(e) = state
                    .renderer
                    .ensure_target_format(render_state.target_format)
                {
                    let message = format!("Renderer target format failed: {e}");
                    self.renderer_error = Some(message);
                    if let Some(message) = self.renderer_error.as_deref() {
                        Self::show_renderer_error(ui, message);
                    }
                    return;
                }
                self.renderer_error = None;
            } else {
                self.initialized = false;
            }
        }

        // 2) UI.
        let pixels_per_point = ctx.pixels_per_point();
        let render_state_clone = render_state.clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(EGUI_BG))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("figgy + egui_wgpu — sine / RC / cross-section");
                    // DPI input — figgy maps dpi/96 → scale → clamp internally.
                    ui.label("DPI:");
                    let dpi_min = (MIN_EXPORT_SCALE * 96.0).round() as u32;
                    // No renderer bound: the device texture limit decides; 16x here.
                    let dpi_max = 16 * 96;
                    ui.add(
                        egui::DragValue::new(&mut self.export_dpi)
                            .range(dpi_min..=dpi_max)
                            .speed(1.0),
                    );
                    ui.label(format!("(min {dpi_min} / max {dpi_max})"));
                    if ui.button("Save PNG").clicked() {
                        let scale = dpi_to_scale(self.export_dpi as f32);
                        let mut renderer_guard = render_state_clone.renderer.write();
                        let state = renderer_guard
                            .callback_resources
                            .get_mut::<FiggyState>()
                            .expect("FiggyState");
                        let (renderer, panels) = (&mut state.renderer, &state.panels);
                        for (i, panel) in panels.iter().enumerate() {
                            let path = format!("/tmp/figgy_egui_panel_{i}.png");
                            match renderer.save_blocking(
                                renderer::ExportSource::chart(&panel.chart, &panel.series),
                                renderer::ExportOptions::panel(scale),
                                &path,
                            ) {
                                Ok(()) => eprintln!(
                                    "[export] saved {path} (DPI={}, scale={:.3})",
                                    self.export_dpi, scale,
                                ),
                                Err(e) => eprintln!("[export] panel {i} failed: {e}"),
                            }
                        }
                    }
                });
                ui.add_space(4.0);
                // Pointer input → (panel index, surface-pixel data); resolved after
                // the closure so the CallbackResources lock isn't held inside the
                // UI pass. Press (click or drag start) selects; drag moves.
                let mut pressed: Option<(usize, f32, f32)> = None;
                let mut dragged: Option<(usize, f32, f32)> = None;
                let mut drag_ended = false;
                ui.columns(3, |cols| {
                    for (i, col_ui) in cols.iter_mut().enumerate() {
                        let avail = col_ui.available_size();
                        let (rect, resp) =
                            col_ui.allocate_exact_size(avail, egui::Sense::click_and_drag());
                        if (resp.clicked() || resp.drag_started())
                            && let Some(pos) = resp.interact_pointer_pos()
                        {
                            // egui points → physical pixels (chart_area space).
                            pressed = Some((i, pos.x * pixels_per_point, pos.y * pixels_per_point));
                        }
                        if resp.dragged() {
                            let d = resp.drag_delta();
                            if d != egui::Vec2::ZERO {
                                dragged = Some((i, d.x * pixels_per_point, d.y * pixels_per_point));
                            }
                        }
                        if resp.drag_stopped() {
                            drag_ended = true;
                        }

                        let panel_rect_px = Rect {
                            x: (rect.min.x * pixels_per_point).round().max(0.0) as u32,
                            y: (rect.min.y * pixels_per_point).round().max(0.0) as u32,
                            width: (rect.width() * pixels_per_point).max(1.0) as u32,
                            height: (rect.height() * pixels_per_point).max(1.0) as u32,
                        };
                        let cb = egui_wgpu::Callback::new_paint_callback(
                            rect,
                            FiggyCallback {
                                panel_idx: i,
                                panel_rect_px,
                            },
                        );
                        col_ui.painter().add(cb);
                    }
                });
                if pressed.is_some() || dragged.is_some() || drag_ended {
                    let mut renderer_guard = render_state_clone.renderer.write();
                    if let Some(state) = renderer_guard.callback_resources.get_mut::<FiggyState>() {
                        if let Some((i, x, y)) = pressed {
                            state.handle_click(i, x, y);
                        }
                        if let Some((i, dx, dy)) = dragged {
                            state.handle_drag(i, dx, dy);
                        }
                        if drag_ended {
                            state.active_resize = None;
                        }
                    }
                }
            });
    }

    fn on_exit(&mut self) {
        self.cleanup_figgy_state();
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1800.0, 500.0])
            .with_title("figgy demo (egui)"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "figgy egui demo",
        options,
        Box::new(|_cc| Ok(Box::new(DemoApp::default()))),
    )
}

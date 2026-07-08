//! figgy SSoT live-edit lab — a runtime proof of the prepare/paint_prepared
//! split, at column-pool scale.
//!
//! Four chart panels in a 2×2 grid, one per draw style (Precise with a
//! dashed line, Sketch, Milkyway, Constellation). All four series read the
//! SAME x column from the pool — 5 columns total (1× x + 4× y), so at the
//! 5M-per-series ceiling the pool holds 25M floats while four styled
//! pipelines, four arc scans, and four transform uniforms are prepared per
//! frame through `Renderer::prepare` and recorded through
//! `Renderer::paint_prepared` — no `Mutex`, no `update_transform`.
//!
//! Vertically adjacent panels are x-linked: the top and bottom chart of
//! each column always show the same x window (one `set_x_range` edit lands
//! on both Charts — an SSoT edit every animated frame). Each column pair
//! has its own pan direction control.
//!
//! Run with:
//! `cargo run --release -p renderer --example ssot_lab --features egui_demo`

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui_wgpu::{self, CallbackTrait};
use eframe::wgpu;

use renderer::color::Color;
use renderer::config::{
    AxisOptions, Config, ConstellationOptions, DrawStyle, GridOptions, MilkywayOptions,
    SketchOptions,
};
use renderer::data::Column;
use renderer::default;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, ChartDrawItem, ChartStyle, ChartView, DataLineStyleConfig, DataRenderType,
    DataScatterStyleConfig, PreparedFrame, Renderer, RendererDevice, ScatterShape, Series,
    SeriesConfig,
};

// 3M points/series × (1 shared x + 4 y) columns × 4 bytes = 60 MB live,
// plus headroom for the remove+add churn the density slider causes. The
// arc-scan compute binds the whole pool as ONE storage binding, so at init
// this target is clamped to the device's `max_storage_buffer_binding_size`
// (raised to the adapter's ceiling in `main` — wgpu's default is only
// 128 MB) and the density slider's ceiling shrinks to match.
const POOL_TARGET_BYTES: u64 = 192 * 1024 * 1024;
const MAX_POINTS_PER_SERIES: u32 = 3_000_000;

const X_ID: &str = "quad_x";
const Y_IDS: [&str; 4] = ["quad_y0", "quad_y1", "quad_y2", "quad_y3"];
const SERIES_IDS: [&str; 4] = ["precise", "sketch", "milkyway", "constellation"];
const X_SPAN: f64 = 12.566; // 4π

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

const FG: Color = Color {
    r: 0.92,
    g: 0.92,
    b: 0.92,
    a: 1.0,
};
const EGUI_BG: egui::Color32 = egui::Color32::from_rgb(18, 18, 22);

fn paint_axis_dark(axis: &mut AxisOptions) {
    axis.line_color = FG;
    axis.label_style.color = FG;
    axis.title_option.text.color = FG;
}

fn paint_grid_dark(grid: &mut GridOptions) {
    let major = Color {
        r: 0.40,
        g: 0.40,
        b: 0.45,
        a: 1.0,
    };
    let minor = Color {
        r: 0.30,
        g: 0.30,
        b: 0.35,
        a: 1.0,
    };
    grid.major_x_color = major;
    grid.major_y_color = major;
    grid.minor_x_color = minor;
    grid.minor_y_color = minor;
}

fn dark_config() -> Config {
    let mut c = default::default_config();
    c.chart_title.text.color = FG;
    paint_axis_dark(&mut c.top_x);
    paint_axis_dark(&mut c.bottom_x);
    paint_axis_dark(&mut c.left_y);
    paint_axis_dark(&mut c.right_y);
    paint_grid_dark(&mut c.grid);
    c.legend.visible = false;
    c
}

/// Shared x samples over `[0, X_SPAN]` — one column serves all four series.
fn lab_xs(n: usize) -> Vec<f64> {
    let n = n.max(2);
    (0..n).map(|i| i as f64 / (n - 1) as f64 * X_SPAN).collect()
}

/// Per-panel waveform over the shared xs. `variant` shifts the phase so a
/// regen visibly changes every panel's shape at once.
fn lab_ys(panel: usize, variant: u32, xs: &[f64]) -> Vec<f64> {
    let phase = variant as f64 * 0.9 + panel as f64 * 1.3;
    let (h1, h2, damp) = match panel {
        0 => (1.0, 0.35, 1.4),
        1 => (0.8, 0.5, 2.0),
        2 => (1.0, 0.2, 9.0),
        _ => (0.7, 0.45, 1.1),
    };
    xs.iter()
        .map(|&x| {
            (h1 * (x + phase).sin() + h2 * ((2.0 + panel as f64) * x + phase * 2.0).sin())
                * (-x / (X_SPAN * damp)).exp()
        })
        .collect()
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

// ============================================================================
// Panels + overall state, stored in CallbackResources without a Mutex.
// ============================================================================

struct PanelEntry {
    chart: Chart,
    view: ChartView,
    series: SeriesConfig,
    style: ChartStyle,
    /// Frame token from `Renderer::prepare` — per panel because egui runs
    /// every callback's prepare before any paint; a shared token would be
    /// overwritten by the other panels' prepares.
    prepared: Option<PreparedFrame>,
}

struct LabState {
    renderer: Renderer,
    panels: Vec<PanelEntry>,
    /// Panel-frames `paint_prepared` refused or skipped. Should stay 0.
    /// Incremented in `paint` (`&CallbackResources`) — hence atomic.
    frames_skipped: AtomicU32,
    last_error: Option<String>,
    data_variant: u32,
    points_per_series: usize,
}

impl LabState {
    fn shutdown(&mut self) {
        self.renderer.wait_idle();
    }

    fn regen_data(&mut self, n: usize, bump_variant: bool) {
        if bump_variant {
            self.data_variant += 1;
        }
        let xs = lab_xs(n);
        self.renderer.remove_column(X_ID);
        for y_id in Y_IDS {
            self.renderer.remove_column(y_id);
        }
        if let Err(e) = self.renderer.add_column(X_ID, &col_f64(xs.clone())) {
            self.last_error = Some(format!("regen add_column x: {e}"));
            return;
        }
        for (i, y_id) in Y_IDS.iter().enumerate() {
            let ys = lab_ys(i, self.data_variant, &xs);
            if let Err(e) = self.renderer.add_column(*y_id, &col_f64(ys)) {
                self.last_error = Some(format!("regen add_column {y_id}: {e}"));
                return;
            }
        }
        self.points_per_series = xs.len();
        for (i, panel) in self.panels.iter_mut().enumerate() {
            let _ = panel.chart.auto_fit_y(self.renderer.pool(), Y_IDS[i], 0.10);
        }
    }
}

impl Drop for LabState {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Free functions over individual fields (not `&LabState`/`&PanelEntry`
/// methods) so `state.renderer` stays free for the `&mut` prepare call —
/// the split-borrow pattern from the renderer's host-integration notes.
fn one_series<'a>(cfg: &'a SeriesConfig, style: &'a ChartStyle) -> [Series<'a>; 1] {
    [Series { config: cfg, style }]
}

fn panel_items<'a>(
    view: &'a ChartView,
    chart_config: &'a Config,
    series: &'a [Series<'a>],
) -> [ChartDrawItem<'a>; 1] {
    [ChartDrawItem {
        view,
        chart_config,
        series,
    }]
}

// ============================================================================
// CallbackTrait — the split frame API against egui's prepare/paint schedule.
// ============================================================================

struct LabCallback {
    panel_idx: usize,
    panel_rect_px: Rect,
}

impl CallbackTrait for LabCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(state) = callback_resources.get_mut::<LabState>() else {
            return Vec::new();
        };
        let Some(panel) = state.panels.get_mut(self.panel_idx) else {
            return Vec::new();
        };

        let cur_rect = panel.view.panel_rect();
        let rect_changed = cur_rect != self.panel_rect_px;
        if rect_changed {
            panel.chart.config_mut().chart_area = ChartArea(self.panel_rect_px);
        }
        let raster_dirty = panel.chart.consume_raster_dirty();
        // The data→NDC transform uniform is written by `Renderer::prepare`
        // below every frame; the flag only needs consuming.
        let _ = panel.chart.consume_data_dirty();
        if rect_changed || raster_dirty {
            let rect = if rect_changed {
                self.panel_rect_px
            } else {
                cur_rect
            };
            if let Err(e) = state
                .renderer
                .refresh_axis(&mut panel.view, &panel.chart, rect)
            {
                state.last_error = Some(format!("refresh_axis: {e}"));
                state.panels[self.panel_idx].prepared = None;
                return Vec::new();
            }
        }

        let prepared = {
            let series = one_series(&panel.series, &panel.style);
            let items = panel_items(&panel.view, panel.chart.config(), &series);
            state.renderer.prepare(&items)
        };
        let panel = &mut state.panels[self.panel_idx];
        match prepared {
            Ok(token) => {
                panel.prepared = Some(token);
                state.last_error = None;
            }
            Err(e) => {
                panel.prepared = None;
                state.last_error = Some(format!("prepare: {e}"));
            }
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(state) = callback_resources.get::<LabState>() else {
            return;
        };
        let Some(panel) = state.panels.get(self.panel_idx) else {
            return;
        };
        let Some(prepared) = panel.prepared.as_ref() else {
            state.frames_skipped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let series = one_series(&panel.series, &panel.style);
        let items = panel_items(&panel.view, panel.chart.config(), &series);
        let target_size = (info.screen_size_px[0], info.screen_size_px[1]);
        if let Err(e) = state
            .renderer
            .paint_prepared(render_pass, target_size, &items, prepared)
        {
            state.frames_skipped.fetch_add(1, Ordering::Relaxed);
            eprintln!("[ssot_lab] paint_prepared skipped a panel frame: {e}");
        }
    }
}

// ============================================================================
// eframe app — pan animation edits two x-linked Chart pairs every frame.
// ============================================================================

struct LabApp {
    initialized: bool,
    render_state: Option<egui_wgpu::RenderState>,
    animate: bool,
    anim_speed: f32,
    window_frac: f32,
    /// Pan direction per x-linked column pair: +1.0 → forward, -1.0 → reverse.
    dir_left: f32,
    dir_right: f32,
    point_count: u32,
    /// Density-slider ceiling — set at init from the actual pool size.
    max_points: u32,
}

impl Default for LabApp {
    fn default() -> Self {
        Self {
            initialized: false,
            render_state: None,
            animate: true,
            anim_speed: 0.6,
            window_frac: 0.55,
            dir_left: 1.0,
            dir_right: -1.0,
            point_count: 1_000_000,
            max_points: MAX_POINTS_PER_SERIES,
        }
    }
}

/// (draw style, title, series config) per panel — fixed, no dropdown.
fn panel_recipe(i: usize) -> (DrawStyle, &'static str, DataRenderType) {
    let line = |style, color, w| DataLineStyleConfig {
        line_style: style,
        line_color: color,
        line_width: w,
    };
    let scatter = |color: Color, size| DataScatterStyleConfig {
        point_color: color,
        point_shape: ScatterShape::CircleFilled,
        point_size: size,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
    };
    match i {
        0 => (
            DrawStyle::Precise,
            "Precise — dashed line (GPU arc scan)",
            DataRenderType::Line {
                line: line(LineStylePreset::Dash, Color::from_rgb8(100, 180, 255), 2.0),
            },
        ),
        1 => (
            DrawStyle::Sketch(SketchOptions::default()),
            "Sketch — arc-parameterized wobble",
            DataRenderType::Line {
                line: line(LineStylePreset::Solid, Color::from_rgb8(255, 150, 110), 2.0),
            },
        ),
        2 => (
            DrawStyle::Milkyway(MilkywayOptions::default()),
            "Milkyway — ribbon + indirect star pass",
            DataRenderType::Line {
                line: line(LineStylePreset::Solid, Color::from_rgb8(190, 150, 255), 2.0),
            },
        ),
        _ => (
            DrawStyle::Constellation(ConstellationOptions::default()),
            "Constellation — PSF stars",
            DataRenderType::ScatterLine {
                line: line(LineStylePreset::Solid, Color::from_rgb8(255, 214, 130), 1.5),
                scatter: scatter(Color::from_rgb8(255, 214, 130), 3.0),
            },
        ),
    }
}

impl LabApp {
    fn init_state(&mut self, render_state: &egui_wgpu::RenderState) -> Result<LabState, String> {
        let device = Arc::new(render_state.device.clone());
        let queue = Arc::new(render_state.queue.clone());
        // Ask for the target pool; `try_new` clamps it to what the device's
        // storage binding can address (the arc scan binds the whole pool as
        // one binding). Size the density ceiling from the GRANTED capacity
        // read back below — 5 columns × 4 bytes per point, ~10% slack for
        // allocator alignment.
        let mut renderer = Renderer::try_new(
            RendererDevice::new(device, queue),
            render_state.target_format,
            POOL_TARGET_BYTES,
        )
        .map_err(|e| format!("Renderer init failed: {e}"))?;
        let pool_capacity = renderer.pool().capacity();
        self.max_points = MAX_POINTS_PER_SERIES.min(((pool_capacity / 20) as f64 * 0.9) as u32);
        self.point_count = self.point_count.min(self.max_points);

        let n = self.point_count as usize;
        let xs = lab_xs(n);
        renderer
            .add_column(X_ID, &col_f64(xs.clone()))
            .map_err(|e| format!("add_column x failed: {e}"))?;
        for (i, y_id) in Y_IDS.iter().enumerate() {
            renderer
                .add_column(*y_id, &col_f64(lab_ys(i, 0, &xs)))
                .map_err(|e| format!("add_column {y_id} failed: {e}"))?;
        }

        let placeholder = Rect {
            x: 0,
            y: 0,
            width: 600,
            height: 350,
        };
        let mut panels = Vec::with_capacity(4);
        for i in 0..4 {
            let (draw_style, title, render_type) = panel_recipe(i);
            let mut config = dark_config();
            config.chart_area = ChartArea(placeholder);
            config.draw_style = draw_style;
            let mut chart = Chart::new(config)
                .with_title(title)
                .with_x_title("x [rad]")
                .with_y_title("y");
            chart.set_x_range(0.0, X_SPAN);
            let _ = chart.auto_fit_y(renderer.pool(), Y_IDS[i], 0.10);
            let view = renderer
                .create_chart_view(&chart, placeholder)
                .map_err(|e| format!("create_chart_view failed: {e}"))?;
            let series = SeriesConfig {
                series_id: SERIES_IDS[i].into(),
                label: None,
                source_id: None,
                x_column: X_ID.into(),
                y_column: Y_IDS[i].into(),
                render_type,
            };
            let style = renderer.create_style_for_series(&series);
            panels.push(PanelEntry {
                chart,
                view,
                series,
                style,
                prepared: None,
            });
        }

        Ok(LabState {
            renderer,
            panels,
            frames_skipped: AtomicU32::new(0),
            last_error: None,
            data_variant: 0,
            points_per_series: n,
        })
    }

    fn cleanup(&mut self) {
        let Some(render_state) = self.render_state.take() else {
            self.initialized = false;
            return;
        };
        {
            let mut guard = render_state.renderer.write();
            if let Some(mut state) = guard.callback_resources.remove::<LabState>() {
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

impl Drop for LabApp {
    fn drop(&mut self) {
        self.cleanup();
    }
}

impl eframe::App for LabApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = EGUI_BG;
        visuals.window_fill = EGUI_BG;
        ctx.set_visuals(visuals);

        let Some(render_state) = frame.wgpu_render_state() else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("No eframe wgpu_render_state — wgpu backend must be enabled.");
            });
            return;
        };
        self.render_state = Some(render_state.clone());

        if !self.initialized {
            match self.init_state(render_state) {
                Ok(state) => {
                    render_state
                        .renderer
                        .write()
                        .callback_resources
                        .insert(state);
                    self.initialized = true;
                }
                Err(msg) => {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.label(msg);
                    });
                    return;
                }
            }
        }

        let time = ctx.input(|i| i.time);
        let pixels_per_point = ctx.pixels_per_point();

        // -- Sidebar + per-frame SSoT edits (x-linked pan). ------------------
        {
            let mut guard = render_state.renderer.write();
            let Some(state) = guard.callback_resources.get_mut::<LabState>() else {
                self.initialized = false;
                return;
            };
            let _ = state
                .renderer
                .ensure_target_format(render_state.target_format);

            // Pan animation: ONE x-range edit per column pair — the top and
            // bottom chart of each column share it (x-linked axes).
            if self.animate {
                let w = X_SPAN * self.window_frac as f64;
                let free = X_SPAN - w;
                let center = |dir: f32, phase_off: f64| {
                    free / 2.0
                        + (time * self.anim_speed as f64 * dir as f64 + phase_off).sin() * free
                            / 2.0
                        + w / 2.0
                };
                let c_left = center(self.dir_left, 0.0);
                let c_right = center(self.dir_right, 1.7);
                for (idx, c) in [(0, c_left), (2, c_left), (1, c_right), (3, c_right)] {
                    state.panels[idx]
                        .chart
                        .set_x_range(c - w / 2.0, c + w / 2.0);
                }
            }

            egui::SidePanel::left("controls")
                .exact_width(300.0)
                .show(ctx, |ui| {
                    ui.add_space(6.0);
                    ui.heading("SSoT editing");
                    ui.label(
                        "Four fixed styles, one shared x column. Vertical pairs are x-linked.",
                    );
                    ui.separator();

                    ui.label("Pan (Chart.set_x_range — two edits/frame, four charts)");
                    ui.checkbox(&mut self.animate, "Pan animation");
                    ui.add(egui::Slider::new(&mut self.anim_speed, 0.05..=6.0).text("pan speed"));
                    ui.add(
                        egui::Slider::new(&mut self.window_frac, 0.15..=1.0).text("window width"),
                    );
                    ui.horizontal(|ui| {
                        ui.label("left pair");
                        ui.selectable_value(&mut self.dir_left, 1.0, "forward");
                        ui.selectable_value(&mut self.dir_left, -1.0, "reverse");
                    });
                    ui.horizontal(|ui| {
                        ui.label("right pair");
                        ui.selectable_value(&mut self.dir_right, 1.0, "forward");
                        ui.selectable_value(&mut self.dir_right, -1.0, "reverse");
                    });

                    ui.separator();
                    ui.label("Data (1 shared x + 4 y pool columns)");
                    // Regenerate on release, not per drag tick — generating
                    // and uploading 25M floats every tick would freeze the UI.
                    let density = ui.add(
                        egui::Slider::new(&mut self.point_count, 10..=self.max_points)
                            .logarithmic(true)
                            .text("points / series"),
                    );
                    if density.drag_stopped() || (density.changed() && !density.dragged()) {
                        state.regen_data(self.point_count as usize, false);
                    }
                    if ui
                        .button("Regenerate data (remove_column + add_column)")
                        .clicked()
                    {
                        state.regen_data(self.point_count as usize, true);
                    }

                    ui.separator();
                    ui.label("Status");
                    // App-level FPS from egui's smoothed frame delta — the
                    // number that matches what the eye sees (the old counter
                    // summed all four panels' paints per app frame).
                    let fps = 1.0 / ctx.input(|i| i.stable_dt).max(1e-6);
                    let skipped = state.frames_skipped.load(Ordering::Relaxed);
                    let per_series = state.points_per_series as u64;
                    ui.monospace(format!("fps              {fps:.0}"));
                    ui.monospace(format!("frames skipped   {skipped}"));
                    ui.monospace(format!("points / series  {}", thousands(per_series)));
                    ui.monospace(format!("series points    {}", thousands(per_series * 4)));
                    ui.monospace(format!(
                        "pool used        {:.1} MB",
                        state.renderer.pool().used_bytes() as f64 / (1024.0 * 1024.0)
                    ));
                    ui.monospace(format!(
                        "pool generation  {}",
                        state.renderer.pool().generation()
                    ));
                    if let Some(e) = &state.last_error {
                        ui.colored_label(egui::Color32::from_rgb(240, 120, 120), e);
                    }
                });
        }

        // -- 2×2 chart grid: register one split-API callback per panel. ------
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(EGUI_BG))
            .show(ctx, |ui| {
                let row_h = ui.available_height() / 2.0;
                ui.columns(2, |cols| {
                    for (col_idx, col_ui) in cols.iter_mut().enumerate() {
                        for row_idx in 0..2 {
                            let panel_idx = row_idx * 2 + col_idx; // 0,1 top / 2,3 bottom
                            let size = egui::vec2(col_ui.available_width(), row_h);
                            let (rect, _resp) =
                                col_ui.allocate_exact_size(size, egui::Sense::hover());
                            let panel_rect_px = Rect {
                                x: (rect.min.x * pixels_per_point).round().max(0.0) as u32,
                                y: (rect.min.y * pixels_per_point).round().max(0.0) as u32,
                                width: (rect.width() * pixels_per_point).max(1.0) as u32,
                                height: (rect.height() * pixels_per_point).max(1.0) as u32,
                            };
                            col_ui
                                .painter()
                                .add(egui_wgpu::Callback::new_paint_callback(
                                    rect,
                                    LabCallback {
                                        panel_idx,
                                        panel_rect_px,
                                    },
                                ));
                        }
                    }
                });
            });

        if self.animate {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self) {
        self.cleanup();
    }
}

fn main() -> eframe::Result<()> {
    // Ask the adapter for its REAL storage-binding/buffer ceilings instead
    // of wgpu's 128 MB default — the multi-million-point pool must stay
    // bindable by the arc-scan compute pass.
    let mut wgpu_options = egui_wgpu::WgpuConfiguration::default();
    if let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.device_descriptor = Arc::new(|adapter| {
            let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
                wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                wgpu::Limits::default()
            };
            let adapter_limits = adapter.limits();
            wgpu::DeviceDescriptor {
                label: Some("egui wgpu device (ssot_lab)"),
                required_limits: wgpu::Limits {
                    max_texture_dimension_2d: 8192,
                    max_storage_buffer_binding_size: adapter_limits.max_storage_buffer_binding_size,
                    max_buffer_size: adapter_limits.max_buffer_size,
                    ..base_limits
                },
                ..Default::default()
            }
        });
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 900.0])
            .with_title("figgy SSoT lab — prepare/paint_prepared"),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        ..Default::default()
    };
    eframe::run_native(
        "figgy ssot lab",
        options,
        Box::new(|_cc| Ok(Box::new(LabApp::default()))),
    )
}

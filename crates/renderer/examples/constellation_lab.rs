//! Constellation parameter lab — live sliders over one figgy panel.
//!
//! Every `ConstellationOptions` field is bound to an egui slider; moving one
//! rewrites `config.draw_style`, which flips the chart's dirty bits, which
//! rewrites the Transform uniform on the next frame — the GPU picks the new
//! parameters up immediately (star/ribbon attributes are derived in-shader).
//!
//! Run with:
//! `cargo run -p renderer --example constellation_lab --features egui_demo`

use std::sync::{Arc, Mutex, PoisonError};

use eframe::egui_wgpu::{self, CallbackTrait};
use eframe::wgpu;

use renderer::color::Color;
use renderer::config::{ConstellationOptions, DrawStyle};
use renderer::data::Column;
use renderer::data_config::{DataLineStyleConfig, DataRenderType, SeriesConfig};
use renderer::default;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{Chart, ChartDrawItem, ChartStyle, ChartView, Renderer, RendererDevice, Series};

const POOL_CAPACITY: u64 = 8 * 1024 * 1024;
/// Deep-space backdrop behind the additive star field.
const SPACE_BG: egui::Color32 = egui::Color32::from_rgb(11, 15, 23);

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

/// All figgy state, stored in `CallbackResources` as `Mutex<FiggyState>` —
/// the same host-responsibility locking pattern as `egui_embed`.
struct FiggyState {
    renderer: Renderer,
    chart: Chart,
    view: ChartView,
    series: Vec<SeriesConfig>,
    styles: Vec<ChartStyle>,
}

impl FiggyState {
    fn shutdown(&mut self) {
        self.renderer.wait_idle();
    }
}

impl Drop for FiggyState {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn build_state(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>, format: wgpu::TextureFormat) -> Result<FiggyState, String> {
    let mut renderer = Renderer::try_new(
        RendererDevice::new(device, queue),
        format,
        POOL_CAPACITY,
    )
    .map_err(|e| format!("Renderer init failed: {e}"))?;

    // Two smooth curves (same data as the headless demo).
    let n = 90;
    let ts: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    let warm: Vec<f64> = ts.iter().map(|&t| 62.0 + 26.0 * (t * 5.2).sin()).collect();
    let cool: Vec<f64> = ts
        .iter()
        .map(|&t| 14.0 + 30.0 * t + 9.0 * (t * 8.0 + 1.0).cos())
        .collect();
    renderer.add_column("t", &col_f64(ts)).map_err(|e| e.to_string())?;
    renderer.add_column("warm", &col_f64(warm)).map_err(|e| e.to_string())?;
    renderer.add_column("cool", &col_f64(cool)).map_err(|e| e.to_string())?;

    let line = |id: &str, y: &str, color: Color| SeriesConfig {
        series_id: id.into(),
        label: None,
        x_column: "t".into(),
        y_column: y.into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: color,
                line_width: 2.0,
            },
        },
    };
    let series = vec![
        line("nebula_warm", "warm", Color::from_rgb8(255, 142, 92)),
        line("nebula_cool", "cool", Color::from_rgb8(96, 168, 255)),
    ];
    let styles: Vec<ChartStyle> =
        series.iter().map(|cfg| renderer.create_style_for_series(cfg)).collect();

    let mut config = default::default_config();
    let placeholder = Rect { x: 0, y: 0, width: 800, height: 600 };
    config.chart_area = ChartArea(placeholder);
    config.draw_style = DrawStyle::Constellation(ConstellationOptions::default());
    // Light chrome on the dark backdrop; no grid; no legend box.
    let chrome = Color::from_rgb8(186, 194, 210);
    for axis in [
        &mut config.top_x, &mut config.bottom_x, &mut config.left_y, &mut config.right_y,
    ] {
        axis.line_color = chrome;
        axis.label_style.color = chrome;
        axis.title_option.text.color = chrome;
    }
    config.chart_title.text.color = Color::from_rgb8(214, 220, 232);
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    config.legend.visible = false;

    let mut chart = Chart::new(config)
        .with_title("Constellation lab")
        .with_x_title("t")
        .with_y_title("value");
    chart.set_x_range(-0.03, 1.03);
    chart.set_y_range(0.0, 100.0);

    let view = renderer
        .create_chart_view(&chart, placeholder)
        .map_err(|e| format!("create_chart_view failed: {e}"))?;

    Ok(FiggyState { renderer, chart, view, series, styles })
}

/// Per-frame callback: carries the panel rect and the CURRENT slider values
/// (a fresh callback is built every frame, so the options just ride along).
struct LabCallback {
    panel_rect_px: Rect,
    opts: ConstellationOptions,
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
        let Some(state) = callback_resources.get_mut::<Mutex<FiggyState>>() else {
            return Vec::new();
        };
        let state = state.get_mut().unwrap_or_else(PoisonError::into_inner);

        // Slider → SSoT. Only on change, so the dirty bits don't spin.
        let wanted = DrawStyle::Constellation(self.opts);
        if state.chart.config().draw_style != wanted {
            state.chart.config_mut().draw_style = wanted;
        }

        let cur_rect = state.view.panel_rect();
        if cur_rect != self.panel_rect_px {
            state.chart.config_mut().chart_area = ChartArea(self.panel_rect_px);
            if let Err(e) =
                state.renderer.refresh_axis(&mut state.view, &state.chart, self.panel_rect_px)
            {
                eprintln!("[lab] refresh_axis failed: {e}");
                return Vec::new();
            }
            let _ = state.chart.consume_data_dirty();
            let _ = state.chart.consume_raster_dirty();
        } else if state.chart.consume_raster_dirty() {
            if let Err(e) = state.renderer.refresh_axis(&mut state.view, &state.chart, cur_rect) {
                eprintln!("[lab] refresh_axis failed: {e}");
                return Vec::new();
            }
            let _ = state.chart.consume_data_dirty();
        } else if state.chart.consume_data_dirty() {
            state.renderer.update_transform(&state.view, &state.chart);
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(state) = callback_resources.get::<Mutex<FiggyState>>() else { return };
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        let state = &mut *state;
        let (renderer, chart, view) = (&mut state.renderer, &state.chart, &state.view);
        let series: Vec<Series<'_>> = state
            .series
            .iter()
            .zip(state.styles.iter())
            .map(|(cfg, style)| Series { config: cfg, style })
            .collect();
        let items = [ChartDrawItem { view, chart_config: chart.config(), series: &series }];
        let target_size = (info.screen_size_px[0], info.screen_size_px[1]);
        if let Err(e) = renderer.paint(render_pass, target_size, &items) {
            eprintln!("[lab] paint failed: {e}");
        }
    }
}

struct LabApp {
    initialized: bool,
    failed: Option<String>,
    opts: ConstellationOptions,
    render_state: Option<egui_wgpu::RenderState>,
}

impl Default for LabApp {
    fn default() -> Self {
        Self {
            initialized: false,
            failed: None,
            opts: ConstellationOptions::default(),
            render_state: None,
        }
    }
}

impl LabApp {
    fn cleanup(&mut self) {
        let Some(render_state) = self.render_state.take() else { return };
        {
            let mut guard = render_state.renderer.write();
            if let Some(state) = guard.callback_resources.remove::<Mutex<FiggyState>>() {
                state.into_inner().unwrap_or_else(PoisonError::into_inner).shutdown();
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
        ctx.set_visuals(egui::Visuals::dark());

        let Some(render_state) = frame.wgpu_render_state() else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("No wgpu render state — the wgpu backend must be enabled.");
            });
            return;
        };
        self.render_state = Some(render_state.clone());

        if !self.initialized && self.failed.is_none() {
            let device = Arc::new(render_state.device.clone());
            let queue = Arc::new(render_state.queue.clone());
            match build_state(device, queue, render_state.target_format) {
                Ok(state) => {
                    render_state
                        .renderer
                        .write()
                        .callback_resources
                        .insert(Mutex::new(state));
                    self.initialized = true;
                }
                Err(e) => self.failed = Some(e),
            }
        }
        if let Some(msg) = &self.failed {
            let msg = msg.clone();
            egui::CentralPanel::default().show(ctx, |ui| { ui.label(msg); });
            return;
        }

        let pixels_per_point = ctx.pixels_per_point();

        egui::SidePanel::left("controls").min_width(250.0).show(ctx, |ui| {
            ui.heading("Constellation");
            ui.add_space(8.0);
            let o = &mut self.opts;
            ui.add(egui::Slider::new(&mut o.star_density, 0.0..=60.0).text("star_density"));
            ui.add(egui::Slider::new(&mut o.faint_bias, 0.5..=10.0).text("faint_bias"));
            ui.add(egui::Slider::new(&mut o.star_scale, 0.3..=3.0).text("star_scale"));
            ui.add(egui::Slider::new(&mut o.spread_px, 0.0..=10.0).text("spread_px"));
            ui.separator();
            ui.add(egui::Slider::new(&mut o.ribbon_width_px, 2.0..=40.0).text("ribbon_width_px"));
            ui.add(egui::Slider::new(&mut o.ribbon_intensity, 0.0..=1.0).text("ribbon_intensity"));
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("seed");
                ui.add(egui::DragValue::new(&mut o.seed).speed(1));
            });
            ui.add_space(8.0);
            if ui.button("Reset to defaults").clicked() {
                *o = ConstellationOptions::default();
            }
            ui.add_space(12.0);
            ui.label(
                "Every slider writes config.draw_style; star/ribbon\n\
                 attributes re-derive in-shader the same frame.",
            );
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(SPACE_BG))
            .show(ctx, |ui| {
                let avail = ui.available_size();
                let (rect, _resp) = ui.allocate_exact_size(avail, egui::Sense::hover());
                let panel_rect_px = Rect {
                    x: (rect.min.x * pixels_per_point).round().max(0.0) as u32,
                    y: (rect.min.y * pixels_per_point).round().max(0.0) as u32,
                    width: (rect.width() * pixels_per_point).max(1.0) as u32,
                    height: (rect.height() * pixels_per_point).max(1.0) as u32,
                };
                let cb = egui_wgpu::Callback::new_paint_callback(
                    rect,
                    LabCallback { panel_rect_px, opts: self.opts },
                );
                ui.painter().add(cb);
            });

        // Keep redrawing so slider drags feel live even without other input.
        ctx.request_repaint();
    }

    fn on_exit(&mut self) {
        self.cleanup();
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 760.0])
            .with_title("figgy constellation lab"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "figgy constellation lab",
        options,
        Box::new(|_cc| Ok(Box::new(LabApp::default()))),
    )
}

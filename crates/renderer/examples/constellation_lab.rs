//! Sparse constellation lab — live sliders over one figgy panel.
//!
//! Every `ConstellationOptions` field is bound to an egui slider; moving one
//! rewrites `config.draw_style`, and `Renderer::prepare` (run each frame in
//! the callback's prepare phase) rewrites the Transform uniform from the
//! current config — the GPU picks the new parameters up immediately
//! (star/line attributes are derived in-shader).
//!
//! Run with:
//! `cargo run -p renderer --example constellation_lab --features egui_demo`

use std::sync::Arc;

use eframe::egui_wgpu::{self, CallbackTrait};
use eframe::wgpu;

use renderer::color::Color;
use renderer::config::{ConstellationOptions, DrawStyle};
use renderer::data::Column;
use renderer::data_config::{
    DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, ScatterShape, SeriesConfig,
};
use renderer::default;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, ChartDrawItem, ChartStyle, ChartView, PreparedFrame, Renderer, RendererDevice, Series,
};

const POOL_CAPACITY: u64 = 8 * 1024 * 1024;
/// Dark host backdrop behind the translucent line + PSF stars.
const SPACE_BG: egui::Color32 = egui::Color32::from_rgb(11, 15, 23);

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

/// All figgy state, stored directly in `CallbackResources` — no `Mutex`.
///
/// The split frame API makes the lock unnecessary: the `&mut` half
/// (`Renderer::prepare`) runs in `CallbackTrait::prepare`, which hands out
/// `&mut CallbackResources`, and stores its token in `prepared`; the
/// immutable half (`Renderer::paint_prepared`) takes `&self`, so
/// `CallbackTrait::paint`'s `&CallbackResources` is enough.
struct FiggyState {
    renderer: Renderer,
    chart: Chart,
    view: ChartView,
    series: Vec<SeriesConfig>,
    styles: Vec<ChartStyle>,
    /// Frame token from `Renderer::prepare`, consumed by `paint_prepared`.
    /// `None` until the first prepare, or after a failed one.
    prepared: Option<PreparedFrame>,
}

/// Series bundles for the draw items — shared by BOTH callback halves so the
/// items passed to `Renderer::prepare` and `Renderer::paint_prepared` are
/// identical and in the same order (a `paint_prepared` contract).
fn build_series<'a>(series: &'a [SeriesConfig], styles: &'a [ChartStyle]) -> Vec<Series<'a>> {
    series
        .iter()
        .zip(styles.iter())
        .map(|(cfg, style)| Series { config: cfg, style })
        .collect()
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

fn build_state(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    format: wgpu::TextureFormat,
) -> Result<FiggyState, String> {
    let mut renderer = Renderer::try_new(RendererDevice::new(device, queue), format, POOL_CAPACITY)
        .map_err(|e| format!("Renderer init failed: {e}"))?;

    // Sparse constellation data: the intended use case is 5-10 source points,
    // not a densely sampled curve.
    let ts = vec![0.04, 0.16, 0.31, 0.44, 0.58, 0.70, 0.84, 0.96];
    let warm = vec![62.0, 77.0, 68.0, 88.0, 74.0, 92.0, 79.0, 86.0];
    let cool = vec![24.0, 39.0, 31.0, 48.0, 43.0, 60.0, 52.0, 67.0];
    renderer
        .add_column("sample_x", &col_f64(ts.clone()))
        .map_err(|e| e.to_string())?;
    renderer
        .add_column("warm", &col_f64(warm))
        .map_err(|e| e.to_string())?;
    renderer
        .add_column("cool", &col_f64(cool.clone()))
        .map_err(|e| e.to_string())?;

    let scatter_line = |id: &str, y: &str, color: Color, point_size: f32| SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "sample_x".into(),
        y_column: y.into(),
        render_type: DataRenderType::ScatterLine {
            scatter: DataScatterStyleConfig {
                point_color: color,
                point_shape: ScatterShape::CircleFilled,
                point_size,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
            },
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: color,
                line_width: 2.0,
            },
        },
    };

    let series = vec![
        scatter_line("sparse_warm", "warm", Color::from_rgb8(255, 142, 92), 6.0),
        scatter_line("sparse_cool", "cool", Color::from_rgb8(96, 168, 255), 6.0),
    ];
    let styles: Vec<ChartStyle> = series
        .iter()
        .map(|cfg| renderer.create_style_for_series(cfg))
        .collect();

    let mut config = default::default_config();
    let placeholder = Rect {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    };
    config.chart_area = ChartArea(placeholder);
    config.draw_style = DrawStyle::Constellation(ConstellationOptions::default());
    // Light chrome on the dark backdrop; no grid; no legend box.
    let chrome = Color::from_rgb8(186, 194, 210);
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
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
        .with_title("Sparse constellation")
        .with_x_title("8 source points")
        .with_y_title("value");
    chart.set_x_range(-0.03, 1.03);
    chart.set_y_range(0.0, 100.0);

    let view = renderer
        .create_chart_view(&chart, placeholder)
        .map_err(|e| format!("create_chart_view failed: {e}"))?;

    Ok(FiggyState {
        renderer,
        chart,
        view,
        series,
        styles,
        prepared: None,
    })
}

/// Per-frame callback: carries the panel rect and the CURRENT slider values
/// (a fresh callback is built every frame, so the options just ride along).
struct LabCallback {
    panel_rect_px: Rect,
    opts: ConstellationOptions,
    /// Base ScatterLine point_size — a PER-SERIES SSoT field
    /// (DataScatterStyleConfig.point_size), not a style option.
    point_size: f32,
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
        let Some(state) = callback_resources.get_mut::<FiggyState>() else {
            return Vec::new();
        };

        // Slider → SSoT. Only on change, so the dirty bits don't spin.
        // `config_mut` flags BOTH dirty bits, but constellation knobs here are
        // GPU-side uniform parameters. No CPU backdrop/glow is tied to this
        // lab style, so cancel the raster bit and let `Renderer::prepare`
        // below carry it (it rewrites the transform uniform every frame).
        let wanted = DrawStyle::Constellation(self.opts);
        if state.chart.config().draw_style != wanted {
            state.chart.config_mut().draw_style = wanted;
            let _ = state.chart.consume_raster_dirty();
        }

        for idx in 0..state.series.len() {
            let mut changed = false;
            if let DataRenderType::ScatterLine { scatter, .. } = &mut state.series[idx].render_type
            {
                if (scatter.point_size - self.point_size).abs() > f32::EPSILON {
                    scatter.point_size = self.point_size;
                    changed = true;
                }
            }
            if changed {
                state.styles[idx] = state.renderer.create_style_for_series(&state.series[idx]);
            }
        }

        let cur_rect = state.view.panel_rect();
        if cur_rect != self.panel_rect_px {
            state.chart.config_mut().chart_area = ChartArea(self.panel_rect_px);
            if let Err(e) =
                state
                    .renderer
                    .refresh_axis(&mut state.view, &state.chart, self.panel_rect_px)
            {
                eprintln!("[lab] refresh_axis failed: {e}");
                state.prepared = None;
                return Vec::new();
            }
            let _ = state.chart.consume_data_dirty();
            let _ = state.chart.consume_raster_dirty();
        } else {
            let raster_dirty = state.chart.consume_raster_dirty();
            // The transform (data) dirty bit needs no explicit handling:
            // `Renderer::prepare` below rewrites the transform uniform from
            // the current config every frame.
            let _ = state.chart.consume_data_dirty();
            if raster_dirty {
                if let Err(e) = state
                    .renderer
                    .refresh_axis(&mut state.view, &state.chart, cur_rect)
                {
                    eprintln!("[lab] refresh_axis failed: {e}");
                    state.prepared = None;
                    return Vec::new();
                }
            }
        }

        // Mutable half of the frame: compiles missing styled pipelines,
        // writes the panel's transform uniform, dispatches arc-length
        // compute. The token is stored for `paint` to consume; the items
        // must be rebuilt there identically (same helper, same order).
        let prepared = {
            let series = build_series(&state.series, &state.styles);
            let items = [ChartDrawItem {
                view: &state.view,
                chart_config: state.chart.config(),
                series: &series,
            }];
            state.renderer.prepare(&items)
        };
        state.prepared = match prepared {
            Ok(token) => Some(token),
            Err(e) => {
                eprintln!("[lab] prepare failed: {e}");
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
        let Some(state) = callback_resources.get::<FiggyState>() else {
            return;
        };
        let Some(prepared) = state.prepared.as_ref() else {
            eprintln!("[lab] paint skipped: no prepared frame");
            return;
        };
        // Same items, in the same order, as at prepare — a `paint_prepared`
        // contract, enforced by the shared `build_series` helper.
        let series = build_series(&state.series, &state.styles);
        let items = [ChartDrawItem {
            view: &state.view,
            chart_config: state.chart.config(),
            series: &series,
        }];
        let target_size = (info.screen_size_px[0], info.screen_size_px[1]);
        if let Err(e) = state
            .renderer
            .paint_prepared(render_pass, target_size, &items, prepared)
        {
            // StalePreparedFrame and friends: skip this frame; the next
            // callback prepare rebuilds the token.
            eprintln!("[lab] paint skipped: {e}");
        }
    }
}

struct LabApp {
    initialized: bool,
    failed: Option<String>,
    opts: ConstellationOptions,
    point_size: f32,
    render_state: Option<egui_wgpu::RenderState>,
}

impl Default for LabApp {
    fn default() -> Self {
        Self {
            initialized: false,
            failed: None,
            opts: ConstellationOptions::default(),
            point_size: 6.0,
            render_state: None,
        }
    }
}

impl LabApp {
    fn cleanup(&mut self) {
        let Some(render_state) = self.render_state.take() else {
            return;
        };
        {
            let mut guard = render_state.renderer.write();
            if let Some(mut state) = guard.callback_resources.remove::<FiggyState>() {
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
                        .insert(state);
                    self.initialized = true;
                }
                Err(e) => self.failed = Some(e),
            }
        }
        if let Some(msg) = &self.failed {
            let msg = msg.clone();
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label(msg);
            });
            return;
        }

        let pixels_per_point = ctx.pixels_per_point();

        egui::SidePanel::left("controls")
            .min_width(250.0)
            .show(ctx, |ui| {
                ui.heading("Sparse constellation");
                ui.add_space(8.0);
                // Sliders are GENERATED from the SSoT's parameter metadata —
                // ranges live in exactly one place (model PARAM_SPECS), shared
                // with the wasm `draw_style_param_specs` export.
                let o = &mut self.opts;
                for spec in ConstellationOptions::PARAM_SPECS {
                    if spec.integer {
                        ui.label(format!("(unbound integer spec: {})", spec.key));
                        continue;
                    }
                    let field: &mut f32 = match spec.key {
                        "star_opacity" => &mut o.star_opacity,
                        "line_opacity" => &mut o.line_opacity,
                        other => {
                            ui.label(format!("(unbound spec: {other})"));
                            continue;
                        }
                    };
                    ui.add(
                        egui::Slider::new(field, spec.min as f32..=spec.max as f32).text(spec.key),
                    );
                }
                ui.separator();
                // Per-series SSoT, not a style option.
                ui.label("series: ScatterLine");
                ui.add(egui::Slider::new(&mut self.point_size, 1.0..=18.0).text("point_size"));
                ui.add_space(8.0);
                if ui.button("Reset to defaults").clicked() {
                    *o = ConstellationOptions::default();
                    self.point_size = 6.0;
                }
                ui.add_space(12.0);
                ui.label(
                    "Every slider writes config.draw_style; star/line\n\
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
                    LabCallback {
                        panel_rect_px,
                        opts: self.opts,
                        point_size: self.point_size,
                    },
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
            .with_title("figgy sparse constellation lab"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "figgy sparse constellation lab",
        options,
        Box::new(|_cc| Ok(Box::new(LabApp::default()))),
    )
}

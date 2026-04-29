//! figgy + iced 0.14 GPU integration demo — uses `shader::Primitive` to put
//! figgy::Renderer on top of the wgpu 27 device/queue iced provides.
//!
//! Layout:
//!
//! - `FiggyPipeline` (impls `iced_wgpu::primitive::Pipeline`) — built once
//!   the first time iced sees it. Owns figgy::Renderer + 3 panels.
//! - `FiggyPrimitive` (impls `shader::Primitive`) — per-frame instance
//!   carrying which panel to draw + panel rect.
//!
//! Run with:
//! `cargo run --example iced_embed --features iced_demo`

use std::sync::Arc;
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
use iced_wgpu::graphics::Viewport;
use iced_wgpu::wgpu;

use figgy::color::Color;
use figgy::config::{AxisScale, LegendEntryKind};
use figgy::data::Column;
use figgy::default;
use figgy::demo;
use figgy::layout::{ChartArea, Rect};
use figgy::line::LineStylePreset;
use figgy::{
    dpi_to_scale, Chart, ChartDrawItem, ChartStyle, ChartView, DataLineStyleConfig,
    DataRenderType, Renderer, Series, SeriesConfig, MAX_EXPORT_SCALE, MIN_EXPORT_SCALE,
};

const POOL_CAPACITY: u64 = 16 * 1024 * 1024;
const N: usize = 1024;

fn col_f64(index: usize, data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { index, data, min, max }
}

// ============================================================================
// figgy state — built once, shared by every Primitive instance.
// ============================================================================

struct PanelEntry {
    chart: Chart,
    view: ChartView,
    series: Vec<SeriesConfig>,
    styles: Vec<ChartStyle>,
}

struct FiggyPipeline {
    renderer: Renderer,
    panels: Vec<PanelEntry>,
}

impl FiggyPipeline {
    fn build(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let device = Arc::new(device.clone());
        let queue = Arc::new(queue.clone());
        let mut renderer = Renderer::try_new(device, queue, format, POOL_CAPACITY)
            .expect("Renderer init");

        let placeholder = Rect { x: 0, y: 0, width: 480, height: 480 };
        let panels = vec![
            build_sine_panel(&mut renderer, placeholder),
            build_rc_panel(&mut renderer, placeholder),
            build_xs_panel(&mut renderer, placeholder),
        ];
        Self { renderer, panels }
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
    renderer.add_column("sine_x", &col_f64(0, xs)).unwrap();
    renderer.add_column("sine_y", &col_f64(1, ys)).unwrap();
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
        series_id: "sin".into(), label: None,
        x_column: "sine_x".into(), y_column: "sine_y".into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color, line_width: line_w,
            },
        },
    };
    let style = renderer.create_style_for_series(&cfg);
    PanelEntry { chart, view, series: vec![cfg], styles: vec![style] }
}

fn build_rc_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    let (ts, vs_charge) = demo::rc_data(N);
    let (_, vs_discharge) = demo::rc_discharge_data(N);
    renderer.add_column("rc_t", &col_f64(0, ts)).unwrap();
    renderer.add_column("rc_charge", &col_f64(1, vs_charge)).unwrap();
    renderer.add_column("rc_discharge", &col_f64(2, vs_discharge)).unwrap();
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
        .with_legend_entry("Discharging", discharge_color, line_w, LegendEntryKind::Line);
    chart.auto_fit_x(renderer.pool(), "rc_t", 0.02).unwrap();
    chart.auto_fit_y_union(renderer.pool(), &["rc_charge", "rc_discharge"], 0.05).unwrap();

    let view = renderer.create_chart_view(&chart, rect).unwrap();
    let mk = |id: &str, x: &str, y: &str, color: Color| SeriesConfig {
        series_id: id.into(), label: None,
        x_column: x.into(), y_column: y.into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: color, line_width: line_w,
            },
        },
    };
    let cfg_charge = mk("charge", "rc_t", "rc_charge", charge_color);
    let cfg_discharge = mk("discharge", "rc_t", "rc_discharge", discharge_color);
    let style_charge = renderer.create_style_for_series(&cfg_charge);
    let style_discharge = renderer.create_style_for_series(&cfg_discharge);
    PanelEntry {
        chart, view,
        series: vec![cfg_charge, cfg_discharge],
        styles: vec![style_charge, style_discharge],
    }
}

fn build_xs_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    let (es, sigmas) = demo::cross_section_data(N);
    renderer.add_column("xs_e", &col_f64(0, es)).unwrap();
    renderer.add_column("xs_sigma", &col_f64(1, sigmas)).unwrap();
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
        series_id: "sigma".into(), label: None,
        x_column: "xs_e".into(), y_column: "xs_sigma".into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color, line_width: line_w,
            },
        },
    };
    let style = renderer.create_style_for_series(&cfg);
    PanelEntry { chart, view, series: vec![cfg], styles: vec![style] }
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
        _viewport: &Viewport,
    ) {
        let panel = &mut pipeline.panels[self.panel_idx];
        let cur = panel.view.panel_rect();
        if cur != self.panel_rect_px {
            panel.chart.config_mut().chart_area = ChartArea(self.panel_rect_px);
            pipeline
                .renderer
                .refresh_axis(&mut panel.view, &panel.chart, self.panel_rect_px)
                .expect("refresh_axis");
            let _ = panel.chart.consume_data_dirty();
            let _ = panel.chart.consume_raster_dirty();
        } else if panel.chart.consume_raster_dirty() {
            pipeline
                .renderer
                .refresh_axis(&mut panel.view, &panel.chart, cur)
                .expect("refresh_axis");
            let _ = panel.chart.consume_data_dirty();
        } else if panel.chart.consume_data_dirty() {
            pipeline.renderer.update_transform(&panel.view, &panel.chart);
        }

        // Export every panel separately, but only once — from panel_idx == 0's prepare.
        // The figgy API returns in-memory PNG bytes; the file write happens here.
        if self.panel_idx == 0 && EXPORT_REQUESTED.swap(false, Ordering::Relaxed) {
            let dpi = EXPORT_DPI.load(Ordering::Relaxed) as f32;
            let scale = dpi_to_scale(dpi);
            for (i, p) in pipeline.panels.iter().enumerate() {
                match pipeline.renderer.export_panel_png_bytes(
                    &p.chart, &p.series, scale,
                ) {
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
    }

    /// `draw` is the efficient path — slots into the RenderPass iced already built.
    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        let panel = &pipeline.panels[self.panel_idx];
        let series: Vec<Series<'_>> = panel.series.iter().zip(panel.styles.iter())
            .map(|(cfg, style)| Series { config: cfg, style })
            .collect();
        let items = [ChartDrawItem {
            view: &panel.view,
            chart_config: panel.chart.config(),
            series: &series,
        }];
        // iced has already set viewport / scissor to the Primitive bounds, but
        // figgy sets its own viewport / scissor based on panel_rect.
        // target_size is the surface pixel size — not exposed to Primitive,
        // so the bounding box of panel_rect_px is enough (clamp trims overflow).
        let target_size = (
            self.panel_rect_px.x + self.panel_rect_px.width,
            self.panel_rect_px.y + self.panel_rect_px.height,
        );
        pipeline
            .renderer
            .paint(render_pass, target_size, &items)
            .expect("paint");
        true
    }
}

// ============================================================================
// shader::Program — issues the Primitive.
// ============================================================================

struct ChartCanvas { panel_idx: usize }

impl<Message> shader::Program<Message> for ChartCanvas {
    type State = ();
    type Primitive = FiggyPrimitive;

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
        FiggyPrimitive { panel_idx: self.panel_idx, panel_rect_px }
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
        (App { dpi_text: "192".to_string() }, Task::none())
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
            ].spacing(12),
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

fn theme_fn(_state: &App) -> Theme { Theme::Light }

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("figgy iced demo")
        .theme(theme_fn)
        .window_size(iced::Size::new(1800.0, 500.0))
        .run()
}

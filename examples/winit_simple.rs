//! figgy + winit demo — 3 panels (sine / RC / cross-section).
//!
//! What this example demonstrates:
//!
//! 1. **Data → Column**: wrap two `Vec<f64>` from `figgy::demo` as
//!    `figgy::Column<f64>` (default `ColumnSource` impl).
//! 2. **One-line WindowedRenderer**: `Renderer::for_window(window, size,
//!    pool_capacity)` — figgy owns instance / adapter / device / queue /
//!    surface / swap chain. Zero wgpu setup code.
//! 3. **Register columns**: `renderer.add_column(id, &column)`.
//! 4. **Chart builder**: `default::default_config()` → `Chart::new(cfg)
//!    .with_title(...).with_x_title(...).with_y_title(...)` →
//!    `auto_fit_x/y` or `set_x_range/set_y_range`. Log scale is a one-liner
//!    via `config.left_y.scale = AxisScale::Logarithmic`.
//! 5. **ChartView / ChartStyle**: `renderer.create_chart_view(chart, rect)`,
//!    `create_style(...)`.
//! 6. **Draw**: a single `renderer.draw(clear, &items)` call — handles
//!    surface frame acquire → encoder → render pass → paint → submit →
//!    present internally.
//!
//! Controls:
//! - ESC: quit

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use figgy::color::Color;
use figgy::config::{AxisScale, LegendEntryKind};
use figgy::data::Column;
use figgy::default;
use figgy::demo;
use figgy::layout::{ChartArea, Rect};
use figgy::line::LineStylePreset;
use figgy::{
    encode_png, Chart, ChartDrawItem, ChartStyle, ChartView, DataLineStyleConfig, DataRenderType,
    Renderer, Series, SeriesConfig, WindowedRenderer,
};

const POOL_CAPACITY: u64 = 16 * 1024 * 1024;
const N: usize = 1024;
const GAP: u32 = 8;

// ============================================================================
// Vec<f64> → Column<f64>
// ============================================================================

fn col_f64(index: usize, data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { index, data, min, max }
}

// ============================================================================
// One panel bundle.
// ============================================================================

struct PanelEntry {
    chart: Chart,
    view: ChartView,
    /// Declarative series spec (source of truth for both paint and export).
    series: Vec<SeriesConfig>,
    /// 1:1 with `series` — pre-built GPU styles for paint. Export rebuilds its own.
    styles: Vec<ChartStyle>,
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<WindowedRenderer<'static>>,
    panels: Vec<PanelEntry>,
}

impl App {
    fn new() -> Self {
        Self { window: None, renderer: None, panels: Vec::new() }
    }

    fn shutdown_gpu(&mut self) {
        if let Some(renderer) = self.renderer.as_ref() {
            renderer.wait_idle();
        }
        self.panels.clear();
        self.renderer.take();
        self.window.take();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.shutdown_gpu();
    }
}

fn compute_panel_rects(w: u32, h: u32) -> [Rect; 3] {
    let avail_w = w.saturating_sub(GAP * 4);
    let pw = avail_w / 3;
    let ph = h.saturating_sub(GAP * 2);
    [
        Rect { x: GAP, y: GAP, width: pw, height: ph },
        Rect { x: GAP * 2 + pw, y: GAP, width: pw, height: ph },
        Rect { x: GAP * 3 + pw * 2, y: GAP, width: pw, height: ph },
    ]
}

// ============================================================================
// Chart setup — data (Vec) → Column → Chart builder → ChartView/Style.
// Same pattern when adding a new chart kind.
// ============================================================================

fn build_sine_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    let (xs, ys) = demo::sine_data(N);
    renderer.add_column("sine_x", &col_f64(0, xs)).expect("add x");
    renderer.add_column("sine_y", &col_f64(1, ys)).expect("add y");

    let mut config = default::default_config();
    config.chart_area = ChartArea(rect);
    // Chart 1: grid off.
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
    chart.auto_fit_x(renderer.pool(), "sine_x", 0.02).expect("fit x");
    chart.auto_fit_y(renderer.pool(), "sine_y", 0.10).expect("fit y");

    let view = renderer.create_chart_view(&chart, rect).expect("view");
    let cfg_sin = SeriesConfig {
        series_id: "sin".into(),
        label: None,
        x_column: "sine_x".into(),
        y_column: "sine_y".into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color, line_width: line_w,
            },
        },
    };
    let style = renderer.create_style_for_series(&cfg_sin);
    PanelEntry {
        chart, view,
        series: vec![cfg_sin],
        styles: vec![style],
    }
}

fn build_rc_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    // Charge + discharge — same t, two V columns.
    let (ts, vs_charge) = demo::rc_data(N);
    let (_, vs_discharge) = demo::rc_discharge_data(N);
    renderer.add_column("rc_t", &col_f64(0, ts)).expect("add t");
    renderer.add_column("rc_charge", &col_f64(1, vs_charge)).expect("add charge");
    renderer.add_column("rc_discharge", &col_f64(2, vs_discharge)).expect("add discharge");

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
        .with_legend_entry("Charging V0(1−e⁻ᵗ)", charge_color, line_w, LegendEntryKind::Line)
        .with_legend_entry("Discharging V0·e⁻ᵗ", discharge_color, line_w, LegendEntryKind::Line);
    chart.auto_fit_x(renderer.pool(), "rc_t", 0.02).expect("fit t");
    // Y range is the union of both series (both span 0..V0 here, but uses the unified API).
    chart.auto_fit_y_union(renderer.pool(), &["rc_charge", "rc_discharge"], 0.05)
        .expect("fit v");

    let view = renderer.create_chart_view(&chart, rect).expect("view");
    let mk = |id: &str, x: &str, y: &str, color: Color| SeriesConfig {
        series_id: id.into(),
        label: None,
        x_column: x.into(),
        y_column: y.into(),
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

fn build_cross_section_panel(renderer: &mut Renderer, rect: Rect) -> PanelEntry {
    let (es, sigmas) = demo::cross_section_data(N);
    renderer.add_column("xs_e", &col_f64(0, es)).expect("add E");
    renderer.add_column("xs_sigma", &col_f64(1, sigmas)).expect("add sigma");

    // Log scale is one line in Config. set_y_range / auto_fit_y automatically picks
    // (a) decade major spacing (b) Power label format (c) multiplicative padding.
    let mut config = default::default_config();
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

    let line_color = Color::from_rgb8(60, 160, 90);
    let line_w = 3.5;
    let mut chart = Chart::new(config)
        .with_title("Cross-section σ(E) — 3.5 px, log + minor grid")
        .with_x_title("E [keV]")
        .with_y_title("σ [barn]")
        .with_legend_entry("σ(E)", line_color, line_w, LegendEntryKind::Line);
    chart.auto_fit_x(renderer.pool(), "xs_e", 0.0).expect("fit E");
    chart.auto_fit_y(renderer.pool(), "xs_sigma", 0.10).expect("fit sigma");

    let view = renderer.create_chart_view(&chart, rect).expect("view");
    let cfg_xs = SeriesConfig {
        series_id: "sigma".into(),
        label: None,
        x_column: "xs_e".into(),
        y_column: "xs_sigma".into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color, line_width: line_w,
            },
        },
    };
    let style = renderer.create_style_for_series(&cfg_xs);
    PanelEntry {
        chart, view,
        series: vec![cfg_xs],
        styles: vec![style],
    }
}

// ============================================================================
// winit ApplicationHandler
// ============================================================================

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }

        let attrs = Window::default_attributes()
            .with_title("figgy demo (winit) — sine / RC / cross-section")
            .with_inner_size(LogicalSize::new(1800.0, 500.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        let size = window.inner_size();

        // One-line setup — figgy owns surface / device / queue / pipelines.
        let mut renderer = match Renderer::for_window(
            Arc::clone(&window),
            (size.width, size.height),
            POOL_CAPACITY,
        ) {
            Ok(renderer) => renderer,
            Err(e) => {
                eprintln!("[init] {e}");
                event_loop.exit();
                return;
            }
        };

        let rects = compute_panel_rects(size.width, size.height);
        let mut panels = vec![
            build_sine_panel(&mut renderer, rects[0]),
            build_rc_panel(&mut renderer, rects[1]),
            build_cross_section_panel(&mut renderer, rects[2]),
        ];
        for p in panels.iter_mut() {
            let _ = p.chart.consume_data_dirty();
            let _ = p.chart.consume_raster_dirty();
        }

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.panels = panels;
        if let Some(w) = self.window.as_ref() { w.request_redraw(); }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.shutdown_gpu();
                event_loop.exit();
            }
            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key, state: ElementState::Pressed, .. },
                ..
            } => match logical_key {
                Key::Named(NamedKey::Escape) => {
                    self.shutdown_gpu();
                    event_loop.exit();
                }
                Key::Character(s) if s.as_str() == "s" || s.as_str() == "S" => {
                    self.export_pngs();
                }
                _ => {}
            },
            WindowEvent::Resized(s) => {
                self.handle_resize(s.width, s.height);
                if let Some(w) = self.window.as_ref() { w.request_redraw(); }
            }
            WindowEvent::RedrawRequested => self.render_frame(),
            _ => {}
        }
    }
}

impl App {
    /// 'S' key — export each panel separately as in-memory PNG bytes (scale=2 = 2x DPI);
    /// the caller (this fn) writes the files. figgy itself only handles memory.
    fn export_pngs(&self) {
        const EXPORT_SCALE: f32 = 2.0;
        let Some(renderer) = self.renderer.as_ref() else { return; };
        for (i, panel) in self.panels.iter().enumerate() {
            match renderer.export_panel_png_bytes(&panel.chart, &panel.series, EXPORT_SCALE) {
                Ok(bytes) => {
                    let path = format!("/tmp/figgy_winit_panel_{i}.png");
                    if let Err(e) = std::fs::write(&path, &bytes) {
                        eprintln!("[export] write {path} failed: {e}");
                    } else {
                        eprintln!("[export] saved {path} ({} bytes)", bytes.len());
                    }
                }
                Err(e) => eprintln!("[export] panel {i} failed: {e}"),
            }
        }
        // encode_png can also be called on an export_panel_rgba result (e.g. RGBA-only for clipboard).
        let _ = encode_png;
    }

    fn handle_resize(&mut self, w: u32, h: u32) {
        let Some(renderer) = self.renderer.as_mut() else { return; };
        let w = w.max(1);
        let h = h.max(1);
        if let Err(e) = renderer.resize(w, h) {
            eprintln!("[resize] {e}");
            return;
        }

        let rects = compute_panel_rects(w, h);
        for (panel, rect) in self.panels.iter_mut().zip(rects.iter()) {
            panel.chart.config_mut().chart_area = ChartArea(*rect);
            if let Err(e) = renderer.refresh_axis(&mut panel.view, &panel.chart, *rect) {
                eprintln!("[refresh_axis] {e}");
                return;
            }
            let _ = panel.chart.consume_data_dirty();
            let _ = panel.chart.consume_raster_dirty();
        }
    }

    fn render_frame(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else { return; };
        if self.panels.is_empty() { return; }

        // ---- prepare: handle dirty flags ----
        for panel in self.panels.iter_mut() {
            let panel_rect = panel.view.panel_rect();
            if panel.chart.consume_raster_dirty() {
                if let Err(e) = renderer.refresh_axis(&mut panel.view, &panel.chart, panel_rect) {
                    eprintln!("[refresh_axis] {e}");
                    return;
                }
                let _ = panel.chart.consume_data_dirty();
            } else if panel.chart.consume_data_dirty() {
                renderer.update_transform(&panel.view, &panel.chart);
            }
        }

        // ---- Series + ChartDrawItem slices ----
        let series_per_panel: Vec<Vec<Series<'_>>> = self.panels.iter().map(|p| {
            p.series.iter().zip(p.styles.iter())
                .map(|(cfg, style)| Series { config: cfg, style })
                .collect()
        }).collect();
        let items: Vec<ChartDrawItem<'_>> = self.panels.iter().zip(series_per_panel.iter()).map(|(p, ss)| {
            ChartDrawItem {
                view: &p.view,
                chart_config: p.chart.config(),
                series: ss.as_slice(),
            }
        }).collect();

        // ---- single draw call — surface/encoder/pass/submit/present all internal. ----
        if let Err(e) = renderer.draw(Color::WHITE, &items) {
            eprintln!("[draw] {e}");
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event_loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("run_app");
}

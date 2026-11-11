//! figgy + winit demo — 3 panels (sine / RC / cross-section).
//!
//! What this example demonstrates:
//!
//! 1. **Data**: two `Vec<f64>` from `renderer::demo`, uploaded as they are
//!    (`ColumnSource` is implemented for `Vec<f64>`).
//! 2. **One-line WindowedRenderer**: `Renderer::for_window_async(window, size,
//!    pool_capacity)` — figgy owns instance / adapter / device / queue /
//!    surface / swap chain. Zero wgpu setup code.
//! 3. **Register columns**: `renderer.add_column(id, &values)`.
//! 4. **Chart builder**: `default::default_config()` → `Chart::new(cfg)
//!    .with_title(...).with_x_title(...).with_y_title(...)` →
//!    `auto_fit_x/y` or `set_x_range/set_y_range`. Log scale is a one-liner
//!    via `config.left_y.scale = AxisScale::Logarithmic`.
//! 5. **Series / ChartView / ChartStyle**: `SeriesConfig::line(...)` with
//!    `.label/.color/.width`, the legend from the series
//!    (`Chart::with_legend_from`), `renderer.create_chart_view(chart, rect)`,
//!    `create_style_for_series(...)`.
//! 6. **Draw**: a single `renderer.draw(clear, &mut items)` call — handles
//!    surface frame acquire → encoder → render pass → paint → submit →
//!    present internally.
//!
//! Controls:
//! - ESC: quit
//! - A: cycle axis frame presets (boxed-inward → boxed-outward → open →
//!   open-inward → minimal) across all panels
//! - C: cycle series color rotations (classic → vivid → balanced →
//!   colorblind-safe → monochrome)
//! - Left click: select the element under the cursor (axis, tick labels,
//!   axis title, chart title, legend, data area) — a blue box marks the
//!   selection. Click empty space to deselect.
//! - Drag a selected title / tick-label band / legend to reposition it.
//!   Dragging an axis detaches it: it moves only perpendicular to itself
//!   (y-axes horizontally, x-axes vertically) while the data area stays
//!   put — tick positions along the axis stay aligned with the data.
//!   The data area shows 8 resize handles when selected; drag the handles
//!   to resize it, or drag inside the area to move it (size preserved,
//!   ticks and data follow).

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use renderer::color::Color;
use renderer::config::AxisScale;
use renderer::data_config::{ContourLabelConfig, MatrixRef, Shading};
use renderer::default;
use renderer::demo;
use renderer::layout::NudgeResult;
use renderer::layout::Rect;
use renderer::line::LineStylePreset;
use renderer::{
    AxisPreset, Chart, ChartDrawItem, ChartStyle, ChartView, ColorCycle, ColumnSource,
    CpuTextMeasure, HitId, HitMap, Renderer, ResizeHandle, Series, SeriesConfig, WindowedRenderer,
};

const AXIS_PRESETS: [AxisPreset; 7] = [
    AxisPreset::BoxedInward,
    AxisPreset::BoxedOutward,
    AxisPreset::OpenOutward,
    AxisPreset::OpenInward,
    AxisPreset::Minimal,
    AxisPreset::Hidden,
    AxisPreset::Grotrian,
];

const COLOR_CYCLES: [ColorCycle; 5] = [
    ColorCycle::Classic,
    ColorCycle::Vivid,
    ColorCycle::Balanced,
    ColorCycle::ColorblindSafe,
    ColorCycle::Monochrome,
];

const POOL_CAPACITY: u64 = 16 * 1024 * 1024;
const N: usize = 1024;
const GAP: u32 = 8;

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
    /// Selectable elements of this panel (axes / titles / data area).
    hitmap: HitMap,
    /// Warnings from the last raster refresh, so only changes are logged.
    reported_warnings: Vec<renderer::RenderWarning>,
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<WindowedRenderer<'static>>,
    panels: Vec<PanelEntry>,
    /// Last cursor position in surface pixels (same space as chart_area).
    cursor: Option<(f32, f32)>,
    /// Currently selected element: (panel index, hit-map id).
    selected: Option<(usize, HitId)>,
    /// Drag in progress for the selected element (set on press over a
    /// draggable element, cleared on release).
    dragging: bool,
    /// Resize in progress via one of the selected element's handles.
    resizing: Option<ResizeHandle>,
    /// Cycling indices for the 'A' (axis preset) / 'C' (color cycle) keys.
    axis_preset_idx: usize,
    color_cycle_idx: usize,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            panels: Vec::new(),
            cursor: None,
            selected: None,
            dragging: false,
            resizing: None,
            axis_preset_idx: 0,
            color_cycle_idx: 0,
        }
    }

    fn shutdown_gpu(&mut self) {
        if let Some(renderer) = self.renderer.as_ref() {
            renderer.wait_idle().unwrap();
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

const PANELS: usize = 4;

fn compute_panel_rects(w: u32, h: u32) -> [Rect; PANELS] {
    let avail_w = w.saturating_sub(GAP * (PANELS as u32 + 1));
    let pw = avail_w / PANELS as u32;
    let ph = h.saturating_sub(GAP * 2);
    std::array::from_fn(|i| Rect {
        x: GAP * (i as u32 + 1) + pw * i as u32,
        y: GAP,
        width: pw,
        height: ph,
    })
}

// ============================================================================
// Chart setup — data (Vec) → columns → series → Chart builder → ChartView/Style.
// Same pattern when adding a new chart kind.
// ============================================================================

fn build_sine_panel(renderer: &mut WindowedRenderer<'_>, rect: Rect) -> PanelEntry {
    let (xs, ys) = demo::sine_data(N);
    renderer.add_column("sine_x", &xs).expect("add x");
    renderer.add_column("sine_y", &ys).expect("add y");

    let mut config = default::default_config();
    // Chart 1: grid off.
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;

    let series = vec![
        SeriesConfig::line("sin", "sine_x", "sine_y")
            .label("sin(x)")
            .color(Color::from_rgb8(20, 110, 230))
            .width(1.0),
    ];
    let mut chart = Chart::new(config)
        .with_area(rect.x, rect.y, rect.width, rect.height)
        .with_title("Sine (1 px line, no grid)")
        .with_x_title("x [rad]")
        .with_y_title("y")
        .with_legend_from(&series);
    chart
        .auto_fit_x(renderer.pool(), "sine_x", 0.02)
        .expect("fit x");
    chart
        .auto_fit_y(renderer.pool(), "sine_y", 0.10)
        .expect("fit y");

    // Fit the margins to the labels and titles.
    chart.fit_layout();
    let view = renderer.create_chart_view(&chart, rect).expect("view");
    panel_entry(renderer, chart, view, series)
}

fn build_rc_panel(renderer: &mut WindowedRenderer<'_>, rect: Rect) -> PanelEntry {
    // Charge + discharge — same t, two V columns.
    let (ts, vs_charge) = demo::rc_data(N);
    let (_, vs_discharge) = demo::rc_discharge_data(N);
    renderer.add_column("rc_t", &ts).expect("add t");
    renderer
        .add_column("rc_charge", &vs_charge)
        .expect("add charge");
    renderer
        .add_column("rc_discharge", &vs_discharge)
        .expect("add discharge");

    let mut config = default::default_config();
    config.grid.show_major_x = true;
    config.grid.show_major_y = true;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;

    // '₀' (U+2080) renders as a subscript segment; plain '0' would not.
    let series = vec![
        SeriesConfig::line("charge", "rc_t", "rc_charge")
            .label("Charging V₀(1−e⁻ᵗ)")
            .color(Color::from_rgb8(220, 90, 60))
            .width(2.0),
        SeriesConfig::line("discharge", "rc_t", "rc_discharge")
            .label("Discharging V₀·e⁻ᵗ")
            .color(Color::from_rgb8(60, 130, 210))
            .width(2.0),
    ];
    let mut chart = Chart::new(config)
        .with_area(rect.x, rect.y, rect.width, rect.height)
        .with_title("RC charge / discharge (2 px, major grid)")
        .with_x_title("t [s]")
        .with_y_title("V [V]")
        .with_legend_from(&series);
    chart
        .auto_fit_x(renderer.pool(), "rc_t", 0.02)
        .expect("fit t");
    // Y range is the union of both series (both span 0..V0 here, but uses the unified API).
    chart
        .auto_fit_y_union(renderer.pool(), &["rc_charge", "rc_discharge"], 0.05)
        .expect("fit v");

    // Fit the margins to the labels and titles.
    chart.fit_layout();
    let view = renderer.create_chart_view(&chart, rect).expect("view");
    panel_entry(renderer, chart, view, series)
}

fn build_cross_section_panel(renderer: &mut WindowedRenderer<'_>, rect: Rect) -> PanelEntry {
    let (es, sigmas) = demo::cross_section_data(N);
    renderer.add_column("xs_e", &es).expect("add E");
    renderer.add_column("xs_sigma", &sigmas).expect("add sigma");

    // Log scale is one line in Config. set_y_range / auto_fit_y automatically picks
    // (a) decade major spacing (b) Power label format (c) multiplicative padding.
    let mut config = default::default_config();
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
            .color(Color::from_rgb8(60, 160, 90))
            .width(3.5),
    ];
    let mut chart = Chart::new(config)
        .with_area(rect.x, rect.y, rect.width, rect.height)
        .with_title("Cross-section σ(E) — 3.5 px, log + minor grid")
        .with_x_title("E [keV]")
        .with_y_title("σ [barn]")
        .with_legend_from(&series);
    chart
        .auto_fit_x(renderer.pool(), "xs_e", 0.0)
        .expect("fit E");
    chart
        .auto_fit_y(renderer.pool(), "xs_sigma", 0.10)
        .expect("fit sigma");

    // Fit the margins to the labels and titles.
    chart.fit_layout();
    let view = renderer.create_chart_view(&chart, rect).expect("view");
    panel_entry(renderer, chart, view, series)
}

/// Field panel — a heatmap with contour lines, labels and a colourbar.
///
/// Also the demo of the batch upload: a grid is 40 columns, and `add_columns`
/// takes the lot in one pool region, one staging buffer and one submit instead of
/// forty of each.
fn build_field_panel(renderer: &mut WindowedRenderer<'_>, rect: Rect) -> PanelEntry {
    const G: usize = 40;
    const SPREAD: f64 = 0.05;
    let coords: Vec<f64> = (0..G).map(|i| i as f64 / (G - 1) as f64).collect();
    let bump = |x: f64, y: f64| {
        let (dx, dy) = (x - 0.5, y - 0.5);
        (-(dx * dx + dy * dy) / SPREAD).exp()
    };
    let grid: Vec<Vec<f64>> = coords
        .iter()
        .map(|x| coords.iter().map(|y| bump(*x, *y)).collect())
        .collect();
    let ids: Vec<String> = (0..G).map(|c| format!("bump_z{c}")).collect();

    let mut batch: Vec<(&str, &dyn ColumnSource)> = Vec::with_capacity(G + 2);
    batch.push(("bump_x", &coords as &dyn ColumnSource));
    batch.push(("bump_y", &coords as &dyn ColumnSource));
    for (id, column) in ids.iter().zip(&grid) {
        batch.push((id.as_str(), column as &dyn ColumnSource));
    }
    renderer.add_columns(&batch).expect("grid batch upload");

    let mut config = default::default_config();
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    // The colourbar owns the z range and the colormap — a field has no value to
    // draw without it, so `validate_renderer_series` insists on one.
    let mut bar = default::default_colorbar_options();
    bar.axis.min = 0.0;
    bar.axis.max = 1.0;
    bar.axis.major_spacing = 0.25;
    config.colorbar = Some(bar);

    let mut chart = Chart::new(config)
        .with_area(rect.x, rect.y, rect.width, rect.height)
        .with_title("Gaussian field — heatmap + labelled contours")
        .with_x_title("x")
        .with_y_title("y");
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);

    // Levels only. Where the labels go is not stated anywhere here: the anchor
    // pass projects seed points onto the implicit field at `spacing_px`. An earlier
    // version of this demo inverted the Gaussian analytically to place them —
    // which only worked because the demo knew the closed form of its own data.
    let levels = vec![0.2f64, 0.4, 0.6, 0.8];

    let ink = Color::from_rgb8(20, 20, 20);
    let series = vec![
        SeriesConfig::contours(
            "bump",
            "bump_x",
            "bump_y",
            MatrixRef::columns_are_x(ids),
            levels,
        )
        .filled()
        .shading(Shading::Interpolated)
        .color(ink)
        .width(1.5)
        .contour_labels(ContourLabelConfig {
            font_size: 11.0,
            color: ink,
            significant_digits: 1,
            bg_color: Some(Color::from_rgb8(250, 250, 250)),
            ..Default::default()
        }),
    ];
    // Fit the margins to the labels and titles.
    chart.fit_layout();
    let view = renderer.create_chart_view(&chart, rect).expect("view");
    panel_entry(renderer, chart, view, series)
}

/// A panel over `series`, with one GPU style per series.
fn panel_entry(
    renderer: &mut WindowedRenderer<'_>,
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
        reported_warnings: Vec::new(),
    }
}

// ============================================================================
// winit ApplicationHandler
// ============================================================================

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("figgy demo (winit) — sine / RC / cross-section")
            .with_inner_size(LogicalSize::new(1800.0, 500.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        let size = window.inner_size();

        // One-line setup — figgy owns surface / device / queue / pipelines.
        let mut renderer = match Renderer::for_window_blocking(
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
            build_field_panel(&mut renderer, rects[3]),
        ];
        for p in panels.iter_mut() {
            let _ = p.chart.consume_data_dirty();
            let _ = p.chart.consume_raster_dirty();
        }

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.panels = panels;
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.shutdown_gpu();
                event_loop.exit();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => match logical_key {
                Key::Named(NamedKey::Escape) => {
                    self.shutdown_gpu();
                    event_loop.exit();
                }
                Key::Character(s) if s.as_str() == "s" || s.as_str() == "S" => {
                    self.export_pngs();
                }
                Key::Character(s) if s.as_str() == "a" || s.as_str() == "A" => {
                    self.cycle_axis_preset();
                }
                Key::Character(s) if s.as_str() == "c" || s.as_str() == "C" => {
                    self.cycle_color_cycle();
                }
                _ => {}
            },
            WindowEvent::Resized(s) => {
                self.handle_resize(s.width, s.height);
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let new_pos = (position.x as f32, position.y as f32);
                let old = self.cursor.replace(new_pos);
                if let Some(old) = old {
                    let (dx, dy) = (new_pos.0 - old.0, new_pos.1 - old.1);
                    if let Some(handle) = self.resizing {
                        self.resize_selected(handle, dx, dy);
                    } else if self.dragging {
                        self.drag_selected(dx, dy);
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.handle_click(),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = false;
                self.resizing = None;
            }
            WindowEvent::RedrawRequested => self.render_frame(),
            _ => {}
        }
    }
}

impl App {
    /// Left press — hit-test every panel's hitmap at the cursor, update the
    /// selection, and arm dragging when the hit element is draggable. A
    /// selection change re-rasters only the panels whose decoration layer
    /// gains / loses the blue box (raster-dirty path).
    fn handle_click(&mut self) {
        let Some((cx, cy)) = self.cursor else { return };

        // Resize handles sit on the selection box edge and must win over
        // plain element hit-testing — check them first.
        if let Some((pi, id)) = self.selected
            && let Some(panel) = self.panels.get(pi)
            && let Some(rz) = panel.hitmap.get(id).and_then(|el| el.as_resizable())
            && let Some(handle) = rz.hit_resize_handle(
                panel.chart.config(),
                &CpuTextMeasure::for_style(&panel.chart.config().draw_style),
                cx,
                cy,
            )
        {
            self.resizing = Some(handle);
            self.dragging = false;
            return;
        }
        self.resizing = None;

        let mut new_sel = None;
        for (i, panel) in self.panels.iter().enumerate() {
            if let Some(id) = panel.hitmap.hit_test(
                panel.chart.config(),
                &CpuTextMeasure::for_style(&panel.chart.config().draw_style),
                cx,
                cy,
            ) {
                new_sel = Some((i, id));
                break; // Panels don't overlap — first hit wins.
            }
        }
        // Press over a draggable element starts a drag (even when re-pressing
        // the already-selected element).
        self.dragging = new_sel.is_some_and(|(i, id)| {
            self.panels[i]
                .hitmap
                .get(id)
                .is_some_and(|el| el.as_draggable().is_some())
        });
        if new_sel == self.selected {
            return;
        }

        for affected in [self.selected, new_sel].into_iter().flatten() {
            if let Some(panel) = self.panels.get_mut(affected.0) {
                // Decoration-only change: the selection box lives in the
                // decoration raster; data and transform are untouched.
                panel.chart.with_decoration_change(|_| {});
            }
        }
        self.selected = new_sel;
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Apply a pointer delta to the selected element via its drag policy.
    /// `config_mut` flags both dirty bits — axis drags move the data area, so
    /// the transform must refresh along with the raster.
    fn drag_selected(&mut self, dx: f32, dy: f32) {
        let Some((pi, id)) = self.selected else {
            return;
        };
        let Some(panel) = self.panels.get_mut(pi) else {
            return;
        };
        let Some(drag) = panel.hitmap.get(id).and_then(|el| el.as_draggable()) else {
            return;
        };
        let measure = CpuTextMeasure::for_style(&panel.chart.config().draw_style);
        if drag.drag_by(panel.chart.config_mut(), &measure, dx, dy) == NudgeResult::Moved
            && let Some(w) = self.window.as_ref()
        {
            w.request_redraw();
        }
    }

    /// Apply a pointer delta to one of the selected element's resize handles.
    fn resize_selected(&mut self, handle: ResizeHandle, dx: f32, dy: f32) {
        let Some((pi, id)) = self.selected else {
            return;
        };
        let Some(panel) = self.panels.get_mut(pi) else {
            return;
        };
        let Some(rz) = panel.hitmap.get(id).and_then(|el| el.as_resizable()) else {
            return;
        };
        let measure = CpuTextMeasure::for_style(&panel.chart.config().draw_style);
        if rz.resize_by(panel.chart.config_mut(), &measure, handle, dx, dy) == NudgeResult::Moved
            && let Some(w) = self.window.as_ref()
        {
            w.request_redraw();
        }
    }

    /// 'A' key — cycle through the axis frame presets on every panel.
    /// One call restyles all four axes; decoration-only change.
    fn cycle_axis_preset(&mut self) {
        self.axis_preset_idx = (self.axis_preset_idx + 1) % AXIS_PRESETS.len();
        let preset = AXIS_PRESETS[self.axis_preset_idx];
        eprintln!("[preset] axis = {preset:?}");
        for panel in self.panels.iter_mut() {
            panel
                .chart
                .with_decoration_change(|cfg| cfg.apply_axis_preset(preset));
        }
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// 'C' key — cycle through the series color rotations. Recolors every
    /// panel's series declarations, rebuilds their GPU styles, and keeps the
    /// legend swatches in sync (demo convention: legend entry i == series i).
    fn cycle_color_cycle(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        self.color_cycle_idx = (self.color_cycle_idx + 1) % COLOR_CYCLES.len();
        let cycle = COLOR_CYCLES[self.color_cycle_idx];
        eprintln!("[preset] colors = {cycle:?}");
        for panel in self.panels.iter_mut() {
            cycle.apply_to_all(&mut panel.series);
            panel.styles = panel
                .series
                .iter()
                .map(|cfg| renderer.create_style_for_series(cfg).unwrap())
                .collect();
            panel.chart.with_decoration_change(|cfg| {
                // The legend is one document: entries are separated by '\n'
                // segments and symbol segments carry per-segment color
                // overrides — recolor those, leave label segments (no
                // override) alone.
                let mut entry = 0usize;
                for seg in &mut cfg.legend.content.segments {
                    if seg.text == '\n' {
                        entry += 1;
                    } else if seg.color.is_some() {
                        seg.color = Some(cycle.color(entry));
                    }
                }
            });
        }
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// 'S' key — save each panel separately as a PNG (scale=2 = 2x DPI).
    fn export_pngs(&mut self) {
        const EXPORT_SCALE: f32 = 2.0;
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        for (i, panel) in self.panels.iter().enumerate() {
            let path = format!("/tmp/figgy_winit_panel_{i}.png");
            match renderer.save_blocking(
                renderer::ExportSource::chart(&panel.chart, &panel.series),
                renderer::ExportOptions::panel(EXPORT_SCALE),
                &path,
            ) {
                Ok(()) => eprintln!("[export] saved {path}"),
                Err(e) => eprintln!("[export] panel {i} failed: {e}"),
            }
        }
    }

    fn handle_resize(&mut self, w: u32, h: u32) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let w = w.max(1);
        let h = h.max(1);
        if let Err(e) = renderer.resize(w, h) {
            eprintln!("[resize] {e}");
            return;
        }

        // The next prepare redraws each panel's raster at its new area.
        let rects = compute_panel_rects(w, h);
        for (panel, rect) in self.panels.iter_mut().zip(rects.iter()) {
            panel
                .chart
                .set_area(rect.x, rect.y, rect.width, rect.height);
            // Refit the margins to the labels at the new size.
            panel.chart.fit_layout();
        }
    }

    fn render_frame(&mut self) {
        let selected = self.selected;
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if self.panels.is_empty() {
            return;
        }

        // ---- the selection is the chart's: prepare draws its box ----
        for (i, panel) in self.panels.iter_mut().enumerate() {
            let selected_id: Option<String> = match selected {
                Some((pi, id)) if pi == i => panel.hitmap.get(id).map(|el| el.element_id()),
                _ => None,
            };
            panel.chart.select(selected_id.as_deref());
            // `Renderer::prepare` (inside `draw` below) redraws a stale raster
            // and rewrites the transform uniform every frame; the dirty flags
            // only need resetting.
            let _ = panel.chart.consume_data_dirty();
            let _ = panel.chart.consume_raster_dirty();
        }

        // ---- Series + ChartDrawItem slices ----
        // Each panel lends its view mutably (prepare redraws a stale raster)
        // and its chart and series immutably: disjoint fields of one panel.
        let mut parts: Vec<(&mut ChartView, &Chart, Vec<Series<'_>>)> = self
            .panels
            .iter_mut()
            .map(|p| {
                let series = p
                    .series
                    .iter()
                    .zip(p.styles.iter())
                    .map(|(cfg, style)| Series { config: cfg, style })
                    .collect();
                (&mut p.view, &p.chart, series)
            })
            .collect();
        let mut items: Vec<ChartDrawItem<'_>> = parts
            .iter_mut()
            .map(|(view, chart, series)| ChartDrawItem {
                view,
                chart,
                series: series.as_slice(),
            })
            .collect();

        // ---- single draw call — surface/encoder/pass/submit/present all internal. ----
        if let Err(e) = renderer.draw(Color::WHITE, &mut items) {
            eprintln!("[draw] {e}");
        }
        drop(items);
        drop(parts);

        // Tick settings the new raster could not draw as written. They are
        // re-derived on every redraw (a drag redraws each frame), so log only
        // when the list changes.
        for (i, panel) in self.panels.iter_mut().enumerate() {
            let warnings = panel.view.render_warnings();
            if warnings != panel.reported_warnings.as_slice() {
                for warning in warnings {
                    eprintln!("[render_warning] panel {i}: {warning}");
                }
                panel.reported_warnings = warnings.to_vec();
            }
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event_loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("run_app");
}

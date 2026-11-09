//! Twin Y axes: the base layer's left axis and an overlay layer with its own
//! right axis share one X axis and one data area.
//! - `twin_axes` writes `gallery-twin-axes.png`: a battery discharge with the
//!   cell voltage on the left axis (filled to its baseline) and the current on
//!   the overlay's right axis, one merged legend, exported as a document.
//!
//! The same example draws the shape annotations of the annotation layer:
//! - `shapes` writes `gallery-shapes.png`: a flow cytometry scatter of 4000
//!   events with three gates over the data, an ellipse, a rectangle and a
//!   polygon, plus an outline-only dashed rectangle around the debris. The
//!   host assigns each event to a gate and computes the percentages.
//!
//! cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::{Result, column, line, scatter, series};
use renderer::{
    Anchor, Annotation, AnnotationKind, AnnotationLineStyle, BoxStyle, Chart, Color, HitMap,
    LayerRef, OverlayLayer, Renderer, ShapeStyle,
    config::{LegendCorner, LegendEntryKind, TickVisibility},
    data_config::{DataRenderType, LineFill},
    line::LineStylePreset,
    text::RichText,
};
use std::path::Path;
use styled::{cartesian, export, rgb, save};

// Twin axes: a battery discharge with the voltage on the left axis and the
// current on an overlay layer's right axis.
fn twin_axes(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(18, 20, 34);
    let ink = rgb(190, 196, 224);
    let mint = rgb(80, 220, 190);
    let lavender = rgb(150, 160, 255);
    let mut chart = cartesian(
        "Battery discharge / one cycle",
        "Minutes",
        "Cell voltage (V)",
        bg,
        ink,
        rgb(38, 42, 66),
    );
    chart.set_x_range(0., 90.);
    chart.set_y_range(3.0, 4.3);
    chart.config_mut().bottom_x.major_spacing = 15.;
    chart.config_mut().left_y.major_spacing = 0.25;
    chart.config_mut().right_y.line_visible = false;
    chart.config_mut().right_y.tick = TickVisibility::None;
    chart.config_mut().left_y.title_option.text.color = mint;
    chart.config_mut().left_y.label_style.color = mint;
    let minutes: Vec<f64> = (0..=360).map(|i| f64::from(i) / 4.).collect();
    let current: Vec<f64> = minutes
        .iter()
        .map(|m| {
            let base = if *m < 30. {
                1.2
            } else if *m < 55. {
                2.6
            } else {
                0.9
            };
            base + 0.12 * (m * 1.3).sin() + 0.05 * (m * 4.7).cos()
        })
        .collect();
    let mut charge = 1.0f64;
    let voltage: Vec<f64> = minutes
        .iter()
        .zip(&current)
        .map(|(_, i)| {
            charge -= i / 60. / 4. / 3.2;
            let open = 3.3 + 0.9 * charge.clamp(0., 1.).powf(0.6);
            open - 0.08 * i
        })
        .collect();
    r.add_column("minutes", &column(minutes))?;
    r.add_column("voltage", &column(voltage))?;
    r.add_column("current", &column(current))?;
    let mut overlay = OverlayLayer::default();
    overlay.series = vec!["current".into()];
    overlay.y_axis.out_margin = 110.;
    overlay.y_axis.line_visible = false;
    overlay.y_axis.tick = TickVisibility::None;
    overlay.y_axis.min = 0.;
    overlay.y_axis.max = 3.5;
    overlay.y_axis.major_spacing = 0.5;
    overlay.y_axis.label_style.color = lavender;
    overlay.y_axis.label_style.font_size = 21.;
    overlay.y_axis.title_option.visible = true;
    overlay.y_axis.title_option.text = RichText::plain("Current (A)", lavender, 23., "");
    // Layers are expanded when the chart is built, so the overlay joins the
    // config before the chart that draws it exists.
    let mut config = chart.config().clone();
    config.overlays.push(overlay);
    let mut chart = Chart::new(config);
    chart.set_x_range(0., 90.);
    chart.set_y_range(3.0, 4.3);
    // Fixed ranges keep round ticks; `auto_fit_y` and
    // `auto_fit_overlay_y_union` fit the same axes from column statistics.
    chart.set_overlay_y_range(0, 0., 3.5);
    chart.with_axis_range_change(|config| {
        config.bottom_x.major_spacing = 15.;
        config.left_y.major_spacing = 0.25;
        config.overlays[0].y_axis.major_spacing = 0.5;
    });
    let mut filled = line(mint, 3.5);
    filled.fill = Some(LineFill::ToBaseline(3.0));
    filled.fill_color = Some(Color::from_rgba(mint.r, mint.g, mint.b, 0.12));
    let specs = [
        series(
            "voltage",
            "minutes",
            "voltage",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(filled),
                errors: None,
                stem: None,
            }),
        ),
        series(
            "current",
            "minutes",
            "current",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(line(lavender, 2.5)),
                errors: None,
                stem: None,
            }),
        ),
    ];
    chart = chart
        .with_legend_entry("Voltage", mint, 3.5, LegendEntryKind::Line)
        .with_legend_entry("Current", lavender, 2.5, LegendEntryKind::Line);
    chart.config_mut().legend.corner = LegendCorner::TopRight;
    // The base layer and the overlay are two panels of one data area.
    let expansion = chart
        .config()
        .expand()
        .map_err(|error| format!("expansion: {error:?}"))?;
    for panel in &expansion.panels {
        let area = panel
            .config
            .data_area()
            .map_err(|error| format!("panel layout: {error:?}"))?;
        assert_eq!(area, expansion.data_area);
    }
    // Pointer interaction addresses one layer at a time: while the overlay is
    // active, its axis answers as `overlay_0:axis_right`.
    let mut interactive = chart.config().clone();
    interactive.set_active_layer(LayerRef::Overlay(0));
    assert!(
        HitMap::for_active_layer(&interactive)
            .find("overlay_0:axis_right")
            .is_some()
    );
    // Overlay layers are part of the document, not the base panel.
    let image = r.export_rgba_blocking(
        renderer::ExportSource::chart(&chart, &specs),
        renderer::ExportOptions::document(1.).with_clear(bg),
    )?;
    save(&image, bg, &out.join("gallery-twin-axes.png"))
}

/// A deterministic value in [0, 1) for sample `i` of stream `seed`.
fn uniform(seed: u32, i: u32) -> f64 {
    ((f64::from(i) * 12.9898 + f64::from(seed) * 78.233).sin() * 43758.5453)
        .fract()
        .abs()
}

/// A deterministic standard normal pair for sample `i` (Box-Muller).
fn normal_pair(seed: u32, i: u32) -> (f64, f64) {
    let u = uniform(seed, i).max(1e-9);
    let v = uniform(seed + 1, i);
    let radius = (-2. * u.ln()).sqrt();
    let angle = std::f64::consts::TAU * v;
    (radius * angle.cos(), radius * angle.sin())
}

/// Whether `(x, y)` lies inside the closed polygon `vertices` (ray casting).
fn inside_polygon(vertices: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut j = vertices.len() - 1;
    for (i, (xi, yi)) in vertices.iter().enumerate() {
        let (xj, yj) = vertices[j];
        if (*yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

// Shape annotations: gates drawn over a flow cytometry scatter. Each gate is
// an annotation (an ellipse, a rectangle, a polygon); the host decides which
// events fall in which gate and colours them by uploading one series per gate.
fn shapes(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(250, 250, 252);
    let ink = rgb(48, 54, 78);
    let grey = rgb(150, 156, 174);
    let teal = rgb(22, 150, 140);
    let amber = rgb(222, 132, 30);
    let violet = rgb(136, 84, 196);
    let mut chart = cartesian(
        "Blood cell gates / 4000 events",
        "Forward scatter (FSC-A, thousands)",
        "Side scatter (SSC-A, thousands)",
        bg,
        ink,
        rgb(232, 234, 242),
    );
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_x = true;
    // The gates, in data units: an ellipse by its bounding box, a rectangle
    // by two corners and a polygon by its vertices.
    let lymph_box = ((44., 9.), (94., 57.));
    let mono_box = ((92., 56.), (140., 108.));
    let gran_polygon = [
        (116., 106.),
        (150., 96.),
        (196., 128.),
        (214., 206.),
        (184., 246.),
        (136., 222.),
        (112., 160.),
    ];
    let debris_box = ((0., 0.), (36., 32.));
    let in_ellipse = |((x0, y0), (x1, y1)): ((f64, f64), (f64, f64)), x: f64, y: f64| {
        let (cx, cy, rx, ry) = (
            (x0 + x1) / 2.,
            (y0 + y1) / 2.,
            (x1 - x0) / 2.,
            (y1 - y0) / 2.,
        );
        ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.
    };
    let in_rect = |((x0, y0), (x1, y1)): ((f64, f64), (f64, f64)), x: f64, y: f64| {
        x >= x0 && x <= x1 && y >= y0 && y <= y1
    };
    // Four populations: debris near the origin, then lymphocytes, monocytes
    // and granulocytes (count, centre, spread, tilt of y along x).
    let populations = [
        (500u32, (14., 12.), (8., 7.), 0.),
        (1300, (69., 33.), (7., 6.5), 0.15),
        (500, (116., 81.), (8.5, 8.5), 0.2),
        (1550, (158., 168.), (15., 20.), 0.75),
    ];
    // Members of each gate, then the events outside every gate.
    let mut groups: [(Vec<f64>, Vec<f64>); 5] = Default::default();
    let mut events = 0u32;
    for (k, (count, (cx, cy), (sx, sy), tilt)) in populations.into_iter().enumerate() {
        for i in 0..count {
            let (zx, zy) = normal_pair(17 + 2 * k as u32, i);
            let x: f64 = (cx + sx * zx).max(1.);
            let y: f64 = (cy + sy * zy + tilt * (x - cx)).max(1.);
            let group = if in_rect(debris_box, x, y) {
                0
            } else if in_ellipse(lymph_box, x, y) {
                1
            } else if in_rect(mono_box, x, y) {
                2
            } else if inside_polygon(&gran_polygon, x, y) {
                3
            } else {
                4
            };
            groups[group].0.push(x);
            groups[group].1.push(y);
            events += 1;
        }
    }
    let percent = |group: usize| 100. * groups[group].0.len() as f64 / f64::from(events);
    let labels = [
        format!("Lymphocytes  {:.1} %", percent(1)),
        format!("Monocytes  {:.1} %", percent(2)),
        format!("Granulocytes  {:.1} %", percent(3)),
    ];
    let mut specs = Vec::new();
    for (k, (id, color, alpha)) in [
        ("debris", grey, 0.45),
        ("lymphocytes", teal, 0.55),
        ("monocytes", amber, 0.6),
        ("granulocytes", violet, 0.5),
        ("ungated", grey, 0.45),
    ]
    .into_iter()
    .enumerate()
    {
        let (x, y) = std::mem::take(&mut groups[k]);
        r.add_column(format!("{id}-fsc"), &column(x))?;
        r.add_column(format!("{id}-ssc"), &column(y))?;
        specs.push(series(
            id,
            &format!("{id}-fsc"),
            &format!("{id}-ssc"),
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(scatter(
                    Color::from_rgba(color.r, color.g, color.b, alpha),
                    4.5,
                )),
                line: None,
                errors: None,
                stem: None,
            }),
        ));
    }
    let data = |(x, y): (f64, f64)| Anchor::Data { x, y };
    let gate = |color: Color, style: LineStylePreset, fill: bool| ShapeStyle {
        fill: fill.then(|| Color::from_rgba(color.r, color.g, color.b, 0.08)),
        border: AnnotationLineStyle {
            color,
            width: 2.5,
            style,
        },
        below: false,
    };
    let label = |at: (f64, f64), text: &str, color: Color| {
        Annotation::new(AnnotationKind::Text {
            anchor: data(at),
            text: RichText::plain(text, color, 20., ""),
            box_style: Some(BoxStyle {
                fill: Color::from_rgba(bg.r, bg.g, bg.b, 0.9),
                border_color: Color::from_rgba(color.r, color.g, color.b, 0.6),
                border_width: 1.5,
                padding: 7.,
            }),
            align: Default::default(),
        })
    };
    chart.config_mut().annotations.items = vec![
        Annotation::new(AnnotationKind::Rect {
            from: data(debris_box.0),
            to: data(debris_box.1),
            style: gate(grey, LineStylePreset::Dash, false),
        }),
        Annotation::new(AnnotationKind::Ellipse {
            from: data(lymph_box.0),
            to: data(lymph_box.1),
            style: gate(teal, LineStylePreset::Solid, true),
        }),
        Annotation::new(AnnotationKind::Rect {
            from: data(mono_box.0),
            to: data(mono_box.1),
            style: gate(amber, LineStylePreset::Solid, true),
        }),
        Annotation::new(AnnotationKind::Polygon {
            vertices: gran_polygon.iter().copied().map(data).collect(),
            style: gate(violet, LineStylePreset::Solid, true),
        }),
        label((6., 58.), "Debris", grey),
        label((99., 40.), &labels[0], teal),
        label((146., 90.), &labels[1], amber),
        label((46., 196.), &labels[2], violet),
    ];
    chart.set_x_range(0., 250.);
    chart.set_y_range(0., 250.);
    chart.config_mut().bottom_x.major_spacing = 50.;
    chart.config_mut().left_y.major_spacing = 50.;
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-shapes.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/overlay-axes")?;
    gallery::with_renderer(|r| {
        twin_axes(r, &out)?;
        shapes(r, &out)
    })
}

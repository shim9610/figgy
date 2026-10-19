//! Twin Y axes: the base layer's left axis and an overlay layer with its own
//! right axis share one X axis and one data area.
//! - `twin_axes` writes `gallery-twin-axes.png`: a battery discharge with the
//!   cell voltage on the left axis (filled to its baseline) and the current on
//!   the overlay's right axis, one merged legend, exported as a document.
//!
//! cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::{Result, column, line, series};
use renderer::{
    Chart, Color, HitMap, LayerRef, OverlayLayer, Renderer,
    config::{LegendCorner, LegendEntryKind, TickVisibility},
    data_config::{DataRenderType, LineFill},
    text::RichText,
};
use std::path::Path;
use styled::{cartesian, rgb, save};

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
            DataRenderType::Line { line: filled },
        ),
        series(
            "current",
            "minutes",
            "current",
            DataRenderType::Line {
                line: line(lavender, 2.5),
            },
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
    let image =
        pollster::block_on(r.export_document_rgba_with_clear_async(&chart, &specs, 1., bg))?;
    save(&image, bg, &out.join("gallery-twin-axes.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/overlay-axes")?;
    gallery::with_renderer(|r| twin_axes(r, &out))
}

//! A ternary plot: the USDA soil texture triangle with three soil cores.
//! - `render` writes `gallery-ternary.png`: the twelve texture classes as
//!   filled, outlined region polygons (`TernaryRegionStyle`), each class
//!   named by a text annotation, and three cores sampled every 10 cm as
//!   lines with markers whose radius follows a depth column
//!   (`point_size_map`). Sand, silt and clay are three ordinary columns; the
//!   renderer places each sample by their sum.
//!
//! cargo run --locked -p figgy-renderer --example ternary -- target/ternary
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::{Result, column, line, scatter, series};
use renderer::{
    Anchor, Annotation, AnnotationKind, Color, CpuTextMeasure, MeasureText, Renderer, TextAlign,
    config::{LegendCorner, LegendEntryKind, TickVisibility},
    data_config::{DataRenderType, PointSizeMap, SizeScale},
    ternary::{TernaryOptions, TernaryRegionStyle},
    text::RichText,
};
use std::path::Path;
use styled::{cartesian, export, rgb, save};

/// A USDA texture class: its name, fill colour, the polygon's vertices as
/// (sand, silt, clay) percentages and the place of its name.
type Class = (&'static str, Color, &'static [[f64; 3]], [f64; 3]);

fn classes() -> [Class; 12] {
    [
        (
            "Clay",
            rgb(214, 160, 140),
            &[
                [45., 15., 40.],
                [45., 0., 55.],
                [0., 0., 100.],
                [0., 40., 60.],
                [20., 40., 40.],
            ],
            [20., 20., 60.],
        ),
        (
            "Silty clay",
            rgb(196, 162, 178),
            &[[20., 40., 40.], [0., 40., 60.], [0., 60., 40.]],
            [6., 47., 47.],
        ),
        (
            "Sandy clay",
            rgb(226, 178, 132),
            &[[65., 0., 35.], [45., 0., 55.], [45., 20., 35.]],
            [52., 6., 42.],
        ),
        (
            "Clay loam",
            rgb(206, 182, 160),
            &[
                [45., 28., 27.],
                [45., 15., 40.],
                [20., 40., 40.],
                [20., 53., 27.],
            ],
            [28., 40., 32.],
        ),
        (
            "Silty clay\nloam",
            rgb(184, 178, 190),
            &[
                [20., 53., 27.],
                [20., 40., 40.],
                [0., 60., 40.],
                [0., 73., 27.],
            ],
            [15., 52., 33.],
        ),
        (
            "Sandy clay\nloam",
            rgb(232, 196, 146),
            &[
                [80., 0., 20.],
                [65., 0., 35.],
                [45., 20., 35.],
                [45., 28., 27.],
                [52., 28., 20.],
            ],
            [60., 13., 27.],
        ),
        (
            "Loam",
            rgb(200, 196, 150),
            &[
                [52., 41., 7.],
                [52., 28., 20.],
                [45., 28., 27.],
                [23., 50., 27.],
                [43., 50., 7.],
            ],
            [40., 45., 15.],
        ),
        (
            "Silt loam",
            rgb(178, 200, 168),
            &[
                [50., 50., 0.],
                [43., 50., 7.],
                [23., 50., 27.],
                [0., 73., 27.],
                [0., 88., 12.],
                [8., 80., 12.],
                [20., 80., 0.],
            ],
            [28., 62., 10.],
        ),
        (
            "Silt",
            rgb(158, 194, 176),
            &[
                [20., 80., 0.],
                [8., 80., 12.],
                [0., 88., 12.],
                [0., 100., 0.],
            ],
            [5., 90., 5.],
        ),
        (
            "Sandy loam",
            rgb(236, 214, 150),
            &[
                [70., 30., 0.],
                [85., 0., 15.],
                [80., 0., 20.],
                [52., 28., 20.],
                [52., 41., 7.],
                [43., 50., 7.],
                [50., 50., 0.],
            ],
            [63., 27., 10.],
        ),
        (
            "Loamy sand",
            rgb(242, 224, 166),
            &[
                [90., 0., 10.],
                [85., 15., 0.],
                [70., 30., 0.],
                [85., 0., 15.],
            ],
            [76., 22., 2.],
        ),
        (
            "Sand",
            rgb(247, 234, 186),
            &[[100., 0., 0.], [85., 15., 0.], [90., 0., 10.]],
            [93., 4., 3.],
        ),
    ]
}

/// The three cores: name, colour and texture as a closed-form function of
/// depth `d` in 0..1 (sand %, clay %); silt is the rest.
type Core = (&'static str, Color, fn(f64) -> (f64, f64));

fn cores() -> [Core; 3] {
    [
        ("Floodplain", rgb(36, 74, 140), |d| {
            (
                12. - 9. * d + 1.2 * (9. * d).sin(),
                10. + 26. * d * d + 1.2 * (7. * d).cos(),
            )
        }),
        ("Glacial till", rgb(150, 40, 70), |d| {
            (
                50. - 10. * d + 1.5 * (8. * d + 1.).sin(),
                10. + 20. * d + 1.2 * (11. * d).sin(),
            )
        }),
        ("Dune margin", rgb(18, 112, 96), |d| {
            (
                85. - 24. * d.powf(1.3) + 1.2 * (10. * d).sin(),
                7. + 10. * d.powf(1.5) + 0.6 * (13. * d).cos(),
            )
        }),
    ]
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(252, 250, 245);
    let ink = rgb(62, 56, 50);
    let mut chart = cartesian(
        "Soil texture / three cores, 0 to 140 cm",
        "Sand (%)",
        "Clay (%)",
        bg,
        ink,
        rgb(226, 220, 208),
    );
    let config = chart.config_mut();
    // The third component's edge reads like the other two.
    config.right_y = config.left_y.clone();
    config.right_y.title_option.text = RichText::plain("Silt (%)", ink, 23., "");
    config.set_ternary(Some(TernaryOptions {}));
    config.grid.show_major_x = true;
    config.grid.show_major_y = true;
    for axis in [
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.min = 0.;
        axis.max = 100.;
        axis.major_spacing = 10.;
        axis.minor_count = 0;
        axis.line_visible = true;
        axis.line_color = rgb(150, 140, 128);
        axis.tick = TickVisibility::Outside;
        axis.label_style.font_size = 16.;
        axis.label_style.color = rgb(120, 110, 100);
    }
    config.legend.corner = LegendCorner::TopLeft;
    // The triangle is as tall as the plot: no chrome below it.
    config.bottom_x.out_margin = 30.;
    config.top_x.out_margin = 10.;

    let mut specs = Vec::new();
    // The class polygons, one series each so that neighbours never form an
    // even-odd hole; the outline repeats the first vertex to close.
    for (k, (name, color, polygon, _)) in classes().iter().enumerate() {
        let ring = polygon.iter().chain(polygon.first());
        for (i, suffix) in ["sand", "silt", "clay"].iter().enumerate() {
            r.add_column(
                format!("class-{k}-{suffix}"),
                &column(ring.clone().map(|v| v[i]).collect()),
            )?;
        }
        specs.push(series(
            name,
            &format!("class-{k}-sand"),
            &format!("class-{k}-clay"),
            DataRenderType::Ternary {
                c_column: format!("class-{k}-silt"),
                scatter: None,
                line: Some(line(bg, 2.)),
                region: Some(TernaryRegionStyle {
                    color: *color,
                    opacity: 0.75,
                }),
            },
        ));
    }

    // The cores: one sample every 10 cm, a marker radius by depth.
    let depths: Vec<f64> = (0..15).map(|i| f64::from(i) * 10.).collect();
    for (k, (name, color, texture)) in cores().iter().enumerate() {
        let (mut sand, mut silt, mut clay) = (Vec::new(), Vec::new(), Vec::new());
        for depth in &depths {
            let (s, c) = texture(depth / 140.);
            sand.push(s);
            clay.push(c);
            silt.push(100. - s - c);
        }
        for (suffix, data) in [
            ("sand", sand),
            ("silt", silt),
            ("clay", clay),
            ("depth", depths.clone()),
        ] {
            r.add_column(format!("core-{k}-{suffix}"), &column(data))?;
        }
        let mut markers = scatter(*color, 6.);
        let mut size = PointSizeMap {
            column: format!("core-{k}-depth"),
            domain_min: 0.,
            domain_max: 1.,
            min_px: 3.5,
            max_px: 9.,
            scale: SizeScale::Radius,
        };
        size.fit_domain(0., 140.);
        markers.point_size_map = Some(size);
        specs.push(series(
            &format!("core-{k}"),
            &format!("core-{k}-sand"),
            &format!("core-{k}-clay"),
            DataRenderType::Ternary {
                c_column: format!("core-{k}-silt"),
                scatter: Some(markers),
                line: Some(line(*color, 2.2)),
                region: None,
            },
        ));
        chart = chart.with_legend_entry(name, *color, 2.2, LegendEntryKind::LineScatter);
    }

    // Each class name centred on its place: the triangle's geometry turns
    // the composition into pixels, the measured text centres the block.
    let config = chart.config_mut();
    let triangle = config.ternary_geometry().ok_or("no ternary geometry")?;
    let plot = config
        .data_area()
        .map_err(|e| format!("data area: {e:?}"))?
        .0;
    let measure = CpuTextMeasure::for_style(&config.draw_style);
    let label_ink = rgb(88, 74, 62);
    let centred = |text: RichText, [x, y]: [f32; 2]| {
        let size = measure.measure_rich(&text);
        Annotation::new(AnnotationKind::Text {
            anchor: Anchor::Ratio {
                x: (x - size.width * 0.5 - plot.x as f32) / plot.width as f32,
                y: (y - size.height() * 0.5 - plot.y as f32) / plot.height as f32,
            },
            text,
            box_style: None,
        })
    };
    for (name, _, _, [sand, silt, clay]) in classes() {
        let place = triangle.point([sand as f32 / 100., clay as f32 / 100., silt as f32 / 100.]);
        let mut text = RichText::plain(name, label_ink, 15., "");
        text.align = TextAlign::Center;
        config.annotations.items.push(centred(text, place));
    }
    let note = RichText::plain("Marker size grows with depth", ink, 18., "");
    config
        .annotations
        .items
        .push(Annotation::new(AnnotationKind::Text {
            anchor: Anchor::Ratio { x: 0.76, y: 0.02 },
            text: note,
            box_style: None,
        }));

    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-ternary.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/ternary")?;
    gallery::with_renderer(|r| render(r, &out))
}

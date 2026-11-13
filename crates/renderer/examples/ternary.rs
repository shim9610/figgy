//! A ternary plot: the USDA soil texture triangle with three soil cores.
//! - `render` writes `gallery-ternary.png`: the twelve texture classes as
//!   filled, outlined region polygons (`TernaryRegionStyle`), each class
//!   named by a text annotation, and three cores sampled every 10 cm as
//!   lines with markers whose radius follows a depth column
//!   (`point_size_map`). Sand, silt and clay are three ordinary columns; the
//!   renderer places each sample by their sum. The look is the ternary
//!   theme `warm-paper`: the classes' and cores' colours, the outlines, the
//!   axes and the grid.
//!
//! cargo run --locked -p figgy-renderer --example ternary -- target/ternary
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::Result;
use renderer::{
    Anchor, Annotation, AnnotationKind, CpuTextMeasure, MeasureText, Renderer, SeriesConfig,
    TextAlign,
    data_config::{DataLineStyleConfig, DataRenderType, SizeScale},
    ternary::{TernaryOptions, TernaryRegionStyle},
    text::RichText,
    theme::ChartKind,
};
use std::path::Path;
use styled::{background, export, save, scene_chart, theme_named, themed};

/// A USDA texture class: its name, the polygon's vertices as (sand, silt,
/// clay) percentages and the place of its name. Class `k` takes the
/// theme's region colour `k`.
type Class = (&'static str, &'static [[f64; 3]], [f64; 3]);

fn classes() -> [Class; 12] {
    [
        (
            "Clay",
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
            &[[20., 40., 40.], [0., 40., 60.], [0., 60., 40.]],
            [6., 47., 47.],
        ),
        (
            "Sandy clay",
            &[[65., 0., 35.], [45., 0., 55.], [45., 20., 35.]],
            [52., 6., 42.],
        ),
        (
            "Clay loam",
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
            &[[100., 0., 0.], [85., 15., 0.], [90., 0., 10.]],
            [93., 4., 3.],
        ),
    ]
}

/// The three cores: name and texture as a closed-form function of depth
/// `d` in 0..1 (sand %, clay %); silt is the rest. Core `k` takes the
/// theme's colour `k`.
type Core = (&'static str, fn(f64) -> (f64, f64));

fn cores() -> [Core; 3] {
    [
        ("Floodplain", |d| {
            (
                12. - 9. * d + 1.2 * (9. * d).sin(),
                10. + 26. * d * d + 1.2 * (7. * d).cos(),
            )
        }),
        ("Glacial till", |d| {
            (
                50. - 10. * d + 1.5 * (8. * d + 1.).sin(),
                10. + 20. * d + 1.2 * (11. * d).sin(),
            )
        }),
        ("Dune margin", |d| {
            (
                85. - 24. * d.powf(1.3) + 1.2 * (10. * d).sin(),
                7. + 10. * d.powf(1.5) + 0.6 * (13. * d).cos(),
            )
        }),
    ]
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    let theme = theme_named(ChartKind::Ternary, "warm-paper")?;
    let bg = background(&theme);
    let ink = theme.chrome.ink;
    let mut chart = scene_chart(
        "Soil texture / three cores, 0 to 140 cm",
        "Sand (%)",
        "Clay (%)",
    );
    let config = chart.config_mut();
    // The third component's edge reads like the other two.
    config.right_y = config.left_y.clone();
    config.right_y.title_option.text.segments =
        renderer::text::rich_segments_from_label("Silt (%)");
    config.set_ternary(Some(TernaryOptions {}));
    for axis in [
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.min = 0.;
        axis.max = 100.;
        axis.major_spacing = 10.;
        axis.minor_count = 0;
    }

    let mut specs = Vec::new();
    // The class polygons, one series each so that neighbours never form an
    // even-odd hole; the outline repeats the first vertex to close.
    for (k, (name, polygon, _)) in classes().iter().enumerate() {
        let ring = polygon.iter().chain(polygon.first());
        for (i, suffix) in ["sand", "silt", "clay"].iter().enumerate() {
            let values: Vec<f64> = ring.clone().map(|v| v[i]).collect();
            r.add_column(format!("class-{k}-{suffix}"), &values)?;
        }
        // An outlined region without markers: no constructor draws that.
        specs.push(SeriesConfig::of_kind(
            *name,
            format!("class-{k}-sand"),
            format!("class-{k}-clay"),
            DataRenderType::Ternary {
                c_column: format!("class-{k}-silt").into(),
                scatter: None,
                line: Some(DataLineStyleConfig::default()),
                region: Some(TernaryRegionStyle::default()),
            },
        ));
    }

    // The cores: one sample every 10 cm, a marker radius by depth.
    let depths: Vec<f64> = (0..15).map(|i| f64::from(i) * 10.).collect();
    for (k, (name, texture)) in cores().iter().enumerate() {
        let (mut sand, mut silt, mut clay) = (Vec::new(), Vec::new(), Vec::new());
        for depth in &depths {
            let (s, c) = texture(depth / 140.);
            sand.push(s);
            clay.push(c);
            silt.push(100. - s - c);
        }
        for (suffix, data) in [
            ("sand", &sand),
            ("silt", &silt),
            ("clay", &clay),
            ("depth", &depths),
        ] {
            r.add_column(format!("core-{k}-{suffix}"), data)?;
        }
        // A line through markers whose radius (not area) follows the depth.
        let core = SeriesConfig::ternary(
            format!("core-{k}"),
            format!("core-{k}-sand"),
            format!("core-{k}-clay"),
            format!("core-{k}-silt"),
        )
        // A line through the markers; the theme sets its width.
        .with_line(1.)
        .size_by(format!("core-{k}-depth"), [0., 140.], [3.5, 9.])
        .style_markers(|markers| {
            if let Some(size) = &mut markers.point_size_map {
                size.scale = SizeScale::Radius;
            }
        });
        specs.push(core.label(name));
    }
    chart = chart.with_legend_from(&specs);
    // The class names below are placed from the data area the theme's
    // text sizes leave.
    themed(&mut chart, &theme, &mut specs)?;

    // Each class name centred on its place: the triangle's geometry turns
    // the composition into pixels, the measured text centres the block.
    let config = chart.config_mut();
    let triangle = config.ternary_geometry().ok_or("no ternary geometry")?;
    let plot = config
        .data_area()
        .map_err(|e| format!("data area: {e:?}"))?
        .0;
    let measure = CpuTextMeasure::for_style(&config.draw_style);
    let centred = |text: RichText, [x, y]: [f32; 2]| {
        let size = measure.measure_rich(&text);
        Annotation::new(AnnotationKind::Text {
            anchor: Anchor::Ratio {
                x: (x - size.width * 0.5 - plot.x as f32) / plot.width as f32,
                y: (y - size.height() * 0.5 - plot.y as f32) / plot.height as f32,
            },
            text,
            box_style: None,
            align: Default::default(),
        })
    };
    for (name, _, [sand, silt, clay]) in classes() {
        let place = triangle.point([sand as f32 / 100., clay as f32 / 100., silt as f32 / 100.]);
        let mut text = RichText::plain(name, ink, 15., "");
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
            align: Default::default(),
        }));

    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-ternary.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/ternary")?;
    gallery::with_renderer(|r| render(r, &out))
}

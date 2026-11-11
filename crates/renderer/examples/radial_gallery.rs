//! Actual native shader renders: materials, rounded corners, outlines, selection.
//! `sunburst` builds `gallery-sunburst.png`: a flat sunburst, slices with
//! `children` drawn as nested rings (source group, source, technology).
//! cargo run -p figgy-renderer --example radial_gallery -- target/radial-gallery
use renderer::{Color, ExportOptions, Material, radial::*};
use std::path::PathBuf;
fn base() -> RadialChart {
    RadialChart {
        kind: RadialKind::Donut { inner_radius: 0.5 },
        labels: RadialLabels::Outside,
        label_format: RadialLabelFormat::NamePercent,
        slices: vec![
            RadialSlice::new("Search", 40.0, Color::from_rgb8(51, 132, 245)),
            RadialSlice::new("Direct", 25.0, Color::from_rgb8(165, 216, 72)),
            RadialSlice::new("Email", 15.0, Color::from_rgb8(80, 96, 121)),
            RadialSlice::new("Union", 12.0, Color::from_rgb8(255, 151, 62)),
            RadialSlice::new("Video", 8.0, Color::from_rgb8(48, 192, 230)),
        ],
        style: RadialStyle {
            material: Material::Matte,
            // Match the approved oblique mockup; 0 degrees is straight overhead.
            tilt_degrees: 55.0,
            depth: 0.16,
            inner_corner: 0.045,
            outer_corner: 0.045,
            bevel: 0.025,
            gap_degrees: 1.4,
            shadow: true,
            roughness: 0.3,
            texture_strength: 0.12,
            ..Default::default()
        },
        ..Default::default()
    }
    .with_title("Regional share")
}
/// A flat sunburst: a year's electricity generation by source group at the
/// centre, the sources around it and their technologies outside. Every node
/// keeps its share of its parent's span; the leaves on the inner rings leave
/// the outer ring empty past them.
fn sunburst() -> RadialChart {
    let node =
        |label: &str, value: f64, rgb: (u8, u8, u8), children: Vec<RadialSlice>| RadialSlice {
            children,
            ..RadialSlice::new(label, value, Color::from_rgb8(rgb.0, rgb.1, rgb.2))
        };
    let leaf = |label: &str, value: f64, rgb: (u8, u8, u8)| node(label, value, rgb, vec![]);
    RadialChart {
        kind: RadialKind::Donut { inner_radius: 0.14 },
        labels: RadialLabels::Outside,
        label_format: RadialLabelFormat::NamePercent,
        label_decimals: 0,
        label_color: Color::from_rgb8(40, 44, 52),
        slices: vec![
            node(
                "Renewables",
                46.0,
                (38, 140, 112),
                vec![
                    node(
                        "Wind",
                        20.0,
                        (72, 172, 140),
                        vec![
                            leaf("Onshore", 13.0, (118, 196, 166)),
                            leaf("Offshore", 7.0, (160, 214, 190)),
                        ],
                    ),
                    node(
                        "Solar",
                        12.0,
                        (104, 186, 128),
                        vec![
                            leaf("Utility", 8.0, (146, 206, 160)),
                            leaf("Rooftop", 4.0, (186, 226, 192)),
                        ],
                    ),
                    leaf("Hydro", 10.0, (130, 200, 196)),
                    leaf("Bio", 4.0, (176, 220, 214)),
                ],
            ),
            node(
                "Fossil",
                38.0,
                (196, 112, 64),
                vec![
                    node(
                        "Gas",
                        26.0,
                        (222, 146, 96),
                        vec![
                            leaf("Combined cycle", 21.0, (236, 174, 128)),
                            leaf("Peakers", 5.0, (246, 204, 168)),
                        ],
                    ),
                    leaf("Coal", 9.0, (188, 160, 146)),
                    leaf("Oil", 3.0, (214, 196, 184)),
                ],
            ),
            leaf("Nuclear", 16.0, (128, 110, 196)),
        ],
        // A sunburst draws flat: no tilt and no depth.
        style: RadialStyle {
            material: Material::Flat,
            gap_degrees: 0.6,
            inner_corner: 0.012,
            outer_corner: 0.012,
            gloss: 0.15,
            ..Default::default()
        },
        ..Default::default()
    }
    .with_title("Electricity generation / group, source, technology")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "target/radial-gallery".into());
    std::fs::create_dir_all(&directory)?;
    let mut renderer = RadialRenderer::headless_blocking()?;
    let mut cases = Vec::new();
    for (name, material) in [
        ("matte", Material::Matte),
        ("ceramic", Material::Ceramic),
        ("satin-metal", Material::SatinMetal),
        ("toon", Material::Toon),
        ("enamel", Material::Enamel),
        ("brushed-metal", Material::BrushedMetal),
        ("paper", Material::Paper),
        ("plywood", Material::Wood),
        ("hatch", Material::Hatch),
        ("pearl", Material::Pearl),
    ] {
        let mut c = base().with_title(&name.to_uppercase());
        c.style.material = material;
        if material == Material::Toon {
            c.style.outline.rim = true;
            c.style.outline.separators = true;
        }
        if matches!(
            material,
            Material::Hatch | Material::Pearl | Material::Paper | Material::Wood
        ) {
            c.style.texture_strength = 0.7;
        }
        if material == Material::Wood {
            c.style.roughness = 0.9;
            c.style.gloss = 0.1;
        }
        cases.push((name.to_owned(), c));
    }
    let mut outlined = base().with_title("Raised pie / visible rim and separators");
    outlined.kind = RadialKind::Pie;
    outlined.style.material = Material::Flat;
    outlined.style.depth = 0.3;
    outlined.style.bevel = 0.0;
    outlined.style.gap_degrees = 0.0;
    outlined.style.inner_corner = 0.0;
    outlined.style.outer_corner = 0.0;
    outlined.style.outline.rim = true;
    outlined.style.outline.separators = true;
    outlined.slices = vec![
        RadialSlice::new("Control", 2., Color::from_rgb8(140, 69, 147)),
        RadialSlice::new("Low", 3., Color::from_rgb8(108, 157, 148)),
        RadialSlice::new("Medium", 4., Color::from_rgb8(224, 164, 77)),
        RadialSlice::new("High", 5., Color::from_rgb8(107, 142, 182)),
    ];
    cases.push(("outlined-raised-pie".into(), outlined));
    for (name, amount) in [("corners-sharp", 0.0), ("corners-rounded", 0.12)] {
        let mut c = base().with_title(name);
        c.style.inner_corner = amount;
        c.style.outer_corner = amount;
        c.style.depth = 0.0;
        c.style.tilt_degrees = 0.0;
        c.style.shadow = false;
        cases.push((name.into(), c));
    }
    let mut edited = base().with_title("Selected slice / individual edits");
    edited.interaction.selected = Some(RadialTarget::main(0));
    let style = RadialStyle {
        material: Material::Ceramic,
        outline: RadialOutline {
            emphasis: true,
            width: 1.5,
            ..Default::default()
        },
        ..edited.style.clone()
    };
    let slice = edited.slice_mut(RadialTarget::main(0)).unwrap();
    slice.color = Color::from_rgb8(195, 80, 210);
    slice.style = Some(style);
    slice.label_format = Some(RadialLabelFormat::NameValuePercent);
    edited.value_suffix = " units".into();
    cases.push(("individual-edit".into(), edited));
    let mut split = base().with_title("Regional share / Video breakdown");
    split.labels = RadialLabels::Inside;
    split.label_format = RadialLabelFormat::Percent;
    split.split = Some(RadialSplit {
        slice_index: 4,
        kind: RadialKind::Donut { inner_radius: 0.4 },
        children: vec![
            RadialSlice::new("D", 2.0, Color::from_rgb8(72, 176, 205)),
            RadialSlice::new("E", 6.0, Color::from_rgb8(106, 218, 234)),
        ],
    });
    cases.push(("pie-of-pie".into(), split));
    cases.push(("gallery-sunburst".into(), sunburst()));
    // Deterministic time samples: host calls the same model transition on pointer events.
    let mut hover = base().with_title("Hover lift / Search");
    hover.style.material = Material::Ceramic;
    for frame in 0..20 {
        hover.interaction.animate_hover(
            if frame < 10 {
                Some(RadialTarget::main(0))
            } else {
                None
            },
            0.02,
            0.2,
        );
        cases.push((format!("hover-{frame:02}"), hover.clone()));
    }
    for (name, mut chart) in cases {
        // The drawn label strings are written once each case is edited.
        chart.format_labels();
        let path = directory.join(format!("{name}.png"));
        renderer.save_blocking(&chart, (900, 620), ExportOptions::document(1.0), &path)?;
        renderer.end_frame();
        eprintln!("{}", path.display());
    }
    Ok(())
}

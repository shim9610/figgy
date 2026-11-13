//! Actual native shader renders: materials, rounded corners, outlines, selection.
//! The look is the `ceramic` radial theme (#291); each case varies the theme
//! and applies it to a chart that holds only names and values.
//! `sunburst` builds `gallery-sunburst.png`: a flat sunburst, slices with
//! `children` drawn as nested rings (source group, source, technology).
//! cargo run -p figgy-renderer --example radial_gallery -- target/radial-gallery
use renderer::{Color, ExportOptions, Material, radial::*, theme::RadialTheme};
use std::path::PathBuf;

/// A slice by its name and value; the theme colours it.
fn slice(label: &str, value: f64) -> RadialSlice {
    RadialSlice::new(label, value, Color::BLACK)
}

/// The traffic donut every material case draws, in `theme`.
fn base(theme: &RadialTheme) -> RadialChart {
    let mut chart = RadialChart {
        kind: RadialKind::Donut { inner_radius: 0.5 },
        labels: RadialLabels::Outside,
        label_format: RadialLabelFormat::NamePercent,
        slices: vec![
            slice("Search", 40.0),
            slice("Direct", 25.0),
            slice("Email", 15.0),
            slice("Union", 12.0),
            slice("Video", 8.0),
        ],
        ..Default::default()
    }
    .with_title("Regional share");
    theme.apply(&mut chart);
    chart
}

/// A flat sunburst: a year's electricity generation by source group at the
/// centre, the sources around it and their technologies outside. Every node
/// keeps its share of its parent's span; the leaves on the inner rings leave
/// the outer ring empty past them. The `sunburst` theme colours the groups
/// and steps each ring paler.
fn sunburst(theme: &RadialTheme) -> RadialChart {
    let node = |label: &str, value: f64, children: Vec<RadialSlice>| RadialSlice {
        children,
        ..slice(label, value)
    };
    let leaf = |label: &str, value: f64| slice(label, value);
    let mut chart = RadialChart {
        kind: RadialKind::Donut { inner_radius: 0.14 },
        labels: RadialLabels::Outside,
        label_format: RadialLabelFormat::NamePercent,
        label_decimals: 0,
        slices: vec![
            node(
                "Renewables",
                46.0,
                vec![
                    node(
                        "Wind",
                        20.0,
                        vec![leaf("Onshore", 13.0), leaf("Offshore", 7.0)],
                    ),
                    node(
                        "Solar",
                        12.0,
                        vec![leaf("Utility", 8.0), leaf("Rooftop", 4.0)],
                    ),
                    leaf("Hydro", 10.0),
                    leaf("Bio", 4.0),
                ],
            ),
            node(
                "Fossil",
                38.0,
                vec![
                    node(
                        "Gas",
                        26.0,
                        vec![leaf("Combined cycle", 21.0), leaf("Peakers", 5.0)],
                    ),
                    leaf("Coal", 9.0),
                    leaf("Oil", 3.0),
                ],
            ),
            leaf("Nuclear", 16.0),
        ],
        ..Default::default()
    }
    .with_title("Electricity generation / group, source, technology");
    theme.apply(&mut chart);
    chart
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "target/radial-gallery".into());
    std::fs::create_dir_all(&directory)?;
    let mut renderer = RadialRenderer::headless_blocking()?;
    // The gallery's raised look; each case varies the theme before applying
    // it.
    let ceramic = RadialTheme::named("ceramic").ok_or("no ceramic radial theme")?;
    let matte = RadialTheme {
        material: Material::Matte,
        ..ceramic.clone()
    };
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
        let mut theme = ceramic.clone();
        theme.material = material;
        if material == Material::Toon {
            theme.outline.rim = true;
            theme.outline.separators = true;
        }
        if matches!(
            material,
            Material::Hatch | Material::Pearl | Material::Paper | Material::Wood
        ) {
            theme.texture_strength = 0.7;
        }
        if material == Material::Wood {
            theme.roughness = 0.9;
            theme.gloss = 0.1;
        }
        cases.push((
            name.to_owned(),
            base(&theme).with_title(&name.to_uppercase()),
        ));
    }
    // A raised pie in the four colours of a dose study.
    let mut flat = matte.clone();
    flat.palette = vec![
        Color::from_rgb8(140, 69, 147),
        Color::from_rgb8(108, 157, 148),
        Color::from_rgb8(224, 164, 77),
        Color::from_rgb8(107, 142, 182),
    ];
    flat.material = Material::Flat;
    flat.depth = 0.3;
    flat.bevel = 0.0;
    flat.gap_degrees = 0.0;
    flat.inner_corner = 0.0;
    flat.outer_corner = 0.0;
    flat.outline.rim = true;
    flat.outline.separators = true;
    let mut outlined = base(&flat).with_title("Raised pie / visible rim and separators");
    outlined.kind = RadialKind::Pie;
    outlined.slices = vec![
        slice("Control", 2.),
        slice("Low", 3.),
        slice("Medium", 4.),
        slice("High", 5.),
    ];
    flat.apply(&mut outlined);
    cases.push(("outlined-raised-pie".into(), outlined));
    for (name, amount) in [("corners-sharp", 0.0), ("corners-rounded", 0.12)] {
        let mut theme = matte.clone();
        theme.inner_corner = amount;
        theme.outer_corner = amount;
        theme.depth = 0.0;
        theme.tilt_degrees = 0.0;
        theme.shadow = false;
        cases.push((name.into(), base(&theme).with_title(name)));
    }
    // One slice edited by hand: its own colour, finish and label.
    let mut edited = base(&matte).with_title("Selected slice / individual edits");
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
    let mut split = base(&matte).with_title("Regional share / Video breakdown");
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
    let burst = RadialTheme::named("sunburst").ok_or("no sunburst radial theme")?;
    cases.push(("gallery-sunburst".into(), sunburst(&burst)));
    // Deterministic time samples: host calls the same model transition on pointer events.
    let mut hover = base(&ceramic).with_title("Hover lift / Search");
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

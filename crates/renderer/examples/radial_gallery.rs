//! Actual native shader renders: materials, rounded corners, outlines, selection.
//! cargo run -p figgy-renderer --example radial_gallery -- target/radial-gallery
use renderer::{
    Color, RendererDevice,
    data_render::{create_instance, request_adapter, request_device},
    encode_png,
    radial::*,
};
use std::{path::PathBuf, sync::Arc};
fn base() -> RadialChart {
    let mut chart = RadialChart {
        title: "Regional share".into(),
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
        ..Default::default()
    };
    chart.style = RadialStyle {
        material: RadialMaterial::Matte,
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
    };
    chart
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "target/radial-gallery".into());
    std::fs::create_dir_all(&directory)?;
    let instance = create_instance();
    let adapter = request_adapter(&instance)?;
    eprintln!("adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device(&adapter)?;
    let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
    let mut renderer = RadialRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)?;
    let mut cases = Vec::new();
    for (name, material) in [
        ("matte", RadialMaterial::Matte),
        ("ceramic", RadialMaterial::Ceramic),
        ("satin-metal", RadialMaterial::SatinMetal),
        ("toon", RadialMaterial::Toon),
        ("enamel", RadialMaterial::Enamel),
        ("brushed-metal", RadialMaterial::BrushedMetal),
        ("paper", RadialMaterial::Paper),
        ("hatch", RadialMaterial::Hatch),
        ("pearl", RadialMaterial::Pearl),
    ] {
        let mut c = base();
        c.title = name.to_uppercase();
        c.style.material = material;
        if material == RadialMaterial::Toon {
            c.style.outline.rim = true;
            c.style.outline.separators = true;
        }
        if matches!(
            material,
            RadialMaterial::Hatch | RadialMaterial::Pearl | RadialMaterial::Paper
        ) {
            c.style.texture_strength = 0.7;
        }
        cases.push((name.to_owned(), c));
    }
    for (name, amount) in [("corners-sharp", 0.0), ("corners-rounded", 0.12)] {
        let mut c = base();
        c.title = name.into();
        c.style.inner_corner = amount;
        c.style.outer_corner = amount;
        c.style.depth = 0.0;
        c.style.tilt_degrees = 0.0;
        c.style.shadow = false;
        cases.push((name.into(), c));
    }
    let mut edited = base();
    edited.title = "Selected slice / individual edits".into();
    edited.interaction.selected = Some(RadialTarget::main(0));
    let style = RadialStyle {
        material: RadialMaterial::Ceramic,
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
    let mut split = base();
    split.title = "Regional share / Video breakdown".into();
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
    // Deterministic time samples: host calls the same model transition on pointer events.
    let mut hover = base();
    hover.title = "Hover lift / Search".into();
    hover.style.material = RadialMaterial::Ceramic;
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
    for (name, chart) in cases {
        let img = renderer.export_rgba(&chart, (900, 620), 1.0)?;
        std::fs::write(directory.join(format!("{name}.png")), encode_png(&img)?)?;
        renderer.end_frame();
        gpu.device().poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })?;
        eprintln!("{name}: {}x{}", img.width, img.height);
    }
    Ok(())
}

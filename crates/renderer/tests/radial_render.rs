#![cfg(not(target_arch = "wasm32"))]
use renderer::{Color, RendererDevice, radial::*};
use std::sync::{Arc, OnceLock};
// A test may register fonts or poll the shared GPU and execute another test's
// completion callbacks. Serialize these shared resources, not production work.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn gpu() -> RendererDevice {
    static GPU: OnceLock<RendererDevice> = OnceLock::new();
    GPU.get_or_init(|| {
        let instance = renderer::data_render::create_instance();
        let adapter =
            renderer::data_render::request_adapter(&instance).expect("native GPU adapter required");
        let (d, q) = renderer::data_render::request_device(&adapter).unwrap();
        RendererDevice::new(Arc::new(d), Arc::new(q))
    })
    .clone()
}
fn chart() -> RadialChart {
    RadialChart {
        labels: RadialLabels::None,
        slices: vec![
            RadialSlice::new("A", 25.0, Color::new(1.0, 0.0, 0.0, 1.0)),
            RadialSlice::new("B", 75.0, Color::new(0.0, 0.0, 1.0, 1.0)),
        ],
        ..Default::default()
    }
}
fn renderer() -> RadialRenderer {
    RadialRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap()
}
fn drain(r: &RadialRenderer) {
    r.end_frame();
    gpu()
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
}
#[test]
fn radial_proportions_hole_zero_and_full_circle_are_correct() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    let image = r.export_rgba(&c, (320, 320), 1.0).unwrap();
    let red = image
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0] > 250 && p[1] < 5 && p[2] < 5)
        .count();
    let blue = image
        .rgba
        .chunks_exact(4)
        .filter(|p| p[2] > 250 && p[1] < 5 && p[0] < 5)
        .count();
    assert!((red as f64 / (red + blue) as f64 - 0.25).abs() < 0.005);
    c.kind = RadialKind::Donut { inner_radius: 0.5 };
    drain(&r);
    let ring = r.export_rgba(&c, (320, 320), 1.0).unwrap();
    assert_eq!(
        &ring.rgba[(160 * 320 + 160) * 4..][..4],
        &[255, 255, 255, 255]
    );
    c.kind = RadialKind::Pie;
    c.start_angle_degrees = f32::MAX;
    c.slices[0].value = 0.0;
    drain(&r);
    let full = r.export_rgba(&c, (320, 320), 1.0).unwrap();
    c.slices[0].explode = 0.35;
    drain(&r);
    let hidden = r.export_rgba(&c, (320, 320), 1.0).unwrap();
    assert!(
        hidden.rgba == full.rgba,
        "zero-value slice changed the layout"
    );
    for y in 100..220 {
        for x in 100..220 {
            assert_eq!(
                &full.rgba[(y * 320 + x) * 4..][..4],
                &[0, 0, 255, 255],
                "full-circle wedge has a seam at {x},{y}"
            );
        }
    }
}
#[test]
fn radial_snapshot_reuse_and_each_visible_edit_updates_pixels() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.title = "Radial title".into();
    c.labels = RadialLabels::Outside;
    c.font_size = 12.0;
    c.kind = RadialKind::Donut { inner_radius: 0.4 };
    c.style.material = RadialMaterial::Ceramic;
    c.style.tilt_degrees = 25.0;
    c.style.depth = 0.18;
    c.style.texture_strength = 0.8;
    let original = r.prepare(&c, (480, 320), 1.0).unwrap();
    let usage = r.gpu_memory_usage();
    assert!(Arc::ptr_eq(
        &original,
        &r.prepare(&c, (480, 320), 1.0).unwrap()
    ));
    assert_eq!(r.gpu_memory_usage(), usage);
    let before = r.export_rgba(&c, (480, 320), 1.0).unwrap().rgba;
    drain(&r);
    renderer::text_render::register_font_bytes(
        include_bytes!("../fonts/ComicNeue-Regular.ttf").to_vec(),
    )
    .unwrap();
    let fonts_updated = r.prepare(&c, (480, 320), 1.0).unwrap();
    assert!(
        !Arc::ptr_eq(&original, &fonts_updated),
        "new font registration reused stale labels"
    );
    drop(fonts_updated);
    drain(&r);
    let edits: Vec<(&str, Box<dyn Fn(&mut RadialChart)>)> = vec![
        ("value", Box::new(|c| c.slices[0].value = 40.0)),
        (
            "color",
            Box::new(|c| c.slices[0].color = Color::from_rgb8(30, 180, 120)),
        ),
        ("explode", Box::new(|c| c.slices[0].explode = 0.18)),
        ("kind", Box::new(|c| c.kind = RadialKind::Pie)),
        ("angle", Box::new(|c| c.start_angle_degrees += 37.0)),
        (
            "material",
            Box::new(|c| c.style.material = RadialMaterial::Wood),
        ),
        ("tilt", Box::new(|c| c.style.tilt_degrees = 45.0)),
        ("depth", Box::new(|c| c.style.depth = 0.3)),
        ("bevel", Box::new(|c| c.style.bevel = 0.08)),
        ("gap", Box::new(|c| c.style.gap_degrees = 5.0)),
        ("roughness", Box::new(|c| c.style.roughness = 0.9)),
        (
            "texture strength",
            Box::new(|c| c.style.texture_strength = 0.0),
        ),
        ("texture scale", Box::new(|c| c.style.texture_scale = 3.0)),
        ("light", Box::new(|c| c.style.light = [0.8, 0.5, 0.4])),
        ("shadow", Box::new(|c| c.style.shadow = true)),
        ("title", Box::new(|c| c.title = "Updated title".into())),
        ("labels", Box::new(|c| c.labels = RadialLabels::None)),
        ("font", Box::new(|c| c.font_family = "Comic Neue".into())),
        ("font size", Box::new(|c| c.font_size = 16.0)),
        (
            "label color",
            Box::new(|c| c.label_color = Color::from_rgb8(120, 20, 180)),
        ),
        (
            "background",
            Box::new(|c| c.background = Color::from_rgb8(236, 240, 247)),
        ),
    ];
    for (name, edit) in edits {
        let mut changed = c.clone();
        edit(&mut changed);
        let frame = r.prepare(&changed, (480, 320), 1.0).unwrap();
        assert!(!Arc::ptr_eq(&original, &frame), "stale {name} snapshot");
        let after = r.export_rgba(&changed, (480, 320), 1.0).unwrap().rgba;
        assert!(before != after, "{name} did not update visible pixels");
        drop(frame);
        drain(&r);
    }
    let resized = r.prepare(&c, (500, 320), 1.0).unwrap();
    assert_eq!(resized.size(), (500, 320));
    let scaled = r.prepare(&c, (480, 320), 2.0).unwrap();
    assert_eq!(scaled.size(), (960, 640));
    assert!(!Arc::ptr_eq(&original, &scaled));
}
#[test]
fn radial_budget_validation_and_retirement_are_transactional() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let c = chart();
    let good = r.prepare(&c, (320, 320), 1.0).unwrap();
    let before = r.gpu_memory_usage();
    let mut invalid = c.clone();
    invalid.slices[0].value = f64::NAN;
    assert!(r.prepare(&invalid, (320, 320), 1.0).is_err());
    assert_eq!(r.gpu_memory_usage(), before);
    r.set_memory_budget(before.total_bytes());
    let mut edit = c.clone();
    edit.style.material = RadialMaterial::Paper;
    assert!(matches!(
        r.prepare(&edit, (320, 320), 1.0),
        Err(RadialError::Budget { .. })
    ));
    assert_eq!(r.gpu_memory_usage(), before);
    assert!(Arc::ptr_eq(&good, &r.prepare(&c, (320, 320), 1.0).unwrap()));
    r.clear_cache();
    drop(good);
    drain(&r);
    assert_eq!(r.gpu_memory_usage().total_bytes(), 0);
}
#[test]
fn radial_export_formats_preserve_straight_alpha_and_png_dimensions() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = chart();
    c.background = Color::new(0.4, 0.6, 0.8, 0.5);
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        let mut r = RadialRenderer::new(gpu(), format).unwrap();
        let image = r.export_rgba(&c, (320, 240), 1.5).unwrap();
        assert_eq!((image.width, image.height), (480, 360));
        for (actual, expected) in image.rgba[..4].iter().zip([102u8, 153, 204, 128]) {
            assert!(
                actual.abs_diff(expected) <= 2,
                "wrong straight alpha for {format:?}: {:?}",
                &image.rgba[..4]
            );
        }
        let bytes = renderer::encode_png(&image).unwrap();
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; decoder.output_buffer_size().unwrap()];
        let info = decoder.next_frame(&mut decoded).unwrap();
        assert_eq!((info.width, info.height), (480, 360));
        assert_eq!(&decoded[..info.buffer_size()], &image.rgba);
        drain(&r);
    }
}
#[test]
fn radial_detail_and_all_materials_render_and_change() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = chart();
    c.labels = RadialLabels::Inside;
    c.font_size = 12.0;
    c.split = Some(RadialSplit {
        slice_index: 1,
        kind: RadialKind::Pie,
        children: vec![
            RadialSlice::new("D", 25.0, Color::from_rgb8(0, 200, 150)),
            RadialSlice::new("E", 50.0, Color::from_rgb8(220, 180, 80)),
        ],
    });
    let mut r = renderer();
    let mut images = Vec::new();
    for material in [
        RadialMaterial::Flat,
        RadialMaterial::Matte,
        RadialMaterial::Ceramic,
        RadialMaterial::BrushedMetal,
        RadialMaterial::Paper,
        RadialMaterial::Wood,
        RadialMaterial::SatinMetal,
        RadialMaterial::Toon,
        RadialMaterial::Enamel,
        RadialMaterial::Hatch,
        RadialMaterial::Pearl,
    ] {
        c.style.material = material;
        c.style.texture_strength = 0.8;
        c.style.tilt_degrees = 30.0;
        c.style.depth = 0.18;
        let img = r.export_rgba(&c, (640, 400), 1.0).unwrap();
        assert!(
            img.rgba
                .chunks_exact(4)
                .filter(|p| p[1] > 100 && p[0] < 80 && p[2] > 30)
                .count()
                > 500,
            "detail chart disappeared"
        );
        assert!(
            images.iter().all(|old| old != &img.rgba),
            "material {material:?} not applied"
        );
        images.push(img.rgba);
        drain(&r);
    }
    c.split.as_mut().unwrap().children[0].value = 100.0;
    assert!(r.prepare(&c, (640, 400), 1.0).is_err());
}

#[test]
fn radial_editing_options_invalidate_pixels_but_hover_reuses_labels() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.kind = RadialKind::Donut { inner_radius: 0.5 };
    c.labels = RadialLabels::Outside;
    c.style.material = RadialMaterial::Ceramic;
    c.style.tilt_degrees = 32.0;
    c.style.depth = 0.18;
    c.style.bevel = 0.025;
    c.style.gap_degrees = 2.0;
    let baseline = r.export_rgba(&c, (480, 320), 1.0).unwrap().rgba;
    drain(&r);
    let edits: Vec<(&str, Box<dyn Fn(&mut RadialChart)>)> = vec![
        ("outer corner", Box::new(|c| c.style.outer_corner = 0.15)),
        ("inner corner", Box::new(|c| c.style.inner_corner = 0.15)),
        (
            "rim",
            Box::new(|c| {
                c.style.outline.rim = true;
                c.style.outline.width = 3.0;
            }),
        ),
        (
            "separators",
            Box::new(|c| {
                c.style.outline.separators = true;
                c.style.outline.width = 3.0;
            }),
        ),
        ("gloss", Box::new(|c| c.style.gloss = 0.0)),
        (
            "selected",
            Box::new(|c| c.interaction.selected = Some(RadialTarget::main(0))),
        ),
        (
            "individual style",
            Box::new(|c| {
                c.slices[0].style = Some(RadialStyle {
                    material: RadialMaterial::Toon,
                    ..c.style.clone()
                })
            }),
        ),
        (
            "individual label format",
            Box::new(|c| c.slices[0].label_format = Some(RadialLabelFormat::Value)),
        ),
        (
            "individual label placement",
            Box::new(|c| c.slices[0].labels = Some(RadialLabels::None)),
        ),
        (
            "individual label color",
            Box::new(|c| c.slices[0].label_color = Some(Color::from_rgb8(0, 150, 70))),
        ),
        (
            "label format",
            Box::new(|c| c.label_format = RadialLabelFormat::Name),
        ),
        (
            "value suffix",
            Box::new(|c| {
                c.label_format = RadialLabelFormat::NameValue;
                c.value_suffix = " kg".into();
            }),
        ),
    ];
    for (name, edit) in edits {
        let mut changed = c.clone();
        edit(&mut changed);
        let frame = r.prepare(&changed, (480, 320), 1.0).unwrap();
        let again = r.prepare(&changed, (480, 320), 1.0).unwrap();
        assert!(
            Arc::ptr_eq(&frame, &again),
            "identical {name} did not reuse"
        );
        assert_ne!(
            r.export_rgba(&changed, (480, 320), 1.0).unwrap().rgba,
            baseline,
            "{name}"
        );
        drop(frame);
        drop(again);
        drain(&r);
    }
    let rest = r.prepare(&c, (480, 320), 1.0).unwrap();
    let mut maximum = 0;
    for step in 0..40 {
        c.interaction.hovered = Some(RadialTarget::main(0));
        c.interaction.hover_progress = (step % 10) as f32 / 9.0;
        let frame = r.prepare(&c, (480, 320), 1.0).unwrap();
        assert!(rest.shares_annotations_with(&frame), "hover rebuilt labels");
        drop(frame);
        drain(&r);
        maximum = maximum.max(r.gpu_memory_usage().total_bytes());
    }
    assert!(
        maximum < 480 * 320 * 4 + 16_384,
        "hover leaked resources: {maximum}"
    );
    c.interaction = RadialInteraction::default();
    assert_eq!(r.export_rgba(&c, (480, 320), 1.0).unwrap().rgba, baseline);
}

#[test]
fn radial_picking_matches_visible_rounded_lifted_surfaces() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.kind = RadialKind::Donut { inner_radius: 0.5 };
    c.style.inner_corner = 0.12;
    c.style.outer_corner = 0.12;
    c.style.tilt_degrees = 38.0;
    c.style.depth = 0.2;
    c.style.bevel = 0.02;
    c.style.gap_degrees = 5.0;
    c.style.hover_brightness = 0.0;
    c.interaction.selected = Some(RadialTarget::main(0));
    let frame = r.prepare(&c, (320, 320), 1.0).unwrap();
    let img = r.export_rgba(&c, (320, 320), 1.0).unwrap();
    let mut hits = [0; 2];
    for y in (10..310).step_by(3) {
        for x in (10..310).step_by(3) {
            let p = &img.rgba[(y * 320 + x) * 4..][..3];
            let expected = if p == [255, 0, 0] {
                Some(0)
            } else if p == [0, 0, 255] {
                Some(1)
            } else {
                None
            };
            if let Some(index) = expected {
                assert_eq!(
                    frame.hit_test([x as f32 + 0.5, y as f32 + 0.5]),
                    Some(RadialTarget::main(index)),
                    "wrong pick at {x},{y}"
                );
                hits[index] += 1;
            }
        }
    }
    assert!(hits.into_iter().all(|n| n > 100));
    assert_eq!(frame.hit_test([160.0, 160.0]), None, "hole must not pick");
    assert_eq!(frame.hit_test([f32::NAN, 0.0]), None);
    assert_eq!(frame.hit_test([-1.0, 0.0]), None);
    // Color edits target the original index, including zero-value entries.
    c.slices
        .insert(0, RadialSlice::new("hidden", 0.0, Color::WHITE));
    c.interaction.selected = Some(RadialTarget::main(1));
    drain(&r);
    let shifted = r.prepare(&c, (320, 320), 1.0).unwrap();
    let mut found = false;
    for y in (20..300).step_by(10) {
        for x in (20..300).step_by(10) {
            if frame.hit_test([x as f32, y as f32]) == Some(RadialTarget::main(0)) {
                assert_eq!(
                    shifted.hit_test([x as f32, y as f32]),
                    Some(RadialTarget::main(1))
                );
                found = true;
            }
        }
    }
    assert!(found);
}

#[test]
fn radial_rounding_changes_geometry_and_outline_is_resolution_independent() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.kind = RadialKind::Donut { inner_radius: 0.5 };
    c.style.gap_degrees = 4.0;
    let sharp = r.export_rgba(&c, (320, 320), 1.0).unwrap();
    drain(&r);
    c.style.inner_corner = 0.15;
    c.style.outer_corner = 0.15;
    let rounded = r.export_rgba(&c, (320, 320), 1.0).unwrap();
    drain(&r);
    let colored =
        |im: &renderer::RasterImage| im.rgba.chunks_exact(4).filter(|p| p[1] < 50).count();
    assert!(
        colored(&sharp) > colored(&rounded) + 50,
        "rounding must cut real corners"
    );
    c.style.outline = RadialOutline {
        rim: true,
        separators: true,
        width: 2.0,
        ..Default::default()
    };
    let one = r.export_rgba(&c, (320, 320), 1.0).unwrap();
    drain(&r);
    let two = r.export_rgba(&c, (320, 320), 2.0).unwrap();
    drain(&r);
    c.style.outline.rim = false;
    c.style.outline.separators = false;
    let plain2 = r.export_rgba(&c, (320, 320), 2.0).unwrap();
    // Integrate coverage rather than thresholding partially antialiased pixels.
    let ink = |plain: &renderer::RasterImage, outlined: &renderer::RasterImage| -> f64 {
        plain
            .rgba
            .chunks_exact(4)
            .zip(outlined.rgba.chunks_exact(4))
            .map(|(a, b)| {
                (a[..3].iter().map(|v| f64::from(*v)).sum::<f64>()
                    - b[..3].iter().map(|v| f64::from(*v)).sum::<f64>())
                .max(0.0)
            })
            .sum()
    };
    let ratio = ink(&plain2, &two) / ink(&rounded, &one);
    assert!(
        (3.3..4.8).contains(&ratio),
        "outline scales as a physical hairline: {ratio}"
    );
}

#[test]
fn radial_secondary_controls_and_detail_edits_are_not_stale() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.labels = RadialLabels::Outside;
    c.label_format = RadialLabelFormat::NameValuePercent;
    c.slices[0].value = 25.1234;
    c.kind = RadialKind::Donut { inner_radius: 0.4 };
    c.style.material = RadialMaterial::Ceramic;
    c.style.depth = 0.2;
    c.style.tilt_degrees = 35.0;
    c.style.inner_corner = 0.05;
    c.style.outer_corner = 0.05;
    c.style.outline = RadialOutline {
        rim: true,
        separators: true,
        emphasis: true,
        width: 1.5,
        ..Default::default()
    };
    c.interaction.hovered = Some(RadialTarget::main(0));
    c.interaction.hover_progress = 0.5;
    let before = r.export_rgba(&c, (500, 360), 1.0).unwrap().rgba;
    drain(&r);
    let edits: Vec<(&str, Box<dyn Fn(&mut RadialChart)>)> = vec![
        ("decimals", Box::new(|c| c.label_decimals = 3)),
        (
            "hover progress",
            Box::new(|c| c.interaction.hover_progress = 1.0),
        ),
        ("hover lift", Box::new(|c| c.style.hover_lift = 0.2)),
        (
            "hover brightness",
            Box::new(|c| c.style.hover_brightness = 0.4),
        ),
        ("outline width", Box::new(|c| c.style.outline.width = 4.0)),
        (
            "outline color",
            Box::new(|c| c.style.outline.color = Color::from_rgb8(200, 80, 10)),
        ),
        (
            "outline opacity",
            Box::new(|c| c.style.outline.color.a = 0.1),
        ),
        (
            "per slice light",
            Box::new(|c| {
                c.slices[0].style = Some(RadialStyle {
                    light: [1.0, 1.0, 0.2],
                    ..c.style.clone()
                })
            }),
        ),
        (
            "per slice shadow",
            Box::new(|c| {
                c.slices[0].style = Some(RadialStyle {
                    shadow: true,
                    ..c.style.clone()
                })
            }),
        ),
    ];
    for (name, edit) in edits {
        let mut changed = c.clone();
        edit(&mut changed);
        assert_ne!(
            r.export_rgba(&changed, (500, 360), 1.0).unwrap().rgba,
            before,
            "{name}"
        );
        drain(&r);
    }
    c.style.material = RadialMaterial::Paper;
    c.style.texture_strength = 0.8;
    let paper = r.export_rgba(&c, (500, 360), 1.0).unwrap().rgba;
    drain(&r);
    c.style.texture_angle_degrees = 90.0;
    assert_ne!(r.export_rgba(&c, (500, 360), 1.0).unwrap().rgba, paper);
    drain(&r);
    c.interaction = RadialInteraction::default();
    c.labels = RadialLabels::None;
    c.split = Some(RadialSplit {
        slice_index: 1,
        kind: RadialKind::Donut { inner_radius: 0.4 },
        children: vec![
            RadialSlice::new("D", 25.0, Color::from_rgb8(30, 180, 90)),
            RadialSlice::new("E", 50.0, Color::from_rgb8(240, 180, 30)),
        ],
    });
    let original = r.export_rgba(&c, (640, 360), 1.0).unwrap();
    drain(&r);
    c.slice_mut(RadialTarget::detail(0)).unwrap().color = Color::from_rgb8(200, 40, 200);
    let edited = r.export_rgba(&c, (640, 360), 1.0).unwrap();
    for y in 0..360 {
        assert_eq!(
            &original.rgba[y * 640 * 4..(y * 640 + 300) * 4],
            &edited.rgba[y * 640 * 4..(y * 640 + 300) * 4],
            "detail color changed main chart"
        );
    }
    assert_ne!(original.rgba, edited.rgba);
}

// Exhaustive destructuring makes future SSOT additions require an explicit review
// of the editing/reuse/pixel contract above (no `..` silently accepting fields).
#[test]
fn radial_ssot_fields_are_explicit() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let RadialChart {
        title: _,
        slices: _,
        kind: _,
        start_angle_degrees: _,
        split: _,
        style: _,
        labels: _,
        label_format: _,
        label_decimals: _,
        value_suffix: _,
        interaction: _,
        font_family: _,
        font_size: _,
        label_color: _,
        background: _,
    } = RadialChart::default();
    let RadialStyle {
        material: _,
        tilt_degrees: _,
        depth: _,
        bevel: _,
        gap_degrees: _,
        roughness: _,
        texture_strength: _,
        texture_scale: _,
        light: _,
        shadow: _,
        inner_corner: _,
        outer_corner: _,
        outline: _,
        hover_lift: _,
        hover_brightness: _,
        gloss: _,
        texture_angle_degrees: _,
    } = RadialStyle::default();
    let RadialSlice {
        label: _,
        value: _,
        color: _,
        explode: _,
        style: _,
        labels: _,
        label_format: _,
        label_color: _,
    } = RadialSlice::new("A", 1.0, Color::WHITE);
    let RadialOutline {
        rim: _,
        separators: _,
        emphasis: _,
        width: _,
        color: _,
    } = RadialOutline::default();
    let RadialInteraction {
        hovered: _,
        selected: _,
        hover_progress: _,
    } = RadialInteraction::default();
}

#[test]
fn radial_inside_labels_follow_lift_without_raster_reallocation() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = RadialChart {
        background: Color::new(0.0, 0.0, 0.0, 1.0),
        labels: RadialLabels::Inside,
        label_format: RadialLabelFormat::Name,
        slices: vec![RadialSlice::new(
            "AAA",
            100.0,
            Color::from_rgb8(10, 30, 100),
        )],
        ..Default::default()
    };
    c.style.tilt_degrees = 40.0;
    c.style.depth = 0.1;
    c.style.hover_lift = 0.15;
    c.style.hover_brightness = 0.0;
    let rest = r.prepare(&c, (320, 320), 1.0).unwrap();
    let a = r.export_rgba(&c, (320, 320), 1.0).unwrap();
    drain(&r);
    c.interaction.selected = Some(RadialTarget::main(0));
    let lifted = r.prepare(&c, (320, 320), 1.0).unwrap();
    assert!(rest.shares_annotations_with(&lifted));
    let b = r.export_rgba(&c, (320, 320), 1.0).unwrap();
    let centroid = |im: &renderer::RasterImage| {
        let mut count = 0.0;
        let mut y = 0.0;
        for (i, p) in im.rgba.chunks_exact(4).enumerate() {
            let ink = f64::from(p[0].saturating_sub(15));
            count += ink;
            y += ink * (i / im.width as usize) as f64;
        }
        (y / count, count)
    };
    let (ay, ac) = centroid(&a);
    let (by, bc) = centroid(&b);
    assert!(
        ay - by > 8.0,
        "inside text did not move with slice: {ay} -> {by}"
    );
    assert!(
        (bc / ac - 1.0).abs() < 0.06,
        "glyphs duplicated or lost: {ac} -> {bc}"
    );
}

#[test]
fn radial_paper_wall_grain_varies_with_height_instead_of_extruding_stripes() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = RadialChart {
        labels: RadialLabels::None,
        kind: RadialKind::Donut { inner_radius: 0.45 },
        slices: vec![RadialSlice::new(
            "Paper",
            100.0,
            Color::from_rgb8(180, 180, 180),
        )],
        ..Default::default()
    };
    c.style = RadialStyle {
        material: RadialMaterial::Paper,
        depth: 0.4,
        tilt_degrees: 55.0,
        bevel: 0.0,
        texture_strength: 1.0,
        shadow: false,
        hover_lift: 0.0,
        ..Default::default()
    };
    let plain = {
        let mut plain = c.clone();
        plain.style.texture_strength = 0.0;
        r.export_rgba(&plain, (512, 512), 1.0).unwrap()
    };
    drain(&r);
    let grain = r.export_rgba(&c, (512, 512), 1.0).unwrap();
    // Locate the front cylindrical wall from the silhouette, then measure ONLY
    // the texture residual against an otherwise identical untextured render.
    // At fixed x the cylinder's x/y and normal are constant; height is different.
    let x = 256usize;
    let bottom = (0..512)
        .rev()
        .find(|&y| plain.rgba[(y * 512 + x) * 4] < 230)
        .unwrap();
    let mut horizontal = 0.0;
    let mut vertical = 0.0;
    let mut n = 0;
    let residual = |x: usize, y: usize| -> f64 {
        let i = (y * 512 + x) * 4;
        f64::from(grain.rgba[i]) - f64::from(plain.rgba[i])
    };
    for y in bottom - 36..bottom - 10 {
        for x in 246..266 {
            let p = residual(x, y);
            horizontal += (p - residual(x + 1, y)).powi(2);
            vertical += (p - residual(x, y + 1)).powi(2);
            n += 1;
        }
    }
    assert!(
        horizontal > 1.0,
        "paper grain was removed instead of corrected"
    );
    assert!(
        vertical > horizontal * 0.12,
        "height-invariant paper stripes: horizontal={horizontal}, vertical={vertical}"
    );
    assert!(
        vertical / (n as f64) > 0.05,
        "no visible grain along wall height"
    );
}

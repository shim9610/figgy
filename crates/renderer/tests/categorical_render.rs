#![cfg(not(target_arch = "wasm32"))]
use renderer::{Color, RendererDevice, categorical::*};
use std::sync::{Arc, OnceLock};
// A test may register fonts or poll the shared GPU and execute another test's
// completion callbacks. Serialize these shared resources, not production work.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn gpu() -> RendererDevice {
    static GPU: OnceLock<RendererDevice> = OnceLock::new();
    GPU.get_or_init(|| {
        let instance = renderer::data_render::create_instance();
        let adapter =
            renderer::data_render::request_adapter(&instance).expect("native GPU required");
        let (d, q) = renderer::data_render::request_device(&adapter).unwrap();
        RendererDevice::new(Arc::new(d), Arc::new(q))
    })
    .clone()
}
fn renderer() -> CategoricalRenderer {
    CategoricalRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap()
}
fn chart() -> CategoricalChart {
    CategoricalChart {
        categories: vec![Category::new("a", "A"), Category::new("b", "B")],
        series: vec![
            BarSeries::new(
                "one",
                "One",
                vec![Some(10.), Some(20.)],
                Color::from_rgb8(220, 30, 50),
            ),
            BarSeries::new(
                "two",
                "Two",
                vec![Some(20.), Some(10.)],
                Color::from_rgb8(30, 80, 220),
            ),
        ],
        labels: CategoryBarLabels::None,
        grid: false,
        legend: false,
        ..Default::default()
    }
}
fn drain(r: &CategoricalRenderer) {
    r.end_frame();
    gpu()
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
}
fn rgba(image: &renderer::RasterImage, x: u32, y: u32) -> [u8; 4] {
    image.rgba[((y * image.width + x) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}
fn center(rect: [f32; 4]) -> [f32; 2] {
    [(rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5]
}
#[test]
fn proportions_orientation_and_signed_stacks_match_pixels() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.style.corner_radius = 0.;
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        c.direction = direction;
        let frame = r.prepare(&c, (640, 440), 1.).unwrap();
        let image = r.export_rgba(&c, (640, 440), 1.).unwrap();
        let a = frame.bar_rect(&CategoryBarTarget::new("a", "one")).unwrap();
        let b = frame.bar_rect(&CategoryBarTarget::new("a", "two")).unwrap();
        let len = |v: [f32; 4]| {
            if direction == CategoryBarDirection::Vertical {
                v[3] - v[1]
            } else {
                v[2] - v[0]
            }
        };
        assert!((len(b) / len(a) - 2.).abs() < 1e-5);
        for (id, expected) in [("one", [220, 30, 50, 255]), ("two", [30, 80, 220, 255])] {
            let t = CategoryBarTarget::new("a", id);
            let p = center(frame.bar_rect(&t).unwrap());
            assert_eq!(frame.hit_test(p), Some(t));
            assert_eq!(rgba(&image, p[0] as u32, p[1] as u32), expected);
        }
        drop(frame);
        drain(&r);
    }
    c.direction = CategoryBarDirection::Vertical;
    c.mode = CategoryBarMode::Stacked;
    c.series[1].values[0] = Some(-20.);
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let a = f.bar_rect(&CategoryBarTarget::new("a", "one")).unwrap();
    let b = f.bar_rect(&CategoryBarTarget::new("a", "two")).unwrap();
    assert!(
        (a[3] - b[1]).abs() < 0.01,
        "signed stacks must share the zero baseline"
    );
}
#[test]
fn percent_stack_endpoints_and_internal_corners_are_exact() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.mode = CategoryBarMode::PercentStacked;
    c.direction = CategoryBarDirection::Horizontal;
    c.style.corner_radius = 24.;
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let image = r.export_rgba(&c, (640, 440), 1.).unwrap();
    let first = f.bar_rect(&CategoryBarTarget::new("a", "one")).unwrap();
    let second = f.bar_rect(&CategoryBarTarget::new("a", "two")).unwrap();
    let other = f.bar_rect(&CategoryBarTarget::new("b", "two")).unwrap();
    assert_eq!(first[2], second[0]);
    assert_eq!(second[2], other[2]);
    let y = first[1].ceil() as u32 + 2;
    let x = first[2] as u32;
    assert_eq!(rgba(&image, x - 2, y), [220, 30, 50, 255]);
    assert_eq!(rgba(&image, x + 2, y), [30, 80, 220, 255]);
    assert!(
        f.hit_test([second[2] - 0.5, second[1] + 0.5]).is_none(),
        "exposed end must be rounded"
    );
    assert!(
        f.hit_test([first[2] - 0.5, first[1] + 0.5]).is_some(),
        "internal corner must stay square"
    );
}
#[test]
fn missing_zero_identity_and_individual_edits_are_isolated() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    let target = CategoryBarTarget::new("a", "one");
    c.series[0].values[1] = None;
    c.series[1].values[1] = Some(0.);
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    assert!(f.bar_rect(&CategoryBarTarget::new("b", "one")).is_none());
    assert!(f.bar_rect(&CategoryBarTarget::new("b", "two")).is_none());
    let mut o = CategoryBarOverride::new(target.clone());
    o.color = Some(Color::from_rgb8(10, 200, 70));
    c.overrides.push(o);
    c.selected = Some(target.clone());
    c.style.emphasis_brightness = 0.;
    let image = r.export_rgba(&c, (640, 440), 1.).unwrap();
    let p = center(f.bar_rect(&target).unwrap());
    assert_eq!(rgba(&image, p[0] as u32, p[1] as u32), [10, 200, 70, 255]);
    c.reorder_categories(&["b", "a"]).unwrap();
    let g = r.prepare(&c, (640, 440), 1.).unwrap();
    let p = center(g.bar_rect(&target).unwrap());
    assert_eq!(g.hit_test(p), Some(target));
    assert_ne!(
        f.bar_rect(&CategoryBarTarget::new("a", "one")),
        g.bar_rect(&CategoryBarTarget::new("a", "one"))
    );
}
#[test]
fn every_visible_ssot_edit_updates_and_identical_snapshot_reuses() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.title = "Categories".into();
    c.labels = CategoryBarLabels::Auto;
    c.legend = true;
    c.grid = true;
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let usage = r.gpu_memory_usage();
    assert!(Arc::ptr_eq(&f, &r.prepare(&c, (640, 440), 1.).unwrap()));
    assert_eq!(usage, r.gpu_memory_usage());
    let before = r.export_rgba(&c, (640, 440), 1.).unwrap().rgba;
    drain(&r);
    let edits: Vec<(&str, Box<dyn Fn(&mut CategoricalChart)>)> = vec![
        ("value", Box::new(|c| c.series[0].values[0] = Some(13.))),
        (
            "category label",
            Box::new(|c| c.categories[0].label = "Changed".into()),
        ),
        (
            "series label",
            Box::new(|c| c.series[0].label = "Changed".into()),
        ),
        (
            "color",
            Box::new(|c| c.series[0].color = Color::from_rgb8(10, 200, 70)),
        ),
        ("title", Box::new(|c| c.title = "Other".into())),
        ("mode", Box::new(|c| c.mode = CategoryBarMode::Stacked)),
        (
            "direction",
            Box::new(|c| c.direction = CategoryBarDirection::Horizontal),
        ),
        ("grid", Box::new(|c| c.grid = false)),
        ("legend", Box::new(|c| c.legend = false)),
        ("width", Box::new(|c| c.group_width = 0.5)),
        ("gap", Box::new(|c| c.bar_gap = 12.)),
        ("font size", Box::new(|c| c.font_size = 18.)),
        (
            "text color",
            Box::new(|c| c.text_color = Color::from_rgb8(170, 50, 20)),
        ),
        (
            "background",
            Box::new(|c| c.background = Color::from_rgb8(240, 245, 250)),
        ),
        (
            "value title",
            Box::new(|c| c.value_title = "Response".into()),
        ),
        (
            "category title",
            Box::new(|c| c.category_title = "Group".into()),
        ),
        (
            "label placement",
            Box::new(|c| c.labels = CategoryBarLabels::Inside),
        ),
        (
            "label format",
            Box::new(|c| c.label_format = CategoryBarLabelFormat::ValuePercent),
        ),
        ("suffix", Box::new(|c| c.value_suffix = " kg".into())),
        (
            "material",
            Box::new(|c| c.style.material = CategoryBarMaterial::SatinMetal),
        ),
        ("rounding", Box::new(|c| c.style.corner_radius = 25.)),
        ("outline", Box::new(|c| c.style.outline = true)),
        (
            "hover",
            Box::new(|c| c.hovered = Some(CategoryBarTarget::new("a", "one"))),
        ),
        (
            "selection",
            Box::new(|c| c.selected = Some(CategoryBarTarget::new("b", "two"))),
        ),
        (
            "per-bar label",
            Box::new(|c| {
                let mut o = CategoryBarOverride::new(CategoryBarTarget::new("a", "one"));
                o.label_format = Some(CategoryBarLabelFormat::Percent);
                c.overrides.push(o);
            }),
        ),
    ];
    for (name, edit) in edits {
        let mut changed = c.clone();
        edit(&mut changed);
        let g = r.prepare(&changed, (640, 440), 1.).unwrap();
        assert!(!Arc::ptr_eq(&f, &g), "stale {name}");
        let after = r.export_rgba(&changed, (640, 440), 1.).unwrap();
        assert_ne!(before, after.rgba, "unchanged pixels for {name}");
        drop(g);
        drain(&r);
    }
}
#[test]
fn materials_geometry_and_interaction_share_annotations_without_unbounded_growth() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.labels = CategoryBarLabels::Inside;
    let original = r.prepare(&c, (640, 440), 1.).unwrap();
    let rect = original.bar_rect(&CategoryBarTarget::new("a", "one"));
    for i in 0..30 {
        c.style.material = match i % 5 {
            0 => CategoryBarMaterial::Flat,
            1 => CategoryBarMaterial::Matte,
            2 => CategoryBarMaterial::SatinMetal,
            3 => CategoryBarMaterial::Enamel,
            _ => CategoryBarMaterial::Paper,
        };
        c.style.outline = i % 2 == 0;
        c.style.corner_radius = (i % 20) as f32;
        c.hovered = Some(CategoryBarTarget::new(
            "a",
            if i % 2 == 0 { "one" } else { "two" },
        ));
        let next = r.prepare(&c, (640, 440), 1.).unwrap();
        assert!(next.shares_annotations_with(&original));
        assert_eq!(next.bar_rect(&CategoryBarTarget::new("a", "one")), rect);
        drop(next);
        drain(&r);
    }
    assert_eq!(
        r.gpu_memory_usage()
            .creations_of(renderer::GpuResourceKind::PanelTexture),
        1
    );
    assert!(r.gpu_memory_usage().total_bytes() < 640 * 440 * 8 + 4096);
    drop(original);
    r.clear_cache();
    drain(&r);
    assert_eq!(r.gpu_memory_usage().total_bytes(), 0);
}
#[test]
fn invalid_input_layout_and_budget_keep_last_valid_frame() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let c = chart();
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let usage = r.gpu_memory_usage();
    let mut bad = c.clone();
    bad.series[0].values.pop();
    assert!(r.prepare(&bad, (640, 440), 1.).is_err());
    bad = c.clone();
    bad.title = "X".repeat(160);
    assert!(r.prepare(&bad, (640, 440), 1.).is_err());
    assert_eq!(r.gpu_memory_usage(), usage);
    r.set_memory_budget(usage.total_bytes());
    bad = c.clone();
    bad.style.outline = true;
    assert!(matches!(
        r.prepare(&bad, (640, 440), 1.),
        Err(CategoricalError::Budget { .. })
    ));
    assert_eq!(usage, r.gpu_memory_usage());
    assert!(Arc::ptr_eq(&f, &r.prepare(&c, (640, 440), 1.).unwrap()));
}
#[test]
fn export_formats_png_and_dpr_are_real_resolution() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = chart();
    c.background = Color::new(0.4, 0.6, 0.8, 0.5);
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        let mut r = CategoricalRenderer::new(gpu(), format).unwrap();
        let first = r.prepare(&c, (640, 440), 1.).unwrap();
        let second = r.prepare(&c, (640, 440), 2.).unwrap();
        assert!(!first.shares_annotations_with(&second));
        let a = first.bar_rect(&CategoryBarTarget::new("a", "one")).unwrap();
        let b = second
            .bar_rect(&CategoryBarTarget::new("a", "one"))
            .unwrap();
        for i in 0..4 {
            assert!((a[i] * 2. - b[i]).abs() < 0.01);
        }
        let image = r.export_rgba(&c, (640, 440), 2.).unwrap();
        assert_eq!((image.width, image.height), (1280, 880));
        for (v, expected) in rgba(&image, 0, 0).into_iter().zip([102u8, 153, 204, 128]) {
            assert!(v.abs_diff(expected) <= 2, "bad alpha: {format:?}");
        }
        let mut reader =
            png::Decoder::new(std::io::Cursor::new(renderer::encode_png(&image).unwrap()))
                .read_info()
                .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        assert_eq!((info.width, info.height), (1280, 880));
        assert_eq!(&bytes[..info.buffer_size()], &image.rgba);
        drop(first);
        drop(second);
        drain(&r);
    }
}
#[test]
fn empty_data_has_axes_but_no_phantom_bar_or_pick() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    for s in &mut c.series {
        s.values = vec![None, Some(0.)];
    }
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let image = r.export_rgba(&c, (640, 440), 1.).unwrap();
    assert!(f.hit_test([320., 220.]).is_none());
    assert!(f.bar_rect(&CategoryBarTarget::new("b", "one")).is_none());
    assert_eq!(rgba(&image, 320, 220), [255; 4]);
}

#[test]
fn fractional_stack_joins_do_not_blend_background_twice() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.categories.truncate(1);
    c.series[0].values = vec![Some(1.)];
    c.series[1].values = vec![Some(2.)];
    c.series[0].color = Color::from_rgb8(255, 0, 0);
    c.series[1].color = Color::from_rgb8(0, 0, 255);
    c.mode = CategoryBarMode::Stacked;
    for direction in [
        CategoryBarDirection::Horizontal,
        CategoryBarDirection::Vertical,
    ] {
        c.direction = direction;
        for size in [(641, 431), (643, 433), (646, 437)] {
            for scale in [1.0, 1.5] {
                let f = r.prepare(&c, size, scale).unwrap();
                let rect = f.bar_rect(&CategoryBarTarget::new("a", "one")).unwrap();
                let image = r.export_rgba(&c, size, scale).unwrap();
                let center = center(rect);
                let boundary = if direction == CategoryBarDirection::Horizontal {
                    rect[2]
                } else {
                    rect[1]
                };
                for offset in -2..=2 {
                    let t = (boundary.floor() as i32 + offset) as u32;
                    let (x, y) = if direction == CategoryBarDirection::Horizontal {
                        (t, center[1] as u32)
                    } else {
                        (center[0] as u32, t)
                    };
                    assert_eq!(
                        rgba(&image, x, y)[1],
                        0,
                        "white seam at {direction:?} {size:?} {scale}: {:?}",
                        rgba(&image, x, y)
                    );
                }
                drop(f);
                drain(&r);
            }
        }
    }
}

#[test]
fn each_material_parameter_changes_pixels_but_reuses_annotations() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let cases: Vec<(
        &str,
        CategoryBarMaterial,
        Box<dyn Fn(&mut CategoryBarStyle)>,
    )> = vec![
        (
            "matte strength",
            CategoryBarMaterial::Matte,
            Box::new(|s| s.texture_strength = 0.0),
        ),
        (
            "matte frequency",
            CategoryBarMaterial::Matte,
            Box::new(|s| s.texture_scale = 3.0),
        ),
        (
            "satin strength",
            CategoryBarMaterial::SatinMetal,
            Box::new(|s| s.texture_strength = 0.0),
        ),
        (
            "satin frequency",
            CategoryBarMaterial::SatinMetal,
            Box::new(|s| s.texture_scale = 3.0),
        ),
        (
            "satin gloss",
            CategoryBarMaterial::SatinMetal,
            Box::new(|s| s.gloss = 0.0),
        ),
        (
            "enamel gloss",
            CategoryBarMaterial::Enamel,
            Box::new(|s| s.gloss = 0.0),
        ),
        (
            "paper strength",
            CategoryBarMaterial::Paper,
            Box::new(|s| s.texture_strength = 0.0),
        ),
        (
            "paper frequency",
            CategoryBarMaterial::Paper,
            Box::new(|s| s.texture_scale = 3.0),
        ),
        (
            "outline width",
            CategoryBarMaterial::Flat,
            Box::new(|s| s.outline_width = 5.0),
        ),
        (
            "outline color",
            CategoryBarMaterial::Flat,
            Box::new(|s| s.outline_color = Color::from_rgb8(30, 220, 50)),
        ),
        (
            "outline alpha",
            CategoryBarMaterial::Flat,
            Box::new(|s| s.outline_color.a = 0.1),
        ),
        (
            "emphasis brightness",
            CategoryBarMaterial::Flat,
            Box::new(|s| s.emphasis_brightness = 0.4),
        ),
    ];
    for (name, material, edit) in cases {
        let mut c = chart();
        c.style.material = material;
        c.style.texture_strength = 1.;
        c.style.gloss = 0.8;
        c.style.outline = true;
        c.selected = Some(CategoryBarTarget::new("a", "one"));
        let before_frame = r.prepare(&c, (640, 440), 1.).unwrap();
        let before = r.export_rgba(&c, (640, 440), 1.).unwrap();
        drain(&r);
        let usage = r.gpu_memory_usage();
        let saved = c.clone();
        edit(&mut c.style);
        let after_frame = r.prepare(&c, (640, 440), 1.).unwrap();
        assert!(!Arc::ptr_eq(&before_frame, &after_frame), "{name}");
        assert!(
            before_frame.shares_annotations_with(&after_frame),
            "{name} rerasterized labels"
        );
        assert_eq!(
            usage.creations_of(renderer::GpuResourceKind::PanelTexture),
            r.gpu_memory_usage()
                .creations_of(renderer::GpuResourceKind::PanelTexture)
        );
        let after = r.export_rgba(&c, (640, 440), 1.).unwrap();
        assert_ne!(before.rgba, after.rgba, "{name} has no visual effect");
        drain(&r);
        let restored = r.export_rgba(&saved, (640, 440), 1.).unwrap();
        assert_eq!(before.rgba, restored.rgba, "{name} did not restore");
        drop(before_frame);
        drop(after_frame);
        drain(&r);
    }
    // Parameters irrelevant to Flat must not accidentally introduce shading.
    let mut c = chart();
    let before = r.export_rgba(&c, (640, 440), 1.).unwrap();
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    c.style.texture_strength = 1.;
    c.style.texture_scale = 8.;
    c.style.gloss = 1.;
    let g = r.prepare(&c, (640, 440), 1.).unwrap();
    assert!(f.shares_annotations_with(&g));
    assert_eq!(before.rgba, r.export_rgba(&c, (640, 440), 1.).unwrap().rgba);
}

fn bar_patch(image: &renderer::RasterImage, rect: [f32; 4]) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in rect[1].ceil() as u32 + 2..rect[3].floor() as u32 - 2 {
        for x in rect[0].ceil() as u32 + 2..rect[2].floor() as u32 - 2 {
            pixels.extend(rgba(image, x, y));
        }
    }
    pixels
}
#[test]
fn three_level_style_and_label_inheritance_restore_the_parent_pixels() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    let target = CategoryBarTarget::new("a", "one");
    c.series[0].style = Some(CategoryBarStyle {
        material: CategoryBarMaterial::SatinMetal,
        ..Default::default()
    });
    let mut edit = CategoryBarOverride::new(target.clone());
    edit.style = Some(CategoryBarStyle {
        material: CategoryBarMaterial::Paper,
        texture_strength: 1.,
        ..Default::default()
    });
    c.overrides.push(edit);
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let a = f.bar_rect(&target).unwrap();
    let b = f.bar_rect(&CategoryBarTarget::new("b", "one")).unwrap();
    let other = f.bar_rect(&CategoryBarTarget::new("a", "two")).unwrap();
    let before = r.export_rgba(&c, (640, 440), 1.).unwrap();
    drain(&r);
    c.style.material = CategoryBarMaterial::Enamel;
    let inherited = r.export_rgba(&c, (640, 440), 1.).unwrap();
    assert_eq!(bar_patch(&before, a), bar_patch(&inherited, a));
    assert_eq!(bar_patch(&before, b), bar_patch(&inherited, b));
    assert_ne!(bar_patch(&before, other), bar_patch(&inherited, other));
    drain(&r);
    // Clearing only the bar style restores the whole series style, not the chart.
    c.overrides[0].style = None;
    let cleared = r.export_rgba(&c, (640, 440), 1.).unwrap();
    let mut expected = c.clone();
    expected.overrides[0].style = expected.series[0].style.clone();
    assert_ne!(bar_patch(&inherited, a), bar_patch(&cleared, a));
    assert_eq!(
        cleared.rgba,
        r.export_rgba(&expected, (640, 440), 1.).unwrap().rgba
    );
    drain(&r);
    c.series[0].style = None;
    let parent = r.export_rgba(&c, (640, 440), 1.).unwrap();
    expected = c.clone();
    expected.series[0].style = Some(expected.style.clone());
    assert_eq!(
        parent.rgba,
        r.export_rgba(&expected, (640, 440), 1.).unwrap().rgba
    );
    drain(&r);
    // Per-bar color and label options clear independently and None means inherit.
    c.labels = CategoryBarLabels::Inside;
    c.label_format = CategoryBarLabelFormat::ValuePercent;
    let inherited = r.export_rgba(&c, (640, 440), 1.).unwrap();
    c.overrides[0].labels = Some(CategoryBarLabels::None);
    c.overrides[0].label_format = Some(CategoryBarLabelFormat::Percent);
    c.overrides[0].color = Some(Color::from_rgb8(20, 210, 40));
    let changed = r.export_rgba(&c, (640, 440), 1.).unwrap();
    assert_ne!(inherited.rgba, changed.rgba);
    c.overrides[0].labels = None;
    c.overrides[0].label_format = None;
    c.overrides[0].color = None;
    assert_eq!(
        inherited.rgba,
        r.export_rgba(&c, (640, 440), 1.).unwrap().rgba
    );
}
#[test]
fn decimals_font_family_and_font_registration_refresh_labels() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.labels = CategoryBarLabels::Auto;
    c.title = "Font and decimals".into();
    c.series[0].values[0] = Some(10.125);
    c.label_decimals = 1;
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let before = r.export_rgba(&c, (640, 440), 1.).unwrap();
    c.label_decimals = 3;
    let g = r.prepare(&c, (640, 440), 1.).unwrap();
    assert!(!f.shares_annotations_with(&g));
    assert_ne!(before.rgba, r.export_rgba(&c, (640, 440), 1.).unwrap().rgba);
    drain(&r);
    let before = r.export_rgba(&c, (640, 440), 1.).unwrap();
    renderer::text_render::register_font_bytes(
        include_bytes!("../fonts/ComicNeue-Regular.ttf").to_vec(),
    )
    .unwrap();
    let generation = r.prepare(&c, (640, 440), 1.).unwrap();
    assert!(
        !g.shares_annotations_with(&generation),
        "font registration kept the old atlas"
    );
    c.font_family = "Comic Neue".into();
    let family = r.prepare(&c, (640, 440), 1.).unwrap();
    assert!(!generation.shares_annotations_with(&family));
    assert_ne!(before.rgba, r.export_rgba(&c, (640, 440), 1.).unwrap().rgba);
    let usage = r.gpu_memory_usage();
    assert!(Arc::ptr_eq(
        &family,
        &r.prepare(&c, (640, 440), 1.).unwrap()
    ));
    assert_eq!(usage, r.gpu_memory_usage());
}

fn maximum_chart(categories: usize, series: usize) -> CategoricalChart {
    CategoricalChart {
        categories: (0..categories)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect(),
        series: (0..series)
            .map(|i| {
                BarSeries::new(
                    format!("s{i}"),
                    format!("S{i}"),
                    vec![Some(10.); categories],
                    if i % 2 == 0 {
                        Color::from_rgb8(220, 30, 50)
                    } else {
                        Color::from_rgb8(30, 80, 220)
                    },
                )
            })
            .collect(),
        direction: CategoryBarDirection::Horizontal,
        labels: CategoryBarLabels::None,
        legend: false,
        grid: false,
        bar_gap: 0.,
        style: CategoryBarStyle {
            corner_radius: 0.,
            ..Default::default()
        },
        ..Default::default()
    }
}
#[test]
fn maximum_512_bars_render_pick_and_reject_over_limit_transactionally() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    for (n, m) in [(64, 8), (32, 16)] {
        let c = maximum_chart(n, m);
        let size = (640, 1800);
        let frame = r.prepare(&c, size, 1.).unwrap();
        let image = r.export_rgba(&c, size, 1.).unwrap();
        for cat in &c.categories {
            for (i, series) in c.series.iter().enumerate() {
                let t = CategoryBarTarget::new(&cat.id, &series.id);
                let rect = frame.bar_rect(&t).expect("truncated bar table");
                let p = center(rect);
                assert_eq!(frame.hit_test(p), Some(t));
                let color = rgba(&image, p[0] as u32, p[1] as u32);
                let channel = if i % 2 == 0 { 0 } else { 2 };
                assert!(
                    color[channel] > color[1] + 80,
                    "bar pixel missing: {color:?}"
                );
            }
        }
        drain(&r);
        let usage = r.gpu_memory_usage();
        let mut bad = c.clone();
        bad.categories.push(Category::new("excess", "Excess"));
        for series in &mut bad.series {
            series.values.push(Some(10.));
        }
        assert!(r.prepare(&bad, size, 1.).is_err());
        assert_eq!(usage, r.gpu_memory_usage());
        assert!(Arc::ptr_eq(&frame, &r.prepare(&c, size, 1.).unwrap()));
        drop(frame);
        r.clear_cache();
        drain(&r);
        assert_eq!(r.gpu_memory_usage().total_bytes(), 0);
    }
}

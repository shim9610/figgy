#![cfg(not(target_arch = "wasm32"))]
use renderer::{Category, Color, RendererDevice, boxplot::*};
use std::sync::{Arc, OnceLock};
// A test may register fonts or poll the shared GPU and execute another test's
// completion callbacks. Serialize these shared resources, not production work.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn gpu() -> RendererDevice {
    static GPU: OnceLock<RendererDevice> = OnceLock::new();
    GPU.get_or_init(|| {
        let instance = renderer::data_render::create_instance();
        let a = renderer::data_render::request_adapter(&instance).expect("GPU required");
        let (d, q) = renderer::data_render::request_device(&a).unwrap();
        RendererDevice::new(Arc::new(d), Arc::new(q))
    })
    .clone()
}
fn renderer() -> BoxPlotRenderer {
    BoxPlotRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap()
}
fn chart() -> BoxPlotChart {
    let mut v = BoxSummary::new(10., 30., 40., 60., 80.);
    v.outliers = vec![95.];
    v.median_ci = Some([35., 45.]);
    v.mean = Some(50.);
    v.sample_count = Some(100);
    let mut c = BoxPlotChart {
        categories: vec![Category::new("a", "A"), Category::new("b", "B")],
        series: vec![BoxPlotSeries::new(
            "s",
            "S",
            vec![Some(v.clone()), Some(v)],
            Color::from_rgb8(30, 110, 210),
        )],
        value_range: Some([0., 100.]),
        grid: false,
        legend: false,
        ..Default::default()
    };
    c.style.material = BoxPlotMaterial::Flat;
    c
}
fn drain(r: &BoxPlotRenderer) {
    r.end_frame();
    gpu()
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
}
fn pixel(img: &renderer::RasterImage, p: [f32; 2]) -> [u8; 4] {
    let i = ((p[1].floor() as u32 * img.width + p[0].floor() as u32) * 4) as usize;
    img.rgba[i..i + 4].try_into().unwrap()
}
fn center(r: [f32; 4]) -> [f32; 2] {
    [(r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5]
}
fn part(t: &BoxPlotTarget, p: BoxPlotPart) -> BoxPlotPick {
    BoxPlotPick {
        target: t.clone(),
        part: p,
    }
}
fn value_pixel(f: &BoxPlotFrame, horizontal: bool, v: f32) -> f32 {
    let p = f.plot_rect();
    if horizontal {
        p[0] + v / 100. * (p[2] - p[0])
    } else {
        p[3] - v / 100. * (p[3] - p[1])
    }
}
#[test]
fn quartiles_median_whiskers_outliers_match_numeric_axis_and_picking_at_dpr() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    let t = BoxPlotTarget::new("a", "s");
    for horizontal in [false, true] {
        c.direction = if horizontal {
            BoxPlotDirection::Horizontal
        } else {
            BoxPlotDirection::Vertical
        };
        for scale in [1., 1.5, 2.] {
            let f = r.prepare(&c, (640, 440), scale).unwrap();
            let image = r.export_rgba(&c, (640, 440), scale).unwrap();
            let bounds = f.box_rect(&t).unwrap();
            let (a, b) = if horizontal {
                (bounds[0], bounds[2])
            } else {
                (bounds[3], bounds[1])
            };
            assert!((a - value_pixel(&f, horizontal, 30.)).abs() < 0.01);
            assert!((b - value_pixel(&f, horizontal, 60.)).abs() < 0.01);
            for (p, value) in [
                (BoxPlotPart::Median, 40.),
                (BoxPlotPart::CapLow, 10.),
                (BoxPlotPart::CapHigh, 80.),
                (BoxPlotPart::Outlier(0), 95.),
            ] {
                let pick = part(&t, p.clone());
                let q = center(f.part_rect(&pick).unwrap());
                assert!(
                    (q[usize::from(!horizontal)] - value_pixel(&f, horizontal, value)).abs() < 0.01
                );
                let q = if matches!(p, BoxPlotPart::Outlier(_)) {
                    [q[0] + 2.8 * scale, q[1]]
                } else {
                    q
                };
                assert_eq!(f.hit_test(q.map(|v| v / scale)), Some(pick));
                assert_ne!(pixel(&image, q), [255; 4]);
            }
            let mut p = center(bounds);
            p[usize::from(!horizontal)] = value_pixel(&f, horizontal, 53.);
            assert_eq!(pixel(&image, p), [30, 110, 210, 255]);
            assert_eq!(
                f.hit_test(p.map(|v| v / scale)),
                Some(part(&t, BoxPlotPart::Box))
            );
            assert!(
                image
                    .rgba
                    .chunks_exact(4)
                    .any(|p| p[0] > 30 && p[0] < 255 && p[2] > p[0]),
                "subpixel edges need partial coverage"
            );
            drop(f);
            drain(&r);
        }
    }
}
#[test]
fn notch_silhouette_and_extended_confidence_interval_have_matching_hits() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.style.notched = true;
    let t = BoxPlotTarget::new("a", "s");
    for horizontal in [false, true] {
        c.direction = if horizontal {
            BoxPlotDirection::Horizontal
        } else {
            BoxPlotDirection::Vertical
        };
        let f = r.prepare(&c, (640, 440), 1.).unwrap();
        let img = r.export_rgba(&c, (640, 440), 1.).unwrap();
        let b = f.box_rect(&t).unwrap();
        let p = if horizontal {
            [value_pixel(&f, true, 40.), b[1] + 2.]
        } else {
            [b[0] + 2., value_pixel(&f, false, 40.)]
        };
        assert!(f.hit_test(p).is_none(), "notch shoulder hole must not pick");
        assert_eq!(pixel(&img, p), [255; 4]);
        let mut mid = center(b);
        mid[usize::from(!horizontal)] = value_pixel(&f, horizontal, 40.);
        assert_eq!(f.hit_test(mid), Some(part(&t, BoxPlotPart::Median)));
    }
    c.direction = BoxPlotDirection::Vertical;
    c.series[0].values[0].as_mut().unwrap().median_ci = Some([20., 75.]);
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let b = f.box_rect(&t).unwrap();
    assert!((b[1] - value_pixel(&f, false, 75.)).abs() < 0.01);
    assert!((b[3] - value_pixel(&f, false, 20.)).abs() < 0.01);
    let img = r.export_rgba(&c, (640, 440), 1.).unwrap();
    assert!(img.rgba.chunks_exact(4).any(|p| p[2] > 150 && p[0] < 80));
}
#[test]
fn outline_off_keeps_median_whiskers_and_mean_and_clips_all_parts() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.style.outline = false;
    c.style.show_mean = true;
    let t = BoxPlotTarget::new("a", "s");
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let img = r.export_rgba(&c, (640, 440), 1.).unwrap();
    for p in [
        BoxPlotPart::Median,
        BoxPlotPart::WhiskerLow,
        BoxPlotPart::WhiskerHigh,
        BoxPlotPart::Mean,
    ] {
        let pick = part(&t, p);
        let pos = center(f.part_rect(&pick).unwrap());
        assert_eq!(f.hit_test(pos), Some(pick));
        assert_ne!(pixel(&img, pos), [255; 4]);
    }
    c.value_range = Some([35., 55.]);
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let img = r.export_rgba(&c, (640, 440), 1.).unwrap();
    let clip = f.plot_rect();
    assert!(f.hit_test([clip[2], (clip[1] + clip[3]) * 0.5]).is_none());
    assert!(f.hit_test([(clip[0] + clip[2]) * 0.5, clip[3]]).is_none());
    let p = [center(f.box_rect(&t).unwrap())[0], clip[1] - 8.];
    assert!(f.hit_test(p).is_none());
    assert_eq!(pixel(&img, p), [255; 4]);
    assert!(f.part_rect(&part(&t, BoxPlotPart::Outlier(0))).is_none());
}
#[test]
fn missing_and_degenerate_boxes_keep_real_medians_and_stable_selection() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    let t = BoxPlotTarget::new("a", "s");
    c.series[0].values = vec![Some(BoxSummary::new(42., 42., 42., 42., 42.)), None];
    c.selected = Some(t.clone());
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    assert!(f.box_rect(&t).is_none());
    let p = center(f.part_rect(&part(&t, BoxPlotPart::Median)).unwrap());
    assert_eq!(f.hit_test(p), Some(part(&t, BoxPlotPart::Median)));
    assert!(f.box_rect(&BoxPlotTarget::new("b", "s")).is_none());
    c.reorder_categories(&["b", "a"]).unwrap();
    let g = r.prepare(&c, (640, 440), 1.).unwrap();
    let q = center(g.part_rect(&part(&t, BoxPlotPart::Median)).unwrap());
    assert_ne!(p, q);
    assert_eq!(g.hit_test(q), Some(part(&t, BoxPlotPart::Median)));
}
#[test]
fn every_style_switch_changes_pixels_and_reuses_only_valid_annotations() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    let t = BoxPlotTarget::new("a", "s");
    c.style.material = BoxPlotMaterial::SatinMetal;
    let first = r.prepare(&c, (640, 440), 1.).unwrap();
    let before = r.export_rgba(&c, (640, 440), 1.).unwrap();
    let edits: Vec<(&str, fn(&mut BoxPlotChart))> = vec![
        ("material", |c| c.style.material = BoxPlotMaterial::Matte),
        ("gloss", |c| c.style.gloss = 0.95),
        ("strength", |c| c.style.texture_strength = 1.),
        ("frequency", |c| c.style.texture_scale = 5.),
        ("corners", |c| c.style.corner_radius = 3.),
        ("outline", |c| c.style.outline = false),
        ("outline width", |c| c.style.outline_width = 4.),
        ("outline color", |c| {
            c.style.outline_color = Color::new(1., 0., 0., 1.)
        }),
        ("median width", |c| c.style.median_width = 5.),
        ("median color", |c| {
            c.style.median_color = Color::new(1., 0., 0., 1.)
        }),
        ("whisker width", |c| c.style.whisker_width = 4.),
        ("whisker color", |c| {
            c.style.whisker_color = Color::new(1., 0., 0., 1.)
        }),
        ("caps", |c| c.style.caps = false),
        ("cap ratio", |c| c.style.cap_ratio = 0.9),
        ("notch", |c| c.style.notched = true),
        ("mean", |c| c.style.show_mean = true),
        ("outliers", |c| c.style.show_outliers = false),
        ("outlier size", |c| c.style.outlier_size = 12.),
        ("outlier shape", |c| {
            c.style.outlier_shape = BoxOutlierShape::Square
        }),
        ("outlier color", |c| {
            c.style.outlier_color = Color::new(1., 0., 0., 1.)
        }),
        ("outlier fill", |c| c.style.outlier_filled = true),
        ("selected", |c| {
            c.selected = Some(BoxPlotTarget::new("a", "s"))
        }),
        ("hovered", |c| {
            c.hovered = Some(BoxPlotTarget::new("a", "s"))
        }),
    ];
    for (name, edit) in edits {
        let mut next = c.clone();
        edit(&mut next);
        let f = r.prepare(&next, (640, 440), 1.).unwrap();
        assert!(first.shares_annotations_with(&f), "{name}");
        let img = r.export_rgba(&next, (640, 440), 1.).unwrap();
        assert_ne!(before.rgba, img.rgba, "{name} must change actual pixels");
        let mut fresh = renderer();
        let expected = fresh.export_rgba(&next, (640, 440), 1.).unwrap();
        assert_eq!(img.rgba, expected.rgba, "first update {name}");
        assert_eq!(
            f.box_rect(&t),
            first.box_rect(&t),
            "style must not move quartiles"
        );
        drop(f);
        drain(&r);
    }
    r.prepare(&c, (640, 440), 1.).unwrap();
    let usage = r.gpu_memory_usage();
    let frame = r.prepare(&c, (640, 440), 1.).unwrap();
    assert!(Arc::ptr_eq(&frame, &r.prepare(&c, (640, 440), 1.).unwrap()));
    assert_eq!(r.gpu_memory_usage(), usage);
}
#[test]
fn inherited_styles_and_individual_color_reset_without_touching_other_boxes() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    let t = BoxPlotTarget::new("a", "s");
    let mut edit = BoxPlotOverride::new(t.clone());
    edit.color = Some(Color::from_rgb8(20, 190, 70));
    edit.style = Some(BoxPlotStyle {
        material: BoxPlotMaterial::SatinMetal,
        ..Default::default()
    });
    c.overrides.push(edit);
    let first = r.export_rgba(&c, (640, 440), 1.).unwrap();
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let mut p = center(f.box_rect(&t).unwrap());
    p[1] = value_pixel(&f, false, 53.);
    let other = BoxPlotTarget::new("b", "s");
    let mut q = center(f.box_rect(&other).unwrap());
    q[1] = p[1];
    assert_ne!(pixel(&first, p), pixel(&first, q));
    c.style.material = BoxPlotMaterial::Matte;
    let second = r.export_rgba(&c, (640, 440), 1.).unwrap();
    assert_eq!(pixel(&first, p), pixel(&second, p));
    assert_ne!(pixel(&first, q), pixel(&second, q));
    c.overrides[0].style = None;
    c.overrides[0].color = None;
    let reset = r.export_rgba(&c, (640, 440), 1.).unwrap();
    c.overrides.clear();
    assert_eq!(reset.rgba, r.export_rgba(&c, (640, 440), 1.).unwrap().rgba);
}
#[test]
fn labels_grid_ranges_fonts_and_statistics_refresh_first_frame() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let c = chart();
    let first = r.prepare(&c, (800, 560), 1.).unwrap();
    let edits: Vec<fn(&mut BoxPlotChart)> = vec![
        |c| c.title = "Updated".into(),
        |c| c.grid = true,
        |c| c.legend = true,
        |c| c.direction = BoxPlotDirection::Horizontal,
        |c| c.value_range = Some([-20., 120.]),
        |c| c.font_size = 18.,
        |c| c.series[0].values[0].as_mut().unwrap().q1 = 20.,
        |c| {
            c.labels = BoxPlotLabels::MedianAndCount;
            c.value_range = Some([0., 140.]);
        },
        |c| c.group_width = 0.8,
        |c| c.categories[0].label = "Renamed".into(),
    ];
    let before = r.export_rgba(&c, (800, 560), 1.).unwrap();
    for edit in edits {
        let mut next = c.clone();
        edit(&mut next);
        let f = r.prepare(&next, (800, 560), 1.).unwrap();
        assert!(!first.shares_annotations_with(&f));
        let image = r.export_rgba(&next, (800, 560), 1.).unwrap();
        assert_ne!(image.rgba, before.rgba);
        let mut fresh = renderer();
        assert_eq!(
            image.rgba,
            fresh.export_rgba(&next, (800, 560), 1.).unwrap().rgba
        );
        drop(f);
        drain(&r);
    }
}
#[test]
fn invalid_and_budget_failures_are_atomic_and_external_frames_keep_resources_alive() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let c = chart();
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let before = r.gpu_memory_usage();
    let mut bad = c.clone();
    bad.series[0].values[0].as_mut().unwrap().median = 1000.;
    assert!(r.prepare(&bad, (640, 440), 1.).is_err());
    assert_eq!(r.gpu_memory_usage(), before);
    assert!(Arc::ptr_eq(&f, &r.prepare(&c, (640, 440), 1.).unwrap()));
    r.set_memory_budget(1);
    let mut changed = c.clone();
    changed.style.gloss = 1.;
    assert!(r.prepare(&changed, (640, 440), 1.).is_err());
    assert_eq!(r.gpu_memory_usage(), before);
    r.clear_cache();
    drain(&r);
    assert!(r.gpu_memory_usage().total_bytes() > 0);
    drop(f);
    drain(&r);
    assert_eq!(r.gpu_memory_usage().total_bytes(), 0);
}
#[test]
fn all_target_formats_png_decode_and_transparent_fill_work() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = chart();
    c.background = Color::new(0., 0., 0., 0.);
    c.style.outline = false;
    c.series[0].color.a = 0.5;
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        let mut r = BoxPlotRenderer::new(gpu(), format).unwrap();
        let f = r.prepare(&c, (640, 440), 2.).unwrap();
        let img = r.export_rgba(&c, (640, 440), 2.).unwrap();
        let mut p = center(f.box_rect(&BoxPlotTarget::new("a", "s")).unwrap());
        p[1] = value_pixel(&f, false, 53.);
        let got = pixel(&img, p);
        assert!(got[3].abs_diff(128) <= 1, "{format:?}: {got:?}");
        for (a, b) in got[..3].iter().zip([30u8, 110, 210]) {
            assert!(a.abs_diff(b) <= 3, "{format:?}: {got:?}");
        }
        let mut decoder =
            png::Decoder::new(std::io::Cursor::new(renderer::encode_png(&img).unwrap()))
                .read_info()
                .unwrap();
        let mut data = vec![0; decoder.output_buffer_size().unwrap()];
        let info = decoder.next_frame(&mut data).unwrap();
        assert_eq!((info.width, info.height), (1280, 880));
        assert_eq!(&data[..info.buffer_size()], &img.rgba);
        drop(f);
        drain(&r);
    }
}
#[test]
fn maximum_boxes_and_outliers_render_with_bounded_resources() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.direction = BoxPlotDirection::Horizontal;
    c.font_size = 8.;
    c.group_width = 0.9;
    c.box_gap = 0.;
    c.style.outline = false;
    c.style.median_width = 0.;
    c.categories = (0..64)
        .map(|i| Category::new(i.to_string(), i.to_string()))
        .collect();
    c.series = (0..8)
        .map(|i| {
            BoxPlotSeries::new(
                i.to_string(),
                i.to_string(),
                vec![Some(BoxSummary::new(10., 30., 40., 60., 80.)); 64],
                Color::from_rgb8(30, 110, 210),
            )
        })
        .collect();
    let f = r.prepare(&c, (640, 1800), 1.).unwrap();
    let img = r.export_rgba(&c, (640, 1800), 1.).unwrap();
    for i in 0..64 {
        for j in 0..8 {
            let t = BoxPlotTarget::new(i.to_string(), j.to_string());
            let p = center(f.box_rect(&t).unwrap());
            assert_eq!(f.hit_test(p), Some(part(&t, BoxPlotPart::Box)));
            assert_eq!(pixel(&img, p), [30, 110, 210, 255]);
        }
    }
    c.categories.push(Category::new("overflow", "Overflow"));
    for s in &mut c.series {
        s.values.push(None);
    }
    assert!(r.prepare(&c, (640, 1800), 1.).is_err());
    drop(f);
    drain(&r);
    let mut c = chart();
    c.direction = BoxPlotDirection::Horizontal;
    c.font_size = 8.;
    c.categories = (0..32)
        .map(|i| Category::new(i.to_string(), i.to_string()))
        .collect();
    let mut v = BoxSummary::new(10., 30., 40., 60., 80.);
    v.outliers = (0..128).map(|i| 81. + i as f64 * 0.1).collect();
    c.series[0].values = vec![Some(v); 32];
    let img = r.export_rgba(&c, (640, 1200), 1.).unwrap();
    assert!(img.rgba.iter().any(|v| *v != 255));
    r.clear_cache();
    drain(&r);
    assert_eq!(r.gpu_memory_usage().total_bytes(), 0);
}

#[test]
fn active_notch_mean_emphasis_and_series_inheritance_controls_are_live() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.style.notched = true;
    c.style.show_mean = true;
    let t = BoxPlotTarget::new("a", "s");
    c.selected = Some(t.clone());
    let original = r.prepare(&c, (640, 440), 1.).unwrap();
    let before = r.export_rgba(&c, (640, 440), 1.).unwrap();
    for edit in [
        |s: &mut BoxPlotStyle| s.notch_depth = 0.4,
        |s: &mut BoxPlotStyle| s.mean_size = 16.,
        |s: &mut BoxPlotStyle| s.mean_color = Color::new(0., 0.8, 0., 1.),
        |s: &mut BoxPlotStyle| s.emphasis_brightness = 0.3,
        |s: &mut BoxPlotStyle| s.outline_color.a = 0.2,
    ] {
        let mut next = c.clone();
        edit(&mut next.style);
        let f = r.prepare(&next, (640, 440), 1.).unwrap();
        assert!(original.shares_annotations_with(&f));
        assert_ne!(
            before.rgba,
            r.export_rgba(&next, (640, 440), 1.).unwrap().rgba
        );
        drop(f);
        drain(&r);
    }
    c.series[0].style = Some(BoxPlotStyle {
        material: BoxPlotMaterial::SatinMetal,
        ..c.style.clone()
    });
    let mut o = BoxPlotOverride::new(t.clone());
    o.style = Some(BoxPlotStyle {
        material: BoxPlotMaterial::Matte,
        ..c.style.clone()
    });
    c.overrides.push(o);
    assert_eq!(
        c.resolved_style(&t).unwrap().material,
        BoxPlotMaterial::Matte
    );
    c.overrides[0].style = None;
    assert_eq!(
        c.resolved_style(&t).unwrap().material,
        BoxPlotMaterial::SatinMetal
    );
    let inherited = r.export_rgba(&c, (640, 440), 1.).unwrap();
    c.overrides.clear();
    assert_eq!(
        inherited.rgba,
        r.export_rgba(&c, (640, 440), 1.).unwrap().rgba
    );
    c.series[0].style = None;
    assert_eq!(
        c.resolved_style(&t).unwrap().material,
        BoxPlotMaterial::Flat
    );
}
#[test]
fn label_decimals_suffix_font_registration_and_fixed_negative_ranges_are_not_stale() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.labels = BoxPlotLabels::Median;
    c.value_range = Some([0., 150.]);
    let first = r.prepare(&c, (800, 560), 1.).unwrap();
    let before = r.export_rgba(&c, (800, 560), 1.).unwrap();
    for edit in [
        |c: &mut BoxPlotChart| c.label_decimals = 3,
        |c: &mut BoxPlotChart| c.value_suffix = " ms".into(),
        |c: &mut BoxPlotChart| c.text_color = Color::new(0.6, 0., 0., 1.),
        |c: &mut BoxPlotChart| c.value_title = "Response".into(),
        |c: &mut BoxPlotChart| c.category_title = "Group".into(),
        |c: &mut BoxPlotChart| c.labels = BoxPlotLabels::MedianAndCount,
    ] {
        let mut next = c.clone();
        edit(&mut next);
        let f = r.prepare(&next, (800, 560), 1.).unwrap();
        assert!(!first.shares_annotations_with(&f));
        assert_ne!(
            before.rgba,
            r.export_rgba(&next, (800, 560), 1.).unwrap().rgba
        );
        drop(f);
        drain(&r);
    }
    renderer::text_render::register_font_bytes(
        include_bytes!("../fonts/ComicNeue-Regular.ttf").to_vec(),
    )
    .unwrap();
    let new_generation = r.prepare(&c, (800, 560), 1.).unwrap();
    assert!(!first.shares_annotations_with(&new_generation));
    c.font_family = "Comic Neue".into();
    assert_ne!(before.rgba, r.export_rgba(&c, (800, 560), 1.).unwrap().rgba);
    c.labels = BoxPlotLabels::None;
    c.value_range = Some([-100., 0.]);
    c.series[0].values = vec![Some(BoxSummary::new(-90., -70., -50., -30., -10.)), None];
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let img = r.export_rgba(&c, (640, 440), 1.).unwrap();
    let rect = f.box_rect(&BoxPlotTarget::new("a", "s")).unwrap();
    let clip = f.plot_rect();
    assert!((rect[1] - (clip[3] - 0.7 * (clip[3] - clip[1]))).abs() < 0.01);
    assert!(img.rgba.chunks_exact(4).any(|p| p[2] > 150 && p[0] < 50));
    c.value_range = None;
    for s in &mut c.series {
        s.values = vec![None; 2];
    }
    let empty = r.prepare(&c, (640, 440), 1.).unwrap();
    assert!(empty.hit_test([320., 220.]).is_none());
    assert!(r.prepare(&c, (100, 100), 1.).is_err());
    assert!(r.prepare(&c, (640, 440), f32::NAN).is_err());
}

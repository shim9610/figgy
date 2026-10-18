#![cfg(not(target_arch = "wasm32"))]

#[path = "support/gpu.rs"]
mod test_gpu;
use renderer::{Category, Color, RendererDevice, radar::*};
use std::sync::{Arc, OnceLock};
// A test may register fonts or poll the shared GPU and execute another test's
// completion callbacks. Serialize these shared resources, not production work.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn gpu() -> RendererDevice {
    static GPU: OnceLock<RendererDevice> = OnceLock::new();
    GPU.get_or_init(|| {
        let (d, q) = test_gpu::device();
        RendererDevice::new(Arc::new(d), Arc::new(q))
    })
    .clone()
}
fn renderer() -> RadarRenderer {
    RadarRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap()
}
/// Four spokes up, right, down, left on a fixed 0..10 axis.
fn chart() -> RadarChart {
    RadarChart {
        categories: ["a", "b", "c", "d"]
            .into_iter()
            .map(|s| Category::new(s, s.to_uppercase()))
            .collect(),
        series: vec![RadarSeries::new(
            "s",
            "S",
            vec![0., 5., 10., 2.5],
            Color::from_rgb8(30, 110, 210),
        )],
        value_range: Some([0., 10.]),
        ring_spacing: Some(2.5),
        legend: false,
        ..Default::default()
    }
}
fn drain(r: &RadarRenderer) {
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
fn pick(t: RadarTarget, part: RadarPart) -> RadarPick {
    RadarPick { target: t, part }
}
fn near(a: [f32; 2], b: [f32; 2], tolerance: f32) -> bool {
    (a[0] - b[0]).abs() <= tolerance && (a[1] - b[1]).abs() <= tolerance
}
#[test]
fn vertices_follow_the_value_axis_and_every_part_picks_at_dpr() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let c = chart();
    for scale in [1., 2.] {
        let f = r.prepare(&c, (640, 480), scale).unwrap();
        let (center, radius) = f.plot_circle();
        assert!(radius > 100. * scale, "radius {radius} at {scale}x");
        let clip = f.plot_rect();
        assert!(center[0] > clip[0] && center[0] < clip[2]);
        assert!(center[1] > clip[1] && center[1] < clip[3]);
        let vertex = |id: &str| f.vertex_position(&RadarTarget::vertex("s", id)).unwrap();
        // -90 degrees clockwise: a up, b right, c down, d left.
        assert!(near(vertex("a"), center, 0.01));
        assert!(near(
            vertex("b"),
            [center[0] + radius * 0.5, center[1]],
            0.01
        ));
        assert!(near(vertex("c"), [center[0], center[1] + radius], 0.01));
        assert!(near(
            vertex("d"),
            [center[0] - radius * 0.25, center[1]],
            0.01
        ));
        let logical = |p: [f32; 2]| [p[0] / scale, p[1] / scale];
        assert_eq!(
            f.hit_test(logical(vertex("c"))),
            Some(pick(RadarTarget::vertex("s", "c"), RadarPart::Marker))
        );
        assert_eq!(
            f.hit_test(logical(vertex("b"))),
            Some(pick(RadarTarget::vertex("s", "b"), RadarPart::Marker))
        );
        // The b-c edge, well away from both markers.
        let b = vertex("b");
        let cc = vertex("c");
        assert_eq!(
            f.hit_test(logical([(b[0] + cc[0]) * 0.5, (b[1] + cc[1]) * 0.5])),
            Some(pick(RadarTarget::series("s"), RadarPart::Outline))
        );
        // Inside the polygon, away from every edge.
        assert_eq!(
            f.hit_test(logical([
                center[0] + radius * 0.1,
                center[1] + radius * 0.3
            ])),
            Some(pick(RadarTarget::series("s"), RadarPart::Fill))
        );
        // The empty upper half and the outside of the plot area.
        assert_eq!(
            f.hit_test(logical([center[0], center[1] - radius * 0.5])),
            None
        );
        assert_eq!(f.hit_test([-1., -1.]), None);
        assert_eq!(f.hit_test([f32::NAN, 10.]), None);
        let marker = f
            .part_rect(&pick(RadarTarget::vertex("s", "c"), RadarPart::Marker))
            .unwrap();
        assert!(marker[2] - marker[0] >= 7. * scale);
        assert_eq!(f.size(), ((640. * scale) as u32, (480. * scale) as u32));
        let img = r.export_rgba(&c, (640, 480), scale).unwrap();
        assert_eq!((img.width, img.height), f.size());
        let inside = pixel(&img, [center[0] + radius * 0.1, center[1] + radius * 0.3]);
        assert!(inside[2] > inside[0], "fill tint missing: {inside:?}");
        drop(f);
        drain(&r);
    }
}
#[test]
fn picked_vertex_grows_hover_brightens_and_emphasized_labels_follow_the_pick() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    let t = RadarTarget::vertex("s", "c");
    let first = r.prepare(&c, (640, 480), 1.).unwrap();
    let plain = first
        .part_rect(&pick(t.clone(), RadarPart::Marker))
        .unwrap();
    let before = r.export_rgba(&c, (640, 480), 1.).unwrap();
    c.selected = Some(t.clone());
    let f = r.prepare(&c, (640, 480), 1.).unwrap();
    let grown = f.part_rect(&pick(t.clone(), RadarPart::Marker)).unwrap();
    assert!(grown[2] - grown[0] > plain[2] - plain[0]);
    assert!(
        first.shares_annotations_with(&f),
        "plain labels ignore picks"
    );
    let (center, radius) = f.plot_circle();
    let p = [center[0] + radius * 0.1, center[1] + radius * 0.3];
    let dim = pixel(&before, p);
    let after = r.export_rgba(&c, (640, 480), 1.).unwrap();
    let bright = pixel(&after, p);
    assert!(
        bright[0] > dim[0] && bright[1] > dim[1],
        "emphasis must brighten the fill: {dim:?} -> {bright:?}"
    );
    c.selected = None;
    c.hovered = Some(RadarTarget::series("s"));
    let hovered = r.export_rgba(&c, (640, 480), 1.).unwrap();
    assert_eq!(pixel(&hovered, p), bright);
    c.labels = RadarLabels::Emphasized;
    let g = r.prepare(&c, (640, 480), 1.).unwrap();
    assert!(!first.shares_annotations_with(&g));
    let data_labels = |f: &RadarFrame| {
        f.decorations()
            .entries()
            .filter(|(_, e)| matches!(e.id, renderer::DecorationId::DataLabel { .. }))
            .count()
    };
    assert_eq!(data_labels(&g), 4);
    c.hovered = None;
    let h = r.prepare(&c, (640, 480), 1.).unwrap();
    assert_eq!(data_labels(&h), 0);
    c.labels = RadarLabels::Values;
    assert_eq!(data_labels(&r.prepare(&c, (640, 480), 1.).unwrap()), 4);
    assert_eq!(c.value(&t), Some(10.));
    assert_eq!(c.value(&RadarTarget::vertex("s", "zz")), None);
    assert!(c.contains(&RadarTarget::series("s")));
    assert!(!c.contains(&RadarTarget::series("x")));
}
#[test]
fn every_style_switch_changes_pixels_and_reuses_only_valid_rasters() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let c = chart();
    let t = RadarTarget::vertex("s", "c");
    let first = r.prepare(&c, (640, 440), 1.).unwrap();
    let before = r.export_rgba(&c, (640, 440), 1.).unwrap();
    let edits: Vec<(&str, bool, fn(&mut RadarChart))> = vec![
        ("line width", true, |c| c.style.line_width_px = 6.),
        ("fill opacity", true, |c| c.style.fill_opacity = 0.9),
        ("marker square", true, |c| {
            c.style.marker = RadarMarker::Square
        }),
        ("marker none", true, |c| c.style.marker = RadarMarker::None),
        ("marker size", true, |c| c.style.marker_size_px = 14.),
        ("series fill", true, |c| {
            c.series[0].style = Some(RadarSeriesStyle {
                fill: Some(Color::new(1., 0., 0., 0.8)),
                ..Default::default()
            })
        }),
        ("series width", true, |c| {
            c.series[0].style = Some(RadarSeriesStyle {
                line_width_px: Some(8.),
                ..Default::default()
            })
        }),
        ("selected", true, |c| {
            c.selected = Some(RadarTarget::series("s"))
        }),
        ("hovered", true, |c| {
            c.hovered = Some(RadarTarget::series("s"))
        }),
        ("emphasis", true, |c| {
            c.style.emphasis_brightness = 0.3;
            c.selected = Some(RadarTarget::series("s"))
        }),
        // The legend swatch lives on the raster.
        ("series color", false, |c| {
            c.series[0].color = Color::from_rgb8(200, 40, 40)
        }),
        ("ring color", false, |c| {
            c.style.ring_color = Color::from_rgb8(220, 30, 30)
        }),
        ("spoke color", false, |c| {
            c.style.spoke_color = Color::from_rgb8(30, 220, 30)
        }),
        ("grid shape", false, |c| {
            c.style.grid_shape = RadarGridShape::Circle
        }),
        ("grid off", false, |c| c.grid = false),
    ];
    // Raster-preserving edits come first: the cache keeps only the frames
    // still alive, so a rebuilt raster would hide a later reuse.
    for (name, raster_reused, edit) in edits {
        let mut next = c.clone();
        edit(&mut next);
        let f = r.prepare(&next, (640, 440), 1.).unwrap();
        assert_eq!(
            first.shares_annotations_with(&f),
            raster_reused,
            "{name}: raster reuse"
        );
        let img = r.export_rgba(&next, (640, 440), 1.).unwrap();
        assert_ne!(before.rgba, img.rgba, "{name} must change actual pixels");
        let mut fresh = renderer();
        let expected = fresh.export_rgba(&next, (640, 440), 1.).unwrap();
        assert_eq!(img.rgba, expected.rgba, "first update {name}");
        if name != "marker none" {
            assert_eq!(
                f.vertex_position(&t),
                first.vertex_position(&t),
                "{name}: style must not move vertices"
            );
        }
        drop(f);
        drain(&r);
    }
    // Angle and direction move the vertices and re-rasterize the grid.
    for (name, edit) in [
        (
            "start angle",
            (|c| c.style.start_angle_degrees = 0.) as fn(&mut RadarChart),
        ),
        ("direction", |c| c.style.clockwise = false),
    ] {
        let mut next = c.clone();
        edit(&mut next);
        let f = r.prepare(&next, (640, 440), 1.).unwrap();
        assert!(!first.shares_annotations_with(&f), "{name}");
        // Spoke c sits at 180 degrees, where a direction flip lands on
        // itself; spoke b at 90 degrees moves under both edits.
        let b = RadarTarget::vertex("s", "b");
        assert_ne!(f.vertex_position(&b), first.vertex_position(&b), "{name}");
        let img = r.export_rgba(&next, (640, 440), 1.).unwrap();
        assert_ne!(before.rgba, img.rgba, "{name}");
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
fn labels_legend_axis_fonts_and_background_refresh_first_frame() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let c = chart();
    let first = r.prepare(&c, (640, 440), 1.).unwrap();
    let before = r.export_rgba(&c, (640, 440), 1.).unwrap();
    let edits: Vec<(&str, fn(&mut RadarChart))> = vec![
        ("values", |c| c.labels = RadarLabels::Values),
        ("legend", |c| c.legend = true),
        ("title", |c| c.title = "Radar".into()),
        ("range", |c| c.value_range = Some([0., 20.])),
        ("auto range", |c| {
            c.value_range = None;
            c.series[0].values[2] = 11.
        }),
        ("spacing", |c| c.ring_spacing = Some(5.)),
        ("auto spacing", |c| c.ring_spacing = None),
        ("decimals", |c| {
            c.labels = RadarLabels::Values;
            c.label_decimals = 3
        }),
        ("suffix", |c| {
            c.labels = RadarLabels::Values;
            c.value_suffix = " pt".into()
        }),
        ("font size", |c| c.font_size = 22.),
        ("text color", |c| {
            c.text_color = Color::from_rgb8(200, 20, 20)
        }),
        ("background", |c| {
            c.background = Color::from_rgb8(20, 20, 40)
        }),
        ("category label", |c| c.categories[0].label = "Alpha".into()),
        ("series label", |c| {
            c.legend = true;
            c.series[0].label = "Renamed".into()
        }),
        ("missing value", |c| c.series[0].values[1] = f64::NAN),
    ];
    for (name, edit) in edits {
        let mut next = c.clone();
        edit(&mut next);
        let f = r.prepare(&next, (640, 440), 1.).unwrap();
        assert!(
            !first.shares_annotations_with(&f) || name == "missing value",
            "{name}"
        );
        let img = r.export_rgba(&next, (640, 440), 1.).unwrap();
        assert_ne!(before.rgba, img.rgba, "{name} must change actual pixels");
        let mut fresh = renderer();
        assert_eq!(
            img.rgba,
            fresh.export_rgba(&next, (640, 440), 1.).unwrap().rgba,
            "first update {name}"
        );
        if name == "missing value" {
            // The broken spoke has no marker, the fill is gone and the
            // outline stops at the neighbours.
            assert_eq!(f.vertex_position(&RadarTarget::vertex("s", "b")), None);
            let (center, radius) = f.plot_circle();
            assert_eq!(
                f.hit_test([center[0] + radius * 0.1, center[1] + radius * 0.3]),
                None
            );
            assert_eq!(
                f.hit_test(f.vertex_position(&RadarTarget::vertex("s", "c")).unwrap()),
                Some(pick(RadarTarget::vertex("s", "c"), RadarPart::Marker))
            );
        }
        drop(f);
        drain(&r);
    }
}
#[test]
fn automatic_axis_covers_the_data_and_negative_values_sit_at_the_centre() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.value_range = None;
    c.ring_spacing = None;
    c.series[0].values = vec![3., 12., 7., 9.];
    let f = r.prepare(&c, (640, 480), 1.).unwrap();
    let (center, radius) = f.plot_circle();
    let b = f.vertex_position(&RadarTarget::vertex("s", "b")).unwrap();
    // The largest value ends on a ring at or inside the outer ring.
    assert!(b[0] - center[0] <= radius + 0.01 && b[0] - center[0] > radius * 0.6);
    // The ring labels are one draggable group along the first spoke.
    let value_labels = f
        .decorations()
        .entries()
        .filter(|(_, e)| e.id == renderer::DecorationId::AxisLabel(renderer::layout::Side::Left))
        .count();
    assert_eq!(value_labels, 1);
    c.value_range = Some([5., 10.]);
    c.series[0].values = vec![-4., 20., 7.5, 5.];
    let g = r.prepare(&c, (640, 480), 1.).unwrap();
    let (center, radius) = g.plot_circle();
    assert!(near(
        g.vertex_position(&RadarTarget::vertex("s", "a")).unwrap(),
        center,
        0.01
    ));
    assert!(near(
        g.vertex_position(&RadarTarget::vertex("s", "b")).unwrap(),
        [center[0] + radius, center[1]],
        0.01
    ));
    assert!(near(
        g.vertex_position(&RadarTarget::vertex("s", "c")).unwrap(),
        [center[0], center[1] + radius * 0.5],
        0.01
    ));
    assert!(near(
        g.vertex_position(&RadarTarget::vertex("s", "d")).unwrap(),
        center,
        0.01
    ));
}
#[test]
fn invalid_and_budget_failures_are_atomic_and_external_frames_keep_resources_alive() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let c = chart();
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let before = r.gpu_memory_usage();
    let mut bad = c.clone();
    bad.categories.truncate(2);
    assert!(r.prepare(&bad, (640, 440), 1.).is_err());
    let mut bad = c.clone();
    bad.series[0].values.pop();
    assert!(r.prepare(&bad, (640, 440), 1.).is_err());
    let mut bad = c.clone();
    bad.value_range = Some([5., 5.]);
    assert!(r.prepare(&bad, (640, 440), 1.).is_err());
    assert!(r.prepare(&c, (100, 100), 1.).is_err());
    assert_eq!(r.gpu_memory_usage(), before);
    assert!(Arc::ptr_eq(&f, &r.prepare(&c, (640, 440), 1.).unwrap()));
    r.set_memory_budget(1);
    let mut changed = c.clone();
    changed.style.fill_opacity = 0.5;
    assert!(matches!(
        r.prepare(&changed, (640, 440), 1.),
        Err(RadarError::Budget { .. })
    ));
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
    c.grid = false;
    c.style.marker = RadarMarker::None;
    c.series[0].style = Some(RadarSeriesStyle {
        fill: Some(Color::new(30. / 255., 110. / 255., 210. / 255., 0.5)),
        ..Default::default()
    });
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        let mut r = RadarRenderer::new(gpu(), format).unwrap();
        let f = r.prepare(&c, (640, 440), 2.).unwrap();
        let img = r.export_rgba(&c, (640, 440), 2.).unwrap();
        let (center, radius) = f.plot_circle();
        let got = pixel(&img, [center[0] + radius * 0.1, center[1] + radius * 0.3]);
        assert!(got[3].abs_diff(128) <= 1, "{format:?}: {got:?}");
        for (a, b) in got[..3].iter().zip([30u8, 110, 210]) {
            assert!(a.abs_diff(b) <= 3, "{format:?}: {got:?}");
        }
        assert_eq!(pixel(&img, [center[0], center[1] - radius * 0.5]), [0; 4]);
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
fn maximum_series_and_categories_render_with_bounded_resources() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.categories = (0..MAX_RADAR_CATEGORIES)
        .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
        .collect();
    c.series = (0..MAX_RADAR_SERIES)
        .map(|s| {
            RadarSeries::new(
                format!("s{s}"),
                format!("Series {s}"),
                (0..MAX_RADAR_CATEGORIES)
                    .map(|i| ((i * 7 + s * 3) % 10) as f64)
                    .collect(),
                Color::from_rgb8(20 + 14 * s as u8, 80, 220 - 12 * s as u8),
            )
        })
        .collect();
    c.legend = true;
    c.labels = RadarLabels::Values;
    let f = r.prepare(&c, (1000, 800), 1.).unwrap();
    let img = r.export_rgba(&c, (1000, 800), 1.).unwrap();
    assert_eq!((img.width, img.height), (1000, 800));
    assert!(r.gpu_memory_usage().total_bytes() < 64 * 1024 * 1024);
    let last = RadarTarget::vertex("s15", "c63");
    let p = f.vertex_position(&last).unwrap();
    assert_eq!(f.hit_test(p).map(|p| p.part), Some(RadarPart::Marker));
    c.series.push(RadarSeries::new(
        "extra",
        "Extra",
        vec![1.; 64],
        Color::BLACK,
    ));
    assert!(r.prepare(&c, (1000, 800), 1.).is_err());
    c.series.pop();
    c.categories.push(Category::new("c64", "C64"));
    assert!(r.prepare(&c, (1000, 800), 1.).is_err());
    drop(f);
    drain(&r);
}

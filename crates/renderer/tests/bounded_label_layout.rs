//! A crowded first frame must remain renderable and editable through the shared API.
#![cfg(not(target_arch = "wasm32"))]
#[path = "support/gpu.rs"]
mod test_gpu;
use ::renderer::{decoration::DecorationMap, layout::NudgeResult, *};
use std::sync::Arc;

fn gpu() -> RendererDevice {
    let (device, queue) = test_gpu::device();
    RendererDevice::new(Arc::new(device), Arc::new(queue))
}

fn labels(map: &DecorationMap) -> Vec<(HitId, DecorationId, layout::RectF)> {
    map.entries()
        .filter(|(_, e)| {
            matches!(
                e.id,
                DecorationId::DataLabel { .. } | DecorationId::RadialLabel(_)
            )
        })
        .map(|(hit, e)| (hit, e.id.clone(), map.bounds(hit).unwrap()))
        .collect()
}

fn editable(map: &DecorationMap, state: &mut DecorationLayout) -> DecorationId {
    let (hit, key, rect) = labels(map).pop().expect("labels must not disappear");
    assert_eq!(
        map.hit_test([rect.x + rect.width * 0.5, rect.y + rect.height * 0.5]),
        Some(hit)
    );
    assert_eq!(map.drag_by(state, hit, 0.5, 3.25), Some(NudgeResult::Moved));
    key
}

fn assert_moved(before: &DecorationMap, after: &DecorationMap, key: &DecorationId) {
    let find = |map: &DecorationMap| {
        let (hit, _) = map.entries().find(|(_, e)| &e.id == key).unwrap();
        map.bounds(hit).unwrap()
    };
    assert_eq!(find(after), find(before).translated(0.5, 3.25));
}

fn assert_separated(map: &DecorationMap) {
    let boxes = labels(map);
    for (i, (_, _, a)) in boxes.iter().enumerate() {
        for (_, _, b) in &boxes[i + 1..] {
            assert!(
                a.x + a.width <= b.x
                    || b.x + b.width <= a.x
                    || a.y + a.height <= b.y
                    || b.y + b.height <= a.y,
                "nearby free label positions were not used: {a:?}, {b:?}"
            );
        }
    }
}

fn save(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            std::path::Path::new(&path).join(format!("{name}.png")),
            encode_png(image).unwrap(),
        )
        .unwrap();
    }
}

fn assert_pixels(image: &RasterImage) {
    assert!(
        image
            .rgba
            .chunks_exact(4)
            .filter(|p| p[3] > 0 && p[0].abs_diff(p[2]) > 40)
            .count()
            > 100,
        "no colored chart pixels"
    );
    let png = encode_png(image).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!((info.width, info.height), (image.width, image.height));
    assert_eq!(&bytes[..info.buffer_size()], image.rgba.as_slice());
}

#[test]
fn crowded_box_labels_prepare_pick_drag_export_and_reset() {
    let mut renderer = BoxPlotRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut chart = BoxPlotChart {
        categories: vec![
            Category::new("a", "Control response group"),
            Category::new("b", "Treatment response group"),
        ],
        series: (0..3)
            .map(|i| {
                let mut value = BoxSummary::new(10., 30., 40., 60., 80.);
                value.sample_count = Some(10000);
                BoxPlotSeries::new(
                    i.to_string(),
                    i.to_string(),
                    vec![Some(value.clone()), Some(value)],
                    Color::from_rgb8(30, 110, 210),
                )
            })
            .collect(),
        labels: BoxPlotLabels::MedianAndCount,
        value_range: Some([0., 80.]),
        legend: false,
        ..Default::default()
    };
    for direction in [BoxPlotDirection::Vertical, BoxPlotDirection::Horizontal] {
        chart.direction = direction;
        for scale in [1., 2.] {
            chart.decorations = Default::default();
            let frame = renderer
                .prepare(&chart, (440, 320), scale)
                .expect("label spacing cannot reject the first frame");
            assert_eq!(labels(frame.decorations()).len(), 6);
            assert_separated(frame.decorations());
            let image = renderer.export_rgba(&chart, (440, 320), scale).unwrap();
            assert_pixels(&image);
            save(&format!("box-{direction:?}-{scale}"), &image);
            assert!(Arc::ptr_eq(
                &frame,
                &renderer.prepare(&chart, (440, 320), scale).unwrap()
            ));
            let key = editable(frame.decorations(), &mut chart.decorations);
            let moved = renderer.prepare(&chart, (440, 320), scale).unwrap();
            assert_moved(frame.decorations(), moved.decorations(), &key);
            assert_ne!(
                image.rgba,
                renderer
                    .export_rgba(&chart, (440, 320), scale)
                    .unwrap()
                    .rgba
            );
            chart.decorations.reset(&key);
            assert_eq!(
                image.rgba,
                renderer
                    .export_rgba(&chart, (440, 320), scale)
                    .unwrap()
                    .rgba
            );
            renderer.end_frame();
        }
    }
}

#[test]
fn crowded_radar_labels_prepare_pick_drag_export_and_reset() {
    let mut renderer = RadarRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut chart = RadarChart {
        categories: ["a", "b", "c", "d", "e"]
            .into_iter()
            .map(|s| Category::new(s, s.to_uppercase()))
            .collect(),
        series: (0..3)
            .map(|i| {
                RadarSeries::new(
                    i.to_string(),
                    i.to_string(),
                    vec![7., 7.2, 6.8, 7.1, 6.9],
                    Color::from_rgb8(30, 110, 210),
                )
            })
            .collect(),
        labels: RadarLabels::Values,
        value_range: Some([0., 10.]),
        legend: false,
        ..Default::default()
    };
    for shape in [RadarGridShape::Polygon, RadarGridShape::Circle] {
        chart.style.grid_shape = shape;
        for scale in [1., 2.] {
            chart.decorations = Default::default();
            let frame = renderer
                .prepare(&chart, (440, 320), scale)
                .expect("label spacing cannot reject the first frame");
            assert_eq!(labels(frame.decorations()).len(), 15);
            assert_separated(frame.decorations());
            let image = renderer.export_rgba(&chart, (440, 320), scale).unwrap();
            assert_pixels(&image);
            save(&format!("radar-{shape:?}-{scale}"), &image);
            assert!(Arc::ptr_eq(
                &frame,
                &renderer.prepare(&chart, (440, 320), scale).unwrap()
            ));
            let key = editable(frame.decorations(), &mut chart.decorations);
            let moved = renderer.prepare(&chart, (440, 320), scale).unwrap();
            assert_moved(frame.decorations(), moved.decorations(), &key);
            assert_ne!(
                image.rgba,
                renderer
                    .export_rgba(&chart, (440, 320), scale)
                    .unwrap()
                    .rgba
            );
            chart.decorations.reset(&key);
            assert_eq!(
                image.rgba,
                renderer
                    .export_rgba(&chart, (440, 320), scale)
                    .unwrap()
                    .rgba
            );
            renderer.end_frame();
        }
    }
}

#[test]
fn crowded_bar_labels_prepare_pick_drag_export_and_reset() {
    let mut renderer = CategoricalRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut chart = CategoricalChart {
        categories: vec![
            Category::new("a", "Control response group"),
            Category::new("b", "Treatment response group"),
        ],
        series: (0..3)
            .map(|i| {
                BarSeries::new(
                    i.to_string(),
                    i.to_string(),
                    vec![Some(40.), Some(40.)],
                    Color::from_rgb8(30, 110, 210),
                )
            })
            .collect(),
        label_format: CategoryBarLabelFormat::CategoryValue,
        labels: CategoryBarLabels::Outside,
        legend: false,
        ..Default::default()
    };
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        chart.direction = direction;
        for scale in [1., 2.] {
            chart.decorations = Default::default();
            let frame = renderer
                .prepare(&chart, (640, 360), scale)
                .expect("label spacing cannot reject the first frame");
            assert_eq!(labels(frame.decorations()).len(), 6);
            assert_separated(frame.decorations());
            let image = renderer.export_rgba(&chart, (640, 360), scale).unwrap();
            assert_pixels(&image);
            save(&format!("bar-{direction:?}-{scale}"), &image);
            let key = editable(frame.decorations(), &mut chart.decorations);
            let moved = renderer.prepare(&chart, (640, 360), scale).unwrap();
            assert_moved(frame.decorations(), moved.decorations(), &key);
            assert_ne!(
                image.rgba,
                renderer
                    .export_rgba(&chart, (640, 360), scale)
                    .unwrap()
                    .rgba
            );
            chart.decorations.reset(&key);
            assert_eq!(
                image.rgba,
                renderer
                    .export_rgba(&chart, (640, 360), scale)
                    .unwrap()
                    .rgba
            );
            renderer.end_frame();
        }
    }
}

#[test]
fn crowded_radial_labels_prepare_and_remain_in_the_hit_map() {
    let mut renderer = RadialRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut chart = RadialChart {
        slices: (0..16)
            .map(|i| RadialSlice::new(format!("Group {i}"), 1., Color::from_rgb8(30, 110, 210)))
            .collect(),
        ..Default::default()
    };
    chart.format_labels();
    for mode in [RadialLabels::Inside, RadialLabels::Outside] {
        chart.labels = mode;
        for scale in [1., 2.] {
            chart.decorations = Default::default();
            let frame = renderer
                .prepare(&chart, (420, 280), scale)
                .expect("label spacing cannot reject the first frame");
            assert_eq!(labels(frame.decorations()).len(), 16);
            let image = renderer.export_rgba(&chart, (420, 280), scale).unwrap();
            assert_pixels(&image);
            save(&format!("radial-{mode:?}-{scale}"), &image);
            let key = editable(frame.decorations(), &mut chart.decorations);
            let moved = renderer.prepare(&chart, (420, 280), scale).unwrap();
            assert_moved(frame.decorations(), moved.decorations(), &key);
            assert_ne!(
                image.rgba,
                renderer
                    .export_rgba(&chart, (420, 280), scale)
                    .unwrap()
                    .rgba
            );
            chart.decorations.reset(&key);
            assert_eq!(
                image.rgba,
                renderer
                    .export_rgba(&chart, (420, 280), scale)
                    .unwrap()
                    .rgba
            );
            renderer.end_frame();
        }
    }
    chart.font_size = 64.;
    chart.decorations = Default::default();
    let small = renderer.prepare(&chart, (240, 200), 1.).unwrap();
    assert_eq!(labels(small.decorations()).len(), 16);
    assert_pixels(&renderer.export_rgba(&chart, (240, 200), 1.).unwrap());
    renderer.end_frame();
}

#[test]
fn exhausted_label_space_still_returns_all_explicit_labels_and_stable_frames() {
    let mut renderer = CategoricalRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let chart = CategoricalChart {
        categories: (0..16)
            .map(|i| Category::new(i.to_string(), format!("Category {i}")))
            .collect(),
        series: (0..8)
            .map(|i| {
                BarSeries::new(
                    i.to_string(),
                    i.to_string(),
                    vec![Some(100.); 16],
                    Color::from_rgb8(30, 110, 210),
                )
            })
            .collect(),
        labels: CategoryBarLabels::Outside,
        label_format: CategoryBarLabelFormat::CategoryValue,
        bar_gap: 0.,
        legend: false,
        ..Default::default()
    };
    let original = chart.clone();
    let frame = renderer.prepare(&chart, (440, 320), 1.).unwrap();
    assert_eq!(labels(frame.decorations()).len(), 128);
    assert_eq!(chart, original, "automatic placement changed the model");
    assert!(Arc::ptr_eq(
        &frame,
        &renderer.prepare(&chart, (440, 320), 1.).unwrap()
    ));
    let before = renderer.export_rgba(&chart, (440, 320), 1.).unwrap();
    assert_pixels(&before);
    renderer.clear_cache();
    let again = renderer.prepare(&chart, (440, 320), 1.).unwrap();
    let geometry = |map: &DecorationMap| {
        labels(map)
            .into_iter()
            .map(|(_, key, r)| (key, r))
            .collect::<Vec<_>>()
    };
    assert_eq!(geometry(frame.decorations()), geometry(again.decorations()));
    assert_eq!(
        before.rgba,
        renderer.export_rgba(&chart, (440, 320), 1.).unwrap().rgba
    );
    renderer.end_frame();
}

//! A crowded first frame must remain renderable and editable through the shared API;
//! an inside label too small to read moves outside, or on an inner
//! sunburst ring is hidden, never shrunk (#357).
#![cfg(not(target_arch = "wasm32"))]
use test_gpu::BlockOn as _;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/radial_decorations.rs"]
mod radial_decorations;
#[path = "support/gpu.rs"]
mod test_gpu;
use ::renderer::{layout::NudgeResult, *};
use harness::fresh_gpu as gpu;
use radial_decorations::Decorations;

/// The slice labels of a radial frame.
fn radial_labels(frame: &RadialFrame, chart: &RadialChart) -> Vec<(HitId, DecorationId)> {
    frame
        .entries(chart)
        .into_iter()
        .filter(|(_, k)| matches!(k, DecorationId::RadialLabel(_)))
        .collect()
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
fn crowded_radial_labels_prepare_and_remain_in_the_hit_map() {
    let mut renderer = RadialRenderer::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut chart = RadialChart {
        slices: (0..16)
            .map(|i| RadialSlice::new(format!("Group {i}"), 1., Color::from_rgb8(30, 110, 210)))
            .collect(),
        ..Default::default()
    };
    chart.format_labels();
    // Too small for its slices, an inside label moves outside with a
    // leader (#357), never shrunk below the readable minimum: the crowded
    // pie draws exactly as with outside labels, and every label stays in
    // the hit map, hit, dragged and reset as an outside label is (the loop
    // below).
    chart.labels = RadialLabels::Inside;
    for scale in [1., 2.] {
        let frame = renderer
            .prepare(&chart, (420, 280), scale)
            .expect("label spacing cannot reject the first frame");
        assert_eq!(radial_labels(&*frame, &chart).len(), 16);
        let mut outside = chart.clone();
        outside.labels = RadialLabels::Outside;
        let export = |renderer: &mut RadialRenderer, chart: &RadialChart| {
            renderer
                .export_rgba(chart, (420, 280), renderer::ExportOptions::document(scale))
                .block_on()
                .unwrap()
                .rgba
        };
        assert_eq!(
            export(&mut renderer, &chart),
            export(&mut renderer, &outside)
        );
        renderer.end_frame();
    }
    // With room, every inside label is drawn, at least the minimum size,
    // and hit; outside labels are always drawn.
    for (mode, size) in [
        (RadialLabels::Inside, (1200, 900)),
        (RadialLabels::Inside, (420, 280)),
        (RadialLabels::Outside, (420, 280)),
    ] {
        chart.labels = mode;
        for scale in [1., 2.] {
            chart.decorations = Default::default();
            let frame = renderer
                .prepare(&chart, size, scale)
                .expect("label spacing cannot reject the first frame");
            assert_eq!(radial_labels(&*frame, &chart).len(), 16);
            if mode == RadialLabels::Inside && size == (1200, 900) {
                for (hit, _) in radial_labels(&*frame, &chart) {
                    let text = frame.text_bounds(&chart, hit).unwrap();
                    // Two lines at no less than 12 px.
                    assert!(text.height >= 2.0 * 12.0, "{text:?}");
                }
            }
            let image = renderer
                .export_rgba(&chart, size, renderer::ExportOptions::document(scale))
                .block_on()
                .unwrap();
            assert_pixels(&image);
            save(&format!("radial-{mode:?}-{scale}"), &image);
            let shown = chart.clone();
            let (hit, key) = radial_labels(&*frame, &chart).pop().unwrap();
            let text = frame.text_bounds(&shown, hit).unwrap();
            assert_eq!(
                frame.hit(
                    &shown,
                    [text.x + text.width * 0.5, text.y + text.height * 0.5]
                ),
                Some(hit)
            );
            assert_eq!(
                frame.drag(&mut chart, hit, 0.5, 3.25),
                Some(NudgeResult::Moved)
            );
            let moved = renderer.prepare(&chart, size, scale).unwrap();
            let after = moved
                .text_bounds(&chart, moved.id_of(&chart, &key))
                .unwrap();
            let want = text.translated(0.5, 3.25);
            assert!(
                [after.x - want.x, after.y - want.y, after.width - want.width]
                    .iter()
                    .all(|d| d.abs() < 1e-3),
                "{after:?} {want:?}"
            );
            assert_ne!(
                image.rgba,
                renderer
                    .export_rgba(&chart, size, renderer::ExportOptions::document(scale))
                    .block_on()
                    .unwrap()
                    .rgba
            );
            chart.decorations.reset(&key);
            assert_eq!(
                image.rgba,
                renderer
                    .export_rgba(&chart, size, renderer::ExportOptions::document(scale))
                    .block_on()
                    .unwrap()
                    .rgba
            );
            renderer.end_frame();
        }
    }
    chart.font_size = 64.;
    chart.decorations = Default::default();
    let small = renderer.prepare(&chart, (240, 200), 1.).unwrap();
    assert_eq!(radial_labels(&*small, &chart).len(), 16);
    assert_pixels(
        &renderer
            .export_rgba(&chart, (240, 200), renderer::ExportOptions::document(1.))
            .block_on()
            .unwrap(),
    );
    renderer.end_frame();
}

/// On an inner sunburst ring no leader reaches past the outer rings, so an
/// inside label too small to read there is hidden (#357): not drawn and not
/// hit. A wide segment on a thin ring at the side of the circle reads
/// upright and keeps its size instead.
#[test]
fn inner_sunburst_labels_read_upright_or_hide() {
    let mut renderer = RadialRenderer::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let node = |label: &str, value: f64, children: Vec<RadialSlice>| RadialSlice {
        children,
        ..RadialSlice::new(label, value, Color::from_rgb8(44, 160, 44))
    };
    let mut chart = RadialChart {
        kind: RadialKind::Donut { inner_radius: 0.14 },
        labels: RadialLabels::Inside,
        slices: vec![
            node(
                "Renewables",
                46.0,
                vec![
                    node("Wind", 30.0, vec![node("On", 20.0, vec![])]),
                    node("Tiny", 1.0, vec![]),
                ],
            ),
            node(
                "Rest",
                54.0,
                vec![node("All", 54.0, vec![node("Leaf", 54.0, vec![])])],
            ),
        ],
        ..Default::default()
    };
    chart.format_labels();
    for scale in [1., 2.] {
        let frame = renderer.prepare(&chart, (800, 600), scale).unwrap();
        let labels = radial_labels(&*frame, &chart);
        let target = |group, index| DecorationId::RadialLabel(RadialTarget { group, index });
        // "Renewables" (ring 0, a 166° segment) is drawn, upright, at its
        // full size: its box is taller than wide.
        let renewables = labels
            .iter()
            .find(|(_, k)| *k == target(0, 0))
            .expect("the wide segment keeps its label");
        let text = frame.text_bounds(&chart, renewables.0).unwrap();
        assert!(text.height > text.width, "{text:?}");
        assert!(text.height >= 2.0 * chart.font_size * 0.9, "{text:?}");
        // "Tiny" (ring 1, 1%) is hidden, so it cannot be hit.
        assert!(labels.iter().all(|(_, k)| *k != target(1, 1)));
        renderer.end_frame();
    }
}

/// With `InnerLabelFallback::Outside` (#357), an inner-ring label too small
/// to read is pulled outside instead of hidden: it stacks with the
/// outermost ring's outside labels without meeting them, is in the hit map,
/// outlined part by part (leader and text) when selected, and drags with
/// its leader, at 1x and 2x.
#[test]
fn pulled_out_inner_labels_are_full_outside_labels() {
    let mut renderer = RadialRenderer::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let node = |label: &str, value: f64, children: Vec<RadialSlice>| RadialSlice {
        children,
        ..RadialSlice::new(label, value, Color::from_rgb8(44, 160, 44))
    };
    let mut chart = RadialChart {
        kind: RadialKind::Donut { inner_radius: 0.14 },
        labels: RadialLabels::Outside,
        slices: vec![
            node(
                "Renewables",
                46.0,
                vec![
                    node("Wind", 30.0, vec![node("On", 20.0, vec![])]),
                    node("Tiny", 1.0, vec![]),
                ],
            ),
            node(
                "Rest",
                54.0,
                vec![node("All", 54.0, vec![node("Leaf", 54.0, vec![])])],
            ),
        ],
        inner_label_fallback: InnerLabelFallback::Outside,
        ..Default::default()
    };
    chart.format_labels();
    let tiny = DecorationId::RadialLabel(RadialTarget { group: 1, index: 1 });
    for scale in [1., 2.] {
        chart.decorations = Default::default();
        let frame = renderer.prepare(&chart, (800, 600), scale).unwrap();
        let labels = radial_labels(&*frame, &chart);
        let (hit, _) = *labels
            .iter()
            .find(|(_, k)| *k == tiny)
            .expect("the pulled-out label is in the hit map");
        // It never meets another label's text.
        let text = frame.text_bounds(&chart, hit).unwrap();
        for (other, _) in labels.iter().filter(|(h, _)| *h != hit) {
            if let Some(b) = frame.text_bounds(&chart, *other) {
                let meets = text.x < b.x + b.width
                    && b.x < text.x + text.width
                    && text.y < b.y + b.height
                    && b.y < text.y + text.height;
                assert!(!meets, "{text:?} meets {b:?}");
            }
        }
        // It is outside the circle: its text lies beyond the rim, and it
        // picks at its text's centre.
        assert_eq!(
            frame.hit(
                &chart,
                [text.x + text.width * 0.5, text.y + text.height * 0.5]
            ),
            Some(hit)
        );
        // Selected, it is outlined part by part, as an outside label is.
        let shown = chart.clone();
        let selection = frame.selection(&shown, hit).unwrap();
        assert!(
            matches!(selection.outline(), ::renderer::select::SelectionOutline::Parts(parts) if parts.len() >= 2),
            "leader and text: {:?}",
            selection.outline()
        );
        // Dragged, it and its leader move; reset, it returns.
        let before = renderer
            .export_rgba(&chart, (800, 600), renderer::ExportOptions::document(scale))
            .block_on()
            .unwrap();
        assert_eq!(
            frame.drag(&mut chart, hit, 4.0, 6.0),
            Some(NudgeResult::Moved)
        );
        let moved = renderer.prepare(&chart, (800, 600), scale).unwrap();
        let after = moved
            .text_bounds(&chart, moved.id_of(&chart, &tiny))
            .unwrap();
        assert!((after.x - text.x - 4.0).abs() < 1e-3 && (after.y - text.y - 6.0).abs() < 1e-3);
        let dragged = renderer
            .export_rgba(&chart, (800, 600), renderer::ExportOptions::document(scale))
            .block_on()
            .unwrap();
        assert_ne!(before.rgba, dragged.rgba);
        chart.decorations.reset(&tiny);
        let reset = renderer
            .export_rgba(&chart, (800, 600), renderer::ExportOptions::document(scale))
            .block_on()
            .unwrap();
        assert_eq!(before.rgba, reset.rgba);
        renderer.end_frame();
    }
    // Hidden by default: the same chart without the option has no such label.
    chart.inner_label_fallback = InnerLabelFallback::Hide;
    let frame = renderer.prepare(&chart, (800, 600), 1.).unwrap();
    assert!(
        radial_labels(&*frame, &chart)
            .iter()
            .all(|(_, k)| *k != tiny)
    );
}

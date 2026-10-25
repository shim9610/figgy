//! A crowded first frame must remain renderable and editable through the shared API.
#![cfg(not(target_arch = "wasm32"))]
#[path = "support/bounded_decorations.rs"]
mod bounded_decorations;
#[path = "support/gpu.rs"]
mod test_gpu;
use ::renderer::{layout::NudgeResult, *};
use bounded_decorations::Decorations;
use std::sync::Arc;

fn gpu() -> RendererDevice {
    let (device, queue) = test_gpu::device();
    RendererDevice::new(Arc::new(device), Arc::new(queue))
}

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
            assert_eq!(radial_labels(&*frame, &chart).len(), 16);
            let image = renderer.export_rgba(&chart, (420, 280), scale).unwrap();
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
            let moved = renderer.prepare(&chart, (420, 280), scale).unwrap();
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
    assert_eq!(radial_labels(&*small, &chart).len(), 16);
    assert_pixels(&renderer.export_rgba(&chart, (240, 200), 1.).unwrap());
    renderer.end_frame();
}

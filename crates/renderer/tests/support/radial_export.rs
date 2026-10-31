//! Shared native/WebGPU check of interaction state in actual decoded PNGs.
use renderer::{Color, RasterImage, RendererDevice, radial::*};

fn decoded(image: &RasterImage) -> Vec<u8> {
    let bytes = renderer::encode_png(image).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgba).unwrap();
    assert_eq!((info.width, info.height), (image.width, image.height));
    rgba.truncate(info.buffer_size());
    assert_eq!(rgba, image.rgba);
    rgba
}
fn red_top(rgba: &[u8], width: u32) -> usize {
    rgba.chunks_exact(4)
        .position(|p| p[0] > 180 && p[1] < 100 && p[2] < 100)
        .unwrap()
        / width as usize
}

pub async fn check(gpu: RendererDevice) {
    let mut r = RadialRenderer::new(gpu, wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut chart = RadialChart {
        kind: RadialKind::Donut { inner_radius: 0.45 },
        labels: RadialLabels::Inside,
        slices: vec![
            RadialSlice::new("A", 1., Color::from_rgb8(220, 70, 40)),
            RadialSlice::new("B", 1., Color::from_rgb8(40, 70, 220)),
        ],
        style: RadialStyle {
            depth: 0.2,
            tilt_degrees: 55.,
            hover_lift: 0.18,
            hover_brightness: 0.,
            bevel: 0.02,
            ..Default::default()
        },
        ..Default::default()
    };
    for scale in [1., 2.] {
        chart.interaction = Default::default();
        let rest = r
            .export_rgba_async(&chart, (400, 320), scale)
            .await
            .unwrap();
        let rest_png = decoded(&rest);
        r.end_frame();
        chart.interaction.hovered = Some(RadialTarget::main(0));
        chart.interaction.hover_progress = 0.5;
        let partial = r
            .export_rgba_async(&chart, (400, 320), scale)
            .await
            .unwrap();
        let partial_png = decoded(&partial);
        assert_ne!(rest_png, partial_png, "PNG lost partial hover");
        r.end_frame();
        chart.interaction.hover_progress = 1.;
        let hover_frame = r.prepare(&chart, (400, 320), scale).unwrap();
        let hover = r
            .export_rgba_async(&chart, (400, 320), scale)
            .await
            .unwrap();
        let hover_png = decoded(&hover);
        assert_ne!(partial_png, hover_png, "PNG ignored hover progress");
        assert!(
            red_top(&rest_png, rest.width)
                > red_top(&hover_png, hover.width) + (5. * scale) as usize,
            "export did not preserve the actual raised geometry"
        );
        r.end_frame();
        // Click selection survives pointer leave and must export the same lift
        // as a completed hover, without needing an active hovered target.
        chart.interaction.selected = chart.interaction.hovered.take();
        chart.interaction.hover_progress = 0.;
        let selected_frame = r.prepare(&chart, (400, 320), scale).unwrap();
        assert!(selected_frame.shares_annotations_with(&hover_frame));
        let selected = r
            .export_rgba_async(&chart, (400, 320), scale)
            .await
            .unwrap();
        assert_eq!(
            decoded(&selected),
            hover_png,
            "PNG lost persistent selection"
        );
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(dir) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
            std::fs::create_dir_all(&dir).unwrap();
            for (name, image) in [("rest", &rest), ("selected", &selected)] {
                std::fs::write(
                    format!("{dir}/radial-export-{name}-{scale}x.png"),
                    renderer::encode_png(image).unwrap(),
                )
                .unwrap();
            }
        }
        drop(hover_frame);
        drop(selected_frame);
        r.end_frame();
        chart.interaction.selected = None;
        assert_eq!(
            decoded(
                &r.export_rgba_async(&chart, (400, 320), scale)
                    .await
                    .unwrap()
            ),
            rest_png,
            "export retained a cleared selection"
        );
        r.end_frame();
    }
}

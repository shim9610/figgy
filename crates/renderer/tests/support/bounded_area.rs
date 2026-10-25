//! Exercise area editing through existing trait objects, not host geometry.
use ::renderer::*;
use std::sync::Arc;
#[path = "bounded_decorations.rs"]
mod decorations;
#[path = "bounded_selection.rs"]
mod selection;
use decorations::Decorations;
pub use selection::assert_selection_only;
#[cfg(not(target_arch = "wasm32"))]
pub use selection::assert_selection_with_rerasterized_label;

pub async fn check(gpu: RendererDevice) {
    macro_rules! check_chart {
        ($renderer:ident, $chart:expr) => {{
            let mut r = $renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
            let mut c = $chart;
            for scale in [1., 2.] {
                c.decorations = Default::default();
                let original = r.prepare(&c, (800, 520), scale).unwrap();
                assert!(Arc::ptr_eq(
                    &original,
                    &r.prepare(&c, (800, 520), scale).unwrap()
                ));
                // The chart the frame was prepared from, which its
                // decorations answer for while `c` is edited below.
                let shown = c.clone();
                let (hit, key) = original
                    .entries(&shown)
                    .into_iter()
                    .find(|(_, e)| e.element_id() == "data_area")
                    .expect(concat!(stringify!($renderer), " omitted data area"));
                let bounds = original.bounds(&shown, hit).unwrap();
                assert_eq!(original.selection(&shown, hit).unwrap().handles().len(), 8);
                let before = r.export_rgba_async(&c, (800, 520), scale).await.unwrap();
                r.end_frame();
                c.decorations.selected = Some(key.clone());
                let selected = r.export_rgba_async(&c, (800, 520), scale).await.unwrap();
                assert_selection_only(
                    &before,
                    &selected,
                    &original.selection(&shown, hit).unwrap(),
                    scale,
                    &format!("{} area at {scale}x", stringify!($renderer)),
                );
                c.decorations.selected = None;
                assert_eq!(
                    r.export_rgba_async(&c, (800, 520), scale)
                        .await
                        .unwrap()
                        .rgba,
                    before.rgba
                );
                r.end_frame();
                let state = c.decorations.clone();
                assert!(matches!(
                    original.resize(&mut c, hit, resize::ResizeHandle::NW, -10000., -10000.),
                    Some(layout::NudgeResult::Rejected(_))
                ));
                assert_eq!(
                    c.decorations, state,
                    "rejected corner resize partially changed state"
                );
                assert_eq!(
                    original.resize(&mut c, hit, resize::ResizeHandle::SE, -100., -80.),
                    Some(layout::NudgeResult::Moved)
                );
                c.decorations.selected = Some(key.clone());
                let smaller = r.prepare(&c, (800, 520), scale).unwrap();
                assert!(!Arc::ptr_eq(&original, &smaller));
                let sshown = c.clone();
                let sh = smaller.id_of(&sshown, &key);
                let sb = smaller.bounds(&sshown, sh).unwrap();
                let selection = smaller.selection(&sshown, sh).unwrap();
                let rect = selection.rect();
                let (left, top) = (rect.x, rect.y);
                let (right, bottom) = (rect.x + rect.width, rect.y + rect.height);
                let (cx, cy) = ((left + right) * 0.5, (top + bottom) * 0.5);
                use resize::ResizeHandle::{E, N, NE, NW, S, SE, SW, W};
                // Independent compass anchors, rather than reusing handle_rects.
                // Feed the picked handle into a real edit and prepare every result.
                for (expected, anchor, delta, change) in [
                    (NW, [left, top], [8., 6.], [8., 6., -8., -6.]),
                    (N, [cx, top], [8., 6.], [0., 6., 0., -6.]),
                    (NE, [right, top], [-8., 6.], [0., 6., -8., -6.]),
                    (E, [right, cy], [-8., 6.], [0., 0., -8., 0.]),
                    (SE, [right, bottom], [-8., -6.], [0., 0., -8., -6.]),
                    (S, [cx, bottom], [8., -6.], [0., 0., 0., -6.]),
                    (SW, [left, bottom], [8., -6.], [8., 0., -8., -6.]),
                    (W, [left, cy], [8., 6.], [8., 0., -8., 0.]),
                ] {
                    let context = format!("{} {expected:?} at {scale}x", stringify!($renderer));
                    let half = resize::HANDLE_SIZE * 0.5;
                    for offset in [
                        [0., 0.],
                        [-half + 0.5, -half + 0.5],
                        [half - 0.5, -half + 0.5],
                        [-half + 0.5, half - 0.5],
                        [half - 0.5, half - 0.5],
                    ] {
                        // Rendered pixels are physical; the public hit API is logical.
                        let pixel = [
                            (anchor[0] + offset[0]) * scale,
                            (anchor[1] + offset[1]) * scale,
                        ];
                        assert_eq!(
                            smaller.handle_at(&sshown, sh, [pixel[0] / scale, pixel[1] / scale]),
                            Some(expected),
                            "{context}: inside hit"
                        );
                    }
                    for offset in [
                        [-half - 0.5, 0.],
                        [half + 0.5, 0.],
                        [0., -half - 0.5],
                        [0., half + 0.5],
                    ] {
                        assert_eq!(
                            smaller.handle_at(
                                &sshown,
                                sh,
                                [anchor[0] + offset[0], anchor[1] + offset[1]]
                            ),
                            None,
                            "{context}: outside hit"
                        );
                    }
                    let picked = smaller
                        .handle_at(&sshown, sh, anchor)
                        .expect("handle center must pick");
                    let mut trial = c.clone();
                    assert_eq!(
                        smaller.resize(&mut trial, sh, picked, delta[0], delta[1]),
                        Some(layout::NudgeResult::Moved),
                        "{context}"
                    );
                    let frame = r.prepare(&trial, (800, 520), scale).unwrap();
                    let edited_id = frame.id_of(&trial, &key);
                    assert_eq!(
                        frame.bounds(&trial, edited_id).unwrap(),
                        layout::RectF {
                            x: sb.x + change[0],
                            y: sb.y + change[1],
                            width: sb.width + change[2],
                            height: sb.height + change[3],
                        },
                        "{context}: picked handle resized the wrong edges"
                    );
                    assert_eq!(
                        smaller.bounds(&sshown, sh),
                        Some(sb),
                        "{context}: previous frame changed"
                    );
                }
                assert_eq!(
                    smaller.handle_at(&sshown, sh, [cx, cy]),
                    None,
                    "area interior is not a resize handle"
                );
                assert_eq!(
                    (sb.x, sb.y, sb.width, sb.height),
                    (bounds.x, bounds.y, bounds.width - 100., bounds.height - 80.)
                );
                assert_eq!(
                    original.bounds(&shown, hit).unwrap(),
                    bounds,
                    "old frame geometry changed"
                );
                for (side, width_delta, height_delta) in [
                    (layout::Side::Bottom, 100., 0.),
                    (layout::Side::Left, 0., 80.),
                ] {
                    let axis = DecorationId::Axis(side);
                    if let Some((old_id, _)) = original
                        .entries(&shown)
                        .into_iter()
                        .find(|(_, e)| *e == axis)
                    {
                        let new_id = smaller.id_of(&sshown, &axis);
                        let old = original.bounds(&shown, old_id).unwrap();
                        let new = smaller.bounds(&sshown, new_id).unwrap();
                        assert!((new.width - (old.width - width_delta)).abs() < 0.01);
                        assert!((new.height - (old.height - height_delta)).abs() < 0.01);
                    }
                }
                let resized = r.export_rgba_async(&c, (800, 520), scale).await.unwrap();
                assert_ne!(before.rgba, resized.rgba, "area edit reused stale pixels");
                for handle in smaller.selection(&sshown, sh).unwrap().handles() {
                    let mut blue = 0;
                    for y in (handle.y * scale).floor() as u32
                        ..((handle.y + handle.height) * scale).ceil() as u32
                    {
                        for x in (handle.x * scale).floor() as u32
                            ..((handle.x + handle.width) * scale).ceil() as u32
                        {
                            let p = &resized.rgba[((y * resized.width + x) * 4) as usize..][..4];
                            blue +=
                                usize::from(p[2] > 140 && i32::from(p[2]) - i32::from(p[0]) > 30);
                        }
                    }
                    assert!(
                        blue > 0,
                        "resize handle was not drawn at the requested scale"
                    );
                }

                r.end_frame();
                assert_eq!(
                    smaller.drag(&mut c, sh, 20., 15.),
                    Some(layout::NudgeResult::Moved)
                );
                let moved = r.prepare(&c, (800, 520), scale).unwrap();
                let mh = moved.id_of(&c, &key);
                assert_eq!(moved.bounds(&c, mh).unwrap(), sb.translated(20., 15.));
                let image = r.export_rgba_async(&c, (800, 520), scale).await.unwrap();
                assert_ne!(image.rgba, resized.rgba);
                // Actual red data pixels, not selection boxes or labels, must
                // continue to pick from the newly prepared logical geometry.
                let mut picked = 0;
                for y in (0..image.height).step_by(11) {
                    for x in (0..image.width).step_by(11) {
                        let p = &image.rgba[((y * image.width + x) * 4) as usize..][..4];
                        if p[0] > 180
                            && p[1] < 100
                            && p[2] < 100
                            && sb
                                .translated(20., 15.)
                                .contains((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale)
                        {
                            assert!(
                                moved
                                    .has_data([(x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale])
                                    .await,
                                "visible data did not move with picking"
                            );
                            picked += 1;
                        }
                    }
                }
                assert!(picked > 10, "test rendered no usable data pixels");
                let bytes = encode_png(&image).unwrap();
                let mut png = png::Decoder::new(std::io::Cursor::new(&bytes))
                    .read_info()
                    .unwrap();
                let mut pixels = vec![0; png.output_buffer_size().unwrap()];
                let info = png.next_frame(&mut pixels).unwrap();
                assert_eq!(&pixels[..info.buffer_size()], image.rgba.as_slice());
                #[cfg(not(target_arch = "wasm32"))]
                if let Ok(dir) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(
                        format!("{dir}/{}-area-{scale}x.png", stringify!($renderer)),
                        bytes,
                    )
                    .unwrap();
                }
                #[cfg(feature = "serde")]
                {
                    let saved = serde_json::to_string(&c).unwrap();
                    let restored = serde_json::from_str(&saved).unwrap();
                    assert_eq!(c, restored);
                }
                r.end_frame();
                c.decorations.reset(&key);
                c.decorations.selected = None;
                assert_eq!(
                    r.export_rgba_async(&c, (800, 520), scale)
                        .await
                        .unwrap()
                        .rgba,
                    before.rgba,
                    "reset did not restore automatic area"
                );
                r.end_frame();
            }
        }};
    }
    let red = Color::from_rgb8(220, 70, 40);
    check_chart!(RadialRenderer, {
        let mut chart = RadialChart {
            title: "Resize data area".into(),
            slices: vec![RadialSlice::new("A", 1., red)],
            labels: RadialLabels::Inside,
            ..Default::default()
        };
        chart.format_labels();
        chart
    });
}

//! Exercise area editing through existing trait objects, not host geometry.
use ::renderer::*;
use std::sync::Arc;

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
                let map = original.decorations();
                let (hit, element) = map
                    .entries()
                    .find(|(_, e)| e.id.element_id() == "data_area")
                    .expect(concat!(stringify!($renderer), " omitted data area"));
                let key = element.id.clone();
                let bounds = map.bounds(hit).unwrap();
                assert_eq!(map.selection_box(hit).unwrap().handles.len(), 8);
                let target = map.hit_map().get(hit).unwrap();
                let resizable = target.as_resizable().expect("area must expose Resizable");
                let before = r.export_rgba_async(&c, (800, 520), scale).await.unwrap();
                r.end_frame();
                let state = c.decorations.clone();
                assert!(matches!(
                    resizable.resize_by(
                        &mut c.decorations,
                        resize::ResizeHandle::NW,
                        -10000.,
                        -10000.
                    ),
                    layout::NudgeResult::Rejected(_)
                ));
                assert_eq!(
                    c.decorations, state,
                    "rejected corner resize partially changed state"
                );
                assert_eq!(
                    resizable.resize_by(&mut c.decorations, resize::ResizeHandle::SE, -100., -80.),
                    layout::NudgeResult::Moved
                );
                c.decorations.selected = Some(key.clone());
                let smaller = r.prepare(&c, (800, 520), scale).unwrap();
                assert!(!Arc::ptr_eq(&original, &smaller));
                let smap = smaller.decorations();
                let sh = smap.entries().find(|(_, e)| e.id == key).unwrap().0;
                let sb = smap.bounds(sh).unwrap();
                assert_eq!(
                    (sb.x, sb.y, sb.width, sb.height),
                    (bounds.x, bounds.y, bounds.width - 100., bounds.height - 80.)
                );
                assert_eq!(
                    map.bounds(hit).unwrap(),
                    bounds,
                    "old frame geometry changed"
                );
                for (side, width_delta, height_delta) in [
                    (layout::Side::Bottom, 100., 0.),
                    (layout::Side::Left, 0., 80.),
                ] {
                    let axis = DecorationId::Axis(side);
                    if let Some((old_id, _)) = map.entries().find(|(_, e)| e.id == axis) {
                        let new_id = smap.entries().find(|(_, e)| e.id == axis).unwrap().0;
                        let old = map.bounds(old_id).unwrap();
                        let new = smap.bounds(new_id).unwrap();
                        assert!((new.width - (old.width - width_delta)).abs() < 0.01);
                        assert!((new.height - (old.height - height_delta)).abs() < 0.01);
                    }
                }
                let resized = r.export_rgba_async(&c, (800, 520), scale).await.unwrap();
                assert_ne!(before.rgba, resized.rgba, "area edit reused stale pixels");
                for handle in smap.selection_box(sh).unwrap().handles {
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
                    smap.drag_by(&mut c.decorations, sh, 20., 15.),
                    Some(layout::NudgeResult::Moved)
                );
                let moved = r.prepare(&c, (800, 520), scale).unwrap();
                let mmap = moved.decorations();
                let mh = mmap.entries().find(|(_, e)| e.id == key).unwrap().0;
                assert_eq!(mmap.bounds(mh).unwrap(), sb.translated(20., 15.));
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
                                    .hit_test([(x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale])
                                    .is_some(),
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
    check_chart!(
        RadialRenderer,
        RadialChart {
            title: "Resize data area".into(),
            slices: vec![RadialSlice::new("A", 1., red)],
            labels: RadialLabels::Inside,
            ..Default::default()
        }
    );
    check_chart!(
        CategoricalRenderer,
        CategoricalChart {
            title: "Resize data area".into(),
            categories: vec![Category::new("a", "Control")],
            series: vec![BarSeries::new("s", "Response", vec![Some(35.)], red)],
            ..Default::default()
        }
    );
    check_chart!(
        BoxPlotRenderer,
        BoxPlotChart {
            title: "Resize data area".into(),
            categories: vec![Category::new("a", "Control")],
            series: vec![BoxPlotSeries::new(
                "s",
                "Distribution",
                vec![Some(BoxSummary::new(10., 25., 38., 52., 70.))],
                red
            )],
            ..Default::default()
        }
    );
}

//! Use the actual common HitMap -> Selectable -> Draggable path, then render
//! and inspect pixels. A successful data pick does not cover decoration editing.
#![cfg(not(target_arch = "wasm32"))]
use pollster::FutureExt as _;
#[path = "support/gpu.rs"]
mod test_gpu;
use ::renderer::{
    layout::{NudgeResult, RectF, Side},
    *,
};
use std::sync::{Arc, Mutex, OnceLock};
static LOCK: Mutex<()> = Mutex::new(());
/// The shared test device in a bundle of its own per call, so each renderer
/// a test builds charges its own ledger and budget (#160).
fn gpu() -> RendererDevice {
    static GPU: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (d, q) = GPU.get_or_init(test_gpu::device);
    RendererDevice::new(d.clone(), q.clone())
}
#[path = "support/radial_decorations.rs"]
mod radial_decorations;
use radial_decorations::Decorations;
/// Equal up to the rounding of placing a moved element again: a radial
/// decoration is placed from its offset, not translated as a box.
fn near(a: Option<RectF>, b: Option<RectF>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => [a.x - b.x, a.y - b.y, a.width - b.width, a.height - b.height]
            .iter()
            .all(|d| d.abs() < 1e-3),
        (a, b) => a.is_none() && b.is_none(),
    }
}
/// Press `key` on frame `f` of `chart` where it picks, drag it by
/// `(dx, dy)` through its `Draggable`, and return what it moved: its text
/// bounds before the drag (a radial label's leader line keeps its start on
/// the slice).
fn moved<F: Decorations>(
    f: &F,
    chart: &mut F::Chart,
    key: &DecorationId,
    dx: f32,
    dy: f32,
) -> RectF {
    let shown = chart.clone();
    let id = f.id_of(&shown, key);
    let old = f.text_bounds(&shown, id).unwrap();
    // Elements hit inside their padded selection box, so a later element
    // drawn next to this one can cover the centre of its bounds. Press the
    // first point of a grid over the bounds, centre first, that picks the
    // element.
    let samples = (0..=8).flat_map(|j| (0..=8).map(move |i| (i, j)));
    let hit = std::iter::once((4, 4))
        .chain(samples)
        .map(|(i, j)| {
            [
                old.x + old.width * i as f32 / 8.0,
                old.y + old.height * j as f32 / 8.0,
            ]
        })
        .filter_map(|p| f.hit(&shown, p))
        .find(|hit| f.key_of(&shown, *hit).as_ref() == Some(key))
        .unwrap_or_else(|| panic!("visible decoration {key:?} must pick"));
    // The same trait-object workflow as existing Cartesian charts.
    assert_eq!(f.drag(chart, hit, dx, dy), Some(NudgeResult::Moved));
    old
}
fn ink(image: &RasterImage, r: RectF, scale: f32) -> usize {
    let x0 = (r.x * scale).floor().max(0.) as u32;
    let y0 = (r.y * scale).floor().max(0.) as u32;
    let x1 = ((r.x + r.width) * scale).ceil().min(image.width as f32) as u32;
    let y1 = ((r.y + r.height) * scale).ceil().min(image.height as f32) as u32;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| ((y * image.width + x) * 4) as usize))
        .filter(|&i| image.rgba[i..i + 3].iter().any(|v| *v < 200))
        .count()
}
fn save(name: &str, img: &RasterImage) {
    if let Ok(dir) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(format!("{dir}/{name}.png"), encode_png(img).unwrap()).unwrap();
    }
}
#[test]
fn radial_label_click_drag_changes_pixels_leader_and_roundtrips() {
    let _guard = LOCK.lock().unwrap();
    let mut r = RadialRenderer::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut c = RadialChart {
        title: "Editable donut labels".into(),
        style: RadialStyle::three_dimensional(),
        kind: RadialKind::Donut { inner_radius: 0.5 },
        slices: vec![
            RadialSlice::new("High", 35., Color::from_rgb8(100, 145, 185)),
            RadialSlice::new("Low", 65., Color::from_rgb8(175, 95, 140)),
        ],
        ..Default::default()
    };
    c.format_labels();
    for scale in [1., 2.] {
        c.decorations = Default::default();
        let f = r.prepare(&c, (800, 520), scale).unwrap();
        let before = r
            .export_rgba(&c, (800, 520), renderer::ExportOptions::document(scale))
            .block_on()
            .unwrap();
        let key = DecorationId::RadialLabel(RadialTarget::main(0));
        let shown = c.clone();
        let old = moved(&*f, &mut c, &key, 0., 45.);
        let g = r.prepare(&c, (800, 520), scale).unwrap();
        assert!(
            !f.shares_annotations_with(&g),
            "label movement must invalidate raster"
        );
        assert!(near(
            g.text_bounds(&c, g.id_of(&c, &key)),
            Some(old.translated(0., 45.))
        ));
        assert_eq!(
            f.text_bounds(&shown, f.id_of(&shown, &key)),
            Some(old),
            "old frame mutated"
        );
        // The leader line is a part of the label: it keeps its start on the
        // slice and ends at the moved text.
        let (before_all, after_all) = (
            f.bounds(&shown, f.id_of(&shown, &key)).unwrap(),
            g.bounds(&c, g.id_of(&c, &key)).unwrap(),
        );
        assert!(before_all.width > old.width && before_all.x < old.x);
        assert!(
            after_all.height > before_all.height,
            "the leader did not follow"
        );
        let mut hovering = c.clone();
        hovering.interaction.hovered = Some(RadialTarget::main(0));
        hovering.interaction.hover_progress = 1.0;
        let hover_frame = r.prepare(&hovering, (800, 520), scale).unwrap();
        assert!(
            g.shares_annotations_with(&hover_frame),
            "edited outside labels must still reuse their raster on data hover"
        );
        let after = r
            .export_rgba(&c, (800, 520), renderer::ExportOptions::document(scale))
            .block_on()
            .unwrap();
        assert!(ink(&before, old, scale) > 20);
        assert!(
            ink(&after, old, scale) < ink(&before, old, scale) / 3,
            "old glyphs remain"
        );
        assert!(ink(&after, old.translated(0., 45.), scale) > 20);
        assert_eq!(c.slices[0].value, 35.);
        #[cfg(feature = "serde")]
        {
            let restored: RadialChart =
                serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
            assert_eq!(restored, c);
            assert_eq!(
                r.export_rgba(
                    &restored,
                    (800, 520),
                    renderer::ExportOptions::document(scale)
                )
                .block_on()
                .unwrap()
                .rgba,
                after.rgba
            );
        }
        c.decorations.selected = Some(key.clone());
        let selected = r
            .export_rgba(&c, (800, 520), renderer::ExportOptions::document(scale))
            .block_on()
            .unwrap();
        assert_ne!(after.rgba, selected.rgba, "selection box missing");
        save("radial-label-selected", &selected);
        let current = c.decorations.clone();
        let hit = g.id_of(&c, &key);
        assert!(matches!(
            g.drag(&mut c, hit, 1e6, 0.),
            Some(NudgeResult::Rejected(_))
        ));
        assert_eq!(c.decorations, current);
        c.decorations = Default::default();
        assert_eq!(
            r.export_rgba(&c, (800, 520), renderer::ExportOptions::document(scale))
                .block_on()
                .unwrap()
                .rgba,
            before.rgba,
            "reset not restored"
        );
        r.end_frame();
    }
}
#[test]
fn inside_radial_labels_pick_at_lifted_position_and_drag_without_jump() {
    let _guard = LOCK.lock().unwrap();
    let mut r = RadialRenderer::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut c = RadialChart {
        labels: RadialLabels::Inside,
        slices: vec![RadialSlice::new("High", 1., Color::from_rgb8(20, 65, 115))],
        ..Default::default()
    };
    c.format_labels();
    c.style.tilt_degrees = 55.;
    c.style.depth = 0.18;
    let key = DecorationId::RadialLabel(RadialTarget::main(0));
    // Inside-label annotations have a separate lifted rendering path. Keep
    // data hover fixed while changing only decoration selection at both DPIs.
    for scale in [1., 2.] {
        for hovered in [false, true] {
            c.interaction.hovered = hovered.then_some(RadialTarget::main(0));
            c.interaction.hover_progress = if hovered { 1. } else { 0. };
            let frame = r.prepare(&c, (600, 420), scale).unwrap();
            let shown = c.clone();
            let selection = frame.selection(&shown, frame.id_of(&shown, &key)).unwrap();
            let unselected = r
                .export_rgba(&c, (600, 420), renderer::ExportOptions::document(scale))
                .block_on()
                .unwrap();
            c.decorations.selected = Some(key.clone());
            let selected_frame = r.prepare(&c, (600, 420), scale).unwrap();
            let label = frame.bounds(&shown, frame.id_of(&shown, &key)).unwrap();
            assert_eq!(
                selected_frame.bounds(&c, selected_frame.id_of(&c, &key)),
                Some(label)
            );
            let selected = r
                .export_rgba(&c, (600, 420), renderer::ExportOptions::document(scale))
                .block_on()
                .unwrap();
            save(
                &format!("radial-inside-before-hover-{hovered}-{scale}x"),
                &unselected,
            );
            save(
                &format!("radial-inside-after-hover-{hovered}-{scale}x"),
                &selected,
            );
            let context = format!("inside radial label, hovered={hovered}, scale={scale}");
            if hovered {
                radial_area::assert_selection_with_rerasterized_label(
                    &unselected,
                    &selected,
                    &selection,
                    scale,
                    label,
                    &context,
                );
            } else {
                radial_area::assert_selection_only(
                    &unselected,
                    &selected,
                    &selection,
                    scale,
                    &context,
                );
            }
            save(
                &format!("radial-inside-selection-only-hover-{hovered}-{scale}x"),
                &selected,
            );
            c.decorations.selected = None;
            assert_eq!(
                r.export_rgba(&c, (600, 420), renderer::ExportOptions::document(scale))
                    .block_on()
                    .unwrap()
                    .rgba,
                unselected.rgba
            );
            r.end_frame();
        }
    }
    c.interaction = Default::default();
    let rest = r.prepare(&c, (600, 420), 2.).unwrap();
    let base = rest.bounds(&c, rest.id_of(&c, &key)).unwrap();
    c.interaction.hovered = Some(RadialTarget::main(0));
    c.interaction.hover_progress = 1.;
    let lifted = r.prepare(&c, (600, 420), 2.).unwrap();
    assert!(
        rest.shares_annotations_with(&lifted),
        "ordinary hover must keep raster cache"
    );
    let before = lifted.bounds(&c, lifted.id_of(&c, &key)).unwrap();
    assert!(before.y < base.y);
    moved(&*lifted, &mut c, &key, 20., 5.);
    c.decorations.selected = Some(key.clone());
    let moved = r.prepare(&c, (600, 420), 2.).unwrap();
    assert!(near(
        moved.bounds(&c, moved.id_of(&c, &key)),
        Some(before.translated(20., 5.))
    ));
    let image = r
        .export_rgba(&c, (600, 420), renderer::ExportOptions::document(2.))
        .block_on()
        .unwrap();
    // White glyphs must be present in their measured edited bounds.
    let b = before.translated(20., 5.);
    let mut white = 0;
    for y in (b.y * 2.).floor() as u32..((b.y + b.height) * 2.).ceil() as u32 {
        for x in (b.x * 2.).floor() as u32..((b.x + b.width) * 2.).ceil() as u32 {
            let i = ((y * image.width + x) * 4) as usize;
            if image.rgba[i..i + 3].iter().all(|c| *c > 230) {
                white += 1;
            }
        }
    }
    assert!(white > 30, "edited inside glyphs disappeared");
    save("radial-inside-label-selected", &image);
    r.end_frame();
}

#[path = "support/radial_area.rs"]
mod radial_area;
#[test]
fn radial_area_select_resize_move_pick_and_export() {
    let _guard = LOCK.lock().unwrap();
    pollster::block_on(radial_area::check(gpu()));
}

#[test]
fn every_radial_decoration_uses_public_edit_reset_and_snapshot_contracts() {
    let _guard = LOCK.lock().unwrap();
    // `$charts` are the variants of one family. The first runs the whole
    // contract at 1x and 2x;
    // the others, at 2x, run what depends on the layout: the move, its
    // redraw, the raster invalidation, the frame reuse and the geometry.
    // `$geometry` compares the data geometry of the frame before and after
    // a decoration edit, which must not move.
    macro_rules! check {
        ($renderer:ident, $charts:expr, $keys:expr) => {
            check!($renderer, $charts, $keys, |_: &_, _: &_| {})
        };
        ($renderer:ident, $charts:expr, $keys:expr, $geometry:expr) => {{
            let mut r = $renderer::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
            let geometry = $geometry;
            for (variant, mut c) in $charts.into_iter().enumerate() {
                let full = variant == 0;
                let scales: &[f32] = if full { &[1.0, 2.0] } else { &[2.0] };
                for &scale in scales {
                    for key in $keys {
                        c.decorations = Default::default();
                        let f = r.prepare(&c, (800, 600), scale).unwrap();
                        let shown = c.clone();
                        let area = f.bounds(&shown, f.id_of(&shown, &DecorationId::DataArea));
                        let before = r
                            .export_rgba(&c, (800, 600), renderer::ExportOptions::document(scale))
                            .block_on()
                            .unwrap();
                        if full {
                            // Selection is the only mutation here: movement must not mask
                            // a missing selection border or stale annotation cache.
                            let unselected_model = c.clone();
                            let selection = f.selection(&shown, f.id_of(&shown, &key)).unwrap();
                            c.decorations.selected = Some(key.clone());
                            let selected_frame = r.prepare(&c, (800, 600), scale).unwrap();
                            assert!(!f.shares_annotations_with(&selected_frame));
                            assert!(Arc::ptr_eq(
                                &selected_frame,
                                &r.prepare(&c, (800, 600), scale).unwrap()
                            ));
                            for (hit, element) in f.entries(&shown) {
                                assert_eq!(
                                    f.bounds(&shown, hit),
                                    selected_frame.bounds(&c, selected_frame.id_of(&c, &element)),
                                    "selection changed {element:?} geometry",
                                );
                            }
                            let selected = r
                                .export_rgba(
                                    &c,
                                    (800, 600),
                                    renderer::ExportOptions::document(scale),
                                )
                                .block_on()
                                .unwrap();
                            radial_area::assert_selection_only(
                                &before,
                                &selected,
                                &selection,
                                scale,
                                &format!("{} {key:?} at {scale}x", stringify!($renderer)),
                            );
                            let artifact_key: String = format!("{key:?}")
                                .chars()
                                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                                .collect();
                            save(
                                &format!(
                                    "{}-{artifact_key}-selection-only-{scale}x",
                                    stringify!($renderer)
                                ),
                                &selected,
                            );
                            c.decorations.selected = None;
                            assert_eq!(c, unselected_model);
                            assert_eq!(
                                r.export_rgba(
                                    &c,
                                    (800, 600),
                                    renderer::ExportOptions::document(scale)
                                )
                                .block_on()
                                .unwrap()
                                .rgba,
                                before.rgba,
                                "selection removal did not restore {key:?} pixels"
                            );
                        }
                        let (dx, dy) = match key {
                            DecorationId::Axis(Side::Left) => (8.0, 0.0),
                            DecorationId::Axis(Side::Bottom) => (0.0, -8.0),
                            DecorationId::Legend | DecorationId::AxisLabel(Side::Bottom) => {
                                (7.0, -5.0)
                            }
                            _ => (7.0, 5.0),
                        };
                        let old = moved(&*f, &mut c, &key, dx, dy);
                        c.decorations.selected = Some(key.clone());
                        let edited = r.prepare(&c, (800, 600), scale).unwrap();
                        assert!(
                            near(
                                edited.text_bounds(&c, edited.id_of(&c, &key)),
                                Some(old.translated(dx, dy))
                            ),
                            "{} {key:?} at {scale}x",
                            stringify!($renderer)
                        );
                        assert_eq!(f.text_bounds(&shown, f.id_of(&shown, &key)), Some(old));
                        assert_eq!(
                            edited.bounds(&c, edited.id_of(&c, &DecorationId::DataArea)),
                            area,
                            "moving a decoration changed the data area"
                        );
                        geometry(&*f, &*edited);
                        assert!(!f.shares_annotations_with(&edited));
                        assert!(Arc::ptr_eq(
                            &edited,
                            &r.prepare(&c, (800, 600), scale).unwrap()
                        ));
                        let after = r
                            .export_rgba(&c, (800, 600), renderer::ExportOptions::document(scale))
                            .block_on()
                            .unwrap();
                        assert_ne!(before.rgba, after.rgba, "{key:?} edit was invisible");
                        #[cfg(feature = "serde")]
                        if full {
                            let restored =
                                serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
                            assert_eq!(c, restored);
                            assert_eq!(
                                r.export_rgba(
                                    &restored,
                                    (800, 600),
                                    renderer::ExportOptions::document(scale)
                                )
                                .block_on()
                                .unwrap()
                                .rgba,
                                after.rgba
                            );
                        }
                        c.decorations.reset(&key);
                        c.decorations.selected = None;
                        if full {
                            assert_eq!(
                                r.export_rgba(
                                    &c,
                                    (800, 600),
                                    renderer::ExportOptions::document(scale)
                                )
                                .block_on()
                                .unwrap()
                                .rgba,
                                before.rgba
                            );
                        }
                        r.end_frame();
                    }
                }
                c.title.clear();
                let hidden = r.prepare(&c, (800, 600), 1.0).unwrap();
                assert!(
                    !hidden
                        .entries(&c)
                        .iter()
                        .any(|(_, e)| *e == DecorationId::ChartTitle)
                );
                r.end_frame();
            }
        }};
    }
    let color = Color::from_rgb8(70, 130, 180);
    check!(
        RadialRenderer,
        [{
            let mut chart = RadialChart {
                title: "Editable donut title".into(),
                kind: RadialKind::Donut { inner_radius: 0.5 },
                slices: vec![
                    RadialSlice::new("A", 35.0, color),
                    RadialSlice::new("B", 65.0, Color::BLACK),
                ],
                ..Default::default()
            };
            chart.format_labels();
            chart
        }],
        [
            DecorationId::ChartTitle,
            DecorationId::RadialLabel(RadialTarget::main(0))
        ]
    );
}

#[test]
fn independent_rich_titles_refresh_export_cache_and_keep_public_drag_contract() {
    let _guard = LOCK.lock().unwrap();
    // All operations use public model/frame/decoration APIs. CPU layout tests
    // separately compare title vs label crops at both output scales.
    macro_rules! check {
        ($renderer:ty, $chart:expr) => {{
            let mut r = <$renderer>::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
            let mut c = $chart;
            c.title = "Title H2O".into();
            c.normalize_title();
            for scale in [1., 2.] {
                let original = c.clone();
                let first = r.prepare(&c, (800, 520), scale).unwrap();
                let same = r.prepare(&c, (800, 520), scale).unwrap();
                assert!(first.shares_annotations_with(&same));
                let before = r
                    .export_rgba(&c, (800, 520), renderer::ExportOptions::document(scale))
                    .block_on()
                    .unwrap();
                let t = c.chart_title.as_mut().unwrap();
                t.text.segments = text::rich_segments_from_text("Independent title\nH2O");
                t.text.segments[0].bold = true;
                t.text
                    .segments
                    .iter_mut()
                    .find(|s| s.text == '2')
                    .unwrap()
                    .subscript = true;
                t.text.align = TextAlign::Right;
                t.text.color = Color::from_rgb8(160, 30, 70);
                let edited = r.prepare(&c, (800, 520), scale).unwrap();
                assert!(!first.shares_annotations_with(&edited));
                let after = r
                    .export_rgba(&c, (800, 520), renderer::ExportOptions::document(scale))
                    .block_on()
                    .unwrap();
                assert_ne!(before.rgba, after.rgba);
                // The cached renderer must match a fresh renderer for the edit.
                let mut fresh =
                    <$renderer>::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
                assert_eq!(
                    fresh
                        .export_rgba(&c, (800, 520), renderer::ExportOptions::document(scale))
                        .block_on()
                        .unwrap()
                        .rgba,
                    after.rgba
                );
                let key = DecorationId::ChartTitle;
                let old = moved(&*edited, &mut c, &key, 9., 7.);
                let dragged = r.prepare(&c, (800, 520), scale).unwrap();
                let bounds = dragged.text_bounds(&c, dragged.id_of(&c, &key)).unwrap();
                assert!(near(Some(bounds), Some(old.translated(9., 7.))));
                let image = r
                    .export_rgba(&c, (800, 520), renderer::ExportOptions::document(scale))
                    .block_on()
                    .unwrap();
                assert!(ink(&image, bounds, scale) > 20);
                let encoded = encode_png(&image).unwrap();
                assert_eq!(&encoded[..8], b"\x89PNG\r\n\x1a\n");
                #[cfg(feature = "serde")]
                {
                    let restored =
                        serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
                    assert_eq!(
                        r.export_rgba(
                            &restored,
                            (800, 520),
                            renderer::ExportOptions::document(scale)
                        )
                        .block_on()
                        .unwrap()
                        .rgba,
                        image.rgba
                    );
                }
                c.chart_title.as_mut().unwrap().visible = false;
                let hidden = r.prepare(&c, (800, 520), scale).unwrap();
                assert!(!hidden.entries(&c).iter().any(|(_, e)| *e == key));
                assert_ne!(
                    r.export_rgba(&c, (800, 520), renderer::ExportOptions::document(scale))
                        .block_on()
                        .unwrap()
                        .rgba,
                    image.rgba
                );
                c = original;
                r.end_frame();
            }
        }};
    }
    check!(RadialRenderer, {
        let mut chart = RadialChart {
            slices: vec![
                RadialSlice::new("A", 30., Color::BLACK),
                RadialSlice::new("B", 70., Color::from_rgb8(70, 120, 170)),
            ],
            ..Default::default()
        };
        chart.format_labels();
        chart
    });
}

/// The frame's decoration methods take the host's chart, which may differ
/// from the one the frame was prepared with (#152). A chart that does not
/// validate (here 200 outside labels in one circle, past
/// `MAX_RADIAL_SLICES`, about 100 on each side) cannot be laid out: every
/// method answers nothing and leaves the chart unchanged instead of
/// overflowing the per-side label table. A 64-slice chart (the most one
/// circle holds) with 63 outside labels on one side, the most a full circle
/// puts there, prepares, hit-tests, drags and exports at 1x and 2x.
#[test]
fn a_chart_other_than_the_prepared_one_answers_nothing_and_never_panics() {
    let _guard = LOCK.lock().unwrap();
    let mut r = RadialRenderer::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let size = (800, 520);
    let slice = |i: usize, value: f64| {
        RadialSlice::new(
            format!("S{i}"),
            value,
            Color::from_rgb8((i * 37 % 255) as u8, 90, 160),
        )
    };
    let mut prepared = RadialChart {
        title: "Prepared".into(),
        kind: RadialKind::Pie,
        labels: RadialLabels::Outside,
        slices: (0..4).map(|i| slice(i, 1.0)).collect(),
        ..Default::default()
    };
    prepared.format_labels();
    let mut other = prepared.clone();
    other.slices = (0..200).map(|i| slice(i, 1.0)).collect();
    other.format_labels();
    assert!(other.validate().is_err(), "the other chart is malformed");
    let label = DecorationId::RadialLabel(RadialTarget::main(0));
    for scale in [1.0, 2.0] {
        let f = r.prepare(&prepared, size, scale).unwrap();
        let hits = [
            f.decoration(&label).expect("the prepared label"),
            f.decoration(&DecorationId::DataArea)
                .expect("the plot area"),
            f.decoration(&DecorationId::ChartTitle).expect("the title"),
        ];
        for j in 0..=26 {
            for i in 0..=40 {
                let p = [i as f32 * 20.0, j as f32 * 20.0];
                assert_eq!(f.decoration_at(&other, p), None, "hit at {p:?}");
            }
        }
        let before = other.clone();
        for hit in hits {
            assert_eq!(f.decoration_bounds(&other, hit), None);
            assert!(f.decoration_selection_box(&other, hit).is_none());
            assert_eq!(
                f.decoration_resize_handle(&other, hit, [400.0, 260.0]),
                None
            );
            let dragged = f.drag_decoration(&mut other, hit, 12.0, -7.0);
            assert_ne!(dragged, Some(NudgeResult::Moved), "{hit:?}");
            let resized = f.resize_decoration(
                &mut other,
                hit,
                ::renderer::resize::ResizeHandle::SE,
                12.0,
                -7.0,
            );
            assert_ne!(resized, Some(NudgeResult::Moved), "{hit:?}");
            assert_eq!(other, before, "a refused edit changed the chart");
        }
        r.end_frame();
    }

    // 63 tiny slices cluster on the right of the start angle; the large
    // one's label sits alone on the left.
    let mut full = RadialChart {
        title: "Crowded".into(),
        kind: RadialKind::Pie,
        labels: RadialLabels::Outside,
        start_angle_degrees: -10.0,
        slices: (0..::renderer::radial::MAX_RADIAL_SLICES)
            .map(|i| {
                slice(
                    i,
                    if i + 1 == ::renderer::radial::MAX_RADIAL_SLICES {
                        1e4
                    } else {
                        1.0
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    full.format_labels();
    full.validate().unwrap();
    for scale in [1.0, 2.0] {
        let f = r.prepare(&full, size, scale).unwrap();
        let mut hit_labels = 0;
        for index in 0..::renderer::radial::MAX_RADIAL_SLICES {
            let key = DecorationId::RadialLabel(RadialTarget::main(index));
            let hit = f.decoration(&key).expect("every label is an element");
            let bounds = f
                .decoration_bounds(&full, hit)
                .expect("every label is placed");
            assert!(f.decoration_selection_box(&full, hit).is_some());
            let centre = [
                bounds.x + bounds.width * 0.5,
                bounds.y + bounds.height * 0.5,
            ];
            if f.decoration_at(&full, centre) == Some(hit) {
                hit_labels += 1;
            }
        }
        assert!(hit_labels > 0, "no label hit-tests at {scale}x");
        let mut dragged = full.clone();
        let hit = f
            .decoration(&DecorationId::RadialLabel(RadialTarget::main(0)))
            .unwrap();
        assert_eq!(
            f.drag_decoration(&mut dragged, hit, 0.0, 6.0),
            Some(NudgeResult::Moved)
        );
        r.end_frame();
        let image = r
            .export_rgba(&full, size, renderer::ExportOptions::document(scale))
            .block_on()
            .unwrap();
        assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
        assert!(
            !r.export_svg(&full, size, renderer::ExportOptions::document(scale))
                .block_on()
                .unwrap()
                .is_empty()
        );
        assert!(
            !r.export_pdf(&full, size, renderer::ExportOptions::document(scale))
                .block_on()
                .unwrap()
                .is_empty()
        );
        r.export_rgba(&dragged, size, renderer::ExportOptions::document(scale))
            .block_on()
            .unwrap();
    }
}

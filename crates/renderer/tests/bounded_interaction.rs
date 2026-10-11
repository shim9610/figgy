//! Use the actual common HitMap -> Selectable -> Draggable path, then render
//! and inspect pixels. A successful data pick does not cover decoration editing.
#![cfg(not(target_arch = "wasm32"))]
#[path = "support/gpu.rs"]
mod test_gpu;
use ::renderer::{
    layout::{NudgeResult, RectF, Side},
    *,
};
use std::sync::{Arc, Mutex, OnceLock};
static LOCK: Mutex<()> = Mutex::new(());
fn gpu() -> RendererDevice {
    static GPU: OnceLock<RendererDevice> = OnceLock::new();
    GPU.get_or_init(|| {
        let (d, q) = test_gpu::device();
        RendererDevice::new(Arc::new(d), Arc::new(q))
    })
    .clone()
}
fn id(map: &decoration::DecorationMap, element: &DecorationId) -> HitId {
    map.entries()
        .find(|(_, e)| &e.id == element)
        .unwrap_or_else(|| panic!("missing {element:?}"))
        .0
}
fn moved(
    map: &decoration::DecorationMap,
    state: &mut DecorationLayout,
    key: &DecorationId,
    dx: f32,
    dy: f32,
) -> RectF {
    let id = id(map, key);
    let old = map.bounds(id).unwrap();
    let hit = map
        .hit_test([old.x + old.width * 0.5, old.y + old.height * 0.5])
        .expect("visible decoration must pick");
    assert_eq!(&map.element(hit).unwrap().id, key);
    // The same trait-object workflow as existing Cartesian charts.
    let element = map.hit_map().get(hit).unwrap();
    assert_eq!(element.element_id(), key.element_id());
    assert_eq!(
        element.as_draggable().unwrap().drag_by(state, dx, dy),
        NudgeResult::Moved
    );
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
    let mut r = RadialRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
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
    for scale in [1., 2.] {
        c.decorations = Default::default();
        let f = r.prepare(&c, (800, 520), scale).unwrap();
        let before = r.export_rgba(&c, (800, 520), scale).unwrap();
        let key = DecorationId::RadialLabel(RadialTarget::main(0));
        let old = moved(f.decorations(), &mut c.decorations, &key, 0., 45.);
        let g = r.prepare(&c, (800, 520), scale).unwrap();
        assert!(
            !f.shares_annotations_with(&g),
            "label movement must invalidate raster"
        );
        assert_eq!(
            g.decorations().bounds(id(g.decorations(), &key)).unwrap(),
            old.translated(0., 45.)
        );
        assert_eq!(
            f.decorations().bounds(id(f.decorations(), &key)).unwrap(),
            old,
            "old frame mutated"
        );
        let mut hovering = c.clone();
        hovering.interaction.hovered = Some(RadialTarget::main(0));
        hovering.interaction.hover_progress = 1.0;
        let hover_frame = r.prepare(&hovering, (800, 520), scale).unwrap();
        assert!(
            g.shares_annotations_with(&hover_frame),
            "edited outside labels must still reuse their raster on data hover"
        );
        let after = r.export_rgba(&c, (800, 520), scale).unwrap();
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
                r.export_rgba(&restored, (800, 520), scale).unwrap().rgba,
                after.rgba
            );
        }
        c.decorations.selected = Some(key.clone());
        let selected = r.export_rgba(&c, (800, 520), scale).unwrap();
        assert_ne!(after.rgba, selected.rgba, "selection box missing");
        save("radial-label-selected", &selected);
        let current = c.decorations.clone();
        assert!(matches!(
            g.decorations()
                .drag_by(&mut c.decorations, id(g.decorations(), &key), 1e6, 0.),
            Some(NudgeResult::Rejected(_))
        ));
        assert_eq!(c.decorations, current);
        c.decorations = Default::default();
        assert_eq!(
            r.export_rgba(&c, (800, 520), scale).unwrap().rgba,
            before.rgba,
            "reset not restored"
        );
        r.end_frame();
    }
}
#[test]
fn boxplot_all_chrome_uses_shared_select_drag_and_preserves_data() {
    let _guard = LOCK.lock().unwrap();
    let mut r = BoxPlotRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut c = BoxPlotChart {
        title: "Editable box plot".into(),
        value_title: "Value".into(),
        category_title: "Group".into(),
        categories: vec![
            Category::new("a", "Control"),
            Category::new("b", "Treatment"),
        ],
        series: vec![BoxPlotSeries::new(
            "s",
            "Distribution",
            vec![
                Some(BoxSummary::new(10., 25., 38., 52., 70.)),
                Some(BoxSummary::new(18., 32., 47., 61., 80.)),
            ],
            Color::from_rgb8(144, 69, 154),
        )],
        ..Default::default()
    };
    for direction in [BoxPlotDirection::Vertical, BoxPlotDirection::Horizontal] {
        c.direction = direction;
        for key in [
            DecorationId::ChartTitle,
            DecorationId::Legend,
            DecorationId::AxisLabel(Side::Left),
            DecorationId::AxisLabel(Side::Bottom),
            DecorationId::AxisTitle(Side::Left),
            DecorationId::AxisTitle(Side::Bottom),
            DecorationId::Axis(Side::Left),
            DecorationId::Axis(Side::Bottom),
        ] {
            c.decorations = Default::default();
            let f = r.prepare(&c, (800, 560), 2.).unwrap();
            let before = r.export_rgba(&c, (800, 560), 2.).unwrap();
            let (dx, dy) = match key {
                DecorationId::Axis(Side::Left) => (8., 0.),
                DecorationId::Axis(Side::Bottom) => (0., -8.),
                DecorationId::AxisLabel(Side::Bottom) | DecorationId::Legend => (7., -5.),
                _ => (7., 5.),
            };
            let old = moved(f.decorations(), &mut c.decorations, &key, dx, dy);
            c.decorations.selected = Some(key.clone());
            let g = r.prepare(&c, (800, 560), 2.).unwrap();
            assert_eq!(
                g.decorations().bounds(id(g.decorations(), &key)).unwrap(),
                old.translated(dx, dy)
            );
            assert_eq!(g.plot_rect(), f.plot_rect());
            assert_eq!(
                g.box_rect(&BoxPlotTarget::new("a", "s")),
                f.box_rect(&BoxPlotTarget::new("a", "s"))
            );
            let after = r.export_rgba(&c, (800, 560), 2.).unwrap();
            assert_ne!(before.rgba, after.rgba, "{key:?} didn't redraw");
            assert!(!f.shares_annotations_with(&g));
            assert!(Arc::ptr_eq(&g, &r.prepare(&c, (800, 560), 2.).unwrap()));
            if key == DecorationId::Legend {
                save("boxplot-legend-selected", &after);
            }
            r.end_frame();
        }
    }
}
#[test]
fn categorical_chrome_survives_hover_cache_reorder_and_resize() {
    let _guard = LOCK.lock().unwrap();
    let mut r = CategoricalRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut c = CategoricalChart {
        title: "Editable bars".into(),
        categories: vec![
            Category::new("a", "Control"),
            Category::new("b", "Treatment"),
        ],
        series: vec![BarSeries::new(
            "s",
            "Trial",
            vec![Some(30.), Some(60.)],
            Color::from_rgb8(60, 130, 210),
        )],
        ..Default::default()
    };
    let f = r.prepare(&c, (800, 560), 1.).unwrap();
    let key = DecorationId::DataLabel {
        category_id: "a".into(),
        series_id: "s".into(),
    };
    moved(f.decorations(), &mut c.decorations, &key, 30., 0.);
    let edited = r.prepare(&c, (800, 560), 1.).unwrap();
    c.hovered = Some(CategoryBarTarget::new("a", "s"));
    let hover = r.prepare(&c, (800, 560), 1.).unwrap();
    assert!(edited.shares_annotations_with(&hover));
    assert_eq!(
        edited.decorations().bounds(id(edited.decorations(), &key)),
        hover.decorations().bounds(id(hover.decorations(), &key))
    );
    c.reorder_categories(&["b", "a"]).unwrap();
    let reordered = r.prepare(&c, (900, 600), 2.).unwrap();
    let bounds = reordered
        .decorations()
        .bounds(id(reordered.decorations(), &key))
        .unwrap();
    let mut plain = c.clone();
    plain.decorations = Default::default();
    let default = r.prepare(&plain, (900, 600), 2.).unwrap();
    assert_eq!(
        bounds,
        default
            .decorations()
            .bounds(id(default.decorations(), &key))
            .unwrap()
            .translated(30., 0.)
    );
    assert!(reordered.decorations().hit_test([f32::NAN, 0.]).is_none());
    c.decorations.selected = Some(key);
    save(
        "categorical-label-selected",
        &r.export_rgba(&c, (900, 600), 2.).unwrap(),
    );
    r.end_frame();
}
#[test]
fn inside_radial_labels_pick_at_lifted_position_and_drag_without_jump() {
    let _guard = LOCK.lock().unwrap();
    let mut r = RadialRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut c = RadialChart {
        labels: RadialLabels::Inside,
        slices: vec![RadialSlice::new("High", 1., Color::from_rgb8(20, 65, 115))],
        ..Default::default()
    };
    c.style.tilt_degrees = 55.;
    c.style.depth = 0.18;
    let key = DecorationId::RadialLabel(RadialTarget::main(0));
    let rest = r.prepare(&c, (600, 420), 2.).unwrap();
    let base = rest
        .decorations()
        .bounds(id(rest.decorations(), &key))
        .unwrap();
    c.interaction.hovered = Some(RadialTarget::main(0));
    c.interaction.hover_progress = 1.;
    let lifted = r.prepare(&c, (600, 420), 2.).unwrap();
    assert!(
        rest.shares_annotations_with(&lifted),
        "ordinary hover must keep raster cache"
    );
    let before = lifted
        .decorations()
        .bounds(id(lifted.decorations(), &key))
        .unwrap();
    assert!(before.y < base.y);
    moved(lifted.decorations(), &mut c.decorations, &key, 20., 5.);
    c.decorations.selected = Some(key.clone());
    let moved = r.prepare(&c, (600, 420), 2.).unwrap();
    assert_eq!(
        moved
            .decorations()
            .bounds(id(moved.decorations(), &key))
            .unwrap(),
        before.translated(20., 5.)
    );
    let image = r.export_rgba(&c, (600, 420), 2.).unwrap();
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

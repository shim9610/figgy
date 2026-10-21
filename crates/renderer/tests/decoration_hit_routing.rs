//! Hit routing between decoration elements, checked with the renderer's
//! measure, which lays out the drawn tick labels and axis strokes: colourbar
//! details are independent targets that follow the strip, and an active
//! overlay's elements answer on the overlay panel and drag only the overlay
//! axis. Ported from the model, which cannot lay tick labels out itself.

use renderer::layout::{ChartArea, NudgeReject, NudgeResult, Rect, Side};
use renderer::select::SELECTION_PADDING;
use renderer::text::RichText;
use renderer::{
    Color, ColorBarAxisElement, ColorBarLabelElement, ColorBarTitleElement, Config, CpuTextMeasure,
    HitMap, LayerRef, Selectable,
};

fn measure(cfg: &Config) -> CpuTextMeasure {
    CpuTextMeasure::for_style(&cfg.draw_style)
}

fn cfg_800x600() -> Config {
    let mut cfg = renderer::default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    });
    cfg
}

fn cfg_with_colorbar(side: Side) -> Config {
    let mut cfg = cfg_800x600();
    let mut bar = renderer::default::default_colorbar_options();
    bar.side = side;
    cfg.colorbar = Some(bar);
    cfg
}

fn cfg_with_overlay() -> Config {
    let mut cfg = cfg_800x600();
    let mut overlay = renderer::OverlayLayer::default();
    overlay.y_axis.title_option.visible = true;
    overlay.y_axis.title_option.text = RichText::plain("Pressure", Color::BLACK, 12.0, "");
    overlay.y_axis.out_margin = 100.0;
    overlay.series = vec!["p".into()];
    cfg.overlays.push(overlay);
    cfg
}

#[test]
fn colorbar_axis_labels_and_title_are_independent_hit_targets() {
    for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
        let mut cfg = cfg_with_colorbar(side.clone());
        {
            let bar = cfg.colorbar.as_mut().expect("colourbar");
            bar.axis.title_option.visible = true;
            bar.axis.title_option.text = RichText::plain("z", Color::BLACK, 12.0, "");
        }
        let map = HitMap::standard_chart();

        let axis = ColorBarAxisElement
            .bounds(&cfg, &measure(&cfg))
            .expect("axis bounds");
        let axis_hit = map
            .hit_test(
                &cfg,
                &measure(&cfg),
                axis.x + axis.width * 0.5,
                axis.y + axis.height * 0.5,
            )
            .expect("axis hit");
        assert_eq!(map.get(axis_hit).unwrap().element_id(), "colorbar_axis");
        let axis_selection = map
            .selection_box(axis_hit, &cfg, &measure(&cfg))
            .expect("axis selection box");
        assert_eq!(axis_selection.rect, axis.expanded(SELECTION_PADDING));
        assert!(axis_selection.handles.is_empty());

        let labels = ColorBarLabelElement
            .bounds(&cfg, &measure(&cfg))
            .expect("label bounds");
        let label_point = match side {
            Side::Top | Side::Bottom => (labels.x + 1.0, labels.y + labels.height * 0.5),
            Side::Left | Side::Right => (labels.x + labels.width * 0.5, labels.y + 1.0),
        };
        let label_hit = map
            .hit_test(&cfg, &measure(&cfg), label_point.0, label_point.1)
            .expect("label hit");
        assert_eq!(
            map.get(label_hit).unwrap().element_id(),
            "colorbar_tick_labels"
        );
        let label_selection = map
            .selection_box(label_hit, &cfg, &measure(&cfg))
            .expect("label selection box");
        assert_eq!(label_selection.rect, labels.expanded(SELECTION_PADDING));
        assert!(label_selection.handles.is_empty());

        let title = ColorBarTitleElement
            .bounds(&cfg, &measure(&cfg))
            .expect("title bounds");
        let title_hit = map
            .hit_test(
                &cfg,
                &measure(&cfg),
                title.x + title.width * 0.5,
                title.y + title.height * 0.5,
            )
            .expect("title hit");
        assert_eq!(map.get(title_hit).unwrap().element_id(), "colorbar_title");
        let title_selection = map
            .selection_box(title_hit, &cfg, &measure(&cfg))
            .expect("title selection box");
        assert_eq!(title_selection.rect, title.expanded(SELECTION_PADDING));
        assert!(title_selection.handles.is_empty());
    }
}

#[test]
fn colorbar_detail_bounds_follow_the_painted_strip_offset() {
    let mut cfg = cfg_with_colorbar(Side::Right);
    {
        let bar = cfg.colorbar.as_mut().expect("colourbar");
        bar.length_frac = 0.4;
        bar.align = renderer::config::BarAlign::End;
        bar.axis.title_option.visible = true;
        bar.axis.title_option.text = RichText::plain("intensity", Color::BLACK, 12.0, "");
    }
    let before = [
        ColorBarAxisElement.bounds(&cfg, &measure(&cfg)).unwrap(),
        ColorBarLabelElement.bounds(&cfg, &measure(&cfg)).unwrap(),
        ColorBarTitleElement.bounds(&cfg, &measure(&cfg)).unwrap(),
    ];
    {
        let bar = cfg.colorbar.as_mut().expect("colourbar");
        bar.offset_x += 17.0;
        bar.offset_y -= 11.0;
    }
    let after = [
        ColorBarAxisElement.bounds(&cfg, &measure(&cfg)).unwrap(),
        ColorBarLabelElement.bounds(&cfg, &measure(&cfg)).unwrap(),
        ColorBarTitleElement.bounds(&cfg, &measure(&cfg)).unwrap(),
    ];
    for (before, after) in before.into_iter().zip(after) {
        assert_eq!(after, before.translated(17.0, -11.0));
    }
}

#[test]
fn overlay_elements_hit_and_drag_the_overlay_axis_only() {
    let mut cfg = cfg_with_overlay();
    cfg.set_active_layer(LayerRef::Overlay(0));
    let map = HitMap::for_active_layer(&cfg);
    let da = cfg.data_area().unwrap().0;
    let panel = cfg.overlay_panel(0).unwrap();
    assert_eq!(panel.data_area().unwrap().0, da);
    let right_edge = (da.x + da.width) as f32;
    let mid_y = da.y as f32 + da.height as f32 / 2.0;
    // Ticks point inside by default: the axis band starts at the edge.
    let hit = map
        .hit_test(&cfg, &measure(&cfg), right_edge - 1.0, mid_y)
        .expect("the overlay axis band is on the right edge");
    assert_eq!(map.get(hit).unwrap().element_id(), "overlay_0:axis_right");
    let labels = map.find("overlay_0:tick_labels_right").unwrap();
    let label_bounds = map
        .get(labels)
        .unwrap()
        .bounds(&cfg, &measure(&cfg))
        .unwrap();
    assert!(label_bounds.x > right_edge, "{label_bounds:?}");
    let title = map.find("overlay_0:axis_title_right").unwrap();
    let title_bounds = map
        .get(title)
        .unwrap()
        .bounds(&cfg, &measure(&cfg))
        .unwrap();
    assert!(title_bounds.x > label_bounds.x, "{title_bounds:?}");
    // No per-layer legend: no bounds, no hit.
    let legend = map.find("overlay_0:legend").unwrap();
    assert!(
        map.get(legend)
            .unwrap()
            .bounds(&cfg, &measure(&cfg))
            .is_none()
    );

    let before = cfg.clone();
    let axis = map.get(hit).unwrap().as_draggable().unwrap();
    assert_eq!(axis.drag_by(&mut cfg, -12.0, 40.0), NudgeResult::Moved);
    assert_eq!(cfg.overlays[0].y_axis.line_offset, -12.0);
    assert_eq!(cfg.right_y, before.right_y);
    assert_eq!(cfg.data_area().unwrap().0, da);
    let labels_drag = map.get(labels).unwrap().as_draggable().unwrap();
    assert_eq!(labels_drag.drag_by(&mut cfg, 3.0, -4.0), NudgeResult::Moved);
    assert_eq!(cfg.overlays[0].y_axis.label_style.label_offset_x, 3.0);
    assert_eq!(cfg.overlays[0].y_axis.label_style.label_offset_y, -4.0);
    let title_drag = map.get(title).unwrap().as_draggable().unwrap();
    assert_eq!(title_drag.drag_by(&mut cfg, 0.0, 10.0), NudgeResult::Moved);
    assert_ne!(
        cfg.overlays[0].y_axis.title_option,
        before.overlays[0].y_axis.title_option
    );
    assert_eq!(cfg.right_y, before.right_y);
    assert_eq!(cfg.left_y, before.left_y);
    assert_eq!(
        cfg.overlays[0].y_axis.out_margin,
        before.overlays[0].y_axis.out_margin
    );
    // A move that would leave the chart area is refused atomically.
    let snapshot = cfg.clone();
    assert_eq!(
        axis.drag_by(&mut cfg, 10_000.0, 0.0),
        NudgeResult::Rejected(NudgeReject::OutOfBounds)
    );
    assert_eq!(cfg, snapshot);
}

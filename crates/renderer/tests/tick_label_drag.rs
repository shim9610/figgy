//! A dragged tick-label set keeps its selection box and hit region on the
//! drawn labels: after a drag along or across the axis, the drawn labels, the
//! box and the hit-map answer all move by the drag.

use renderer::axis_render::try_raster_chart_to_rgba;
use renderer::config::TickVisibility;
use renderer::layout::{ChartArea, NudgeResult, Rect, RectF, Side};
use renderer::{
    AxisLabelElement, ColorBarLabelElement, Config, CpuTextMeasure, Draggable, HitMap, Selectable,
};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// A chart with no axis lines, ticks, titles or grid, so the only ink that a
/// label set adds is its labels.
fn bare_config(colorbar: bool) -> Config {
    let mut cfg = renderer::default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    cfg.chart_title.visible = false;
    for axis in [
        &mut cfg.top_x,
        &mut cfg.bottom_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.label_style.visible = true;
        axis.label_style.label_visible = true;
        axis.title_option.visible = false;
    }
    cfg.grid.show_major_x = false;
    cfg.grid.show_major_y = false;
    cfg.grid.show_minor_x = false;
    cfg.grid.show_minor_y = false;
    if colorbar {
        let mut bar = renderer::default::default_colorbar_options();
        bar.axis.line_visible = false;
        bar.axis.tick = TickVisibility::None;
        bar.axis.title_option.visible = false;
        cfg.colorbar = Some(bar);
    }
    cfg
}

/// The label set one case drags.
#[derive(Clone, Debug)]
enum Labels {
    Axis(Side),
    ColorBar,
}

impl Labels {
    fn name(&self) -> String {
        match self {
            Labels::Axis(side) => AxisLabelElement { side: side.clone() }.element_id(),
            Labels::ColorBar => ColorBarLabelElement.element_id(),
        }
    }

    fn element(&self) -> Box<dyn Selectable> {
        match self {
            Labels::Axis(side) => Box::new(AxisLabelElement { side: side.clone() }),
            Labels::ColorBar => Box::new(ColorBarLabelElement),
        }
    }

    fn drag(&self, cfg: &mut Config, dx: f32, dy: f32) {
        let result = match self {
            Labels::Axis(side) => AxisLabelElement { side: side.clone() }.drag_by(cfg, dx, dy),
            Labels::ColorBar => ColorBarLabelElement.drag_by(cfg, dx, dy),
        };
        assert_eq!(result, NudgeResult::Moved, "{self:?} by ({dx}, {dy})");
    }

    fn hide(&self, cfg: &mut Config) {
        let ls = match self {
            Labels::Axis(Side::Top) => &mut cfg.top_x.label_style,
            Labels::Axis(Side::Bottom) => &mut cfg.bottom_x.label_style,
            Labels::Axis(Side::Left) => &mut cfg.left_y.label_style,
            Labels::Axis(Side::Right) => &mut cfg.right_y.label_style,
            Labels::ColorBar => &mut cfg.colorbar.as_mut().unwrap().axis.label_style,
        };
        ls.label_visible = false;
    }
}

/// Bounding box of the pixels `labels` adds to the raster of `cfg`: the
/// raster with the set shown against the raster with it hidden.
fn label_ink(cfg: &Config, labels: &Labels) -> RectF {
    let shown = try_raster_chart_to_rgba(cfg).unwrap();
    let mut without = cfg.clone();
    labels.hide(&mut without);
    let hidden = try_raster_chart_to_rgba(&without).unwrap();
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let i = ((y * WIDTH + x) * 4) as usize;
            if shown[i..i + 4] != hidden[i..i + 4] {
                let (x0, y0, x1, y1) = found.unwrap_or((x, y, x, y));
                found = Some((x0.min(x), y0.min(y), x1.max(x), y1.max(y)));
            }
        }
    }
    let (x0, y0, x1, y1) = found.unwrap_or_else(|| panic!("{labels:?} drew nothing"));
    RectF {
        x: x0 as f32,
        y: y0 as f32,
        width: (x1 - x0 + 1) as f32,
        height: (y1 - y0 + 1) as f32,
    }
}

fn bounds(cfg: &Config, labels: &Labels) -> RectF {
    labels
        .element()
        .bounds(cfg, &CpuTextMeasure::for_style(&cfg.draw_style))
        .unwrap_or_else(|| panic!("{labels:?} has no bounds"))
}

/// The element the standard hit map answers at `(x, y)`.
fn hit(cfg: &Config, x: f32, y: f32) -> Option<String> {
    let map = HitMap::standard_chart();
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let id = map.hit_test(cfg, &measure, x, y)?;
    Some(map.get(id)?.element_id())
}

fn cases() -> [(Labels, bool); 5] {
    [
        (Labels::Axis(Side::Bottom), false),
        (Labels::Axis(Side::Top), false),
        (Labels::Axis(Side::Left), false),
        (Labels::Axis(Side::Right), false),
        (Labels::ColorBar, true),
    ]
}

/// The along-axis direction of a label set and its inward normal, towards
/// the middle of the chart.
fn directions(labels: &Labels, cfg: &Config) -> ((f32, f32), (f32, f32)) {
    let side = match labels {
        Labels::Axis(side) => side.clone(),
        Labels::ColorBar => cfg.colorbar.as_ref().unwrap().side.clone(),
    };
    match side {
        Side::Top => ((1.0, 0.0), (0.0, 1.0)),
        Side::Bottom => ((1.0, 0.0), (0.0, -1.0)),
        Side::Left => ((0.0, 1.0), (1.0, 0.0)),
        Side::Right => ((0.0, 1.0), (-1.0, 0.0)),
    }
}

// The drawn labels, the bounds and the selection box move by the drag, along
// the axis as well as across it. Before the fix, a drag along the axis moved
// the labels and left the bounds on the data area.
#[test]
fn a_dragged_label_set_keeps_its_bounds_on_the_drawn_labels() {
    for (labels, colorbar) in cases() {
        let base = bare_config(colorbar);
        let ((ax, ay), (nx, ny)) = directions(&labels, &base);
        let ink_before = label_ink(&base, &labels);
        let bounds_before = bounds(&base, &labels);
        for (dx, dy) in [
            (40.0 * ax, 40.0 * ay),
            (-40.0 * ax, -40.0 * ay),
            (6.0 * nx, 6.0 * ny),
            (25.0 * ax + 10.0 * nx, 25.0 * ay + 10.0 * ny),
        ] {
            let mut cfg = base.clone();
            labels.drag(&mut cfg, dx, dy);
            let ink = label_ink(&cfg, &labels);
            let moved = bounds(&cfg, &labels);
            let case = format!("{labels:?} dragged by ({dx}, {dy})");

            // The labels themselves moved by the drag (whole-pixel ink).
            assert!(
                (ink.x - ink_before.x - dx).abs() <= 1.0
                    && (ink.y - ink_before.y - dy).abs() <= 1.0,
                "{case}: ink {ink_before:?} -> {ink:?}"
            );
            // The bounds moved by exactly the same drag.
            assert_eq!(
                (moved.x - bounds_before.x, moved.y - bounds_before.y),
                (dx, dy),
                "{case}: bounds {bounds_before:?} -> {moved:?}"
            );
            assert_eq!(
                (moved.width, moved.height),
                (bounds_before.width, bounds_before.height)
            );

            // The selection box keeps its padding around the moved bounds.
            let selection = |cfg: &Config| {
                labels
                    .element()
                    .selection_box(cfg, &CpuTextMeasure::for_style(&cfg.draw_style))
                    .unwrap()
                    .rect
            };
            let padding = bounds_before.x - selection(&base).x;
            assert_eq!(selection(&cfg), moved.expanded(padding), "{case}");

            // A click on the middle of the drawn labels hits this label set.
            let (cx, cy) = (ink.x + ink.width * 0.5, ink.y + ink.height * 0.5);
            assert_eq!(
                hit(&cfg, cx, cy),
                Some(labels.name()),
                "{case}: click ({cx}, {cy})"
            );
        }
    }
}

// A drag along the axis moves the hit region with the labels: where only the
// dragged labels are, the click lands on them, and the stretch they left
// behind no longer answers.
#[test]
fn a_drag_along_the_axis_moves_the_hit_region() {
    for (labels, colorbar) in cases() {
        let base = bare_config(colorbar);
        let ((ax, ay), _) = directions(&labels, &base);
        let before = bounds(&base, &labels);
        let shift = 60.0;
        let mut cfg = base.clone();
        labels.drag(&mut cfg, shift * ax, shift * ay);
        let after = bounds(&cfg, &labels);

        // A point inside the new region but outside the old one, and the
        // converse, a little in from the moved edge.
        let mid = (after.x + after.width * 0.5, after.y + after.height * 0.5);
        let (gained, left) = if ax > 0.0 {
            (
                (before.x + before.width + shift * 0.5, mid.1),
                (before.x + shift * 0.5, mid.1),
            )
        } else {
            (
                (mid.0, before.y + before.height + shift * 0.5),
                (mid.0, before.y + shift * 0.5),
            )
        };
        assert!(after.contains(gained.0, gained.1) && !before.contains(gained.0, gained.1));
        assert!(before.contains(left.0, left.1) && !after.contains(left.0, left.1));
        assert_eq!(
            hit(&cfg, gained.0, gained.1),
            Some(labels.name()),
            "{labels:?}"
        );
        assert_ne!(hit(&cfg, left.0, left.1), Some(labels.name()), "{labels:?}");
    }
}

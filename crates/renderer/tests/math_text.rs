//! Math text in every decoration that draws a rich text (#45, rule 6): the
//! chart title, the axis titles (the left one rotated), the legend and the
//! colourbar title, at 1x, 2x and 3x. Each formula's ink lies inside the
//! element's measured bounds, the hit test answers at its ink and not past
//! its box, the selection box is the bounds padded, and a drag moves the
//! pixels and the bounds together. The legend keeps its labels' node runs
//! whole through its update and remove helpers, and a tall formula grows
//! the title band.
//!
//! The decoration rasters are the CPU placement pass the renderer uploads;
//! the catalogue scenes (`support/playground_scenes.rs`) carry math titles,
//! labels and annotations through the decoration oracle's GPU exports,
//! overlays and subplots.

use renderer::axis_render::try_raster_chart_to_rgba;
use renderer::config::TickVisibility;
use renderer::layout::{ChartArea, NudgeResult, Rect, RectF, Side};
use renderer::legend::{
    append_legend_entry_rich, legend_entry_count, remove_legend_entry, set_legend_entry_label,
};
use renderer::select::SELECTION_PADDING;
use renderer::text::{RichSegment, RichText};
use renderer::{
    AxisTitleElement, ChartTitleElement, Color, ColorBarTitleElement, Config, CpuTextMeasure,
    Draggable, LegendElement, SelectionGeometry,
};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;
/// One pixel of anti-aliasing, and the half pixel the glyph raster snaps a
/// baseline by, as for any text.
const SLACK: f32 = 1.5;

/// The formulas each element is checked with: scripts, a fraction, a big
/// operator with limits, a radical with its index, delimiters around a
/// fraction, an accent and an over brace.
const FORMULAS: &[&str] = &[
    r"Flux $\Phi = \oint \vec{E}\cdot d\vec{A}$",
    r"$\frac{\partial u}{\partial t} = \alpha \nabla^2 u$",
    r"$\sum\limits_{i=1}^{n} \sqrt[3]{x_i^2}$",
    r"$\left(\frac{a}{b}\right)^{2} + \overbrace{x+y}^{k}$",
];

/// A chart whose only ink is its decorations' text: no axis lines, ticks,
/// tick labels or grid.
fn bare() -> Config {
    let mut cfg = renderer::default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    for axis in [
        &mut cfg.top_x,
        &mut cfg.bottom_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.label_style.visible = false;
        axis.label_style.label_visible = false;
        axis.title_option.visible = false;
        axis.out_margin = 60.0;
    }
    cfg.chart_title.top_margin = 90.0;
    cfg.grid.show_major_x = false;
    cfg.grid.show_major_y = false;
    cfg.grid.show_minor_x = false;
    cfg.grid.show_minor_y = false;
    cfg.legend.visible = false;
    cfg
}

fn tex(src: &str, size: f32) -> RichText {
    RichText::from_tex(src, Color::BLACK, size, "").unwrap_or_else(|e| panic!("{src}: {e}"))
}

/// The pixels where two rasters of one size differ, as a box.
fn diff_box(a: &[u8], b: &[u8], width: u32) -> Option<RectF> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for (i, (pa, pb)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        if pa != pb {
            let (x, y) = (i as u32 % width, i as u32 / width);
            let (x0, y0, x1, y1) = found.unwrap_or((x, y, x, y));
            found = Some((x0.min(x), y0.min(y), x1.max(x), y1.max(y)));
        }
    }
    found.map(|(x0, y0, x1, y1)| RectF {
        x: x0 as f32,
        y: y0 as f32,
        width: (x1 - x0 + 1) as f32,
        height: (y1 - y0 + 1) as f32,
    })
}

fn inside(inner: RectF, outer: RectF, slack: f32) -> bool {
    inner.x >= outer.x - slack
        && inner.y >= outer.y - slack
        && inner.x + inner.width <= outer.x + outer.width + slack
        && inner.y + inner.height <= outer.y + outer.height + slack
}

/// One element with a math text: `with` the formula, `without` the same
/// chart with the element's text emptied, so their difference is the
/// formula's ink.
fn check_element<E: SelectionGeometry + Draggable>(
    name: &str,
    element: &E,
    with: &Config,
    without: &Config,
    tight: bool,
) {
    for scale in [1.0f32, 2.0, 3.0] {
        let (cfg, empty) = if scale == 1.0 {
            (with.clone(), without.clone())
        } else {
            (with.scaled(scale), without.scaled(scale))
        };
        let width = cfg.chart_area.0.width;
        let at = format!("{name} at {scale}x");
        let measure = CpuTextMeasure::for_style(&cfg.draw_style);
        let base = try_raster_chart_to_rgba(&empty).expect("raster");
        let drawn = try_raster_chart_to_rgba(&cfg).expect("raster");
        let ink = diff_box(&drawn, &base, width).unwrap_or_else(|| panic!("{at}: no ink"));
        let bounds = element.bounds(&cfg, &measure).expect("bounds");
        assert!(
            inside(ink, bounds, SLACK),
            "{at}: ink {ink:?} outside {bounds:?}"
        );
        if tight {
            // The bounds are the formula's ink, not a guess around it.
            assert!(
                inside(bounds, ink, 3.0 * scale),
                "{at}: {bounds:?} loose around {ink:?}"
            );
        }
        // Hit test: at the ink's centre it hits, past the padded box not.
        let (cx, cy) = (ink.x + ink.width * 0.5, ink.y + ink.height * 0.5);
        assert!(
            element.contains(&cfg, &measure, cx, cy),
            "{at}: centre misses"
        );
        let past = bounds.x + bounds.width + SELECTION_PADDING + 2.0;
        assert!(
            !element.contains(&cfg, &measure, past, cy),
            "{at}: past the box hits"
        );
        // The selection box is the bounds padded.
        let selection = element.selection_box(&cfg, &measure).expect("selection");
        assert_eq!(selection.rect(), bounds.expanded(SELECTION_PADDING), "{at}");
        // A drag moves the bounds and the pixels by the same amount.
        let mut moved = cfg.clone();
        let result = element.drag_by(&mut moved, &measure, 7.0, 5.0);
        assert_eq!(result, NudgeResult::Moved, "{at}");
        let moved_bounds = element.bounds(&moved, &measure).expect("bounds");
        assert!(
            (moved_bounds.x - bounds.x - 7.0).abs() < 1e-3,
            "{at}: {moved_bounds:?}"
        );
        assert!(
            (moved_bounds.y - bounds.y - 5.0).abs() < 1e-3,
            "{at}: {moved_bounds:?}"
        );
        // The chart without the formula, dragged the same way, is what the
        // moved formula's ink is told apart from.
        let mut moved_empty = empty.clone();
        element.drag_by(&mut moved_empty, &measure, 7.0, 5.0);
        let moved_base = try_raster_chart_to_rgba(&moved_empty).unwrap();
        let moved_ink = diff_box(
            &try_raster_chart_to_rgba(&moved).unwrap(),
            &moved_base,
            width,
        )
        .unwrap();
        assert!(
            (moved_ink.x - ink.x - 7.0).abs() <= 1.0,
            "{at}: {moved_ink:?} vs {ink:?}"
        );
        assert!(
            (moved_ink.y - ink.y - 5.0).abs() <= 1.0,
            "{at}: {moved_ink:?} vs {ink:?}"
        );
    }
}

#[test]
fn a_math_chart_title_draws_hits_selects_and_drags() {
    for formula in FORMULAS {
        let mut with = bare();
        with.chart_title.text = tex(formula, 18.0);
        let mut without = bare();
        without.chart_title.text.segments.clear();
        check_element("chart title", &ChartTitleElement, &with, &without, true);
    }
}

#[test]
fn math_axis_titles_draw_hits_select_and_drag_rotated_too() {
    for formula in FORMULAS {
        for side in [Side::Bottom, Side::Left] {
            let mut with = bare();
            let axis = match side {
                Side::Bottom => &mut with.bottom_x,
                _ => &mut with.left_y,
            };
            axis.title_option.visible = true;
            axis.title_option.text = tex(formula, 14.0);
            let mut without = with.clone();
            match side {
                Side::Bottom => without.bottom_x.title_option.text.segments.clear(),
                _ => without.left_y.title_option.text.segments.clear(),
            }
            let name = format!("{side:?} axis title");
            let element = AxisTitleElement { side };
            check_element(&name, &element, &with, &without, true);
        }
    }
}

#[test]
fn a_math_colorbar_title_draws_hits_selects_and_drags() {
    for formula in FORMULAS {
        let mut with = bare();
        let mut bar = renderer::default::default_colorbar_options();
        bar.axis.line_visible = false;
        bar.axis.tick = TickVisibility::None;
        bar.axis.label_style.label_visible = false;
        bar.axis.title_option.visible = true;
        bar.axis.title_option.text = tex(formula, 13.0);
        with.colorbar = Some(bar);
        with.right_y.out_margin = 200.0;
        let mut without = with.clone();
        let bar = without.colorbar.as_mut().unwrap();
        bar.axis.title_option.text.segments.clear();
        check_element(
            "colourbar title",
            &ColorBarTitleElement,
            &with,
            &without,
            true,
        );
    }
}

// Two legend labels that both number their nodes from 0 (D1) stay valid
// together; replacing and removing an entry keeps every node run whole; the
// legend's box holds the formulas' ink, hits, selects and drags.
#[test]
fn math_legend_labels_keep_their_node_runs_through_the_helpers() {
    let symbol = || vec![RichSegment::fielded('●', 2.0, Some(Color::BLACK))];
    let label = |src: &str| tex(src, 12.0).segments;
    let mut content = RichText::plain("", Color::BLACK, 12.0, "");
    append_legend_entry_rich(&mut content, symbol(), label(r"$x_i^2$"));
    append_legend_entry_rich(&mut content, symbol(), label(r"$\frac{1}{n}\sum_i y_i$"));
    let ids: Vec<u32> = content
        .segments
        .iter()
        .filter_map(|s| s.math_node.map(|n| n.id))
        .collect();
    assert_eq!(ids.iter().filter(|id| **id == 0).count(), 2, "{ids:?}");
    assert_eq!(content.validate_math(), Ok(()));
    assert_eq!(legend_entry_count(&content), 2);

    set_legend_entry_label(&mut content, 0, symbol(), label(r"$\sqrt{\alpha}$"));
    assert_eq!(content.validate_math(), Ok(()));
    assert_eq!(legend_entry_count(&content), 2);

    let mut with = bare();
    with.legend.visible = true;
    with.legend.content = content.clone();
    let mut without = with.clone();
    without.legend.content.segments = content
        .segments
        .iter()
        .filter(|s| s.math_node.is_none() && s.math_parent.is_none())
        .cloned()
        .collect();
    check_element("legend", &LegendElement, &with, &without, false);

    assert!(remove_legend_entry(&mut content, 0));
    assert_eq!(content.validate_math(), Ok(()));
    assert_eq!(legend_entry_count(&content), 1);
    assert!(content.segments.iter().any(|s| s.text == '∑'));
}

// A tall formula title grows the title band, as a multi-line title does.
#[test]
fn a_tall_formula_grows_the_title_band() {
    let mut cfg = bare();
    cfg.chart_title.top_margin = 32.0;
    cfg.chart_title.text = tex(
        r"$\dfrac{\sum\limits_{k=0}^{\infty} x^k}{\left(1 - \dfrac{1}{y}\right)}$",
        16.0,
    );
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let height = renderer::text_render::measure_rich_text(
        &cfg.chart_title.text,
        renderer::FontPolicy::Standard,
    )
    .height();
    assert!(height > 32.0, "{height}");
    assert!(renderer::layout::fit_chart_title_margin(&mut cfg, &measure));
    assert!(cfg.chart_title.top_margin >= height);
    let bounds = ChartTitleElement.bounds(&cfg, &measure).unwrap();
    assert!(bounds.y >= 0.0 && bounds.y + bounds.height <= cfg.chart_title.top_margin);
}

// The sketch draw style keeps math in the math face (D6): the formula
// part of a title draws the same pixels in both styles.
#[test]
fn a_sketch_chart_draws_math_from_the_math_face() {
    let mut precise = bare();
    precise.chart_title.text = tex(r"$\int_0^1 f(x)\,dx$", 18.0);
    let mut sketch = precise.clone();
    sketch.draw_style = renderer::config::DrawStyle::Sketch(Default::default());
    assert_eq!(
        try_raster_chart_to_rgba(&precise).unwrap(),
        try_raster_chart_to_rgba(&sketch).unwrap()
    );
}

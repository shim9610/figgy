//! Selection policy — `Selectable` and the chart elements that implement it.
//!
//! Selectable objects expose their exact pixel bounds (`bounds`); what
//! selection *does* is fixed by the trait's default methods so it applies
//! uniformly to every element: a blue highlight box around the bounds
//! (`selection_box`) and point hit-testing (`contains`). The renderer draws
//! the box with skia (`axis_render::draw_selection_boxes`); the model only
//! states the policy.
//!
//! Bounds formulas mirror the renderer's draw formulas one-to-one
//! (`axis_render::draw_chart_title` / `draw_axis_title` / axis bands), with
//! glyph extents supplied through the [`MeasureText`] contract — so the box
//! encloses exactly what is drawn.

use crate::color::Color;
use crate::config::{AxisOptions, Config, LegendCorner, TickVisibility};
use crate::drag::Draggable;
use crate::layout::{RectF, Side};
use crate::resize::Resizable;
use crate::text::MeasureText;

/// Selection highlight color (blue).
pub const SELECTION_COLOR: Color = Color {
    r: 0.13,
    g: 0.47,
    b: 0.95,
    a: 1.0,
};
/// Stroke width of the highlight box, px.
pub const SELECTION_STROKE_WIDTH: f32 = 1.5;
/// Gap between the element bounds and the highlight box, px.
pub const SELECTION_PADDING: f32 = 2.0;

/// The blue box drawn around a selected element. Produced by
/// [`Selectable::selection_box`]; consumed by the renderer's skia pass.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionBox {
    /// Box rect in chart-surface pixels (already padding-expanded).
    pub rect: RectF,
    pub color: Color,
    pub stroke_width: f32,
    /// Resize handle squares (empty for non-resizable elements). Drawn as
    /// filled squares on top of the box outline.
    pub handles: Vec<RectF>,
}

/// An object the user can select on the chart.
///
/// Implementations provide [`Self::bounds`] only. Selection behavior lives in
/// the default methods so it is identical across elements; override them only
/// to change the policy itself.
pub trait Selectable {
    /// Exact pixel bounds of this element under `cfg`, or `None` when the
    /// element is hidden, empty, or the layout is infeasible. Text-bearing
    /// elements compute glyph-precise boxes via `measure`.
    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF>;

    /// Default selection policy: a blue box [`SELECTION_PADDING`] px outside
    /// the bounds, stroked [`SELECTION_STROKE_WIDTH`] px in
    /// [`SELECTION_COLOR`]. Resizable elements additionally carry the eight
    /// resize handle squares.
    fn selection_box(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<SelectionBox> {
        let rect = self.bounds(cfg, measure)?.expanded(SELECTION_PADDING);
        let handles = match self.as_resizable() {
            Some(_) => crate::resize::handle_rects(&rect)
                .iter()
                .map(|(_, r)| *r)
                .collect(),
            None => Vec::new(),
        };
        Some(SelectionBox {
            rect,
            color: SELECTION_COLOR,
            stroke_width: SELECTION_STROKE_WIDTH,
            handles,
        })
    }

    /// Default hit test: point-in-bounds.
    fn contains(&self, cfg: &Config, measure: &dyn MeasureText, x: f32, y: f32) -> bool {
        self.bounds(cfg, measure).is_some_and(|b| b.contains(x, y))
    }

    /// Default registration: every `Selectable` can enter a [`HitMap`] the
    /// same way. Returns the id the map will report from `hit_test`.
    /// (`Send + Sync` because hosts store hit maps in cross-thread render
    /// state, e.g. egui's `CallbackResources`.)
    fn register_into(self, map: &mut HitMap) -> HitId
    where
        Self: Sized + Send + Sync + 'static,
    {
        map.register(self)
    }

    /// This element's drag capability, if any. Elements that also implement
    /// [`Draggable`] override this with `Some(self)`. Lets hosts go from a
    /// [`HitMap`] hit straight to dragging without knowing concrete types.
    fn as_draggable(&self) -> Option<&dyn Draggable> {
        None
    }

    /// This element's resize capability, if any. Elements that also implement
    /// [`Resizable`] override this with `Some(self)` — their selection box
    /// then grows the eight resize handles automatically.
    fn as_resizable(&self) -> Option<&dyn Resizable> {
        None
    }

    /// Stable element name for hosts that key on identity rather than
    /// [`HitId`] (e.g. a wasm host exposing hit-testing as strings):
    /// `"data_area"`, `"axis_bottom"`, `"tick_labels_left"`,
    /// `"axis_title_left"`, `"legend"`, `"chart_title"`. Custom host
    /// elements keep the default.
    fn element_id(&self) -> String {
        "custom".to_string()
    }
}

fn side_str(side: &Side) -> &'static str {
    match side {
        Side::Top => "top",
        Side::Bottom => "bottom",
        Side::Left => "left",
        Side::Right => "right",
    }
}

/// Id of an entry registered in a [`HitMap`]. Stable for the map's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitId(usize);

/// Registry of selectable elements for one chart panel + point hit-testing.
///
/// Hosts feed pointer events here: `hit_test` returns the topmost registered
/// element under the point (later registrations win — register background
/// elements like the data area first, small foreground elements like titles
/// last). The model owns this because hit-testing is pure bounds geometry;
/// the renderer only supplies the [`MeasureText`] implementation.
pub struct HitMap {
    entries: Vec<Box<dyn Selectable + Send + Sync>>,
}

impl HitMap {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// The standard registration set for a single chart panel, back-to-front:
    /// data area → axes → tick-label bands → axis titles → legend → chart
    /// title.
    pub fn standard_chart() -> Self {
        let mut map = Self::new();
        map.register(DataAreaElement);
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            map.register(AxisElement { side });
        }
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            map.register(AxisLabelElement { side });
        }
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            map.register(AxisTitleElement { side });
        }
        map.register(LegendElement);
        map.register(ChartTitleElement);
        map
    }

    pub fn register(&mut self, el: impl Selectable + Send + Sync + 'static) -> HitId {
        self.entries.push(Box::new(el));
        HitId(self.entries.len() - 1)
    }

    /// Topmost element containing `(x, y)` — entries are tested in reverse
    /// registration order.
    pub fn hit_test(
        &self,
        cfg: &Config,
        measure: &dyn MeasureText,
        x: f32,
        y: f32,
    ) -> Option<HitId> {
        self.entries
            .iter()
            .enumerate()
            .rev()
            .find(|(_, el)| el.contains(cfg, measure, x, y))
            .map(|(i, _)| HitId(i))
    }

    pub fn get(&self, id: HitId) -> Option<&(dyn Selectable + Send + Sync)> {
        self.entries.get(id.0).map(|b| b.as_ref())
    }

    /// Selection highlight for a registered element — the registered
    /// element's [`Selectable::selection_box`] policy.
    pub fn selection_box(
        &self,
        id: HitId,
        cfg: &Config,
        measure: &dyn MeasureText,
    ) -> Option<SelectionBox> {
        self.get(id)?.selection_box(cfg, measure)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// Selectable chart elements.

/// One of the four axes — the axis line plus its tick band.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisElement {
    pub side: Side,
}

/// One axis title (rotated 90° on Left/Right sides).
#[derive(Debug, Clone, PartialEq)]
pub struct AxisTitleElement {
    pub side: Side,
}

/// One axis's tick-value labels — the band the numeric labels live in,
/// between the tick ends and the outer chart edge.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisLabelElement {
    pub side: Side,
}

/// The chart title in the top band.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartTitleElement;

/// The data plotting area (everything inside the margins).
#[derive(Debug, Clone, PartialEq)]
pub struct DataAreaElement;

/// The legend box in one corner of the data area.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendElement;

fn axis_of<'a>(cfg: &'a Config, side: &Side) -> &'a AxisOptions {
    match side {
        Side::Top => &cfg.top_x,
        Side::Bottom => &cfg.bottom_x,
        Side::Left => &cfg.left_y,
        Side::Right => &cfg.right_y,
    }
}

impl Selectable for AxisElement {
    fn element_id(&self) -> String {
        format!("axis_{}", side_str(&self.side))
    }

    fn bounds(&self, cfg: &Config, _measure: &dyn MeasureText) -> Option<RectF> {
        let axis = axis_of(cfg, &self.side);
        if !axis.line_visible && matches!(axis.tick, TickVisibility::None) {
            return None;
        }
        let da = cfg.data_area().ok()?;

        // Tick extents on each side of the axis line. The stroke itself is at
        // least 1 px in the renderer (`stroke_paint` clamps), so mirror that.
        let t = axis.major_tick_length;
        let (inward, outward) = match axis.tick {
            TickVisibility::None => (0.0, 0.0),
            TickVisibility::Outside => (0.0, t),
            TickVisibility::Inside => (t, 0.0),
            TickVisibility::Both => (t, t),
        };
        let half_line = if axis.line_visible {
            axis.line_width.max(1.0) * 0.5
        } else {
            0.0
        };
        let in_ext = inward.max(half_line);
        let out_ext = outward.max(half_line);

        let (dax, day) = (da.x as f32, da.y as f32);
        let (daw, dah) = (da.width as f32, da.height as f32);
        let band = match self.side {
            Side::Top => RectF {
                x: dax,
                y: day - out_ext,
                width: daw,
                height: in_ext + out_ext,
            },
            Side::Bottom => RectF {
                x: dax,
                y: day + dah - in_ext,
                width: daw,
                height: in_ext + out_ext,
            },
            Side::Left => RectF {
                x: dax - out_ext,
                y: day,
                width: in_ext + out_ext,
                height: dah,
            },
            Side::Right => RectF {
                x: dax + daw - in_ext,
                y: day,
                width: in_ext + out_ext,
                height: dah,
            },
        };
        // A detached axis carries its band with it (perpendicular shift).
        Some(match self.side {
            Side::Left | Side::Right => band.translated(axis.line_offset, 0.0),
            Side::Top | Side::Bottom => band.translated(0.0, axis.line_offset),
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

/// Gap between a tick end and its label — keep in sync with the renderer's
/// `axis_render::LABEL_GAP`.
const LABEL_GAP: f32 = 4.0;

impl Selectable for AxisLabelElement {
    /// The strip the label glyphs occupy: it starts `major_tick_length +
    /// LABEL_GAP` outward of the axis line (same placement rule as the
    /// renderer's `draw_tick_label`) and is sized from the label font via
    /// `measure` — its height for horizontal axes, and a
    /// `significant_digits`-wide digit sample for the vertical axes' width.
    fn element_id(&self) -> String {
        format!("tick_labels_{}", side_str(&self.side))
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let axis = axis_of(cfg, &self.side);
        let ls = &axis.label_style;
        if !ls.visible || !ls.label_visible {
            return None;
        }
        let da = cfg.data_area().ok()?;
        let start = axis.major_tick_length + LABEL_GAP;

        // Representative label extents at the label font/size. Digits share
        // one height; width approximates a `significant_digits`-long number
        // (+2 for a sign / decimal point).
        let digits = (ls.significant_digits.max(1) as usize) + 2;
        let sample = crate::text::RichText {
            segments: crate::text::rich_segments_from_text(&"0".repeat(digits)),
            color: ls.color,
            font_size: ls.font_size,
            font: ls.label_font.clone(),
        };
        let m = measure.measure_rich(&sample);

        let (dax, day) = (da.x as f32, da.y as f32);
        let (daw, dah) = (da.width as f32, da.height as f32);
        let strip = match self.side {
            Side::Top => RectF {
                x: dax,
                y: day - start - m.height(),
                width: daw,
                height: m.height(),
            },
            Side::Bottom => RectF {
                x: dax,
                y: day + dah + start,
                width: daw,
                height: m.height(),
            },
            Side::Left => RectF {
                x: dax - start - m.width,
                y: day,
                width: m.width,
                height: dah,
            },
            Side::Right => RectF {
                x: dax + daw + start,
                y: day,
                width: m.width,
                height: dah,
            },
        };
        // Labels follow a detached axis (perpendicular shift), then translate
        // with the user's visual offset.
        let strip = match self.side {
            Side::Left | Side::Right => strip.translated(axis.line_offset, 0.0),
            Side::Top | Side::Bottom => strip.translated(0.0, axis.line_offset),
        };
        Some(strip.translated(ls.label_offset_x, ls.label_offset_y))
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for AxisTitleElement {
    fn element_id(&self) -> String {
        format!("axis_title_{}", side_str(&self.side))
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let axis = axis_of(cfg, &self.side);
        let to = &axis.title_option;
        if !to.visible || to.text.segments.is_empty() {
            return None;
        }
        let da = cfg.data_area().ok()?;
        let ca = &cfg.chart_area;
        let m = measure.measure_rich(&to.text);

        Some(match self.side {
            // Horizontal text — same formulas as `draw_axis_title`.
            Side::Top => {
                let band_top = ca.y as f32 + cfg.chart_title.top_margin;
                let baseline = band_top + (axis.out_margin - m.height()) * 0.5 + m.ascent;
                let x = da.x as f32 + da.width as f32 * 0.5 - m.width * 0.5;
                RectF {
                    x: x + to.offset_x,
                    y: baseline + to.offset_y - m.ascent,
                    width: m.width,
                    height: m.height(),
                }
            }
            Side::Bottom => {
                let band_top = (ca.y + ca.height) as f32 - axis.out_margin;
                let baseline = band_top + (axis.out_margin - m.height()) * 0.5 + m.ascent;
                let x = da.x as f32 + da.width as f32 * 0.5 - m.width * 0.5;
                RectF {
                    x: x + to.offset_x,
                    y: baseline + to.offset_y - m.ascent,
                    width: m.width,
                    height: m.height(),
                }
            }
            // Rotated text (Left −90°, Right +90°): the local-frame box is
            // centered on the rotation center; the screen box is the axis-
            // aligned image of that — width/height swap, offsets mapped with
            // the same rotation as `nudge`'s `local_to_screen_offset`.
            Side::Left => {
                let cx = ca.x as f32 + axis.out_margin * 0.5;
                let cy = da.y as f32 + da.height as f32 * 0.5;
                let (sx, sy) = (cx + to.offset_y, cy - to.offset_x);
                RectF {
                    x: sx - m.height() * 0.5,
                    y: sy - m.width * 0.5,
                    width: m.height(),
                    height: m.width,
                }
            }
            Side::Right => {
                let cx = (ca.x + ca.width) as f32 - axis.out_margin * 0.5;
                let cy = da.y as f32 + da.height as f32 * 0.5;
                let (sx, sy) = (cx - to.offset_y, cy + to.offset_x);
                RectF {
                    x: sx - m.height() * 0.5,
                    y: sy - m.width * 0.5,
                    width: m.height(),
                    height: m.width,
                }
            }
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for ChartTitleElement {
    fn element_id(&self) -> String {
        "chart_title".to_string()
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let ct = &cfg.chart_title;
        if !ct.visible || ct.text.segments.is_empty() {
            return None;
        }
        let ca = &cfg.chart_area;
        let m = measure.measure_rich(&ct.text);

        // Same formulas as `draw_chart_title`.
        let baseline = ca.y as f32 + (ct.top_margin - m.height()) * 0.5 + m.ascent;
        let x = ca.x as f32 + ca.width as f32 * 0.5 - m.width * 0.5;
        Some(RectF {
            x: x + ct.offset_x,
            y: baseline + ct.offset_y - m.ascent,
            width: m.width,
            height: m.height(),
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for DataAreaElement {
    fn element_id(&self) -> String {
        "data_area".to_string()
    }

    fn bounds(&self, cfg: &Config, _measure: &dyn MeasureText) -> Option<RectF> {
        cfg.data_area().ok().map(|da| RectF::from_rect(&da.0))
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }

    fn as_resizable(&self) -> Option<&dyn Resizable> {
        Some(self)
    }
}

impl Selectable for LegendElement {
    /// Same box formulas as the renderer's `draw_legend` (change together):
    /// the whole content document is measured as one rich text (`'\n'`
    /// segments break lines), the box is the measured envelope expanded by
    /// `padding` on every side, inset 6 px from the data-area corner.
    fn element_id(&self) -> String {
        "legend".to_string()
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let lg = &cfg.legend;
        if !lg.visible || lg.content.segments.is_empty() {
            return None;
        }
        let da = cfg.data_area().ok()?;

        let m = measure.measure_rich(&lg.content);
        let box_w = m.width + lg.padding * 2.0;
        let box_h = m.height() + lg.padding * 2.0;

        let inset = 6.0;
        let (x, y) = match lg.corner {
            LegendCorner::TopLeft => (da.x as f32 + inset, da.y as f32 + inset),
            LegendCorner::TopRight => (
                (da.x + da.width) as f32 - box_w - inset,
                da.y as f32 + inset,
            ),
            LegendCorner::BottomLeft => (
                da.x as f32 + inset,
                (da.y + da.height) as f32 - box_h - inset,
            ),
            LegendCorner::BottomRight => (
                (da.x + da.width) as f32 - box_w - inset,
                (da.y + da.height) as f32 - box_h - inset,
            ),
        };
        Some(RectF {
            x: x + lg.offset_x,
            y: y + lg.offset_y,
            width: box_w,
            height: box_h,
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;
    use crate::layout::{ChartArea, Rect};
    use crate::text::{RichText, TextExtents, rich_segments_from_text};

    /// `element_id` gives every standard element a distinct, stable name —
    /// hosts key hover/context UI on these strings.
    #[test]
    fn element_ids_are_stable_and_distinct() {
        let ids = [
            DataAreaElement.element_id(),
            AxisElement { side: Side::Bottom }.element_id(),
            AxisLabelElement { side: Side::Left }.element_id(),
            AxisTitleElement { side: Side::Left }.element_id(),
            LegendElement.element_id(),
            ChartTitleElement.element_id(),
        ];
        assert_eq!(
            ids,
            [
                "data_area",
                "axis_bottom",
                "tick_labels_left",
                "axis_title_left",
                "legend",
                "chart_title"
            ]
        );
    }

    /// Deterministic stub mirroring the renderer's line splitting: 8 px per
    /// segment on the widest line, 10 up / 3 down per line, `'\n'` segments
    /// start a new 13 px line below the first baseline.
    struct FixedMeasure;
    impl MeasureText for FixedMeasure {
        fn measure_rich(&self, rt: &RichText) -> TextExtents {
            let mut line_lens = vec![0usize];
            for seg in &rt.segments {
                if seg.text == '\n' {
                    line_lens.push(0);
                } else if seg.text != '\t' {
                    // '\t' is a column separator (zero-width here; the real
                    // engine aligns columns — irrelevant for these tests).
                    *line_lens.last_mut().unwrap() += 1;
                }
            }
            TextExtents {
                width: line_lens.iter().copied().max().unwrap_or(0) as f32 * 8.0,
                ascent: 10.0,
                descent: 3.0 + (line_lens.len() - 1) as f32 * 13.0,
            }
        }
    }

    fn cfg_800x600() -> Config {
        let mut cfg = default_config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        });
        cfg
    }

    #[test]
    fn data_area_bounds_match_layout() {
        let cfg = cfg_800x600();
        let da = cfg.data_area().unwrap();
        let b = DataAreaElement.bounds(&cfg, &FixedMeasure).unwrap();
        assert_eq!(b, RectF::from_rect(&da.0));
    }

    #[test]
    fn chart_title_bounds_centered_in_band() {
        let mut cfg = cfg_800x600();
        cfg.chart_title.text.segments = rich_segments_from_text("Title");

        let b = ChartTitleElement.bounds(&cfg, &FixedMeasure).unwrap();
        // 5 chars × 8 px = 40 wide, centered at x = 400.
        assert_eq!(b.width, 40.0);
        assert_eq!(b.height, 13.0);
        assert!((b.x - (400.0 - 20.0)).abs() < 1e-4);
        // Vertically inside the 32 px title band.
        assert!(b.y >= 0.0 && b.y + b.height <= cfg.chart_title.top_margin);
    }

    #[test]
    fn hidden_or_empty_title_has_no_bounds() {
        let mut cfg = cfg_800x600();
        // Empty text (default) → None.
        assert!(ChartTitleElement.bounds(&cfg, &FixedMeasure).is_none());
        // Filled but hidden → None.
        cfg.chart_title.text.segments = rich_segments_from_text("T");
        cfg.chart_title.visible = false;
        assert!(ChartTitleElement.bounds(&cfg, &FixedMeasure).is_none());
    }

    #[test]
    fn bottom_axis_bounds_hug_data_area_edge() {
        let cfg = cfg_800x600();
        let da = cfg.data_area().unwrap();
        let b = AxisElement { side: Side::Bottom }
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        assert_eq!(b.x, da.x as f32);
        assert_eq!(b.width, da.width as f32);
        let axis_y = (da.y + da.height) as f32;
        assert!(b.y <= axis_y && axis_y <= b.y + b.height);
    }

    #[test]
    fn left_axis_title_bounds_swap_extents() {
        let mut cfg = cfg_800x600();
        cfg.left_y.title_option.text.segments = rich_segments_from_text("Volt"); // 4 chars

        let b = AxisTitleElement { side: Side::Left }
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        // Rotated 90°: screen width = text height (13), screen height = text width (32).
        assert_eq!(b.width, 13.0);
        assert_eq!(b.height, 32.0);
    }

    #[test]
    fn selection_box_pads_bounds_with_blue() {
        let cfg = cfg_800x600();
        let b = DataAreaElement.bounds(&cfg, &FixedMeasure).unwrap();
        let sb = DataAreaElement.selection_box(&cfg, &FixedMeasure).unwrap();
        assert_eq!(sb.rect, b.expanded(SELECTION_PADDING));
        assert_eq!(sb.color, SELECTION_COLOR);
        assert_eq!(sb.stroke_width, SELECTION_STROKE_WIDTH);
    }

    #[test]
    fn contains_hits_inside_and_misses_outside() {
        let cfg = cfg_800x600();
        let b = DataAreaElement.bounds(&cfg, &FixedMeasure).unwrap();
        let (cx, cy) = (b.x + b.width * 0.5, b.y + b.height * 0.5);
        assert!(DataAreaElement.contains(&cfg, &FixedMeasure, cx, cy));
        assert!(!DataAreaElement.contains(&cfg, &FixedMeasure, b.x - 10.0, b.y - 10.0));
    }

    #[test]
    fn hitmap_topmost_wins_over_background() {
        let mut cfg = cfg_800x600();
        cfg.chart_title.text.segments = rich_segments_from_text("Title");
        let map = HitMap::standard_chart();

        // A point inside the chart title box must report the title, not the
        // (earlier-registered) bands behind it.
        let tb = ChartTitleElement.bounds(&cfg, &FixedMeasure).unwrap();
        let id = map
            .hit_test(
                &cfg,
                &FixedMeasure,
                tb.x + tb.width * 0.5,
                tb.y + tb.height * 0.5,
            )
            .unwrap();
        let sb = map.selection_box(id, &cfg, &FixedMeasure).unwrap();
        assert_eq!(sb.rect, tb.expanded(SELECTION_PADDING));

        // Center of the data area → data area element.
        let db = DataAreaElement.bounds(&cfg, &FixedMeasure).unwrap();
        let id2 = map
            .hit_test(
                &cfg,
                &FixedMeasure,
                db.x + db.width * 0.5,
                db.y + db.height * 0.5,
            )
            .unwrap();
        assert_ne!(id, id2);

        // Far outside the chart → no hit.
        assert!(map.hit_test(&cfg, &FixedMeasure, 5000.0, 5000.0).is_none());
    }

    #[test]
    fn bottom_axis_label_strip_hugs_glyph_extent() {
        let cfg = cfg_800x600();
        let da = cfg.data_area().unwrap();
        let b = AxisLabelElement { side: Side::Bottom }
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        // Starts past the tick + LABEL_GAP; thickness is the measured glyph
        // height (FixedMeasure: ascent 10 + descent 3), not the whole margin.
        assert_eq!(
            b.y,
            (da.y + da.height) as f32 + cfg.bottom_x.major_tick_length + 4.0
        );
        assert_eq!(b.height, 13.0);
        assert!(b.height < cfg.bottom_x.out_margin);
        assert_eq!(b.x, da.x as f32);
        assert_eq!(b.width, da.width as f32);
    }

    #[test]
    fn left_axis_label_strip_width_scales_with_digits() {
        let cfg = cfg_800x600();
        let da = cfg.data_area().unwrap();
        let b = AxisLabelElement { side: Side::Left }
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        // Width = (significant_digits + 2) digits × 8 px under FixedMeasure.
        let digits = cfg.left_y.label_style.significant_digits.max(1) as f32 + 2.0;
        assert_eq!(b.width, digits * 8.0);
        // Right edge sits tick + LABEL_GAP inward of the axis line.
        assert_eq!(
            b.x + b.width,
            da.x as f32 - cfg.left_y.major_tick_length - 4.0
        );
    }

    #[test]
    fn legend_drag_offset_moves_bounds() {
        use crate::legend::{LegendEntryKind, append_legend_entry, symbol_segments};
        let mut cfg = cfg_800x600();
        cfg.legend.visible = true;
        append_legend_entry(
            &mut cfg.legend.content,
            symbol_segments(&LegendEntryKind::Line, crate::color::Color::BLACK),
            "a",
        );
        let before = LegendElement.bounds(&cfg, &FixedMeasure).unwrap();

        // Drag through the trait-object route hosts use.
        let drag = LegendElement.as_draggable().expect("legend is draggable");
        assert_eq!(
            drag.drag_by(&mut cfg, 9.0, -5.0),
            crate::layout::NudgeResult::Moved
        );
        let after = LegendElement.bounds(&cfg, &FixedMeasure).unwrap();
        assert_eq!(after.x, before.x + 9.0);
        assert_eq!(after.y, before.y - 5.0);
    }

    #[test]
    fn data_area_selection_box_carries_resize_handles() {
        let cfg = cfg_800x600();
        let sb = DataAreaElement.selection_box(&cfg, &FixedMeasure).unwrap();
        assert_eq!(sb.handles.len(), 8);
        // Non-resizable elements carry none.
        let sb2 = AxisElement { side: Side::Bottom }
            .selection_box(&cfg, &FixedMeasure)
            .unwrap();
        assert!(sb2.handles.is_empty());
    }

    #[test]
    fn hidden_axis_labels_have_no_bounds() {
        let mut cfg = cfg_800x600();
        cfg.left_y.label_style.label_visible = false;
        assert!(
            AxisLabelElement { side: Side::Left }
                .bounds(&cfg, &FixedMeasure)
                .is_none()
        );
    }

    #[test]
    fn legend_bounds_mirror_renderer_box() {
        use crate::legend::{LegendCorner, LegendEntryKind, append_legend_entry, symbol_segments};

        let mut cfg = cfg_800x600();
        cfg.legend.visible = true;
        cfg.legend.corner = LegendCorner::TopRight;
        // One-line content "— abc": 5 segments × 8 px wide, 13 px tall.
        append_legend_entry(
            &mut cfg.legend.content,
            symbol_segments(&LegendEntryKind::Line, crate::color::Color::BLACK),
            "abc",
        );

        let da = cfg.data_area().unwrap();
        let lg = &cfg.legend;
        let b = LegendElement.bounds(&cfg, &FixedMeasure).unwrap();
        let box_w = 5.0 * 8.0 + lg.padding * 2.0;
        let box_h = 13.0 + lg.padding * 2.0;
        assert_eq!(b.width, box_w);
        assert_eq!(b.height, box_h);
        // One-line content: wider than tall.
        assert!(b.width > b.height);
        assert_eq!(b.x, (da.x + da.width) as f32 - box_w - 6.0);
        assert_eq!(b.y, da.y as f32 + 6.0);

        // Invisible legend → no bounds.
        let mut hidden = cfg.clone();
        hidden.legend.visible = false;
        assert!(LegendElement.bounds(&hidden, &FixedMeasure).is_none());
    }

    #[test]
    fn legend_bounds_multiline_content_grows_taller() {
        use crate::legend::{LegendEntryKind, append_legend_entry, symbol_segments};

        let mut cfg = cfg_800x600();
        cfg.legend.visible = true;
        append_legend_entry(
            &mut cfg.legend.content,
            symbol_segments(&LegendEntryKind::Line, crate::color::Color::BLACK),
            "abc",
        );
        let one_line = LegendElement.bounds(&cfg, &FixedMeasure).unwrap();

        // Second append inserts an explicit '\n' → one more 13 px line; the
        // width stays the widest line's.
        append_legend_entry(
            &mut cfg.legend.content,
            symbol_segments(&LegendEntryKind::Line, crate::color::Color::BLACK),
            "abc",
        );
        let two_lines = LegendElement.bounds(&cfg, &FixedMeasure).unwrap();
        assert_eq!(two_lines.height, one_line.height + 13.0);
        assert_eq!(two_lines.width, one_line.width);
    }

    #[test]
    fn register_into_default_method_round_trips() {
        let mut cfg = cfg_800x600();
        cfg.chart_title.text.segments = rich_segments_from_text("T");
        let mut map = HitMap::new();
        let id = ChartTitleElement.register_into(&mut map);
        assert_eq!(map.len(), 1);
        let tb = ChartTitleElement.bounds(&cfg, &FixedMeasure).unwrap();
        let hit = map
            .hit_test(&cfg, &FixedMeasure, tb.x + 1.0, tb.y + 1.0)
            .unwrap();
        assert_eq!(hit, id);
    }
}

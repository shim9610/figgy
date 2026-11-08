//! Where an energy-level series' parts are placed each draw: the arrows, the level labels stacked and kept off other columns, the transition labels, and the emit and the pick that read them (#205, #248).

use super::*;

/// Where an energy-level series is placed: the panel's anchor space (a
/// categorical `bottom_x`, the energy on `left_y`), its categories, the view
/// the series draws, the colour bar its mapped transitions read and the row
/// drawn picked. Everything here is a display parameter (rule 1).
pub struct LevelPlacement<'a> {
    pub space: &'a dyn crate::annotation::AnnotationSpace,
    pub categories: &'a [crate::config::Category],
    pub view: LevelView,
    pub colorbar: Option<&'a crate::config::ColorBarOptions>,
    pub picked: Option<PickedRow>,
}

/// The gap between a stroke's end and its label, logical pixels.
pub(super) const LABEL_GAP: f32 = 4.0;
/// The space kept between two stacked level labels, logical pixels.
pub(super) const LABEL_STACK_GAP: f32 = 1.0;
/// How far a moved label stands off its stroke's end so its leader line
/// shows, logical pixels.
pub(super) const LEADER_RUN: f32 = 10.0;
/// A run of at least this many labels in one lane whose strokes lie closer
/// than [`COLLAPSE_SPACING`] of a label's height apart collapses (#205):
/// its first and last labels stay, joined by a bracket, and the labels
/// between are not drawn. A Rydberg series near its limit is such a run.
pub(super) const COLLAPSE_COUNT: usize = 4;
pub(super) const COLLAPSE_SPACING: f32 = 0.5;
/// How far a stacked label may stand from its own stroke, in label
/// heights (#205). A stack that would move one of its labels farther is a
/// crowd, not a cluster: it collapses to its first and last labels joined
/// by a bracket, as a dense run does, and the rest is stacked again.
pub(super) const MAX_LABEL_SHIFT: f32 = 1.5;
/// The widest share of a level's slot that the arrows meeting one of its
/// ends spread over, and the most room between two of them, pixels.
pub(super) const ARROW_SPREAD: f32 = 0.6;
pub(super) const ARROW_STEP: f32 = 12.0;
/// Where along its arrow a transition label is tried, from the middle out
/// in steps of a twentieth.
pub(super) const LABEL_STOPS: [f32; 17] = [
    0.5, 0.45, 0.55, 0.4, 0.6, 0.35, 0.65, 0.3, 0.7, 0.25, 0.75, 0.2, 0.8, 0.15, 0.85, 0.1, 0.9,
];

/// Where the emit and the pick place a series' parts this draw (#205):
/// each arrow's ends spread across the levels they meet, each level label
/// stacked clear of the others in its column, and each transition label at
/// the first free spot along its arrow. Display parameters only (rule 1),
/// computed from the SSOT and the panel on every raster and pick and
/// dropped after it: the three lists are the per-raster placement scratch
/// #205 names (rule 3).
pub struct LevelLayout {
    arrows: Vec<Option<([f32; 2], [f32; 2])>>,
    /// Per label item (the levels, then the bands): its shift from its own
    /// place, `[run, dy]`; `run` is the leader's horizontal run (zero when
    /// the label did not move).
    level_shift: Vec<[f32; 2]>,
    /// Per label item: the side it is drawn on, the style's or the other
    /// one for a column whose labels would leave the plot on that side.
    level_side: Vec<LevelLabelSide>,
    /// Per level: whether it lies in a band (#205), drawn thin and without
    /// its label.
    inner: Vec<bool>,
    /// Per label item: whether a collapsed run hides it (#205).
    hidden: Vec<bool>,
    /// The collapsed runs (#205): the items of each run's first and last
    /// labels, which a bracket joins.
    brackets: Vec<(usize, usize)>,
    transition_spot: Vec<Option<([f32; 2], LevelLabelSide)>>,
    /// Per transition: the label [`Transition::drawn_label`] formatted
    /// from its wavelength this draw (#258), `None` for a host label or no
    /// label. The label text the raster builds, dropped with the layout.
    formatted: Vec<Option<RichText>>,
}

impl LevelLayout {
    /// The label drawn beside transition row `index` of `records`: the
    /// one [`Transition::drawn_label`] chose, the host's or the formatted
    /// one.
    fn transition_text<'t>(
        &'t self,
        records: &'t EnergyLevelSeries,
        index: usize,
    ) -> Option<&'t RichText> {
        transition_text(records, &self.formatted, index)
    }
}

/// The label drawn beside transition row `index`: the host's label of
/// `records` or the formatted one of `formatted` (only one is set; see
/// [`Transition::drawn_label`]).
fn transition_text<'t>(
    records: &'t EnergyLevelSeries,
    formatted: &'t [Option<RichText>],
    index: usize,
) -> Option<&'t RichText> {
    let transition = records.transitions.get(index)?;
    transition
        .label
        .as_ref()
        .or_else(|| formatted.get(index)?.as_ref())
}

/// The room a transition label keeps from the parts around it, pixels.
pub(super) const LABEL_CLEARANCE: f32 = 2.0;

/// Whether the segment `a`–`b` passes within [`LABEL_CLEARANCE`] of `rect`
/// (Liang–Barsky clipping against the grown rectangle).
pub(super) fn crosses(a: [f32; 2], b: [f32; 2], rect: crate::layout::RectF) -> bool {
    let c = LABEL_CLEARANCE;
    let (x0, y0) = (rect.x - c, rect.y - c);
    let (x1, y1) = (rect.x + rect.width + c, rect.y + rect.height + c);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [
        (-dx, a[0] - x0),
        (dx, x1 - a[0]),
        (-dy, a[1] - y0),
        (dy, y1 - a[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// Whether `rect` reaches past the plot's left or right edge.
pub(super) fn leaves_plot(plot: crate::layout::RectF, rect: crate::layout::RectF) -> bool {
    rect.x < plot.x - 0.5 || rect.x + rect.width > plot.x + plot.width + 0.5
}

/// Whether two rectangles come closer than [`LABEL_CLEARANCE`].
pub(super) fn overlaps(a: crate::layout::RectF, b: crate::layout::RectF) -> bool {
    let c = LABEL_CLEARANCE;
    a.x < b.x + b.width + c
        && b.x < a.x + a.width + c
        && a.y < b.y + b.height + c
        && b.y < a.y + a.height + c
}

impl LevelPlacement<'_> {
    fn column_x(&self, column: &str) -> Option<f64> {
        self.categories
            .iter()
            .position(|c| c.id == column)
            .map(|i| i as f64)
    }

    fn px(&self, axis: AnnotationAxis, value: f64) -> Option<f32> {
        self.space.value_to_px(axis, value)
    }

    /// The stroke of level row `index` of `records`, left end then right
    /// end, or `None` when the view does not draw it, its column is not on
    /// the axis or it falls outside the axes' ranges.
    pub fn level_stroke(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        index: usize,
    ) -> Option<([f32; 2], [f32; 2])> {
        let level = records.levels.get(index)?;
        if !level.role.drawn_in(self.view) {
            return None;
        }
        let x = self.column_x(&level.column)?;
        let half = f64::from(style.level.slot_fraction) / 2.0;
        let y = self.px(AnnotationAxis::Y, level.energy)?;
        let x0 = self.px(AnnotationAxis::X, x - half)?;
        let x1 = self.px(AnnotationAxis::X, x + half)?;
        Some(([x0, y], [x1, y]))
    }

    /// The level rows transition row `index` joins, lower then upper, with
    /// both drawn.
    fn transition_rows(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        index: usize,
    ) -> Option<(usize, usize)> {
        let transition = records.transitions.get(index)?;
        let row = |id: &str| {
            let row = records.levels.iter().position(|l| l.id == id)?;
            self.level_stroke(records, style, row).map(|_| row)
        };
        Some((row(&transition.lower)?, row(&transition.upper)?))
    }

    /// The arrows of every transition row, their ends spread across the
    /// levels they meet: the arrows that meet one end of a level are
    /// ordered by where their other ends lie, so they do not cross there,
    /// and spaced over [`ARROW_SPREAD`] of the slot, at most [`ARROW_STEP`]
    /// apart. A host's `x_offset` adds to the spread.
    fn arrows(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
    ) -> Vec<Option<([f32; 2], [f32; 2])>> {
        let rows: Vec<Option<(usize, usize)>> = (0..records.transitions.len())
            .map(|index| self.transition_rows(records, style, index))
            .collect();
        let centre = |row: usize| -> Option<([f32; 2], f32)> {
            let (a, b) = self.level_stroke(records, style, row)?;
            Some(([(a[0] + b[0]) / 2.0, a[1]], (b[0] - a[0]).abs()))
        };
        // Each end's place among the arrows meeting that level: a sort key
        // per (transition, end), then the rank within the level.
        let mut ends: Vec<(usize, usize, bool, f32, f32)> = Vec::new();
        for (index, pair) in rows.iter().enumerate() {
            let Some((lower, upper)) = *pair else {
                continue;
            };
            for (row, other, is_lower) in [(lower, upper, true), (upper, lower, false)] {
                if let Some((c, _)) = centre(other) {
                    ends.push((row, index, is_lower, c[0], c[1]));
                }
            }
        }
        ends.sort_by(|a, b| {
            (a.0, a.3, a.4, a.1)
                .partial_cmp(&(b.0, b.3, b.4, b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut x_of = vec![[f32::NAN; 2]; records.transitions.len()];
        let mut start = 0;
        while start < ends.len() {
            let row = ends[start].0;
            let end = ends[start..]
                .iter()
                .position(|e| e.0 != row)
                .map_or(ends.len(), |n| start + n);
            let count = end - start;
            if let Some((c, width)) = centre(row) {
                let step = if count > 1 {
                    (width * ARROW_SPREAD / (count - 1) as f32).min(ARROW_STEP)
                } else {
                    0.0
                };
                for (k, e) in ends[start..end].iter().enumerate() {
                    let x = c[0] + (k as f32 - (count - 1) as f32 / 2.0) * step;
                    x_of[e.1][usize::from(!e.2)] = x;
                }
            }
            start = end;
        }
        rows.iter()
            .enumerate()
            .map(|(index, pair)| {
                let (lower, upper) = (*pair)?;
                let transition = &records.transitions[index];
                let shift = {
                    let x = self.column_x(&records.levels[lower].column)?;
                    let a = self.px(AnnotationAxis::X, x)?;
                    let b = self.px(AnnotationAxis::X, x + f64::from(transition.x_offset))?;
                    b - a
                };
                let point = |row: usize, slot: usize| -> Option<[f32; 2]> {
                    let (c, _) = centre(row)?;
                    let x = x_of[index][slot];
                    Some([if x.is_finite() { x } else { c[0] } + shift, c[1]])
                };
                let lower = point(lower, 0)?;
                let upper = point(upper, 1)?;
                Some(match style.transition.direction {
                    TransitionDirection::Emission => (upper, lower),
                    TransitionDirection::Absorption | TransitionDirection::None => (lower, upper),
                })
            })
            .collect()
    }

    /// The fill of band `index` (#205): its column's slot, from its lowest
    /// to its highest level, half a stroke past each; `None` when either
    /// level is not drawn.
    fn band_rect(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        index: usize,
    ) -> Option<crate::layout::RectF> {
        let band = records.bands.get(index)?;
        let stroke = |id: &str| {
            let row = records.levels.iter().position(|l| l.id == id)?;
            self.level_stroke(records, style, row)
        };
        let (a0, b0) = stroke(&band.first)?;
        let (a1, _) = stroke(&band.last)?;
        let half = style.level.width / 2.0;
        let (top, bottom) = (a0[1].min(a1[1]) - half, a0[1].max(a1[1]) + half);
        Some(crate::layout::RectF {
            x: a0[0].min(b0[0]),
            y: top,
            width: (b0[0] - a0[0]).abs(),
            height: bottom - top,
        })
    }

    /// Which levels lie in a band of their column (#205).
    fn inner_levels(&self, records: &EnergyLevelSeries, style: &EnergyLevelStyle) -> Vec<bool> {
        let mut inner = vec![false; records.levels.len()];
        for (index, band) in records.bands.iter().enumerate() {
            let Some(rect) = self.band_rect(records, style, index) else {
                continue;
            };
            for (row, level) in records.levels.iter().enumerate() {
                if level.column != band.column {
                    continue;
                }
                if let Some((a, _)) = self.level_stroke(records, style, row)
                    && a[1] >= rect.y - 0.01
                    && a[1] <= rect.y + rect.height + 0.01
                {
                    inner[row] = true;
                }
            }
        }
        inner
    }

    /// The column label item `item` (a level, then the bands) lies in.
    fn item_column<'r>(records: &'r EnergyLevelSeries, item: usize) -> Option<&'r str> {
        let levels = records.levels.len();
        if item < levels {
            Some(records.levels[item].column.as_str())
        } else {
            records.bands.get(item - levels).map(|b| b.column.as_str())
        }
    }

    /// The ends a label item stands beside: a level's stroke, or the
    /// middle of a band's fill across its slot (its top edge for a label
    /// above it).
    fn item_ends(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        item: usize,
        side: LevelLabelSide,
    ) -> Option<([f32; 2], [f32; 2])> {
        let levels = records.levels.len();
        if item < levels {
            return self.level_stroke(records, style, item);
        }
        let rect = self.band_rect(records, style, item - levels)?;
        let y = match side {
            LevelLabelSide::Above => rect.y,
            _ => rect.y + rect.height / 2.0,
        };
        Some(([rect.x, y], [rect.x + rect.width, y]))
    }

    /// The layout of `records` in this placement: see [`LevelLayout`].
    pub fn layout(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        measure: &dyn crate::text::MeasureText,
    ) -> LevelLayout {
        let arrows = self.arrows(records, style);
        // The labels formatted from the wavelengths (#258): the host's
        // labels are borrowed, so only these are built, and nothing is
        // built when the style asks for none.
        let unit = records.wavelength_unit.as_deref();
        let formatted: Vec<Option<RichText>> = match &style.wavelength_labels {
            None => Vec::new(),
            Some(labels) => records
                .transitions
                .iter()
                .map(|t| match t.drawn_label(Some(labels), unit) {
                    Some(std::borrow::Cow::Owned(text)) => Some(text),
                    _ => None,
                })
                .collect(),
        };
        let plot = self.space.plot_rect();
        let inner = self.inner_levels(records, style);
        // Label items: the levels, then the bands.
        let items = records.levels.len() + records.bands.len();
        let flip = match style.label_side {
            LevelLabelSide::Right => Some(LevelLabelSide::Left),
            LevelLabelSide::Left => Some(LevelLabelSide::Right),
            LevelLabelSide::Above => None,
        };
        let mut level_side = self.label_sides(records, style, &inner, measure, plot, flip);
        let mut level_shift = vec![[0.0f32; 2]; items];
        let mut hidden = vec![false; items];
        let mut brackets: Vec<(usize, usize)> = Vec::new();
        // Per item, its label on each side (right, left, above),
        // unshifted, measured the first time it is asked for: a shift
        // only translates it.
        let unshifted: Vec<[std::cell::OnceCell<Option<crate::layout::RectF>>; 3]> =
            (0..items).map(|_| Default::default()).collect();
        // Per item, an item of the stacked group or collapsed run it moves
        // with (#248): the group's first label, or the first end of the
        // bracket that hid it.
        let mut root: Vec<usize> = (0..items).collect();
        self.stack_level_labels(
            records,
            style,
            &inner,
            measure,
            plot,
            &level_side,
            &unshifted,
            &mut level_shift,
            &mut hidden,
            &mut brackets,
            &mut root,
        );
        let obstacles = self.level_obstacles(records, style);
        let rect_of = |item: usize, shift: [f32; 2], side: LevelLabelSide| {
            let (slot, run) = match side {
                LevelLabelSide::Right => (0, shift[0]),
                LevelLabelSide::Left => (1, -shift[0]),
                LevelLabelSide::Above => (2, 0.0),
            };
            unshifted[item][slot]
                .get_or_init(|| {
                    self.label_on(records, style, &inner, item, measure, [0.0; 2], side)
                        .map(|t| t.rect())
                })
                .map(|r| crate::layout::RectF {
                    x: r.x + run,
                    y: r.y + shift[1],
                    ..r
                })
        };
        // Labels that lie on a stroke, fill or limit line or meet another
        // label move (#248): a stacked group or collapsed run as one unit, or one of
        // its labels alone, to its column's other side, above its stroke,
        // or up or down, wherever it overlaps least within the plot. A
        // label beside its own column never reaches that column's strokes
        // or fill, so they count only above it. A unit with a label the
        // host moved keeps its place.
        if flip.is_some() {
            self.avoid_overlaps(
                records,
                plot,
                &obstacles,
                &hidden,
                &brackets,
                &rect_of,
                &mut root,
                &mut level_shift,
                &mut level_side,
            );
        }
        let transition_spot = self.place_transition_labels(
            records,
            &formatted,
            measure,
            &arrows,
            obstacles,
            &hidden,
            &level_shift,
            &level_side,
            &rect_of,
        );
        LevelLayout {
            arrows,
            level_shift,
            level_side,
            inner,
            hidden,
            brackets,
            transition_spot,
            formatted,
        }
    }

    /// The side each label item (the levels, then the bands) is drawn on:
    /// the style's, or `flip` for a column whose labels would leave the plot
    /// on the style's side.
    fn label_sides(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        inner: &[bool],
        measure: &dyn crate::text::MeasureText,
        plot: crate::layout::RectF,
        flip: Option<LevelLabelSide>,
    ) -> Vec<LevelLabelSide> {
        let items = records.levels.len() + records.bands.len();
        let mut level_side = vec![style.label_side; items];
        if let Some(other) = flip {
            let mut flipped: Vec<&str> = Vec::new();
            for index in 0..items {
                let Some(column) = Self::item_column(records, index) else {
                    continue;
                };
                if flipped.contains(&column) {
                    continue;
                }
                let leaves = self
                    .label_on(
                        records,
                        style,
                        inner,
                        index,
                        measure,
                        [0.0; 2],
                        style.label_side,
                    )
                    .is_some_and(|t| leaves_plot(plot, t.rect()));
                if leaves {
                    flipped.push(column);
                }
            }
            for (index, side) in level_side.iter_mut().enumerate() {
                if Self::item_column(records, index).is_some_and(|c| flipped.contains(&c)) {
                    *side = other;
                }
            }
        }
        level_side
    }

    /// Stacks the level labels per lane so no two overlap, collapsing the
    /// dense runs and the crowds to their ends joined by a bracket, and
    /// resolves each item to the root of the unit it moves with (#205,
    /// #248). Writes `level_shift`, `hidden`, `brackets` and `root`.
    #[allow(clippy::too_many_arguments)]
    fn stack_level_labels(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        inner: &[bool],
        measure: &dyn crate::text::MeasureText,
        plot: crate::layout::RectF,
        level_side: &[LevelLabelSide],
        unshifted: &[[std::cell::OnceCell<Option<crate::layout::RectF>>; 3]],
        level_shift: &mut [[f32; 2]],
        hidden: &mut [bool],
        brackets: &mut Vec<(usize, usize)>,
        root: &mut [usize],
    ) {
        let items = records.levels.len() + records.bands.len();
        // Level labels: per lane, in order of their strokes, stacked so no
        // two overlap. A lane is the gap a label is drawn in: right of its
        // column, left of it (the lane right of the column before), or above
        // it. A run of overlapping labels becomes one group centred on the
        // mean of their own places, and a group that then meets the one
        // above it merges with it.
        let lane = |index: usize| -> (u8, i64) {
            let column = Self::item_column(records, index)
                .and_then(|c| self.column_x(c))
                .map_or(i64::MIN, |x| x as i64);
            match level_side[index] {
                LevelLabelSide::Right => (0, column),
                LevelLabelSide::Left => (0, column - 1),
                LevelLabelSide::Above => (1, column),
            }
        };
        let mut placed: Vec<(usize, f32, f32)> = Vec::new();
        for index in 0..items {
            let Some(text) = self.label_at(
                records,
                style,
                inner,
                index,
                measure,
                [0.0; 2],
                level_side[index],
            ) else {
                continue;
            };
            let rect = text.rect();
            let slot = match level_side[index] {
                LevelLabelSide::Right => 0,
                LevelLabelSide::Left => 1,
                LevelLabelSide::Above => 2,
            };
            unshifted[index][slot].get_or_init(|| Some(rect));
            placed.push((index, rect.y + rect.height / 2.0, rect.height));
        }
        placed.sort_by(|a, b| {
            lane(a.0)
                .cmp(&lane(b.0))
                .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut start = 0;
        while start < placed.len() {
            let key = lane(placed[start].0);
            let end = placed[start..]
                .iter()
                .position(|p| lane(p.0) != key)
                .map_or(placed.len(), |n| start + n);
            // Collapse the dense runs first: consecutive labels whose strokes
            // lie closer than half a label apart, at least four of them.
            let mut kept: Vec<(usize, f32, f32)> = Vec::new();
            let lane_run = &placed[start..end];
            let mut k = 0;
            while k < lane_run.len() {
                let mut j = k;
                while j + 1 < lane_run.len()
                    && (lane_run[j + 1].1 - lane_run[j].1).abs()
                        < COLLAPSE_SPACING * lane_run[j].2.max(lane_run[j + 1].2)
                {
                    j += 1;
                }
                if j + 1 - k >= COLLAPSE_COUNT {
                    for m in &lane_run[k + 1..j] {
                        hidden[m.0] = true;
                        root[m.0] = lane_run[k].0;
                    }
                    kept.push(lane_run[k]);
                    kept.push(lane_run[j]);
                    brackets.push((lane_run[k].0, lane_run[j].0));
                } else {
                    kept.extend_from_slice(&lane_run[k..=j]);
                }
                k = j + 1;
            }
            // Groups as (first, last, top), in screen order.
            let stack = |run: &[(usize, f32, f32)]| {
                let mut groups: Vec<(usize, usize, f32)> = Vec::new();
                for (k, &(_, centre, height)) in run.iter().enumerate() {
                    groups.push((k, k, centre - height / 2.0));
                    while groups.len() > 1 {
                        let (b0, b1, _) = groups[groups.len() - 1];
                        let (a0, a1, a_top) = groups[groups.len() - 2];
                        let a_height: f32 =
                            run[a0..=a1].iter().map(|p| p.2 + LABEL_STACK_GAP).sum();
                        if a_top + a_height <= groups[groups.len() - 1].2 {
                            break;
                        }
                        let members = &run[a0..=b1];
                        let total: f32 = members.iter().map(|p| p.2 + LABEL_STACK_GAP).sum();
                        let mean = members.iter().map(|p| p.1).sum::<f32>() / members.len() as f32;
                        let top = (mean - total / 2.0)
                            .min(plot.y + plot.height - total)
                            .max(plot.y);
                        groups.pop();
                        groups.pop();
                        groups.push((a0, b1.max(b0), top));
                    }
                }
                groups
            };
            // The farthest a group moves one of its labels, in its tallest
            // label's heights.
            let reach = |members: &[(usize, f32, f32)], top: f32| {
                let tallest = members.iter().map(|p| p.2).fold(0.0f32, f32::max).max(1.0);
                let mut y = top;
                let mut far = 0.0f32;
                for &(_, centre, height) in members {
                    far = far.max((y + height / 2.0 - centre).abs());
                    y += height + LABEL_STACK_GAP;
                }
                far / tallest
            };
            // A crowd collapses to its ends; the rest is stacked again,
            // until every stack keeps its labels near their strokes. Each
            // pass hides at least one label, so the walk ends.
            let mut groups = stack(&kept);
            loop {
                let mut collapsed = false;
                let mut next: Vec<(usize, f32, f32)> = Vec::with_capacity(kept.len());
                for &(first, last, top) in &groups {
                    let members = &kept[first..=last];
                    if members.len() <= 2 || reach(members, top) <= MAX_LABEL_SHIFT {
                        next.extend_from_slice(members);
                        continue;
                    }
                    // The crowd's dense runs (strokes closer than a label
                    // apart, three or more) collapse first, so a label
                    // with room of its own keeps it; a crowd without one
                    // collapses whole.
                    let close = |a: &(usize, f32, f32), b: &(usize, f32, f32)| {
                        (b.1 - a.1).abs() < a.2.max(b.2)
                    };
                    let mut runs = 0;
                    let mut k = 0;
                    while k < members.len() {
                        let mut j = k;
                        while j + 1 < members.len() && close(&members[j], &members[j + 1]) {
                            j += 1;
                        }
                        if j >= k + 2 {
                            for m in &members[k + 1..j] {
                                hidden[m.0] = true;
                                root[m.0] = members[k].0;
                            }
                            next.push(members[k]);
                            next.push(members[j]);
                            brackets.push((members[k].0, members[j].0));
                            runs += 1;
                        } else {
                            next.extend_from_slice(&members[k..=j]);
                        }
                        k = j + 1;
                    }
                    if runs == 0 {
                        // Drop what this group pushed and collapse it whole.
                        next.truncate(next.len() - members.len());
                        for m in &members[1..members.len() - 1] {
                            hidden[m.0] = true;
                            root[m.0] = members[0].0;
                        }
                        next.push(members[0]);
                        next.push(members[members.len() - 1]);
                        brackets.push((members[0].0, members[members.len() - 1].0));
                    }
                    collapsed = true;
                }
                if !collapsed {
                    break;
                }
                kept = next;
                groups = stack(&kept);
            }
            let run = &kept[..];
            for (first, last, top) in groups {
                let members = &run[first..=last];
                let moved = last > first;
                let mut y = top;
                for &(index, centre, height) in members {
                    // A group can hold labels of the two columns either
                    // side of its gap; each column's part is a unit.
                    let column = Self::item_column(records, index);
                    root[index] = members
                        .iter()
                        .find(|m| Self::item_column(records, m.0) == column)
                        .map_or(index, |m| m.0);
                    let new_centre = y + height / 2.0;
                    level_shift[index] =
                        [if moved { LEADER_RUN } else { 0.0 }, new_centre - centre];
                    y += height + LABEL_STACK_GAP;
                }
            }
            start = end;
        }
        // A bracket whose end a later collapse hid is part of the
        // larger bracket now.
        brackets.retain(|&(a, b)| !hidden[a] && !hidden[b]);
        brackets.sort_unstable();
        brackets.dedup();
        // Resolve each item to the root of its unit: a hidden end
        // points at the end that hid it, which belongs to a group.
        for item in 0..items {
            let mut r = root[item];
            let mut steps = 0;
            while root[r] != r && steps < items {
                r = root[r];
                steps += 1;
            }
            root[item] = r;
        }
    }

    /// The level strokes and band fills, sorted by their tops: the obstacles
    /// a level label keeps clear of (#248) and, with the level labels added,
    /// a transition label.
    fn level_obstacles(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
    ) -> Vec<crate::layout::RectF> {
        let mut obstacles: Vec<crate::layout::RectF> = Vec::new();
        for index in 0..records.levels.len() {
            if let Some((a, b)) = self.level_stroke(records, style, index) {
                let half = style.level.width / 2.0 + 1.0;
                obstacles.push(crate::layout::RectF {
                    x: a[0].min(b[0]),
                    y: a[1] - half,
                    width: (b[0] - a[0]).abs(),
                    height: half * 2.0,
                });
            }
        }
        for index in 0..records.bands.len() {
            if let Some(rect) = self.band_rect(records, style, index) {
                obstacles.push(rect);
            }
        }
        obstacles.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
        obstacles
    }

    /// Moves the labels that lie on a stroke, fill or limit line or meet
    /// another label (#248); see [`Self::layout`]. Writes `root`,
    /// `level_shift` and `level_side`.
    #[allow(clippy::too_many_arguments)]
    fn avoid_overlaps(
        &self,
        records: &EnergyLevelSeries,
        plot: crate::layout::RectF,
        obstacles: &[crate::layout::RectF],
        hidden: &[bool],
        brackets: &[(usize, usize)],
        rect_of: &dyn Fn(usize, [f32; 2], LevelLabelSide) -> Option<crate::layout::RectF>,
        root: &mut [usize],
        level_shift: &mut [[f32; 2]],
        level_side: &mut [LevelLabelSide],
    ) {
        let items = records.levels.len() + records.bands.len();
        let tallest = obstacles.iter().map(|o| o.height).fold(0.0f32, f32::max);
        // The strokes, fills and limit lines a rect overlaps.
        let hits = |rect: crate::layout::RectF| {
            let from = obstacles.partition_point(|o| o.y < rect.y - tallest - LABEL_CLEARANCE);
            let to = obstacles.partition_point(|o| o.y <= rect.y + rect.height + LABEL_CLEARANCE);
            let limits = (0..records.limits.len())
                .filter_map(|index| self.limit_line(records, index))
                .filter(|(a, _)| a[1] > rect.y && a[1] < rect.y + rect.height)
                .count();
            obstacles[from..to.max(from)]
                .iter()
                .filter(|o| overlaps(**o, rect))
                .count()
                + limits
        };
        let inside = |rect: crate::layout::RectF| {
            !leaves_plot(plot, rect)
                && rect.y >= plot.y - 0.5
                && rect.y + rect.height <= plot.y + plot.height + 0.5
        };
        let host_moved = |item: usize| {
            let levels = records.levels.len();
            let offset = if item < levels {
                records.levels[item].label_offset
            } else {
                records
                    .bands
                    .get(item - levels)
                    .map_or([0.0; 2], |b| b.label_offset)
            };
            offset != [0.0; 2]
        };
        // Per item: its drawn rect; and the drawn items sorted by their
        // tops, so a label meets only the ones whose rows reach it.
        let mut rects: Vec<Option<crate::layout::RectF>> = (0..items)
            .map(|item| {
                (!hidden[item])
                    .then(|| rect_of(item, level_shift[item], level_side[item]))
                    .flatten()
            })
            .collect();
        let mut by_top: Vec<usize> = (0..items).filter(|&i| rects[i].is_some()).collect();
        let top = |rects: &[Option<crate::layout::RectF>], i: usize| {
            rects[i].map_or(f32::NEG_INFINITY, |r| r.y)
        };
        by_top.sort_by(|&a, &b| {
            top(&rects, a)
                .partial_cmp(&top(&rects, b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let label_height = rects
            .iter()
            .flatten()
            .map(|r| r.height)
            .fold(0.0f32, f32::max);
        // Whether two labels share ink: stacked labels stand a gap
        // apart and never meet.
        let meets = |a: crate::layout::RectF, b: crate::layout::RectF| {
            a.x < b.x + b.width
                && b.x < a.x + a.width
                && a.y < b.y + b.height
                && b.y < a.y + a.height
        };
        // The overlaps of `members`, the labels `moving` names, when
        // drawn at `place`: the strokes and fills they lie on and the other
        // labels they meet; `None` when one would leave the plot.
        let cost_of = |members: &[usize],
                       moving: &dyn Fn(usize) -> bool,
                       rects: &[Option<crate::layout::RectF>],
                       by_top: &[usize],
                       place: &dyn Fn(usize) -> Option<crate::layout::RectF>|
         -> Option<usize> {
            let mut cost = 0;
            for &item in members.iter().filter(|&&i| !hidden[i]) {
                let Some(rect) = place(item) else {
                    continue;
                };
                if !inside(rect) {
                    return None;
                }
                cost += hits(rect);
                let from = by_top.partition_point(|&o| top(rects, o) < rect.y - label_height);
                let to = by_top.partition_point(|&o| top(rects, o) < rect.y + rect.height);
                cost += by_top[from..to.max(from)]
                    .iter()
                    .filter(|&&other| !moving(other))
                    .filter_map(|&other| rects[other])
                    .filter(|other| meets(*other, rect))
                    .count();
            }
            Some(cost)
        };
        // A label that ends a bracket stays with its run.
        let bracket_end = |item: usize| brackets.iter().any(|&(a, b)| a == item || b == item);
        // The vertical moves a lone label may take, in quarters of a
        // label height; a stack of several moves by at most
        // `STACK_NUDGE` heights, so its leaders stay short.
        const STACK_NUDGE: f32 = 1.0;
        const NUDGES: [f32; 17] = [
            0.0, -0.25, 0.25, -0.5, 0.5, -0.75, 0.75, -1.0, 1.0, -1.25, 1.25, -1.5, 1.5, -2.0, 2.0,
            -2.5, 2.5,
        ];
        // Sweeps over the units: each takes its first place of least
        // overlap (a unit, or one label of a stack taken out of it
        // alone) as soon as one overlaps less, until a sweep moves
        // nothing. A move lowers the overlaps of what moved, and two
        // labels that meet count for both, so the strokes and fills
        // under every label plus the pairs of labels of different
        // units that meet fall with each move: the sweeps end.
        let mut by_unit: Vec<usize> = (0..items).collect();
        loop {
            by_unit.sort_by_key(|&i| root[i]);
            let mut swept = false;
            let mut first = 0;
            while first < items {
                let key = root[by_unit[first]];
                let mut past = first + 1;
                while past < items && root[by_unit[past]] == key {
                    past += 1;
                }
                let chunk = &by_unit[first..past];
                first = past;
                // The chunk's labels still in its unit: an earlier move
                // of this sweep may have taken one out.
                let unit = root[chunk[0]];
                let mut taken = false;
                let lone_count = chunk.iter().filter(|&&i| root[i] == unit).count();
                for single in std::iter::once(None).chain(chunk.iter().copied().map(Some)) {
                    if taken {
                        break;
                    }
                    let unit = match single {
                        Some(one) => {
                            if root[one] != unit
                                || lone_count < 2
                                || hidden[one]
                                || bracket_end(one)
                                || rects[one].is_none()
                            {
                                continue;
                            }
                            unit
                        }
                        None => unit,
                    };
                    let moving = |i: usize| match single {
                        Some(one) => i == one,
                        None => root[i] == unit,
                    };
                    let in_unit = |i: &&usize| root[**i] == unit;
                    if chunk.iter().filter(in_unit).any(|&i| host_moved(i))
                        || chunk
                            .iter()
                            .filter(in_unit)
                            .any(|&i| level_side[i] == LevelLabelSide::Above)
                    {
                        break;
                    }
                    let one;
                    let moved_items: &[usize] = match single {
                        Some(item) => {
                            one = [item];
                            &one[..]
                        }
                        None => chunk,
                    };
                    let current = |item: usize| if moving(item) { rects[item] } else { None };
                    let Some(now) = cost_of(moved_items, &moving, &rects, &by_top, &current) else {
                        continue;
                    };
                    if now == 0 {
                        // A unit clear of everything has no label that
                        // is not: its stacked labels never meet.
                        if single.is_none() {
                            break;
                        }
                        continue;
                    }
                    let height = moved_items
                        .iter()
                        .filter(|&&i| moving(i))
                        .filter_map(|&i| rects[i])
                        .map(|r| r.height)
                        .fold(0.0f32, f32::max)
                        .max(1.0);
                    let side = level_side[moved_items
                        .iter()
                        .copied()
                        .find(|&i| moving(i))
                        .unwrap_or(unit)];
                    let other = match side {
                        LevelLabelSide::Right => LevelLabelSide::Left,
                        _ => LevelLabelSide::Right,
                    };
                    let place_at = |item: usize, to: LevelLabelSide, dy: f32| {
                        let shift = level_shift[item];
                        let moved = dy != 0.0 || shift[0] != 0.0;
                        let shift = match to {
                            LevelLabelSide::Above => [0.0; 2],
                            _ => [if moved { LEADER_RUN } else { 0.0 }, shift[1] + dy],
                        };
                        (shift, rect_of(item, shift, to))
                    };
                    // Nearest first: the other side at the same height,
                    // a lone label above its stroke, then each nudge on
                    // either side; a later place must overlap less.
                    let lone = lone_count == 1 || single.is_some();
                    let candidates = std::iter::once((other, 0.0))
                        .chain(lone.then_some((LevelLabelSide::Above, 0.0)))
                        .chain(
                            NUDGES[1..]
                                .iter()
                                .filter(|n| lone || n.abs() <= STACK_NUDGE)
                                .flat_map(|&n| [(side, n * height), (other, n * height)]),
                        );
                    let mut best: Option<(usize, LevelLabelSide, f32)> = None;
                    for (to, dy) in candidates {
                        let place = |item: usize| {
                            if moving(item) {
                                place_at(item, to, dy).1
                            } else {
                                None
                            }
                        };
                        if let Some(cost) = cost_of(moved_items, &moving, &rects, &by_top, &place)
                            && cost < best.map_or(now, |(c, _, _)| c)
                        {
                            best = Some((cost, to, dy));
                            if cost == 0 {
                                break;
                            }
                        }
                    }
                    let Some((_, to, dy)) = best else {
                        continue;
                    };
                    // A label taken out of its stack is a unit of its
                    // own; when it was the stack's root, the rest takes
                    // its next label.
                    if let Some(one) = single {
                        if root[one] == one
                            && let Some(&next) = chunk.iter().find(|&&i| i != one && root[i] == one)
                        {
                            for &item in chunk {
                                if item != one && root[item] == one {
                                    root[item] = next;
                                }
                            }
                        }
                        root[one] = one;
                    }
                    for &item in moved_items {
                        if single.is_none() && root[item] != unit {
                            continue;
                        }
                        let held = level_shift[item];
                        let moved = dy != 0.0 || held[0] != 0.0;
                        let shift = match to {
                            LevelLabelSide::Above => [0.0; 2],
                            _ => [if moved { LEADER_RUN } else { 0.0 }, held[1] + dy],
                        };
                        level_shift[item] = shift;
                        level_side[item] = to;
                        rects[item] = (!hidden[item]).then(|| rect_of(item, shift, to)).flatten();
                    }
                    by_top.sort_by(|&a, &b| {
                        top(&rects, a)
                            .partial_cmp(&top(&rects, b))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    swept = true;
                    taken = single.is_none();
                }
            }
            if !swept {
                break;
            }
        }
    }

    /// The spot of each transition label: the first spot along its arrow,
    /// right of it then left, clear of the level labels, the strokes, the
    /// other arrows and the labels placed before it; when none is clear, the
    /// middle. `obstacles` are the level strokes and band fills.
    #[allow(clippy::too_many_arguments)]
    fn place_transition_labels(
        &self,
        records: &EnergyLevelSeries,
        formatted: &[Option<RichText>],
        measure: &dyn crate::text::MeasureText,
        arrows: &[Option<([f32; 2], [f32; 2])>],
        mut obstacles: Vec<crate::layout::RectF>,
        hidden: &[bool],
        level_shift: &[[f32; 2]],
        level_side: &[LevelLabelSide],
        rect_of: &dyn Fn(usize, [f32; 2], LevelLabelSide) -> Option<crate::layout::RectF>,
    ) -> Vec<Option<([f32; 2], LevelLabelSide)>> {
        let items = records.levels.len() + records.bands.len();
        for index in (0..items).filter(|i| !hidden[*i]) {
            if let Some(rect) = rect_of(index, level_shift[index], level_side[index]) {
                obstacles.push(rect);
            }
        }
        // The strokes and level labels sorted by their tops, so a spot
        // tests only the ones whose rows can reach it; the transition labels
        // placed so far are a list of their own.
        obstacles.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
        let tallest = obstacles.iter().map(|o| o.height).fold(0.0f32, f32::max);
        let mut placed_labels: Vec<crate::layout::RectF> = Vec::new();
        let near = |rect: crate::layout::RectF| {
            let from = obstacles.partition_point(|o| o.y < rect.y - tallest - LABEL_CLEARANCE);
            let to = obstacles.partition_point(|o| o.y <= rect.y + rect.height + LABEL_CLEARANCE);
            &obstacles[from..to.max(from)]
        };
        let mut transition_spot = vec![None; records.transitions.len()];
        for (index, transition) in records.transitions.iter().enumerate() {
            let (Some(text), Some((tail, tip))) =
                (transition_text(records, formatted, index), arrows[index])
            else {
                continue;
            };
            let at = |t: f32| {
                [
                    tail[0] + (tip[0] - tail[0]) * t,
                    tail[1] + (tip[1] - tail[1]) * t,
                ]
            };
            let block = |t: f32, side: LevelLabelSide| {
                let p = at(t);
                Self::label_block(text, measure, p, p[0], side, transition.label_offset).rect()
            };
            // The cost of a spot: a label or stroke it overlaps weighs more
            // than every arrow it crosses together, so a crowded bundle
            // still keeps its labels apart. The first spot of least cost
            // wins; a label the host moved stays at the middle.
            let host_placed = transition.label_offset != [0.0; 2];
            let cost = |t: f32, side: LevelLabelSide| -> usize {
                let rect = block(t, side);
                let hits = near(rect).iter().filter(|o| overlaps(**o, rect)).count()
                    + placed_labels.iter().filter(|o| overlaps(**o, rect)).count();
                let reach = |a: [f32; 2], b: [f32; 2]| {
                    a[0].max(b[0]) >= rect.x - LABEL_CLEARANCE
                        && a[0].min(b[0]) <= rect.x + rect.width + LABEL_CLEARANCE
                        && a[1].max(b[1]) >= rect.y - LABEL_CLEARANCE
                        && a[1].min(b[1]) <= rect.y + rect.height + LABEL_CLEARANCE
                };
                let crossings = arrows
                    .iter()
                    .enumerate()
                    .filter(|(other, arrow)| {
                        *other != index
                            && arrow.is_some_and(|(a, b)| reach(a, b) && crosses(a, b, rect))
                    })
                    .count();
                hits * (arrows.len() + 1) + crossings
            };
            let spot = if host_placed {
                (at(0.5), LevelLabelSide::Right)
            } else {
                let mut best = (usize::MAX, 0.5, LevelLabelSide::Right);
                'search: for &t in &LABEL_STOPS {
                    for side in [LevelLabelSide::Right, LevelLabelSide::Left] {
                        let c = cost(t, side);
                        if c < best.0 {
                            best = (c, t, side);
                            if c == 0 {
                                break 'search;
                            }
                        }
                    }
                }
                (at(best.1), best.2)
            };
            let rect = {
                let p = spot.0;
                Self::label_block(text, measure, p, p[0], spot.1, transition.label_offset).rect()
            };
            placed_labels.push(rect);
            transition_spot[index] = Some(spot);
        }
        transition_spot
    }

    /// The arrow of transition row `index` in `layout`: its tail and its
    /// tip (the head end under `style.transition.direction`; for `None`,
    /// lower then upper), or `None` when either level is not drawn.
    pub fn transition_arrow(
        &self,
        layout: &LevelLayout,
        index: usize,
    ) -> Option<([f32; 2], [f32; 2])> {
        layout.arrows.get(index).copied().flatten()
    }

    /// The line of limit row `index` across the plot, or `None` outside
    /// the energy range.
    pub fn limit_line(
        &self,
        records: &EnergyLevelSeries,
        index: usize,
    ) -> Option<([f32; 2], [f32; 2])> {
        let limit = records.limits.get(index)?;
        let y = self.px(AnnotationAxis::Y, limit.energy)?;
        let plot = self.space.plot_rect();
        Some(([plot.x, y], [plot.x + plot.width, y]))
    }

    fn transition_color(&self, style: &EnergyLevelStyle, transition: &Transition) -> Color {
        let arrow = &style.transition;
        if !arrow.color_by_value {
            return arrow.color;
        }
        match (self.colorbar, transition.value) {
            (Some(bar), Some(value)) => match bar.normalized_z(value) {
                Some(t) => bar.colormap.sample(t),
                None => bar.nan_color,
            },
            _ => arrow.color,
        }
    }

    /// The text block of a label at `anchor` (a stroke end or a midpoint),
    /// placed on `side` and moved by `offset`.
    fn label_block<'t>(
        text: &'t RichText,
        measure: &dyn crate::text::MeasureText,
        anchor: [f32; 2],
        left: f32,
        side: LevelLabelSide,
        offset: [f32; 2],
    ) -> crate::sink::PlacedText<'t> {
        let extents = measure.measure_rich(text);
        let centred = anchor[1] + (extents.ascent - extents.descent) / 2.0;
        let origin = match side {
            LevelLabelSide::Right => (anchor[0] + LABEL_GAP, centred),
            LevelLabelSide::Left => (left - LABEL_GAP - extents.width, centred),
            LevelLabelSide::Above => (
                (left + anchor[0]) / 2.0 - extents.width / 2.0,
                anchor[1] - LABEL_GAP - extents.descent,
            ),
        };
        crate::sink::PlacedText {
            text: crate::sink::TextRef::Rich(text),
            origin: (origin.0 + offset[0], origin.1 + offset[1]),
            extents,
            rotation: None,
        }
    }

    /// The label of level row `index` moved by `shift` (`[run, dy]`, see
    /// [`LevelLayout`]) away from its own place: further out by the run, on
    /// its side, and down by `dy`.
    #[allow(clippy::too_many_arguments)]
    fn label_on<'t>(
        &self,
        records: &'t EnergyLevelSeries,
        style: &EnergyLevelStyle,
        inner: &[bool],
        index: usize,
        measure: &dyn crate::text::MeasureText,
        shift: [f32; 2],
        side: LevelLabelSide,
    ) -> Option<crate::sink::PlacedText<'t>> {
        let levels = records.levels.len();
        let (text, offset) = if index < levels {
            if inner.get(index).copied().unwrap_or(false) {
                return None;
            }
            let level = &records.levels[index];
            (level.label.as_ref()?, level.label_offset)
        } else {
            let band = records.bands.get(index - levels)?;
            (band.label.as_ref()?, band.label_offset)
        };
        let (a, b) = self.item_ends(records, style, index, side)?;
        // A level outside the plot (beyond the energy range) is clipped
        // away, and so is its label: it is not pulled into the plot.
        let plot = self.space.plot_rect();
        if a[1] < plot.y - 0.5 || a[1] > plot.y + plot.height + 0.5 {
            return None;
        }
        let run = match side {
            LevelLabelSide::Right => shift[0],
            LevelLabelSide::Left => -shift[0],
            LevelLabelSide::Above => 0.0,
        };
        Some(Self::label_block(
            text,
            measure,
            b,
            a[0],
            side,
            [offset[0] + run, offset[1] + shift[1]],
        ))
    }

    /// The label of item `index` (a level, then the bands) on the side
    /// `layout` gives it.
    #[allow(clippy::too_many_arguments)]
    fn label_at<'t>(
        &self,
        records: &'t EnergyLevelSeries,
        style: &EnergyLevelStyle,
        inner: &[bool],
        index: usize,
        measure: &dyn crate::text::MeasureText,
        shift: [f32; 2],
        side: LevelLabelSide,
    ) -> Option<crate::sink::PlacedText<'t>> {
        self.label_on(records, style, inner, index, measure, shift, side)
    }

    /// The leader line from level row `index`'s stroke to its label when
    /// the layout moved the label, or `None`.
    fn leader(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        layout: &LevelLayout,
        index: usize,
        measure: &dyn crate::text::MeasureText,
    ) -> Option<([f32; 2], [f32; 2])> {
        let shift = *layout.level_shift.get(index)?;
        if shift[0] == 0.0 && shift[1].abs() < 0.5 {
            return None;
        }
        let side = *layout.level_side.get(index)?;
        let (a, b) = self.item_ends(records, style, index, side)?;
        let rect = self
            .label_at(records, style, &layout.inner, index, measure, shift, side)?
            .rect();
        let mid = rect.y + rect.height / 2.0;
        Some(match side {
            LevelLabelSide::Right => ([b[0] + 1.0, b[1]], [rect.x - 1.0, mid]),
            LevelLabelSide::Left => ([a[0] - 1.0, a[1]], [rect.x + rect.width + 1.0, mid]),
            LevelLabelSide::Above => (
                [(a[0] + b[0]) / 2.0, a[1] - 1.0],
                [rect.x + rect.width / 2.0, rect.y + rect.height + 1.0],
            ),
        })
    }

    /// The placed label of row `index` of `part` in `layout`, or `None`
    /// when the row has no label or is not drawn.
    pub fn label<'t>(
        &self,
        records: &'t EnergyLevelSeries,
        style: &EnergyLevelStyle,
        layout: &'t LevelLayout,
        part: LabelPart,
        index: usize,
        measure: &dyn crate::text::MeasureText,
    ) -> Option<crate::sink::PlacedText<'t>> {
        match part {
            LabelPart::Level | LabelPart::Band => {
                let item = match part {
                    LabelPart::Band => records.levels.len() + index,
                    _ => {
                        if index >= records.levels.len() {
                            return None;
                        }
                        index
                    }
                };
                if layout.hidden.get(item).copied().unwrap_or(false) {
                    return None;
                }
                let shift = layout.level_shift.get(item).copied().unwrap_or([0.0; 2]);
                let side = layout
                    .level_side
                    .get(item)
                    .copied()
                    .unwrap_or(style.label_side);
                self.label_at(records, style, &layout.inner, item, measure, shift, side)
            }
            LabelPart::Transition => {
                let transition = records.transitions.get(index)?;
                let (spot, side) = layout.transition_spot.get(index).copied().flatten()?;
                Some(Self::label_block(
                    layout.transition_text(records, index)?,
                    measure,
                    spot,
                    spot[0],
                    side,
                    transition.label_offset,
                ))
            }
            LabelPart::Limit => {
                let limit = records.limits.get(index)?;
                let (_, end) = self.limit_line(records, index)?;
                let text = limit.label.as_ref()?;
                let extents = measure.measure_rich(text);
                let origin = (
                    end[0] - LABEL_GAP - extents.width + limit.label_offset[0],
                    end[1] - LABEL_GAP - extents.descent + limit.label_offset[1],
                );
                Some(crate::sink::PlacedText {
                    text: crate::sink::TextRef::Rich(text),
                    origin,
                    extents,
                    rotation: None,
                })
            }
        }
    }

    /// Emit every drawn part of `records` in the look of `style` into
    /// `sink`, in the [`crate::sink::Slot::Above`] slot: the limit lines,
    /// the level strokes, the transition arrows over them, then every
    /// label. The picked row is drawn `style.picked_width_scale` times as
    /// wide. Nothing is stored; the caller's sink paints, bounds or hit
    /// tests the parts.
    pub fn emit(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        measure: &dyn crate::text::MeasureText,
        sink: &mut dyn crate::sink::Sink,
    ) {
        use crate::sink::{Slot, StrokeStyle};
        let slot = Slot::Above;
        let layout = self.layout(records, style, measure);
        for index in 0..records.limits.len() {
            if let Some((a, b)) = self.limit_line(records, index) {
                let line = &style.limit;
                sink.stroke(
                    slot,
                    a,
                    b,
                    &StrokeStyle {
                        width: line.width,
                        color: line.color,
                        dash: line.style.pattern(),
                        tag: "energy_limit",
                    },
                );
            }
        }
        let widen = |picked: bool, width: f32| {
            if picked {
                width * style.picked_width_scale
            } else {
                width
            }
        };
        // The bands under their levels (#205).
        for index in 0..records.bands.len() {
            if let Some(rect) = self.band_rect(records, style, index) {
                sink.rect(slot, rect, Some(style.band.fill), None);
            }
        }
        for index in 0..records.levels.len() {
            if let Some((a, b)) = self.level_stroke(records, style, index) {
                let picked = self.picked == Some(PickedRow::Level(index));
                let width = if layout.inner[index] {
                    style.band.inner_width
                } else {
                    style.level.width
                };
                sink.stroke(
                    slot,
                    a,
                    b,
                    &StrokeStyle {
                        width: widen(picked, width),
                        color: style.level.color,
                        dash: &[],
                        tag: "energy_level",
                    },
                );
            }
        }
        let arrow = &style.transition;
        for (index, transition) in records.transitions.iter().enumerate() {
            let Some((tail, tip)) = self.transition_arrow(&layout, index) else {
                continue;
            };
            let picked = self.picked == Some(PickedRow::Transition(index));
            let color = self.transition_color(style, transition);
            let width = widen(picked, arrow.width);
            let (dx, dy) = (tip[0] - tail[0], tip[1] - tail[1]);
            let length = dx.hypot(dy);
            let head = if arrow.direction == TransitionDirection::None {
                0.0
            } else {
                arrow.head_length.min(length)
            };
            let (ux, uy) = if length > 0.0 {
                (dx / length, dy / length)
            } else {
                (0.0, 0.0)
            };
            let base = [tip[0] - ux * head, tip[1] - uy * head];
            sink.stroke(
                slot,
                tail,
                if head > 0.0 { base } else { tip },
                &StrokeStyle {
                    width,
                    color,
                    dash: &[],
                    tag: "energy_transition",
                },
            );
            if head > 0.0 {
                let half = widen(picked, arrow.head_width) / 2.0;
                let (nx, ny) = (-uy * half, ux * half);
                sink.triangle(
                    slot,
                    [
                        tip,
                        [base[0] + nx, base[1] + ny],
                        [base[0] - nx, base[1] - ny],
                    ],
                    color,
                );
            }
        }
        for index in 0..records.levels.len() + records.bands.len() {
            if let Some((a, b)) = self.leader(records, style, &layout, index, measure) {
                sink.stroke(
                    slot,
                    a,
                    b,
                    &StrokeStyle {
                        width: 0.6,
                        color: style.level.color,
                        dash: &[],
                        tag: "energy_leader",
                    },
                );
            }
        }
        // A collapsed run's bracket: from its first stroke to its last,
        // just past the strokes' ends on the labels' side, with a tick to
        // each end.
        for &(lo, hi) in &layout.brackets {
            let side = layout.level_side[lo];
            let ends = |item: usize| self.item_ends(records, style, item, side);
            let (Some((a0, b0)), Some((a1, _))) = (ends(lo), ends(hi)) else {
                continue;
            };
            let (x, tick) = match side {
                LevelLabelSide::Left => (a0[0] - LABEL_GAP / 2.0, LABEL_GAP / 2.0),
                _ => (b0[0] + LABEL_GAP / 2.0, -LABEL_GAP / 2.0),
            };
            let line = StrokeStyle {
                width: 0.8,
                color: style.level.color,
                dash: &[],
                tag: "energy_bracket",
            };
            sink.stroke(slot, [x, a0[1]], [x, a1[1]], &line);
            sink.stroke(slot, [x, a0[1]], [x + tick, a0[1]], &line);
            sink.stroke(slot, [x, a1[1]], [x + tick, a1[1]], &line);
        }
        for part in [
            LabelPart::Level,
            LabelPart::Band,
            LabelPart::Transition,
            LabelPart::Limit,
        ] {
            for index in 0..records.part_len(part) {
                if let Some(text) = self.label(records, style, &layout, part, index, measure) {
                    sink.text(slot, &text);
                }
            }
        }
    }
}

/// What a pick of an energy-level series hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelHit {
    Level(usize),
    Transition(usize),
    Label(LabelPart, usize),
}

/// The distance from `p` to the segment `a`–`b`.
pub(super) fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))
}

/// The distance from `p` to `rect`, zero inside it.
pub(super) fn rect_distance(p: [f32; 2], rect: crate::layout::RectF) -> f32 {
    let dx = (rect.x - p[0]).max(p[0] - (rect.x + rect.width)).max(0.0);
    let dy = (rect.y - p[1]).max(p[1] - (rect.y + rect.height)).max(0.0);
    dx.hypot(dy)
}

impl LevelPlacement<'_> {
    /// The drawn part of `records` nearest `cursor`, within `max_distance`
    /// of its edge, and that distance (zero on the part). Walks the SSOT
    /// through the same placement the emit draws with, so the pick is the
    /// drawn geometry; nothing is registered per row (rule 3). On a tie a
    /// part drawn later wins: labels over arrows over strokes.
    pub fn pick(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        measure: &dyn crate::text::MeasureText,
        cursor: [f32; 2],
        max_distance: f32,
    ) -> Option<(LevelHit, f32)> {
        let layout = self.layout(records, style, measure);
        let mut best: Option<(LevelHit, f32)> = None;
        let mut offer = |hit: LevelHit, distance: f32| {
            if distance <= max_distance && best.is_none_or(|(_, d)| distance <= d) {
                best = Some((hit, distance));
            }
        };
        // A band answers for its first level anywhere on its fill; the
        // levels inside it, offered after, win on their strokes.
        for (index, band) in records.bands.iter().enumerate() {
            if let (Some(rect), Some(row)) = (
                self.band_rect(records, style, index),
                records.levels.iter().position(|l| l.id == band.first),
            ) {
                offer(LevelHit::Level(row), rect_distance(cursor, rect));
            }
        }
        for index in 0..records.levels.len() {
            if let Some((a, b)) = self.level_stroke(records, style, index) {
                let width = if layout.inner[index] {
                    style.band.inner_width
                } else {
                    style.level.width
                };
                let edge = segment_distance(cursor, a, b) - width / 2.0;
                offer(LevelHit::Level(index), edge.max(0.0));
            }
        }
        for index in 0..records.transitions.len() {
            if let Some((tail, tip)) = self.transition_arrow(&layout, index) {
                let half = style.transition.width.max(style.transition.head_width) / 2.0;
                let edge = segment_distance(cursor, tail, tip) - half;
                offer(LevelHit::Transition(index), edge.max(0.0));
            }
        }
        for part in [
            LabelPart::Level,
            LabelPart::Band,
            LabelPart::Transition,
            LabelPart::Limit,
        ] {
            for index in 0..records.part_len(part) {
                if let Some(text) = self.label(records, style, &layout, part, index, measure) {
                    offer(
                        LevelHit::Label(part, index),
                        rect_distance(cursor, text.rect()),
                    );
                }
            }
        }
        best
    }
}

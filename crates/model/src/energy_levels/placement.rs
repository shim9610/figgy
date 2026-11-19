//! Where an energy-level series' parts are placed each draw: the arrows, the level labels stacked and kept off other columns, the transition labels, and the emit and the pick that read them (#205, #248).

use super::*;

/// Where an energy-level series is placed: the panel's anchor space (a
/// categorical `bottom_x`, the energy on `left_y`), its categories, the view
/// the series draws, the colour bar its mapped transitions read and the row
/// drawn picked. Everything here is a display parameter.
pub struct LevelPlacement<'a> {
    pub space: &'a dyn crate::annotation::AnnotationSpace,
    pub categories: &'a [crate::config::Category],
    pub view: LevelView,
    pub colorbar: Option<&'a crate::config::ColorBarOptions>,
    pub picked: Option<PickedRow>,
}

/// The gap between a stroke's end and its label, logical pixels.
pub(super) const LABEL_GAP: f32 = 4.0;
/// The pad around a transition label's box that a knockout
/// ([`TransitionArrowStyle::label_knockout`], #357) cuts from the shafts,
/// logical pixels.
pub(super) const KNOCKOUT_PAD: f32 = 2.0;
/// The narrowest stroke share of a column slot the one-time fit
/// ([`LevelPlacement::fitted_slot_fraction`], #357) narrows a style to.
pub(super) const MIN_FITTED_SLOT_FRACTION: f32 = 0.35;
/// The fit tries the shares that are multiples of one over this.
pub(super) const SLOT_FIT_STEPS: usize = 20;
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
/// The room kept between two arrow ends on one level, pixels, when the
/// stroke is long enough and the arrowhead is narrower.
pub(super) const ARROW_STEP: f32 = 12.0;
/// The weights a fan's label column is scored by (#357): a label or leader
/// on a stroke or another label, and an arrow through a label.
const HIT: usize = 100;
const CROSS: usize = 3;

/// Where along its arrow a transition label is tried, from the middle out
/// in steps of a twentieth.
pub(super) const LABEL_STOPS: [f32; 17] = [
    0.5, 0.45, 0.55, 0.4, 0.6, 0.35, 0.65, 0.3, 0.7, 0.25, 0.75, 0.2, 0.8, 0.15, 0.85, 0.1, 0.9,
];

/// Where the emit and the pick place a series' parts this draw (#205):
/// each arrow's ends spread across the levels they meet, each level label
/// stacked clear of the others in its column, and each transition label at
/// the first free spot along its arrow. Display parameters only,
/// computed from the SSOT and the panel on every raster and pick and
/// dropped after it.
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
    /// Per transition: the point on its arrow a leader joins to its label
    /// when the label stands in a fan's column (#357), else `None`.
    transition_leader: Vec<Option<[f32; 2]>>,
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

    /// The arrows of every transition row, their ends spread along the
    /// levels they meet (#357), as in a textbook Grotrian diagram. The
    /// arrows of one bundle (the transitions between the same two columns)
    /// are ordered across the bundle by where they lie across its mean
    /// direction, and each takes the same offset from its level's centre
    /// at both ends, so they keep one order, do not cross and run
    /// near-parallel. The offsets are [`ARROW_STEP`] apart, or an
    /// arrowhead plus a margin when that is wider, squeezed evenly when the
    /// strokes are too short. Ends of other bundles on the same level are
    /// then kept that far apart, and every end stays on its stroke. A
    /// host's `x_offset` adds to the spread.
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
        let arrow = &style.transition;
        let head = if arrow.head_length > 0.0 {
            arrow.head_width
        } else {
            0.0
        };
        // The gap kept between neighbouring ends: an arrowhead and a
        // margin of one and a half shafts (at least 2 px), or
        // [`ARROW_STEP`] when that is wider.
        let gap = (head + (1.5 * arrow.width).max(2.0)).max(ARROW_STEP);
        // The span the ends may use on a stroke: the stroke, less half an
        // arrowhead at each side so every head stays on it.
        let usable = |width: f32| (width - head).max(0.0);
        let column = |row: usize| self.column_x(&records.levels[row].column);
        // Per transition: its bundle (lower and upper column), the key it
        // is ordered by across the bundle, and its offset from the levels'
        // centres.
        let mut members: Vec<(i64, i64, f32, usize)> = Vec::new();
        for (index, pair) in rows.iter().enumerate() {
            let Some((lower, upper)) = *pair else {
                continue;
            };
            let (Some(lc), Some(uc)) = (column(lower), column(upper)) else {
                continue;
            };
            members.push((lc as i64, uc as i64, 0.0, index));
        }
        members.sort_by_key(|m| (m.0, m.1));
        let mut offset = vec![0.0f32; records.transitions.len()];
        let mut start = 0;
        while start < members.len() {
            let key = (members[start].0, members[start].1);
            let end = members[start..]
                .iter()
                .position(|m| (m.0, m.1) != key)
                .map_or(members.len(), |n| start + n);
            let ends = |m: &(i64, i64, f32, usize)| {
                let (lower, upper) = rows[m.3]?;
                Some((centre(lower)?, centre(upper)?))
            };
            // The bundle's mean direction, upper end to lower end, and the
            // span its narrowest stroke leaves.
            let mut d = [0.0f32; 2];
            let mut span = f32::INFINITY;
            for m in &members[start..end] {
                if let Some(((l, lw), (u, uw))) = ends(m) {
                    d[0] += l[0] - u[0];
                    d[1] += l[1] - u[1];
                    span = span.min(usable(lw)).min(usable(uw));
                }
            }
            // Where each arrow lies across the bundle's direction: its
            // midpoint's signed distance, growing with x.
            for m in &mut members[start..end] {
                if let Some(((l, _), (u, _))) = ends(m) {
                    let mid = [(l[0] + u[0]) / 2.0, (l[1] + u[1]) / 2.0];
                    let side = if d[1] < 0.0 { -1.0 } else { 1.0 };
                    m.2 = (mid[0] * d[1] - mid[1] * d[0]) * side;
                }
            }
            members[start..end].sort_by(|a, b| {
                (a.2, a.3)
                    .partial_cmp(&(b.2, b.3))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let count = end - start;
            let step = if count > 1 && span.is_finite() {
                gap.min(span / (count - 1) as f32)
            } else {
                0.0
            };
            for (k, m) in members[start..end].iter().enumerate() {
                offset[m.3] = (k as f32 - (count - 1) as f32 / 2.0) * step;
            }
            start = end;
        }
        // Per level, its ends in order of their offsets, pushed at least a
        // gap apart (less on a stroke too short for that, and no more than
        // their bundle spaced them) and kept on it.
        let mut x_of = vec![[f32::NAN; 2]; records.transitions.len()];
        let mut ends: Vec<(usize, f32, usize, usize)> = Vec::new();
        for (index, pair) in rows.iter().enumerate() {
            let Some((lower, upper)) = *pair else {
                continue;
            };
            for (row, slot) in [(lower, 0), (upper, 1)] {
                if let Some((c, _)) = centre(row) {
                    ends.push((row, c[0] + offset[index], index, slot));
                }
            }
        }
        ends.sort_by(|a, b| {
            (a.0, a.1, a.2)
                .partial_cmp(&(b.0, b.1, b.2))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut start = 0;
        while start < ends.len() {
            let row = ends[start].0;
            let end = ends[start..]
                .iter()
                .position(|e| e.0 != row)
                .map_or(ends.len(), |n| start + n);
            if let Some((c, width)) = centre(row) {
                let half = usable(width) / 2.0;
                let count = end - start;
                let room = if count > 1 {
                    gap.min(2.0 * half / (count - 1) as f32)
                } else {
                    0.0
                };
                // The ends keep their order and at least `room` apart, so
                // another bundle's end never lands on one of these.
                // Two ends of one bundle keep the spacing the bundle gave
                // them when that is closer, so its arrows stay parallel.
                let mut previous = (f32::NAN, 0usize);
                for k in start..end {
                    let (given, index) = (ends[k].1, ends[k].2);
                    if k > start {
                        let bundle = |i: usize| rows[i].map(|p| (column(p.0), column(p.1)));
                        let least = if bundle(index) == bundle(previous.1) {
                            room.min(given - previous.0)
                        } else {
                            room
                        };
                        ends[k].1 = ends[k].1.max(ends[k - 1].1 + least);
                    }
                    previous = (given, index);
                }
                // A run wider than the stroke is squeezed onto it in
                // order; a narrower one past an end slides back.
                let (first, last) = (ends[start].1, ends[end - 1].1);
                if last - first > 2.0 * half {
                    let scale = 2.0 * half / (last - first);
                    for e in &mut ends[start..end] {
                        e.1 = c[0] - half + (e.1 - first) * scale;
                    }
                } else {
                    let shift = if last > c[0] + half {
                        c[0] + half - last
                    } else if first < c[0] - half {
                        c[0] - half - first
                    } else {
                        0.0
                    };
                    for e in &mut ends[start..end] {
                        e.1 += shift;
                    }
                }
                for e in &ends[start..end] {
                    x_of[e.2][e.3] = e.1;
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
        let formatted: Vec<Option<RichText>> = match &style.wavelength_labels {
            None => Vec::new(),
            Some(labels) => records
                .transitions
                .iter()
                .map(|t| match t.drawn_label(Some(labels)) {
                    Some(std::borrow::Cow::Owned(mut text)) => {
                        // A label formatted for an arrow coloured on its
                        // own (by the host or by its wavelength) takes
                        // that colour, adjusted for contrast (#357).
                        if let Some(color) = self.own_transition_color(style, t) {
                            text.color = readable_on_side_of(color, text.color);
                            for segment in &mut text.segments {
                                segment.color = None;
                            }
                        }
                        Some(text)
                    }
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
        let obstacles = self.level_obstacles(records, style, measure);
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
        let (transition_spot, transition_leader) = self.place_transition_labels(
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
            transition_leader,
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

    /// The level strokes, band fills and limit labels, sorted by their
    /// tops: the obstacles a level label keeps clear of (#248, #357) and,
    /// with the level labels added, a transition label.
    fn level_obstacles(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        measure: &dyn crate::text::MeasureText,
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
        // The limit labels (#357): a level or transition label keeps clear
        // of them as it does of a stroke.
        for index in 0..records.limits.len() {
            if let Some(label) = self.limit_label(records, index, measure) {
                obstacles.push(label.rect());
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
    /// right of it then left, inside the plot (#357) and clear of the level
    /// labels, the strokes, the other arrows and the labels placed before
    /// it; when none is clear, the spot of least overlap, and the middle
    /// only when every spot would leave the plot. `obstacles` are the level strokes and band fills.
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
    ) -> (
        Vec<Option<([f32; 2], LevelLabelSide)>>,
        Vec<Option<[f32; 2]>>,
    ) {
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
        // Per transition: its label's rect and whether its spot of least
        // cost still lies on another arrow or label (a crowded fan).
        let mut placed_rect: Vec<Option<crate::layout::RectF>> =
            vec![None; records.transitions.len()];
        let mut crowded = vec![false; records.transitions.len()];
        let plot = self.space.plot_rect();
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
            // The label's own place along its arrow, before the host's
            // offset: the search and the fan's column read it, so a label
            // the host drags keeps the place it was dragged from.
            let block = |t: f32, side: LevelLabelSide| {
                let p = at(t);
                Self::label_block(text, measure, p, p[0], side, [0.0; 2]).rect()
            };
            // The cost of a spot: a label or stroke it overlaps weighs more
            // than every arrow it crosses together, so a crowded bundle
            // still keeps its labels apart. The first spot of least cost
            // wins; a label the host moved stays at the middle.
            let host_placed = transition.label_offset != [0.0; 2];
            let cost = |t: f32, side: LevelLabelSide| -> usize {
                let rect = block(t, side);
                // A spot that the plot's edge would clip is taken only when
                // every spot is clipped (#357): the other side of the arrow
                // or another stop along it comes first.
                let clipped = rect.x < plot.x - 0.5
                    || rect.y < plot.y - 0.5
                    || rect.x + rect.width > plot.x + plot.width + 0.5
                    || rect.y + rect.height > plot.y + plot.height + 0.5;
                if clipped {
                    return usize::MAX - 1;
                }
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
            crowded[index] = best.0 != 0;
            let spot = if host_placed {
                (at(0.5), LevelLabelSide::Right)
            } else {
                (at(best.1), best.2)
            };
            let rect = {
                let p = spot.0;
                Self::label_block(text, measure, p, p[0], spot.1, transition.label_offset).rect()
            };
            placed_labels.push(rect);
            placed_rect[index] = Some(rect);
            transition_spot[index] = Some(spot);
        }
        let transition_leader = self.column_crowded_labels(
            records,
            formatted,
            measure,
            arrows,
            &obstacles,
            plot,
            &crowded,
            &mut placed_rect,
            &mut transition_spot,
        );
        (transition_spot, transition_leader)
    }

    /// The labels of a crowded fan in one aligned column beside it (#357):
    /// the transitions whose every spot along their arrow lies on another
    /// arrow or label, grouped by the fans their arrows form, each group of
    /// two or more stood in a column beside its fan, at one x, left-aligned,
    /// evenly spaced and in the order of their arrows, each joined to its
    /// own arrow by a leader. The column takes the side of the fan where it
    /// lies inside the plot and on the fewest strokes, labels and arrows,
    /// and only when that is fewer than the labels had along their arrows.
    /// Writes the moved labels' spots and rects; returns, per transition,
    /// the point on its arrow its leader starts from. A display parameter:
    /// the labels' text and the arrows do not change.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    fn column_crowded_labels(
        &self,
        records: &EnergyLevelSeries,
        formatted: &[Option<RichText>],
        measure: &dyn crate::text::MeasureText,
        arrows: &[Option<([f32; 2], [f32; 2])>],
        obstacles: &[crate::layout::RectF],
        plot: crate::layout::RectF,
        crowded: &[bool],
        placed_rect: &mut [Option<crate::layout::RectF>],
        transition_spot: &mut [Option<([f32; 2], LevelLabelSide)>],
    ) -> Vec<Option<[f32; 2]>> {
        let n = records.transitions.len();
        let mut leaders = vec![None; n];
        // A label in a column the host dragged stays in it, moved by its
        // offset: membership reads the labels' own places only.
        let candidate = |i: usize| {
            crowded[i] && arrows[i].is_some() && transition_text(records, formatted, i).is_some()
        };
        let bbox = |i: usize| {
            arrows[i].map(|(a, b)| crate::layout::RectF {
                x: a[0].min(b[0]),
                y: a[1].min(b[1]),
                width: (a[0] - b[0]).abs(),
                height: (a[1] - b[1]).abs(),
            })
        };
        let union = |a: crate::layout::RectF, b: crate::layout::RectF| {
            let x = a.x.min(b.x);
            let y = a.y.min(b.y);
            crate::layout::RectF {
                x,
                y,
                width: (a.x + a.width).max(b.x + b.width) - x,
                height: (a.y + a.height).max(b.y + b.height) - y,
            }
        };
        // The fans: crowded transitions whose arrows' boxes meet, grouped
        // by a union over their indices.
        let mut group: Vec<usize> = (0..n).collect();
        let find = |group: &mut Vec<usize>, mut i: usize| {
            while group[i] != i {
                group[i] = group[group[i]];
                i = group[i];
            }
            i
        };
        for i in (0..n).filter(|&i| candidate(i)) {
            for j in (i + 1..n).filter(|&j| candidate(j)) {
                if let (Some(a), Some(b)) = (bbox(i), bbox(j))
                    && overlaps(a, b)
                {
                    let (ri, rj) = (find(&mut group, i), find(&mut group, j));
                    group[rj] = ri;
                }
            }
        }
        for root in 0..n {
            let mut members: Vec<usize> = (0..n)
                .filter(|&i| candidate(i) && find(&mut group, i) == root)
                .collect();
            if members.len() < 2 {
                continue;
            }
            let Some(fan) = members.iter().filter_map(|&i| bbox(i)).reduce(union) else {
                continue;
            };
            // Each leader starts at the middle of its arrow, where the
            // label stood along it; the labels take the order of those
            // points from the top, so no two leaders cross.
            let attach = |i: usize| {
                arrows[i].map_or([0.0; 2], |(a, b)| {
                    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
                })
            };
            members.sort_by(|&a, &b| {
                attach(a)[1]
                    .total_cmp(&attach(b)[1])
                    .then(attach(a)[0].total_cmp(&attach(b)[0]))
            });
            let mid_y = members.iter().map(|&i| attach(i)[1]).sum::<f32>() / members.len() as f32;
            let sizes: Vec<crate::text::TextExtents> = members
                .iter()
                .filter_map(|&i| transition_text(records, formatted, i))
                .map(|text| measure.measure_rich(text))
                .collect();
            if sizes.len() != members.len() {
                continue;
            }
            let row = sizes
                .iter()
                .map(|e| e.ascent + e.descent)
                .fold(0.0f32, f32::max)
                + LABEL_STACK_GAP
                + 1.0;
            let width = sizes.iter().map(|e| e.width).fold(0.0f32, f32::max);
            let height = row * members.len() as f32;
            // The column stays within the fan's own height (a row past it
            // at most) and inside the plot: below a fan may lie an axis
            // break, where nothing is drawn.
            let low = (fan.y - row).max(plot.y);
            let high = (fan.y + fan.height + row).min(plot.y + plot.height) - height;
            let clamp_top = |top: f32| top.min(high).max(low);
            let others: Vec<crate::layout::RectF> = (0..n)
                .filter(|i| !members.contains(i))
                .filter_map(|i| placed_rect[i])
                .collect();
            // The rects of the column with its labels' left edge at `x` and
            // its top at `top`.
            let column = |x: f32, top: f32| -> Vec<crate::layout::RectF> {
                sizes
                    .iter()
                    .enumerate()
                    .map(|(k, e)| crate::layout::RectF {
                        x: x + LABEL_GAP,
                        y: top + row * k as f32 + (row - e.ascent - e.descent) / 2.0,
                        width: e.width,
                        height: e.ascent + e.descent,
                    })
                    .collect()
            };
            // A label or leader on a stroke or label weighs far more than an
            // arrow a label crosses.
            let cost_of = |rects: &[crate::layout::RectF]| -> Option<usize> {
                let mut cost = 0;
                // A leader that crosses a stroke or a label weighs as much
                // as a label on one.
                for (&i, rect) in members.iter().zip(rects) {
                    let from = attach(i);
                    let mid = rect.y + rect.height / 2.0;
                    let to = if from[0] <= rect.x {
                        [rect.x - 1.0, mid]
                    } else {
                        [rect.x + rect.width + 1.0, mid]
                    };
                    cost += obstacles
                        .iter()
                        .chain(&others)
                        .filter(|o| crosses(from, to, **o))
                        .count()
                        * HIT;
                }
                for rect in rects {
                    if rect.x < plot.x - 0.5 || rect.x + rect.width > plot.x + plot.width + 0.5 {
                        return None;
                    }
                    if rect.y < low - 0.5 || rect.y + rect.height > high + height + 0.5 {
                        return None;
                    }
                    cost += obstacles.iter().filter(|o| overlaps(**o, *rect)).count() * HIT;
                    cost += others.iter().filter(|o| overlaps(**o, *rect)).count() * HIT;
                    cost += arrows
                        .iter()
                        .flatten()
                        .filter(|(a, b)| crosses(*a, *b, *rect))
                        .count()
                        * CROSS;
                }
                Some(cost)
            };
            // Nearest first: beside the fan on its right then its left,
            // centred on it, then moved up and down by whole rows, then
            // further out past what the column met. The first place of
            // least cost wins.
            let near_x = [
                fan.x + fan.width + LEADER_RUN,
                fan.x - LEADER_RUN - LABEL_GAP * 2.0 - width,
            ];
            // The place of least score: its cost, then one point per row
            // it lies from the fan's middle and per step further out, so a
            // near column with an arrow through it beats a far clear one.
            let mut best: Option<(usize, usize, f32, f32)> = None;
            let consider =
                |x: f32, top: f32, far: usize, best: &mut Option<(usize, usize, f32, f32)>| {
                    if let Some(c) = cost_of(&column(x, top)) {
                        let score = c + far;
                        if best.is_none_or(|(b, _, _, _)| score < b) {
                            *best = Some((score, c, x, top));
                        }
                    }
                };
            let centred = clamp_top(mid_y - height / 2.0);
            let steps = 2 * members.len();
            for step in 0..=steps {
                let dy = row * step.div_ceil(2) as f32 * if step % 2 == 0 { 1.0 } else { -1.0 };
                for (side, x) in near_x.into_iter().enumerate() {
                    consider(
                        x,
                        clamp_top(centred + dy),
                        step.div_ceil(2) + side,
                        &mut best,
                    );
                }
            }
            // Past the strokes and labels the nearest columns met.
            for x0 in near_x {
                let met = column(x0, centred)
                    .iter()
                    .flat_map(|rect| {
                        obstacles
                            .iter()
                            .chain(&others)
                            .filter(move |o| overlaps(**o, *rect))
                    })
                    .map(|o| (o.x, o.x + o.width))
                    .fold(None, |acc: Option<(f32, f32)>, (l, r)| {
                        Some(acc.map_or((l, r), |(a, b)| (a.min(l), b.max(r))))
                    });
                if let Some((l, r)) = met {
                    let x = if x0 > fan.x {
                        r + LABEL_GAP
                    } else {
                        l - LABEL_GAP * 3.0 - width
                    };
                    for step in 0..=steps {
                        let dy =
                            row * step.div_ceil(2) as f32 * if step % 2 == 0 { 1.0 } else { -1.0 };
                        consider(x, clamp_top(centred + dy), step.div_ceil(2) + 2, &mut best);
                    }
                }
            }
            let best = best.map(|(_, c, x, top)| (c, x, top));
            let Some((cost, x, top)) = best else {
                continue;
            };
            let before: usize = members
                .iter()
                .filter_map(|&i| placed_rect[i])
                .map(|rect| {
                    obstacles.iter().filter(|o| overlaps(**o, rect)).count() * HIT
                        + arrows
                            .iter()
                            .flatten()
                            .filter(|(a, b)| crosses(*a, *b, rect))
                            .count()
                            * CROSS
                })
                .sum();
            if cost >= before {
                continue;
            }
            for (k, (&i, rect)) in members.iter().zip(column(x, top)).enumerate() {
                let y = top + row * k as f32 + row / 2.0;
                transition_spot[i] = Some(([x, y], LevelLabelSide::Right));
                placed_rect[i] = Some(rect);
                leaders[i] = Some(attach(i));
            }
        }
        leaders
    }

    /// The leader of transition row `index`'s label in `layout` (#357):
    /// from the point on its arrow to the near edge of the label as drawn,
    /// so it follows the label when the host drags it; `None` for a label
    /// beside its arrow.
    fn transition_leader(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        layout: &LevelLayout,
        index: usize,
        measure: &dyn crate::text::MeasureText,
    ) -> Option<([f32; 2], [f32; 2])> {
        let from = layout.transition_leader.get(index).copied().flatten()?;
        let rect = self
            .label(
                records,
                style,
                layout,
                LabelPart::Transition,
                index,
                measure,
            )?
            .rect();
        let mid = rect.y + rect.height / 2.0;
        let to = if from[0] <= rect.x {
            [rect.x - 1.0, mid]
        } else {
            [rect.x + rect.width + 1.0, mid]
        };
        Some((from, to))
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

    /// The label of limit row `index` (#357): right-aligned at the plot's
    /// right edge, just above its line, or just below it when above would
    /// leave the plot (a limit near the top of the energy range), so the
    /// label is not clipped. The host's `label_offset` moves it from
    /// there. `None` when the limit has no label or is not drawn.
    fn limit_label<'t>(
        &self,
        records: &'t EnergyLevelSeries,
        index: usize,
        measure: &dyn crate::text::MeasureText,
    ) -> Option<crate::sink::PlacedText<'t>> {
        let limit = records.limits.get(index)?;
        let (_, end) = self.limit_line(records, index)?;
        let text = limit.label.as_ref()?;
        let extents = measure.measure_rich(text);
        let plot = self.space.plot_rect();
        let above = end[1] - LABEL_GAP - extents.descent;
        let baseline = if above - extents.ascent >= plot.y - 0.5 {
            above
        } else {
            end[1] + LABEL_GAP + extents.ascent
        };
        let origin = (
            end[0] - LABEL_GAP - extents.width + limit.label_offset[0],
            baseline + limit.label_offset[1],
        );
        Some(crate::sink::PlacedText {
            text: crate::sink::TextRef::Rich(text),
            origin,
            extents,
            rotation: None,
        })
    }

    /// The stroke share of the column slots ([`LevelStrokeStyle::slot_fraction`])
    /// whose layout keeps the level and band labels best: the fewest labels
    /// on another column's strokes, fills, a limit line or another label,
    /// then the fewest labels moved off their own place (to the other side
    /// or onto a leader), then the widest share. The shares tried run from
    /// `style`'s own down to the one that leaves the widest label that can
    /// fit room before the next column's strokes, never below 0.35 (or
    /// `style`'s own when that is smaller). A display parameter of the
    /// renderer's one-time fit (#357), which writes it to the SSOT; the
    /// placement never calls it.
    pub fn fitted_slot_fraction(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        measure: &dyn crate::text::MeasureText,
    ) -> f32 {
        let current = style.level.slot_fraction;
        if style.label_side == LevelLabelSide::Above {
            return current;
        }
        let narrowest = self.room_slot_fraction(records, style, measure);
        if narrowest >= current {
            return current;
        }
        // The shares tried are fixed steps, so a second fit over its own
        // result tries only shares the first found no better: it keeps it.
        let mut best = (self.label_cost(records, style, measure), current);
        let mut tried = style.clone();
        for step in (0..=SLOT_FIT_STEPS).rev() {
            let fraction = step as f32 / SLOT_FIT_STEPS as f32;
            if fraction >= current || fraction < narrowest - 1e-4 {
                continue;
            }
            tried.level.slot_fraction = fraction;
            let cost = self.label_cost(records, &tried, measure);
            if cost < best.0 {
                best = (cost, fraction);
            }
        }
        best.1
    }

    /// The share that leaves each level and band label room before the
    /// next column's strokes on its side (where that column has a stroke
    /// or fill within reach of the label's height), for the labels that fit
    /// at the narrowest share; see [`Self::fitted_slot_fraction`].
    fn room_slot_fraction(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        measure: &dyn crate::text::MeasureText,
    ) -> f32 {
        let current = style.level.slot_fraction;
        let floor = MIN_FITTED_SLOT_FRACTION.min(current);
        let flip = match style.label_side {
            LevelLabelSide::Right => Some(LevelLabelSide::Left),
            LevelLabelSide::Left => Some(LevelLabelSide::Right),
            LevelLabelSide::Above => return current,
        };
        let plot = self.space.plot_rect();
        let inner = self.inner_levels(records, style);
        let sides = self.label_sides(records, style, &inner, measure, plot, flip);
        let items = records.levels.len() + records.bands.len();
        let mut fraction = current;
        for item in 0..items {
            let Some(column) = Self::item_column(records, item).and_then(|c| self.column_x(c))
            else {
                continue;
            };
            let neighbour = match sides[item] {
                LevelLabelSide::Right => column + 1.0,
                LevelLabelSide::Left => column - 1.0,
                LevelLabelSide::Above => continue,
            };
            let Some(label) = self
                .label_on(records, style, &inner, item, measure, [0.0; 2], sides[item])
                .map(|t| t.rect())
            else {
                continue;
            };
            let reach = label.height * MAX_LABEL_SHIFT;
            let (top, bottom) = (label.y - reach, label.y + label.height + reach);
            let crowded = (0..items).any(|other| {
                Self::item_column(records, other).and_then(|c| self.column_x(c)) == Some(neighbour)
                    && self
                        .item_span(records, style, other)
                        .is_some_and(|(a, b)| b >= top && a <= bottom)
            });
            if !crowded {
                continue;
            }
            let (Some(a), Some(b)) = (
                self.px(AnnotationAxis::X, column - 0.5),
                self.px(AnnotationAxis::X, column + 0.5),
            ) else {
                continue;
            };
            let slot = (b - a).abs();
            if slot <= 0.0 {
                continue;
            }
            // A label too wide for the narrowest strokes is left to the
            // stack and the other side: it does not narrow the rest.
            let needed = 1.0 - (label.width + 3.0 * LABEL_GAP) / slot;
            if needed >= floor {
                fraction = fraction.min(needed);
            }
        }
        fraction.clamp(floor, current)
    }

    /// The vertical span label item `item` covers in its column, pixels:
    /// a level's stroke or a band's fill.
    fn item_span(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        item: usize,
    ) -> Option<(f32, f32)> {
        let levels = records.levels.len();
        if item < levels {
            let level = &records.levels[item];
            if !level.role.drawn_in(self.view) {
                return None;
            }
            let y = self.px(AnnotationAxis::Y, level.energy)?;
            Some((y, y))
        } else {
            let rect = self.band_rect(records, style, item - levels)?;
            Some((rect.y, rect.y + rect.height))
        }
    }

    /// How well `style`'s layout keeps the level and band labels, lower is
    /// better: the labels on (or within a label gap of) another column's
    /// stroke or fill, on a limit line or on another label, then the labels
    /// moved off their own place.
    fn label_cost(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        measure: &dyn crate::text::MeasureText,
    ) -> (usize, usize) {
        use crate::layout::RectF;
        let overlap = |a: RectF, b: RectF| {
            a.x < b.x + b.width - 0.5
                && b.x < a.x + a.width - 0.5
                && a.y < b.y + b.height - 0.5
                && b.y < a.y + a.height - 0.5
        };
        let layout = self.layout(records, style, measure);
        let items = records.levels.len() + records.bands.len();
        let half = style.level.width / 2.0;
        // The ink of item `item` in its column.
        let ink = |item: usize| -> Option<RectF> {
            let levels = records.levels.len();
            if item < levels {
                let (a, b) = self.level_stroke(records, style, item)?;
                Some(RectF {
                    x: a[0].min(b[0]),
                    y: a[1] - half,
                    width: (b[0] - a[0]).abs(),
                    height: half * 2.0,
                })
            } else {
                self.band_rect(records, style, item - levels)
            }
        };
        // Each item's drawn label and ink, measured once.
        let levels = records.levels.len();
        let labels: Vec<Option<RectF>> = (0..items)
            .map(|item| {
                let (part, index) = if item < levels {
                    (LabelPart::Level, item)
                } else {
                    (LabelPart::Band, item - levels)
                };
                self.label(records, style, &layout, part, index, measure)
                    .map(|t| t.rect())
            })
            .collect();
        let inks: Vec<Option<RectF>> = (0..items).map(ink).collect();
        let limits: Vec<RectF> = (0..records.limits.len())
            .filter_map(|index| self.limit_line(records, index))
            .map(|(a, b)| RectF {
                x: a[0],
                y: a[1] - 0.5,
                width: b[0] - a[0],
                height: 1.0,
            })
            .collect();
        let mut conflicts = 0;
        let mut moved = 0;
        for item in 0..items {
            let Some(label) = labels[item] else {
                continue;
            };
            if layout.level_side[item] != style.label_side || layout.level_shift[item] != [0.0; 2] {
                moved += 1;
            }
            let column = Self::item_column(records, item);
            // Another column's ink counts within a label gap of the label,
            // so a label never ends flush against the next stroke.
            let clear = RectF {
                x: label.x - LABEL_GAP,
                width: label.width + 2.0 * LABEL_GAP,
                ..label
            };
            let on_ink = (0..items).any(|other| {
                Self::item_column(records, other) != column
                    && inks[other].is_some_and(|r| overlap(clear, r))
            });
            let on_label =
                (item + 1..items).any(|other| labels[other].is_some_and(|r| overlap(label, r)));
            let on_limit = limits.iter().any(|r| overlap(label, *r));
            conflicts += usize::from(on_ink) + usize::from(on_label) + usize::from(on_limit);
        }
        (conflicts, moved)
    }

    /// The colour of a transition's arrow (#357): its own
    /// ([`Transition::color`]), else its wavelength's
    /// ([`TransitionArrowStyle::color_by_wavelength`]), else its value's
    /// through the colour bar ([`TransitionArrowStyle::color_by_value`]),
    /// else the style's.
    fn transition_color(&self, style: &EnergyLevelStyle, transition: &Transition) -> Color {
        if let Some(color) = self.own_transition_color(style, transition) {
            return color;
        }
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

    /// The colour a transition's arrow takes on its own, from the host
    /// ([`Transition::color`]) or from its wavelength, or `None` when it
    /// takes a mapped value's or the style's.
    fn own_transition_color(
        &self,
        style: &EnergyLevelStyle,
        transition: &Transition,
    ) -> Option<Color> {
        transition.color.or_else(|| {
            let spectral = style.transition.color_by_wavelength.as_ref()?;
            spectral.color(transition.wavelength?)
        })
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
            LabelPart::Limit => self.limit_label(records, index, measure),
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
        // The knockout boxes (#357): every drawn transition label's box
        // plus a pad. Every shaft is broken where it crosses one.
        let knockouts: Vec<crate::layout::RectF> = if arrow.label_knockout {
            (0..records.transitions.len())
                .filter_map(|index| {
                    self.label(
                        records,
                        style,
                        &layout,
                        LabelPart::Transition,
                        index,
                        measure,
                    )
                })
                .map(|text| {
                    let r = text.rect();
                    crate::layout::RectF {
                        x: r.x - KNOCKOUT_PAD,
                        y: r.y - KNOCKOUT_PAD,
                        width: r.width + 2.0 * KNOCKOUT_PAD,
                        height: r.height + 2.0 * KNOCKOUT_PAD,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut cuts: Vec<(f32, f32)> = Vec::new();
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
            let end = if head > 0.0 { base } else { tip };
            let shaft = StrokeStyle {
                width,
                color,
                dash: &[],
                tag: "energy_transition",
            };
            // The shaft's pieces between the knockout boxes it crosses.
            knockout_cuts(tail, end, &knockouts, &mut cuts);
            let at = |t: f32| {
                [
                    tail[0] + (end[0] - tail[0]) * t,
                    tail[1] + (end[1] - tail[1]) * t,
                ]
            };
            let mut from = 0.0f32;
            for &(a, b) in &cuts {
                if a > from {
                    sink.stroke(slot, at(from), at(a), &shaft);
                }
                from = from.max(b);
            }
            if from < 1.0 {
                sink.stroke(slot, at(from), end, &shaft);
            }
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
        // The leaders of a fan's column of transition labels (#357), in
        // each arrow's colour.
        for (index, transition) in records.transitions.iter().enumerate() {
            if let Some((a, b)) = self.transition_leader(records, style, &layout, index, measure) {
                sink.stroke(
                    slot,
                    a,
                    b,
                    &StrokeStyle {
                        width: 0.6,
                        color: self.transition_color(style, transition),
                        dash: &[],
                        tag: "energy_transition_leader",
                    },
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

impl LevelPlacement<'_> {
    /// The drawn part of `records` nearest `cursor`, within `max_distance`
    /// of its edge, and that distance (zero on the part). Walks the SSOT
    /// through the same placement the emit draws with, so the pick is the
    /// drawn geometry; nothing is registered per row. On a tie a
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
                offer(
                    LevelHit::Level(row),
                    crate::select::rect_distance(&rect, cursor[0], cursor[1]),
                );
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
                        crate::select::rect_distance(&text.rect(), cursor[0], cursor[1]),
                    );
                }
            }
        }
        best
    }
}

/// `color` moved toward the side of `ink` (darker when `ink` is dark, a
/// relative luminance below 0.4, as on a light chart; lighter when it is
/// light) until it stands at a WCAG contrast of 4.5:1
/// against the opposite side's extreme (white or black), the background
/// `ink` was chosen for (#357). A colour already that readable is kept.
fn readable_on_side_of(color: Color, ink: Color) -> Color {
    let linear = |c: f32| {
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = |c: Color| 0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b);
    let dark_ink = luminance(ink) < 0.4;
    let background = if dark_ink { 1.0 } else { 0.0 };
    let contrast = |c: Color| {
        let (a, b) = (luminance(c), background);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    };
    let target = if dark_ink { 0.0 } else { 1.0 };
    let mix = |t: f32| Color {
        r: color.r + (target - color.r) * t,
        g: color.g + (target - color.g) * t,
        b: color.b + (target - color.b) * t,
        a: 1.0,
    };
    // The least mix toward the ink's side that reaches the contrast, in
    // steps of a twentieth.
    (0..=20)
        .map(|step| mix(step as f32 / 20.0))
        .find(|c| contrast(*c) >= 4.5)
        .unwrap_or(mix(1.0))
}

/// The parts of the segment from `a` to `b` (as fractions `0..=1` along
/// it) that lie inside any of `boxes`, sorted and merged, written into
/// `cuts` (#357). Each box clips the segment as a slab intersection.
fn knockout_cuts(
    a: [f32; 2],
    b: [f32; 2],
    boxes: &[crate::layout::RectF],
    cuts: &mut Vec<(f32, f32)>,
) {
    cuts.clear();
    let d = [b[0] - a[0], b[1] - a[1]];
    for r in boxes {
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        let mut inside = true;
        for (p, q) in [
            (-d[0], a[0] - r.x),
            (d[0], r.x + r.width - a[0]),
            (-d[1], a[1] - r.y),
            (d[1], r.y + r.height - a[1]),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    inside = false;
                    break;
                }
            } else {
                let t = q / p;
                if p < 0.0 {
                    t0 = t0.max(t);
                } else {
                    t1 = t1.min(t);
                }
            }
        }
        if inside && t0 < t1 {
            cuts.push((t0, t1));
        }
    }
    cuts.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
}

#[cfg(test)]
mod knockout_tests {
    use super::knockout_cuts;
    use crate::layout::RectF;

    // #357: a shaft is cut where it crosses each box, in order along it;
    // a box it misses cuts nothing, and a box over one end cuts from it.
    #[test]
    fn a_shaft_is_cut_where_it_crosses_each_box() {
        let boxes = [
            RectF {
                x: 60.0,
                y: -5.0,
                width: 10.0,
                height: 10.0,
            },
            RectF {
                x: 20.0,
                y: -5.0,
                width: 10.0,
                height: 10.0,
            },
            RectF {
                x: 40.0,
                y: 20.0,
                width: 10.0,
                height: 10.0,
            },
            RectF {
                x: -5.0,
                y: -5.0,
                width: 10.0,
                height: 10.0,
            },
        ];
        let mut cuts = Vec::new();
        knockout_cuts([0.0, 0.0], [100.0, 0.0], &boxes, &mut cuts);
        assert_eq!(cuts, vec![(0.0, 0.05), (0.2, 0.3), (0.6, 0.7)]);
        knockout_cuts([0.0, 50.0], [100.0, 50.0], &boxes, &mut cuts);
        assert!(cuts.is_empty());
    }
}

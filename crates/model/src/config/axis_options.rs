//! Axis options: scales, breaks, labels, units and categories.

use super::*;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AxisScale {
    Linear,
    Logarithmic,
    /// Probability paper (#209): the range and the values are cumulative
    /// probabilities in `(0, 1)`, placed at `F⁻¹(p)` of the scale
    /// ([`crate::probability`]). JSON `{"Probability": "Normal"}`.
    Probability(ProbabilityScale),
}

impl AxisScale {
    /// The transformed position of `value` on this scale, in which the axis
    /// is linear: the value itself on a linear axis, `log10` on a
    /// logarithmic one, `F⁻¹(p)` on a probability axis. `None` for a value
    /// with no position: non-finite, non-positive on a logarithmic axis, or
    /// outside `(0, 1)` on a probability axis. The one CPU scale map: the
    /// geometry, the ticks, the annotation space and the range helpers read
    /// it.
    pub fn forward(&self, value: f64) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        match self {
            AxisScale::Linear => Some(value),
            AxisScale::Logarithmic => (value > 0.0).then(|| value.log10()),
            AxisScale::Probability(scale) => scale.forward(value),
        }
    }

    /// The value at transformed position `t`, the inverse of
    /// [`Self::forward`]. `None` for a non-finite `t` or result.
    pub fn inverse(&self, t: f64) -> Option<f64> {
        let value = match self {
            AxisScale::Linear => t,
            AxisScale::Logarithmic => 10f64.powf(t),
            AxisScale::Probability(scale) => return scale.inverse(t),
        };
        (t.is_finite() && value.is_finite()).then_some(value)
    }
}

/// Which major ticks a logarithmic axis draws ([`AxisOptions::log_ticks`]).
///
/// The major set is a function of the axis range alone, so linked charts and
/// every output scale place the same ticks; which of them carry a label is
/// decided by the renderer's label pruning. A linear axis ignores the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LogTickPolicy {
    /// Every `major_spacing`-th whole decade (`10^k`), the floored step at
    /// least 1, as every axis drew before the policy existed.
    Decades,
    /// Majors from the range: whole decades (every `n`-th one on a range of
    /// more than six decades), or on a narrower range the first of the
    /// decades, the 1, 2 and 5 × 10^k ladder and the 1 to 9 × 10^k ladder
    /// that puts at least [`crate::tick::LOG_LADDER_MIN_MAJORS`] majors
    /// inside it. `major_spacing` is ignored.
    Adaptive,
}

impl LogTickPolicy {
    /// `true` for the default, `Decades`. Used to keep it out of serialized
    /// JSON.
    #[cfg(feature = "serde")]
    fn is_decades(&self) -> bool {
        *self == LogTickPolicy::Decades
    }
}

/// The most ranges one axis may cut out ([`AxisBreaks::ranges`], #200). A
/// bound on configuration, not on data: three cuts leave four pieces, past
/// what still reads as one axis, and the bound sizes the break tables of the
/// renderer's transform.
pub const MAX_AXIS_BREAKS: usize = 3;

/// The widest gap a cut range may be drawn as, in logical pixels
/// ([`AxisBreaks::gap_px`], #249). An axis too short for its gaps is drawn
/// unbroken.
pub const MAX_AXIS_BREAK_GAP_PX: f32 = 200.0;

/// Ranges cut out of a value axis (#200): the axis draws the rest, each cut
/// shown as a gap of [`Self::gap_px`] logical pixels, marked by
/// [`Self::mark`]. The pieces between the cuts share the axis length as
/// [`Self::sizing`] says: by default in proportion to their spans (in
/// decades on a logarithmic axis).
///
/// A cut outside the axis' current range (after a zoom or an auto-fit) has
/// no effect; one that overlaps an end is cut at the end. See
/// [`AxisBreakStyle`] for how a mark that crosses a gap is drawn.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default = "crate::defaults::axis_breaks")
)]
pub struct AxisBreaks {
    /// The cut ranges, in data units, sorted and not overlapping; at most
    /// [`MAX_AXIS_BREAKS`].
    pub ranges: Vec<AxisBreak>,
    pub style: AxisBreakStyle,
    /// The width of each gap in logical pixels, in
    /// `0..=`[`MAX_AXIS_BREAK_GAP_PX`] (#249), so the gaps of two broken
    /// axes of different lengths are equally wide. The export scale scales
    /// it, so the 2x export is the 1x layout doubled.
    pub gap_px: f32,
    /// What marks each gap on the axis line.
    pub mark: BreakMark,
    /// Whether each piece draws its own frame along the gaps: a line across
    /// the plot at each edge of each gap, in the axis line's style, so the
    /// pieces read as separate panels. Default `true`, in both styles.
    pub frame_each_panel: bool,
    /// How the pieces share the axis length (#205). Omitted from JSON when
    /// it is [`PieceSizing::ByValue`], the default.
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::piece_sizing",
            skip_serializing_if = "PieceSizing::is_by_value"
        )
    )]
    pub sizing: PieceSizing,
}

/// How the pieces of a broken axis share its length (#205), the gaps
/// aside. With `Equal` or `Weights` each piece has a scale of its own (a
/// short span can get room to read) and walks its own tick spacing; a mark
/// that crosses a gap is still the straight line between its drawn points.
/// Only a `Continuous` axis takes them: a `Panels` axis is refused.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum PieceSizing {
    /// In proportion to the pieces' spans: one scale for the whole axis.
    ByValue,
    /// Every piece the same length.
    Equal,
    /// In proportion to one positive weight per piece, in axis order: one
    /// more weight than cut ranges.
    Weights(Vec<f32>),
}

impl PieceSizing {
    pub fn is_by_value(&self) -> bool {
        matches!(self, Self::ByValue)
    }
}

/// One range cut out of an axis: the values from `from` to `to` are not
/// drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AxisBreak {
    pub from: f64,
    pub to: f64,
}

impl AxisBreak {
    pub fn new(from: f64, to: f64) -> Self {
        Self { from, to }
    }
}

/// How a broken axis draws a mark that crosses a gap (#200). Marks aligned
/// with the axes (markers, bars, cells, error bars) look the same in both;
/// lines, fills, contours and arrows differ in slope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AxisBreakStyle {
    /// One axis, linear (or logarithmic) within each piece: a line that
    /// crosses a gap is the straight line between its two drawn points,
    /// hidden inside the gap.
    Continuous,
    /// Every piece is an axis of its own, drawn as a panel of its own: a
    /// line that crosses a gap is drawn in each piece at that piece's scale
    /// and clipped at the piece's edge.
    Panels,
}

/// What marks a gap of a broken axis on the axis line.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BreakMark {
    None,
    /// Two parallel diagonal strokes across the axis line at the gap's edges,
    /// `size` pixels long, `width` wide, in `color` or, when it is `None`
    /// (the default, omitted from JSON), the axis line's colour. The marks
    /// are part of the axis that draws them.
    Slash {
        size: f32,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        color: Option<Color>,
        width: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TickVisibility {
    None,
    Outside,
    Inside,
    Both,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AxisTitleOptions {
    pub text: RichText,
    pub visible: bool,
    pub offset_x: f32,
    pub offset_y: f32,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LabelStyle {
    pub visible: bool,
    pub color: Color,
    pub font_size: f32,
    pub label_visible: bool,
    pub label_font: String,
    pub label_offset_x: f32,
    pub label_offset_y: f32,
    pub format: LabelFormat,
    pub significant_digits: u8,
    /// Whether this axis' tick labels make way where they collide with the
    /// labels of the axis across them at a corner or with a break mark's
    /// slash (#272). [`LabelClearanceMode::Auto`] (the default, omitted
    /// from JSON) drops a label whose box comes within a small visual gap
    /// of the other box (2 px in total at the default 18 px tick label
    /// font); [`LabelClearanceMode::Keep`] never drops one. A categorical axis never drops a label at a corner
    /// in either mode.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "LabelClearanceMode::is_auto")
    )]
    pub clearance: LabelClearanceMode,
}

/// How an axis' tick labels make way where they collide with the labels of
/// the axis across them or with a break mark ([`LabelStyle::clearance`],
/// #272).
///
/// Where two axes meet, one of them yields: a y axis to the x axis, an x
/// axis only to a categorical y axis
/// ([`crate::chrome::yields_corner`]). The yielding axis' setting decides:
/// `Auto` drops its labels whose box overlaps the other axis' label
/// nearest the corner, `Keep` drops none.
///
/// Two boxes overlap here when they come within a small visual gap of each
/// other, closer than it both across and along the screen: 2 px in total
/// between two labels in the default 18 px tick label font, each box's half
/// read as that share of its own font, so the gap between a label in an
/// `a` px font and one in a `b` px font is `(a + b) / 18` px and it scales
/// with the display or export scale. Boxes 1 px apart at 18 px overlap;
/// boxes the gap apart or more do not, so labels that look separate stay.
/// The other axis' setting does not matter there. A break mark is the axis' own, so its own setting
/// decides there too. Which labels go is decided at rest (a drag moves the
/// kept set as one), the same at every export scale, and never leaves an
/// axis with fewer than two labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LabelClearanceMode {
    /// Drop a label whose box comes within the visual gap (2 px in total
    /// at the default 18 px tick label font) of the corner label of the
    /// axis it yields to or of a break mark's slash. Labels the gap apart
    /// or more stay.
    #[default]
    Auto,
    /// Never drop a label for a corner or a break mark.
    Keep,
}

impl LabelClearanceMode {
    /// `true` for the default, `Auto`. Used to keep it out of serialized
    /// JSON.
    #[cfg(feature = "serde")]
    fn is_auto(&self) -> bool {
        *self == LabelClearanceMode::Auto
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AxisOptions {
    pub scale: AxisScale,
    pub min: f64,
    pub max: f64,
    pub major_spacing: f64,
    pub minor_count: usize,
    pub inverted: bool,
    pub label_style: LabelStyle,
    pub tick: TickVisibility,
    pub title_option: AxisTitleOptions,
    /// Outer margin past the axis title band. Always counted, regardless of title visibility.
    pub out_margin: f32,

    /// Detached-axis offset: shifts the axis line + ticks + tick labels
    /// perpendicular to the axis (Δx for y-axes, Δy for x-axes) away from the
    /// data-area edge they normally sit on. Margin-noncontributing visual
    /// offset — the data area, grid, and data transform are unaffected, so
    /// tick positions along the axis stay aligned with the data.
    pub line_offset: f32,

    // Axis line appearance. Tick marks reuse these (color / width / style).
    pub line_visible: bool,
    pub line_color: Color,
    pub line_width: f32,
    pub line_style: LineStylePreset,

    // Tick mark lengths. `margins()` uses `major_tick_length`.
    pub major_tick_length: f32,
    pub minor_tick_length: f32,

    /// The categories of a categorical axis, in axis order; `None` (the
    /// default) is a numeric axis.
    ///
    /// A categorical axis with `n` categories maps the data range
    /// `[-0.5, n - 0.5]`: category `i` sits at `i`, with a major tick at every
    /// integer and no minor ticks, and its tick label is `categories[i].label`
    /// in the axis' `label_style` (font, colour, offsets and visibility),
    /// thinned to every 2nd, 5th, 10th... label when neighbours crowd, like
    /// numeric labels. The grid draws its major lines at the category
    /// positions when that axis' major grid is on.
    ///
    /// On a categorical axis `min`, `max`, `major_spacing` and `minor_count`
    /// are ignored (they keep their values and their numeric meaning for when
    /// `categories` is removed again), `label_style.format` and
    /// `label_style.significant_digits` are ignored, the `auto_fit_*` family
    /// leaves the axis unchanged, the helpers that move a view set its
    /// [`AxisOptions::category_window`] (#205), and the renderer rejects a logarithmic `scale` and
    /// lists outside [`AxisOptions::validate_categories`]' limits. `inverted`
    /// flips the walk: the first category sits at the far end, and the ticks,
    /// labels, grid, data and picks follow the same `axis_fraction`. A polar chart's angle and radius axes and the colourbar axis
    /// cannot be categorical yet.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub categories: Option<Vec<Category>>,

    /// The slots a categorical axis shows (#205): `[from, to]` in category
    /// coordinates, where category `i` is centred at `i` and its slot runs
    /// `i - 0.5..i + 0.5`. `None` (the default, omitted from JSON) shows
    /// every slot, `-0.5..n - 0.5`. Ignored on a numeric axis, where `min` /
    /// `max` are the range. The helpers that move a view
    /// (`Chart::set_x_range`, a view state, zoom and pan) set it; the
    /// `auto_fit_*` family leaves it. Ticks, labels, grid, data, picks and
    /// annotations follow it through [`AxisOptions::data_range`].
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub category_window: Option<[f64; 2]>,

    /// Group headers over runs of this axis' categories (#203). Drawn on a
    /// categorical `bottom_x` or `top_x`; refused on a numeric axis and on
    /// the y axes. `None` (omitted from JSON) draws none.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub category_groups: Option<CategoryGroups>,

    /// Where this axis' line crosses the perpendicular axis, as a value of
    /// that axis: `Some(0.0)` on `bottom_x` draws the bottom axis line at
    /// `left_y = 0`, the zero line of a bar chart with negative values, and
    /// on `left_y` it draws the left line at `bottom_x = 0`. `None` (the
    /// default) keeps the line on the plot edge. A value outside the
    /// perpendicular axis' range puts the line on the nearer edge, and the
    /// line keeps its half-pixel snap. Only the line moves: the ticks, tick
    /// labels and axis title stay at the edge. A polar chart ignores it; the
    /// renderer rejects a value that is not finite.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub cross_at: Option<f64>,

    /// Which major ticks the axis draws when it is logarithmic (#42).
    /// `Decades` (the default, omitted from JSON) walks every
    /// `major_spacing`-th decade; `Adaptive` chooses decades, the 1-2-5
    /// ladder or the 1..9 ladder from the range and ignores `major_spacing`
    /// ([`crate::tick::log_ladder`]). The minors are the multiples 2 to 9
    /// that are not majors. A linear or categorical axis ignores it; the
    /// range helpers and a view state leave it as it is, so a zoom re-plans
    /// the ticks from the new range.
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::log_tick_policy",
            skip_serializing_if = "LogTickPolicy::is_decades"
        )
    )]
    pub log_ticks: LogTickPolicy,

    /// Ranges cut out of a value axis (#200). `None` (the default, omitted
    /// from JSON) draws the whole range. Allowed on `bottom_x`, `left_y` and
    /// an overlay's `y_axis` of a rectangular chart; a categorical axis,
    /// polar, ternary and 3D charts and streamed series refuse it.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub breaks: Option<AxisBreaks>,

    /// The unit the axis prints its values in (#273): a tick label reads
    /// `shown = scale * value + offset`, so raw counts can read as volts or
    /// kelvin as degrees Celsius without the host converting the data.
    /// `None` (the default, omitted from JSON) prints the data values.
    ///
    /// Only what the axis prints changes. `min`, `max`, the data, `breaks`,
    /// `cross_at`, annotations and picks stay in data units. The ticks are
    /// planned in shown units over the shown range, so the labels are round
    /// numbers after the conversion, and `major_spacing` is read in shown
    /// units ([`AxisOptions::shown_range`], [`crate::tick::major_ticks`]);
    /// each tick sits at `value = (shown - offset) / scale`. The range
    /// helpers (`Chart::set_x_range`, the `auto_fit_*` family, a view state)
    /// choose the spacing for the shown range. A linked subplot axis takes
    /// its target's unit with its spacing.
    ///
    /// `scale` is finite and non-zero (a negative one prints values that
    /// decrease along the axis) and `offset` finite. A logarithmic axis
    /// takes `offset == 0` and `scale > 0`, and its ticks are decades of the
    /// shown value. A probability axis, a categorical axis and an axis
    /// with `LabelFormat::Timestamp` labels refuse a unit
    /// ([`AxisOptions::validate_unit`]).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub unit: Option<AxisUnit>,

    /// Bands across the plot behind alternate categories (#357), a
    /// timeline's or a table's row striping. Drawn on a categorical
    /// `bottom_x` or `left_y` of a rectangular chart, under the grid and the
    /// data; any other axis draws none. `None` (the default, omitted from
    /// JSON) draws no bands. Reads the categories only: every category
    /// draws, with no count limit.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub category_bands: Option<CategoryBands>,
}

/// The bands of [`AxisOptions::category_bands`]: a fill across the plot
/// over the slot of each category `every` picks, one category wide.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CategoryBands {
    /// The band colour, every channel in `0..=1`; a translucent fill tints
    /// the plot background.
    pub fill: crate::color::Color,
    /// Which categories carry a band.
    pub every: CategoryBandEvery,
}

/// Which categories [`CategoryBands`] fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CategoryBandEvery {
    /// Every other category, starting at the second (indices 1, 3, 5, …
    /// of the axis' categories): the first row stays clear.
    Alternate,
}

impl CategoryBandEvery {
    /// Whether the category at `index` (its place in the axis'
    /// categories) carries a band.
    pub fn fills(self, index: usize) -> bool {
        match self {
            Self::Alternate => index % 2 == 1,
        }
    }
}

/// The affine unit an axis prints its values in ([`AxisOptions::unit`],
/// #273): `shown = scale * value + offset`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AxisUnit {
    /// Shown units per data unit: finite and non-zero, positive on a
    /// logarithmic axis.
    pub scale: f64,
    /// The shown value of data value 0: finite, 0 on a logarithmic axis.
    pub offset: f64,
}

impl AxisUnit {
    /// The unit that prints `scale * value + offset`.
    pub const fn new(scale: f64, offset: f64) -> Self {
        Self { scale, offset }
    }

    /// The value printed for data value `value`: `scale * value + offset`.
    pub fn shown(&self, value: f64) -> f64 {
        self.scale * value + self.offset
    }

    /// The data value printed as `shown`: `(shown - offset) / scale`.
    pub fn value(&self, shown: f64) -> f64 {
        (shown - self.offset) / self.scale
    }
}

/// One category of a categorical axis ([`AxisOptions::categories`]): a stable
/// `id` that names it in edits and selections, and the `label` its tick
/// shows.
///
/// `crate::categorical::Category` is the same type, re-exported from its
/// earlier home.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Category {
    pub id: String,
    pub label: String,
    /// The tick label drawn as rich text (scripts, TeX math) in place of
    /// `label`, such as a term symbol `$^3P^o$` (#203); `label` stays the
    /// plain name. `None` (omitted from JSON) draws `label`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub rich_label: Option<crate::text::RichText>,
    /// This category's own label offset, logical pixels, written by a drag
    /// of [`Element::CategoryLabel`](crate::layout::Element::CategoryLabel)
    /// (#309). On a polar angle axis it is `[along, across]` the category's
    /// spoke, outward and then a quarter turn counter-clockwise on screen,
    /// added after the axis' shared radial offset, so the label keeps its
    /// place relative to the spoke when the chart turns. Straight
    /// categorical axes do not read it. Zero (omitted from JSON) keeps the
    /// label where the axis places it.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "is_zero_offset")
    )]
    pub label_offset: [f32; 2],
}

impl Category {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            rich_label: None,
            label_offset: [0.0, 0.0],
        }
    }
}

/// Group headers of a categorical axis (#203): a label centred under (or,
/// on a top axis, over) a run of neighbouring categories, past the tick
/// labels, with an optional bracket spanning the run: "Singlets" over the
/// singlet term columns of an energy-level diagram.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CategoryGroups {
    /// The groups, in axis order; runs do not overlap. No count limit.
    pub groups: Vec<CategoryGroup>,
    /// The headers' drag offset from their drawn place, logical pixels,
    /// written by a drag of
    /// [`Element::CategoryGroupLabel`](crate::layout::Element::CategoryGroupLabel).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "is_zero_offset")
    )]
    pub offset: [f32; 2],
}

/// One group header of [`CategoryGroups`].
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CategoryGroup {
    /// The drawn header.
    pub label: crate::text::RichText,
    /// The id of the run's first category.
    pub first: String,
    /// The id of the run's last category (the first again for a run of
    /// one).
    pub last: String,
    /// Draw a bracket along the run between the tick labels and the header,
    /// in the header's colour.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_not"))]
    pub bracket: bool,
}

#[cfg(feature = "serde")]
fn is_not(v: &bool) -> bool {
    !*v
}

#[cfg(feature = "serde")]
fn is_zero_offset(offset: &[f32; 2]) -> bool {
    *offset == [0.0; 2]
}

impl CategoryGroups {
    /// The rules group headers are held to on an axis with `categories`:
    /// every run names categories of the axis, first not after last, the
    /// runs in axis order without overlap, valid rich text headers and a
    /// finite offset within [`crate::data_config::MAX_VALUE_LABEL_OFFSET`].
    pub fn validate(&self, categories: &[Category]) -> Result<(), crate::RuleKind> {
        let position = |id: &str| categories.iter().position(|c| c.id == id);
        let mut end: Option<usize> = None;
        for group in &self.groups {
            let (Some(first), Some(last)) = (position(&group.first), position(&group.last)) else {
                return Err(crate::RuleKind::CategoryGroupCategories);
            };
            if first > last {
                return Err(crate::RuleKind::CategoryGroupOrder);
            }
            if end.is_some_and(|end| first <= end) {
                return Err(crate::RuleKind::CategoryGroupOverlap);
            }
            end = Some(last);
            group
                .label
                .validate_math()
                .map_err(|error| crate::RuleKind::InvalidMath { error })?;
            if !group.label.within_limits() {
                return Err(crate::RuleKind::CategoryGroupLabel);
            }
        }
        if !self
            .offset
            .iter()
            .all(|v| v.is_finite() && v.abs() <= crate::data_config::MAX_VALUE_LABEL_OFFSET)
        {
            return Err(crate::RuleKind::CategoryGroupOffset);
        }
        Ok(())
    }
}

/// The most characters a category id or label may have.
pub const MAX_CATEGORY_TEXT_CHARS: usize = 80;

/// Whether `text` is one line of at most [`MAX_CATEGORY_TEXT_CHARS`]
/// characters.
fn category_text_ok(text: &str) -> bool {
    text.chars().count() <= MAX_CATEGORY_TEXT_CHARS && !text.chars().any(char::is_control)
}

impl AxisOptions {
    /// The number of categories of a categorical axis, `None` for a numeric
    /// one.
    pub fn category_count(&self) -> Option<usize> {
        self.categories.as_ref().map(Vec::len)
    }

    /// The range the axis maps: its [`Self::category_window`] on a
    /// categorical axis with `n` categories, `[-0.5, n - 0.5]` without one,
    /// and `min..max` on a numeric one.
    pub fn data_range(&self) -> (f64, f64) {
        match self.category_count() {
            Some(n) => match self.category_window {
                Some([from, to]) => (from, to),
                None => (-0.5, n as f64 - 0.5),
            },
            None => (self.min, self.max),
        }
    }

    /// Why the axis does not print in its [`Self::unit`], or `None` when it
    /// does or has none (#273). The one decision behind both
    /// [`Self::validate_unit`] and [`Self::effective_unit`], so the tick
    /// walk never uses a unit validation refuses.
    pub fn unit_rejection(&self) -> Option<crate::RuleKind> {
        let unit = self.unit?;
        if self.categories.is_some() {
            return Some(crate::RuleKind::AxisUnitCategorical);
        }
        if matches!(self.scale, AxisScale::Probability(_)) {
            return Some(crate::RuleKind::AxisUnitProbability);
        }
        if matches!(self.label_style.format, LabelFormat::Timestamp(_)) {
            return Some(crate::RuleKind::AxisUnitTimestamp);
        }
        if !unit.scale.is_finite() || unit.scale == 0.0 {
            return Some(crate::RuleKind::AxisUnitScale);
        }
        if !unit.offset.is_finite() {
            return Some(crate::RuleKind::AxisUnitOffset);
        }
        if matches!(self.scale, AxisScale::Logarithmic) && (unit.offset != 0.0 || unit.scale < 0.0)
        {
            return Some(crate::RuleKind::AxisUnitLog);
        }
        None
    }

    /// The rules [`Self::unit`] is held to (#273); an axis without a unit
    /// always passes. A finite, non-zero scale and a finite offset; on a
    /// logarithmic axis offset 0 and a positive scale; and no unit on a
    /// categorical axis, a probability axis or an axis with timestamp
    /// labels.
    pub fn validate_unit(&self) -> Result<(), crate::RuleKind> {
        self.unit_rejection().map_or(Ok(()), Err)
    }

    /// The unit the axis prints in: [`Self::unit`] when it passes
    /// [`Self::validate_unit`], otherwise `None` (the data values are
    /// printed).
    pub fn effective_unit(&self) -> Option<AxisUnit> {
        self.unit.filter(|_| self.unit_rejection().is_none())
    }

    /// The value the axis prints for data value `value`
    /// ([`Self::effective_unit`]); `value` itself without a unit.
    pub fn shown_value(&self, value: f64) -> f64 {
        match self.effective_unit() {
            Some(unit) => unit.shown(value),
            None => value,
        }
    }

    /// The shown values of the data interval `a..b`, low then high: the
    /// interval the tick plan walks (#273). `(a, b)` as given without a
    /// unit.
    pub fn shown_interval(&self, a: f64, b: f64) -> (f64, f64) {
        match self.effective_unit() {
            Some(unit) => {
                let (p, q) = (unit.shown(a), unit.shown(b));
                if p <= q { (p, q) } else { (q, p) }
            }
            None => (a, b),
        }
    }

    /// The data values of the shown interval `a..b`, low then high: the
    /// inverse of [`Self::shown_interval`].
    pub fn data_interval(&self, a: f64, b: f64) -> (f64, f64) {
        match self.effective_unit() {
            Some(unit) => {
                let (p, q) = (unit.value(a), unit.value(b));
                if p <= q { (p, q) } else { (q, p) }
            }
            None => (a, b),
        }
    }

    /// The axis range `min..max` in shown units, low then high
    /// ([`Self::shown_interval`]): `(min, max)` without a unit.
    pub fn shown_range(&self) -> (f64, f64) {
        self.shown_interval(self.min, self.max)
    }

    /// The rules [`Self::breaks`] is held to (#200); an axis without breaks
    /// always passes. At most [`MAX_AXIS_BREAKS`] finite ranges with
    /// `from < to`, sorted and not touching, positive on a logarithmic
    /// axis; a gap in `0..=`[`MAX_AXIS_BREAK_GAP_PX`] px; a mark of finite size in
    /// `0..=64` px and width in `0..=16` px with a colour in range; and a
    /// numeric axis, since a categorical axis has no range to cut.
    pub fn validate_breaks(&self) -> Result<(), crate::RuleKind> {
        let Some(breaks) = &self.breaks else {
            return Ok(());
        };
        if self.categories.is_some() {
            return Err(crate::RuleKind::BreakCategorical);
        }
        if breaks.ranges.len() > MAX_AXIS_BREAKS {
            return Err(crate::RuleKind::BreakCount);
        }
        let log = matches!(self.scale, AxisScale::Logarithmic);
        let probability = matches!(self.scale, AxisScale::Probability(_));
        let mut last = f64::NEG_INFINITY;
        for cut in &breaks.ranges {
            if !cut.from.is_finite() || !cut.to.is_finite() || cut.from >= cut.to {
                return Err(crate::RuleKind::BreakRange);
            }
            if log && cut.from <= 0.0 {
                return Err(crate::RuleKind::BreakLogRange);
            }
            if probability && !(cut.from > 0.0 && cut.to < 1.0) {
                return Err(crate::RuleKind::BreakProbabilityRange);
            }
            if cut.from <= last {
                return Err(crate::RuleKind::BreakOrder);
            }
            last = cut.to;
        }
        if !(0.0..=MAX_AXIS_BREAK_GAP_PX).contains(&breaks.gap_px) {
            return Err(crate::RuleKind::BreakGap);
        }
        if !breaks.sizing.is_by_value() && breaks.style == AxisBreakStyle::Panels {
            return Err(crate::RuleKind::BreakSizing);
        }
        if let PieceSizing::Weights(weights) = &breaks.sizing {
            if weights.len() != breaks.ranges.len() + 1 {
                return Err(crate::RuleKind::BreakWeightCount);
            }
            if !weights.iter().all(|w| w.is_finite() && *w > 0.0) {
                return Err(crate::RuleKind::BreakWeights);
            }
        }
        if let BreakMark::Slash { size, color, width } = &breaks.mark {
            let channels = color.map_or([0.0; 4], |c| [c.r, c.g, c.b, c.a]);
            if !(0.0..=64.0).contains(size)
                || !(0.0..=16.0).contains(width)
                || !channels.iter().all(|c| (0.0..=1.0).contains(c))
            {
                return Err(crate::RuleKind::BreakMark);
            }
        }
        Ok(())
    }

    /// The rules a categorical axis is held to; a numeric axis always passes.
    /// At least one category, with no upper count, ids non-empty and
    /// unique, ids and labels single-line and at most
    /// [`MAX_CATEGORY_TEXT_CHARS`] characters, and a linear scale. An
    /// inverted categorical axis is valid.
    pub fn validate_categories(&self) -> Result<(), crate::RuleKind> {
        let Some(categories) = &self.categories else {
            return Ok(());
        };
        if categories.is_empty() {
            return Err(crate::RuleKind::CategoriesEmpty);
        }
        for (index, category) in categories.iter().enumerate() {
            if category.id.is_empty()
                || !category_text_ok(&category.id)
                || categories[..index].iter().any(|c| c.id == category.id)
            {
                return Err(crate::RuleKind::CategoryIds);
            }
            if !category_text_ok(&category.label) {
                return Err(crate::RuleKind::CategoryLabels);
            }
            if let Some(rich) = &category.rich_label {
                rich.validate_math()
                    .map_err(|error| crate::RuleKind::InvalidMath { error })?;
                if !rich.within_limits() {
                    return Err(crate::RuleKind::CategoryRichLabel);
                }
            }
        }
        if let Some(groups) = &self.category_groups {
            groups.validate(categories)?;
        }
        match self.scale {
            AxisScale::Linear => {}
            AxisScale::Logarithmic => return Err(crate::RuleKind::CategoricalLogAxis),
            AxisScale::Probability(_) => {
                return Err(crate::RuleKind::CategoricalProbabilityAxis);
            }
        }
        Ok(())
    }
}

// The categorical axis (`AxisOptions::categories`): its limits, its range
// and walk, and its additive wire form.
#[cfg(test)]
pub(super) mod category_axis_tests {
    use super::{AxisOptions, AxisScale, Category};
    use crate::layout::axis_fraction;
    use crate::tick::{major_ticks, minor_ticks};

    fn categorical(n: usize) -> AxisOptions {
        let mut axis = crate::defaults::axis_options_x();
        axis.categories = Some(
            (0..n)
                .map(|i| Category::new(format!("c{i}"), format!("Category {i}")))
                .collect(),
        );
        axis
    }

    #[test]
    fn the_old_category_path_is_the_same_type() {
        let a: crate::categorical::Category = Category::new("a", "A");
        let b: super::Category = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn a_numeric_axis_always_passes_and_maps_its_own_range() {
        let mut axis = crate::defaults::axis_options_x();
        axis.min = 3.0;
        axis.max = 7.0;
        axis.scale = AxisScale::Logarithmic;
        axis.inverted = true;
        assert_eq!(axis.validate_categories(), Ok(()));
        assert_eq!(axis.category_count(), None);
        assert_eq!(axis.data_range(), (3.0, 7.0));
    }

    #[test]
    fn any_nonzero_category_count_passes() {
        // No upper count (#107): the old 64-category cap is gone.
        for n in [1, 2, 7, 64, 65, 300, 5000] {
            assert_eq!(categorical(n).validate_categories(), Ok(()), "{n}");
        }
        assert_eq!(
            categorical(0).validate_categories(),
            Err(crate::RuleKind::CategoriesEmpty)
        );
        // Every category keeps its tick past the numeric tick budget.
        let ticks: Vec<f64> = major_ticks(&categorical(5000)).collect();
        assert_eq!(ticks.len(), 5000);
        assert_eq!(ticks.last(), Some(&4999.0));
    }

    #[test]
    fn ids_must_be_unique_non_empty_single_line_and_short() {
        let ids = crate::RuleKind::CategoryIds;
        let labels = crate::RuleKind::CategoryLabels;
        let edit = |f: &dyn Fn(&mut Vec<Category>)| {
            let mut axis = categorical(3);
            f(axis.categories.as_mut().unwrap());
            axis.validate_categories()
        };
        assert_eq!(edit(&|c| c[1].id.clear()), Err(ids.clone()));
        assert_eq!(edit(&|c| c[2].id = "c0".into()), Err(ids.clone()));
        assert_eq!(edit(&|c| c[0].id = "a\nb".into()), Err(ids.clone()));
        assert_eq!(edit(&|c| c[0].id = "x".repeat(81)), Err(ids.clone()));
        assert_eq!(edit(&|c| c[0].id = "x".repeat(80)), Ok(()));
        assert_eq!(
            edit(&|c| c[1].label = "two\nlines".into()),
            Err(labels.clone())
        );
        assert_eq!(edit(&|c| c[1].label = "é".repeat(81)), Err(labels.clone()));
        assert_eq!(edit(&|c| c[1].label = "é".repeat(80)), Ok(()));
        // An empty label is a category without tick text.
        assert_eq!(edit(&|c| c[1].label.clear()), Ok(()));
    }

    #[test]
    fn a_logarithmic_categorical_axis_is_rejected_and_an_inverted_one_is_valid() {
        let mut axis = categorical(3);
        axis.scale = AxisScale::Logarithmic;
        assert_eq!(
            axis.validate_categories(),
            Err(crate::RuleKind::CategoricalLogAxis)
        );
        let mut axis = categorical(3);
        axis.inverted = true;
        assert_eq!(axis.validate_categories(), Ok(()));
    }

    /// An inverted categorical axis walks its categories from the far end:
    /// category `i` lands where category `n - 1 - i` lands on the upright
    /// axis, and the range and the major ticks stay the category positions.
    #[test]
    fn an_inverted_categorical_axis_walks_from_the_far_end() {
        let upright = categorical(3);
        let mut inverted = categorical(3);
        inverted.inverted = true;
        assert_eq!(inverted.data_range(), upright.data_range());
        for i in 0..3 {
            let a = crate::layout::axis_fraction(i as f64, &inverted, 100.0);
            let b = crate::layout::axis_fraction((2 - i) as f64, &upright, 100.0);
            assert!((a - b).abs() < 1e-6, "category {i}: {a} vs {b}");
        }
        // The first category's slot is the far end of the axis.
        assert!((crate::layout::axis_fraction(-0.5, &inverted, 100.0) - 1.0).abs() < 1e-6);
        assert!((crate::layout::axis_fraction(0.0, &inverted, 100.0) - 5.0 / 6.0).abs() < 1e-6);
    }

    /// Category `i` at `i`, majors at every category, no minors, whatever
    /// `min`, `max`, `major_spacing` and `minor_count` say.
    #[test]
    fn the_walk_of_a_categorical_axis_is_its_category_positions() {
        let mut axis = categorical(7);
        axis.min = 100.0;
        axis.max = 50.0;
        axis.major_spacing = 0.0;
        axis.minor_count = 9;
        assert_eq!(axis.data_range(), (-0.5, 6.5));
        let majors: Vec<f64> = major_ticks(&axis).collect();
        assert_eq!(majors, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(minor_ticks(&axis).count(), 0);
        let single = categorical(1);
        assert_eq!(major_ticks(&single).collect::<Vec<_>>(), vec![0.0]);
        // An empty list (rejected by validation) walks nothing.
        assert_eq!(major_ticks(&categorical(0)).count(), 0);
        // The positions sit in the middle of equal slots.
        for (i, expected) in [(0, 0.5 / 7.0), (3, 3.5 / 7.0), (6, 6.5 / 7.0)] {
            let t = axis_fraction(f64::from(i), &axis, 100.0);
            assert!((t - expected as f32).abs() < 1e-6, "{i}: {t}");
        }
        assert_eq!(axis_fraction(-0.5, &axis, 100.0), 0.0);
        assert_eq!(axis_fraction(6.5, &axis, 100.0), 1.0);
    }

    #[test]
    fn removing_the_categories_restores_the_numeric_axis() {
        let mut axis = categorical(4);
        axis.min = 2.0;
        axis.max = 4.0;
        axis.major_spacing = 1.0;
        axis.categories = None;
        assert_eq!(major_ticks(&axis).collect::<Vec<_>>(), vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn scaling_keeps_the_categories() {
        let mut cfg = crate::defaults::config();
        cfg.bottom_x = categorical(5);
        let scaled = cfg.scaled(2.0);
        assert_eq!(scaled.bottom_x.categories, cfg.bottom_x.categories);
    }

    /// Run by `serde_tests`' one test.
    #[cfg(feature = "serde")]
    pub(in crate::config) fn categories_are_additive_on_the_wire() {
        let axis = crate::defaults::axis_options_x();
        let json = serde_json::to_value(&axis).expect("serialize");
        assert!(json.get("categories").is_none(), "{json}");
        let back: AxisOptions = serde_json::from_value(json).expect("parse");
        assert_eq!(back, axis);

        let axis = categorical(3);
        let json = serde_json::to_value(&axis).expect("serialize");
        assert_eq!(
            json["categories"],
            serde_json::json!([
                {"id": "c0", "label": "Category 0"},
                {"id": "c1", "label": "Category 1"},
                {"id": "c2", "label": "Category 2"},
            ])
        );
        let back: AxisOptions = serde_json::from_value(json).expect("parse");
        assert_eq!(back, axis);

        let mut cfg = crate::defaults::config();
        cfg.left_y = categorical(2);
        let text = serde_json::to_string(&cfg).expect("serialize");
        let back: super::Config = serde_json::from_str(&text).expect("parse");
        assert_eq!(back, cfg);
        assert_eq!(serde_json::to_string(&back).expect("serialize"), text);
    }
}

#[cfg(test)]
mod axis_break_config_tests {
    //! The rules and the serialized form of [`AxisBreaks`] (#200).
    use super::{AxisBreak, AxisBreaks, BreakMark, Color};
    #[cfg(feature = "serde")]
    use super::{AxisBreakStyle, Config};

    fn broken() -> AxisBreaks {
        AxisBreaks {
            ranges: vec![AxisBreak::new(10.0, 20.0), AxisBreak::new(40.0, 50.0)],
            ..crate::defaults::axis_breaks()
        }
    }

    #[test]
    fn breaks_are_validated_on_the_axis() {
        let mut config = crate::defaults::config();
        config.left_y.min = 0.0;
        config.left_y.max = 100.0;
        config.left_y.breaks = Some(broken());
        assert_eq!(config.validate_chart(), Ok(()));
        let cases: [(fn(&mut AxisBreaks), &str); 6] = [
            (|b| b.ranges.push(AxisBreak::new(60.0, 61.0)), ""),
            (|b| b.ranges.reverse(), "sorted"),
            (|b| b.ranges[1].from = 20.0, "touch"),
            (|b| b.ranges[0].to = 5.0, "from < to"),
            (|b| b.gap_px = 300.0, "gap"),
            (
                |b| {
                    b.mark = BreakMark::Slash {
                        size: 100.0,
                        color: Some(Color::BLACK),
                        width: 1.0,
                    }
                },
                "mark",
            ),
        ];
        for (index, (edit, reason)) in cases.into_iter().enumerate() {
            let mut config = config.clone();
            edit(config.left_y.breaks.as_mut().unwrap());
            let result = config.validate_chart();
            if index == 0 {
                // Three cuts are allowed; a fourth is not.
                assert_eq!(result, Ok(()));
                config
                    .left_y
                    .breaks
                    .as_mut()
                    .unwrap()
                    .ranges
                    .push(AxisBreak::new(70.0, 71.0));
                let error = config.validate_chart().unwrap_err();
                assert!(matches!(error.kind, crate::RuleKind::BreakCount), "{error}");
                continue;
            }
            let error = result.unwrap_err();
            assert_eq!(error.field, "left_y");
            assert!(error.kind.to_string().contains(reason), "{error}");
        }
    }

    #[test]
    fn breaks_go_only_on_the_data_axes_of_a_rectangular_chart() {
        for field in ["top_x", "right_y"] {
            let mut config = crate::defaults::config();
            match field {
                "top_x" => config.top_x.breaks = Some(broken()),
                _ => config.right_y.breaks = Some(broken()),
            }
            assert_eq!(config.validate_chart().unwrap_err().field, field);
        }
        let mut config = crate::defaults::config();
        config.bottom_x.categories = Some(vec![super::Category::new("a", "A")]);
        config.bottom_x.breaks = Some(broken());
        assert!(
            config
                .validate_chart()
                .unwrap_err()
                .kind
                .to_string()
                .contains("categorical")
        );
        let mut config = crate::defaults::config();
        config.left_y.min = 0.0;
        config.left_y.max = 100.0;
        config.left_y.breaks = Some(broken());
        config.coordinates =
            crate::coordinates::Coordinates::Polar(crate::defaults::polar_options());
        assert_eq!(config.validate_chart().unwrap_err().field, "coordinates");
        let mut log = crate::defaults::config();
        log.left_y.scale = super::AxisScale::Logarithmic;
        log.left_y.min = 1.0;
        log.left_y.max = 1e6;
        log.left_y.breaks = Some(AxisBreaks {
            ranges: vec![AxisBreak::new(-1.0, 10.0)],
            ..crate::defaults::axis_breaks()
        });
        assert!(
            log.validate_chart()
                .unwrap_err()
                .kind
                .to_string()
                .contains("positive")
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn breaks_round_trip_and_are_omitted_when_absent() {
        let config: Config = crate::defaults::config();
        let json = serde_json::to_value(&config).unwrap();
        assert!(json["left_y"].get("breaks").is_none());
        let mut config = config;
        config.left_y.breaks = Some(AxisBreaks {
            style: AxisBreakStyle::Panels,
            frame_each_panel: false,
            ..broken()
        });
        let text = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back, config);
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
        // A document naming only the ranges takes the defaults.
        let mut json = serde_json::to_value(crate::defaults::config()).unwrap();
        json["left_y"]["breaks"] = serde_json::json!({"ranges": [{"from": 1.0, "to": 2.0}]});
        let parsed: Config = serde_json::from_value(json).unwrap();
        let breaks = parsed.left_y.breaks.unwrap();
        assert_eq!(breaks.style, AxisBreakStyle::Continuous);
        assert_eq!(breaks.gap_px, crate::defaults::axis_breaks().gap_px);
    }

    #[test]
    fn the_export_scale_scales_the_mark_and_the_gap() {
        let mut config = crate::defaults::config();
        config.left_y.breaks = Some(broken());
        let mut scaled = config.clone();
        scaled.scale_in_place(2.0);
        let (a, b) = (
            config.left_y.breaks.as_ref().unwrap(),
            scaled.left_y.breaks.as_ref().unwrap(),
        );
        assert_eq!(b.gap_px, a.gap_px * 2.0);
        let (
            BreakMark::Slash { size, width, .. },
            BreakMark::Slash {
                size: s2,
                width: w2,
                ..
            },
        ) = (&a.mark, &b.mark)
        else {
            panic!("slash marks")
        };
        assert_eq!((*s2, *w2), (size * 2.0, width * 2.0));
    }
}

#[cfg(test)]
mod axis_unit_tests {
    //! The unit an axis prints in (#273): ticks planned in shown units and
    //! mapped back to their data positions, the labels that print them, the
    //! rules and the serialized form.
    use super::{AxisOptions, AxisScale, AxisUnit, Config};
    use crate::format::LabelFormat;
    use crate::layout::Side;
    use crate::text::{MeasureText, RichText, TextExtents};
    use crate::tick::{major_ticks, minor_ticks, shown_tick_value};

    struct NoMeasure;
    impl MeasureText for NoMeasure {
        fn measure_rich(&self, _: &RichText) -> TextExtents {
            TextExtents {
                width: 0.0,
                ascent: 0.0,
                descent: 0.0,
            }
        }
    }

    fn linear(min: f64, max: f64, spacing: f64, unit: AxisUnit) -> AxisOptions {
        let mut axis = crate::defaults::axis_options_y();
        axis.min = min;
        axis.max = max;
        axis.major_spacing = spacing;
        axis.unit = Some(unit);
        axis.label_style.format = LabelFormat::Decimal;
        axis.label_style.significant_digits = 1;
        axis
    }

    /// The labels the axis draws, in data order, all of them kept (the
    /// ticks are placed far apart).
    fn labels(axis: &AxisOptions) -> Vec<String> {
        let majors: Vec<f64> = major_ticks(axis).collect();
        let span = axis.max - axis.min;
        crate::chrome::visible_numeric_labels(axis, Side::Left, &NoMeasure, &majors, |v| {
            (0.0, ((v - axis.min) / span * 10_000.0) as f32)
        })
        .into_iter()
        .map(|label| match label.text {
            crate::chrome::NumericTickText::Plain(text) => text,
            crate::chrome::NumericTickText::Power(rich) => {
                rich.segments.iter().map(|segment| segment.text).collect()
            }
        })
        .collect()
    }

    /// Every major is the shown grid `shown` mapped back exactly, in
    /// increasing data order, and prints its walked shown value.
    fn assert_plan(axis: &AxisOptions, shown: &[f64]) {
        let unit = axis.unit.unwrap();
        let majors: Vec<f64> = major_ticks(axis).collect();
        let mut expected: Vec<f64> = shown.iter().map(|&s| unit.value(s)).collect();
        expected.sort_by(f64::total_cmp);
        assert_eq!(
            majors.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "{majors:?} != {expected:?}"
        );
        for &value in &majors {
            let printed = shown_tick_value(axis, value);
            assert!(
                shown.iter().any(|s| s.to_bits() == printed.to_bits()),
                "{value} prints {printed}, not one of {shown:?}"
            );
            assert_eq!(unit.value(printed).to_bits(), value.to_bits());
        }
    }

    #[test]
    fn counts_print_as_volts_on_round_steps() {
        // A 16-bit ADC over 5 V: 32768 counts read 0 to 5 V.
        let axis = linear(0.0, 32768.0, 0.5, AxisUnit::new(5.0 / 32768.0, 0.0));
        assert_eq!(axis.shown_range(), (0.0, 5.0));
        let shown: Vec<f64> = (0..=10).map(|k| f64::from(k) * 0.5).collect();
        assert_plan(&axis, &shown);
        assert_eq!(
            labels(&axis),
            [
                "0", "0.5", "1.0", "1.5", "2.0", "2.5", "3.0", "3.5", "4.0", "4.5", "5.0"
            ]
        );
        // The minors subdivide the shown spacing and leave the majors out.
        let minors: Vec<f64> = minor_ticks(&axis).collect();
        assert_eq!(minors.len(), 40);
        for value in minors {
            let k = (axis.shown_value(value) / 0.1).round();
            assert_eq!(value, AxisUnit::new(5.0 / 32768.0, 0.0).value(k * 0.1));
            assert_ne!(k.rem_euclid(5.0), 0.0);
        }
        // An unusable spacing falls back to the nice step of the shown range.
        let mut fallback = axis.clone();
        fallback.major_spacing = 0.0;
        assert_eq!(crate::tick::effective_major_spacing(&fallback), 1.0);
        assert_plan(&fallback, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn kelvin_prints_as_celsius_and_fahrenheit() {
        let celsius = linear(250.0, 400.0, 20.0, AxisUnit::new(1.0, -273.15));
        let shown: Vec<f64> = (-1..=6).map(|k| f64::from(k) * 20.0).collect();
        assert_plan(&celsius, &shown);
        assert_eq!(
            labels(&celsius),
            ["-20", "0", "20", "40", "60", "80", "100", "120"]
        );
        // 1.8 and 459.67 are not exact in binary: the zero tick still
        // prints `0`, not `-0` or `1e-14`.
        let fahrenheit = linear(250.0, 400.0, 50.0, AxisUnit::new(1.8, -459.67));
        let shown: Vec<f64> = (0..=5).map(|k| f64::from(k) * 50.0).collect();
        assert_plan(&fahrenheit, &shown);
        assert_eq!(labels(&fahrenheit), ["0", "50", "100", "150", "200", "250"]);
    }

    #[test]
    fn a_negative_scale_prints_decreasing_values() {
        let axis = linear(0.0, 10.0, 5.0, AxisUnit::new(-2.0, 0.0));
        assert_eq!(axis.shown_range(), (-20.0, 0.0));
        assert_plan(&axis, &[-20.0, -15.0, -10.0, -5.0, 0.0]);
        assert_eq!(
            major_ticks(&axis).collect::<Vec<_>>(),
            [0.0, 2.5, 5.0, 7.5, 10.0]
        );
        assert_eq!(labels(&axis), ["0", "-5", "-10", "-15", "-20"]);
        let minors: Vec<f64> = minor_ticks(&axis).collect();
        assert!(minors.windows(2).all(|w| w[0] < w[1]), "{minors:?}");
        assert_eq!(minors.len(), 16);
    }

    #[test]
    fn a_log_axis_walks_decades_of_the_shown_value() {
        let mut axis = crate::defaults::axis_options_y();
        axis.scale = AxisScale::Logarithmic;
        axis.min = 1.0;
        axis.max = 1e6;
        axis.major_spacing = 1.0;
        axis.unit = Some(AxisUnit::new(1e-3, 0.0));
        let shown: Vec<f64> = (-3..=3).map(|k| 10f64.powf(f64::from(k))).collect();
        assert_plan(&axis, &shown);
        // A scale that is not a power of ten: the data positions are not
        // decades, the printed values are.
        axis.unit = Some(AxisUnit::new(5.0 / 32768.0, 0.0));
        let (low, high) = axis.shown_range();
        let shown: Vec<f64> = (-3..=2).map(|k| 10f64.powf(f64::from(k))).collect();
        assert!(low < shown[0] && *shown.last().unwrap() < high);
        assert_plan(&axis, &shown);
        // The 2..9 minors are multiples of the shown decades too.
        for value in minor_ticks(&axis) {
            let m = crate::tick::log_multiple(shown_tick_value(&axis, value));
            assert!(m.is_some_and(|m| (2..=9).contains(&m)), "{value}");
        }
    }

    #[test]
    fn a_broken_axis_walks_its_pieces_in_shown_units() {
        for scale in [2.0, -2.0] {
            for sizing in [super::PieceSizing::ByValue, super::PieceSizing::Equal] {
                let mut axis = linear(0.0, 100.0, 20.0, AxisUnit::new(scale, 1.0));
                axis.breaks = Some(super::AxisBreaks {
                    ranges: vec![super::AxisBreak::new(40.0, 60.0)],
                    sizing: sizing.clone(),
                    ..crate::defaults::axis_breaks()
                });
                let majors: Vec<f64> = crate::chrome::major_tick_values_for_axis(
                    &axis,
                    Side::Left,
                    &crate::layout::RectF {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 400.0,
                    },
                    &NoMeasure,
                )
                .into_iter()
                .collect();
                assert!(majors.len() >= 4, "{majors:?}");
                assert!(majors.windows(2).all(|w| w[0] < w[1]), "{majors:?}");
                for value in majors {
                    assert!(!(40.0 < value && value < 60.0), "{value} is in the cut");
                    let printed = shown_tick_value(&axis, value);
                    assert_eq!(printed, printed.round(), "{value} prints {printed}");
                }
            }
        }
    }

    /// A unit of 1 and 0 walks the same ticks as no unit, bit for bit.
    #[test]
    fn the_identity_unit_walks_the_ticks_of_no_unit() {
        let mut axis = crate::defaults::axis_options_y();
        for (min, max, spacing) in [(0.0, 1.0, 0.2), (-3.7, 12.1, 2.0), (1e-9, 3e-9, 5e-10)] {
            axis.min = min;
            axis.max = max;
            axis.major_spacing = spacing;
            axis.unit = None;
            let bare: Vec<u64> = major_ticks(&axis).map(f64::to_bits).collect();
            let bare_minor: Vec<u64> = minor_ticks(&axis).map(f64::to_bits).collect();
            axis.unit = Some(AxisUnit::new(1.0, 0.0));
            assert_eq!(
                major_ticks(&axis).map(f64::to_bits).collect::<Vec<_>>(),
                bare
            );
            assert_eq!(
                minor_ticks(&axis).map(f64::to_bits).collect::<Vec<_>>(),
                bare_minor
            );
        }
    }

    /// `auto_ticks` widens the range to whole shown steps and keeps the
    /// spacing in shown units.
    #[test]
    fn auto_ticks_plans_whole_shown_steps() {
        let mut axis = linear(0.0, 1.0, 1.0, AxisUnit::new(5.0 / 32768.0, 0.0));
        axis.auto_ticks(0.0, 30000.0, 5).unwrap();
        let (low, high) = axis.shown_range();
        let steps = |v: f64| v / axis.major_spacing;
        assert!(axis.min <= 0.0 && axis.max >= 30000.0);
        assert!((steps(low) - steps(low).round()).abs() < 1e-9, "{low}");
        assert!((steps(high) - steps(high).round()).abs() < 1e-9, "{high}");
        assert_eq!(axis.major_spacing, 1.0);
    }

    #[test]
    fn a_unit_is_validated_on_its_axis() {
        let mut config = crate::defaults::config();
        config.left_y.min = 1.0;
        config.left_y.max = 100.0;
        config.left_y.unit = Some(AxisUnit::new(-2.0, 1.0));
        assert_eq!(config.validate_chart(), Ok(()));
        let refused = |edit: fn(&mut Config), field: &str, kind: crate::RuleKind| {
            let mut config = config.clone();
            edit(&mut config);
            let error = config.validate_chart().unwrap_err();
            assert_eq!((error.field, error.kind), (field, kind));
        };
        use crate::RuleKind as K;
        refused(
            |c| c.left_y.unit = Some(AxisUnit::new(0.0, 0.0)),
            "left_y",
            K::AxisUnitScale,
        );
        refused(
            |c| c.left_y.unit = Some(AxisUnit::new(f64::INFINITY, 0.0)),
            "left_y",
            K::AxisUnitScale,
        );
        refused(
            |c| c.left_y.unit = Some(AxisUnit::new(1.0, f64::NAN)),
            "left_y",
            K::AxisUnitOffset,
        );
        // A log axis takes a positive scale and no offset.
        refused(
            |c| {
                c.left_y.scale = AxisScale::Logarithmic;
                c.left_y.unit = Some(AxisUnit::new(2.0, 1.0));
            },
            "left_y",
            K::AxisUnitLog,
        );
        refused(
            |c| {
                c.left_y.scale = AxisScale::Logarithmic;
                c.left_y.unit = Some(AxisUnit::new(-2.0, 0.0));
            },
            "left_y",
            K::AxisUnitLog,
        );
        refused(
            |c| {
                c.left_y.min = 0.1;
                c.left_y.max = 0.9;
                c.left_y.scale =
                    AxisScale::Probability(crate::probability::ProbabilityScale::Normal);
            },
            "left_y",
            K::AxisUnitProbability,
        );
        refused(
            |c| {
                use crate::format::*;
                c.left_y.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat {
                    unit: TimestampUnit::Seconds,
                    timezone: TimestampZone::FixedOffsetMinutes(0),
                    label: TimestampLabelMode::Pattern("%H:%M".into()),
                    fractional: FractionalSecondDigits::Fixed(0),
                    tick_policy: TimestampTickPolicy::NumericSpacing,
                    step: None,
                    period_labels: false,
                })
            },
            "left_y",
            K::AxisUnitTimestamp,
        );
        refused(
            |c| {
                c.bottom_x.categories = Some(vec![super::Category::new("a", "A")]);
                c.bottom_x.unit = Some(AxisUnit::new(2.0, 0.0));
            },
            "bottom_x",
            K::AxisUnitCategorical,
        );
        // The overlay and colourbar axes are held to the same rules.
        refused(
            |c| {
                let mut y = crate::defaults::axis_options_y();
                y.unit = Some(AxisUnit::new(0.0, 0.0));
                c.overlays.push(crate::layer::OverlayLayer {
                    y_axis: y,
                    ..crate::defaults::overlay_layer()
                });
            },
            "overlays.y_axis",
            K::AxisUnitScale,
        );
        refused(
            |c| {
                let mut bar = crate::defaults::color_bar_options();
                bar.axis.unit = Some(AxisUnit::new(1.0, f64::INFINITY));
                c.colorbar = Some(bar);
            },
            "colorbar.axis",
            K::AxisUnitOffset,
        );
        // A refused unit is not walked: the ticks are the data's.
        let mut log = config.left_y.clone();
        log.scale = AxisScale::Logarithmic;
        log.unit = Some(AxisUnit::new(2.0, 1.0));
        assert_eq!(log.effective_unit(), None);
        let walked: Vec<f64> = major_ticks(&log).collect();
        log.unit = None;
        assert_eq!(major_ticks(&log).collect::<Vec<_>>(), walked);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn a_unit_round_trips_and_is_omitted_when_absent() {
        // An older document has no `unit` and re-serializes unchanged.
        let text = serde_json::to_string(&crate::defaults::config()).unwrap();
        assert!(!text.contains("\"unit\""), "{text}");
        let old: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(old.left_y.unit, None);
        assert_eq!(serde_json::to_string(&old).unwrap(), text);
        // Set, it is written as its two numbers and reads back.
        let mut config = crate::defaults::config();
        config.left_y.unit = Some(AxisUnit::new(5.0 / 32768.0, 0.0));
        config.bottom_x.unit = Some(AxisUnit::new(1.0, -273.15));
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(
            json["bottom_x"]["unit"],
            serde_json::json!({ "scale": 1.0, "offset": -273.15 })
        );
        assert!(json["top_x"].get("unit").is_none());
        let text = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back, config);
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
    }
}

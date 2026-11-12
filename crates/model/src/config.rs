use crate::color::Color;
use crate::colormap::ColorMap;
use crate::format::LabelFormat;
use crate::layer::{AnnotationLayer, LayerRef, OverlayLayer, Subplot};
use crate::layout::{ChartArea, Rect, Side};
use crate::line::LineStylePreset;
use crate::polar::PolarOptions;
pub use crate::probability::ProbabilityScale;
use crate::text::RichText;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChartTitleOptions {
    pub text: RichText,
    pub visible: bool,
    pub offset_x: f32,
    pub offset_y: f32,
    /// Height of the title band at the top of the chart area; the title sits
    /// on a baseline in it ([`chart_title_placement`]). The title helpers
    /// grow it through [`fit_chart_title_margin`] when the title is taller
    /// than the band; a value injected directly is kept as given.
    ///
    /// [`fit_chart_title_margin`]: crate::layout::fit_chart_title_margin
    /// [`chart_title_placement`]: crate::layout::chart_title_placement
    pub top_margin: f32,
}

impl ChartTitleOptions {
    /// A plain title in one style: the characters of `title` in `font`,
    /// `label_size * 1.5` high, in `color`, left-aligned, visible, with no
    /// offset and no band of its own (`top_margin` 0). This is the rich
    /// title the radial chart's legacy plain `title` stands for; once
    /// installed as its `chart_title` it is authoritative and the legacy
    /// string is ignored.
    pub fn from_legacy_plain(title: &str, font: &str, label_size: f32, color: Color) -> Self {
        Self {
            text: RichText {
                segments: title.chars().map(crate::text::RichSegment::plain).collect(),
                color,
                font_size: label_size * 1.5,
                font: font.into(),
                align: crate::text::TextAlign::Left,
            },
            visible: true,
            offset_x: 0.0,
            offset_y: 0.0,
            top_margin: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GridOptions {
    pub show_major_x: bool,
    pub major_x_color: Color,
    pub major_x_width: f32,
    pub major_x_style: LineStylePreset,

    pub show_major_y: bool,
    pub major_y_color: Color,
    pub major_y_width: f32,
    pub major_y_style: LineStylePreset,

    pub show_minor_x: bool,
    pub minor_x_color: Color,
    pub minor_x_width: f32,
    pub minor_x_style: LineStylePreset,

    pub show_minor_y: bool,
    pub minor_y_color: Color,
    pub minor_y_width: f32,
    pub minor_y_style: LineStylePreset,
}

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
}

impl Category {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            rich_label: None,
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

    /// The data value the axis prints as `shown`; `shown` itself without a
    /// unit.
    pub fn data_value(&self, shown: f64) -> f64 {
        match self.effective_unit() {
            Some(unit) => unit.value(shown),
            None => shown,
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

/// Hand-drawn ("sketch") render-mode parameters. Every field has a default —
/// JSON `{"mode":"sketch"}` alone enables the mode with stock values
/// (`serde(default = "crate::defaults::sketch_options")`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default = "crate::defaults::sketch_options"))]
pub struct SketchOptions {
    /// Perpendicular path-displacement amplitude, in px. Default 1.5.
    pub amplitude_px: f32,
    /// Displacement wavelength, in px — one wobble per this arc length along
    /// the path. Default 60.0.
    pub wavelength_px: f32,
    /// Global seed. Same (config, data) → byte-identical output. Default 0.
    pub seed: u32,
}

/// Milkyway draw-style parameters. All fields have defaults — JSON
/// `{"mode":"milkyway"}` alone works. Lines render as star chains over
/// a soft nebula ribbon tinted with the series `line_color` (that ribbon is
/// what keeps multiple series distinguishable); star counts stay uniform per
/// arc length, while star colors stay physical (blackbody locus, population
/// mix follows the clump field).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default = "crate::defaults::milkyway_options")
)]
pub struct MilkywayOptions {
    /// Stars per 100 px of arc length. Default 14.0.
    pub star_density: f32,
    /// Nebula ribbon full width, in px ("두께"). Default 14.0.
    pub ribbon_width_px: f32,
    /// Ribbon brightness 0..1 — how strongly the series-colored haze shows
    /// behind the stars. Default 0.30 (thin).
    pub ribbon_intensity: f32,
    /// Global star size multiplier. Default 1.0.
    pub star_scale: f32,
    /// Global star brightness multiplier. Default 1.0.
    pub star_brightness: f32,
    /// Perpendicular star scatter σ from the path, in px. Larger reads more
    /// like a loose cluster, smaller tracks the data tighter. Default 2.5.
    pub spread_px: f32,
    /// Multiplier on the style's px-denominated STRUCTURE constants — the
    /// clump/population noise wavelength (90 px base) and binary-companion
    /// separation. This is what keeps the star texture resolution-invariant:
    /// export scaling multiplies it (and divides `star_density`)
    /// automatically; hosts that render a larger physical canvas (dpr > 1)
    /// should do the same. Default 1.0.
    pub structure_scale: f32,
    /// Luminosity-function slope: the exponent of the brightness power law.
    /// Higher → a larger fraction of faint small stars per bright anchor
    /// (real fields sit faint-heavy). Sensible range ~1.5..6. Default 3.0.
    pub faint_bias: f32,
    /// Decoration glow (line-light bloom) gain on axes/ticks/labels/titles.
    /// 0 disables the bloom. Default 0.55.
    pub glow: f32,
    /// Nebula cloud intensity multiplier for the space backdrop. 1.0 is the
    /// legibility-calibrated default (peak ≤ ~12/255 per channel); the
    /// painter keeps the panel center cleanest at any setting. Default 1.0.
    pub nebula: f32,
    /// Background dust-star density multiplier (1 px, far dimmer than data
    /// stars). 0 removes the dust. Default 1.0.
    pub dust: f32,
    /// Planet atmospheric rim-glow strength (the series color's only
    /// appearance on scatter planets). Default 0.34.
    pub planet_rim: f32,
    /// Global seed. Same (config, data) → identical output. Default 0.
    pub seed: u32,
}

/// Constellation draw-style parameters. All fields have defaults. Only
/// `ScatterLine` series are drawn: PSF-rendered stars sit at the scatter data
/// points, connected by a translucent line.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default = "crate::defaults::constellation_options")
)]
pub struct ConstellationOptions {
    /// Opacity applied to the point stars. Default 1.0.
    pub star_opacity: f32,
    /// Opacity applied to the connecting line. Default 0.45.
    pub line_opacity: f32,
}

/// UI-facing metadata for one stylized-mode parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StyleParamSpec {
    /// JSON field name inside the `draw_style` object.
    pub key: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Integer-valued (e.g. `seed`) — hosts render a stepper, not a slider.
    pub integer: bool,
}

const fn spec(key: &'static str, min: f64, max: f64, default: f64) -> StyleParamSpec {
    StyleParamSpec {
        key,
        min,
        max,
        default,
        integer: false,
    }
}

const fn spec_int(key: &'static str, min: f64, max: f64, default: f64) -> StyleParamSpec {
    StyleParamSpec {
        key,
        min,
        max,
        default,
        integer: true,
    }
}

impl SketchOptions {
    /// Parameter metadata for hosts — see [`StyleParamSpec`]. A model test
    /// pins each `default` to [`Default`], so the two cannot drift.
    pub const PARAM_SPECS: &'static [StyleParamSpec] = &[
        spec("amplitude_px", 0.0, 8.0, 1.5),
        spec("wavelength_px", 10.0, 200.0, 60.0),
        spec_int("seed", 0.0, 9999.0, 0.0),
    ];
}

impl MilkywayOptions {
    /// Parameter metadata for hosts — see [`StyleParamSpec`]. A model test
    /// pins each `default` to [`Default`], so the two cannot drift.
    pub const PARAM_SPECS: &'static [StyleParamSpec] = &[
        spec("star_density", 0.0, 120.0, 14.0),
        spec("ribbon_width_px", 2.0, 40.0, 14.0),
        spec("ribbon_intensity", 0.0, 1.0, 0.30),
        spec("star_scale", 0.3, 3.0, 1.0),
        spec("star_brightness", 0.0, 4.0, 1.0),
        spec("spread_px", 0.0, 10.0, 2.5),
        spec("structure_scale", 0.25, 4.0, 1.0),
        spec("faint_bias", 0.5, 24.0, 3.0),
        spec("glow", 0.0, 1.5, 0.55),
        spec("nebula", 0.0, 1.5, 1.0),
        spec("dust", 0.0, 3.0, 1.0),
        spec("planet_rim", 0.0, 1.0, 0.34),
        spec_int("seed", 0.0, 9999.0, 0.0),
    ];
}

impl ConstellationOptions {
    /// Parameter metadata for hosts -- see [`StyleParamSpec`]. A model test
    /// pins each `default` to [`Default`], so the two cannot drift.
    pub const PARAM_SPECS: &'static [StyleParamSpec] = &[
        spec("star_opacity", 0.0, 1.0, 1.0),
        spec("line_opacity", 0.0, 1.0, 0.45),
    ];
}

/// Chart-global render style. `Precise` is the default scientific path;
/// every other variant is an opt-in stylized mode with its own GPU
/// pipeline variants and decoration stroker.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "mode", rename_all = "snake_case"))]
pub enum DrawStyle {
    Precise,
    Sketch(SketchOptions),
    Milkyway(MilkywayOptions),
    Constellation(ConstellationOptions),
}

/// A renderer-independent reference to one selected data primitive.
///
/// These records carry provenance only. Coordinates, bin edges, matrix
/// values, and contour values remain in the caller's registered source
/// columns / series configuration; read them from there.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
pub enum PickedDataRef {
    Point {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        point_index: usize,
    },
    HistogramBin {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        bin_index: usize,
        /// The half of a split violin (`ViolinSide::Low` or `High`) at the
        /// category `bin_index`; `None` for every other bin, a box, or a
        /// symmetric violin.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        half: Option<crate::data_config::ViolinSide>,
    },
    MatrixCell {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        /// Canonical x-axis cell/quad index, independent of matrix storage
        /// orientation.
        x_index: usize,
        /// Canonical y-axis cell/quad index, independent of matrix storage
        /// orientation.
        y_index: usize,
    },
    ContourLevel {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        /// Index into `ContourConfig.levels`; the level value itself stays in
        /// the chart configuration SSoT.
        level_index: usize,
        /// Canonical x/y sample-cell indices containing the hit. They identify
        /// the local piece without copying a reconstructed coordinate.
        x_index: usize,
        y_index: usize,
    },
    /// The value label of one category of a category bar, box plot or
    /// radar series: its `labels.text[category_index]`, drawn at the bar
    /// end, above the box or outside the vertex. A selected label is
    /// outlined by its frame and is what a drag moves
    /// (`SeriesConfig::drag_value_label`); `step_selection` does not step
    /// it.
    ValueLabel {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        /// The category, in axis order: the index into `labels.text`. On an
        /// energy-level series, the row in the list `part` names.
        category_index: usize,
        /// On an energy-level series, the list the label belongs to (#203);
        /// `None` for every other series.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        part: Option<crate::energy_levels::LabelPart>,
    },
    /// One triangle of a triangle mesh series (#210): its row in the
    /// triangle index column, in triangle order. The vertex nearest the
    /// cursor is a hover answer and is not stored.
    MeshTriangle {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        triangle_index: usize,
    },
    /// A vertex of a Feynman series (#234): its row in `vertices`. A drag
    /// moves it (`FeynmanSeries::drag_vertex`).
    FeynmanVertex {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        vertex_index: usize,
    },
    /// A propagator of a Feynman series (#234): its row in `propagators`.
    /// A drag bends it (`FeynmanSeries::drag_bend`).
    FeynmanPropagator {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        propagator_index: usize,
    },
    /// A label of a Feynman series (#234): the row `index` of the list
    /// `part` names. A drag moves it (`FeynmanSeries::drag_label`);
    /// `step_selection` does not step it.
    FeynmanLabel {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        part: crate::feynman::FeynmanLabelPart,
        index: usize,
    },
}

impl PickedDataRef {
    pub fn source_id(&self) -> Option<&str> {
        match self {
            Self::Point { source_id, .. }
            | Self::HistogramBin { source_id, .. }
            | Self::MatrixCell { source_id, .. }
            | Self::ContourLevel { source_id, .. }
            | Self::ValueLabel { source_id, .. }
            | Self::MeshTriangle { source_id, .. }
            | Self::FeynmanVertex { source_id, .. }
            | Self::FeynmanPropagator { source_id, .. }
            | Self::FeynmanLabel { source_id, .. } => source_id.as_deref(),
        }
    }

    pub fn series_id(&self) -> &str {
        match self {
            Self::Point { series_id, .. }
            | Self::HistogramBin { series_id, .. }
            | Self::MatrixCell { series_id, .. }
            | Self::ContourLevel { series_id, .. }
            | Self::ValueLabel { series_id, .. }
            | Self::MeshTriangle { series_id, .. }
            | Self::FeynmanVertex { series_id, .. }
            | Self::FeynmanPropagator { series_id, .. }
            | Self::FeynmanLabel { series_id, .. } => series_id,
        }
    }
}

/// Visual policy for typed data selections.
///
/// The selected records are small stable identities. Every overlay resolves
/// its geometry from the same current `Config` and `ColumnPool` used by the
/// normal draw, so an axis/range update cannot leave a stored CPU rectangle or
/// contour segment behind.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default = "crate::defaults::data_selections_config")
)]
pub struct DataSelectionsConfig {
    pub visible: bool,
    pub refs: Vec<PickedDataRef>,
    pub highlight_color: Color,
    pub outline_width_px: f32,
    pub point_radius_extra_px: f32,
    pub contour_width_extra_px: f32,
}

impl DataSelectionsConfig {
    /// No selection, with the default highlight look (#266).
    pub fn empty() -> Self {
        crate::defaults::data_selections_config()
    }
}

impl DrawStyle {
    /// True for the default scientific path (used by serde skip).
    pub fn is_precise(&self) -> bool {
        matches!(self, DrawStyle::Precise)
    }
    /// Sketch parameters when the sketch style is active.
    pub fn sketch(&self) -> Option<&SketchOptions> {
        match self {
            DrawStyle::Sketch(s) => Some(s),
            _ => None,
        }
    }
    /// Milkyway parameters when the milkyway style is active.
    pub fn milkyway(&self) -> Option<&MilkywayOptions> {
        match self {
            DrawStyle::Milkyway(c) => Some(c),
            _ => None,
        }
    }
    /// Constellation parameters when the constellation style is active.
    pub fn constellation(&self) -> Option<&ConstellationOptions> {
        match self {
            DrawStyle::Constellation(c) => Some(c),
            _ => None,
        }
    }

    /// Every mode tag, in declaration order — the values valid as the JSON
    /// `"mode"` of `draw_style`.
    pub fn mode_tags() -> &'static [&'static str] {
        &["precise", "sketch", "milkyway", "constellation"]
    }

    /// Parameter metadata for one mode tag. `precise` has no parameters
    /// (empty slice); unknown tags yield `None`. Hosts generate their
    /// parameter UI (sliders/steppers) from this instead of hardcoding
    /// ranges.
    pub fn param_specs_for_mode(mode: &str) -> Option<&'static [StyleParamSpec]> {
        match mode {
            "precise" => Some(&[]),
            "sketch" => Some(SketchOptions::PARAM_SPECS),
            "milkyway" => Some(MilkywayOptions::PARAM_SPECS),
            "constellation" => Some(ConstellationOptions::PARAM_SPECS),
            _ => None,
        }
    }
}

/// Where the colourbar sits along its side when it is shorter than the data
/// area (`length_frac < 1`).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BarAlign {
    /// Top for a vertical bar, left for a horizontal one.
    Start,
    Center,
    /// Bottom for a vertical bar, right for a horizontal one.
    End,
}

/// The colourbar — and, because it owns the z axis, the chart's only z scale.
///
/// `axis` is the whole point. It is the same [`AxisOptions`] the four chart
/// axes use, so `scale` (including `Logarithmic`), `min` / `max`,
/// `major_spacing`, `minor_count`, `label_style` (including
/// [`LabelFormat::Power`]), `tick`, and `title_option` all mean here exactly
/// what they mean there — and tick generation, label formatting, and log
/// handling are the same code, not a parallel implementation that drifts.
///
/// `axis.min` / `axis.max` are the authority on the z range. A helper may fill
/// them in from column statistics (matrix z values, or the value columns of
/// point and vector colour maps), the way [`AxisOptions::auto_ticks`] fills in
/// spacing, but nothing else stores a z range.
///
/// Consequences:
///
/// - A `Heatmap` / `Contour` / `HeatmapContour` series, a series whose
///   scatter style sets `point_color_map` and a vector series whose style
///   sets `color_map` require this to be `Some`. Without it there is no z
///   range and no colormap anywhere, so there is no value or colour to draw —
///   the renderer rejects the series rather than inventing one.
/// - To hide the bar, set `visible: false`. The options stay, those series
///   still render, and the band is given back to the data area.
/// - There is therefore **one z scale per chart**. Every heatmap, contour and
///   colour-mapped scatter or vector series in one chart shares it; a chart
///   cannot have two colourbars with different scales.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColorBarOptions {
    /// False draws nothing and reserves no band. Field and colour-mapped
    /// series still render.
    pub visible: bool,
    /// Which margin the bar occupies. `Left` / `Right` give a vertical bar,
    /// `Top` / `Bottom` a horizontal one, and this alone decides which.
    ///
    /// The discrete half of the bar's placement, the way
    /// [`Legend::corner`](crate::legend::Legend::corner) is for the legend box;
    /// `offset_x` / `offset_y` are the continuous half.
    pub side: Side,
    /// The strip's short dimension: width for a vertical bar, height for a
    /// horizontal one.
    pub thickness_px: f32,
    /// Space between the strip and the band of the chart axis on the same
    /// side (that axis' ticks and labels next to the data area).
    pub gap_px: f32,
    /// Strip length as a fraction of the data area's length along its side.
    pub length_frac: f32,
    pub align: BarAlign,
    /// Free offset from the `side` + `align` anchor, in screen pixels — the
    /// same contract as `Legend::offset_{x,y}` and the title / label offsets:
    /// **margin-noncontributing**, so nudging the bar moves it without
    /// reflowing the data area under the pointer.
    ///
    /// This is where a drag accumulates: it is the continuous position the
    /// selection / drag / resize machinery moves, as for every other piece of
    /// chrome.
    pub offset_x: f32,
    pub offset_y: f32,
    pub colormap: ColorMap,
    /// Colour for z values the ramp cannot place: NaN, infinities, and
    /// non-positive z on a logarithmic colourbar. Fully transparent by default
    /// — "missing" reads as absent rather than as some particular value.
    pub nan_color: Color,
    pub border_color: Color,
    pub border_width: f32,
    /// The z axis. See the type docs — this is the SSoT for the z range.
    pub axis: AxisOptions,
}

impl ColorBarOptions {
    /// Where `z` falls on the ramp, in `[0, 1]`, or `None` when the ramp cannot
    /// place it — draw those with [`Self::nan_color`].
    ///
    /// This is **the** z→colour normalization. The CPU colourbar strip and the
    /// GPU field, colour-mapped scatter and vector shaders all go through this
    /// formula, so a cell or marker and the tick beside it cannot disagree
    /// about what a value looks like:
    ///
    /// ```text
    /// linear: t = (z - min) / (max - min)
    /// log:    t = (log10 z - log10 min) / (log10 max - log10 min)
    /// ```
    ///
    /// clamped to `[0, 1]`. `None` for a non-finite `z` (NaN or an infinity),
    /// for a non-positive `z` on a logarithmic bar (no logarithm to take), and
    /// for a non-finite or degenerate range.
    ///
    /// Two things it deliberately does **not** do:
    ///
    /// - It does not clamp an unplaceable value to an endpoint. "Missing" and
    ///   "smallest" are different facts, and `nan_color` is how the second one
    ///   stays distinguishable.
    /// - It does not apply `axis.inverted`. Inverting an axis moves where a
    ///   value is *drawn*; it does not change which colour that value has. The
    ///   strip's screen ends swap, its ramp does not reverse.
    pub fn normalized_z(&self, z: f64) -> Option<f32> {
        let axis = &self.axis;
        if !z.is_finite() || !axis.min.is_finite() || !axis.max.is_finite() {
            return None;
        }
        let (value, low, high) = match axis.scale {
            AxisScale::Linear => (z, axis.min, axis.max),
            AxisScale::Logarithmic => {
                if z <= 0.0 || axis.min <= 0.0 || axis.max <= 0.0 {
                    return None;
                }
                (z.log10(), axis.min.log10(), axis.max.log10())
            }
            // Refused on a colourbar axis (#209): no colour.
            AxisScale::Probability(_) => return None,
        };
        let span = high - low;
        if span <= 0.0 {
            return None;
        }
        Some((((value - low) / span) as f32).clamp(0.0, 1.0))
    }

    /// The colour for `z`: the ramp where it can be placed, `nan_color` where it
    /// cannot.
    pub fn color_for_z(&self, z: f64) -> Color {
        match self.normalized_z(z) {
            Some(t) => self.colormap.sample(t),
            None => self.nan_color,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Config {
    pub chart_area: ChartArea,
    pub top_x: AxisOptions,
    pub bottom_x: AxisOptions,
    pub left_y: AxisOptions,
    pub right_y: AxisOptions,
    pub chart_title: ChartTitleOptions,
    pub grid: GridOptions,
    pub legend: Legend,
    /// The selected points, bins, cells, contour pieces and value labels,
    /// and how they are highlighted: the chart's one selection (#160).
    ///
    /// A document written before 0.19 may also carry `picked_points`, the
    /// former point-only selection. It is read into this field: its points
    /// become [`PickedDataRef::Point`] refs after the ones `picked_data`
    /// lists, and its ring colour, width and radius become the highlight's
    /// when the document has no `picked_data`. Only `picked_data` is
    /// written.
    #[cfg_attr(feature = "serde", serde(flatten, with = "picked_data_wire"))]
    pub picked_data: Option<DataSelectionsConfig>,
    /// The colourbar and, with it, the chart's z scale. `None` (key absent in
    /// JSON) is a chart with no z dimension — the shape of every document
    /// written before field series existed.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub colorbar: Option<ColorBarOptions>,
    /// Chart-global render style. `Precise` (default, key absent in JSON) is
    /// identical to current rendering; every other variant is an opt-in
    /// stylized mode. No per-series mixing.
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::draw_style",
            skip_serializing_if = "DrawStyle::is_precise"
        )
    )]
    pub draw_style: DrawStyle,
    /// The coordinate system: rectangular (the default, no key in JSON),
    /// polar, ternary, a 3D box or parallel axes, exactly one (#160).
    /// Written as the one key `polar`, `ternary`, `view_3d` or `parallel`
    /// it has always been written as. Read and set one system through
    /// [`Config::polar`] / [`Config::set_polar`] and their siblings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub coordinates: crate::coordinates::Coordinates,
    /// Overlay layers: secondary Y axes drawn on the right side of the shared
    /// data area, each scaling the series it lists. Empty (key absent in
    /// JSON) is the single-layer chart every document had before the field.
    /// The renderer expands the document into one internal panel per layer
    /// with [`Config::expand`]; see [`OverlayLayer`].
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub overlays: Vec<OverlayLayer>,
    /// More charts drawn in the same document, each with its own chart area
    /// and its own axes; axes are shared only through a subplot's links.
    /// Empty (key absent in JSON) is the single chart every document had
    /// before the field. See [`Subplot`].
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub subplots: Vec<Subplot>,
    /// The layer that receives pointer interaction. `Base` (default, key
    /// absent in JSON) is the chart's own series, axes and decorations. Drawn
    /// pixels do not depend on this field; only hit-map registration and
    /// selection chrome do. An `Overlay` or `Subplot` index past the end of its
    /// list reads as `Base`; a `SubplotOverlay` reads as `Base` when its
    /// subplot is missing and as that subplot when only its overlay is
    /// ([`Config::active_layer`]).
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::layer_ref",
            skip_serializing_if = "LayerRef::is_base"
        )
    )]
    pub active_layer: LayerRef,
    /// The dedicated annotation layer. Default (key absent in JSON) is a
    /// visible, empty layer.
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::annotation_layer",
            skip_serializing_if = "AnnotationLayer::is_default"
        )
    )]
    pub annotations: AnnotationLayer,
    /// How the category bar series of a layer share its categorical axis:
    /// side by side, stacked or stacked to 100 %, the bar direction, the
    /// group width and the gap
    /// ([`BarGroups`](crate::data_config::BarGroups)). `None` (key absent in
    /// JSON) reads as the documented default, resolved by
    /// [`Config::effective_bar_groups`]. It applies to the base layer
    /// and to every overlay, whose category bar series form groups of their
    /// own; a subplot has its own field. Charts without
    /// [`DataRenderType::CategoryBars`](crate::data_config::DataRenderType::CategoryBars)
    /// series ignore it.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub bar_groups: Option<crate::data_config::BarGroups>,
    /// Offset spectra: line series drawn shifted from each other by a step
    /// per member, in stacking order
    /// ([`LineOffsets`](crate::data_config::LineOffsets)). `None` (key
    /// absent in JSON) draws every series where its values are. The members
    /// are series of this chart's base layer or of its overlays; a subplot
    /// has its own field. The renderer draws each member as an internal
    /// panel of its own ([`Config::expand`]); the shift is a display
    /// parameter and reported values stay unshifted.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub line_offsets: Option<crate::data_config::LineOffsets>,
    /// A fill under the data area, below the grid (#203). `None` (key absent
    /// in JSON) leaves the plot transparent, as every chart before the
    /// field. An inset subplot sets it so the parent's grid and data do not
    /// show through.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub plot_background: Option<crate::color::Color>,
    /// A fill over the whole chart area, under everything else the chart
    /// draws (#265). `None` (key absent in JSON) leaves the chart area
    /// transparent, as every chart before the field: the host's clear colour
    /// or an export's clear colour shows through. A subplot fills its own chart area from its own config;
    /// overlay layers draw none. The Milkyway style's deep-space backdrop
    /// takes this slot, as it takes the plot fill's.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub background: Option<crate::color::Color>,
}

impl Config {
    /// Multiply every **pixel-based visual dim** by `scale`. Data ranges, colors,
    /// and scale enums are untouched. For resolution-invariant high-DPI export.
    pub fn scaled(&self, scale: f32) -> Self {
        let mut c = self.clone();
        c.scale_in_place(scale);
        c
    }

    /// [`Self::scaled`] in place: every pixel dimension is multiplied by
    /// `s`; data values, colours, enums and fractions are left as they are.
    pub fn scale_in_place(&mut self, s: f32) {
        // Every field of the chart is named below, so a new one is a decision
        // here: scaled when it is a pixel dimension, written as `name: _` when
        // it is a data value, a colour, an enum or a fraction.
        let Config {
            chart_area,
            top_x,
            bottom_x,
            left_y,
            right_y,
            chart_title,
            grid,
            legend,
            picked_data,
            colorbar,
            draw_style,
            // A ternary switch, a 3D camera (angles, a distance in box
            // half-diagonals, relative lengths) and the polar options hold
            // no pixel dimension; a parallel axis' label offsets are logical
            // pixels, scaled below.
            coordinates,
            overlays,
            subplots,
            // Which layer receives the pointer: not a dimension.
            active_layer: _,
            annotations,
            bar_groups,
            // Steps in data units (or decades) and series ids: not a
            // dimension.
            line_offsets: _,
            plot_background: _,
            background: _,
        } = self;
        // A parallel axis's labels sit to its left with the fixed label gap,
        // as `left_y`'s: its scaled offset makes up the gap as
        // `scale_label_gap` does.
        if let crate::coordinates::Coordinates::Parallel(parallel) = coordinates {
            let extra = (s - 1.0) * crate::layout::LABEL_GAP;
            for axis in &mut parallel.axes {
                axis.label_offset[0] = axis.label_offset[0] * s - extra;
                axis.label_offset[1] *= s;
            }
        }

        let scale_u32 = |v: u32| ((v as f32) * s).round() as u32;
        let ChartArea(Rect {
            x,
            y,
            width,
            height,
        }) = chart_area;
        *x = scale_u32(*x);
        *y = scale_u32(*y);
        *width = scale_u32(*width);
        *height = scale_u32(*height);

        let ChartTitleOptions {
            text,
            visible: _,
            offset_x,
            offset_y,
            top_margin,
        } = chart_title;
        scale_rich_text(text, s);
        *top_margin *= s;
        *offset_x *= s;
        *offset_y *= s;

        // The polar options are angles, an angle unit and a fraction of the
        // outer radius: none of them is a pixel dimension.
        let polar = match coordinates.polar() {
            Some(PolarOptions {
                zero_angle_degrees: _,
                clockwise: _,
                angle_unit: _,
                sector: _,
                inner_radius_fraction: _,
                interpolation: _,
                radial_label_angle_degrees: _,
                // A ring shape, not a pixel dimension.
                grid_shape: _,
                // A switch.
                hide_first_radius_label: _,
            }) => true,
            None => false,
        };
        for (axis, side) in [
            (top_x, Side::Top),
            (bottom_x, Side::Bottom),
            (left_y, Side::Left),
            (right_y, Side::Right),
        ] {
            scale_axis_in_place(axis, s);
            // Ternary and 3D box labels keep a font-relative gap, which
            // scales with the font: no fixed gap to make up for.
            if coordinates.ternary().is_none() && coordinates.view_3d().is_none() {
                scale_label_gap(axis, &side, polar, s);
            }
        }

        let GridOptions {
            show_major_x: _,
            major_x_color: _,
            major_x_width,
            major_x_style: _,
            show_major_y: _,
            major_y_color: _,
            major_y_width,
            major_y_style: _,
            show_minor_x: _,
            minor_x_color: _,
            minor_x_width,
            minor_x_style: _,
            show_minor_y: _,
            minor_y_color: _,
            minor_y_width,
            minor_y_style: _,
        } = grid;
        *major_x_width *= s;
        *major_y_width *= s;
        *minor_x_width *= s;
        *minor_y_width *= s;

        scale_legend_in_place(legend, s);

        // Overlay layers carry their own axis and optional legend; both are
        // pixel-dimensioned like the parent's and scale by the same rules.
        for overlay in overlays {
            let OverlayLayer {
                y_axis,
                // Series ids.
                series: _,
                legend,
            } = overlay;
            scale_axis_in_place(y_axis, s);
            scale_label_gap(y_axis, &Side::Right, false, s);
            if let Some(legend) = legend.as_mut() {
                scale_legend_in_place(legend, s);
            }
        }

        // A subplot is a chart of its own, scaled by the same rules; its
        // chart area scales with the same rounding as the parent's.
        for subplot in subplots {
            let Subplot {
                config,
                // Series ids and link targets.
                series: _,
                link_x: _,
                link_y: _,
                // The marker's line width is pixels.
                marker,
            } = subplot;
            config.scale_in_place(s);
            if let Some(marker) = marker.as_mut() {
                marker.line.width *= s;
            }
        }

        if let Some(picked_data) = picked_data.as_mut() {
            let DataSelectionsConfig {
                visible: _,
                refs: _,
                highlight_color: _,
                outline_width_px,
                point_radius_extra_px,
                contour_width_extra_px,
            } = picked_data;
            *outline_width_px *= s;
            *point_radius_extra_px *= s;
            *contour_width_extra_px *= s;
        }

        // Annotation strokes, text and box padding are pixel dims; their
        // values and anchors are data or plot fractions and stay put.
        annotations.scale_in_place(s);

        // The gap between grouped bars is a pixel dim; the mode, the
        // direction and the group width (a fraction of the slot) are not.
        if let Some(crate::data_config::BarGroups {
            mode: _,
            direction: _,
            group_width: _,
            bar_gap,
        }) = bar_groups.as_mut()
        {
            *bar_gap *= s;
        }

        // The colourbar's axis goes through the same helper as the four chart
        // axes — a new pixel field on `AxisOptions` must not be scalable on the
        // chart axes and silently fixed on the colourbar's. `length_frac` is a
        // fraction of the data area and stays put.
        if let Some(bar) = colorbar.as_mut() {
            let ColorBarOptions {
                visible: _,
                side,
                thickness_px,
                gap_px,
                length_frac: _,
                align: _,
                offset_x,
                offset_y,
                colormap: _,
                nan_color: _,
                border_color: _,
                border_width,
                axis,
            } = bar;
            *thickness_px *= s;
            *gap_px *= s;
            *border_width *= s;
            *offset_x *= s;
            *offset_y *= s;
            scale_axis_in_place(axis, s);
            scale_label_gap(axis, side, false, s);
        }

        // Sketch wobble dims are pixel-based visual dims too. Scaled only in
        // Stylized modes — `Precise` carries no dims, so the precise path
        // sees no change. Milkyway must come out RESOLUTION-INVARIANT:
        // px-sized dims (ribbon width, spread, star size, structure
        // constants via structure_scale) multiply, and `star_density`
        // (stars per arc-PX) divides — the arc itself gains px with the
        // scale, so dividing keeps the star COUNT per data span identical.
        // Without the division a 2× export doubles the stars and the chains
        // bloat into overexposed blobs (field report).
        match draw_style {
            DrawStyle::Sketch(SketchOptions {
                amplitude_px,
                wavelength_px,
                seed: _,
            }) => {
                *amplitude_px *= s;
                *wavelength_px *= s;
            }
            DrawStyle::Milkyway(MilkywayOptions {
                star_density,
                ribbon_width_px,
                ribbon_intensity: _,
                star_scale,
                star_brightness: _,
                spread_px,
                structure_scale,
                faint_bias: _,
                glow: _,
                nebula: _,
                dust: _,
                planet_rim: _,
                seed: _,
            }) => {
                *ribbon_width_px *= s;
                *spread_px *= s;
                *star_scale *= s;
                *structure_scale *= s;
                if s > 0.0 {
                    *star_density /= s;
                }
            }
            DrawStyle::Constellation(ConstellationOptions {
                star_opacity: _,
                line_opacity: _,
            }) => {}
            DrawStyle::Precise => {}
        }
    }
}

/// Scale the pixel sizes of an axis' category text (#203): each rich tick
/// label's font size, and the group headers' font sizes and drag offset.
fn scale_category_text(axis: &mut AxisOptions, s: f32) {
    for category in axis.categories.iter_mut().flatten() {
        if let Some(rich) = &mut category.rich_label {
            rich.font_size *= s;
        }
    }
    if let Some(groups) = &mut axis.category_groups {
        for group in &mut groups.groups {
            group.label.font_size *= s;
        }
        groups.offset = [groups.offset[0] * s, groups.offset[1] * s];
    }
}

/// Scale one axis' pixel-based dims. Shared by the four chart axes and the
/// colourbar's z axis so the set of "which fields are pixels" is written once.
fn scale_axis_in_place(axis: &mut AxisOptions, s: f32) {
    let AxisOptions {
        scale: _,
        min: _,
        max: _,
        major_spacing: _,
        minor_count: _,
        inverted: _,
        label_style,
        tick: _,
        title_option,
        out_margin,
        line_offset,
        line_visible: _,
        line_color: _,
        line_width,
        line_style: _,
        major_tick_length,
        minor_tick_length,
        // Configuration, not pixels.
        // Category text sizes and the group headers' offset are pixels
        // (#203); scaled below.
        categories: _,
        category_groups: _,
        category_window: _,
        // A value of the perpendicular axis.
        cross_at: _,
        // A tick policy, not pixels.
        log_ticks: _,
        // The gap and the mark are in pixels (#249).
        breaks,
        // Shown units, not pixels.
        unit: _,
    } = axis;
    if let Some(AxisBreaks {
        ranges: _,
        style: _,
        gap_px,
        mark,
        frame_each_panel: _,
        // Shares of the axis length, not pixels.
        sizing: _,
    }) = breaks
    {
        *gap_px *= s;
        if let BreakMark::Slash {
            size,
            color: _,
            width,
        } = mark
        {
            *size *= s;
            *width *= s;
        }
    }
    let LabelStyle {
        visible: _,
        color: _,
        font_size,
        label_visible: _,
        label_font: _,
        label_offset_x,
        label_offset_y,
        format: _,
        significant_digits: _,
        clearance: _,
    } = label_style;
    *font_size *= s;
    *label_offset_x *= s;
    *label_offset_y *= s;
    let AxisTitleOptions {
        text,
        visible: _,
        offset_x,
        offset_y,
    } = title_option;
    scale_rich_text(text, s);
    *offset_x *= s;
    *offset_y *= s;
    *out_margin *= s;
    *line_offset *= s;
    *line_width *= s;
    *major_tick_length *= s;
    *minor_tick_length *= s;
    scale_category_text(axis, s);
}

/// Scale one legend's pixel-based dims: the parent's and every overlay's.
fn scale_legend_in_place(legend: &mut crate::legend::Legend, s: f32) {
    let crate::legend::Legend {
        visible: _,
        content,
        corner: _,
        offset_x,
        offset_y,
        padding,
        bg_color: _,
        border_color: _,
    } = legend;
    *offset_x *= s;
    *offset_y *= s;
    *padding *= s;
    scale_rich_text(content, s);
    scale_legend_inset(legend, s);
}

/// The tick-label gap ([`crate::layout::LABEL_GAP`]) is a fixed pixel
/// distance of the shared layout, not a config field, so scaling cannot
/// multiply it. A scaled config moves the labels outward by the extra
/// `(s − 1) × LABEL_GAP` through their offset instead, and lays them out
/// exactly as the 1x config scaled. Polar labels read the offset in their own
/// frame: outward along a spoke for the angle axis, towards the label side
/// of the ray for the radius axis.
fn scale_label_gap(axis: &mut AxisOptions, side: &Side, polar: bool, s: f32) {
    let extra = (s - 1.0) * crate::layout::LABEL_GAP;
    let ls = &mut axis.label_style;
    match side {
        _ if polar && matches!(side, Side::Bottom | Side::Left) => ls.label_offset_y += extra,
        Side::Top => ls.label_offset_y -= extra,
        Side::Bottom => ls.label_offset_y += extra,
        Side::Left => ls.label_offset_x -= extra,
        Side::Right => ls.label_offset_x += extra,
    }
}

/// The legend's inset from its data-area corner
/// ([`crate::layout::LEGEND_INSET`]) is a fixed pixel distance, so a scaled
/// config moves the legend away from its corner by the extra
/// `(s − 1) × LEGEND_INSET` through its offset, as [`scale_label_gap`] does
/// for tick labels.
fn scale_legend_inset(legend: &mut crate::legend::Legend, s: f32) {
    use crate::legend::LegendCorner;
    let extra = (s - 1.0) * crate::layout::LEGEND_INSET;
    let (sx, sy) = match legend.corner {
        LegendCorner::TopLeft => (1.0, 1.0),
        LegendCorner::TopRight => (-1.0, 1.0),
        LegendCorner::BottomLeft => (1.0, -1.0),
        LegendCorner::BottomRight => (-1.0, -1.0),
    };
    legend.offset_x += sx * extra;
    legend.offset_y += sy * extra;
}

/// Scale a `RichText`'s pixel-based dims: the document-level `font_size` and
/// every per-segment `font_size` override.
fn scale_rich_text(rt: &mut RichText, s: f32) {
    rt.font_size *= s;
    for seg in &mut rt.segments {
        if let Some(size) = seg.font_size.as_mut() {
            *size *= s;
        }
    }
}

// z -> colour normalization. This is the formula the CPU strip and the GPU field
// sampler share, so its edges are contracts, not incidental behaviour.
#[cfg(test)]
mod colorbar_normalization_tests {
    use crate::color::Color;
    use crate::colormap::ColorMap;
    use crate::config::AxisScale;

    #[test]
    fn a_linear_bar_normalizes_and_clamps() {
        let mut bar = crate::defaults::color_bar_options();
        bar.axis.min = -50.0;
        bar.axis.max = 150.0;

        assert_eq!(bar.normalized_z(-50.0), Some(0.0));
        assert_eq!(bar.normalized_z(150.0), Some(1.0));
        assert_eq!(bar.normalized_z(50.0), Some(0.5));
        // Outside the range is the endpoint colour, not an out-of-range index.
        assert_eq!(bar.normalized_z(-1000.0), Some(0.0));
        assert_eq!(bar.normalized_z(1000.0), Some(1.0));
    }

    #[test]
    fn a_logarithmic_bar_normalizes_in_decades_and_rejects_non_positive() {
        let mut bar = crate::defaults::color_bar_options();
        bar.axis.scale = AxisScale::Logarithmic;
        bar.axis.min = 1.0e-2;
        bar.axis.max = 1.0e2;

        assert_eq!(bar.normalized_z(1.0e-2), Some(0.0));
        assert_eq!(bar.normalized_z(1.0e2), Some(1.0));
        assert_eq!(bar.normalized_z(1.0), Some(0.5));

        // No logarithm to take. These are `nan_color`'s job — clamping them to
        // the low end would draw them as "smallest", which they are not.
        assert_eq!(bar.normalized_z(0.0), None);
        assert_eq!(bar.normalized_z(-1.0), None);
    }

    #[test]
    fn unplaceable_values_and_ranges_yield_none() {
        let mut bar = crate::defaults::color_bar_options();
        bar.axis.min = 0.0;
        bar.axis.max = 1.0;
        assert_eq!(bar.normalized_z(f64::NAN), None);
        assert_eq!(bar.normalized_z(f64::INFINITY), None);

        // A degenerate range has no ramp position for anything.
        bar.axis.max = bar.axis.min;
        assert_eq!(bar.normalized_z(0.0), None);

        bar.axis.max = f64::NAN;
        assert_eq!(bar.normalized_z(0.0), None);
    }

    #[test]
    fn color_for_z_falls_back_to_nan_color() {
        let mut bar = crate::defaults::color_bar_options();
        bar.axis.scale = AxisScale::Logarithmic;
        bar.axis.min = 1.0;
        bar.axis.max = 100.0;
        bar.colormap = ColorMap::GrayScale;
        bar.nan_color = Color::from_rgb8(255, 0, 255);

        assert_eq!(bar.color_for_z(1.0), ColorMap::GrayScale.sample(0.0));
        assert_eq!(bar.color_for_z(100.0), ColorMap::GrayScale.sample(1.0));
        assert_eq!(bar.color_for_z(0.0), bar.nan_color);
        assert_eq!(bar.color_for_z(f64::NAN), bar.nan_color);
    }

    // `inverted` is screen layout, not colour assignment.
    #[test]
    fn inverting_the_axis_does_not_reverse_the_ramp() {
        let mut bar = crate::defaults::color_bar_options();
        bar.axis.min = 0.0;
        bar.axis.max = 10.0;
        let upright = bar.normalized_z(2.5);
        bar.axis.inverted = true;
        assert_eq!(bar.normalized_z(2.5), upright);
    }
}

// High-DPI export invariance for the colourbar. `scaled(s)` must multiply every
// pixel dim it owns — including the ones inside its axis, which is why the axis
// scaling is a shared helper and not a loop over the four chart axes.
#[cfg(test)]
mod colorbar_scale_tests {

    #[test]
    fn scaling_a_config_scales_the_colorbar_pixels_but_not_its_fractions() {
        let mut cfg = crate::defaults::config();
        cfg.colorbar = Some(crate::defaults::color_bar_options());
        let before = cfg.colorbar.clone().expect("colourbar");

        let scaled = cfg.scaled(2.0);
        let after = scaled.colorbar.as_ref().expect("colourbar");

        assert_eq!(after.thickness_px, before.thickness_px * 2.0);
        assert_eq!(after.gap_px, before.gap_px * 2.0);
        assert_eq!(after.border_width, before.border_width * 2.0);
        assert_eq!(after.axis.out_margin, before.axis.out_margin * 2.0);
        assert_eq!(
            after.axis.major_tick_length,
            before.axis.major_tick_length * 2.0
        );
        assert_eq!(
            after.axis.minor_tick_length,
            before.axis.minor_tick_length * 2.0
        );
        assert_eq!(after.axis.line_width, before.axis.line_width * 2.0);
        assert_eq!(
            after.axis.label_style.font_size,
            before.axis.label_style.font_size * 2.0
        );

        // Fractions, ranges, and colours are not pixels.
        assert_eq!(after.length_frac, before.length_frac);
        assert_eq!(after.axis.min, before.axis.min);
        assert_eq!(after.axis.max, before.axis.max);
        assert_eq!(after.colormap, before.colormap);
        assert_eq!(after.nan_color, before.nan_color);
    }

    // The colourbar's axis must scale by exactly the same rule as a chart axis
    // on the same side: if the two ever diverge, a high-DPI export puts the
    // bar's labels at a different size or gap than the axis labels beside them.
    #[test]
    fn the_colorbar_axis_scales_like_a_chart_axis() {
        let mut cfg = crate::defaults::config();
        let mut bar = crate::defaults::color_bar_options();
        assert_eq!(bar.side, crate::layout::Side::Right);
        bar.axis = cfg.right_y.clone();
        cfg.colorbar = Some(bar);

        let scaled = cfg.scaled(1.75);
        assert_eq!(scaled.colorbar.expect("colourbar").axis, scaled.right_y);
    }

    // The fixed tick-label gap and legend inset are layout constants, not
    // fields, so a scaled config moves labels and legend by the extra gap and
    // lays them out as the 1x config scaled.
    #[test]
    fn scaling_keeps_the_fixed_label_gap_and_legend_inset_proportional() {
        use crate::layout::{LABEL_GAP, LEGEND_INSET};
        let cfg = crate::defaults::config();
        let scaled = cfg.scaled(2.0);
        for (before, after, outward) in [
            (&cfg.top_x, &scaled.top_x, (0.0, -1.0)),
            (&cfg.bottom_x, &scaled.bottom_x, (0.0, 1.0)),
            (&cfg.left_y, &scaled.left_y, (-1.0, 0.0)),
            (&cfg.right_y, &scaled.right_y, (1.0, 0.0)),
        ] {
            let ls = (&before.label_style, &after.label_style);
            assert_eq!(
                ls.1.label_offset_x,
                ls.0.label_offset_x * 2.0 + outward.0 * LABEL_GAP
            );
            assert_eq!(
                ls.1.label_offset_y,
                ls.0.label_offset_y * 2.0 + outward.1 * LABEL_GAP
            );
        }
        let (sx, sy) = match cfg.legend.corner {
            crate::legend::LegendCorner::TopLeft => (1.0, 1.0),
            crate::legend::LegendCorner::TopRight => (-1.0, 1.0),
            crate::legend::LegendCorner::BottomLeft => (1.0, -1.0),
            crate::legend::LegendCorner::BottomRight => (-1.0, -1.0),
        };
        assert_eq!(
            scaled.legend.offset_x,
            cfg.legend.offset_x * 2.0 + sx * LEGEND_INSET
        );
        assert_eq!(
            scaled.legend.offset_y,
            cfg.legend.offset_y * 2.0 + sy * LEGEND_INSET
        );
        // At 1x nothing moves.
        assert_eq!(cfg.scaled(1.0), cfg);
    }
}

// Legend types live in `crate::legend`; re-exported here so existing
// `config::Legend…` paths keep working.
pub use crate::legend::{
    Legend, LegendCorner, LegendEntryKind, append_legend_entry, append_legend_entry_rich,
    legend_entry_count, legend_entry_of_series, legend_entry_series_id, remove_legend_entry,
    scatter_shape_char, series_symbol_segments, set_legend_entry_label, symbol_segments,
    update_legend_symbols_preserving_text,
};

// The wire form of `Config`: every field added after a release is additive
// schema, absent at its default so a document written before it parses and
// re-serializes byte-identically, and round-trips when set. `draw_style` is
// the pattern: `Precise` must not serialize, the enum is internally tagged
// (`"mode"`) so a style's parameters sit INLINE next to the tag — the same
// discipline as `RichSegment`'s per-segment overrides (text.rs).
//
// One test runs every check in turn (#47 §3): the renderer's
// `released_documents.rs` holds the fixture documents, so this module keeps
// the model's own coverage without a copy of them per field. Each check is a
// plain function so a failure names it.
#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::{Config, DataSelectionsConfig, DrawStyle, PickedDataRef, SketchOptions};

    /// Serialized default config — `draw_style` is `Precise`, so the JSON has
    /// no `"draw_style"` key: exactly the shape of pre-sketch documents.
    fn default_config_json() -> serde_json::Value {
        serde_json::to_value(crate::defaults::config()).expect("serialize Config")
    }

    // `line_offsets` is additive: absent at its default, so an older document
    // parses and re-serializes byte-identically; `step_x` is omitted at zero;
    // set, it round-trips, on the chart and on a subplot's chart.
    fn line_offsets_round_trip_and_old_documents_unchanged() {
        use crate::data_config::LineOffsets;
        let json = default_config_json();
        assert!(json.get("line_offsets").is_none(), "{json}");
        let text = serde_json::to_string(&crate::defaults::config()).unwrap();
        let old: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(old.line_offsets, None);
        assert_eq!(serde_json::to_string(&old).unwrap(), text);

        let mut cfg = crate::defaults::config();
        cfg.line_offsets = Some(LineOffsets {
            series: vec!["a".into(), "b".into()],
            step_y: 0.5,
            step_x: 0.0,
        });
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(
            json["line_offsets"],
            serde_json::json!({ "series": ["a", "b"], "step_y": 0.5 })
        );
        assert_eq!(serde_json::from_value::<Config>(json).unwrap(), cfg);
        cfg.line_offsets.as_mut().unwrap().step_x = -0.25;
        let mut subplot = crate::defaults::subplot();
        subplot.config.line_offsets = cfg.line_offsets.clone();
        cfg.subplots.push(subplot);
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["line_offsets"]["step_x"], serde_json::json!(-0.25));
        assert_eq!(
            json["subplots"][0]["config"]["line_offsets"],
            json["line_offsets"]
        );
        assert_eq!(serde_json::from_value::<Config>(json).unwrap(), cfg);
        // A partial group reads the missing keys as their defaults.
        let partial: LineOffsets = serde_json::from_str(r#"{"series":["a"]}"#).unwrap();
        assert_eq!(partial.step_y, 0.0);
        // A scaled export keeps the steps: they are data units.
        assert_eq!(cfg.scaled(2.0).line_offsets, cfg.line_offsets);
    }

    // The layer fields are additive schema in the same way: `active_layer`,
    // `annotations` and `overlays` are absent at their defaults (so every
    // document written before them is byte-identical), parse to the defaults
    // when missing, and round-trip when set. `active_layer` and `annotations`
    // also exist on the radial chart and the legacy documents.
    #[allow(deprecated)]
    fn layer_fields_default_and_are_omitted_from_json() {
        use crate::LayerRef;
        let json = default_config_json();
        assert!(json.get("active_layer").is_none(), "{json}");
        assert!(json.get("annotations").is_none(), "{json}");
        assert!(json.get("overlays").is_none(), "{json}");
        let cfg: Config = serde_json::from_value(json).expect("pre-layer document parses");
        assert_eq!(cfg.active_layer, LayerRef::Base);
        assert_eq!(cfg.annotations, crate::defaults::annotation_layer());

        let mut cfg = crate::defaults::config();
        cfg.active_layer = LayerRef::Overlay(1);
        cfg.annotations.visible = false;
        cfg.overlays.push(crate::layer::OverlayLayer {
            series: vec!["b".into()],
            ..crate::defaults::overlay_layer()
        });
        cfg.overlays.push(crate::defaults::overlay_layer());
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["active_layer"], serde_json::json!({ "Overlay": 1 }));
        assert_eq!(json["annotations"], serde_json::json!({ "visible": false }));
        assert_eq!(json["overlays"].as_array().map(Vec::len), Some(2));
        assert_eq!(json["overlays"][0]["series"], serde_json::json!(["b"]));
        let back: Config = serde_json::from_value(json).unwrap();
        assert_eq!(back, cfg);

        for (name, json) in [
            (
                "radial",
                serde_json::to_value(crate::defaults::radial_chart()).unwrap(),
            ),
            (
                "categorical",
                serde_json::to_value(crate::defaults::categorical_chart()).unwrap(),
            ),
            (
                "boxplot",
                serde_json::to_value(crate::defaults::box_plot_chart()).unwrap(),
            ),
            (
                "radar",
                serde_json::to_value(crate::defaults::radar_chart()).unwrap(),
            ),
        ] {
            assert!(json.get("active_layer").is_none(), "{name}: {json}");
            assert!(json.get("annotations").is_none(), "{name}: {json}");
        }
        let mut radial = crate::defaults::radial_chart();
        radial.active_layer = LayerRef::Annotations;
        let json = serde_json::to_value(&radial).unwrap();
        assert_eq!(json["active_layer"], serde_json::json!("Annotations"));
        let back: crate::radial::RadialChart = serde_json::from_value(json).unwrap();
        assert_eq!(back, radial);
    }

    /// The default document is the shape of documents written before
    /// `draw_style` and `picked_data` existed: `Precise` is
    /// not serialized, and each missing key parses to its default.
    fn a_document_without_the_additive_keys_parses_to_their_defaults() {
        let json = default_config_json();
        assert!(
            json.get("draw_style").is_none(),
            "Precise draw_style must be skipped in serialization: {json}"
        );
        let cfg: Config = serde_json::from_value(json)
            .expect("pre-sketch, pre-picker and pre-data-pick document parses");
        assert_eq!(cfg.draw_style, DrawStyle::Precise);
        assert_eq!(cfg.picked_data, None);
    }

    fn sketch_tag_alone_yields_all_defaults() {
        let mut json = default_config_json();
        json.as_object_mut()
            .unwrap()
            .insert("draw_style".into(), serde_json::json!({ "mode": "sketch" }));
        let cfg: Config = serde_json::from_value(json).expect("tag-only sketch object parses");
        let s = *cfg.draw_style.sketch().expect("sketch enabled");
        assert_eq!(s, crate::defaults::sketch_options());
        // Pin the stock values themselves, not just Default == Default.
        assert_eq!(s.amplitude_px, 1.5);
        assert_eq!(s.wavelength_px, 60.0);
        assert_eq!(s.seed, 0);
    }

    fn partial_sketch_fields_fill_remaining_defaults() {
        let mut json = default_config_json();
        json.as_object_mut().unwrap().insert(
            "draw_style".into(),
            serde_json::json!({ "mode": "sketch", "seed": 7 }),
        );
        let cfg: Config = serde_json::from_value(json).expect("partial sketch object parses");
        assert_eq!(
            cfg.draw_style,
            DrawStyle::Sketch(SketchOptions {
                seed: 7,
                ..crate::defaults::sketch_options()
            })
        );
    }

    fn sketch_round_trips_with_inline_fields() {
        let mut cfg = crate::defaults::config();
        cfg.draw_style = DrawStyle::Sketch(SketchOptions {
            amplitude_px: 2.5,
            wavelength_px: 42.0,
            seed: 9,
        });
        let json = serde_json::to_value(&cfg).expect("serialize Config");
        // Internally tagged: the SketchOptions fields are inline siblings of
        // `"mode"`, not a nested object.
        let ds = json.get("draw_style").expect("draw_style key present");
        assert_eq!(ds.get("mode"), Some(&serde_json::json!("sketch")));
        assert_eq!(ds.get("amplitude_px"), Some(&serde_json::json!(2.5)));
        assert_eq!(ds.get("wavelength_px"), Some(&serde_json::json!(42.0)));
        assert_eq!(ds.get("seed"), Some(&serde_json::json!(9)));
        let back: Config = serde_json::from_value(json).expect("round-trip parses");
        assert_eq!(back.draw_style, cfg.draw_style);
    }

    fn explicit_precise_mode_deserializes_to_precise() {
        let mut json = default_config_json();
        json.as_object_mut().unwrap().insert(
            "draw_style".into(),
            serde_json::json!({ "mode": "precise" }),
        );
        let cfg: Config = serde_json::from_value(json).expect("explicit precise parses");
        assert_eq!(cfg.draw_style, DrawStyle::Precise);
    }

    fn legacy_picked_points_empty_object_yields_the_default_selection() {
        let mut json = default_config_json();
        json.as_object_mut()
            .unwrap()
            .insert("picked_points".into(), serde_json::json!({}));
        let cfg: Config = serde_json::from_value(json).expect("tag-only picked_points parses");
        assert_eq!(
            cfg.picked_data,
            Some(crate::defaults::data_selections_config())
        );
        let written = serde_json::to_value(&cfg).unwrap();
        assert!(written.get("picked_points").is_none(), "{written}");
    }

    /// A document written before 0.19 with only `picked_points`: its points
    /// become point refs drawn in its ring's look.
    fn legacy_picked_points_alone_become_the_selection() {
        let mut json = default_config_json();
        json.as_object_mut().unwrap().insert(
            "picked_points".into(),
            serde_json::json!({
                "visible": false,
                "refs": [
                    {"series_id": "series-a", "point_index": 2},
                    {"source_id": "source-a", "series_id": "series-b", "point_index": 4}
                ],
                "ring_color": {"r": 0.0, "g": 1.0, "b": 0.0, "a": 1.0},
                "ring_width_px": 3.0,
                "radius_extra_px": 2.5
            }),
        );
        let cfg: Config = serde_json::from_value(json).expect("legacy picks parse");
        assert_eq!(
            cfg.picked_data,
            Some(DataSelectionsConfig {
                visible: false,
                refs: vec![
                    PickedDataRef::Point {
                        source_id: None,
                        series_id: "series-a".into(),
                        point_index: 2,
                    },
                    PickedDataRef::Point {
                        source_id: Some("source-a".into()),
                        series_id: "series-b".into(),
                        point_index: 4,
                    },
                ],
                highlight_color: crate::color::Color::from_rgba(0.0, 1.0, 0.0, 1.0),
                outline_width_px: 3.0,
                point_radius_extra_px: 2.5,
                ..crate::defaults::data_selections_config()
            })
        );
    }

    /// Beside `picked_data`, the shown legacy points join its refs in its
    /// look; hidden ones are left out.
    fn legacy_picked_points_join_picked_data() {
        let legacy = |visible: bool| {
            let mut json = default_config_json();
            let object = json.as_object_mut().unwrap();
            object.insert(
                "picked_points".into(),
                serde_json::json!({
                    "visible": visible,
                    "refs": [{"series_id": "line", "point_index": 4}],
                    "ring_width_px": 9.0
                }),
            );
            object.insert(
                "picked_data".into(),
                serde_json::json!({
                    "refs": [{"kind": "histogram_bin", "series_id": "hist", "bin_index": 2}]
                }),
            );
            serde_json::from_value::<Config>(json)
                .expect("both selections parse")
                .picked_data
                .expect("one selection")
        };
        let shown = legacy(true);
        assert_eq!(shown.outline_width_px, 2.0, "picked_data keeps its look");
        assert_eq!(
            shown.refs,
            vec![
                PickedDataRef::HistogramBin {
                    source_id: None,
                    series_id: "hist".into(),
                    bin_index: 2,
                    half: None,
                },
                PickedDataRef::Point {
                    source_id: None,
                    series_id: "line".into(),
                    point_index: 4,
                },
            ]
        );
        assert_eq!(legacy(false).refs.len(), 1);
    }

    #[test]
    fn scale_scales_picked_data_pixel_dimensions() {
        let mut cfg = crate::defaults::config();
        cfg.picked_data = Some(crate::defaults::data_selections_config());
        cfg.scale_in_place(2.0);
        let picked = cfg.picked_data.expect("picked data");
        assert_eq!(picked.outline_width_px, 4.0);
        assert_eq!(picked.point_radius_extra_px, 6.0);
        assert_eq!(picked.contour_width_extra_px, 4.0);
    }

    fn picked_data_empty_object_yields_defaults() {
        let mut json = default_config_json();
        json.as_object_mut()
            .unwrap()
            .insert("picked_data".into(), serde_json::json!({}));
        let cfg: Config = serde_json::from_value(json).expect("empty picked_data parses");
        assert_eq!(
            cfg.picked_data,
            Some(crate::defaults::data_selections_config())
        );
    }

    fn typed_picked_data_round_trips_with_minimal_provenance() {
        let mut cfg = crate::defaults::config();
        cfg.picked_data = Some(DataSelectionsConfig {
            refs: vec![
                PickedDataRef::Point {
                    source_id: None,
                    series_id: "points".into(),
                    point_index: 3,
                },
                PickedDataRef::HistogramBin {
                    source_id: Some("source-a".into()),
                    series_id: "hist".into(),
                    bin_index: 4,
                    half: None,
                },
                PickedDataRef::MatrixCell {
                    source_id: None,
                    series_id: "heat".into(),
                    x_index: 5,
                    y_index: 6,
                },
                PickedDataRef::ContourLevel {
                    source_id: None,
                    series_id: "contour".into(),
                    level_index: 2,
                    x_index: 7,
                    y_index: 8,
                },
            ],
            ..crate::defaults::data_selections_config()
        });

        let json = serde_json::to_value(&cfg).expect("serialize typed selections");
        assert_eq!(json["picked_data"]["refs"][0]["kind"], "point");
        assert!(json["picked_data"]["refs"][0].get("source_id").is_none());
        assert_eq!(json["picked_data"]["refs"][1]["kind"], "histogram_bin");
        assert_eq!(json["picked_data"]["refs"][2]["kind"], "matrix_cell");
        assert_eq!(json["picked_data"]["refs"][3]["kind"], "contour_level");
        let back: Config = serde_json::from_value(json).expect("parse typed selections");
        assert_eq!(back.picked_data, cfg.picked_data);
    }

    #[test]
    fn scale_scales_typed_selection_pixel_dimensions() {
        let mut cfg = crate::defaults::config();
        cfg.picked_data = Some(crate::defaults::data_selections_config());
        cfg.scale_in_place(2.0);
        let picked = cfg.picked_data.expect("picked data");
        assert_eq!(picked.outline_width_px, 4.0);
        assert_eq!(picked.point_radius_extra_px, 6.0);
        assert_eq!(picked.contour_width_extra_px, 4.0);
    }

    fn constellation_tag_alone_yields_all_defaults() {
        let mut json = default_config_json();
        json["draw_style"] = serde_json::json!({ "mode": "constellation" });
        let cfg: Config = serde_json::from_value(json).expect("tag-only constellation parses");
        assert_eq!(
            cfg.draw_style,
            super::DrawStyle::Constellation(crate::defaults::constellation_options()),
        );
    }

    fn milkyway_tag_alone_yields_all_defaults() {
        let mut json = default_config_json();
        json["draw_style"] = serde_json::json!({ "mode": "milkyway" });
        let cfg: Config = serde_json::from_value(json).expect("tag-only milkyway parses");
        assert_eq!(
            cfg.draw_style,
            super::DrawStyle::Milkyway(crate::defaults::milkyway_options()),
        );
    }

    /// PARAM_SPECS defaults are literals (const context) — pin them to the
    /// `Default` impls so the two sources cannot drift, and pin every spec
    /// key to a real serde field by writing it through `draw_style` JSON.
    fn param_specs_match_defaults_and_serde_fields() {
        use super::{ConstellationOptions, MilkywayOptions, SketchOptions};

        let check = |mode: &str, specs: &[super::StyleParamSpec], defaults: serde_json::Value| {
            for s in specs {
                let d = defaults
                    .get(s.key)
                    .unwrap_or_else(|| panic!("{mode}: spec key {} is not a serde field", s.key))
                    .as_f64()
                    .expect("numeric field");
                assert!(
                    (d - s.default).abs() < 1e-6,
                    "{mode}.{}: spec default {} != Default impl {}",
                    s.key,
                    s.default,
                    d
                );
                assert!(
                    s.min <= s.default && s.default <= s.max,
                    "{mode}.{}: default outside range",
                    s.key
                );

                // Round-trip the key through the tagged enum to prove it is
                // accepted (a typo'd key would be silently ignored). Integer
                // specs must be written as JSON integers — u32 fields reject
                // a float literal.
                let mut style = serde_json::json!({ "mode": mode });
                style[s.key] = if s.integer {
                    serde_json::json!(s.max as i64)
                } else {
                    serde_json::json!(s.max)
                };
                let parsed: super::DrawStyle =
                    serde_json::from_value(style).expect("spec key parses in draw_style");
                let back = serde_json::to_value(parsed).expect("serialize");
                let v = back
                    .get(s.key)
                    .expect("key survives round trip")
                    .as_f64()
                    .unwrap();
                assert!(
                    (v - s.max).abs() < 1e-4,
                    "{mode}.{}: value did not stick",
                    s.key
                );
            }
        };

        check(
            "sketch",
            SketchOptions::PARAM_SPECS,
            serde_json::to_value(crate::defaults::sketch_options()).unwrap(),
        );
        check(
            "milkyway",
            MilkywayOptions::PARAM_SPECS,
            serde_json::to_value(crate::defaults::milkyway_options()).unwrap(),
        );
        check(
            "constellation",
            ConstellationOptions::PARAM_SPECS,
            serde_json::to_value(crate::defaults::constellation_options()).unwrap(),
        );
        assert_eq!(super::DrawStyle::param_specs_for_mode("nope"), None);
        assert_eq!(
            super::DrawStyle::param_specs_for_mode("precise"),
            Some(&[][..])
        );
    }

    fn constellation_round_trips_with_inline_fields() {
        let mut cfg = crate::defaults::config();
        cfg.draw_style = super::DrawStyle::Constellation(super::ConstellationOptions {
            star_opacity: 0.75,
            line_opacity: 0.25,
        });
        let json = serde_json::to_value(&cfg).expect("serialize");
        assert_eq!(json["draw_style"]["mode"], "constellation");
        assert_eq!(json["draw_style"]["star_opacity"], 0.75);
        assert_eq!(json["draw_style"]["line_opacity"], 0.25);
        let back: Config = serde_json::from_value(json).expect("parse back");
        assert_eq!(back.draw_style, cfg.draw_style);
    }

    fn milkyway_round_trips_with_inline_fields() {
        let mut cfg = crate::defaults::config();
        cfg.draw_style = super::DrawStyle::Milkyway(super::MilkywayOptions {
            star_density: 22.0,
            ribbon_width_px: 20.0,
            ribbon_intensity: 0.4,
            star_scale: 1.3,
            star_brightness: 1.8,
            spread_px: 3.5,
            structure_scale: 1.25,
            faint_bias: 4.5,
            glow: 0.8,
            nebula: 0.7,
            dust: 1.5,
            planet_rim: 0.5,
            seed: 9,
        });
        let json = serde_json::to_value(&cfg).expect("serialize");
        assert_eq!(json["draw_style"]["mode"], "milkyway");
        assert_eq!(json["draw_style"]["star_density"], 22.0);
        let back: Config = serde_json::from_value(json).expect("parse back");
        assert_eq!(back.draw_style, cfg.draw_style);
    }

    /// The one serde test of this file: the additive keys of `Config` and its
    /// draw styles, selections and categorical axes, in turn.
    #[test]
    fn serde_round_trips_the_additive_keys_and_parses_old_documents() {
        line_offsets_round_trip_and_old_documents_unchanged();
        layer_fields_default_and_are_omitted_from_json();
        a_document_without_the_additive_keys_parses_to_their_defaults();
        sketch_tag_alone_yields_all_defaults();
        partial_sketch_fields_fill_remaining_defaults();
        sketch_round_trips_with_inline_fields();
        explicit_precise_mode_deserializes_to_precise();
        legacy_picked_points_empty_object_yields_the_default_selection();
        legacy_picked_points_alone_become_the_selection();
        legacy_picked_points_join_picked_data();
        picked_data_empty_object_yields_defaults();
        typed_picked_data_round_trips_with_minimal_provenance();
        constellation_tag_alone_yields_all_defaults();
        milkyway_tag_alone_yields_all_defaults();
        param_specs_match_defaults_and_serde_fields();
        constellation_round_trips_with_inline_fields();
        milkyway_round_trips_with_inline_fields();
        super::category_axis_tests::categories_are_additive_on_the_wire();
    }
}

// The categorical axis (`AxisOptions::categories`): its limits, its range
// and walk, and its additive wire form.
#[cfg(test)]
mod category_axis_tests {
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
    pub(super) fn categories_are_additive_on_the_wire() {
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

/// The document form of [`Config::picked_data`]: the `picked_data` key, and
/// on reading also the `picked_points` key documents wrote before 0.19.
#[cfg(feature = "serde")]
mod picked_data_wire {
    use super::{Color, DataSelectionsConfig, PickedDataRef};

    /// The point selection documents wrote before 0.19, read only to be
    /// folded into `picked_data`.
    #[derive(serde::Deserialize)]
    #[serde(default)]
    struct LegacyPickedPoints {
        visible: bool,
        refs: Vec<LegacyPointRef>,
        ring_color: Color,
        ring_width_px: f32,
        radius_extra_px: f32,
    }

    impl Default for LegacyPickedPoints {
        fn default() -> Self {
            let highlight = DataSelectionsConfig::empty();
            Self {
                visible: highlight.visible,
                refs: Vec::new(),
                ring_color: highlight.highlight_color,
                ring_width_px: highlight.outline_width_px,
                radius_extra_px: highlight.point_radius_extra_px,
            }
        }
    }

    #[derive(serde::Deserialize)]
    struct LegacyPointRef {
        #[serde(default)]
        source_id: Option<String>,
        series_id: String,
        point_index: usize,
    }

    #[derive(serde::Deserialize)]
    struct Wire {
        #[serde(default)]
        picked_points: Option<LegacyPickedPoints>,
        #[serde(default)]
        picked_data: Option<DataSelectionsConfig>,
    }

    #[derive(serde::Serialize)]
    struct WireRef<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        picked_data: Option<&'a DataSelectionsConfig>,
    }

    pub(super) fn serialize<S: serde::Serializer>(
        picked_data: &Option<DataSelectionsConfig>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(
            &WireRef {
                picked_data: picked_data.as_ref(),
            },
            serializer,
        )
    }

    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<DataSelectionsConfig>, D::Error> {
        let Wire {
            picked_points,
            picked_data,
        } = <Wire as serde::Deserialize>::deserialize(deserializer)?;
        let Some(legacy) = picked_points else {
            return Ok(picked_data);
        };
        let points = legacy.refs.into_iter().map(|point| PickedDataRef::Point {
            source_id: point.source_id,
            series_id: point.series_id,
            point_index: point.point_index,
        });
        Ok(Some(match picked_data {
            // The typed selection keeps its own look; the points join it
            // when they were shown.
            Some(mut selection) => {
                if legacy.visible {
                    selection.refs.extend(points);
                }
                selection
            }
            None => DataSelectionsConfig {
                visible: legacy.visible,
                refs: points.collect(),
                highlight_color: legacy.ring_color,
                outline_width_px: legacy.ring_width_px,
                point_radius_extra_px: legacy.radius_extra_px,
                // The legacy points had no contour to widen.
                contour_width_extra_px: DataSelectionsConfig::empty().contour_width_extra_px,
            },
        }))
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
                    series: Vec::new(),
                    legend: None,
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

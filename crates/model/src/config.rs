use crate::color::Color;
use crate::colormap::ColorMap;
use crate::format::LabelFormat;
use crate::layout::{ChartArea, Side};
use crate::line::LineStylePreset;
use crate::text::RichText;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChartTitleOptions {
    pub text: RichText,
    pub visible: bool,
    pub offset_x: f32,
    pub offset_y: f32,
    pub top_margin: f32,
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
}

/// Hand-drawn ("sketch") render-mode parameters. Every field has a default —
/// JSON `{"mode":"sketch"}` alone enables the mode with stock values
/// (`serde(default)`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct SketchOptions {
    /// Perpendicular path-displacement amplitude, in px. Default 1.5.
    pub amplitude_px: f32,
    /// Displacement wavelength, in px — one wobble per this arc length along
    /// the path. Default 60.0.
    pub wavelength_px: f32,
    /// Global seed. Same (config, data) → byte-identical output. Default 0.
    pub seed: u32,
}

impl Default for SketchOptions {
    fn default() -> Self {
        Self {
            amplitude_px: 1.5,
            wavelength_px: 60.0,
            seed: 0,
        }
    }
}

/// Milkyway draw-style parameters. All fields have defaults — JSON
/// `{"mode":"milkyway"}` alone works. Lines render as star chains over
/// a soft nebula ribbon tinted with the series `line_color` (that ribbon is
/// what keeps multiple series distinguishable); star counts stay uniform per
/// arc length, while star colors stay physical (blackbody locus, population
/// mix follows the clump field).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
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

impl Default for MilkywayOptions {
    fn default() -> Self {
        Self {
            star_density: 14.0,
            ribbon_width_px: 14.0,
            ribbon_intensity: 0.30,
            star_scale: 1.0,
            star_brightness: 1.0,
            spread_px: 2.5,
            structure_scale: 1.0,
            faint_bias: 3.0,
            glow: 0.55,
            nebula: 1.0,
            dust: 1.0,
            planet_rim: 0.34,
            seed: 0,
        }
    }
}

/// Constellation draw-style parameters. All fields have defaults. Only
/// `ScatterLine` series are drawn: PSF-rendered stars sit at the scatter data
/// points, connected by a translucent line.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ConstellationOptions {
    /// Opacity applied to the point stars. Default 1.0.
    pub star_opacity: f32,
    /// Opacity applied to the connecting line. Default 0.45.
    pub line_opacity: f32,
}

impl Default for ConstellationOptions {
    fn default() -> Self {
        Self {
            star_opacity: 1.0,
            line_opacity: 0.45,
        }
    }
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
#[derive(Clone, Copy, Debug, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "mode", rename_all = "snake_case"))]
pub enum DrawStyle {
    #[default]
    Precise,
    Sketch(SketchOptions),
    Milkyway(MilkywayOptions),
    Constellation(ConstellationOptions),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PickedPointRef {
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub source_id: Option<String>,
    pub series_id: String,
    pub point_index: usize,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct PickedPointsConfig {
    pub visible: bool,
    pub refs: Vec<PickedPointRef>,
    pub ring_color: Color,
    pub ring_width_px: f32,
    pub radius_extra_px: f32,
}

impl Default for PickedPointsConfig {
    fn default() -> Self {
        Self {
            visible: true,
            refs: Vec::new(),
            ring_color: Color::from_rgb8(255, 215, 0),
            ring_width_px: 2.0,
            radius_extra_px: 3.0,
        }
    }
}

/// A renderer-independent reference to one selected data primitive.
///
/// These records deliberately carry provenance only. Coordinates, bin edges,
/// matrix values, and contour values remain in the caller's registered source
/// columns / series configuration; the renderer never creates a CPU shadow of
/// GPU-owned data just to remember a selection.
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
}

impl PickedDataRef {
    pub fn source_id(&self) -> Option<&str> {
        match self {
            Self::Point { source_id, .. }
            | Self::HistogramBin { source_id, .. }
            | Self::MatrixCell { source_id, .. }
            | Self::ContourLevel { source_id, .. } => source_id.as_deref(),
        }
    }

    pub fn series_id(&self) -> &str {
        match self {
            Self::Point { series_id, .. }
            | Self::HistogramBin { series_id, .. }
            | Self::MatrixCell { series_id, .. }
            | Self::ContourLevel { series_id, .. } => series_id,
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
#[cfg_attr(feature = "serde", serde(default))]
pub struct DataSelectionsConfig {
    pub visible: bool,
    pub refs: Vec<PickedDataRef>,
    pub highlight_color: Color,
    pub outline_width_px: f32,
    pub point_radius_extra_px: f32,
    pub contour_width_extra_px: f32,
}

impl Default for DataSelectionsConfig {
    fn default() -> Self {
        Self {
            visible: true,
            refs: Vec::new(),
            highlight_color: Color::from_rgb8(255, 215, 0),
            outline_width_px: 2.0,
            point_radius_extra_px: 3.0,
            contour_width_extra_px: 2.0,
        }
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
/// them in from matrix statistics, the way [`AxisOptions::auto_ticks`] fills in
/// spacing, but nothing else stores a z range.
///
/// Consequences worth stating, because they are decisions and not accidents:
///
/// - A `Heatmap` / `Contour` / `HeatmapContour` series requires this to be
///   `Some`. Without it there is no z range and no colormap anywhere, so there
///   is no value to draw — the renderer rejects the series rather than
///   inventing one.
/// - To hide the bar, set `visible: false`. The options stay, the field still
///   renders, and the band is given back to the data area.
/// - There is therefore **one z scale per chart**. Several heatmaps in one
///   chart share it; two colourbars with different scales is out of scope.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColorBarOptions {
    /// False draws nothing and reserves no band. Field series still render.
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
    /// Space between the data area and the strip.
    pub gap_px: f32,
    /// Strip length as a fraction of the data area's length along its side.
    pub length_frac: f32,
    pub align: BarAlign,
    /// Free offset from the `side` + `align` anchor, in screen pixels — the
    /// same contract as `Legend::offset_{x,y}` and the title / label offsets:
    /// **margin-noncontributing**, so nudging the bar moves it without
    /// reflowing the data area under the pointer.
    ///
    /// This is where a drag accumulates. Without it the bar had no continuous
    /// position at all, which is what kept it out of the selection / drag /
    /// resize machinery every other piece of chrome already uses.
    pub offset_x: f32,
    pub offset_y: f32,
    pub colormap: ColorMap,
    /// Colour for z values the ramp cannot place: NaN, and non-positive z on a
    /// logarithmic colourbar. Fully transparent by default — "missing" reads as
    /// absent rather than as some particular value.
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
    /// GPU field sampler both go through this formula, so a cell and the tick
    /// beside it cannot disagree about what a value looks like:
    ///
    /// ```text
    /// linear: t = (z - min) / (max - min)
    /// log:    t = (log10 z - log10 min) / (log10 max - log10 min)
    /// ```
    ///
    /// clamped to `[0, 1]`. `None` for NaN, for a non-positive `z` on a
    /// logarithmic bar (no logarithm to take), and for a degenerate range.
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
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub picked_points: Option<PickedPointsConfig>,
    /// Typed point/bin/cell/contour selections. Additive to the legacy
    /// `picked_points` field so existing documents and hosts keep their exact
    /// point-only contract.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
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
        serde(default, skip_serializing_if = "DrawStyle::is_precise")
    )]
    pub draw_style: DrawStyle,
}

impl Config {
    /// Multiply every **pixel-based visual dim** by `scale`. Data ranges, colors,
    /// and scale enums are untouched. For resolution-invariant high-DPI export.
    pub fn scaled(&self, scale: f32) -> Self {
        let mut c = self.clone();
        c.scale_in_place(scale);
        c
    }

    pub fn scale_in_place(&mut self, s: f32) {
        let scale_u32 = |v: u32| ((v as f32) * s).round() as u32;
        self.chart_area.0.x = scale_u32(self.chart_area.0.x);
        self.chart_area.0.y = scale_u32(self.chart_area.0.y);
        self.chart_area.0.width = scale_u32(self.chart_area.0.width);
        self.chart_area.0.height = scale_u32(self.chart_area.0.height);

        scale_rich_text(&mut self.chart_title.text, s);
        self.chart_title.top_margin *= s;
        self.chart_title.offset_x *= s;
        self.chart_title.offset_y *= s;

        for axis in [
            &mut self.top_x,
            &mut self.bottom_x,
            &mut self.left_y,
            &mut self.right_y,
        ] {
            scale_axis_in_place(axis, s);
        }

        self.grid.major_x_width *= s;
        self.grid.major_y_width *= s;
        self.grid.minor_x_width *= s;
        self.grid.minor_y_width *= s;

        self.legend.offset_x *= s;
        self.legend.offset_y *= s;
        self.legend.padding *= s;
        scale_rich_text(&mut self.legend.content, s);

        if let Some(picked_points) = self.picked_points.as_mut() {
            picked_points.ring_width_px *= s;
            picked_points.radius_extra_px *= s;
        }
        if let Some(picked_data) = self.picked_data.as_mut() {
            picked_data.outline_width_px *= s;
            picked_data.point_radius_extra_px *= s;
            picked_data.contour_width_extra_px *= s;
        }

        // The colourbar's axis goes through the same helper as the four chart
        // axes — a new pixel field on `AxisOptions` must not be scalable on the
        // chart axes and silently fixed on the colourbar's. `length_frac` is a
        // fraction of the data area and stays put.
        if let Some(bar) = self.colorbar.as_mut() {
            bar.thickness_px *= s;
            bar.gap_px *= s;
            bar.border_width *= s;
            bar.offset_x *= s;
            bar.offset_y *= s;
            scale_axis_in_place(&mut bar.axis, s);
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
        match &mut self.draw_style {
            DrawStyle::Sketch(sketch) => {
                sketch.amplitude_px *= s;
                sketch.wavelength_px *= s;
            }
            DrawStyle::Milkyway(c) => {
                c.ribbon_width_px *= s;
                c.spread_px *= s;
                c.star_scale *= s;
                c.structure_scale *= s;
                if s > 0.0 {
                    c.star_density /= s;
                }
            }
            DrawStyle::Constellation(_) => {}
            DrawStyle::Precise => {}
        }
    }
}

/// Scale one axis' pixel-based dims. Shared by the four chart axes and the
/// colourbar's z axis so the set of "which fields are pixels" is written once.
fn scale_axis_in_place(axis: &mut AxisOptions, s: f32) {
    axis.label_style.font_size *= s;
    axis.label_style.label_offset_x *= s;
    axis.label_style.label_offset_y *= s;
    scale_rich_text(&mut axis.title_option.text, s);
    axis.title_option.offset_x *= s;
    axis.title_option.offset_y *= s;
    axis.out_margin *= s;
    axis.line_offset *= s;
    axis.line_width *= s;
    axis.major_tick_length *= s;
    axis.minor_tick_length *= s;
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
    use crate::default::default_colorbar_options;

    #[test]
    fn a_linear_bar_normalizes_and_clamps() {
        let mut bar = default_colorbar_options();
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
        let mut bar = default_colorbar_options();
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
        let mut bar = default_colorbar_options();
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
        let mut bar = default_colorbar_options();
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
        let mut bar = default_colorbar_options();
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
    use crate::default::{default_colorbar_options, default_config};

    #[test]
    fn scaling_a_config_scales_the_colorbar_pixels_but_not_its_fractions() {
        let mut cfg = default_config();
        cfg.colorbar = Some(default_colorbar_options());
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

    // The colourbar's axis must scale by exactly the same rule as a chart axis:
    // if the two ever diverge, a high-DPI export puts the bar's labels at a
    // different size than the axis labels beside them.
    #[test]
    fn the_colorbar_axis_scales_like_a_chart_axis() {
        let mut cfg = default_config();
        let mut bar = default_colorbar_options();
        bar.axis = cfg.left_y.clone();
        cfg.colorbar = Some(bar);

        let scaled = cfg.scaled(1.75);
        assert_eq!(scaled.colorbar.expect("colourbar").axis, scaled.left_y);
    }
}

// Legend types live in `crate::legend`; re-exported here so existing
// `config::Legend…` paths keep working.
pub use crate::legend::{
    Legend, LegendCorner, LegendEntryKind, append_legend_entry, append_legend_entry_rich,
    legend_entry_count, remove_legend_entry, scatter_shape_char, series_symbol_segments,
    set_legend_entry_label, symbol_segments, update_legend_symbols_preserving_text,
};

// `draw_style` is additive schema: configs serialized before the field
// existed must still parse (→ `Precise` mode), and `Precise` must not
// serialize — the same discipline as `RichSegment`'s per-segment overrides
// (text.rs). The enum is internally tagged (`"mode"`), so a style's
// parameters sit INLINE next to the tag.
#[cfg(all(test, feature = "serde"))]
mod draw_style_serde_tests {
    use super::{
        Config, DataSelectionsConfig, DrawStyle, PickedDataRef, PickedPointRef, PickedPointsConfig,
        SketchOptions,
    };
    use crate::default::default_config;

    /// Serialized default config — `draw_style` is `Precise`, so the JSON has
    /// no `"draw_style"` key: exactly the shape of pre-sketch documents.
    fn default_config_json() -> serde_json::Value {
        serde_json::to_value(default_config()).expect("serialize Config")
    }

    #[test]
    fn config_without_draw_style_key_deserializes_to_precise() {
        let cfg: Config =
            serde_json::from_value(default_config_json()).expect("pre-sketch document parses");
        assert_eq!(cfg.draw_style, DrawStyle::Precise);
    }

    #[test]
    fn sketch_tag_alone_yields_all_defaults() {
        let mut json = default_config_json();
        json.as_object_mut()
            .unwrap()
            .insert("draw_style".into(), serde_json::json!({ "mode": "sketch" }));
        let cfg: Config = serde_json::from_value(json).expect("tag-only sketch object parses");
        let s = *cfg.draw_style.sketch().expect("sketch enabled");
        assert_eq!(s, SketchOptions::default());
        // Pin the stock values themselves, not just Default == Default.
        assert_eq!(s.amplitude_px, 1.5);
        assert_eq!(s.wavelength_px, 60.0);
        assert_eq!(s.seed, 0);
    }

    #[test]
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
                ..SketchOptions::default()
            })
        );
    }

    #[test]
    fn sketch_round_trips_with_inline_fields() {
        let mut cfg = default_config();
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

    #[test]
    fn explicit_precise_mode_deserializes_to_precise() {
        let mut json = default_config_json();
        json.as_object_mut().unwrap().insert(
            "draw_style".into(),
            serde_json::json!({ "mode": "precise" }),
        );
        let cfg: Config = serde_json::from_value(json).expect("explicit precise parses");
        assert_eq!(cfg.draw_style, DrawStyle::Precise);
    }

    #[test]
    fn precise_serializes_without_key() {
        let json = default_config_json();
        assert!(
            json.get("draw_style").is_none(),
            "Precise draw_style must be skipped in serialization: {json}"
        );
    }

    #[test]
    fn config_without_picked_points_key_deserializes_to_none() {
        let cfg: Config =
            serde_json::from_value(default_config_json()).expect("pre-picker document parses");
        assert_eq!(cfg.picked_points, None);
    }

    #[test]
    fn picked_points_empty_object_yields_defaults() {
        let mut json = default_config_json();
        json.as_object_mut()
            .unwrap()
            .insert("picked_points".into(), serde_json::json!({}));
        let cfg: Config = serde_json::from_value(json).expect("tag-only picked_points parses");
        assert_eq!(cfg.picked_points, Some(PickedPointsConfig::default()));
    }

    #[test]
    fn picked_point_refs_skip_absent_source_id() {
        let mut cfg = default_config();
        cfg.picked_points = Some(PickedPointsConfig {
            refs: vec![
                PickedPointRef {
                    source_id: None,
                    series_id: "series-a".into(),
                    point_index: 2,
                },
                PickedPointRef {
                    source_id: Some("source-a".into()),
                    series_id: "series-b".into(),
                    point_index: 4,
                },
            ],
            ..PickedPointsConfig::default()
        });

        let json = serde_json::to_value(&cfg).expect("serialize Config");
        assert!(json["picked_points"]["refs"][0].get("source_id").is_none());
        assert_eq!(json["picked_points"]["refs"][1]["source_id"], "source-a");
        let back: Config = serde_json::from_value(json).expect("parse back");
        assert_eq!(back.picked_points, cfg.picked_points);
    }

    #[test]
    fn scale_scales_picked_point_pixel_dimensions() {
        let mut cfg = default_config();
        cfg.picked_points = Some(PickedPointsConfig::default());
        cfg.scale_in_place(2.0);
        let picked = cfg.picked_points.expect("picked points");
        assert_eq!(picked.ring_width_px, 4.0);
        assert_eq!(picked.radius_extra_px, 6.0);
    }

    #[test]
    fn config_without_picked_data_key_deserializes_to_none() {
        let cfg: Config =
            serde_json::from_value(default_config_json()).expect("pre-data-pick document parses");
        assert_eq!(cfg.picked_data, None);
    }

    #[test]
    fn picked_data_empty_object_yields_defaults() {
        let mut json = default_config_json();
        json.as_object_mut()
            .unwrap()
            .insert("picked_data".into(), serde_json::json!({}));
        let cfg: Config = serde_json::from_value(json).expect("empty picked_data parses");
        assert_eq!(cfg.picked_data, Some(DataSelectionsConfig::default()));
    }

    #[test]
    fn typed_picked_data_round_trips_with_minimal_provenance() {
        let mut cfg = default_config();
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
            ..DataSelectionsConfig::default()
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
        let mut cfg = default_config();
        cfg.picked_data = Some(DataSelectionsConfig::default());
        cfg.scale_in_place(2.0);
        let picked = cfg.picked_data.expect("picked data");
        assert_eq!(picked.outline_width_px, 4.0);
        assert_eq!(picked.point_radius_extra_px, 6.0);
        assert_eq!(picked.contour_width_extra_px, 4.0);
    }

    #[test]
    fn constellation_tag_alone_yields_all_defaults() {
        let mut json = default_config_json();
        json["draw_style"] = serde_json::json!({ "mode": "constellation" });
        let cfg: Config = serde_json::from_value(json).expect("tag-only constellation parses");
        assert_eq!(
            cfg.draw_style,
            super::DrawStyle::Constellation(super::ConstellationOptions::default()),
        );
    }

    #[test]
    fn milkyway_tag_alone_yields_all_defaults() {
        let mut json = default_config_json();
        json["draw_style"] = serde_json::json!({ "mode": "milkyway" });
        let cfg: Config = serde_json::from_value(json).expect("tag-only milkyway parses");
        assert_eq!(
            cfg.draw_style,
            super::DrawStyle::Milkyway(super::MilkywayOptions::default()),
        );
    }

    /// PARAM_SPECS defaults are literals (const context) — pin them to the
    /// `Default` impls so the two sources cannot drift, and pin every spec
    /// key to a real serde field by writing it through `draw_style` JSON.
    #[test]
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
            serde_json::to_value(SketchOptions::default()).unwrap(),
        );
        check(
            "milkyway",
            MilkywayOptions::PARAM_SPECS,
            serde_json::to_value(MilkywayOptions::default()).unwrap(),
        );
        check(
            "constellation",
            ConstellationOptions::PARAM_SPECS,
            serde_json::to_value(ConstellationOptions::default()).unwrap(),
        );
        assert_eq!(super::DrawStyle::param_specs_for_mode("nope"), None);
        assert_eq!(
            super::DrawStyle::param_specs_for_mode("precise"),
            Some(&[][..])
        );
    }

    #[test]
    fn constellation_round_trips_with_inline_fields() {
        let mut cfg = default_config();
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

    #[test]
    fn milkyway_round_trips_with_inline_fields() {
        let mut cfg = default_config();
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
}

use crate::color::Color;
use crate::format::LabelFormat;
use crate::layout::ChartArea;
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
        Self { amplitude_px: 1.5, wavelength_px: 60.0, seed: 0 }
    }
}

/// Constellation draw-style parameters. All fields have defaults — JSON
/// `{"mode":"constellation"}` alone works. Lines render as star chains over
/// a soft nebula ribbon tinted with the series `line_color` (that ribbon is
/// what keeps multiple series distinguishable); star colors stay physical
/// (blackbody locus, population mix follows local density).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ConstellationOptions {
    /// Stars per 100 px of arc length. Default 14.0.
    pub star_density: f32,
    /// Nebula ribbon full width, in px ("두께"). Default 14.0.
    pub ribbon_width_px: f32,
    /// Ribbon brightness 0..1 — how strongly the series-colored haze shows
    /// behind the stars. Default 0.30 (thin).
    pub ribbon_intensity: f32,
    /// Global star size multiplier. Default 1.0.
    pub star_scale: f32,
    /// Perpendicular star scatter σ from the path, in px. Larger reads more
    /// like a loose cluster, smaller tracks the data tighter. Default 2.5.
    pub spread_px: f32,
    /// Luminosity-function slope: the exponent of the brightness power law.
    /// Higher → a larger fraction of faint small stars per bright anchor
    /// (real fields sit faint-heavy). Sensible range ~1.5..6. Default 3.0.
    pub faint_bias: f32,
    /// Global seed. Same (config, data) → identical output. Default 0.
    pub seed: u32,
}

impl Default for ConstellationOptions {
    fn default() -> Self {
        Self {
            star_density: 14.0,
            ribbon_width_px: 14.0,
            ribbon_intensity: 0.30,
            star_scale: 1.0,
            spread_px: 2.5,
            faint_bias: 3.0,
            seed: 0,
        }
    }
}

/// UI-facing metadata for one stylized-mode parameter — the single source
/// for host slider ranges, so frontends never hardcode them. The range is
/// the RECOMMENDED span (what a slider should cover), not a hard limit:
/// the SSoT stores whatever the host sets, and the renderer applies only
/// its own safety guards (non-negative sizes, shader-side clamps).
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
    StyleParamSpec { key, min, max, default, integer: false }
}

const fn spec_int(key: &'static str, min: f64, max: f64, default: f64) -> StyleParamSpec {
    StyleParamSpec { key, min, max, default, integer: true }
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

impl ConstellationOptions {
    /// Parameter metadata for hosts — see [`StyleParamSpec`]. A model test
    /// pins each `default` to [`Default`], so the two cannot drift.
    pub const PARAM_SPECS: &'static [StyleParamSpec] = &[
        spec("star_density", 0.0, 60.0, 14.0),
        spec("ribbon_width_px", 2.0, 40.0, 14.0),
        spec("ribbon_intensity", 0.0, 1.0, 0.30),
        spec("star_scale", 0.3, 3.0, 1.0),
        spec("spread_px", 0.0, 10.0, 2.5),
        spec("faint_bias", 0.5, 10.0, 3.0),
        spec_int("seed", 0.0, 9999.0, 0.0),
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
    Constellation(ConstellationOptions),
}

impl DrawStyle {
    /// True for the default scientific path (used by serde skip).
    pub fn is_precise(&self) -> bool { matches!(self, DrawStyle::Precise) }
    /// Sketch parameters when the sketch style is active.
    pub fn sketch(&self) -> Option<&SketchOptions> {
        match self { DrawStyle::Sketch(s) => Some(s), _ => None }
    }
    /// Constellation parameters when the constellation style is active.
    pub fn constellation(&self) -> Option<&ConstellationOptions> {
        match self { DrawStyle::Constellation(c) => Some(c), _ => None }
    }

    /// Every mode tag, in declaration order — the values valid as the JSON
    /// `"mode"` of `draw_style`.
    pub fn mode_tags() -> &'static [&'static str] {
        &["precise", "sketch", "constellation"]
    }

    /// Parameter metadata for one mode tag. `precise` has no parameters
    /// (empty slice); unknown tags yield `None`. Hosts generate their
    /// parameter UI (sliders/steppers) from this instead of hardcoding
    /// ranges.
    pub fn param_specs_for_mode(mode: &str) -> Option<&'static [StyleParamSpec]> {
        match mode {
            "precise" => Some(&[]),
            "sketch" => Some(SketchOptions::PARAM_SPECS),
            "constellation" => Some(ConstellationOptions::PARAM_SPECS),
            _ => None,
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
    /// Chart-global render style. `Precise` (default, key absent in JSON) is
    /// identical to current rendering; every other variant is an opt-in
    /// stylized mode. No per-series mixing.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "DrawStyle::is_precise"))]
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

        for axis in [&mut self.top_x, &mut self.bottom_x, &mut self.left_y, &mut self.right_y] {
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

        self.grid.major_x_width *= s;
        self.grid.major_y_width *= s;
        self.grid.minor_x_width *= s;
        self.grid.minor_y_width *= s;

        self.legend.offset_x *= s;
        self.legend.offset_y *= s;
        self.legend.padding *= s;
        scale_rich_text(&mut self.legend.content, s);

        // Sketch wobble dims are pixel-based visual dims too. Scaled only in
        // Stylized modes — `Precise` carries no dims, so the precise path
        // sees no change. Constellation's `star_density` is per arc-px and
        // scales implicitly with the arc itself; `star_scale` multiplies
        // px-sized star sprites, so it scales like a px dim.
        match &mut self.draw_style {
            DrawStyle::Sketch(sketch) => {
                sketch.amplitude_px *= s;
                sketch.wavelength_px *= s;
            }
            DrawStyle::Constellation(c) => {
                c.ribbon_width_px *= s;
                c.spread_px *= s;
                c.star_scale *= s;
            }
            DrawStyle::Precise => {}
        }
    }
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

// Legend types live in `crate::legend`; re-exported here so existing
// `config::Legend…` paths keep working.
pub use crate::legend::{
    append_legend_entry, scatter_shape_char, series_symbol_segments, symbol_segments, Legend,
    LegendCorner, LegendEntryKind,
};

// `draw_style` is additive schema: configs serialized before the field
// existed must still parse (→ `Precise` mode), and `Precise` must not
// serialize — the same discipline as `RichSegment`'s per-segment overrides
// (text.rs). The enum is internally tagged (`"mode"`), so a style's
// parameters sit INLINE next to the tag.
#[cfg(all(test, feature = "serde"))]
mod draw_style_serde_tests {
    use super::{Config, DrawStyle, SketchOptions};
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
        json.as_object_mut()
            .unwrap()
            .insert("draw_style".into(), serde_json::json!({ "mode": "sketch", "seed": 7 }));
        let cfg: Config = serde_json::from_value(json).expect("partial sketch object parses");
        assert_eq!(
            cfg.draw_style,
            DrawStyle::Sketch(SketchOptions { seed: 7, ..SketchOptions::default() })
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
        json.as_object_mut()
            .unwrap()
            .insert("draw_style".into(), serde_json::json!({ "mode": "precise" }));
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
    fn constellation_tag_alone_yields_all_defaults() {
        let mut json = default_config_json();
        json["draw_style"] = serde_json::json!({ "mode": "constellation" });
        let cfg: Config = serde_json::from_value(json).expect("tag-only constellation parses");
        assert_eq!(
            cfg.draw_style,
            super::DrawStyle::Constellation(super::ConstellationOptions::default()),
        );
    }

    /// PARAM_SPECS defaults are literals (const context) — pin them to the
    /// `Default` impls so the two sources cannot drift, and pin every spec
    /// key to a real serde field by writing it through `draw_style` JSON.
    #[test]
    fn param_specs_match_defaults_and_serde_fields() {
        use super::{ConstellationOptions, SketchOptions};

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
                    s.key, s.default, d
                );
                assert!(s.min <= s.default && s.default <= s.max, "{mode}.{}: default outside range", s.key);

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
                let v = back.get(s.key).expect("key survives round trip").as_f64().unwrap();
                assert!((v - s.max).abs() < 1e-4, "{mode}.{}: value did not stick", s.key);
            }
        };

        check(
            "sketch",
            SketchOptions::PARAM_SPECS,
            serde_json::to_value(SketchOptions::default()).unwrap(),
        );
        check(
            "constellation",
            ConstellationOptions::PARAM_SPECS,
            serde_json::to_value(ConstellationOptions::default()).unwrap(),
        );
        assert_eq!(super::DrawStyle::param_specs_for_mode("nope"), None);
        assert_eq!(super::DrawStyle::param_specs_for_mode("precise"), Some(&[][..]));
    }

    #[test]
    fn constellation_round_trips_with_inline_fields() {
        let mut cfg = default_config();
        cfg.draw_style = super::DrawStyle::Constellation(super::ConstellationOptions {
            star_density: 22.0,
            ribbon_width_px: 20.0,
            ribbon_intensity: 0.4,
            star_scale: 1.3,
            spread_px: 3.5,
            faint_bias: 4.5,
            seed: 9,
        });
        let json = serde_json::to_value(&cfg).expect("serialize");
        assert_eq!(json["draw_style"]["mode"], "constellation");
        assert_eq!(json["draw_style"]["star_density"], 22.0);
        let back: Config = serde_json::from_value(json).expect("parse back");
        assert_eq!(back.draw_style, cfg.draw_style);
    }
}


//! Small categorical charts. Values and appearance are independent of Cartesian
//! axes and column streams. The renderer consumes a complete validated snapshot.
use crate::Color;

pub const MAX_RADIAL_SLICES: usize = 64;

/// One category. Construct with [`RadialSlice::new`]; optional fields inherit
/// chart settings. Zero-valued slices keep their index but have no visible surface.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RadialSlice {
    pub label: String,
    pub value: f64,
    pub color: Color,
    /// Offset along the slice bisector, as a fraction of the outer radius.
    pub explode: f32,
    /// None inherits the chart style. Some replaces the entire style, including
    /// camera tilt and depth; later chart-style edits do not propagate here.
    #[cfg_attr(feature = "serde", serde(default))]
    pub style: Option<RadialStyle>,
    /// None inherits placement; Some(RadialLabels::None) hides this label.
    #[cfg_attr(feature = "serde", serde(default))]
    pub labels: Option<RadialLabels>,
    /// None inherits the chart's label content format.
    #[cfg_attr(feature = "serde", serde(default))]
    pub label_format: Option<RadialLabelFormat>,
    /// Explicit color overrides automatic inside-label contrast selection.
    #[cfg_attr(feature = "serde", serde(default))]
    pub label_color: Option<Color>,
}
impl RadialSlice {
    pub fn new(label: impl Into<String>, value: f64, color: Color) -> Self {
        Self {
            label: label.into(),
            value,
            color,
            explode: 0.0,
            style: None,
            labels: None,
            label_format: None,
            label_color: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RadialKind {
    Pie,
    Donut { inner_radius: f32 },
}
impl RadialKind {
    pub fn inner_radius(self) -> f32 {
        match self {
            Self::Pie => 0.0,
            Self::Donut { inner_radius } => inner_radius,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RadialMaterial {
    Flat,
    Matte,
    Ceramic,
    BrushedMetal,
    Paper,
    Wood,
    SatinMetal,
    Toon,
    Enamel,
    Hatch,
    Pearl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RadialLabels {
    None,
    Inside,
    Outside,
}

/// Label content is independent of placement and can be overridden per slice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RadialLabelFormat {
    Name,
    Value,
    Percent,
    #[default]
    NamePercent,
    NameValue,
    ValuePercent,
    NameValuePercent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RadialOutline {
    pub rim: bool,
    pub separators: bool,
    pub emphasis: bool,
    /// Logical pixels, independent of DPR/export scale.
    pub width: f32,
    pub color: Color,
}
impl Default for RadialOutline {
    fn default() -> Self {
        Self {
            rim: false,
            separators: false,
            emphasis: false,
            width: 1.0,
            color: Color::new(0.08, 0.12, 0.2, 1.0),
        }
    }
}

/// Index in the original array (including zero-valued slices). group 1 is detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RadialTarget {
    pub group: usize,
    pub index: usize,
}
impl RadialTarget {
    pub fn main(index: usize) -> Self {
        Self { group: 0, index }
    }
    pub fn detail(index: usize) -> Self {
        Self { group: 1, index }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RadialInteraction {
    /// Current animation target; the host advances it with [`Self::animate_hover`].
    pub hovered: Option<RadialTarget>,
    /// Persistent selection, independent of the pointer. None clears selection.
    pub selected: Option<RadialTarget>,
    /// Linear progress in 0..1. Rendering applies smoothstep via [`Self::amount`].
    pub hover_progress: f32,
}
impl Default for RadialInteraction {
    fn default() -> Self {
        Self {
            hovered: None,
            selected: None,
            hover_progress: 0.0,
        }
    }
}
impl RadialInteraction {
    /// Host supplies elapsed seconds; no wall clock, browser, or permanent animation loop.
    /// Exit completes before a different slice enters. Returns whether state changed.
    /// A zero duration switches immediately. Negative/nonfinite time inputs are ignored.
    /// The host must keep requesting frames until the transition settles.
    pub fn animate_hover(&mut self, target: Option<RadialTarget>, dt: f32, duration: f32) -> bool {
        if !dt.is_finite() || dt < 0.0 || !duration.is_finite() || duration < 0.0 {
            return false;
        }
        let old = self.clone();
        if duration == 0.0 {
            self.hovered = target;
            self.hover_progress = if target.is_some() { 1.0 } else { 0.0 };
        } else {
            let mut step = dt / duration;
            if self.hovered != target {
                let used = step.min(self.hover_progress);
                self.hover_progress -= used;
                step -= used;
                if self.hover_progress <= 0.0 {
                    self.hovered = target;
                }
            }
            if self.hovered == target && target.is_some() {
                self.hover_progress = (self.hover_progress + step).min(1.0);
            }
        }
        *self != old
    }
    pub fn amount(&self, target: RadialTarget) -> f32 {
        if self.selected == Some(target) {
            1.0
        } else if self.hovered == Some(target) {
            let t = self.hover_progress;
            t * t * (3.0 - 2.0 * t)
        } else {
            0.0
        }
    }
}

/// Complete style, not a sparse override. Length ratios use the outer radius.
/// Defaults are flat/top-down; setting a material does not enable extrusion.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct RadialStyle {
    pub material: RadialMaterial,
    /// Orthographic camera tilt, 0..65 degrees. Zero is an undistorted top view.
    pub tilt_degrees: f32,
    /// Extrusion height / radius, 0..0.4. Does not change slice proportions.
    pub depth: f32,
    /// Geometric extrusion bevel / outer radius, clamped to half the depth.
    pub bevel: f32,
    /// Visual angular separation, 0..8 degrees, capped at 20% of each slice.
    pub gap_degrees: f32,
    /// Highlight spread, 0.05..1; material-dependent.
    pub roughness: f32,
    /// Procedural pattern intensity, 0..1. Zero disables the pattern.
    pub texture_strength: f32,
    /// Pattern frequency multiplier, 0.1..8. Larger values give finer grain;
    /// independent of export resolution.
    pub texture_scale: f32,
    pub light: [f32; 3],
    pub shadow: bool,
    /// Plan-view corner radii / outer radius; individually clamped for narrow slices.
    pub inner_corner: f32,
    pub outer_corner: f32,
    pub outline: RadialOutline,
    /// Vertical translation / outer radius at full hover or selection. Zero disables lift.
    pub hover_lift: f32,
    /// Added brightness at full hover/selection, 0..0.5.
    pub hover_brightness: f32,
    /// Highlight intensity, 0..1; material-dependent.
    pub gloss: f32,
    /// Finite pattern rotation in degrees, reduced modulo 360 by the renderer.
    pub texture_angle_degrees: f32,
}
impl Default for RadialStyle {
    fn default() -> Self {
        Self {
            material: RadialMaterial::Flat,
            tilt_degrees: 0.0,
            depth: 0.0,
            bevel: 0.015,
            gap_degrees: 0.0,
            roughness: 0.35,
            texture_strength: 0.2,
            texture_scale: 1.0,
            light: [-0.5, -0.6, 1.0],
            shadow: false,
            inner_corner: 0.0,
            outer_corner: 0.0,
            outline: RadialOutline::default(),
            hover_lift: 0.08,
            hover_brightness: 0.04,
            gloss: 0.55,
            texture_angle_degrees: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RadialSplit {
    pub slice_index: usize,
    /// Same units as the parent. Their sum must equal the parent's value.
    /// Labels on the detail chart show percentages of this subtotal.
    pub children: Vec<RadialSlice>,
    pub kind: RadialKind,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RadialChart {
    pub title: String,
    pub slices: Vec<RadialSlice>,
    pub kind: RadialKind,
    /// Clockwise from the right; -90 starts at the top.
    pub start_angle_degrees: f32,
    pub split: Option<RadialSplit>,
    pub style: RadialStyle,
    pub labels: RadialLabels,
    #[cfg_attr(feature = "serde", serde(default))]
    pub label_format: RadialLabelFormat,
    #[cfg_attr(feature = "serde", serde(default = "default_decimals"))]
    pub label_decimals: usize,
    #[cfg_attr(feature = "serde", serde(default))]
    pub value_suffix: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub interaction: RadialInteraction,
    pub font_family: String,
    pub font_size: f32,
    pub label_color: Color,
    pub background: Color,
}
impl Default for RadialChart {
    fn default() -> Self {
        Self {
            title: String::new(),
            slices: Vec::new(),
            kind: RadialKind::Pie,
            start_angle_degrees: -90.0,
            split: None,
            style: RadialStyle::default(),
            labels: RadialLabels::Outside,
            label_format: RadialLabelFormat::NamePercent,
            label_decimals: 1,
            value_suffix: String::new(),
            interaction: RadialInteraction::default(),
            font_family: "sans-serif".into(),
            font_size: 16.0,
            label_color: Color::new(0.12, 0.16, 0.22, 1.0),
            background: Color::WHITE,
        }
    }
}

#[cfg(feature = "serde")]
fn default_decimals() -> usize {
    1
}

fn range(x: f32, lo: f32, hi: f32) -> bool {
    x.is_finite() && x >= lo && x <= hi
}
fn color(c: Color) -> bool {
    [c.r, c.g, c.b, c.a].into_iter().all(|v| range(v, 0.0, 1.0))
}
fn kind(k: RadialKind) -> bool {
    match k {
        RadialKind::Pie => true,
        RadialKind::Donut { inner_radius } => range(inner_radius, 0.1, 0.85),
    }
}
fn total(slices: &[RadialSlice]) -> Result<f64, &'static str> {
    if slices.is_empty() || slices.len() > MAX_RADIAL_SLICES {
        return Err("a radial chart needs 1..64 slices");
    }
    let mut sum = 0.0;
    for s in slices {
        if !s.value.is_finite() || s.value < 0.0 {
            return Err("slice values must be finite and nonnegative");
        }
        if !color(s.color) || s.color.a != 1.0 {
            return Err("slice colors must be finite opaque RGBA in 0..1");
        }
        if let Some(style) = &s.style {
            validate_style(style)?;
        }
        if s.label_color.is_some_and(|c| !color(c)) {
            return Err("invalid slice label color");
        }
        if !range(s.explode, 0.0, 0.35) {
            return Err("slice explode must be in 0..0.35");
        }
        if s.label.chars().count() > 80 || s.label.contains(['\n', '\r']) {
            return Err("slice labels must be single-line, at most 80 characters");
        }
        sum += s.value;
    }
    if !sum.is_finite() || sum <= 0.0 {
        return Err("slice total must be positive and finite");
    }
    Ok(sum)
}
fn validate_style(s: &RadialStyle) -> Result<(), &'static str> {
    if !range(s.tilt_degrees, 0.0, 65.0)
        || !range(s.depth, 0.0, 0.4)
        || !range(s.bevel, 0.0, 0.1)
        || !range(s.gap_degrees, 0.0, 8.0)
        || !range(s.roughness, 0.05, 1.0)
        || !range(s.texture_strength, 0.0, 1.0)
        || !range(s.texture_scale, 0.1, 8.0)
        || !s.light.into_iter().all(|v| range(v, -100.0, 100.0))
        || !range(s.inner_corner, 0.0, 0.25)
        || !range(s.outer_corner, 0.0, 0.25)
        || !range(s.hover_lift, 0.0, 0.25)
        || !range(s.hover_brightness, 0.0, 0.5)
        || !range(s.gloss, 0.0, 1.0)
        || !s.texture_angle_degrees.is_finite()
        || !range(s.outline.width, 0.0, 8.0)
        || !color(s.outline.color)
        || s.light.iter().map(|x| x * x).sum::<f32>() < 0.0001
    {
        return Err("invalid radial material, light, or projection");
    }
    Ok(())
}
impl RadialChart {
    /// Resolve an original array index; invalid groups/indices return None.
    pub fn slice(&self, target: RadialTarget) -> Option<&RadialSlice> {
        match target.group {
            0 => self.slices.get(target.index),
            1 => self.split.as_ref()?.children.get(target.index),
            _ => None,
        }
    }
    /// Edit the source model, then prepare a new renderer frame to display it.
    pub fn slice_mut(&mut self, target: RadialTarget) -> Option<&mut RadialSlice> {
        match target.group {
            0 => self.slices.get_mut(target.index),
            1 => self.split.as_mut()?.children.get_mut(target.index),
            _ => None,
        }
    }
    /// Two label lines at most. Values use the original units; detail percentages
    /// use the detail subtotal. Formatting never changes numeric data.
    /// Call on a validated chart and pass the positive finite sum for this circle.
    pub fn label_lines(&self, slice: &RadialSlice, total: f64) -> Vec<String> {
        let d = self.label_decimals;
        let trim = |s: String| {
            if d == 0 {
                s
            } else {
                s.trim_end_matches('0').trim_end_matches('.').to_owned()
            }
        };
        let value = format!(
            "{}{}",
            trim(format!("{:.*}", d, slice.value)),
            self.value_suffix
        );
        let pct = format!(
            "{}%",
            trim(format!("{:.*}", d, slice.value / total * 100.0))
        );
        match slice.label_format.unwrap_or(self.label_format) {
            RadialLabelFormat::Name => vec![slice.label.clone()],
            RadialLabelFormat::Value => vec![value],
            RadialLabelFormat::Percent => vec![pct],
            RadialLabelFormat::NamePercent => vec![slice.label.clone(), pct],
            RadialLabelFormat::NameValue => vec![slice.label.clone(), value],
            RadialLabelFormat::ValuePercent => vec![format!("{value} ({pct})")],
            RadialLabelFormat::NameValuePercent => {
                vec![slice.label.clone(), format!("{value} ({pct})")]
            }
        }
    }

    /// Zero values are retained in the model, but have no geometry or label.
    /// This checks model constraints; the renderer additionally checks output size,
    /// device limits, label layout and GPU budget when preparing a frame.
    pub fn validate(&self) -> Result<(), &'static str> {
        total(&self.slices)?;
        if !kind(self.kind) || !self.start_angle_degrees.is_finite() {
            return Err("invalid radial shape or start angle");
        }
        if !range(self.font_size, 8.0, 64.0) || !color(self.label_color) || !color(self.background)
        {
            return Err("invalid radial text or background");
        }
        if self.title.chars().count() > 160 || self.title.contains(['\n', '\r']) {
            return Err("title must be single-line, at most 160 characters");
        }
        validate_style(&self.style)?;
        if self.label_decimals > 6
            || self.value_suffix.chars().count() > 24
            || self.value_suffix.contains(['\n', '\r'])
            || !range(self.interaction.hover_progress, 0.0, 1.0)
        {
            return Err("invalid radial label format or interaction");
        }
        for target in [self.interaction.hovered, self.interaction.selected]
            .into_iter()
            .flatten()
        {
            let slice = self
                .slice(target)
                .ok_or("radial interaction target is out of range")?;
            if slice.value == 0.0 {
                return Err("cannot emphasize a zero-valued slice");
            }
        }
        if let Some(split) = &self.split {
            let parent = self
                .slices
                .get(split.slice_index)
                .ok_or("split slice index is out of range")?;
            let sum = total(&split.children)?;
            if parent.value <= 0.0 || (sum - parent.value).abs() > parent.value.abs() * 1e-9 {
                return Err("detail values must sum to the parent slice value");
            }
            if !kind(split.kind) {
                return Err("invalid detail chart shape");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chart() -> RadialChart {
        RadialChart {
            slices: vec![
                RadialSlice::new("A", 87.0, Color::WHITE),
                RadialSlice::new("Other", 13.0, Color::WHITE),
            ],
            ..Default::default()
        }
    }
    #[test]
    fn radial_rejects_invalid_values_and_materials() {
        for value in [f64::NAN, f64::INFINITY, -1.0] {
            let mut c = chart();
            c.slices[0].value = value;
            assert!(c.validate().is_err());
        }
        let mut c = chart();
        c.slices.iter_mut().for_each(|s| s.value = 0.0);
        assert!(c.validate().is_err());
        let mut c = chart();
        c.style.light = [0.0; 3];
        assert!(c.validate().is_err());
        let mut c = chart();
        c.kind = RadialKind::Donut { inner_radius: 1.0 };
        assert!(c.validate().is_err());
        let mut c = chart();
        c.slices[0].value = 0.0;
        assert!(c.validate().is_ok());
    }
    #[test]
    fn radial_detail_conserves_parent_total() {
        let mut c = chart();
        c.split = Some(RadialSplit {
            slice_index: 1,
            kind: RadialKind::Pie,
            children: vec![
                RadialSlice::new("D", 2.08, Color::WHITE),
                RadialSlice::new("E", 1.82, Color::WHITE),
                RadialSlice::new("F", 9.1, Color::WHITE),
            ],
        });
        assert!(c.validate().is_ok());
        c.split.as_mut().unwrap().children[0].value = 16.0;
        assert!(c.validate().is_err());
    }
    #[cfg(feature = "serde")]
    #[test]
    fn radial_serialization_round_trip() {
        let c = chart();
        assert_eq!(
            c,
            serde_json::from_str::<RadialChart>(&serde_json::to_string(&c).unwrap()).unwrap()
        );
    }
}

#[cfg(test)]
mod editing_contract {
    use super::*;
    #[test]
    fn formats_selection_and_hover_are_model_owned() {
        let mut chart = RadialChart {
            slices: vec![
                RadialSlice::new("A", 20.0, Color::WHITE),
                RadialSlice::new("B", 80.0, Color::WHITE),
            ],
            ..Default::default()
        };
        let a = RadialTarget::main(0);
        chart.label_decimals = 0;
        assert_eq!(
            chart.label_lines(chart.slice(a).unwrap(), 100.0),
            ["A", "20%"]
        );
        chart.slice_mut(a).unwrap().label_format = Some(RadialLabelFormat::NameValuePercent);
        chart.value_suffix = " kg".into();
        assert_eq!(
            chart.label_lines(chart.slice(a).unwrap(), 100.0),
            ["A", "20 kg (20%)"]
        );
        chart.interaction.animate_hover(Some(a), 0.05, 0.2);
        assert_eq!(chart.interaction.hover_progress, 0.25);
        assert!(chart.interaction.amount(a) > 0.0);
        chart.interaction.animate_hover(None, 1.0, 0.2);
        assert_eq!(chart.interaction, RadialInteraction::default());
        chart.interaction.selected = Some(a);
        assert_eq!(chart.interaction.amount(a), 1.0);
        chart.slice_mut(a).unwrap().color = Color::from_rgb8(20, 120, 240);
        assert_eq!(chart.slices[1].color, Color::WHITE);
        assert!(chart.validate().is_ok());
        chart.interaction.selected = Some(RadialTarget::detail(0));
        assert!(chart.validate().is_err());
    }
    #[test]
    fn invalid_overrides_are_rejected_before_rendering() {
        let mut chart = RadialChart {
            slices: vec![RadialSlice::new("A", 1.0, Color::WHITE)],
            ..Default::default()
        };
        chart.slices[0].style = Some(RadialStyle {
            outer_corner: f32::NAN,
            ..Default::default()
        });
        assert!(chart.validate().is_err());
        chart.slices[0].style = None;
        chart.label_decimals = 7;
        assert!(chart.validate().is_err());
    }
    #[cfg(feature = "serde")]
    #[test]
    fn editing_options_roundtrip_and_old_snapshots_default() {
        let mut chart = RadialChart {
            slices: vec![RadialSlice::new("A", 1.0, Color::WHITE)],
            ..Default::default()
        };
        chart.style.outer_corner = 0.05;
        chart.style.material = RadialMaterial::SatinMetal;
        chart.slices[0].label_format = Some(RadialLabelFormat::Value);
        chart.interaction.selected = Some(RadialTarget::main(0));
        let json = serde_json::to_value(&chart).unwrap();
        assert_eq!(
            serde_json::from_value::<RadialChart>(json.clone()).unwrap(),
            chart
        );
        let mut old = json;
        for key in [
            "label_format",
            "label_decimals",
            "value_suffix",
            "interaction",
        ] {
            old.as_object_mut().unwrap().remove(key);
        }
        for key in ["style", "labels", "label_format", "label_color"] {
            old["slices"][0].as_object_mut().unwrap().remove(key);
        }
        old["style"].as_object_mut().unwrap().remove("outer_corner");
        let restored: RadialChart = serde_json::from_value(old).unwrap();
        assert_eq!(restored.style.outer_corner, 0.0);
        assert_eq!(restored.label_format, RadialLabelFormat::NamePercent);
        assert_eq!(restored.label_decimals, 1);
    }
}

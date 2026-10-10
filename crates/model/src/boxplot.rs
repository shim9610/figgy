//! Small, precomputed box plots. Statistical policy belongs to the host.
use crate::{Color, categorical::Category};
use std::collections::HashSet;

pub const MAX_BOX_CATEGORIES: usize = 64;
pub const MAX_BOX_SERIES: usize = 16;
pub const MAX_BOXES: usize = 512;
pub const MAX_BOX_OUTLIERS: usize = 128;
pub const MAX_BOXPLOT_OUTLIERS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoxPlotTarget {
    pub category_id: String,
    pub series_id: String,
}
impl BoxPlotTarget {
    pub fn new(category: impl Into<String>, series: impl Into<String>) -> Self {
        Self {
            category_id: category.into(),
            series_id: series.into(),
        }
    }
}
/// Actual whisker endpoints, not the theoretical IQR fences. CI endpoints may
/// extend outside Q1/Q3. No sample statistics are inferred by the renderer.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoxSummary {
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub whisker_low: f64,
    pub whisker_high: f64,
    pub mean: Option<f64>,
    pub sample_count: Option<u64>,
    pub median_ci: Option<[f64; 2]>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub outliers: Vec<f64>,
}
impl BoxSummary {
    pub fn new(low: f64, q1: f64, median: f64, q3: f64, high: f64) -> Self {
        Self {
            q1,
            median,
            q3,
            whisker_low: low,
            whisker_high: high,
            mean: None,
            sample_count: None,
            median_ci: None,
            outliers: vec![],
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let ordered = [
            self.whisker_low,
            self.q1,
            self.median,
            self.q3,
            self.whisker_high,
        ];
        if ordered.iter().any(|v| !v.is_finite()) || ordered.windows(2).any(|p| p[0] > p[1]) {
            return Err("box statistics must be finite and low <= Q1 <= median <= Q3 <= high");
        }
        if self.mean.is_some_and(|v| !v.is_finite()) || self.sample_count == Some(0) {
            return Err("mean must be finite; sample count must be positive when supplied");
        }
        if self.median_ci.is_some_and(|p| {
            !p[0].is_finite() || !p[1].is_finite() || p[0] > self.median || p[1] < self.median
        }) {
            return Err("median CI must be finite and contain the median");
        }
        if self.outliers.len() > MAX_BOX_OUTLIERS
            || self
                .outliers
                .iter()
                .any(|v| !v.is_finite() || (*v >= self.whisker_low && *v <= self.whisker_high))
        {
            return Err("at most 128 finite outliers per box, strictly outside the whiskers");
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BoxPlotDirection {
    #[default]
    Vertical,
    Horizontal,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BoxPlotMaterial {
    Flat,
    #[default]
    Matte,
    SatinMetal,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BoxPlotLabels {
    #[default]
    None,
    Median,
    MedianAndCount,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BoxOutlierShape {
    #[default]
    Circle,
    Square,
}
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct BoxPlotStyle {
    pub material: BoxPlotMaterial,
    /// Logical pixels, capped at 3 and at half the actual box extent.
    pub corner_radius: f32,
    pub outline: bool,
    pub outline_width: f32,
    pub outline_color: Color,
    pub median_width: f32,
    pub median_color: Color,
    pub whisker_width: f32,
    pub whisker_color: Color,
    pub caps: bool,
    pub cap_ratio: f32,
    pub notched: bool,
    pub notch_depth: f32,
    pub show_mean: bool,
    pub mean_size: f32,
    pub mean_color: Color,
    pub show_outliers: bool,
    pub outlier_size: f32,
    pub outlier_color: Color,
    pub outlier_shape: BoxOutlierShape,
    pub outlier_filled: bool,
    pub texture_strength: f32,
    pub texture_scale: f32,
    pub gloss: f32,
    pub emphasis_brightness: f32,
}
impl Default for BoxPlotStyle {
    fn default() -> Self {
        let ink = Color::from_rgb8(32, 46, 60);
        Self {
            material: BoxPlotMaterial::Matte,
            corner_radius: 0.0,
            outline: true,
            outline_width: 1.2,
            outline_color: ink,
            median_width: 1.2,
            median_color: ink,
            whisker_width: 1.2,
            whisker_color: ink,
            caps: true,
            cap_ratio: 0.5,
            notched: false,
            notch_depth: 0.22,
            show_mean: false,
            mean_size: 7.0,
            mean_color: Color::from_rgb8(180, 45, 45),
            show_outliers: true,
            outlier_size: 7.0,
            outlier_color: ink,
            outlier_shape: BoxOutlierShape::Circle,
            outlier_filled: false,
            texture_strength: 0.2,
            texture_scale: 1.0,
            gloss: 0.45,
            emphasis_brightness: 0.08,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoxPlotSeries {
    pub id: String,
    pub label: String,
    pub values: Vec<Option<BoxSummary>>,
    pub color: Color,
    pub style: Option<BoxPlotStyle>,
}
impl BoxPlotSeries {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        values: Vec<Option<BoxSummary>>,
        color: Color,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            values,
            color,
            style: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoxPlotOverride {
    pub target: BoxPlotTarget,
    pub color: Option<Color>,
    pub style: Option<BoxPlotStyle>,
    pub labels: Option<BoxPlotLabels>,
}
impl BoxPlotOverride {
    pub fn new(target: BoxPlotTarget) -> Self {
        Self {
            target,
            color: None,
            style: None,
            labels: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct BoxPlotChart {
    pub title: String,
    pub categories: Vec<Category>,
    pub series: Vec<BoxPlotSeries>,
    pub direction: BoxPlotDirection,
    pub style: BoxPlotStyle,
    pub overrides: Vec<BoxPlotOverride>,
    pub hovered: Option<BoxPlotTarget>,
    pub selected: Option<BoxPlotTarget>,
    pub labels: BoxPlotLabels,
    pub label_decimals: usize,
    pub value_suffix: String,
    pub value_title: String,
    pub category_title: String,
    /// None fits ALL supplied summary values, including hidden means/outliers/CI.
    /// Some fixes the linear viewport; geometry and picking are clipped to it.
    pub value_range: Option<[f64; 2]>,
    pub grid: bool,
    pub legend: bool,
    pub group_width: f32,
    pub box_gap: f32,
    pub font_family: String,
    pub font_size: f32,
    pub text_color: Color,
    pub background: Color,
}
impl Default for BoxPlotChart {
    fn default() -> Self {
        Self {
            title: String::new(),
            categories: vec![],
            series: vec![],
            direction: BoxPlotDirection::Vertical,
            style: BoxPlotStyle::default(),
            overrides: vec![],
            hovered: None,
            selected: None,
            labels: BoxPlotLabels::None,
            label_decimals: 1,
            value_suffix: String::new(),
            value_title: String::new(),
            category_title: String::new(),
            value_range: None,
            grid: true,
            legend: true,
            group_width: 0.65,
            box_gap: 8.0,
            font_family: "sans-serif".into(),
            font_size: 14.0,
            text_color: Color::from_rgb8(32, 46, 60),
            background: Color::WHITE,
        }
    }
}
impl BoxPlotChart {
    pub fn summary(&self, target: &BoxPlotTarget) -> Option<&BoxSummary> {
        let i = self
            .categories
            .iter()
            .position(|c| c.id == target.category_id)?;
        self.series
            .iter()
            .find(|s| s.id == target.series_id)?
            .values
            .get(i)?
            .as_ref()
    }
    pub fn box_override(&self, target: &BoxPlotTarget) -> Option<&BoxPlotOverride> {
        self.overrides.iter().find(|o| &o.target == target)
    }
    pub fn resolved_style(&self, target: &BoxPlotTarget) -> Option<&BoxPlotStyle> {
        let s = self.series.iter().find(|s| s.id == target.series_id)?;
        self.summary(target)?;
        Some(
            self.box_override(target)
                .and_then(|o| o.style.as_ref())
                .or(s.style.as_ref())
                .unwrap_or(&self.style),
        )
    }
    pub fn label_text(&self, target: &BoxPlotTarget) -> Option<String> {
        let summary = self.summary(target)?;
        let mode = self
            .box_override(target)
            .and_then(|o| o.labels)
            .unwrap_or(self.labels);
        if mode == BoxPlotLabels::None {
            return None;
        }
        let median = format!(
            "{:.*}{}",
            self.label_decimals, summary.median, self.value_suffix
        );
        Some(if mode == BoxPlotLabels::MedianAndCount {
            format!(
                "{median} (n={})",
                summary
                    .sample_count
                    .map_or_else(|| "—".into(), |n| n.to_string())
            )
        } else {
            median
        })
    }
    pub fn reorder_categories(&mut self, ids: &[&str]) -> Result<(), &'static str> {
        self.validate()?;
        if ids.len() != self.categories.len()
            || ids.iter().collect::<HashSet<_>>().len() != ids.len()
        {
            return Err("category order must be a permutation");
        }
        let order = ids
            .iter()
            .map(|id| {
                self.categories
                    .iter()
                    .position(|c| c.id == *id)
                    .ok_or("unknown category ID")
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.categories = order.iter().map(|i| self.categories[*i].clone()).collect();
        for series in &mut self.series {
            series.values = order.iter().map(|i| series.values[*i].clone()).collect();
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let (n, m) = (self.categories.len(), self.series.len());
        if n == 0 || n > MAX_BOX_CATEGORIES || m == 0 || m > MAX_BOX_SERIES || n * m > MAX_BOXES {
            return Err("boxplot limits: 1..64 categories, 1..16 series, at most 512 entries");
        }
        let mut ids = HashSet::new();
        for c in &self.categories {
            if c.id.is_empty() || !text(&c.id, 80) || !text(&c.label, 80) || !ids.insert(&c.id) {
                return Err("invalid category ID/label");
            }
        }
        ids.clear();
        let mut outliers = 0;
        for s in &self.series {
            if s.id.is_empty()
                || !text(&s.id, 80)
                || !text(&s.label, 80)
                || !ids.insert(&s.id)
                || s.values.len() != n
                || !color(s.color)
            {
                return Err("invalid series ID/label, color or value count");
            }
            if let Some(st) = &s.style {
                validate_style(st)?;
            }
            for v in s.values.iter().flatten() {
                v.validate()?;
                outliers += v.outliers.len();
            }
        }
        if outliers > MAX_BOXPLOT_OUTLIERS {
            return Err("at most 4096 outliers per chart");
        }
        validate_style(&self.style)?;
        if !range(self.font_size, 8.0, 32.0)
            || !range(self.group_width, 0.1, 0.95)
            || !range(self.box_gap, 0.0, 24.0)
            || !color(self.text_color)
            || !color(self.background)
            || self.label_decimals > 6
            || !text(&self.font_family, 160)
            || !text(&self.value_suffix, 24)
        {
            return Err("invalid boxplot layout, font or colors");
        }
        for s in [&self.title, &self.value_title, &self.category_title] {
            if !text(s, 160) {
                return Err("invalid title");
            }
        }
        if self
            .value_range
            .is_some_and(|r| !r[0].is_finite() || !r[1].is_finite() || r[0] >= r[1])
        {
            return Err("invalid linear value range");
        }
        let mut targets = HashSet::new();
        for o in &self.overrides {
            if !self.categories.iter().any(|c| c.id == o.target.category_id)
                || !self.series.iter().any(|s| s.id == o.target.series_id)
                || !targets.insert(&o.target)
                || o.color.is_some_and(|c| !color(c))
            {
                return Err("invalid or duplicate box override");
            }
            if let Some(st) = &o.style {
                validate_style(st)?;
            }
        }
        for (i, c) in self.categories.iter().enumerate() {
            for s in &self.series {
                if let Some(v) = &s.values[i] {
                    let target = BoxPlotTarget::new(&c.id, &s.id);
                    if self.resolved_style(&target).unwrap().notched && v.median_ci.is_none() {
                        return Err("notched boxes require an explicit median CI");
                    }
                }
            }
        }
        for t in [&self.hovered, &self.selected].into_iter().flatten() {
            if self.summary(t).is_none() {
                return Err("unknown or missing interaction target");
            }
        }
        Ok(())
    }
}
fn text(s: &str, max: usize) -> bool {
    s.chars().count() <= max && !s.chars().any(char::is_control)
}
fn range(v: f32, a: f32, b: f32) -> bool {
    v.is_finite() && (a..=b).contains(&v)
}
fn color(c: Color) -> bool {
    [c.r, c.g, c.b, c.a].into_iter().all(|v| range(v, 0.0, 1.0))
}
fn validate_style(s: &BoxPlotStyle) -> Result<(), &'static str> {
    if !range(s.corner_radius, 0.0, 3.0)
        || !range(s.outline_width, 0.0, 8.0)
        || !range(s.median_width, 0.0, 8.0)
        || !range(s.whisker_width, 0.0, 8.0)
        || !range(s.cap_ratio, 0.0, 1.0)
        || !range(s.notch_depth, 0.0, 0.45)
        || !range(s.mean_size, 1.0, 24.0)
        || !range(s.outlier_size, 1.0, 24.0)
        || !range(s.texture_strength, 0.0, 1.0)
        || !range(s.texture_scale, 0.1, 8.0)
        || !range(s.gloss, 0.0, 1.0)
        || !range(s.emphasis_brightness, 0.0, 0.5)
        || [
            s.outline_color,
            s.median_color,
            s.whisker_color,
            s.mean_color,
            s.outlier_color,
        ]
        .into_iter()
        .any(|c| !color(c))
    {
        Err("invalid boxplot style")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chart() -> BoxPlotChart {
        BoxPlotChart {
            categories: vec![Category::new("a", "A"), Category::new("b", "B")],
            series: vec![BoxPlotSeries::new(
                "s",
                "S",
                vec![Some(BoxSummary::new(1., 2., 3., 4., 5.)), None],
                Color::WHITE,
            )],
            ..Default::default()
        }
    }
    #[test]
    fn summaries_reject_invalid_order_nonfinite_and_nonoutliers() {
        let v = BoxSummary::new(1., 2., 3., 4., 5.);
        assert!(v.validate().is_ok());
        for invalid in [
            BoxSummary::new(2., 1., 3., 4., 5.),
            BoxSummary::new(1., 4., 3., 4., 5.),
            BoxSummary::new(1., 2., 3., 2., 5.),
            BoxSummary::new(1., 2., 3., 4., 3.),
            BoxSummary::new(1., 2., f64::NAN, 4., 5.),
        ] {
            assert!(invalid.validate().is_err());
        }
        for value in [1., 3., 5., f64::INFINITY] {
            let mut x = v.clone();
            x.outliers.push(value);
            assert!(x.validate().is_err());
        }
        let mut x = v.clone();
        x.outliers = vec![-1., 9.];
        x.mean = Some(2.5);
        assert!(x.validate().is_ok());
        x.mean = Some(f64::NAN);
        assert!(x.validate().is_err());
        assert!(BoxSummary::new(3., 3., 3., 3., 3.).validate().is_ok());
    }
    #[test]
    fn notches_require_explicit_ci_at_every_style_level_and_keep_extended_ci() {
        let mut c = chart();
        c.style.notched = true;
        assert!(c.validate().is_err());
        c.series[0].values[0].as_mut().unwrap().median_ci = Some([-5., 10.]);
        assert!(c.validate().is_ok());
        assert_eq!(
            c.summary(&BoxPlotTarget::new("a", "s")).unwrap().median_ci,
            Some([-5., 10.])
        );
        c.series[0].values[0].as_mut().unwrap().median_ci = None;
        c.style.notched = false;
        c.series[0].style = Some(BoxPlotStyle {
            notched: true,
            ..Default::default()
        });
        assert!(c.validate().is_err());
        c.series[0].style = None;
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("a", "s"));
        o.style = Some(BoxPlotStyle {
            notched: true,
            ..Default::default()
        });
        c.overrides.push(o);
        assert!(c.validate().is_err());
        c.series[0].values[0].as_mut().unwrap().median_ci = Some([4., 6.]);
        assert!(c.validate().is_err());
        c.series[0].values[0].as_mut().unwrap().median_ci = Some([3., 3.]);
        assert!(c.validate().is_ok());
    }
    #[test]
    fn reorder_is_atomic_and_preserves_identity_edits_and_missing_values() {
        let mut c = chart();
        let t = BoxPlotTarget::new("a", "s");
        c.selected = Some(t.clone());
        c.overrides.push(BoxPlotOverride::new(t.clone()));
        let before = c.clone();
        assert!(c.reorder_categories(&["a", "a"]).is_err());
        assert_eq!(c, before);
        c.reorder_categories(&["b", "a"]).unwrap();
        assert!(c.series[0].values[0].is_none());
        assert_eq!(c.summary(&t).unwrap().median, 3.);
        assert_eq!(c.selected, Some(t));
        c.selected = Some(BoxPlotTarget::new("b", "s"));
        assert!(c.validate().is_err());
    }
    #[test]
    fn bounds_are_independent_and_total_outliers_are_bounded() {
        for (n, m, valid) in [
            (64, 8, true),
            (32, 16, true),
            (65, 1, false),
            (1, 17, false),
            (33, 16, false),
            (0, 1, false),
        ] {
            let mut c = chart();
            c.categories = (0..n).map(|i| Category::new(i.to_string(), "A")).collect();
            c.series = (0..m)
                .map(|i| {
                    BoxPlotSeries::new(
                        i.to_string(),
                        "S",
                        vec![Some(BoxSummary::new(1., 2., 3., 4., 5.)); n],
                        Color::WHITE,
                    )
                })
                .collect();
            assert_eq!(c.validate().is_ok(), valid, "{n}x{m}");
        }
        let mut c = chart();
        let v = c.series[0].values[0].as_mut().unwrap();
        v.outliers = vec![9.; 128];
        assert!(c.validate().is_ok());
        c.series[0].values[0].as_mut().unwrap().outliers.push(9.);
        assert!(c.validate().is_err());
        c.categories = (0..33).map(|i| Category::new(i.to_string(), "A")).collect();
        let mut v = BoxSummary::new(1., 2., 3., 4., 5.);
        v.outliers = vec![9.; 128];
        c.series[0].values = vec![Some(v); 33];
        assert!(c.validate().is_err());
        c.categories.pop();
        c.series[0].values.pop();
        assert!(c.validate().is_ok());
    }
    #[test]
    fn style_ranges_and_override_targets_are_validated() {
        for edit in [
            |s: &mut BoxPlotStyle| s.corner_radius = 4.,
            |s: &mut BoxPlotStyle| s.notch_depth = 0.5,
            |s: &mut BoxPlotStyle| s.gloss = f32::NAN,
            |s: &mut BoxPlotStyle| s.whisker_width = -1.,
            |s: &mut BoxPlotStyle| s.outlier_color.a = 2.,
        ] {
            for level in 0..3 {
                let mut c = chart();
                let mut st = BoxPlotStyle::default();
                edit(&mut st);
                match level {
                    0 => c.style = st,
                    1 => c.series[0].style = Some(st),
                    _ => {
                        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("a", "s"));
                        o.style = Some(st);
                        c.overrides.push(o);
                    }
                }
                assert!(c.validate().is_err());
            }
        }
        let mut c = chart();
        c.overrides = vec![BoxPlotOverride::new(BoxPlotTarget::new("a", "s")); 2];
        assert!(c.validate().is_err());
        c.overrides = vec![BoxPlotOverride::new(BoxPlotTarget::new("unknown", "s"))];
        assert!(c.validate().is_err());
        c.overrides.clear();
        c.value_range = Some([1., 1.]);
        assert!(c.validate().is_err());
        c.value_range = Some([-1., 2.]);
        assert!(c.validate().is_ok());
    }
    #[test]
    fn labels_use_supplied_median_and_sample_count_with_individual_override() {
        let mut c = chart();
        let t = BoxPlotTarget::new("a", "s");
        assert_eq!(c.label_text(&t), None);
        c.labels = BoxPlotLabels::MedianAndCount;
        c.value_suffix = " ms".into();
        assert_eq!(c.label_text(&t).unwrap(), "3.0 ms (n=—)");
        c.series[0].values[0].as_mut().unwrap().sample_count = Some(42);
        c.label_decimals = 2;
        assert_eq!(c.label_text(&t).unwrap(), "3.00 ms (n=42)");
        let mut o = BoxPlotOverride::new(t.clone());
        o.labels = Some(BoxPlotLabels::Median);
        c.overrides.push(o);
        assert_eq!(c.label_text(&t).unwrap(), "3.00 ms");
    }
    #[cfg(feature = "serde")]
    #[test]
    fn complete_ssot_round_trips_without_losing_optional_statistics() {
        let minimal: BoxSummary =
            serde_json::from_str(r#"{"whisker_low":1,"q1":2,"median":3,"q3":4,"whisker_high":5}"#)
                .unwrap();
        assert_eq!(minimal, BoxSummary::new(1., 2., 3., 4., 5.));
        let mut c = chart();
        let v = c.series[0].values[0].as_mut().unwrap();
        v.median_ci = Some([2.5, 3.5]);
        v.mean = Some(3.1);
        v.sample_count = Some(21);
        v.outliers = vec![9.];
        c.style.notched = true;
        c.direction = BoxPlotDirection::Horizontal;
        c.selected = Some(BoxPlotTarget::new("a", "s"));
        let json = serde_json::to_string(&c).unwrap();
        let back: BoxPlotChart = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
        back.validate().unwrap();
    }
}

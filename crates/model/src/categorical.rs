//! Bounded categorical bars. Values are model-owned, not ColumnSource payloads.
//! Stable category/series IDs identify edits and selection; values follow category order.
use crate::Color;
use std::collections::HashSet;

pub const MAX_CATEGORIES: usize = 64;
pub const MAX_BAR_SERIES: usize = 16;
pub const MAX_CATEGORY_BARS: usize = 512;

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Category {
    pub id: String,
    pub label: String,
}
impl Category {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CategoryBarTarget {
    pub category_id: String,
    pub series_id: String,
}
impl CategoryBarTarget {
    pub fn new(category: impl Into<String>, series: impl Into<String>) -> Self {
        Self {
            category_id: category.into(),
            series_id: series.into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BarSeries {
    pub id: String,
    pub label: String,
    /// Exactly one entry per category. None is missing; Some(0) is a real zero.
    pub values: Vec<Option<f64>>,
    pub color: Color,
    /// Complete override; None inherits the chart style.
    pub style: Option<CategoryBarStyle>,
}
impl BarSeries {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        values: Vec<Option<f64>>,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CategoryBarMode {
    #[default]
    Grouped,
    Stacked,
    PercentStacked,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CategoryBarDirection {
    #[default]
    Vertical,
    Horizontal,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CategoryBarMaterial {
    #[default]
    Flat,
    Matte,
    SatinMetal,
    Enamel,
    Paper,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CategoryBarLabels {
    None,
    Inside,
    Outside,
    #[default]
    Auto,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CategoryBarLabelFormat {
    #[default]
    Value,
    Percent,
    ValuePercent,
    CategoryValue,
}
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct CategoryBarStyle {
    pub material: CategoryBarMaterial,
    /// Logical pixels. Only the exposed end of a stack is rounded.
    pub corner_radius: f32,
    pub outline: bool,
    pub outline_width: f32,
    pub outline_color: Color,
    pub texture_strength: f32,
    pub texture_scale: f32,
    pub gloss: f32,
    /// Highlight preserves the data endpoint; it never grows/lifts a bar.
    pub emphasis_brightness: f32,
}
impl Default for CategoryBarStyle {
    fn default() -> Self {
        Self {
            material: CategoryBarMaterial::Flat,
            corner_radius: 5.0,
            outline: false,
            outline_width: 1.0,
            outline_color: Color::from_rgb8(40, 55, 75),
            texture_strength: 0.2,
            texture_scale: 1.0,
            gloss: 0.45,
            emphasis_brightness: 0.08,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CategoryBarOverride {
    pub target: CategoryBarTarget,
    pub color: Option<Color>,
    pub style: Option<CategoryBarStyle>,
    pub labels: Option<CategoryBarLabels>,
    pub label_format: Option<CategoryBarLabelFormat>,
}
impl CategoryBarOverride {
    pub fn new(target: CategoryBarTarget) -> Self {
        Self {
            target,
            color: None,
            style: None,
            labels: None,
            label_format: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct CategoricalChart {
    pub title: String,
    pub categories: Vec<Category>,
    pub series: Vec<BarSeries>,
    pub mode: CategoryBarMode,
    pub direction: CategoryBarDirection,
    pub style: CategoryBarStyle,
    pub overrides: Vec<CategoryBarOverride>,
    pub hovered: Option<CategoryBarTarget>,
    pub selected: Option<CategoryBarTarget>,
    pub labels: CategoryBarLabels,
    pub label_format: CategoryBarLabelFormat,
    pub label_decimals: usize,
    pub value_suffix: String,
    pub value_title: String,
    pub category_title: String,
    pub grid: bool,
    pub legend: bool,
    /// Fraction of the category slot occupied by all bars, 0.1..0.95.
    pub group_width: f32,
    /// Logical pixels between grouped bars; rejected if bars no longer fit.
    pub bar_gap: f32,
    pub font_family: String,
    pub font_size: f32,
    pub text_color: Color,
    pub background: Color,
}
impl Default for CategoricalChart {
    fn default() -> Self {
        Self {
            title: String::new(),
            categories: vec![],
            series: vec![],
            mode: CategoryBarMode::Grouped,
            direction: CategoryBarDirection::Vertical,
            style: CategoryBarStyle::default(),
            overrides: vec![],
            hovered: None,
            selected: None,
            labels: CategoryBarLabels::Auto,
            label_format: CategoryBarLabelFormat::Value,
            label_decimals: 1,
            value_suffix: String::new(),
            value_title: String::new(),
            category_title: String::new(),
            grid: true,
            legend: true,
            group_width: 0.7,
            bar_gap: 3.0,
            font_family: "sans-serif".into(),
            font_size: 14.0,
            text_color: Color::from_rgb8(35, 45, 65),
            background: Color::WHITE,
        }
    }
}
/// Resolved data segment in axis units. Percent stacks use 0..100; originals stay unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct CategoryBarSegment {
    pub target: CategoryBarTarget,
    pub category_index: usize,
    pub series_index: usize,
    pub value: f64,
    pub start: f64,
    pub end: f64,
    pub percent: Option<f64>,
    pub exposed_end: bool,
}
fn text(s: &str, max: usize) -> bool {
    s.chars().count() <= max && !s.chars().any(char::is_control)
}
fn range(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}
fn color(c: Color, opaque: bool) -> bool {
    [c.r, c.g, c.b, c.a].into_iter().all(|v| range(v, 0.0, 1.0)) && (!opaque || c.a == 1.0)
}
fn style(s: &CategoryBarStyle) -> bool {
    range(s.corner_radius, 0.0, 32.0)
        && range(s.outline_width, 0.0, 8.0)
        && color(s.outline_color, false)
        && range(s.texture_strength, 0.0, 1.0)
        && range(s.texture_scale, 0.1, 8.0)
        && range(s.gloss, 0.0, 1.0)
        && range(s.emphasis_brightness, 0.0, 0.5)
}
impl CategoricalChart {
    pub fn value(&self, target: &CategoryBarTarget) -> Option<f64> {
        let i = self
            .categories
            .iter()
            .position(|c| c.id == target.category_id)?;
        *self
            .series
            .iter()
            .find(|s| s.id == target.series_id)?
            .values
            .get(i)?
    }
    pub fn bar_override(&self, target: &CategoryBarTarget) -> Option<&CategoryBarOverride> {
        self.overrides.iter().find(|o| &o.target == target)
    }
    /// Reorders categories and every value column together. IDs/styles/selection are preserved.
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
            series.values = order.iter().map(|i| series.values[*i]).collect();
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let n = self.categories.len();
        let m = self.series.len();
        if n == 0 || n > MAX_CATEGORIES || m == 0 || m > MAX_BAR_SERIES || n * m > MAX_CATEGORY_BARS
        {
            return Err("categorical limits: 1..64 categories, 1..16 series, at most 512 entries");
        }
        let mut ids = HashSet::new();
        for c in &self.categories {
            if c.id.is_empty() || !text(&c.id, 80) || !text(&c.label, 80) || !ids.insert(&c.id) {
                return Err("invalid or duplicate category ID/label");
            }
        }
        ids.clear();
        for s in &self.series {
            if s.id.is_empty()
                || !text(&s.id, 80)
                || !text(&s.label, 80)
                || !ids.insert(&s.id)
                || s.values.len() != n
                || !color(s.color, true)
                || s.style.as_ref().is_some_and(|s| !style(s))
            {
                return Err("invalid series, value count, color or style");
            }
            if s.values.iter().flatten().any(|v| {
                !v.is_finite() || (self.mode == CategoryBarMode::PercentStacked && *v < 0.0)
            }) {
                return Err("values must be finite; percent stacks require nonnegative values");
            }
        }
        if !style(&self.style)
            || !range(self.font_size, 8.0, 32.0)
            || !range(self.group_width, 0.1, 0.95)
            || !range(self.bar_gap, 0.0, 24.0)
            || !color(self.text_color, false)
            || !color(self.background, false)
            || self.label_decimals > 6
        {
            return Err("invalid categorical style or layout");
        }
        for s in [&self.title, &self.value_title, &self.category_title] {
            if !text(s, 160) {
                return Err("titles must be single-line, at most 160 characters");
            }
        }
        if !text(&self.value_suffix, 24) {
            return Err("invalid value suffix");
        }
        let mut targets = HashSet::new();
        for o in &self.overrides {
            if !self.categories.iter().any(|c| c.id == o.target.category_id)
                || !self.series.iter().any(|s| s.id == o.target.series_id)
                || !targets.insert(&o.target)
                || o.color.is_some_and(|c| !color(c, true))
                || o.style.as_ref().is_some_and(|s| !style(s))
            {
                return Err("invalid or duplicate bar override");
            }
        }
        for t in [&self.hovered, &self.selected].into_iter().flatten() {
            if self.value(t).is_none() {
                return Err("interaction target is missing or unknown");
            }
        }
        self.resolve_segments()?;
        Ok(())
    }
    /// Validates the model, then computes positive/negative stacks independently.
    /// Percentages are undefined (None) for a zero or signed category total.
    pub fn segments(&self) -> Result<Vec<CategoryBarSegment>, &'static str> {
        self.validate()?;
        self.resolve_segments()
    }
    fn resolve_segments(&self) -> Result<Vec<CategoryBarSegment>, &'static str> {
        let mut out = Vec::new();
        for (ci, c) in self.categories.iter().enumerate() {
            let max = self
                .series
                .iter()
                .filter_map(|s| s.values.get(ci).copied().flatten())
                .fold(0.0, f64::max);
            let has_negative = self
                .series
                .iter()
                .any(|s| s.values.get(ci).copied().flatten().is_some_and(|v| v < 0.0));
            // Normalize before summing: percent stacks remain valid even if the raw total overflows.
            let denominator = if max > 0.0 {
                self.series
                    .iter()
                    .filter_map(|s| s.values.get(ci).copied().flatten())
                    .map(|v| v / max)
                    .sum::<f64>()
            } else {
                0.0
            };
            let mut positive = 0.0;
            let mut negative = 0.0;
            let mut last_positive = None;
            let mut last_negative = None;
            for (si, s) in self.series.iter().enumerate() {
                let Some(value) = s.values.get(ci).copied().flatten() else {
                    continue;
                };
                let percent = if !has_negative && denominator > 0.0 {
                    Some(value / max / denominator * 100.0)
                } else {
                    None
                };
                let amount = if self.mode == CategoryBarMode::PercentStacked {
                    percent.unwrap_or(0.0)
                } else {
                    value
                };
                let start = if self.mode == CategoryBarMode::Grouped {
                    0.0
                } else if amount < 0.0 {
                    negative
                } else {
                    positive
                };
                let end = start + amount;
                if !end.is_finite() {
                    return Err("stack total overflow");
                }
                if amount < 0.0 {
                    negative = end;
                    last_negative = Some(out.len());
                } else if amount > 0.0 {
                    positive = end;
                    last_positive = Some(out.len());
                }
                out.push(CategoryBarSegment {
                    target: CategoryBarTarget::new(&c.id, &s.id),
                    category_index: ci,
                    series_index: si,
                    value,
                    start,
                    end,
                    percent,
                    exposed_end: self.mode == CategoryBarMode::Grouped,
                });
            }
            if self.mode != CategoryBarMode::Grouped {
                for i in [last_positive, last_negative].into_iter().flatten() {
                    out[i].exposed_end = true;
                }
            }
            // Sum rounding must not leave the final percent stack short of 100.
            if self.mode == CategoryBarMode::PercentStacked {
                if let Some(i) = last_positive {
                    out[i].end = 100.0;
                }
            }
        }
        Ok(out)
    }
    pub fn label_text(&self, segment: &CategoryBarSegment) -> String {
        let number = |v: f64| {
            let s = format!("{:.*}", self.label_decimals, v);
            if self.label_decimals == 0 {
                s
            } else {
                s.trim_end_matches('0').trim_end_matches('.').to_owned()
            }
        };
        let value = format!("{}{}", number(segment.value), self.value_suffix);
        let pct = segment
            .percent
            .map(|v| format!("{}%", number(v)))
            .unwrap_or_else(|| "—".into());
        match self
            .bar_override(&segment.target)
            .and_then(|o| o.label_format)
            .unwrap_or(self.label_format)
        {
            CategoryBarLabelFormat::Value => value,
            CategoryBarLabelFormat::Percent => pct,
            CategoryBarLabelFormat::ValuePercent => format!("{value} ({pct})"),
            CategoryBarLabelFormat::CategoryValue => {
                format!("{}: {value}", self.categories[segment.category_index].label)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chart() -> CategoricalChart {
        CategoricalChart {
            categories: vec![Category::new("a", "A"), Category::new("b", "B")],
            series: vec![
                BarSeries::new("s", "S", vec![Some(2.0), None], Color::WHITE),
                BarSeries::new("t", "T", vec![Some(3.0), Some(0.0)], Color::WHITE),
            ],
            ..Default::default()
        }
    }
    #[test]
    fn missing_zero_and_stacks_are_distinct() {
        let mut c = chart();
        c.mode = CategoryBarMode::Stacked;
        let s = c.segments().unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!((s[1].start, s[1].end), (2.0, 5.0));
        assert!(!s[0].exposed_end);
        assert!(s[1].exposed_end);
        assert_eq!(s[2].percent, None);
        c.mode = CategoryBarMode::PercentStacked;
        let s = c.segments().unwrap();
        assert_eq!(s[0].end, 40.0);
        assert_eq!(s[1].end, 100.0);
        assert_eq!(s[2].end, 0.0);
    }
    #[test]
    fn signed_stacks_and_overflow_have_explicit_rules() {
        let mut c = chart();
        c.mode = CategoryBarMode::Stacked;
        c.series[1].values[0] = Some(-3.0);
        let s = c.segments().unwrap();
        assert_eq!((s[1].start, s[1].end), (0.0, -3.0));
        assert!(s[0].exposed_end && s[1].exposed_end);
        assert_eq!(s[0].percent, None);
        c.mode = CategoryBarMode::PercentStacked;
        assert!(c.validate().is_err());
        c.series[0].values[0] = Some(f64::MAX);
        c.series[1].values[0] = Some(f64::MAX);
        assert_eq!(c.segments().unwrap()[1].end, 100.0);
        c.mode = CategoryBarMode::Stacked;
        assert!(c.validate().is_err());
    }
    #[test]
    fn identity_survives_reordering_and_rename() {
        let mut c = chart();
        let t = CategoryBarTarget::new("a", "s");
        c.selected = Some(t.clone());
        c.overrides.push(CategoryBarOverride::new(t.clone()));
        c.reorder_categories(&["b", "a"]).unwrap();
        c.categories[1].label = "Renamed".into();
        assert_eq!(c.value(&t), Some(2.0));
        assert_eq!(c.selected, Some(t));
        let old = c.clone();
        assert!(c.reorder_categories(&["a", "a"]).is_err());
        assert_eq!(old, c);
    }
    #[test]
    fn malformed_payload_is_rejected() {
        let mut c = chart();
        c.series[0].values.pop();
        assert!(c.validate().is_err());
        let mut c = chart();
        c.categories[1].id = "a".into();
        assert!(c.validate().is_err());
        let mut c = chart();
        c.series[0].values[0] = Some(f64::NAN);
        assert!(c.validate().is_err());
        let mut c = chart();
        c.selected = Some(CategoryBarTarget::new("b", "s"));
        assert!(c.validate().is_err());
    }
    #[cfg(feature = "serde")]
    #[test]
    fn serde_preserves_full_edit_state() {
        let mut c = chart();
        c.style.material = CategoryBarMaterial::Paper;
        c.selected = Some(CategoryBarTarget::new("a", "s"));
        let mut o = CategoryBarOverride::new(CategoryBarTarget::new("a", "t"));
        o.label_format = Some(CategoryBarLabelFormat::ValuePercent);
        c.overrides.push(o);
        assert_eq!(
            c,
            serde_json::from_str::<CategoricalChart>(&serde_json::to_string(&c).unwrap()).unwrap()
        );
    }
    #[test]
    fn independent_limits_and_duplicate_overrides_are_checked() {
        let make = |n: usize, m: usize| CategoricalChart {
            categories: (0..n)
                .map(|i| Category::new(format!("c{i}"), "Category"))
                .collect(),
            series: (0..m)
                .map(|i| BarSeries::new(format!("s{i}"), "Series", vec![Some(1.); n], Color::WHITE))
                .collect(),
            ..Default::default()
        };
        for (n, m) in [(64, 8), (32, 16), (1, 1)] {
            assert!(make(n, m).validate().is_ok());
        }
        for (n, m) in [(65, 1), (1, 17), (33, 16), (64, 9), (0, 1), (1, 0)] {
            assert!(
                make(n, m).validate().is_err(),
                "accepted {n} categories / {m} series"
            );
        }
        let mut c = make(1, 1);
        let target = CategoryBarTarget::new("c0", "s0");
        c.overrides = vec![
            CategoryBarOverride::new(target.clone()),
            CategoryBarOverride::new(target),
        ];
        assert!(c.validate().is_err());
        c.overrides.truncate(1);
        c.overrides[0].target.category_id = "missing".into();
        assert!(c.validate().is_err());
    }
    #[test]
    fn style_validation_is_identical_at_every_inheritance_level() {
        let edits: Vec<Box<dyn Fn(&mut CategoryBarStyle)>> = vec![
            Box::new(|s| s.corner_radius = -0.1),
            Box::new(|s| s.corner_radius = 32.1),
            Box::new(|s| s.outline_width = 8.1),
            Box::new(|s| s.outline_color.a = -0.1),
            Box::new(|s| s.texture_strength = 1.1),
            Box::new(|s| s.texture_scale = 0.09),
            Box::new(|s| s.texture_scale = 8.1),
            Box::new(|s| s.gloss = f32::NAN),
            Box::new(|s| s.emphasis_brightness = 0.51),
        ];
        for edit in edits {
            let mut invalid = CategoryBarStyle::default();
            edit(&mut invalid);
            let mut c = chart();
            c.style = invalid.clone();
            assert!(c.validate().is_err());
            let mut c = chart();
            c.series[0].style = Some(invalid.clone());
            assert!(c.validate().is_err());
            let mut c = chart();
            let mut o = CategoryBarOverride::new(CategoryBarTarget::new("a", "s"));
            o.style = Some(invalid);
            c.overrides.push(o);
            assert!(c.validate().is_err());
        }
    }
}

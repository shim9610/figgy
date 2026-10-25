//! The legacy radar (spider) chart document, kept as a reader.
//!
//! A radar chart is a polar chart now: a `Config` with
//! [`Config::polar`](crate::config::Config::polar), a categorical angle axis
//! ([`crate::config::AxisOptions::categories`] on `bottom_x`) and one
//! [`DataRenderType::Radar`](crate::data_config::DataRenderType::Radar)
//! series per radar series, whose values are a column of the renderer's
//! column pool. A [`RadarChart`] document still parses and validates here,
//! and `Config::from_radar` turns it into that chart and the uploads of its
//! values. Values are the host's; the renderer never normalizes a spoke to
//! its own range.
use crate::{Color, categorical::Category};
use std::collections::HashSet;

/// The marker, label and series style types live with the series kinds
/// now ([`crate::data_config::DataRenderType::Radar`]); these paths keep
/// working.
pub use crate::data_config::{RadarLabels, RadarMarker, RadarSeriesStyle};

/// The fewest categories (spokes) a radar chart draws.
pub const MIN_RADAR_CATEGORIES: usize = 3;
/// The most categories (spokes) a radar chart draws.
pub const MAX_RADAR_CATEGORIES: usize = 64;
/// The most series a radar chart draws.
pub const MAX_RADAR_SERIES: usize = 16;

/// One vertex of one series (`category_id` set) or a whole series polygon
/// (`category_id` absent): what hover, selection and hit testing address.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RadarTarget {
    /// The series the target belongs to.
    pub series_id: String,
    /// The category of the vertex; `None` addresses the whole series.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub category_id: Option<String>,
}
impl RadarTarget {
    /// A whole-series target.
    pub fn series(series: impl Into<String>) -> Self {
        Self {
            series_id: series.into(),
            category_id: None,
        }
    }
    /// The vertex of `series` on `category`'s spoke.
    pub fn vertex(series: impl Into<String>, category: impl Into<String>) -> Self {
        Self {
            series_id: series.into(),
            category_id: Some(category.into()),
        }
    }
}

/// The grid behind the series: rings as regular polygons through the spokes
/// (the classic spider web) or as circles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RadarGridShape {
    /// Rings as regular polygons through the spokes (the default).
    #[default]
    Polygon,
    /// Rings as circles.
    Circle,
}

/// The chart-wide appearance: spoke layout, grid, and the series defaults a
/// `RadarSeriesStyle` can override.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct RadarStyle {
    /// Where the first category's spoke points, in degrees; -90 is up.
    pub start_angle_degrees: f32,
    /// Whether the categories follow clockwise on screen.
    pub clockwise: bool,
    /// Polygon or circular rings.
    pub grid_shape: RadarGridShape,
    /// Colour of the spokes.
    pub spoke_color: Color,
    /// Colour of the rings.
    pub ring_color: Color,
    /// Default outline width of a series, in logical pixels.
    pub line_width_px: f32,
    /// Alpha of a series' fill when its style names no fill colour.
    pub fill_opacity: f32,
    /// Default marker at every vertex.
    pub marker: RadarMarker,
    /// Default marker diameter (circle) or side length (square) in logical
    /// pixels.
    pub marker_size_px: f32,
    /// Added to the colour of the hovered or selected series.
    pub emphasis_brightness: f32,
}
impl Default for RadarStyle {
    fn default() -> Self {
        Self {
            start_angle_degrees: -90.0,
            clockwise: true,
            grid_shape: RadarGridShape::Polygon,
            spoke_color: Color::from_rgb8(200, 206, 214),
            ring_color: Color::from_rgb8(224, 229, 235),
            line_width_px: RadarSeriesStyle::DEFAULT_LINE_WIDTH_PX,
            fill_opacity: RadarSeriesStyle::DEFAULT_FILL_OPACITY,
            marker: RadarSeriesStyle::DEFAULT_MARKER,
            marker_size_px: RadarSeriesStyle::DEFAULT_MARKER_SIZE_PX,
            emphasis_brightness: RadarSeriesStyle::DEFAULT_EMPHASIS_BRIGHTNESS,
        }
    }
}

/// One series: a closed polygon with one value per category.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RadarSeries {
    /// Stable identity used by targets and the web wrapper.
    pub id: String,
    /// Legend text.
    pub label: String,
    /// `values[i]` belongs to `categories[i]`. A non-finite value breaks the
    /// outline at that spoke and drops the series' fill.
    pub values: Vec<f64>,
    /// The outline and marker colour; the fill uses it at `fill_opacity`
    /// unless the series style names a fill.
    pub color: Color,
    /// Per-series overrides of the chart style.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub style: Option<RadarSeriesStyle>,
}
impl RadarSeries {
    /// A series in the chart style, with no overrides.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        values: Vec<f64>,
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

/// A legacy radar chart document. [`Self::validate`] checks it, and
/// `Config::from_radar` opens it as a polar chart of radar series.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct RadarChart {
    /// Editable decoration positions and selection, in logical pixels.
    #[cfg_attr(feature = "serde", serde(default))]
    pub decorations: crate::DecorationLayout,
    /// Legacy plain title, used only when `chart_title` is absent.
    pub title: String,
    /// Independent rich title. Position remains in `decorations`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub chart_title: Option<crate::BoundedTitleOptions>,
    /// The layer that receives pointer interaction. Bounded charts have no
    /// overlays, so only `Base` and `Annotations` are meaningful here.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "crate::LayerRef::is_base")
    )]
    pub active_layer: crate::LayerRef,
    /// The dedicated annotation layer, shared with every chart family.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "crate::AnnotationLayer::is_default")
    )]
    pub annotations: crate::AnnotationLayer,
    /// One spoke per category, in drawing order from `start_angle_degrees`.
    pub categories: Vec<Category>,
    /// The series, drawn in order; every fill before any outline.
    pub series: Vec<RadarSeries>,
    /// Chart-wide appearance and series defaults.
    pub style: RadarStyle,
    /// The hovered vertex or series. Its series is brightened by
    /// `emphasis_brightness`, and a hovered vertex grows its marker like a
    /// selected one.
    pub hovered: Option<RadarTarget>,
    /// The selected vertex or series; a selected vertex grows its marker.
    pub selected: Option<RadarTarget>,
    /// Which vertices carry data labels.
    pub labels: RadarLabels,
    /// Decimals of the data labels, at most 6. The value axis labels ignore
    /// it and show each ring value with trailing zeros trimmed.
    pub label_decimals: usize,
    /// Appended to every data label, such as a unit.
    pub value_suffix: String,
    /// The one value axis every spoke shares, from the centre (`[0]`) to the
    /// outer ring (`[1]`): the fixed radius axis of the chart
    /// [`Config::from_radar`](crate::config::Config::from_radar) reads.
    /// `None` leaves the range to the host's fit (the browser wrapper fits
    /// the supplied values with a nice range starting at zero when every
    /// value is non-negative). A value below the start is not drawn, so the
    /// outline breaks there; a value above the end is clipped at the outer
    /// ring.
    pub value_range: Option<[f64; 2]>,
    /// Ring spacing in value units: the radius axis' major spacing, under
    /// that axis' tick limits. `None` picks a nice spacing for about five
    /// rings.
    pub ring_spacing: Option<f64>,
    /// Whether rings and spokes are drawn.
    pub grid: bool,
    /// Whether the legend is drawn.
    pub legend: bool,
    /// Font family of every text; empty for the bundled default.
    pub font_family: String,
    /// Base font size of labels, in logical pixels.
    pub font_size: f32,
    /// Colour of every text.
    pub text_color: Color,
    /// The frame's clear colour.
    pub background: Color,
}
impl Default for RadarChart {
    fn default() -> Self {
        Self {
            decorations: Default::default(),
            title: String::new(),
            chart_title: None,
            active_layer: Default::default(),
            annotations: Default::default(),
            categories: vec![],
            series: vec![],
            style: RadarStyle::default(),
            hovered: None,
            selected: None,
            labels: RadarLabels::None,
            label_decimals: 1,
            value_suffix: String::new(),
            value_range: None,
            ring_spacing: None,
            grid: true,
            legend: true,
            font_family: "sans-serif".into(),
            font_size: 14.0,
            text_color: Color::from_rgb8(32, 46, 60),
            background: Color::WHITE,
        }
    }
}
impl RadarChart {
    /// The layer that receives pointer interaction. Bounded charts have no
    /// overlays or subplots, so an `Overlay`, `Subplot` or `SubplotOverlay`
    /// value reads as `Base`.
    pub fn active_layer(&self) -> crate::LayerRef {
        self.active_layer.bounded()
    }
    /// Set the interaction target layer and return the stored value
    /// (`Overlay`, `Subplot` and `SubplotOverlay` are stored as `Base`). A
    /// change clears `decorations.selected`, which names an element of the
    /// previous layer's pointer route; the data selection is kept.
    pub fn set_active_layer(&mut self, layer: crate::LayerRef) -> crate::LayerRef {
        crate::layer::set_bounded_active_layer(&mut self.active_layer, &mut self.decorations, layer)
    }
    /// The title to draw: `chart_title`, or the legacy `title` in the chart
    /// font and colour.
    pub fn resolved_title(&self) -> crate::BoundedTitleOptions {
        self.chart_title.clone().unwrap_or_else(|| {
            crate::BoundedTitleOptions::from_legacy(
                &self.title,
                &self.font_family,
                self.font_size,
                self.text_color,
            )
        })
    }
    /// Copy the legacy `title` into `chart_title` in the current chart font
    /// and colour, so later edits to `font_family`, `font_size` or
    /// `text_color` no longer change the title. Does nothing when
    /// `chart_title` is already set.
    pub fn normalize_title(&mut self) {
        if self.chart_title.is_none() {
            self.chart_title = Some(self.resolved_title());
        }
    }
    /// The series with `id`.
    pub fn series(&self, id: &str) -> Option<&RadarSeries> {
        self.series.iter().find(|s| s.id == id)
    }
    /// The value at `target`'s vertex, or `None` for a series target, an
    /// unknown id or a category the series has no value for.
    pub fn value(&self, target: &RadarTarget) -> Option<f64> {
        let category = target.category_id.as_deref()?;
        let i = self.categories.iter().position(|c| c.id == category)?;
        self.series(&target.series_id)?.values.get(i).copied()
    }
    /// Whether `target` names something this chart draws.
    pub fn contains(&self, target: &RadarTarget) -> bool {
        match target.category_id.as_deref() {
            None => self.series(&target.series_id).is_some(),
            Some(_) => self.value(target).is_some(),
        }
    }
    /// The resolved outline width, fill colour, marker and marker size of a
    /// series: its own style where set, else the chart style.
    pub fn resolved_style(&self, series: &RadarSeries) -> (f32, Color, RadarMarker, f32) {
        let style = series
            .style
            .clone()
            .unwrap_or_default()
            .resolved_from(&self.style);
        (
            style.line_width(),
            style.fill_color(series.color),
            style.marker(),
            style.marker_size(),
        )
    }
    /// Whether the hovered or selected target emphasizes `series_id`.
    pub fn is_emphasized(&self, series_id: &str) -> bool {
        [&self.hovered, &self.selected]
            .into_iter()
            .flatten()
            .any(|t| t.series_id == series_id)
    }
    /// Reorder the spokes to `ids`, a permutation of the category ids,
    /// moving every series' values with them.
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
    /// Check the limits and invariants the renderer relies on: category and
    /// series counts, unique ids, one value per category, finite style
    /// numbers, a valid value axis and targets that exist.
    pub fn validate(&self) -> Result<(), &'static str> {
        self.decorations.validate()?;
        self.annotations.validate()?;
        self.resolved_title().validate()?;
        let (n, m) = (self.categories.len(), self.series.len());
        if !(MIN_RADAR_CATEGORIES..=MAX_RADAR_CATEGORIES).contains(&n)
            || m == 0
            || m > MAX_RADAR_SERIES
        {
            return Err("radar limits: 3..64 categories and 1..16 series");
        }
        let mut ids = HashSet::new();
        for c in &self.categories {
            if c.id.is_empty() || !text(&c.id, 80) || !text(&c.label, 80) || !ids.insert(&c.id) {
                return Err("invalid category ID/label");
            }
        }
        ids.clear();
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
                st.validate()?;
            }
        }
        let st = &self.style;
        if !st.start_angle_degrees.is_finite()
            || !range(st.line_width_px, 0.0, 12.0)
            || !range(st.fill_opacity, 0.0, 1.0)
            || !range(st.marker_size_px, 1.0, 24.0)
            || !range(st.emphasis_brightness, 0.0, 0.5)
            || !color(st.spoke_color)
            || !color(st.ring_color)
        {
            return Err("invalid radar style");
        }
        if !range(self.font_size, 8.0, 32.0)
            || !color(self.text_color)
            || !color(self.background)
            || self.label_decimals > 6
            || !text(&self.font_family, 160)
            || !text(&self.value_suffix, 24)
            || !text(&self.title, 160)
        {
            return Err("invalid radar layout, font or colors");
        }
        if self
            .value_range
            .is_some_and(|r| !r[0].is_finite() || !r[1].is_finite() || r[0] >= r[1])
        {
            return Err("invalid value range");
        }
        if self
            .ring_spacing
            .is_some_and(|s| !s.is_finite() || s <= 0.0)
        {
            return Err("ring spacing must be finite and positive");
        }
        for t in [&self.hovered, &self.selected].into_iter().flatten() {
            if !self.contains(t) {
                return Err("unknown interaction target");
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

#[cfg(test)]
mod tests {
    use super::*;
    fn chart() -> RadarChart {
        RadarChart {
            categories: vec![
                Category::new("a", "A"),
                Category::new("b", "B"),
                Category::new("c", "C"),
            ],
            series: vec![RadarSeries::new(
                "s",
                "S",
                vec![1.0, 2.0, 3.0],
                Color::WHITE,
            )],
            ..Default::default()
        }
    }
    #[test]
    fn limits_and_value_counts_are_validated() {
        let mut c = chart();
        assert!(c.validate().is_ok());
        c.categories.pop();
        assert!(c.validate().is_err(), "two categories are not a polygon");
        c = chart();
        c.series[0].values.pop();
        assert!(c.validate().is_err(), "one value per category");
        c = chart();
        c.categories = (0..65).map(|i| Category::new(i.to_string(), "A")).collect();
        c.series[0].values = vec![1.0; 65];
        assert!(c.validate().is_err());
        c.categories.pop();
        c.series[0].values.pop();
        assert!(c.validate().is_ok());
        c.series = (0..17)
            .map(|i| RadarSeries::new(i.to_string(), "S", vec![1.0; 64], Color::WHITE))
            .collect();
        assert!(c.validate().is_err());
        c.series.pop();
        assert!(c.validate().is_ok());
    }
    #[test]
    fn non_finite_values_are_allowed_as_gaps() {
        let mut c = chart();
        c.series[0].values[1] = f64::NAN;
        assert!(c.validate().is_ok());
        assert!(c.value(&RadarTarget::vertex("s", "b")).unwrap().is_nan());
    }
    #[test]
    fn targets_name_a_series_or_one_of_its_vertices() {
        let mut c = chart();
        assert!(c.contains(&RadarTarget::series("s")));
        assert!(c.contains(&RadarTarget::vertex("s", "b")));
        assert!(!c.contains(&RadarTarget::vertex("s", "nope")));
        assert!(!c.contains(&RadarTarget::series("nope")));
        assert_eq!(c.value(&RadarTarget::vertex("s", "c")), Some(3.0));
        assert_eq!(c.value(&RadarTarget::series("s")), None);
        c.selected = Some(RadarTarget::vertex("s", "nope"));
        assert!(c.validate().is_err());
        c.selected = Some(RadarTarget::series("s"));
        assert!(c.validate().is_ok());
        assert!(c.is_emphasized("s"));
        assert!(!c.is_emphasized("t"));
    }
    #[test]
    fn resolved_style_falls_back_to_the_chart_style() {
        let mut c = chart();
        let (width, fill, marker, size) = c.resolved_style(&c.series[0]);
        assert_eq!(width, 2.0);
        assert_eq!(fill.a, 0.25);
        assert_eq!(marker, RadarMarker::Circle);
        assert_eq!(size, 7.0);
        c.series[0].style = Some(RadarSeriesStyle {
            line_width_px: Some(4.0),
            fill: Some(Color::new(0.0, 0.0, 0.0, 0.0)),
            marker: Some(RadarMarker::None),
            ..Default::default()
        });
        let (width, fill, marker, size) = c.resolved_style(&c.series[0]);
        assert_eq!(
            (width, fill.a, marker, size),
            (4.0, 0.0, RadarMarker::None, 7.0)
        );
        c.series[0].style.as_mut().unwrap().line_width_px = Some(13.0);
        assert!(c.validate().is_err());
    }
    #[test]
    fn ranges_and_style_are_validated() {
        let mut c = chart();
        c.value_range = Some([1.0, 1.0]);
        assert!(c.validate().is_err());
        c.value_range = Some([-1.0, 5.0]);
        assert!(c.validate().is_ok());
        c.ring_spacing = Some(0.0);
        assert!(c.validate().is_err());
        c.ring_spacing = Some(0.5);
        assert!(c.validate().is_ok());
        c.style.fill_opacity = 1.5;
        assert!(c.validate().is_err());
        c = chart();
        c.style.start_angle_degrees = f32::NAN;
        assert!(c.validate().is_err());
    }
    #[test]
    fn reorder_is_atomic_and_moves_values_with_their_categories() {
        let mut c = chart();
        let before = c.clone();
        assert!(c.reorder_categories(&["a", "a", "b"]).is_err());
        assert_eq!(c, before);
        c.reorder_categories(&["c", "a", "b"]).unwrap();
        assert_eq!(c.series[0].values, vec![3.0, 1.0, 2.0]);
        assert_eq!(c.value(&RadarTarget::vertex("s", "c")), Some(3.0));
    }
    #[cfg(feature = "serde")]
    #[test]
    fn complete_ssot_round_trips_and_omits_defaults() {
        let mut c = chart();
        c.series[0].style = Some(RadarSeriesStyle {
            marker: Some(RadarMarker::Square),
            ..Default::default()
        });
        c.style.grid_shape = RadarGridShape::Circle;
        c.selected = Some(RadarTarget::vertex("s", "b"));
        c.hovered = Some(RadarTarget::series("s"));
        c.value_range = Some([0.0, 5.0]);
        let json = serde_json::to_string(&c).unwrap();
        let back: RadarChart = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
        back.validate().unwrap();
        let minimal: RadarChart = serde_json::from_str(
            r#"{"categories":[{"id":"a","label":"A"},{"id":"b","label":"B"},{"id":"c","label":"C"}],
                "series":[{"id":"s","label":"S","values":[1,2,3],"color":{"r":1,"g":1,"b":1,"a":1}}]}"#,
        )
        .unwrap();
        assert_eq!(minimal, chart());
        let series_target = serde_json::to_string(&RadarTarget::series("s")).unwrap();
        assert_eq!(series_target, r#"{"series_id":"s"}"#);
    }
}

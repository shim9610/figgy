//! Ternary coordinates for the Cartesian chart: `Config.ternary`.
//!
//! With `ternary: Some(..)` the chart draws an equilateral triangle in its
//! data area and places each sample of a [`DataRenderType::Ternary`] series
//! by its three components: `(x·L + y·T + c·R) / (x + y + c)`, the
//! triangle's corners weighted by the component columns. The host supplies
//! the three columns unnormalized; normalizing only to place a sample is
//! appearance (`CLAUDE.md` rule 1), computed in the vertex stage on every
//! draw and never stored. Hover, selection and the reported values are the
//! original components.
//!
//! The convention is fixed: `x_column` is read on `bottom_x`, whose 100 %
//! corner is the bottom-left (L); `y_column` on `left_y`, the top (T);
//! `c_column` on `right_y`, the bottom-right (R). Each axis increases
//! clockwise towards its own corner: x along the bottom edge from R to L,
//! y along the left edge from L to T, c along the right edge from T to R.
//! The three axes span the whole triangle: `min` 0 and the same `max` (1
//! or 100, which only scales the labels). `top_x` stays in the document
//! but is not drawn.
//!
//! A series may also fill closed regions ([`TernaryRegionStyle`]), drawn
//! under every series' markers and lines through an even-odd stencil.
//!
//! A ternary chart draws only ternary series, in the Precise draw style;
//! overlays, line offsets, the polar system, subplot axis links and
//! streamed sources are rejected. `None` (the default, omitted from JSON)
//! draws the rectangular chart.

use crate::Color;
use crate::config::{AxisOptions, AxisScale, Config};
use crate::data_config::{DataRenderType, LineInterpolation, SeriesConfig};
use crate::layout::{Side, TitlePlacement};
use crate::text::TextExtents;

/// The ternary coordinate system of a `Config`. It has no field in this
/// release: it is the switch that turns the chart into a triangle, and
/// later options (such as a partial triangle) join it additively.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct TernaryOptions {}

/// The fill of the closed regions a ternary series outlines: phase fields,
/// soil-texture classes, rock classification areas. The host gives each
/// polygon's vertices in order in the series' x, y and c columns, a NaN row
/// separating polygons within one series; the renderer closes each polygon
/// from its last vertex to its first and creates no vertex of its own. The
/// fill is painted with an even-odd stencil over the whole series, so a
/// polygon inside another is a hole and two polygons that overlap leave
/// their overlap unfilled: regions that should overlap go in separate
/// series. Drawn under every series' markers and lines, in series order,
/// as a line's area fill is.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct TernaryRegionStyle {
    /// The fill colour; its alpha multiplies `opacity`.
    pub color: Color,
    /// `0..=1`.
    pub opacity: f32,
}

impl Default for TernaryRegionStyle {
    fn default() -> Self {
        Self {
            color: Color::new(0.3, 0.5, 0.8, 1.0),
            opacity: 0.35,
        }
    }
}

impl TernaryRegionStyle {
    /// The premultiplied colour the fill paints: the colour at its alpha
    /// times `opacity`.
    pub fn color_premul(&self) -> [f32; 4] {
        let a = self.color.a.clamp(0.0, 1.0) * self.opacity.clamp(0.0, 1.0);
        [self.color.r * a, self.color.g * a, self.color.b * a, a]
    }
}

/// The triangle a ternary chart draws into, in the chart's pixel space
/// (y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TernaryGeometry {
    /// The corners L (bottom-left, x's 100 %), T (top, y's 100 %) and R
    /// (bottom-right, c's 100 %).
    pub corners: [[f32; 2]; 3],
    /// The side length.
    pub side: f32,
}

impl TernaryGeometry {
    /// The pixel position of the fractions `[x, y, c]` (summing to 1).
    pub fn point(&self, fractions: [f32; 3]) -> [f32; 2] {
        let [l, t, r] = self.corners;
        [
            fractions[0] * l[0] + fractions[1] * t[0] + fractions[2] * r[0],
            fractions[0] * l[1] + fractions[1] * t[1] + fractions[2] * r[1],
        ]
    }

    /// The centroid.
    pub fn centroid(&self) -> [f32; 2] {
        self.point([1.0 / 3.0; 3])
    }

    /// The edge component `k` (0 x, 1 y, 2 c) is read along, as `[from,
    /// to]`: from its 0 to its 100 % corner. x runs along the bottom edge
    /// from R to L, y along the left edge from L to T, c along the right
    /// edge from T to R.
    pub fn edge(&self, k: usize) -> [[f32; 2]; 2] {
        let [l, t, r] = self.corners;
        match k {
            0 => [r, l],
            1 => [l, t],
            _ => [t, r],
        }
    }

    /// The unit direction of edge `k`, from its 0 to its 100 % corner.
    pub fn edge_direction(&self, k: usize) -> [f32; 2] {
        let [a, b] = self.edge(k);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy).max(f32::EPSILON);
        [dx / len, dy / len]
    }

    /// The unit outward normal of edge `k` (pointing away from the
    /// triangle).
    pub fn outward_normal(&self, k: usize) -> [f32; 2] {
        let [a, b] = self.edge(k);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy).max(f32::EPSILON);
        let mut n = [dy / len, -dx / len];
        let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
        let c = self.centroid();
        if (mid[0] - c[0]) * n[0] + (mid[1] - c[1]) * n[1] < 0.0 {
            n = [-n[0], -n[1]];
        }
        n
    }
}

/// The side of component `k`'s axis: x `Bottom`, y `Left`, c `Right`.
pub fn component_side(k: usize) -> Side {
    match k {
        0 => Side::Bottom,
        1 => Side::Left,
        _ => Side::Right,
    }
}

/// The component of the axis on `side`, `None` for the undrawn top axis.
pub fn side_component(side: &Side) -> Option<usize> {
    match side {
        Side::Bottom => Some(0),
        Side::Left => Some(1),
        Side::Right => Some(2),
        Side::Top => None,
    }
}

/// The band outside an edge, in pixels: the tick and a line of labels with
/// their clearance, then the axis title. Every term is a size
/// `Config::scaled` scales, so a 2x export is the 1x layout doubled.
fn edge_band(axis: &AxisOptions) -> (f32, f32) {
    let labels = &axis.label_style;
    let font = labels.font_size.max(0.0);
    let tick = axis.major_tick_length.max(0.0);
    let text = if labels.visible && labels.label_visible {
        font * 1.4 + font / 3.0
    } else {
        font / 9.0
    };
    let title = &axis.title_option;
    let title = if title.visible && !title.text.segments.is_empty() {
        title.text.font_size.max(0.0) * 1.4
    } else {
        0.0
    };
    (tick + text, title)
}

/// The axis component `k` is read on: 0 `bottom_x`, 1 `left_y`, 2
/// `right_y`.
pub fn ternary_axis(config: &Config, k: usize) -> &AxisOptions {
    match k {
        0 => &config.bottom_x,
        1 => &config.left_y,
        _ => &config.right_y,
    }
}

impl TernaryOptions {
    /// Validate against the axes the coordinate system reads.
    pub fn validate(&self, config: &Config) -> Result<(), &'static str> {
        let axes = [&config.bottom_x, &config.left_y, &config.right_y];
        if axes.iter().any(|a| a.scale != AxisScale::Linear) {
            return Err("ternary axes (bottom_x, left_y, right_y) must be linear");
        }
        if axes.iter().any(|a| a.inverted) {
            return Err("ternary axes cannot be inverted: each increases clockwise");
        }
        if axes.iter().any(|a| a.categories.is_some()) {
            return Err("ternary axes cannot be categorical");
        }
        let max = config.bottom_x.max;
        if axes
            .iter()
            .any(|a| a.min != 0.0 || a.max != max || !(a.max.is_finite() && a.max > 0.0))
        {
            return Err(
                "ternary axes span the whole triangle: every axis has min 0 and the same \
                 positive max (1 or 100)",
            );
        }
        if config.polar.is_some() {
            return Err("a chart is either polar or ternary");
        }
        if !config.overlays.is_empty() {
            return Err("ternary charts have no overlay layers");
        }
        if config
            .line_offsets
            .as_ref()
            .is_some_and(|offsets| !offsets.series.is_empty())
        {
            return Err("ternary charts have no line offsets");
        }
        if !config.draw_style.is_precise() {
            return Err("ternary charts are drawn in the Precise style only");
        }
        if config
            .subplots
            .iter()
            .any(|s| s.link_x.is_some() || s.link_y.is_some())
        {
            return Err("ternary charts take no subplot axis links");
        }
        Ok(())
    }
}

impl Config {
    /// The triangle of this ternary chart, when the chart is ternary and has
    /// a data area: the largest equilateral triangle inside the data area
    /// that leaves a band for each edge's ticks, labels and title outside
    /// it, centred with its bands. The GPU transform, the grid and axes, the decoration hit
    /// regions and drags all read this triangle.
    pub fn ternary_geometry(&self) -> Option<TernaryGeometry> {
        self.ternary.as_ref()?;
        let area = self.data_area().ok()?.0;
        // One band per edge, the widest of the three.
        let band = (0..3)
            .map(|k| {
                let (labels, title) = edge_band(ternary_axis(self, k));
                labels + title
            })
            .fold(0.0f32, f32::max);
        // The band outside each edge reaches past the triangle's bounding
        // box by `band · cos 30°` left and right (the slanted edges), by
        // `band` below and by `band · sin 30°` above the top corner. The
        // triangle is the largest whose box grown by those reaches fits the
        // data area, and that grown box is centred.
        let (cos30, sin30) = (3f32.sqrt() * 0.5, 0.5);
        let width = (area.width as f32 - 2.0 * cos30 * band).max(0.0);
        let height = (area.height as f32 - (1.0 + sin30) * band).max(0.0);
        let side = width.min(height / cos30);
        let h = side * cos30;
        let cx = area.x as f32 + area.width as f32 * 0.5;
        let top =
            area.y as f32 + (area.height as f32 - h - (1.0 + sin30) * band) * 0.5 + sin30 * band;
        Some(TernaryGeometry {
            corners: [
                [cx - side * 0.5, top + h],
                [cx, top],
                [cx + side * 0.5, top + h],
            ],
            side,
        })
    }

    /// Where the title of component `k`'s axis sits: centred past the
    /// edge's tick and label band along the outward normal, parallel to the
    /// edge and reading left to right, moved by the title offsets in screen
    /// space. `None` when the chart is not ternary.
    pub fn ternary_title_placement(
        &self,
        k: usize,
        offset: (f32, f32),
        extents: TextExtents,
    ) -> Option<TitlePlacement> {
        let g = self.ternary_geometry()?;
        let (labels, _) = edge_band(ternary_axis(self, k));
        let [a, b] = g.edge(k);
        let n = g.outward_normal(k);
        let reach = labels + extents.height() * 0.5;
        let center = (
            (a[0] + b[0]) * 0.5 + n[0] * reach + offset.0,
            (a[1] + b[1]) * 0.5 + n[1] * reach + offset.1,
        );
        // The edge's direction, turned to read left to right.
        let (mut dx, mut dy) = (b[0] - a[0], b[1] - a[1]);
        if dx < 0.0 {
            (dx, dy) = (-dx, -dy);
        }
        Some(TitlePlacement {
            origin: (
                center.0 - extents.width * 0.5,
                center.1 + (extents.ascent - extents.descent) * 0.5,
            ),
            rotation_center: center,
            rotation_degrees: dy.atan2(dx).to_degrees(),
        })
    }

    /// Validate `ternary` against the axes it reads; `None` always passes.
    pub fn validate_ternary(&self) -> Result<(), &'static str> {
        match &self.ternary {
            Some(ternary) => ternary.validate(self),
            None => Ok(()),
        }
    }

    /// The rules the series of a chart are held to with the ternary system:
    /// a ternary chart draws only [`DataRenderType::Ternary`] series, and a
    /// ternary series needs a ternary chart. A ternary series names three
    /// distinct component columns and draws markers, a line or both; its
    /// line is linear (no step modes) and has no fill.
    pub fn validate_ternary_series(&self, series: &[SeriesConfig]) -> Result<(), &'static str> {
        for item in series {
            match (&item.render_type, self.ternary.is_some()) {
                (DataRenderType::Ternary { .. }, false) => {
                    return Err("a ternary series is drawn on a ternary chart (Config.ternary)");
                }
                (
                    DataRenderType::Ternary {
                        c_column,
                        scatter,
                        line,
                        region,
                    },
                    true,
                ) => {
                    if item.x_column.is_empty()
                        || item.y_column.is_empty()
                        || c_column.is_empty()
                        || item.x_column == item.y_column
                        || item.x_column == *c_column
                        || item.y_column == *c_column
                    {
                        return Err(
                            "a ternary series names three distinct component columns (x_column, \
                             y_column, c_column)",
                        );
                    }
                    if scatter.is_none() && line.is_none() && region.is_none() {
                        return Err(
                            "a ternary series draws markers, a line, a region fill or any of them",
                        );
                    }
                    if let Some(region) = region {
                        let finite = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
                        if !finite(region.opacity)
                            || !finite(region.color.r)
                            || !finite(region.color.g)
                            || !finite(region.color.b)
                            || !finite(region.color.a)
                        {
                            return Err(
                                "a ternary region fill's colour and opacity are finite and in 0..=1",
                            );
                        }
                    }
                    if let Some(line) = line {
                        if line.interpolation != LineInterpolation::Linear {
                            return Err("a ternary line is linear: step modes are not drawn");
                        }
                        if line.fill.is_some() {
                            return Err("a ternary line has no fill");
                        }
                    }
                }
                (_, true) => return Err("a ternary chart draws only ternary series"),
                (_, false) => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DataSelectionsConfig, PickedDataRef};
    use crate::data_config::{
        DataLineStyleConfig, DataScatterStyleConfig, LineFill, LineInterpolation,
    };
    use crate::layer::{OverlayLayer, Subplot};
    use crate::line::LineStylePreset;
    use crate::{Color, ScatterShape};

    fn chart() -> Config {
        let mut config = crate::default::default_config();
        for axis in [
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.min = 0.0;
            axis.max = 100.0;
        }
        config.ternary = Some(TernaryOptions {});
        config
    }

    fn scatter() -> DataScatterStyleConfig {
        DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::CircleFilled,
            point_size: 4.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        }
    }

    fn line() -> DataLineStyleConfig {
        DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 1.0,
            fill: None,
            fill_color: None,
            interpolation: LineInterpolation::Linear,
        }
    }

    fn series(id: &str, render_type: DataRenderType) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "a".into(),
            y_column: "b".into(),
            z_column: None,
            render_type,
        }
    }

    fn ternary(
        scatter: Option<DataScatterStyleConfig>,
        line: Option<DataLineStyleConfig>,
    ) -> DataRenderType {
        DataRenderType::Ternary {
            c_column: "c".into(),
            scatter,
            line,
            region: None,
        }
    }

    #[test]
    fn geometry_is_an_equilateral_triangle_centred_in_the_data_area() {
        let config = chart();
        let g = config.ternary_geometry().unwrap();
        let area = config.data_area().unwrap().0;
        let [l, t, r] = g.corners;
        for (a, b) in [(l, t), (t, r), (r, l)] {
            let side = (b[0] - a[0]).hypot(b[1] - a[1]);
            assert!((side - g.side).abs() < 1e-3, "{side} vs {}", g.side);
        }
        assert_eq!(l[1], r[1], "the bottom edge is horizontal");
        let cx = area.x as f32 + area.width as f32 * 0.5;
        assert!((t[0] - cx).abs() < 1e-3);
        for p in g.corners {
            assert!(p[0] >= area.x as f32 && p[0] <= (area.x + area.width) as f32);
            assert!(p[1] >= area.y as f32 && p[1] <= (area.y + area.height) as f32);
        }
        // Each component's 100 % corner and its outward normals.
        assert_eq!(g.point([1.0, 0.0, 0.0]), l);
        assert_eq!(g.point([0.0, 1.0, 0.0]), t);
        assert_eq!(g.point([0.0, 0.0, 1.0]), r);
        assert_eq!(
            g.outward_normal(0),
            [0.0, 1.0],
            "the bottom edge points down"
        );
        assert!(g.outward_normal(1)[0] < 0.0 && g.outward_normal(2)[0] > 0.0);
        // No triangle without the switch.
        let mut plain = config.clone();
        plain.ternary = None;
        assert!(plain.ternary_geometry().is_none());
    }

    #[test]
    fn the_triangle_scales_with_the_chart_at_2x() {
        let config = chart();
        let one = config.ternary_geometry().unwrap();
        let two = config.scaled(2.0).ternary_geometry().unwrap();
        for (a, b) in one.corners.iter().zip(two.corners.iter()) {
            assert!((a[0] * 2.0 - b[0]).abs() < 0.01 && (a[1] * 2.0 - b[1]).abs() < 0.01);
        }
    }

    #[test]
    fn validation_names_each_rejection() {
        assert_eq!(chart().validate_ternary(), Ok(()));
        let reject = |edit: &dyn Fn(&mut Config)| {
            let mut config = chart();
            edit(&mut config);
            config.validate_ternary().unwrap_err()
        };
        assert!(reject(&|c| c.left_y.scale = AxisScale::Logarithmic).contains("linear"));
        assert!(reject(&|c| c.right_y.inverted = true).contains("inverted"));
        assert!(reject(&|c| c.bottom_x.min = 10.0).contains("min 0"));
        assert!(reject(&|c| c.right_y.max = 1.0).contains("same"));
        assert!(
            reject(&|c| c.polar = Some(crate::polar::PolarOptions::default()))
                .contains("polar or ternary")
        );
        assert!(reject(&|c| c.overlays.push(OverlayLayer::default())).contains("overlay"));
        assert!(
            reject(&|c| c.draw_style = crate::config::DrawStyle::Sketch(Default::default()))
                .contains("Precise")
        );
        let mut linked = Subplot::default();
        linked.link_x = Some(crate::layer::CellRef::Parent);
        assert!(reject(&|c| c.subplots.push(linked.clone())).contains("links"));
        // The axes may span 0..1 as well as 0..100.
        let mut unit = chart();
        for axis in [&mut unit.bottom_x, &mut unit.left_y, &mut unit.right_y] {
            axis.max = 1.0;
        }
        assert_eq!(unit.validate_ternary(), Ok(()));
    }

    #[test]
    fn series_rules_are_checked_both_ways() {
        let config = chart();
        let ok = series("t", ternary(Some(scatter()), Some(line())));
        assert_eq!(config.validate_ternary_series(&[ok.clone()]), Ok(()));
        let err = |s: SeriesConfig| config.validate_ternary_series(&[s]).unwrap_err();
        assert!(err(series("t", ternary(None, None))).contains("markers, a line, a region fill"));
        let mut filled_region = ternary(None, None);
        if let DataRenderType::Ternary { region, .. } = &mut filled_region {
            *region = Some(TernaryRegionStyle::default());
        }
        assert_eq!(
            config.validate_ternary_series(&[series("t", filled_region.clone())]),
            Ok(()),
            "a region alone draws"
        );
        if let DataRenderType::Ternary { region, .. } = &mut filled_region {
            *region = Some(TernaryRegionStyle {
                opacity: 2.0,
                ..TernaryRegionStyle::default()
            });
        }
        assert!(err(series("t", filled_region)).contains("0..=1"));
        let mut filled = line();
        filled.fill = Some(LineFill::ToBaseline(0.0));
        assert!(err(series("t", ternary(None, Some(filled)))).contains("no fill"));
        let mut stepped = line();
        stepped.interpolation = LineInterpolation::StepPost;
        assert!(err(series("t", ternary(None, Some(stepped)))).contains("linear"));
        let mut same = ok.clone();
        same.y_column = "c".into();
        assert!(err(same).contains("three distinct"));
        let plain = series("p", DataRenderType::Scatter { scatter: scatter() });
        assert!(err(plain).contains("only ternary series"));
        let mut rectangular = config.clone();
        rectangular.ternary = None;
        assert!(
            rectangular
                .validate_ternary_series(&[ok])
                .unwrap_err()
                .contains("on a ternary chart")
        );
    }

    #[test]
    fn step_selection_walks_points_to_the_shortest_column() {
        let mut config = chart();
        let s = [series("t", ternary(Some(scatter()), None))];
        config.picked_data = Some(DataSelectionsConfig {
            refs: vec![PickedDataRef::Point {
                source_id: None,
                series_id: "t".into(),
                point_index: 1,
            }],
            ..DataSelectionsConfig::default()
        });
        let len = |id: &str| match id {
            "a" => Some(10),
            "b" => Some(8),
            "c" => Some(3),
            _ => None,
        };
        assert_eq!(config.step_selection(&s, 1, 0, len), Ok(true));
        assert_eq!(
            config.step_selection(&s, 5, 0, len),
            Ok(false),
            "c has 3 values"
        );
        let Some(PickedDataRef::Point { point_index, .. }) =
            config.picked_data.as_ref().and_then(|p| p.refs.first())
        else {
            panic!("a point selection");
        };
        assert_eq!(*point_index, 2);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serialization_round_trips_and_omits_the_switch_when_off() {
        let config = chart();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"ternary\":{}"), "{json}");
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ternary, Some(TernaryOptions {}));
        let mut plain = config.clone();
        plain.ternary = None;
        let json = serde_json::to_string(&plain).unwrap();
        assert!(
            !json.contains("ternary"),
            "an older document stays as it was"
        );
        let s = series("t", ternary(Some(scatter()), None));
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            json.contains("\"c_column\":\"c\"") && !json.contains("\"line\""),
            "{json}"
        );
        let back: SeriesConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert!(!json.contains("\"region\""), "unset, omitted: {json}");
        // A region fill round-trips and reads with every field defaulted.
        let mut filled = ternary(None, None);
        if let DataRenderType::Ternary { region, .. } = &mut filled {
            *region = Some(TernaryRegionStyle {
                color: Color::new(0.1, 0.2, 0.3, 1.0),
                opacity: 0.5,
            });
        }
        let s = series("r", filled);
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            json.contains("\"region\":{") && json.contains("\"opacity\":0.5"),
            "{json}"
        );
        let back: SeriesConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        let partial: TernaryRegionStyle = serde_json::from_str("{\"opacity\":0.2}").unwrap();
        assert_eq!(partial.opacity, 0.2);
        assert_eq!(partial.color, TernaryRegionStyle::default().color);
    }

    #[test]
    fn titles_sit_past_each_edge_along_it() {
        let config = chart();
        let g = config.ternary_geometry().unwrap();
        let m = TextExtents {
            width: 40.0,
            ascent: 10.0,
            descent: 3.0,
        };
        for k in 0..3 {
            let p = config.ternary_title_placement(k, (0.0, 0.0), m).unwrap();
            let [a, b] = g.edge(k);
            let n = g.outward_normal(k);
            let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            let c = p.rotation_center;
            let out = (c.0 - mid[0]) * n[0] + (c.1 - mid[1]) * n[1];
            let along = (c.0 - mid[0]) * n[1] - (c.1 - mid[1]) * n[0];
            assert!(out > m.height() * 0.5, "{k}: {out}");
            assert!(along.abs() < 1e-3, "{k}: centred on the edge");
            assert!(p.rotation_degrees.abs() <= 90.0, "{k}: reads left to right");
        }
        assert_eq!(
            config
                .ternary_title_placement(0, (0.0, 0.0), m)
                .unwrap()
                .rotation_degrees,
            0.0
        );
        // The offset moves the title in screen space.
        let at = config.ternary_title_placement(1, (0.0, 0.0), m).unwrap();
        let moved = config.ternary_title_placement(1, (5.0, -7.0), m).unwrap();
        assert_eq!(
            moved.rotation_center,
            (at.rotation_center.0 + 5.0, at.rotation_center.1 - 7.0)
        );
    }
}

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
//! A ternary chart draws only ternary series, in the Precise draw style;
//! overlays, line offsets, the polar system, subplot axis links and
//! streamed sources are rejected. `None` (the default, omitted from JSON)
//! draws the rectangular chart.

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
    /// it, centred. The GPU transform, the grid and axes, the decoration hit
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
        let width = (area.width as f32 - 2.0 * band).max(0.0);
        let height = (area.height as f32 - 2.0 * band).max(0.0);
        let side = width.min(height * 2.0 / 3f32.sqrt());
        let h = side * 3f32.sqrt() * 0.5;
        let cx = area.x as f32 + area.width as f32 * 0.5;
        let cy = area.y as f32 + area.height as f32 * 0.5;
        Some(TernaryGeometry {
            corners: [
                [cx - side * 0.5, cy + h * 0.5],
                [cx, cy - h * 0.5],
                [cx + side * 0.5, cy + h * 0.5],
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
                    if scatter.is_none() && line.is_none() {
                        return Err("a ternary series draws markers, a line or both");
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

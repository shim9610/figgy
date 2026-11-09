//! Chainable modifiers for the parts one chart family owns (#265
//! follow-up): ternary regions, range bar parts, box plot violins and
//! points, field and mesh contours and surfaces, vector scales, and the
//! column swap of a horizontal histogram. Like the other modifiers, each
//! is a plain method on the document type; a modifier that does not apply
//! to a series' family leaves the series as it is.

use crate::color::Color;
use crate::data::ColumnId;
use crate::data_config::{
    BoxPlotOverride, BoxPoints, CategoryBarLabels, ContourConfig, DataRenderType, FieldSeries,
    RangeColor, RangeConnector, RangeProgress, RangeWhiskers, SeriesConfig, SizeScale, VectorScale,
    Violin, XySeries,
};
use crate::ternary::TernaryRegionStyle;
use crate::view3d::SurfaceStyle;

impl SeriesConfig {
    // Markers and regions.

    /// Drop the markers of an x-y or ternary series.
    pub fn no_markers(mut self) -> Self {
        match &mut self.render_type {
            DataRenderType::Xy(xy) => xy.scatter = None,
            DataRenderType::Ternary { scatter, .. } => *scatter = None,
            _ => {}
        }
        self
    }

    /// Fill the polygons a ternary series outlines in `color` at `opacity`
    /// (`0..=1`, multiplying the colour's alpha).
    pub fn region(mut self, color: Color, opacity: f32) -> Self {
        if let DataRenderType::Ternary { region, .. } = &mut self.render_type {
            *region = Some(TernaryRegionStyle { color, opacity });
        }
        self
    }

    /// The scale of a marker size map set with [`Self::size_by`]: values
    /// proportional to the marker's area or to its radius.
    pub fn size_scale(mut self, scale: SizeScale) -> Self {
        let scatter = match &mut self.render_type {
            DataRenderType::Xy(XySeries {
                scatter: Some(scatter),
                ..
            })
            | DataRenderType::Ternary {
                scatter: Some(scatter),
                ..
            } => scatter,
            _ => return self,
        };
        if let Some(size) = &mut scatter.point_size_map {
            size.scale = scale;
        }
        self
    }

    // Range bars.

    /// How each range bar is filled: the series colour, by direction, or
    /// by a host state column.
    pub fn range_colors(mut self, color_by: RangeColor) -> Self {
        if let DataRenderType::RangeBars { color_by: c, .. } = &mut self.render_type {
            *c = color_by;
        }
        self
    }

    /// Range bar whiskers from the `low` to the `high` column, `width`
    /// logical pixels wide, in the series colour.
    pub fn whiskers(
        mut self,
        low: impl Into<ColumnId>,
        high: impl Into<ColumnId>,
        width: f32,
    ) -> Self {
        if let DataRenderType::RangeBars { whiskers, .. } = &mut self.render_type {
            *whiskers = Some(RangeWhiskers {
                low: low.into(),
                high: high.into(),
                width,
                color: None,
            });
        }
        self
    }

    /// Range bar progress from the fraction `column`, the unfinished part
    /// drawn at `remaining_opacity` (`0..=1`).
    pub fn progress(mut self, column: impl Into<ColumnId>, remaining_opacity: f32) -> Self {
        if let DataRenderType::RangeBars { progress, .. } = &mut self.render_type {
            *progress = Some(RangeProgress {
                column: column.into(),
                remaining_opacity,
            });
        }
        self
    }

    /// A line from each range bar's end to the next bar (a waterfall), in
    /// `color` and `width` logical pixels wide.
    pub fn connector(mut self, color: Color, width: f32) -> Self {
        if let DataRenderType::RangeBars { connector, .. } = &mut self.render_type {
            *connector = Some(RangeConnector { color, width });
        }
        self
    }

    /// The slot of one range bar row on a numeric position axis, in data
    /// units.
    pub fn slot_width(mut self, width: f64) -> Self {
        if let DataRenderType::RangeBars { slot_width, .. } = &mut self.render_type {
            *slot_width = Some(width);
        }
        self
    }

    /// Where the value labels of a category or range bar series go, and the
    /// host's drawn `text`, one string per bar (moved in, not copied).
    pub fn bar_labels(mut self, placement: CategoryBarLabels, text: Vec<String>) -> Self {
        if let DataRenderType::CategoryBars { labels, .. }
        | DataRenderType::RangeBars { labels, .. } = &mut self.render_type
        {
            labels.placement = placement;
            labels.text = text;
        }
        self
    }

    // Box plots.

    /// The violin drawn behind each box of a box plot.
    pub fn violin(mut self, violin: Violin) -> Self {
        if let DataRenderType::BoxPlot { violin: v, .. } = &mut self.render_type {
            *v = Some(violin);
        }
        self
    }

    /// The raw points drawn beside each box of a box plot.
    pub fn box_points(mut self, points: BoxPoints) -> Self {
        if let DataRenderType::BoxPlot { points: p, .. } = &mut self.render_type {
            *p = Some(points);
        }
        self
    }

    /// The per-box overrides of a box plot, moved in as given.
    pub fn box_overrides(mut self, overrides: Vec<BoxPlotOverride>) -> Self {
        if let DataRenderType::BoxPlot { overrides: o, .. } = &mut self.render_type {
            *o = Some(overrides);
        }
        self
    }

    /// The ragged column of a box plot's outliers, one group per category.
    pub fn outliers(mut self, column: impl Into<ColumnId>) -> Self {
        if let DataRenderType::BoxPlot { outliers, .. } = &mut self.render_type {
            *outliers = Some(column.into());
        }
        self
    }

    // Fields and meshes.

    /// Edit the contours of a field or mesh that has them.
    pub fn style_contours(mut self, edit: impl FnOnce(&mut ContourConfig)) -> Self {
        if let Some(contour) = self.family_contour() {
            edit(contour);
        }
        self
    }

    /// The host's contour levels of a field or mesh that has contours,
    /// moved in as given.
    pub fn contour_levels(mut self, levels: Vec<f64>) -> Self {
        if let Some(contour) = self.family_contour() {
            contour.levels = levels;
        }
        self
    }

    /// Edit the 3D surface look of a filled field or mesh, starting from
    /// the default look when it has none. A field or mesh without a fill
    /// draws no surface and is left as it is.
    pub fn style_surface(mut self, edit: impl FnOnce(&mut SurfaceStyle)) -> Self {
        let (fill, surface) = match &mut self.render_type {
            DataRenderType::Field(field) => (field.fill.is_some(), &mut field.surface),
            DataRenderType::TriMesh(mesh) => (mesh.fill.is_some(), &mut mesh.surface),
            _ => return self,
        };
        if fill {
            edit(surface.get_or_insert_with(crate::defaults::surface_style));
        }
        self
    }

    // Vectors.

    /// How a vector's magnitude becomes an arrow length.
    pub fn vector_scale(mut self, scale: VectorScale) -> Self {
        if let DataRenderType::Vector { style, .. } = &mut self.render_type {
            style.scale = scale;
        }
        self
    }

    /// The shortest arrow drawn, in logical pixels; shorter ones are not
    /// drawn.
    pub fn min_length(mut self, px: f32) -> Self {
        if let DataRenderType::Vector { style, .. } = &mut self.render_type {
            style.min_length_px = px;
        }
        self
    }

    // Columns.

    /// Swap `x_column` and `y_column`, as a horizontal histogram's roles
    /// swap.
    pub fn swap_xy(mut self) -> Self {
        std::mem::swap(&mut self.x_column, &mut self.y_column);
        self
    }

    fn family_contour(&mut self) -> Option<&mut ContourConfig> {
        match &mut self.render_type {
            DataRenderType::Field(FieldSeries { contour, .. }) => contour.as_mut(),
            DataRenderType::TriMesh(mesh) => mesh.contour.as_mut(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;

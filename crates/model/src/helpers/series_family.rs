//! Chainable modifiers for the parts one chart family owns (#265
//! follow-up): ternary regions, range bar colours, whiskers, connectors
//! and labels, box plot violins and outliers, field and mesh contours and
//! surfaces, and vector scales. Like the other modifiers, each is a plain
//! method on the document type; a modifier that does not apply to a
//! series' family leaves the series as it is.
//!
//! The rarer settings of a range bar or box plot series have no modifier
//! of their own: [`SeriesConfig::style_range_bars`] and
//! [`SeriesConfig::style_box_plot`] hand a closure every field of the
//! family at once, as `&mut` [`RangeBarSeries`] and [`BoxSeries`].

use crate::color::Color;
use crate::data::ColumnId;
use crate::data_config::{
    BoxSeries, CategoryBarLabels, CategoryBarSeries, ContourConfig, DataRenderType, FieldSeries,
    RangeBarSeries, RangeColor, RangeConnector, RangeWhiskers, SeriesConfig, TernarySeries,
    VectorScale, VectorSeries, Violin,
};
use crate::ternary::TernaryRegionStyle;
use crate::view3d::SurfaceStyle;

impl SeriesConfig {
    // Markers and regions.

    /// Drop the markers of an x-y or ternary series.
    pub fn no_markers(mut self) -> Self {
        match &mut self.render_type {
            DataRenderType::Xy(xy) => xy.scatter = None,
            DataRenderType::Ternary(TernarySeries { scatter, .. }) => *scatter = None,
            _ => {}
        }
        self
    }

    /// Fill the polygons a ternary series outlines in `color` at `opacity`
    /// (`0..=1`, multiplying the colour's alpha).
    pub fn region(mut self, color: Color, opacity: f32) -> Self {
        if let DataRenderType::Ternary(TernarySeries { region, .. }) = &mut self.render_type {
            *region = Some(TernaryRegionStyle { color, opacity });
        }
        self
    }

    // Range bars.

    /// How each range bar is filled: the series colour, by direction, or
    /// by a host state column.
    pub fn range_colors(mut self, color_by: RangeColor) -> Self {
        if let DataRenderType::RangeBars(RangeBarSeries { color_by: c, .. }) = &mut self.render_type
        {
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
        if let DataRenderType::RangeBars(RangeBarSeries { whiskers, .. }) = &mut self.render_type {
            *whiskers = Some(RangeWhiskers {
                low: low.into(),
                high: high.into(),
                width,
                color: None,
            });
        }
        self
    }

    /// A line from each range bar's end to the next bar (a waterfall), in
    /// `color` and `width` logical pixels wide.
    pub fn connector(mut self, color: Color, width: f32) -> Self {
        if let DataRenderType::RangeBars(RangeBarSeries { connector, .. }) = &mut self.render_type {
            *connector = Some(RangeConnector { color, width });
        }
        self
    }

    /// Where the value labels of a category or range bar series go, and the
    /// host's drawn `text`, one string per bar (moved in, not copied).
    pub fn bar_labels(mut self, placement: CategoryBarLabels, text: Vec<String>) -> Self {
        if let DataRenderType::CategoryBars(CategoryBarSeries { labels, .. })
        | DataRenderType::RangeBars(RangeBarSeries { labels, .. }) = &mut self.render_type
        {
            labels.placement = placement;
            labels.text = text;
        }
        self
    }

    /// Edit any field of a range bar series: its bar style, progress (a
    /// Gantt chart), slot width on a numeric position axis, or any part
    /// the modifiers above set, as the series' [`RangeBarSeries`].
    pub fn style_range_bars(mut self, edit: impl FnOnce(&mut RangeBarSeries)) -> Self {
        if let DataRenderType::RangeBars(bars) = &mut self.render_type {
            edit(bars);
        }
        self
    }

    // Box plots.

    /// The violin drawn behind each box of a box plot.
    pub fn violin(mut self, violin: Violin) -> Self {
        if let DataRenderType::BoxPlot(BoxSeries { violin: v, .. }) = &mut self.render_type {
            *v = Some(violin);
        }
        self
    }

    /// The ragged column of a box plot's outliers, one group per category.
    pub fn outliers(mut self, column: impl Into<ColumnId>) -> Self {
        if let DataRenderType::BoxPlot(BoxSeries { outliers, .. }) = &mut self.render_type {
            *outliers = Some(column.into());
        }
        self
    }

    /// Edit any field of a box plot series: its box style, labels, raw
    /// points (a raincloud) or per-box overrides, as the series' [`BoxSeries`].
    pub fn style_box_plot(mut self, edit: impl FnOnce(&mut BoxSeries)) -> Self {
        if let DataRenderType::BoxPlot(boxes) = &mut self.render_type {
            edit(boxes);
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

    /// How a vector's magnitude becomes an arrow length. The rest of the
    /// glyph (its minimum length, colour map, line) is set through
    /// [`Self::style_vectors`].
    pub fn vector_scale(mut self, scale: VectorScale) -> Self {
        if let DataRenderType::Vector(VectorSeries { style, .. }) = &mut self.render_type {
            style.scale = scale;
        }
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

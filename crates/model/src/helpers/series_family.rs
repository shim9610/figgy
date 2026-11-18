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
//! family at once, as [`RangeBarsParts`] and [`BoxPlotParts`].

use crate::color::Color;
use crate::data::ColumnId;
use crate::data_config::{
    BoxPlotOverride, BoxPlotSeriesLabels, BoxPlotStyle, BoxPoints, CategoryBarLabels,
    CategoryBarSeriesLabels, CategoryBarStyle, ContourConfig, DataRenderType, FieldSeries,
    RangeColor, RangeConnector, RangeProgress, RangeWhiskers, SeriesConfig, VectorScale, Violin,
};
use crate::ternary::TernaryRegionStyle;
use crate::view3d::SurfaceStyle;

/// Every field of a range bar series ([`DataRenderType::RangeBars`]),
/// borrowed for editing by [`SeriesConfig::style_range_bars`]. Each field
/// is the variant's field of the same name; the read-only view is
/// [`crate::data_config::RangeBarsRef`].
#[derive(Debug)]
pub struct RangeBarsParts<'a> {
    /// The fill colour under [`RangeColor::Single`].
    pub color: &'a mut Color,
    /// The look of the bars.
    pub style: &'a mut CategoryBarStyle,
    /// The column of the start values.
    pub start: &'a mut ColumnId,
    /// The whiskers from a low to a high value.
    pub whiskers: &'a mut Option<RangeWhiskers>,
    /// How each bar's fill is chosen.
    pub color_by: &'a mut RangeColor,
    /// The finished share of each bar (a Gantt task's progress).
    pub progress: &'a mut Option<RangeProgress>,
    /// The line from each bar's end to the next bar.
    pub connector: &'a mut Option<RangeConnector>,
    /// The slot of one row along a numeric position axis, in data units.
    pub slot_width: &'a mut Option<f64>,
    /// Where the value labels go and their drawn text.
    pub labels: &'a mut CategoryBarSeriesLabels,
}

/// Every field of a box plot series ([`DataRenderType::BoxPlot`]),
/// borrowed for editing by [`SeriesConfig::style_box_plot`]. Each field is
/// the variant's field of the same name; the read-only view is
/// [`crate::data_config::BoxPlotRef`].
#[derive(Debug)]
pub struct BoxPlotParts<'a> {
    /// The fill colour of the boxes.
    pub color: &'a mut Color,
    /// The look of the boxes and their parts.
    pub style: &'a mut BoxPlotStyle,
    /// Which labels the boxes show and their drawn text.
    pub labels: &'a mut BoxPlotSeriesLabels,
    /// The ragged column of the outliers, one group per category.
    pub outliers: &'a mut Option<ColumnId>,
    /// Sparse per-box overrides.
    pub overrides: &'a mut Option<Vec<BoxPlotOverride>>,
    /// The violin drawn behind each box.
    pub violin: &'a mut Option<Violin>,
    /// The raw points drawn beside each box (a raincloud).
    pub points: &'a mut Option<BoxPoints>,
}

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

    /// A line from each range bar's end to the next bar (a waterfall), in
    /// `color` and `width` logical pixels wide.
    pub fn connector(mut self, color: Color, width: f32) -> Self {
        if let DataRenderType::RangeBars { connector, .. } = &mut self.render_type {
            *connector = Some(RangeConnector { color, width });
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

    /// Edit any field of a range bar series: its bar style, progress (a
    /// Gantt chart), slot width on a numeric position axis, or any part
    /// the modifiers above set, as [`RangeBarsParts`].
    pub fn style_range_bars(mut self, edit: impl FnOnce(RangeBarsParts<'_>)) -> Self {
        if let DataRenderType::RangeBars {
            color,
            style,
            start,
            whiskers,
            color_by,
            progress,
            connector,
            slot_width,
            labels,
        } = &mut self.render_type
        {
            edit(RangeBarsParts {
                color,
                style,
                start,
                whiskers,
                color_by,
                progress,
                connector,
                slot_width,
                labels,
            });
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

    /// The ragged column of a box plot's outliers, one group per category.
    pub fn outliers(mut self, column: impl Into<ColumnId>) -> Self {
        if let DataRenderType::BoxPlot { outliers, .. } = &mut self.render_type {
            *outliers = Some(column.into());
        }
        self
    }

    /// Edit any field of a box plot series: its box style, labels, raw
    /// points (a raincloud) or per-box overrides, as [`BoxPlotParts`].
    pub fn style_box_plot(mut self, edit: impl FnOnce(BoxPlotParts<'_>)) -> Self {
        if let DataRenderType::BoxPlot {
            color,
            style,
            labels,
            outliers,
            overrides,
            violin,
            points,
        } = &mut self.render_type
        {
            edit(BoxPlotParts {
                color,
                style,
                labels,
                outliers,
                overrides,
                violin,
                points,
            });
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
        if let DataRenderType::Vector { style, .. } = &mut self.render_type {
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

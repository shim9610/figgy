//! More chainable series modifiers and constructors (#265 follow-up).
//!
//! The fields the examples still set by hand after the first round: line
//! fills and interpolation, stems, error bars, histogram bars, point colour
//! and size maps, field fills and contour labels, vector glyphs, and
//! closures that reach a sub-style no modifier names. Like the first round,
//! each is a plain method on the document type; a modifier that does not
//! apply to a series' family leaves the series as it is.

use crate::color::Color;
use crate::data::ColumnId;
use crate::data_config::{
    BoxPlotStyle, CategoryBarDirection, CategoryBarStyle, ContourLabelConfig, DataLineStyleConfig,
    DataRenderType, DataScatterStyleConfig, FieldFillConfig, FieldSeries, LineFill,
    LineInterpolation, MatrixRef, PointColorMap, PointSizeMap, SeriesConfig, Shading, Stem,
    VectorComponents, VectorPivot, VectorStyle, XySeries,
};

impl SeriesConfig {
    /// Arrows on a regular grid: `u` and `v` are matrices over the `x` and
    /// `y` axis columns, one arrow per cell.
    pub fn vectors_grid(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        u: MatrixRef,
        v: MatrixRef,
    ) -> Self {
        Self::of_kind(
            id,
            x,
            y,
            DataRenderType::Vector {
                components: VectorComponents::Grid { u, v },
                style: VectorStyle::default(),
            },
        )
    }

    /// Arrows in a 3D box: one per sample at `(x, y, z)` with the
    /// components `u`, `v` and `w`.
    pub fn vectors_3d(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        z: impl Into<ColumnId>,
        u: impl Into<ColumnId>,
        v: impl Into<ColumnId>,
        w: impl Into<ColumnId>,
    ) -> Self {
        Self::of_kind(
            id,
            x,
            y,
            DataRenderType::Vector {
                components: VectorComponents::Points {
                    u: u.into(),
                    v: v.into(),
                    w: Some(w.into()),
                },
                style: VectorStyle::default(),
            },
        )
        .z(z)
    }

    // Lines.

    /// The colour of an x-y line's fill, set with [`Self::fill_to`] or
    /// [`Self::fill_between`]; without it the fill is the line colour at a
    /// quarter opacity.
    pub fn fill_color(mut self, color: Color) -> Self {
        if let Some(line) = self.xy_line() {
            line.fill_color = Some(color);
        }
        self
    }

    /// Fill between an x-y line and the series `other`, drawn by the same
    /// chart.
    pub fn fill_between(mut self, other: impl Into<String>) -> Self {
        if let Some(line) = self.xy_line() {
            line.fill = Some(LineFill::ToSeries(other.into()));
        }
        self
    }

    /// How an x-y line joins its points: straight or in steps.
    pub fn interpolation(mut self, interpolation: LineInterpolation) -> Self {
        if let Some(line) = self.xy_line() {
            line.interpolation = interpolation;
        }
        self
    }

    /// A line `width` logical pixels wide through the markers of an x-y or
    /// ternary series, in the markers' colour, when it has none.
    pub fn with_line(mut self, width: f32) -> Self {
        let (scatter, line) = match &mut self.render_type {
            DataRenderType::Xy(xy) => (xy.scatter.as_ref(), &mut xy.line),
            DataRenderType::Ternary { scatter, line, .. } => (scatter.as_ref(), line),
            _ => return self,
        };
        let color = scatter.map(|scatter| scatter.point_color);
        let line = line.get_or_insert_with(|| {
            let mut style = DataLineStyleConfig::default();
            if let Some(color) = color {
                style.line_color = color;
            }
            style
        });
        line.line_width = width;
        self
    }

    // Stems and error bars.

    /// A stem from each point of an x-y series to `baseline`, in the
    /// series' colour; an existing stem moves to `baseline`.
    pub fn with_stem(mut self, baseline: f64) -> Self {
        if let DataRenderType::Xy(xy) = &mut self.render_type {
            let color = xy_color(xy);
            let stem = xy.stem.get_or_insert_with(|| {
                let mut stem = Stem::default();
                if let Some(color) = color {
                    stem.color = color;
                }
                stem
            });
            stem.baseline = baseline;
        }
        self
    }

    /// The stem's own colour, apart from its markers.
    pub fn stem_color(mut self, color: Color) -> Self {
        if let DataRenderType::Xy(XySeries {
            stem: Some(stem), ..
        }) = &mut self.render_type
        {
            stem.color = color;
        }
        self
    }

    /// The error bars' stroke and cap widths (`width`) and cap length
    /// (`cap_size`), in logical pixels.
    pub fn error_bars(mut self, width: f32, cap_size: f32) -> Self {
        if let DataRenderType::Xy(XySeries {
            errors: Some(errors),
            ..
        }) = &mut self.render_type
        {
            errors.style.error_bar_width = width;
            errors.style.cap_width = width;
            errors.style.error_bar_cap_size = cap_size;
        }
        self
    }

    /// The error bars' own colour, apart from the markers.
    pub fn error_color(mut self, color: Color) -> Self {
        if let DataRenderType::Xy(XySeries {
            errors: Some(errors),
            ..
        }) = &mut self.render_type
        {
            errors.style.error_bar_color = color;
        }
        self
    }

    // Bars.

    /// A histogram bar's border.
    pub fn bar_border(mut self, color: Color, width: f32) -> Self {
        if let DataRenderType::Histogram { bar } = &mut self.render_type {
            bar.border_color = color;
            bar.border_width = width;
        }
        self
    }

    /// The gap between histogram bars, in logical pixels.
    pub fn bar_gap(mut self, px: f32) -> Self {
        if let DataRenderType::Histogram { bar } = &mut self.render_type {
            bar.gap_px = px;
        }
        self
    }

    // Point maps.

    /// Colour each marker of an x-y or ternary series by `column` through
    /// the chart's colour bar, or each polyline of a parallel series, each
    /// streamline or each arrow of a point vector series (a grid series
    /// takes a colour grid through [`Self::style_vectors`]).
    pub fn color_by(mut self, column: impl Into<ColumnId>) -> Self {
        let column = column.into();
        if let DataRenderType::Parallel { color_column, .. } = &mut self.render_type {
            *color_column = Some(column);
        } else if let DataRenderType::Streamlines { color_map, .. } = &mut self.render_type {
            *color_map = Some(PointColorMap { column });
        } else if let DataRenderType::Vector {
            components: VectorComponents::Points { .. },
            style,
        } = &mut self.render_type
        {
            style.color_map = Some(crate::data_config::VectorColorMap::Column(column));
        } else if let Some(scatter) = self.markers() {
            scatter.point_color_map = Some(PointColorMap { column });
        }
        self
    }

    /// Size each marker by `column`: values over `domain` map to radii
    /// over `px` (logical pixels) on the default size scale.
    pub fn size_by(mut self, column: impl Into<ColumnId>, domain: [f64; 2], px: [f32; 2]) -> Self {
        let column = column.into();
        if let Some(scatter) = self.markers() {
            scatter.point_size_map = Some(PointSizeMap {
                column,
                domain_min: domain[0],
                domain_max: domain[1],
                min_px: px[0],
                max_px: px[1],
                scale: crate::defaults::size_scale(),
            });
        }
        self
    }

    // Fields, meshes and images.

    /// How a field, mesh or RGB image fills its cells: flat or
    /// interpolated.
    pub fn shading(mut self, shading: Shading) -> Self {
        if let DataRenderType::RgbImage { shading: s, .. } = &mut self.render_type {
            *s = shading;
        } else if let Some(fill) = self.cell_fill() {
            fill.shading = shading;
        }
        self
    }

    /// The opacity of a field, mesh or RGB image fill, from 0 to 1.
    pub fn opacity(mut self, opacity: f32) -> Self {
        if let DataRenderType::RgbImage { opacity: o, .. } = &mut self.render_type {
            *o = opacity;
        } else if let Some(fill) = self.cell_fill() {
            fill.opacity = opacity;
        }
        self
    }

    /// A colour fill under a contour field or mesh, from the chart's
    /// colour bar.
    pub fn filled(mut self) -> Self {
        match &mut self.render_type {
            DataRenderType::Field(FieldSeries { fill, .. }) => {
                fill.get_or_insert_with(FieldFillConfig::default);
            }
            DataRenderType::TriMesh(mesh) => {
                mesh.fill.get_or_insert_with(FieldFillConfig::default);
            }
            _ => {}
        }
        self
    }

    /// Labels on the contour lines of a field or mesh.
    pub fn contour_labels(mut self, labels: ContourLabelConfig) -> Self {
        let contour = match &mut self.render_type {
            DataRenderType::Field(FieldSeries { contour, .. }) => contour.as_mut(),
            DataRenderType::TriMesh(mesh) => mesh.contour.as_mut(),
            _ => None,
        };
        if let Some(contour) = contour {
            contour.labels = Some(labels);
        }
        self
    }

    // Vectors.

    /// The arrowheads' length and width, in logical pixels: a vector
    /// series' heads, or a streamline series' arrows.
    pub fn arrow_head(mut self, length: f32, width: f32) -> Self {
        match &mut self.render_type {
            DataRenderType::Vector { style, .. } => {
                style.head_length_px = length;
                style.head_width_px = width;
            }
            DataRenderType::Streamlines {
                arrows: Some(arrows),
                ..
            } => {
                arrows.head_length_px = length;
                arrows.head_width_px = width;
            }
            _ => {}
        }
        self
    }

    /// The point of each arrow that sits on its sample.
    pub fn pivot(mut self, pivot: VectorPivot) -> Self {
        if let DataRenderType::Vector { style, .. } = &mut self.render_type {
            style.pivot = pivot;
        }
        self
    }

    // Closures on a sub-style.

    /// Edit every line style the series draws (its line, contour lines,
    /// streamlines or parallel polylines).
    pub fn style_line(mut self, edit: impl FnMut(&mut DataLineStyleConfig)) -> Self {
        self.for_each_line(edit);
        self
    }

    /// Edit the markers of an x-y or ternary series.
    pub fn style_markers(mut self, edit: impl FnOnce(&mut DataScatterStyleConfig)) -> Self {
        if let Some(scatter) = self.markers() {
            edit(scatter);
        }
        self
    }

    /// Edit the bar style of a category or range bar series.
    pub fn style_bars(mut self, edit: impl FnOnce(&mut CategoryBarStyle)) -> Self {
        if let DataRenderType::CategoryBars { style, .. }
        | DataRenderType::RangeBars { style, .. } = &mut self.render_type
        {
            edit(style);
        }
        self
    }

    /// Edit the box style of a box plot series.
    pub fn style_boxes(mut self, edit: impl FnOnce(&mut BoxPlotStyle)) -> Self {
        if let DataRenderType::BoxPlot { style, .. } = &mut self.render_type {
            edit(style);
        }
        self
    }

    /// Edit the glyph style of a vector series.
    pub fn style_vectors(mut self, edit: impl FnOnce(&mut VectorStyle)) -> Self {
        if let DataRenderType::Vector { style, .. } = &mut self.render_type {
            edit(style);
        }
        self
    }

    fn xy_line(&mut self) -> Option<&mut DataLineStyleConfig> {
        match &mut self.render_type {
            DataRenderType::Xy(XySeries {
                line: Some(line), ..
            }) => Some(line),
            _ => None,
        }
    }

    fn markers(&mut self) -> Option<&mut DataScatterStyleConfig> {
        match &mut self.render_type {
            DataRenderType::Xy(XySeries {
                scatter: Some(scatter),
                ..
            })
            | DataRenderType::Ternary {
                scatter: Some(scatter),
                ..
            } => Some(scatter),
            _ => None,
        }
    }

    fn cell_fill(&mut self) -> Option<&mut FieldFillConfig> {
        match &mut self.render_type {
            DataRenderType::Field(FieldSeries { fill, .. }) => fill.as_mut(),
            DataRenderType::TriMesh(mesh) => mesh.fill.as_mut(),
            _ => None,
        }
    }

    /// Stems drawn sideways, from the points to a vertical baseline.
    pub(crate) fn horizontal_stem(&mut self) {
        if let DataRenderType::Xy(XySeries {
            stem: Some(stem), ..
        }) = &mut self.render_type
        {
            stem.direction = CategoryBarDirection::Horizontal;
        }
    }
}

impl crate::radial::RadialChart {
    /// The chart with a plain title in its own font, label colour and one
    /// and a half times its label size. The radial chart takes no title
    /// margin or offset, so this title has neither.
    pub fn with_title(mut self, text: &str) -> Self {
        self.chart_title = Some(crate::config::ChartTitleOptions::from_legacy_plain(
            text,
            &self.font_family,
            self.font_size,
            self.label_color,
        ));
        self
    }
}

/// The colour an x-y series draws in: its line's, else its markers'.
fn xy_color(xy: &XySeries) -> Option<Color> {
    xy.line
        .as_ref()
        .map(|line| line.line_color)
        .or_else(|| xy.scatter.as_ref().map(|scatter| scatter.point_color))
}

#[cfg(test)]
mod tests;

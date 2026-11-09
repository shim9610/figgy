//! Series constructors and chainable modifiers, for hosts and examples (#265).
//!
//! One constructor per chart family returns a [`SeriesConfig`] that names
//! its columns and draws with the host style defaults
//! ([`crate::helpers`]): a valid series a document can hold as it stands.
//! The modifiers set the common fields of whichever family a series is.
//! They are plain methods on the document types, not a parallel builder:
//! the series stays the single source of truth, and a field no modifier
//! covers is set directly.
//!
//! Each constructor names the columns a family reads, in the roles its
//! [`DataRenderType`] variant documents; the ids must match the ones given
//! to the renderer's `add_column`. Nothing here reads or derives data.

use crate::color::Color;
use crate::data::ColumnId;
use crate::data_config::{
    DataBarStyleConfig, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
    DataScatterStyleConfig, ErrorBars, ErrorRef, FieldFillConfig, FieldSeries, GridLayout,
    LineFill, MatrixOrientation, MatrixRef, ScatterShape, SeriesConfig, Stem, VectorComponents,
    VectorStyle, XySeries,
};
use crate::line::LineStylePreset;
use crate::text::RichText;

impl MatrixRef {
    /// A matrix whose columns run along x: `columns[i]` is the column of
    /// values at the i-th x position, one value per y position.
    pub fn columns_are_x<I, C>(columns: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<ColumnId>,
    {
        Self {
            columns: columns.into_iter().map(Into::into).collect(),
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Centers,
        }
    }

    /// A matrix whose columns run along y: `columns[j]` is the column of
    /// values at the j-th y position, one value per x position.
    pub fn columns_are_y<I, C>(columns: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<ColumnId>,
    {
        Self {
            orientation: MatrixOrientation::ColumnsAreY,
            ..Self::columns_are_x(columns)
        }
    }

    /// The same matrix on another lattice: cell `Centers` (the constructors'
    /// choice), cell `Edges`, or a `Hexagonal` grid.
    pub fn with_layout(mut self, grid_layout: GridLayout) -> Self {
        self.grid_layout = grid_layout;
        self
    }
}

impl SeriesConfig {
    /// A series of `render_type` over the `x` and `y` columns, unlabelled
    /// and flat.
    pub fn of_kind(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        render_type: DataRenderType,
    ) -> Self {
        Self {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            z_column: None,
            render_type,
        }
    }

    fn xy(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        xy: XySeries,
    ) -> Self {
        Self::of_kind(id, x, y, DataRenderType::Xy(xy))
    }

    /// A line through `(x, y)`.
    pub fn line(id: impl Into<String>, x: impl Into<ColumnId>, y: impl Into<ColumnId>) -> Self {
        Self::xy(
            id,
            x,
            y,
            XySeries {
                scatter: None,
                line: Some(DataLineStyleConfig::default()),
                errors: None,
                stem: None,
            },
        )
    }

    /// A marker at each `(x, y)`.
    pub fn scatter(id: impl Into<String>, x: impl Into<ColumnId>, y: impl Into<ColumnId>) -> Self {
        Self::xy(
            id,
            x,
            y,
            XySeries {
                scatter: Some(DataScatterStyleConfig::default()),
                line: None,
                errors: None,
                stem: None,
            },
        )
    }

    /// Markers joined by a line.
    pub fn scatter_line(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
    ) -> Self {
        Self::xy(
            id,
            x,
            y,
            XySeries {
                scatter: Some(DataScatterStyleConfig::default()),
                line: Some(DataLineStyleConfig::default()),
                errors: None,
                stem: None,
            },
        )
    }

    /// Markers on stems down to the baseline 0 (a lollipop or stem plot).
    pub fn stem(id: impl Into<String>, x: impl Into<ColumnId>, y: impl Into<ColumnId>) -> Self {
        Self::xy(
            id,
            x,
            y,
            XySeries {
                scatter: Some(DataScatterStyleConfig::default()),
                line: None,
                errors: None,
                stem: Some(Stem::default()),
            },
        )
    }

    /// Vertical bars from pre-binned data: `edges` holds the bin edges and
    /// `counts` one value per bin. Binning is the host's job.
    pub fn histogram(
        id: impl Into<String>,
        edges: impl Into<ColumnId>,
        counts: impl Into<ColumnId>,
    ) -> Self {
        Self::of_kind(
            id,
            edges,
            counts,
            DataRenderType::Histogram {
                bar: DataBarStyleConfig::default(),
            },
        )
    }

    fn field(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        field: FieldSeries,
    ) -> Self {
        Self::of_kind(id, x, y, DataRenderType::Field(field))
    }

    /// A heatmap of `matrix` over the `x` and `y` coordinate columns,
    /// coloured through `Config.colorbar`.
    pub fn heatmap(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        matrix: MatrixRef,
    ) -> Self {
        Self::field(
            id,
            x,
            y,
            FieldSeries {
                matrix,
                fill: Some(FieldFillConfig::default()),
                contour: None,
                surface: None,
            },
        )
    }

    /// Contour lines of `matrix` at the host's `levels`.
    pub fn contours(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        matrix: MatrixRef,
        levels: Vec<f64>,
    ) -> Self {
        Self::field(
            id,
            x,
            y,
            FieldSeries {
                matrix,
                fill: None,
                contour: Some(crate::data_config::ContourConfig {
                    levels,
                    line: DataLineStyleConfig::default(),
                    per_level_color: None,
                    labels: None,
                }),
                surface: None,
            },
        )
    }

    /// A hexagonal heatmap of pre-binned `matrix` cells: the host bins,
    /// the renderer draws each hexagon its colour.
    pub fn hexbin(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        matrix: MatrixRef,
    ) -> Self {
        Self::heatmap(id, x, y, matrix.with_layout(GridLayout::Hexagonal))
    }

    /// An RGB image: three matrices on one lattice, each channel's values
    /// mapped from `[0, 1]`.
    pub fn rgb_image(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        red: MatrixRef,
        green: MatrixRef,
        blue: MatrixRef,
    ) -> Self {
        Self::of_kind(
            id,
            x,
            y,
            DataRenderType::RgbImage {
                red,
                green,
                blue,
                range: [0.0, 1.0],
                shading: crate::data_config::Shading::Flat,
                opacity: 1.0,
            },
        )
    }

    /// One arrow per sample at `(x, y)`, with the host's `u` and `v`
    /// components.
    pub fn vectors(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        u: impl Into<ColumnId>,
        v: impl Into<ColumnId>,
    ) -> Self {
        Self::of_kind(
            id,
            x,
            y,
            DataRenderType::Vector {
                components: VectorComponents::Points {
                    u: u.into(),
                    v: v.into(),
                    w: None,
                },
                style: VectorStyle::default(),
            },
        )
    }

    /// The host's streamlines: paths through `x` and `y`, separated by
    /// NaN rows. Integrating them is the host's job.
    pub fn streamlines(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
    ) -> Self {
        Self::of_kind(
            id,
            x,
            y,
            DataRenderType::Streamlines {
                line: DataLineStyleConfig::default(),
                arrows: Some(crate::defaults::streamline_arrows()),
                color_map: None,
            },
        )
    }

    /// Vertical category bars: `positions` holds `0, 1, ..., n - 1` and
    /// `values` one value per category.
    pub fn category_bars(
        id: impl Into<String>,
        positions: impl Into<ColumnId>,
        values: impl Into<ColumnId>,
        color: Color,
    ) -> Self {
        Self::of_kind(
            id,
            positions,
            values,
            DataRenderType::CategoryBars {
                color,
                style: crate::defaults::category_bar_style(),
                labels: crate::defaults::category_bar_series_labels(),
                label_format: crate::defaults::category_bar_label_format(),
                overrides: None,
            },
        )
    }

    /// Box plots: `positions` holds `0, 1, ..., n - 1` and `records` names
    /// the column of the host's five-number records, one per category.
    pub fn box_plot(
        id: impl Into<String>,
        positions: impl Into<ColumnId>,
        records: impl Into<ColumnId>,
        color: Color,
    ) -> Self {
        Self::of_kind(
            id,
            positions,
            records,
            DataRenderType::BoxPlot {
                color,
                style: crate::defaults::box_plot_style(),
                labels: crate::defaults::box_plot_series_labels(),
                outliers: None,
                overrides: None,
                violin: None,
                points: None,
            },
        )
    }

    /// Range bars (a Gantt or floating-bar chart): one bar per row at
    /// `positions`, from `start` to `end`, on a category axis. On a numeric
    /// axis the series also needs its `slot_width`.
    pub fn range_bars(
        id: impl Into<String>,
        positions: impl Into<ColumnId>,
        start: impl Into<ColumnId>,
        end: impl Into<ColumnId>,
        color: Color,
    ) -> Self {
        Self::of_kind(
            id,
            positions,
            end,
            DataRenderType::RangeBars {
                color,
                style: crate::defaults::category_bar_style(),
                start: start.into(),
                whiskers: None,
                color_by: crate::defaults::range_color(),
                progress: None,
                connector: None,
                slot_width: None,
                labels: crate::defaults::category_bar_series_labels(),
            },
        )
    }

    /// A radar polygon: `categories` holds `0, 1, ..., n - 1` and `values`
    /// one value per category on the radius axis.
    pub fn radar(
        id: impl Into<String>,
        categories: impl Into<ColumnId>,
        values: impl Into<ColumnId>,
        color: Color,
    ) -> Self {
        Self::of_kind(
            id,
            categories,
            values,
            DataRenderType::Radar {
                color,
                style: crate::defaults::radar_series_style(),
                labels: crate::defaults::radar_series_labels(),
            },
        )
    }

    /// Markers at ternary samples of the three components `a`, `b` and `c`
    /// (unnormalized; the renderer places them).
    pub fn ternary(
        id: impl Into<String>,
        a: impl Into<ColumnId>,
        b: impl Into<ColumnId>,
        c: impl Into<ColumnId>,
    ) -> Self {
        Self::of_kind(
            id,
            a,
            b,
            DataRenderType::Ternary {
                c_column: c.into(),
                scatter: Some(DataScatterStyleConfig::default()),
                line: None,
                region: None,
            },
        )
    }

    /// One polyline per row across the parallel axes, one column per axis
    /// in the axes' order. The first two also name the series' `x_column`
    /// and `y_column`, as the variant requires.
    pub fn parallel<I, C>(id: impl Into<String>, columns: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<ColumnId>,
    {
        let columns: Vec<ColumnId> = columns.into_iter().map(Into::into).collect();
        let axis = |index: usize| columns.get(index).cloned().unwrap_or_default();
        let (x, y) = (axis(0), axis(1));
        Self::of_kind(
            id,
            x,
            y,
            DataRenderType::Parallel {
                columns,
                line: DataLineStyleConfig::default(),
                color_column: None,
            },
        )
    }

    /// An energy-level (Grotrian) diagram held in the document.
    pub fn energy_diagram(
        id: impl Into<String>,
        levels: crate::energy_levels::EnergyLevelSeries,
    ) -> Self {
        Self::of_kind(id, "", "", DataRenderType::EnergyLevels(levels))
    }

    /// A triangle mesh over the vertices `(x, y)`, its triangles in the
    /// index column `triangles`, filled by `values`.
    pub fn mesh(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        triangles: impl Into<ColumnId>,
        values: crate::tri_mesh::MeshValues,
    ) -> Self {
        Self::of_kind(
            id,
            x,
            y,
            DataRenderType::TriMesh(crate::tri_mesh::TriMeshSeries {
                triangles: triangles.into(),
                values,
                fill: Some(FieldFillConfig::default()),
                contour: None,
                surface: None,
            }),
        )
    }

    /// A Feynman diagram held in the document.
    pub fn feynman_diagram(id: impl Into<String>, diagram: crate::feynman::FeynmanSeries) -> Self {
        Self::of_kind(id, "", "", DataRenderType::Feynman(diagram))
    }

    // Modifiers.

    /// The legend text, in the legend's own text look.
    pub fn label(mut self, text: &str) -> Self {
        let look = crate::defaults::legend().content;
        self.label = Some(RichText::plain(text, look.color, look.font_size, look.font));
        self
    }

    /// The height column, for a series on a 3D chart.
    pub fn z(mut self, column: impl Into<ColumnId>) -> Self {
        self.z_column = Some(column.into());
        self
    }

    /// The colour of every part with one: lines, markers, stems and error
    /// bars, bars, arrows, contour lines, and the colour of a category,
    /// box, range or radar series. A field fill, an RGB image, an energy
    /// diagram and a Feynman diagram take their colours from elsewhere and
    /// keep them.
    pub fn color(mut self, color: Color) -> Self {
        match &mut self.render_type {
            DataRenderType::Xy(xy) => {
                if let Some(line) = &mut xy.line {
                    line.line_color = color;
                }
                if let Some(scatter) = &mut xy.scatter {
                    scatter.point_color = color;
                }
                if let Some(stem) = &mut xy.stem {
                    stem.color = color;
                }
                if let Some(errors) = &mut xy.errors {
                    errors.style.error_bar_color = color;
                }
            }
            DataRenderType::Field(field) => {
                if let Some(contour) = &mut field.contour {
                    contour.line.line_color = color;
                }
            }
            DataRenderType::TriMesh(mesh) => {
                if let Some(contour) = &mut mesh.contour {
                    contour.line.line_color = color;
                }
            }
            DataRenderType::Histogram { bar } => bar.fill_color = color,
            DataRenderType::Vector { style, .. } => style.color = color,
            DataRenderType::CategoryBars { color: c, .. }
            | DataRenderType::BoxPlot { color: c, .. }
            | DataRenderType::RangeBars { color: c, .. }
            | DataRenderType::Radar { color: c, .. } => *c = color,
            DataRenderType::Ternary { scatter, line, .. } => {
                if let Some(scatter) = scatter {
                    scatter.point_color = color;
                }
                if let Some(line) = line {
                    line.line_color = color;
                }
            }
            DataRenderType::Streamlines { line, .. } | DataRenderType::Parallel { line, .. } => {
                line.line_color = color;
            }
            DataRenderType::RgbImage { .. }
            | DataRenderType::EnergyLevels(_)
            | DataRenderType::Feynman(_) => {}
        }
        self
    }

    /// The line width in logical pixels: lines, contour lines, stems,
    /// arrow shafts and a radar outline.
    pub fn width(mut self, width: f32) -> Self {
        self.for_each_line(|line| line.line_width = width);
        match &mut self.render_type {
            DataRenderType::Xy(xy) => {
                if let Some(stem) = &mut xy.stem {
                    stem.width = width;
                }
            }
            DataRenderType::Vector { style, .. } => style.width_px = width,
            DataRenderType::Radar { style, .. } => style.line_width_px = Some(width),
            _ => {}
        }
        self
    }

    /// The dash pattern of every line the series draws.
    pub fn dash(mut self, style: LineStylePreset) -> Self {
        self.for_each_line(|line| line.line_style = style);
        if let DataRenderType::Xy(xy) = &mut self.render_type
            && let Some(stem) = &mut xy.stem
        {
            stem.line_style = style;
        }
        self
    }

    /// Markers of `shape` and `size` (logical pixels) on an x-y or ternary
    /// series, added in the series' colour when it has none.
    pub fn marker(mut self, shape: ScatterShape, size: f32) -> Self {
        let mut color = None;
        self.for_each_line(|line| {
            color.get_or_insert(line.line_color);
        });
        let slot = match &mut self.render_type {
            DataRenderType::Xy(xy) => Some(&mut xy.scatter),
            DataRenderType::Ternary { scatter, .. } => Some(scatter),
            _ => None,
        };
        if let Some(slot) = slot {
            let scatter = slot.get_or_insert_with(|| {
                let mut style = DataScatterStyleConfig::default();
                if let Some(color) = color {
                    style.point_color = color;
                }
                style
            });
            scatter.point_shape = shape;
            scatter.point_size = size;
        }
        self
    }

    /// Fill between an x-y line and the horizontal line `y = baseline`.
    pub fn fill_to(mut self, baseline: f64) -> Self {
        if let DataRenderType::Xy(XySeries {
            line: Some(line), ..
        }) = &mut self.render_type
        {
            line.fill = Some(LineFill::ToBaseline(baseline));
        }
        self
    }

    /// Symmetric error bars along y, ±`column` at each point, on an x-y
    /// series with markers.
    pub fn y_errors(self, column: impl Into<ColumnId>) -> Self {
        self.errors(None, Some(column.into()))
    }

    /// Symmetric error bars along x, ±`column` at each point, on an x-y
    /// series with markers.
    pub fn x_errors(self, column: impl Into<ColumnId>) -> Self {
        self.errors(Some(column.into()), None)
    }

    fn errors(mut self, x: Option<ColumnId>, y: Option<ColumnId>) -> Self {
        if let DataRenderType::Xy(xy) = &mut self.render_type {
            let color = xy.scatter.as_ref().map(|scatter| scatter.point_color);
            let errors = xy.errors.get_or_insert_with(|| ErrorBars {
                x: None,
                y: None,
                style: {
                    let mut style = DataErrorBarStyleConfig::default();
                    if let Some(color) = color {
                        style.error_bar_color = color;
                    }
                    style
                },
            });
            if let Some(column) = x {
                errors.x = Some(ErrorRef::Symmetric { column });
            }
            if let Some(column) = y {
                errors.y = Some(ErrorRef::Symmetric { column });
            }
        }
        self
    }

    /// Horizontal histogram bars: `x_column` and `y_column` swap roles, as
    /// [`DataRenderType::Histogram`] documents. An x-y series' stems run
    /// sideways to a vertical baseline. (Category bars take their direction
    /// from the chart's `bar_groups`.)
    pub fn horizontal(mut self) -> Self {
        if let DataRenderType::Histogram { bar } = &mut self.render_type {
            bar.orientation = crate::data_config::BarOrientation::Horizontal;
        }
        self.horizontal_stem();
        self
    }

    /// Visit every line style the series draws: its line, contour lines,
    /// streamlines or parallel polylines.
    pub(crate) fn for_each_line(&mut self, mut visit: impl FnMut(&mut DataLineStyleConfig)) {
        match &mut self.render_type {
            DataRenderType::Xy(XySeries {
                line: Some(line), ..
            })
            | DataRenderType::Ternary {
                line: Some(line), ..
            }
            | DataRenderType::Streamlines { line, .. }
            | DataRenderType::Parallel { line, .. } => visit(line),
            DataRenderType::Field(FieldSeries {
                contour: Some(contour),
                ..
            }) => visit(&mut contour.line),
            DataRenderType::TriMesh(mesh) => {
                if let Some(contour) = &mut mesh.contour {
                    visit(&mut contour.line);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use crate::config::Category;

    fn grid(prefix: &str) -> MatrixRef {
        MatrixRef::columns_are_x((0..3).map(|i| format!("{prefix}{i}")))
    }

    fn valid_on(config: &Config, series: &[SeriesConfig]) {
        for item in series {
            if let Err(error) = config.validate_series(std::slice::from_ref(item)) {
                panic!("{:?}: {error:?}", item.render_type);
            }
        }
    }

    fn categorical() -> Config {
        let mut config = Config::default();
        config.bottom_x.categories = Some(
            (0..3)
                .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
                .collect(),
        );
        config
    }

    // Each constructor returns a series that validates on the chart its
    // family draws on (#265).
    #[test]
    fn each_constructor_validates_on_its_chart() {
        valid_on(
            &Config::default(),
            &[
                SeriesConfig::line("a", "x", "y"),
                SeriesConfig::scatter("a", "x", "y"),
                SeriesConfig::scatter_line("a", "x", "y"),
                SeriesConfig::stem("a", "x", "y"),
                SeriesConfig::histogram("a", "edges", "counts"),
                SeriesConfig::heatmap("a", "x", "y", grid("z")),
                SeriesConfig::contours("a", "x", "y", grid("z"), vec![0.5]),
                SeriesConfig::hexbin("a", "x", "y", grid("z")),
                SeriesConfig::rgb_image("a", "x", "y", grid("r"), grid("g"), grid("b")),
                SeriesConfig::vectors("a", "x", "y", "u", "v"),
                SeriesConfig::streamlines("a", "x", "y"),
            ],
        );
        let ink = Color::BLACK;
        valid_on(
            &categorical(),
            &[
                SeriesConfig::category_bars("a", "i", "v", ink),
                SeriesConfig::box_plot("a", "i", "records", ink),
                SeriesConfig::range_bars("a", "i", "start", "end", ink),
            ],
        );
        let mut polar = categorical();
        polar.set_polar(Some(crate::defaults::polar_options()));
        valid_on(&polar, &[SeriesConfig::radar("a", "i", "v", ink)]);
        let mut ternary = Config::default();
        ternary.set_ternary(Some(crate::defaults::ternary_options()));
        valid_on(&ternary, &[SeriesConfig::ternary("a", "p", "q", "r")]);
        let mut parallel = Config::default();
        let mut options = crate::defaults::parallel_options();
        options.axes = vec![
            crate::parallel::ParallelAxis::new(0.0, 1.0),
            crate::parallel::ParallelAxis::new(0.0, 1.0),
        ];
        parallel.set_parallel(Some(options));
        valid_on(&parallel, &[SeriesConfig::parallel("a", ["p", "q"])]);
    }

    // The modifiers write the fields they name and the series stays valid.
    #[test]
    fn modifiers_set_the_common_fields() {
        let red = Color::from_rgb8(200, 30, 30);
        let series = SeriesConfig::line("a", "x", "y")
            .label("sin x")
            .color(red)
            .width(2.5)
            .dash(LineStylePreset::Dash)
            .marker(ScatterShape::Diamond, 7.0)
            .fill_to(0.0)
            .y_errors("e");
        let DataRenderType::Xy(xy) = &series.render_type else {
            panic!("an x-y series");
        };
        let line = xy.line.as_ref().unwrap();
        assert_eq!(
            (line.line_color, line.line_width, line.line_style),
            (red, 2.5, LineStylePreset::Dash)
        );
        assert_eq!(line.fill, Some(LineFill::ToBaseline(0.0)));
        let scatter = xy.scatter.as_ref().unwrap();
        assert_eq!(
            (
                scatter.point_color,
                scatter.point_shape.clone(),
                scatter.point_size
            ),
            (red, ScatterShape::Diamond, 7.0)
        );
        let errors = xy.errors.as_ref().unwrap();
        assert_eq!(errors.style.error_bar_color, red);
        assert_eq!(errors.y, Some(ErrorRef::Symmetric { column: "e".into() }));
        let label = series.label.as_ref().unwrap();
        assert_eq!(label.font_size, crate::defaults::legend().content.font_size);
        valid_on(&Config::default(), std::slice::from_ref(&series));

        let bars = SeriesConfig::histogram("h", "e", "c")
            .color(red)
            .horizontal();
        let DataRenderType::Histogram { bar } = &bars.render_type else {
            panic!("a histogram");
        };
        assert_eq!(bar.fill_color, red);
        assert_eq!(
            bar.orientation,
            crate::data_config::BarOrientation::Horizontal
        );
        let surface = SeriesConfig::scatter("p", "x", "y").z("h");
        assert_eq!(surface.z_column, Some("h".into()));
    }
}

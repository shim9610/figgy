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
    BoxSeries, CategoryBarLabels, CategoryBarSeries, DataBarStyleConfig, DataErrorBarStyleConfig,
    DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, ErrorBars, ErrorRef,
    FieldFillConfig, FieldSeries, GridLayout, HistogramSeries, LineFill, MatrixOrientation,
    MatrixRef, ParallelSeries, RadarPolygonSeries, RangeBarSeries, RangeColor, RangeLinkSeries,
    RangeProgress, RgbImageSeries, ScatterShape, SeriesConfig, Stem, StreamlineSeries,
    TernarySeries, VectorComponents, VectorSeries, VectorStyle, XySeries,
};
use crate::line::LineStylePreset;
use crate::preset::ColorCycle;
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

/// The colour a constructor gives a series that takes one colour, the
/// ink a line or a marker starts in: the first colour of
/// [`ColorCycle::DEFAULT`] until [`SeriesConfig::color`], a theme's palette
/// or [`ColorCycle::color_unset`] sets it (#357). It was black before.
pub(super) const SERIES_INK: Color = ColorCycle::DEFAULT.color(0);

/// The corner radius of [`SeriesConfig::timeline_bars`], logical pixels:
/// the rounding of a web timeline's bars.
const TIMELINE_CORNER_RADIUS: f32 = 4.0;

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
        Self::of_kind(id, x, y, DataRenderType::Xy(xy)).color(SERIES_INK)
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
    /// `counts` one value per bin. Binning is the host's job. For
    /// horizontal bars, pass the counts first and the edges second and
    /// chain [`Self::horizontal`].
    pub fn histogram(
        id: impl Into<String>,
        edges: impl Into<ColumnId>,
        counts: impl Into<ColumnId>,
    ) -> Self {
        Self::of_kind(
            id,
            edges,
            counts,
            DataRenderType::Histogram(HistogramSeries {
                bar: DataBarStyleConfig::default(),
            }),
        )
        .color(SERIES_INK)
    }

    fn field(
        id: impl Into<String>,
        x: impl Into<ColumnId>,
        y: impl Into<ColumnId>,
        field: FieldSeries,
    ) -> Self {
        Self::of_kind(id, x, y, DataRenderType::Field(field)).color(SERIES_INK)
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
            DataRenderType::RgbImage(RgbImageSeries {
                red,
                green,
                blue,
                range: [0.0, 1.0],
                shading: crate::data_config::Shading::Flat,
                opacity: 1.0,
            }),
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
            DataRenderType::Vector(VectorSeries {
                components: VectorComponents::Points {
                    u: u.into(),
                    v: v.into(),
                    w: None,
                },
                style: VectorStyle::default(),
            }),
        )
        .color(SERIES_INK)
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
            DataRenderType::Streamlines(StreamlineSeries {
                line: DataLineStyleConfig::default(),
                arrows: Some(crate::defaults::streamline_arrows()),
                color_map: None,
            }),
        )
        .color(SERIES_INK)
    }

    /// Vertical category bars: `positions` holds `0, 1, ..., n - 1` and
    /// `values` one value per category. In the default cycle's first colour
    /// until [`Self::color`] or a theme's palette colours it, like every
    /// constructor here.
    pub fn category_bars(
        id: impl Into<String>,
        positions: impl Into<ColumnId>,
        values: impl Into<ColumnId>,
    ) -> Self {
        Self::of_kind(
            id,
            positions,
            values,
            DataRenderType::CategoryBars(CategoryBarSeries {
                color: SERIES_INK,
                style: crate::defaults::category_bar_style(),
                labels: crate::defaults::category_bar_series_labels(),
                label_format: crate::defaults::category_bar_label_format(),
                overrides: None,
            }),
        )
    }

    /// Box plots: `positions` holds `0, 1, ..., n - 1` and `records` names
    /// the column of the host's five-number records, one per category.
    /// In the default cycle's first colour until [`Self::color`] or a
    /// theme's palette colours it.
    pub fn box_plot(
        id: impl Into<String>,
        positions: impl Into<ColumnId>,
        records: impl Into<ColumnId>,
    ) -> Self {
        Self::of_kind(
            id,
            positions,
            records,
            DataRenderType::BoxPlot(BoxSeries {
                color: SERIES_INK,
                style: crate::defaults::box_plot_style(),
                labels: crate::defaults::box_plot_series_labels(),
                outliers: None,
                overrides: None,
                violin: None,
                points: None,
            }),
        )
    }

    /// Range bars (a Gantt or floating-bar chart): one bar per row at
    /// `positions`, from `start` to `end`, on a category axis. On a numeric
    /// axis the series also needs its `slot_width`, set through
    /// [`Self::style_range_bars`]. In the default cycle's first colour until
    /// [`Self::color`] or a theme's palette colours it.
    pub fn range_bars(
        id: impl Into<String>,
        positions: impl Into<ColumnId>,
        start: impl Into<ColumnId>,
        end: impl Into<ColumnId>,
    ) -> Self {
        Self::of_kind(
            id,
            positions,
            end,
            DataRenderType::RangeBars(RangeBarSeries {
                color: SERIES_INK,
                style: crate::defaults::category_bar_style(),
                start: start.into(),
                whiskers: None,
                color_by: crate::defaults::range_color(),
                progress: None,
                connector: None,
                slot_width: None,
                labels: crate::defaults::category_bar_series_labels(),
            }),
        )
    }

    /// Timeline (Gantt) bars: [`Self::range_bars`] in the timeline look of
    /// the common web timelines (#357). Each row takes its own colour from
    /// the muted [`crate::default::TIMELINE_COLORS`] ([`RangeColor::ByRow`]; the series
    /// colour, the legend swatch, is the first), and the bars are flat with
    /// 4 px corners, without texture or gloss. A label the host writes
    /// (`labels.text`) sits inside its bar when it fits and beside the bar's
    /// end otherwise ([`CategoryBarLabels::InsideOrBeside`]), in the dark or
    /// light [`crate::default::TIMELINE_LABEL_INK`], whichever contrasts more with what lies
    /// under it. Pair it with [`crate::default::default_timeline_config`]
    /// for the chart, and [`Self::with_progress`] for a progress column.
    /// Every choice is a field of the series, so a theme or an edit can
    /// change it; [`Self::range_bars`] keeps the documented look for
    /// waterfalls and candlesticks.
    pub fn timeline_bars(
        id: impl Into<String>,
        positions: impl Into<ColumnId>,
        start: impl Into<ColumnId>,
        end: impl Into<ColumnId>,
    ) -> Self {
        Self::range_bars(id, positions, start, end)
            .color(super::default::TIMELINE_COLORS[0])
            .style_range_bars(|bars| {
                bars.color_by = RangeColor::ByRow {
                    colors: super::default::TIMELINE_COLORS.to_vec(),
                };
                bars.style.corner_radius = TIMELINE_CORNER_RADIUS;
                bars.style.texture_strength = 0.0;
                bars.style.gloss = 0.0;
                bars.labels.placement = CategoryBarLabels::InsideOrBeside;
                bars.labels.contrast_ink = Some(super::default::TIMELINE_LABEL_INK);
            })
    }

    /// Dependency links (#357) between the bars of the range bar series
    /// `bars`, one per `(from, to)` row pair, in the web Gantt look: amber
    /// (`#D4A24C`), 1.5 px wide, leaving the predecessor 6 px from its start
    /// and ending in a 6 px open chevron ([`RangeLinkSeries`]). The series
    /// is DataLess: the links are its SSOT. List the links before their
    /// bars so the bars cover the link ends.
    pub fn range_links(
        id: impl Into<String>,
        bars: impl Into<String>,
        links: impl IntoIterator<Item = (u32, u32)>,
    ) -> Self {
        Self::of_kind(
            id,
            "",
            "",
            DataRenderType::RangeLinks(RangeLinkSeries {
                bars: bars.into(),
                color: Color::from_rgb8(212, 162, 76),
                width: 1.5,
                head: 6.0,
                inset: 6.0,
                links: links
                    .into_iter()
                    .map(|(from, to)| crate::data_config::RangeLink::new(from, to))
                    .collect(),
            }),
        )
    }

    /// A range bar series' progress (#357): each bar is drawn whole up to
    /// its share in `column` (`0..=1`, the host's) and in the light tint of
    /// [`crate::default::TIMELINE_REMAINING_OPACITY`] past it ([`RangeProgress`]). No effect
    /// on any other kind of series.
    pub fn with_progress(self, column: impl Into<ColumnId>) -> Self {
        let column = column.into();
        self.style_range_bars(|bars| {
            bars.progress = Some(RangeProgress {
                column,
                remaining_opacity: super::default::TIMELINE_REMAINING_OPACITY,
            });
        })
    }

    /// A radar polygon: `categories` holds `0, 1, ..., n - 1` and `values`
    /// one value per category on the radius axis. In the default cycle's
    /// first colour until [`Self::color`] or a theme's palette colours it.
    pub fn radar(
        id: impl Into<String>,
        categories: impl Into<ColumnId>,
        values: impl Into<ColumnId>,
    ) -> Self {
        Self::of_kind(
            id,
            categories,
            values,
            DataRenderType::Radar(RadarPolygonSeries {
                color: SERIES_INK,
                style: crate::defaults::radar_series_style(),
                labels: crate::defaults::radar_series_labels(),
            }),
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
            DataRenderType::Ternary(TernarySeries {
                c_column: c.into(),
                scatter: Some(DataScatterStyleConfig::default()),
                line: None,
                region: None,
            }),
        )
        .color(SERIES_INK)
    }

    /// One polyline per row across the parallel axes, one column per axis
    /// in the axes' order. The first two also name the series' `x_column`
    /// and `y_column`, as the variant requires. The rows draw 1 px wide in
    /// the series ink, opaque, so [`ColorCycle::color_unset`] still finds
    /// them unset; a theme's `ParallelTheme::line_opacity` or the host's
    /// alpha makes many rows read as a density (#357), and a colour column
    /// ([`Self::color_by`]) colours them through the chart's colourbar.
    ///
    /// [`ColorCycle::color_unset`]: crate::preset::ColorCycle::color_unset
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
            DataRenderType::Parallel(ParallelSeries {
                columns,
                line: DataLineStyleConfig::default(),
                color_column: None,
            }),
        )
        .color(SERIES_INK)
    }

    /// An energy-level (Grotrian) diagram held in the document.
    pub fn energy_levels(
        id: impl Into<String>,
        levels: crate::energy_levels::EnergyLevelSeries,
    ) -> Self {
        Self::of_kind(id, "", "", DataRenderType::EnergyLevels(levels))
    }

    /// A triangle mesh over the vertices `(x, y)`, its triangles in the
    /// index column `triangles`, filled by `values`.
    pub fn tri_mesh(
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

    /// Channel `channel` of the monotonic group `group` (#271), drawn as a
    /// line in the host default style. Its envelope columns are named after
    /// the series ([`SeriesConfig::monotonic_column_ids`]); the renderer
    /// writes them on every `set_monotonic` and `append_monotonic` call, so
    /// nothing is uploaded under them by the host.
    pub fn monotonic_line(id: impl Into<String>, group: impl Into<String>, channel: usize) -> Self {
        let id = id.into();
        let (x, y) = Self::monotonic_column_ids(&id);
        Self::of_kind(
            id,
            x,
            y,
            DataRenderType::MonotonicLine(crate::data_config::MonotonicLineSeries {
                group: group.into(),
                channel,
                line: DataLineStyleConfig::default(),
            }),
        )
        .color(SERIES_INK)
    }

    /// A Feynman diagram held in the document.
    pub fn feynman(id: impl Into<String>, diagram: crate::feynman::FeynmanSeries) -> Self {
        Self::of_kind(id, "", "", DataRenderType::Feynman(diagram))
    }

    // Modifiers.

    /// The legend text, in the legend's own text look. `$…$`, `\(…\)` and
    /// `$$…$$` spans are TeX and `\$` is a dollar sign
    /// ([`crate::text::rich_segments_from_label`]).
    pub fn label(mut self, text: &str) -> Self {
        let look = crate::defaults::legend().content;
        self.label = Some(RichText::from_label(
            text,
            look.color,
            look.font_size,
            look.font,
        ));
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
            DataRenderType::Histogram(HistogramSeries { bar }) => bar.fill_color = color,
            DataRenderType::Vector(VectorSeries { style, .. }) => style.color = color,
            DataRenderType::CategoryBars(CategoryBarSeries { color: c, .. })
            | DataRenderType::BoxPlot(BoxSeries { color: c, .. })
            | DataRenderType::RangeBars(RangeBarSeries { color: c, .. })
            | DataRenderType::RangeLinks(RangeLinkSeries { color: c, .. })
            | DataRenderType::Radar(RadarPolygonSeries { color: c, .. }) => *c = color,
            DataRenderType::Ternary(TernarySeries { scatter, line, .. }) => {
                if let Some(scatter) = scatter {
                    scatter.point_color = color;
                }
                if let Some(line) = line {
                    line.line_color = color;
                }
            }
            DataRenderType::Streamlines(StreamlineSeries { line, .. })
            | DataRenderType::Parallel(ParallelSeries { line, .. }) => {
                line.line_color = color;
            }
            DataRenderType::MonotonicLine(series) => series.line.line_color = color,
            DataRenderType::RgbImage(_)
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
            DataRenderType::Vector(VectorSeries { style, .. }) => style.width_px = width,
            DataRenderType::Radar(RadarPolygonSeries { style, .. }) => {
                style.line_width_px = Some(width)
            }
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
            DataRenderType::Ternary(TernarySeries { scatter, .. }) => Some(scatter),
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
        if let DataRenderType::Histogram(HistogramSeries { bar }) = &mut self.render_type {
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
            | DataRenderType::Ternary(TernarySeries {
                line: Some(line), ..
            })
            | DataRenderType::Streamlines(StreamlineSeries { line, .. })
            | DataRenderType::Parallel(ParallelSeries { line, .. }) => visit(line),
            DataRenderType::MonotonicLine(series) => visit(&mut series.line),
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

    /// The parallel rows draw 1 px in the opaque series ink, which the colour
    /// cycle recognises as unset (#357), and the parallel
    /// preset is a valid chart once the host writes its axes and names, and
    /// reads back unchanged.
    #[test]
    fn the_parallel_helpers_are_a_light_valid_chart() {
        let series = SeriesConfig::parallel("rows", ["a", "b", "c"]);
        let DataRenderType::Parallel(ParallelSeries { line, .. }) = &series.render_type else {
            panic!("a parallel series");
        };
        assert_eq!((line.line_width, line.line_color), (1.0, SERIES_INK));
        let mut config = crate::helpers::default::default_parallel_config();
        let options = config.parallel().unwrap();
        assert!(options.names_on_top);
        assert_eq!(options.label_backdrop.unwrap().a, 0.8);
        assert_eq!((config.left_y.min, config.left_y.max), (0.0, 1.0));
        assert_eq!(config.left_y.minor_count, 0);
        assert!(!config.top_x.line_visible && !config.right_y.line_visible);
        assert!(!config.grid.show_major_x && !config.grid.show_major_y);
        // A new chart's layout: its chart object fits it.
        assert_eq!(config.layout_state, crate::config::LayoutState::Fitted);
        config.bottom_x.categories = Some(
            ["a", "b", "c"]
                .map(|id| Category::new(id, id.to_uppercase()))
                .to_vec(),
        );
        config.parallel_mut().unwrap().axes =
            vec![crate::parallel::ParallelAxis::new(0.0, 10.0); 3];
        assert_eq!(config.validate_parallel(), Ok(()));
        valid_on(&config, &[series]);
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string(&config).unwrap();
            let back: Config = serde_json::from_str(&json).unwrap();
            assert_eq!(back, config);
        }
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
    fn constructors_start_in_the_default_cycle() {
        // #357: an uncoloured series is no longer black.
        let ink = ColorCycle::DEFAULT.color(0);
        assert_ne!(ink, Color::BLACK);
        let line = SeriesConfig::scatter_line("s", "x", "y").y_errors("e");
        let DataRenderType::Xy(xy) = &line.render_type else {
            panic!("an x-y series");
        };
        assert_eq!(xy.line.as_ref().unwrap().line_color, ink);
        assert_eq!(xy.scatter.as_ref().unwrap().point_color, ink);
        assert_eq!(xy.errors.as_ref().unwrap().style.error_bar_color, ink);
        let bars = SeriesConfig::category_bars("b", "p", "v");
        let DataRenderType::CategoryBars(bars) = &bars.render_type else {
            panic!("category bars");
        };
        assert_eq!(bars.color, ink);
        let hist = SeriesConfig::histogram("h", "e", "c");
        let DataRenderType::Histogram(hist) = &hist.render_type else {
            panic!("a histogram");
        };
        assert_eq!(hist.bar.fill_color, ink);
    }

    #[test]
    fn color_unset_cycles_in_series_order_and_keeps_set_colours() {
        let red = Color::from_rgb8(200, 0, 0);
        let mut series = [
            SeriesConfig::line("a", "x", "y"),
            SeriesConfig::scatter("b", "x", "y").color(red),
            SeriesConfig::line("c", "x", "y").color(Color::BLACK),
            SeriesConfig::stem("d", "x", "y"),
            SeriesConfig::box_plot("e", "p", "r"),
            SeriesConfig::histogram("f", "e", "c"),
            SeriesConfig::line("g", "x", "y"),
        ];
        ColorCycle::DEFAULT.color_unset(&mut series);
        let first = |s: &SeriesConfig| match &s.render_type {
            DataRenderType::Xy(xy) => xy
                .line
                .as_ref()
                .map(|l| l.line_color)
                .or(xy.scatter.as_ref().map(|m| m.point_color))
                .unwrap(),
            DataRenderType::BoxPlot(b) => b.color,
            DataRenderType::Histogram(h) => h.bar.fill_color,
            _ => unreachable!(),
        };
        let colors: Vec<Color> = series.iter().map(first).collect();
        let d = ColorCycle::DEFAULT;
        assert_eq!(
            colors,
            [
                d.color(0),
                red,
                Color::BLACK,
                d.color(3),
                d.color(4),
                d.color(5),
                d.color(6),
            ]
        );
        // A stem's marker and stem take the same colour.
        let DataRenderType::Xy(stem) = &series[3].render_type else {
            unreachable!()
        };
        assert_eq!(stem.stem.as_ref().unwrap().color, d.color(3));
        // Another preset cycles the same way, and a second call changes
        // nothing it already coloured.
        let mut again = series.clone();
        ColorCycle::DEFAULT.color_unset(&mut again);
        assert_eq!(again, series);
        let mut set2 = [
            SeriesConfig::line("a", "x", "y"),
            SeriesConfig::line("b", "x", "y"),
        ];
        ColorCycle::Set2.color_unset(&mut set2);
        assert_eq!(first(&set2[1]), ColorCycle::Set2.color(1));
    }

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
                SeriesConfig::category_bars("a", "i", "v").color(ink),
                SeriesConfig::box_plot("a", "i", "records").color(ink),
                SeriesConfig::range_bars("a", "i", "start", "end").color(ink),
            ],
        );
        let mut polar = categorical();
        polar.set_polar(Some(crate::defaults::polar_options()));
        valid_on(&polar, &[SeriesConfig::radar("a", "i", "v").color(ink)]);
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
        let DataRenderType::Histogram(HistogramSeries { bar }) = &bars.render_type else {
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

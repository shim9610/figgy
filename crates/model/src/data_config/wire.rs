//! The wire form of [`DataRenderType`]: the tagged JSON shape documents use, and its conversions.

use super::*;

impl From<WireRenderType> for DataRenderType {
    fn from(wire: WireRenderType) -> Self {
        let xy = |scatter, line, errors, stem| {
            DataRenderType::Xy(XySeries {
                scatter,
                line,
                errors,
                stem,
            })
        };
        let errors = |x, y, style| Some(ErrorBars { x, y, style });
        match wire {
            WireRenderType::Scatter { scatter, stem } => xy(Some(scatter), None, None, stem),
            WireRenderType::Line { line } => xy(None, Some(line), None, None),
            WireRenderType::ScatterLine {
                scatter,
                line,
                stem,
            } => xy(Some(scatter), Some(line), None, stem),
            WireRenderType::ScatterErrorbarX {
                scatter,
                err_x,
                err_style,
                stem,
            } => xy(
                Some(scatter),
                None,
                errors(Some(err_x), None, err_style),
                stem,
            ),
            WireRenderType::ScatterErrorbarY {
                scatter,
                err_y,
                err_style,
                stem,
            } => xy(
                Some(scatter),
                None,
                errors(None, Some(err_y), err_style),
                stem,
            ),
            WireRenderType::ScatterErrorbarXY {
                scatter,
                err_x,
                err_y,
                err_style,
                stem,
            } => xy(
                Some(scatter),
                None,
                errors(Some(err_x), Some(err_y), err_style),
                stem,
            ),
            WireRenderType::LineScatterErrorbarX {
                scatter,
                line,
                err_x,
                err_style,
                stem,
            } => xy(
                Some(scatter),
                Some(line),
                errors(Some(err_x), None, err_style),
                stem,
            ),
            WireRenderType::LineScatterErrorbarY {
                scatter,
                line,
                err_y,
                err_style,
                stem,
            } => xy(
                Some(scatter),
                Some(line),
                errors(None, Some(err_y), err_style),
                stem,
            ),
            WireRenderType::LineScatterErrorbarXY {
                scatter,
                line,
                err_x,
                err_y,
                err_style,
                stem,
            } => xy(
                Some(scatter),
                Some(line),
                errors(Some(err_x), Some(err_y), err_style),
                stem,
            ),
            WireRenderType::Heatmap {
                matrix,
                fill,
                surface,
            } => DataRenderType::Field(FieldSeries {
                matrix,
                fill: Some(fill),
                contour: None,
                surface,
            }),
            WireRenderType::Contour { matrix, contour } => DataRenderType::Field(FieldSeries {
                matrix,
                fill: None,
                contour: Some(contour),
                surface: None,
            }),
            WireRenderType::HeatmapContour {
                matrix,
                fill,
                contour,
                surface,
            } => DataRenderType::Field(FieldSeries {
                matrix,
                fill: Some(fill),
                contour: Some(contour),
                surface,
            }),
            WireRenderType::Histogram { bar } => DataRenderType::Histogram { bar },
            WireRenderType::RgbImage {
                red,
                green,
                blue,
                range,
                shading,
                opacity,
            } => DataRenderType::RgbImage {
                red,
                green,
                blue,
                range,
                shading,
                opacity,
            },
            WireRenderType::Vector { components, style } => {
                DataRenderType::Vector { components, style }
            }
            WireRenderType::CategoryBars {
                color,
                style,
                labels,
                label_format,
                overrides,
            } => DataRenderType::CategoryBars {
                color,
                style,
                labels,
                label_format,
                overrides,
            },
            WireRenderType::BoxPlot {
                color,
                style,
                labels,
                outliers,
                overrides,
                violin,
                points,
            } => DataRenderType::BoxPlot {
                color,
                style,
                labels,
                outliers,
                overrides,
                violin,
                points,
            },
            WireRenderType::RangeBars {
                color,
                style,
                start,
                whiskers,
                color_by,
                progress,
                connector,
                slot_width,
                labels,
            } => DataRenderType::RangeBars {
                color,
                style,
                start,
                whiskers,
                color_by,
                progress,
                connector,
                slot_width,
                labels,
            },
            WireRenderType::Radar {
                color,
                style,
                labels,
            } => DataRenderType::Radar {
                color,
                style,
                labels,
            },
            WireRenderType::Ternary {
                c_column,
                scatter,
                line,
                region,
            } => DataRenderType::Ternary {
                c_column,
                scatter,
                line,
                region,
            },
            WireRenderType::Streamlines {
                line,
                arrows,
                color_map,
            } => DataRenderType::Streamlines {
                line,
                arrows,
                color_map,
            },
            WireRenderType::Parallel {
                columns,
                line,
                color_column,
            } => DataRenderType::Parallel {
                columns,
                line,
                color_column,
            },
            WireRenderType::EnergyLevels(levels) => DataRenderType::EnergyLevels(levels),
            WireRenderType::TriMesh(mesh) => DataRenderType::TriMesh(mesh),
            WireRenderType::Feynman(diagram) => DataRenderType::Feynman(diagram),
        }
    }
}

impl TryFrom<DataRenderType> for WireRenderType {
    type Error = crate::RuleKind;

    /// The written form of a render type; a combination
    /// [`XySeries::validate`] or [`FieldSeries::validate`] refuses has none.
    fn try_from(render: DataRenderType) -> Result<Self, crate::RuleKind> {
        Ok(match render {
            DataRenderType::Xy(xy) => {
                xy.validate()?;
                let stem = xy.stem;
                match (xy.scatter, xy.line, xy.errors) {
                    (Some(scatter), None, None) => WireRenderType::Scatter { scatter, stem },
                    (None, Some(line), None) => WireRenderType::Line { line },
                    (Some(scatter), Some(line), None) => WireRenderType::ScatterLine {
                        scatter,
                        line,
                        stem,
                    },
                    (
                        Some(scatter),
                        line,
                        Some(ErrorBars {
                            x,
                            y,
                            style: err_style,
                        }),
                    ) => match (line, x, y) {
                        (None, Some(err_x), None) => WireRenderType::ScatterErrorbarX {
                            scatter,
                            err_x,
                            err_style,
                            stem,
                        },
                        (None, None, Some(err_y)) => WireRenderType::ScatterErrorbarY {
                            scatter,
                            err_y,
                            err_style,
                            stem,
                        },
                        (None, Some(err_x), Some(err_y)) => WireRenderType::ScatterErrorbarXY {
                            scatter,
                            err_x,
                            err_y,
                            err_style,
                            stem,
                        },
                        (Some(line), Some(err_x), None) => WireRenderType::LineScatterErrorbarX {
                            scatter,
                            line,
                            err_x,
                            err_style,
                            stem,
                        },
                        (Some(line), None, Some(err_y)) => WireRenderType::LineScatterErrorbarY {
                            scatter,
                            line,
                            err_y,
                            err_style,
                            stem,
                        },
                        (Some(line), Some(err_x), Some(err_y)) => {
                            WireRenderType::LineScatterErrorbarXY {
                                scatter,
                                line,
                                err_x,
                                err_y,
                                err_style,
                                stem,
                            }
                        }
                        (_, None, None) => return Err(crate::RuleKind::ErrorBarsDirection),
                    },
                    (None, _, _) => return Err(crate::RuleKind::XyWireNothingDrawn),
                }
            }
            DataRenderType::Field(field) => {
                field.validate()?;
                let FieldSeries {
                    matrix,
                    fill,
                    contour,
                    surface,
                } = field;
                match (fill, contour) {
                    (Some(fill), None) => WireRenderType::Heatmap {
                        matrix,
                        fill,
                        surface,
                    },
                    (None, Some(contour)) => WireRenderType::Contour { matrix, contour },
                    (Some(fill), Some(contour)) => WireRenderType::HeatmapContour {
                        matrix,
                        fill,
                        contour,
                        surface,
                    },
                    (None, None) => return Err(crate::RuleKind::FieldNothingDrawn),
                }
            }
            DataRenderType::Histogram { bar } => WireRenderType::Histogram { bar },
            DataRenderType::RgbImage {
                red,
                green,
                blue,
                range,
                shading,
                opacity,
            } => WireRenderType::RgbImage {
                red,
                green,
                blue,
                range,
                shading,
                opacity,
            },
            DataRenderType::Vector { components, style } => {
                WireRenderType::Vector { components, style }
            }
            DataRenderType::CategoryBars {
                color,
                style,
                labels,
                label_format,
                overrides,
            } => WireRenderType::CategoryBars {
                color,
                style,
                labels,
                label_format,
                overrides,
            },
            DataRenderType::BoxPlot {
                color,
                style,
                labels,
                outliers,
                overrides,
                violin,
                points,
            } => WireRenderType::BoxPlot {
                color,
                style,
                labels,
                outliers,
                overrides,
                violin,
                points,
            },
            DataRenderType::RangeBars {
                color,
                style,
                start,
                whiskers,
                color_by,
                progress,
                connector,
                slot_width,
                labels,
            } => WireRenderType::RangeBars {
                color,
                style,
                start,
                whiskers,
                color_by,
                progress,
                connector,
                slot_width,
                labels,
            },
            DataRenderType::Radar {
                color,
                style,
                labels,
            } => WireRenderType::Radar {
                color,
                style,
                labels,
            },
            DataRenderType::Ternary {
                c_column,
                scatter,
                line,
                region,
            } => WireRenderType::Ternary {
                c_column,
                scatter,
                line,
                region,
            },
            DataRenderType::Streamlines {
                line,
                arrows,
                color_map,
            } => WireRenderType::Streamlines {
                line,
                arrows,
                color_map,
            },
            DataRenderType::Parallel {
                columns,
                line,
                color_column,
            } => WireRenderType::Parallel {
                columns,
                line,
                color_column,
            },
            DataRenderType::EnergyLevels(levels) => WireRenderType::EnergyLevels(levels),
            DataRenderType::TriMesh(mesh) => WireRenderType::TriMesh(mesh),
            DataRenderType::Feynman(diagram) => WireRenderType::Feynman(diagram),
        })
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for DataRenderType {
    /// Written in the one-variant-per-combination form documents have
    /// always used; a combination with no written form fails.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        WireRenderType::try_from(self.clone())
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

/// The serialized form of [`DataRenderType`]: one variant per combination,
/// as documents have always been written (#160). Never used in memory.
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(not(feature = "serde"), allow(dead_code))]
pub(crate) enum WireRenderType {
    Scatter {
        scatter: DataScatterStyleConfig,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        stem: Option<Stem>,
    },
    Line {
        line: DataLineStyleConfig,
    },
    ScatterLine {
        scatter: DataScatterStyleConfig,
        line: DataLineStyleConfig,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        stem: Option<Stem>,
    },
    ScatterErrorbarX {
        scatter: DataScatterStyleConfig,
        err_x: ErrorRef,
        err_style: DataErrorBarStyleConfig,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        stem: Option<Stem>,
    },
    ScatterErrorbarY {
        scatter: DataScatterStyleConfig,
        err_y: ErrorRef,
        err_style: DataErrorBarStyleConfig,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        stem: Option<Stem>,
    },
    ScatterErrorbarXY {
        scatter: DataScatterStyleConfig,
        err_x: ErrorRef,
        err_y: ErrorRef,
        err_style: DataErrorBarStyleConfig,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        stem: Option<Stem>,
    },
    LineScatterErrorbarX {
        scatter: DataScatterStyleConfig,
        line: DataLineStyleConfig,
        err_x: ErrorRef,
        err_style: DataErrorBarStyleConfig,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        stem: Option<Stem>,
    },
    LineScatterErrorbarY {
        scatter: DataScatterStyleConfig,
        line: DataLineStyleConfig,
        err_y: ErrorRef,
        err_style: DataErrorBarStyleConfig,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        stem: Option<Stem>,
    },
    LineScatterErrorbarXY {
        scatter: DataScatterStyleConfig,
        line: DataLineStyleConfig,
        err_x: ErrorRef,
        err_y: ErrorRef,
        err_style: DataErrorBarStyleConfig,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        stem: Option<Stem>,
    },
    /// Bars from pre-binned data. Binning is the host's job: the renderer is
    /// handed `(edges, counts)` and draws them.
    ///
    /// `bar.orientation` alone decides which column is which — `Vertical` reads
    /// `x_column` as edges and `y_column` as counts, `Horizontal` the reverse.
    /// The lengths are not consulted to guess the roles.
    Histogram {
        bar: DataBarStyleConfig,
    },
    /// A filled field and nothing else. `x_column` / `y_column` are the grid
    /// coordinates; `matrix` names the columns that make up z.
    Heatmap {
        matrix: MatrixRef,
        fill: FieldFillConfig,
        /// The look of the heatmap drawn as a surface on a 3D chart
        /// (`Config.view_3d`); kept and unused on a flat chart. `None` (the
        /// default, omitted from JSON) draws the surface with
        /// [`SurfaceStyle::default`].
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        surface: Option<SurfaceStyle>,
    },
    /// Three matrices drawn as a colour image (#139): `red`, `green` and
    /// `blue` on one lattice (the series' coordinate columns; the grids
    /// share their column count, orientation and layout), each channel's
    /// value mapped linearly from `range` to 0..1 and clamped. No colourbar
    /// is read: the colours are the data. `Flat` paints each cell its
    /// colour, `Interpolated` blends each channel between sample points; a
    /// cell with a non-finite value in any channel is unpainted. Drawn flat
    /// only; see [`Config::validate_rgb_image_series`](crate::config::Config::validate_rgb_image_series).
    RgbImage {
        red: MatrixRef,
        green: MatrixRef,
        blue: MatrixRef,
        /// The channel values that map to 0 and 1 (`[0, 1]` by default;
        /// `[0, 255]` for bytes).
        #[cfg_attr(
            feature = "serde",
            serde(
                default = "crate::defaults::image_range",
                skip_serializing_if = "is_default_image_range"
            )
        )]
        range: [f64; 2],
        #[cfg_attr(
            feature = "serde",
            serde(
                default = "crate::defaults::image_shading",
                skip_serializing_if = "is_default_image_shading"
            )
        )]
        shading: Shading,
        #[cfg_attr(
            feature = "serde",
            serde(
                default = "crate::defaults::image_opacity",
                skip_serializing_if = "is_default_image_opacity"
            )
        )]
        opacity: f32,
    },
    /// Contour lines only — the field is not filled.
    Contour {
        matrix: MatrixRef,
        contour: ContourConfig,
    },
    /// Filled field with contour lines drawn over it.
    HeatmapContour {
        matrix: MatrixRef,
        fill: FieldFillConfig,
        contour: ContourConfig,
        /// The surface's look on a 3D chart, as [`DataRenderType::Heatmap`]'s;
        /// the contours draw on the box floor.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        surface: Option<SurfaceStyle>,
    },
    /// A vector field (quiver plot): one arrow per sample, with the direction
    /// and length given by the host's `u` / `v` components.
    ///
    /// The renderer draws the vectors it is given. It does not derive them
    /// from a scalar field and it does not reduce them to a scalar: colouring
    /// by magnitude takes a host-supplied value column or grid through
    /// [`VectorStyle::color_map`].
    ///
    /// The columns must be GPU resident; a streamed source is rejected with
    /// an error. Polar charts (`Config.polar`) reject vector series.
    Vector {
        components: VectorComponents,
        style: VectorStyle,
    },
    /// The bars of a categorical bar chart: one bar per category of the
    /// chart's categorical axis ([`AxisOptions::categories`]), in this
    /// series' colour and look.
    ///
    /// The bar direction ([`BarGroups::direction`] of `Config::bar_groups`)
    /// decides the column roles, like a histogram's orientation:
    /// `Vertical` reads `x_column` as the category positions (the host
    /// uploads `0, 1, ..., n - 1`) and `y_column` as the values, one per
    /// category with NaN for a missing bar; `Horizontal` reads `y_column`
    /// as the positions and `x_column` as the values. Both columns hold
    /// exactly one value per category.
    ///
    /// The `CategoryBars` series of one layer form its bar group: they
    /// stand side by side, stack or stack to 100 % as
    /// [`BarGroups::mode`] says, in series order. The renderer reads the
    /// other series' values from the pool for a stack; nothing per bar is
    /// kept on the CPU. A `PercentStacked` group whose value column has a
    /// negative value (the pool's minimum statistic below 0) is refused when
    /// it is drawn or picked, with `InvalidSeriesConfig` naming that series
    /// and the mode; `Stacked` and `Grouped` draw negative values.
    ///
    /// Value labels: `labels.text` holds the drawn strings ([`format_labels`]
    /// writes them; the renderer never formats a value), one per category,
    /// drawn at each bar's end as `labels.placement` (and a bar override's
    /// `labels`) says, moved by `labels.offsets`. An empty string draws
    /// nothing. Labels never displace each other: thinning the category
    /// axis is the only way to thin them.
    ///
    /// The columns must be GPU resident; a streamed source is rejected with
    /// an error. On a polar chart (`Config.polar`) with a categorical angle
    /// axis the bars fan around the centre as wedges, a rose (#133): the
    /// group is vertical, the value is the radius.
    CategoryBars {
        /// The fill colour of the series' bars, opaque.
        color: Color,
        /// The look of the bars: material, corner radius, outline, texture,
        /// gloss and the brightness of a picked bar.
        #[cfg_attr(
            feature = "serde",
            serde(default = "crate::defaults::category_bar_style")
        )]
        style: CategoryBarStyle,
        /// Where the value labels go, how values are written and the drawn
        /// text of each bar.
        #[cfg_attr(
            feature = "serde",
            serde(default = "crate::defaults::category_bar_series_labels")
        )]
        labels: CategoryBarSeriesLabels,
        /// What a value label says.
        #[cfg_attr(
            feature = "serde",
            serde(default = "crate::defaults::category_bar_label_format")
        )]
        label_format: CategoryBarLabelFormat,
        /// Sparse per-bar overrides, keyed by category id and this series'
        /// id. At most one per bar; an override's `style` replaces the
        /// series' style whole.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        overrides: Option<Vec<CategoryBarOverride>>,
    },
    /// The boxes of a box plot: one box per category of the chart's
    /// categorical axis ([`AxisOptions::categories`]), in this series'
    /// colour and look, with its median, whiskers and caps, its mean and its
    /// outliers.
    ///
    /// `x_column` is the category index column (the host uploads
    /// `0, 1, ..., n - 1`) and `y_column` the id of the series' **record**
    /// of box statistics (the renderer's `BOX_PLOT_RECORD` schema: `median`,
    /// `q1`, `q3`, `whisker_low`, `whisker_high`, and optionally `mean`,
    /// `sample_count`, `ci_low`, `ci_high`), one row per category; a missing
    /// box is a row of NaN. The roles do not swap with the direction as a
    /// category bar's do: a record is not a value column of either axis, so
    /// the series names it in its value column slot and keeps its shape.
    /// `outliers` names a ragged column holding one group of outliers per
    /// category, when the series has any.
    ///
    /// The `BoxPlot` series of one layer form its box group: they stand side
    /// by side in series order, laid out by `Config::bar_groups`'
    /// direction, group width and gap ([`BarGroups`]); its mode must be
    /// grouped. Statistics the record does not order (whisker ≤ q1 ≤
    /// median ≤ q3 ≤ whisker, outliers outside the whiskers) are not
    /// checked: the drawing clamps. Nothing per box is kept on the CPU.
    ///
    /// Value labels: `labels.text` holds the drawn strings, which the host
    /// writes (the renderer has no per-box statistics to write them from;
    /// [`format_box_plot_labels`] helps), one per category, drawn past the
    /// top of each box (its high whisker, mean, notch and outliers) unless
    /// `labels.placement` (or a box override's `labels`) is `None`, moved by
    /// `labels.offsets`. An empty string draws nothing.
    ///
    /// The record and the outliers must be GPU resident; a streamed source
    /// is rejected with an error. Polar charts (`Config.polar`) reject box
    /// plots.
    BoxPlot {
        /// The fill colour of the series' boxes, opaque.
        color: Color,
        /// The look of the boxes and their parts.
        #[cfg_attr(feature = "serde", serde(default = "crate::defaults::box_plot_style"))]
        style: BoxPlotStyle,
        /// Which labels the boxes show, how a median is written and the
        /// drawn text of each box.
        #[cfg_attr(
            feature = "serde",
            serde(default = "crate::defaults::box_plot_series_labels")
        )]
        labels: BoxPlotSeriesLabels,
        /// The ragged column of the outliers, one group per category.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        outliers: Option<ColumnId>,
        /// Sparse per-box overrides, keyed by category id and this series'
        /// id. At most one per box; an override's `style` replaces the
        /// series' style whole.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        overrides: Option<Vec<BoxPlotOverride>>,
        /// The violin drawn behind each box: a host density curve per
        /// category, mirrored or one per side ([`Violin`]). With a violin,
        /// an empty `y_column` draws no box.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        violin: Option<Violin>,
        /// The raw points drawn beside each box, jittered across the slot
        /// ([`BoxPoints`]): a raincloud with a one-sided violin.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        points: Option<BoxPoints>,
    },
    /// Range bars: one bar per row, floating from a start value to an end
    /// value at a position, with optional whiskers from a low to a high
    /// value. One kind draws waterfall charts (the running total before and
    /// after each step), candlesticks (open to close, low to high), state
    /// timelines and Gantt charts (an interval per row), and floating
    /// min–max bars.
    ///
    /// `x_column` holds the positions and `y_column` the end values,
    /// whatever the direction (the box plot convention); `start` holds the
    /// start values. Every role is a column of one value per row, supplied
    /// by the host: the renderer draws the values it is given and computes
    /// none (a running total, a duration or a difference is host data). A
    /// row with a NaN position, start or end, or with equal start and end,
    /// draws no bar.
    ///
    /// The `RangeBars` series of one layer form its range group: they stand
    /// side by side in series order (planned beside actual), laid out by
    /// `Config::bar_groups`' direction, group width and gap ([`BarGroups`]),
    /// whose mode must be grouped. `Vertical` stands the bars up from a
    /// position axis on `bottom_x` (a waterfall, candles); `Horizontal` lays
    /// them along x from a position axis on `left_y` (a Gantt chart). The
    /// position axis is categorical (one slot per category) or linear
    /// numeric, where `slot_width` gives the slot in data units (a time
    /// axis); the value axis is linear or logarithmic.
    ///
    /// `color_by` chooses each bar's fill: the series colour, the direction
    /// of the given ends (`up` when end ≥ start, else `down`, and a host
    /// flagged total), or a host state index into a style list.
    /// `progress` shades the part of a bar past a host fraction; `connector`
    /// joins each bar's end to the next bar.
    ///
    /// Value labels: `labels.text` holds the drawn strings, which the host
    /// writes (the renderer never formats a value), one per row, drawn past
    /// each bar's end (or inside it) as `labels.placement` says, moved by
    /// `labels.offsets`. An empty string draws nothing.
    ///
    /// The columns must be GPU resident; a streamed source is rejected with
    /// an error. Polar charts (`Config.polar`) reject range bars.
    RangeBars {
        /// The fill colour of the series' bars under [`RangeColor::Single`],
        /// opaque; the legend swatch and the whisker colour by default.
        color: Color,
        /// The look of the bars: material, corner radius, outline, texture,
        /// gloss and the brightness of a picked bar.
        #[cfg_attr(
            feature = "serde",
            serde(default = "crate::defaults::category_bar_style")
        )]
        style: CategoryBarStyle,
        /// The column of the start values.
        start: ColumnId,
        /// The whiskers from a low to a high value (a candle's wick).
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        whiskers: Option<RangeWhiskers>,
        /// How each bar's fill is chosen.
        #[cfg_attr(
            feature = "serde",
            serde(
                default = "crate::defaults::range_color",
                skip_serializing_if = "RangeColor::is_single"
            )
        )]
        color_by: RangeColor,
        /// The finished share of each bar (a Gantt task's progress).
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        progress: Option<RangeProgress>,
        /// The line from each bar's end to the next bar (a waterfall).
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        connector: Option<RangeConnector>,
        /// The slot of one row along a numeric position axis, in data units;
        /// `None` on a categorical axis, where the slot is one category.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        slot_width: Option<f64>,
        /// Where the value labels go and the drawn text of each bar. The
        /// renderer never writes the text; `decimals` and `value_suffix` are
        /// not read.
        #[cfg_attr(
            feature = "serde",
            serde(default = "crate::defaults::category_bar_series_labels")
        )]
        labels: CategoryBarSeriesLabels,
    },
    /// The polygon of a radar chart: one vertex per category of the polar
    /// chart's categorical angle axis (`bottom_x` with
    /// [`AxisOptions::categories`]), joined into a closed outline, filled
    /// to the centre (the inner ring) and marked at every vertex, in this
    /// series' colour and look.
    ///
    /// `x_column` is the category index column (the host uploads
    /// `0, 1, ..., n - 1`) and `y_column` the value of each category on the
    /// radius axis (`left_y`), NaN for a missing value. Both columns hold
    /// exactly one value per category. The outline is the polar line path
    /// with its last segment drawn back to the first vertex; a NaN vertex
    /// breaks the outline and the fill on both of its sides, as a NaN
    /// breaks a line. The fill is the polar area path to the inner ring at
    /// [`RadarSeriesStyle::fill_color`], and the markers are the scatter
    /// path. Nothing per vertex is kept on the CPU.
    ///
    /// Value labels: `labels.text` holds the drawn strings
    /// ([`format_radar_labels`] writes them), one per category, drawn
    /// outside each vertex along its spoke when `labels.placement` is
    /// `Values` (or `Emphasized` and the series has a selected vertex or
    /// label), moved by `labels.offsets`. An empty string draws nothing.
    ///
    /// Only a polar chart (`Config.polar`) with a categorical angle axis
    /// draws radar series, in the precise draw style; the columns must be
    /// GPU resident, and a streamed source is rejected with an error.
    Radar {
        /// The outline and marker colour; the fill uses it at the style's
        /// fill opacity unless the style names a fill.
        color: Color,
        /// Outline width, fill and marker; unset fields take
        /// their defaults.
        #[cfg_attr(
            feature = "serde",
            serde(default = "crate::defaults::radar_series_style")
        )]
        style: RadarSeriesStyle,
        /// Which vertices carry value labels, how a value is written and
        /// the drawn text of each vertex.
        #[cfg_attr(
            feature = "serde",
            serde(default = "crate::defaults::radar_series_labels")
        )]
        labels: RadarSeriesLabels,
    },
    /// A ternary series on a ternary chart (`Config.ternary`): markers and/or
    /// a line through samples of three components, `x_column`, `y_column`
    /// and `c_column`, each an ordinary column, unnormalized. A sample is
    /// placed at `(x·L + y·T + c·R) / (x + y + c)`, the triangle's corners
    /// weighted by its components (see [`crate::ternary`]); the renderer
    /// normalizes only to place it and reports the original components. A
    /// sample with a negative or non-finite component, or a sum ≤ 0, is not
    /// drawn and breaks the line. The markers and the line take every
    /// option of a scatter and a line series (style tables, colour and size
    /// maps, dashes) except step modes and fills. With `region`, the series'
    /// samples are the vertices of closed polygons (a NaN row separates
    /// them) filled with [`crate::ternary::TernaryRegionStyle`] under every
    /// series' markers and lines. Streamed sources are rejected.
    Ternary {
        /// The third component's column, read on `right_y`.
        c_column: ColumnId,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        scatter: Option<DataScatterStyleConfig>,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        line: Option<DataLineStyleConfig>,
        /// The fill of the polygons the samples outline; `None` (omitted
        /// from JSON) fills nothing.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        region: Option<crate::ternary::TernaryRegionStyle>,
    },
    /// Streamlines of a flow, electric or magnetic field: polylines the
    /// host integrated from its field, drawn by the line path, with
    /// arrowheads along each drawn line and an optional colour column.
    ///
    /// The renderer draws the paths it is given; integrating the field
    /// into paths is the host's job. `x_column` / `y_column` hold every
    /// streamline of the series one after another, separated by a row of
    /// NaN, which breaks a line as it does in any line series. The line
    /// is drawn with `line` (dash presets included), linearly
    /// interpolated and without a fill.
    ///
    /// With `arrows`, an arrowhead (the vector glyph's head) sits every
    /// `spacing_px` along the drawn path, measured over the whole series
    /// as one arc in pixels, pointing along the drawn path's tangent.
    /// Arrowheads are an appearance of the drawn line, like its dash
    /// phase: their positions follow the view, so a pan or a zoom moves
    /// them along the path and they keep their pixel spacing at every
    /// export scale. A NaN gap holds no arrowhead.
    ///
    /// With `color_map`, the line and its arrowheads take the colour of a
    /// host column (a speed, say), index-aligned with the coordinates,
    /// through `Config.colorbar`; the colourbar is the only authority on
    /// the range, scale, ramp and `nan_color`, as for a colour-mapped
    /// scatter, and a series with a colour map and no colourbar is
    /// rejected. The colour varies along each segment between its two
    /// samples' values. The renderer never computes a magnitude.
    ///
    /// Drawn in the precise draw style only. The columns must be GPU
    /// resident; a streamed source is rejected with an error. Polar charts
    /// (`Config.polar`) reject streamlines.
    Streamlines {
        /// The line the streamlines are drawn with. `interpolation` must
        /// be `Linear` and `fill` `None`.
        line: DataLineStyleConfig,
        /// Arrowheads along the drawn lines, or none.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        arrows: Option<StreamlineArrows>,
        /// Colour the lines and arrowheads by a host column through
        /// `Config.colorbar`.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        color_map: Option<PointColorMap>,
    },
    /// Parallel coordinates (#141): one polyline per row across the axes of
    /// a parallel chart (`Config.parallel`), row `r` through
    /// `(axis i, (v_i − min_i) / (max_i − min_i))` for every axis, the
    /// normalization only to place the sample (rule 1). `columns` names one
    /// column per axis, in the axes' order; `x_column` and `y_column` are
    /// its first two. A row with a non-finite value breaks at that axis.
    /// With `color_column`, each row's polyline takes the colour of that
    /// column's value through `Config.colorbar`; without, the line's
    /// colour. Drawn in the precise draw style only, solid and linear
    /// without a fill; the columns must be GPU resident. See
    /// [`Config::validate_parallel_series`](crate::config::Config::validate_parallel_series).
    Parallel {
        /// One column id per axis of the parallel chart, in order (a `Vec`
        /// of configuration: SSOT, rule 3).
        columns: Vec<ColumnId>,
        /// The rows' line: its colour (without `color_column`) and width.
        line: DataLineStyleConfig,
        /// Colour each row by a host column through `Config.colorbar`.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        color_column: Option<ColumnId>,
    },
    EnergyLevels(crate::energy_levels::EnergyLevelSeries),
    TriMesh(crate::tri_mesh::TriMeshSeries),
    Feynman(crate::feynman::FeynmanSeries),
}

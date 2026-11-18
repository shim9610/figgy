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
            WireRenderType::Histogram(series) => DataRenderType::Histogram(series),
            WireRenderType::RgbImage(series) => DataRenderType::RgbImage(series),
            WireRenderType::Vector(series) => DataRenderType::Vector(series),
            WireRenderType::CategoryBars(series) => DataRenderType::CategoryBars(series),
            WireRenderType::BoxPlot(series) => DataRenderType::BoxPlot(series),
            WireRenderType::RangeBars(series) => DataRenderType::RangeBars(series),
            WireRenderType::Radar(series) => DataRenderType::Radar(series),
            WireRenderType::Ternary(series) => DataRenderType::Ternary(series),
            WireRenderType::Streamlines(series) => DataRenderType::Streamlines(series),
            WireRenderType::Parallel(series) => DataRenderType::Parallel(series),
            WireRenderType::EnergyLevels(levels) => DataRenderType::EnergyLevels(levels),
            WireRenderType::TriMesh(mesh) => DataRenderType::TriMesh(mesh),
            WireRenderType::Feynman(diagram) => DataRenderType::Feynman(diagram),
            WireRenderType::MonotonicLine(series) => DataRenderType::MonotonicLine(series),
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
            DataRenderType::Histogram(series) => WireRenderType::Histogram(series),
            DataRenderType::RgbImage(series) => WireRenderType::RgbImage(series),
            DataRenderType::Vector(series) => WireRenderType::Vector(series),
            DataRenderType::CategoryBars(series) => WireRenderType::CategoryBars(series),
            DataRenderType::BoxPlot(series) => WireRenderType::BoxPlot(series),
            DataRenderType::RangeBars(series) => WireRenderType::RangeBars(series),
            DataRenderType::Radar(series) => WireRenderType::Radar(series),
            DataRenderType::Ternary(series) => WireRenderType::Ternary(series),
            DataRenderType::Streamlines(series) => WireRenderType::Streamlines(series),
            DataRenderType::Parallel(series) => WireRenderType::Parallel(series),
            DataRenderType::EnergyLevels(levels) => WireRenderType::EnergyLevels(levels),
            DataRenderType::TriMesh(mesh) => WireRenderType::TriMesh(mesh),
            DataRenderType::Feynman(diagram) => WireRenderType::Feynman(diagram),
            DataRenderType::MonotonicLine(series) => WireRenderType::MonotonicLine(series),
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
    Histogram(HistogramSeries),
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
    RgbImage(RgbImageSeries),
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
    Vector(VectorSeries),
    CategoryBars(CategoryBarSeries),
    BoxPlot(BoxSeries),
    RangeBars(RangeBarSeries),
    Radar(RadarPolygonSeries),
    Ternary(TernarySeries),
    Streamlines(StreamlineSeries),
    Parallel(ParallelSeries),
    EnergyLevels(crate::energy_levels::EnergyLevelSeries),
    TriMesh(crate::tri_mesh::TriMeshSeries),
    Feynman(crate::feynman::FeynmanSeries),
    MonotonicLine(MonotonicLineSeries),
}

//! Radar series on a polar chart.

use super::*;

/// The marker drawn at every vertex of a radar series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RadarMarker {
    /// No marker.
    None,
    /// A filled circle (the default).
    Circle,
    /// A filled square.
    Square,
}

/// Which vertices of a radar series carry value labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RadarLabels {
    /// No data labels (the default).
    None,
    /// Every vertex of every series.
    Values,
    /// The vertices of the hovered or selected series only.
    Emphasized,
}

/// The look of a radar series. Every field is optional: an unset field
/// takes its default ([`Self::DEFAULT_LINE_WIDTH_PX`] and the other
/// `DEFAULT_*` constants), and a legacy [`RadarChart`](crate::radar::RadarChart)
/// series reads an unset field from its chart style
/// ([`Self::resolved_from`]).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default = "crate::defaults::radar_series_style")
)]
pub struct RadarSeriesStyle {
    /// Outline width in logical pixels, `0..=12`; zero draws no outline.
    pub line_width_px: Option<f32>,
    /// The polygon fill. `None` fills with the series colour at
    /// `fill_opacity`; a transparent colour draws the outline only.
    pub fill: Option<Color>,
    /// The marker at every vertex.
    pub marker: Option<RadarMarker>,
    /// Marker diameter (circle) or side length (square) in logical pixels,
    /// `1..=24`.
    pub marker_size_px: Option<f32>,
    /// Alpha factor of the series colour the fill takes when `fill` is
    /// unset, `0..=1`.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub fill_opacity: Option<f32>,
}

impl RadarSeriesStyle {
    /// The outline width when `line_width_px` is unset.
    pub const DEFAULT_LINE_WIDTH_PX: f32 = 2.0;
    /// The fill opacity when `fill_opacity` is unset.
    pub const DEFAULT_FILL_OPACITY: f32 = 0.25;
    /// The marker when `marker` is unset.
    pub const DEFAULT_MARKER: RadarMarker = RadarMarker::Circle;
    /// The marker size when `marker_size_px` is unset.
    pub const DEFAULT_MARKER_SIZE_PX: f32 = 7.0;

    /// The outline width in logical pixels.
    pub fn line_width(&self) -> f32 {
        self.line_width_px.unwrap_or(Self::DEFAULT_LINE_WIDTH_PX)
    }

    /// The fill colour of a series in `color`: `fill`, or `color` with its
    /// alpha multiplied by the fill opacity.
    pub fn fill_color(&self, color: Color) -> Color {
        self.fill.unwrap_or(Color {
            a: color.a * self.fill_opacity.unwrap_or(Self::DEFAULT_FILL_OPACITY),
            ..color
        })
    }

    /// The vertex marker.
    pub fn marker(&self) -> RadarMarker {
        self.marker.unwrap_or(Self::DEFAULT_MARKER)
    }

    /// The marker diameter or side length in logical pixels.
    pub fn marker_size(&self) -> f32 {
        self.marker_size_px.unwrap_or(Self::DEFAULT_MARKER_SIZE_PX)
    }

    /// This style with every unset field read from a legacy chart style:
    /// what a legacy radar series draws with. The chart style's
    /// `emphasis_brightness` has no counterpart: the picked ring
    /// (`Config.picked_data`) is the highlight of a picked vertex, as on
    /// every polar line or marker, and no series is brightened.
    #[allow(deprecated)] // reads the legacy radar document's style
    pub fn resolved_from(&self, chart: &crate::radar::RadarStyle) -> RadarSeriesStyle {
        RadarSeriesStyle {
            line_width_px: Some(self.line_width_px.unwrap_or(chart.line_width_px)),
            fill: self.fill,
            marker: Some(self.marker.unwrap_or(chart.marker)),
            marker_size_px: Some(self.marker_size_px.unwrap_or(chart.marker_size_px)),
            fill_opacity: Some(self.fill_opacity.unwrap_or(chart.fill_opacity)),
        }
    }

    /// The ranges a set field is held to: an outline width in `0..=12`, a
    /// fill colour with channels in `0..=1`, a marker size in `1..=24` and
    /// a fill opacity in `0..=1`.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        if self.line_width_px.is_some_and(|v| !finite_in(v, 0.0, 12.0))
            || self.fill.is_some_and(|c| !valid_bar_color(c, false))
            || self
                .marker_size_px
                .is_some_and(|v| !finite_in(v, 1.0, 24.0))
            || self.fill_opacity.is_some_and(|v| !finite_in(v, 0.0, 1.0))
        {
            return Err(crate::RuleKind::RadarSeriesStyle);
        }
        Ok(())
    }
}

/// The value labels of a radar series: which vertices show one, how a
/// value is written, and the drawn text of each vertex.
///
/// `text` holds one string per category, written at edit time by
/// [`format_radar_labels`] (or by a document reader); the renderer draws
/// strings and never formats a value. It is empty until written.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default = "crate::defaults::radar_series_labels")
)]
pub struct RadarSeriesLabels {
    pub placement: RadarLabels,
    /// Fixed decimals of a written value, `0..=6`.
    pub decimals: usize,
    /// Appended to a written value; one line of at most 24 characters.
    pub value_suffix: String,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub text: Vec<String>,
    /// Where each label was dragged to, in logical pixels from where it is
    /// placed (x right, y down), one per category in axis order, or empty
    /// when no label was dragged. The renderer adds the offset to the
    /// label's placement on every draw; a drag writes it
    /// ([`SeriesConfig::drag_value_label`]).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub offsets: Vec<[f32; 2]>,
}

/// The most characters of one radar value label.
pub const MAX_RADAR_LABEL_CHARS: usize = 160;

/// The baseline a radar fill reaches: a value below every radius axis, which
/// the polar projection puts on the inner ring (the centre when
/// `inner_radius_fraction` is zero). Finite in both lanes of its `(hi, lo)`
/// pair, so the fill's sample check keeps it.
pub const RADAR_FILL_BASELINE: f64 = -(f32::MAX as f64);

/// The fields of a [`DataRenderType::Radar`] series, borrowed.
#[derive(Clone, Copy, Debug)]
pub struct RadarRef<'a> {
    pub color: Color,
    pub style: &'a RadarSeriesStyle,
    pub labels: &'a RadarSeriesLabels,
}

impl RadarRef<'_> {
    /// The polar line a radar series' outline and fill are drawn as: a
    /// solid, linearly interpolated line in the series colour at the
    /// outline width (transparent at width zero) whose fill reaches the
    /// inner ring ([`RADAR_FILL_BASELINE`]) in the fill colour, or no fill
    /// when that colour is transparent. The renderer draws it closed.
    pub fn line_style(&self) -> DataLineStyleConfig {
        let width = self.style.line_width();
        let fill = self.style.fill_color(self.color);
        DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: if width > 0.0 {
                self.color
            } else {
                Color {
                    a: 0.0,
                    ..self.color
                }
            },
            line_width: width,
            fill: (fill.a > 0.0).then_some(LineFill::ToBaseline(RADAR_FILL_BASELINE)),
            fill_color: Some(fill),
            interpolation: LineInterpolation::Linear,
        }
    }

    /// The markers a radar series' vertices are drawn with, `None` without
    /// a marker: filled circles of the marker size as diameter, or filled
    /// squares of the marker size as side length, in the series colour.
    pub fn scatter_style(&self) -> Option<DataScatterStyleConfig> {
        let half = self.style.marker_size() * 0.5;
        let (point_shape, point_size) = match self.style.marker() {
            RadarMarker::None => return None,
            RadarMarker::Circle => (ScatterShape::CircleFilled, half),
            // The shared marker sizes a square by the circle of equal area
            // (`visual_shape_radius`): its half side is `sqrt(pi) / 2` of
            // the radius.
            RadarMarker::Square => (ScatterShape::SquareFilled, half / SQUARE_HALF_SIDE),
        };
        Some(DataScatterStyleConfig {
            point_color: self.color,
            point_shape,
            point_size,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        })
    }
}

/// The half side of a filled square marker of radius 1 (`sqrt(pi) / 2`,
/// SHADER_COMMON.md §5.5).
pub(super) const SQUARE_HALF_SIDE: f32 = 0.886_226_95;

impl DataRenderType {
    /// The fields of a radar series, or `None` for every other kind.
    pub fn radar(&self) -> Option<RadarRef<'_>> {
        match self {
            DataRenderType::Radar {
                color,
                style,
                labels,
            } => Some(RadarRef {
                color: *color,
                style,
                labels,
            }),
            _ => None,
        }
    }
}

/// The rules one radar series is held to against the `categories` of its
/// angle axis: a colour with channels in `0..=1`, a valid style, label
/// decimals `0..=6`, a one-line value suffix of at most 24 characters,
/// `labels.text` empty or one one-line string of at most
/// [`MAX_RADAR_LABEL_CHARS`] characters per category, and named category
/// index and value columns.
pub fn validate_radar_series(
    series: &SeriesConfig,
    categories: &[Category],
) -> Result<(), crate::RuleKind> {
    let Some(radar) = series.render_type.radar() else {
        return Ok(());
    };
    if !valid_bar_color(radar.color, false) {
        return Err(crate::RuleKind::RadarColor);
    }
    radar.style.validate()?;
    if radar.labels.decimals > 6 || !single_line_within(&radar.labels.value_suffix, 24) {
        return Err(crate::RuleKind::RadarLabelFormat);
    }
    if !radar.labels.text.is_empty() && radar.labels.text.len() != categories.len() {
        return Err(crate::RuleKind::RadarLabelTextCount);
    }
    validate_value_label_offsets(&radar.labels.offsets, categories.len())?;
    if radar
        .labels
        .text
        .iter()
        .any(|text| !single_line_within(text, MAX_RADAR_LABEL_CHARS))
    {
        return Err(crate::RuleKind::RadarLabel);
    }
    if series.x_column.is_empty() || series.y_column.is_empty() {
        return Err(crate::RuleKind::RadarColumns);
    }
    Ok(())
}

impl crate::config::Config {
    /// The rules the radar series of a chart are held to, with the series
    /// list it draws.
    ///
    /// A radar series is drawn on a polar chart (`Config.polar`, the
    /// parent's or a subplot's own) whose angle axis (`bottom_x`) is
    /// categorical, with chord segments
    /// ([`PolarInterpolation::Chord`](crate::polar::PolarInterpolation)):
    /// a polygon's edges are straight. Any number of radar series share
    /// one chart (#107), and each passes [`validate_radar_series`] against
    /// the angle axis' categories. Whether a column holds one value per
    /// category is data: the renderer checks it against the pool when it
    /// prepares.
    /// The rules a hexagonal grid ([`GridLayout::Hexagonal`]) is held to:
    /// it is the grid of a `Heatmap` (a contour or a heatmap with contours
    /// reads a rectangular lattice, as does a grid vector), shaded flat, on a
    /// flat chart (no `view_3d`) whose x and y axes are linear, so the
    /// lattice is uniform on screen as it is in data. The coordinate
    /// columns' uniformity is the renderer's check, from their statistics.
    pub fn validate_hexagonal_series(
        &self,
        series: &[SeriesConfig],
    ) -> Result<(), crate::RuleKind> {
        for item in series {
            let (matrix, shading) = match &item.render_type {
                DataRenderType::Field(crate::data_config::FieldSeries {
                    matrix,
                    fill: Some(fill),
                    contour: None,
                    surface: _,
                }) => (matrix, Some(&fill.shading)),
                // An image's three grids share one layout (its own rule).
                DataRenderType::RgbImage { red, shading, .. } => (red, Some(shading)),
                DataRenderType::Field(crate::data_config::FieldSeries {
                    matrix,
                    contour: Some(_),
                    ..
                }) => (matrix, None),
                DataRenderType::Vector {
                    components: VectorComponents::Grid { u, v },
                    style,
                } => {
                    let color = match &style.color_map {
                        Some(VectorColorMap::Grid(color)) => Some(color),
                        _ => None,
                    };
                    if [Some(u), Some(v), color]
                        .into_iter()
                        .flatten()
                        .any(|grid| grid.grid_layout == GridLayout::Hexagonal)
                    {
                        return Err(crate::RuleKind::HexagonalGridVector);
                    }
                    continue;
                }
                _ => continue,
            };
            if matrix.grid_layout != GridLayout::Hexagonal {
                continue;
            }
            let Some(shading) = shading else {
                return Err(crate::RuleKind::HexagonalContours);
            };
            if *shading != Shading::Flat {
                return Err(crate::RuleKind::HexagonalShading);
            }
            if self.view_3d().is_some() {
                return Err(crate::RuleKind::View3dHexagonalGrid);
            }
            if self.bottom_x.scale != AxisScale::Linear || self.left_y.scale != AxisScale::Linear {
                return Err(crate::RuleKind::HexagonalAxes);
            }
            // A `Continuous` break remaps the axis, so the lattice would not
            // be uniform; each `Panels` piece is linear and keeps it (#200).
            let continuous_cut = |axis: &crate::config::AxisOptions| {
                axis.breaks.as_ref().is_some_and(|breaks| {
                    !breaks.ranges.is_empty()
                        && breaks.style == crate::config::AxisBreakStyle::Continuous
                })
            };
            if continuous_cut(&self.bottom_x) || continuous_cut(&self.left_y) {
                return Err(crate::RuleKind::HexagonalBrokenAxis);
            }
        }
        Ok(())
    }

    /// The rules an image series ([`DataRenderType::RgbImage`], #139) is
    /// held to: its three grids share their column count, orientation and
    /// grid layout (one lattice, three tables), `range` is finite with a
    /// smaller low end, `opacity` is in `0..=1`, and the chart is flat (no
    /// `view_3d`). Polar charts reject every field series (the renderer's
    /// rule), and a hexagonal lattice follows
    /// [`validate_hexagonal_series`](Self::validate_hexagonal_series).
    pub fn validate_rgb_image_series(
        &self,
        series: &[SeriesConfig],
    ) -> Result<(), crate::RuleKind> {
        for item in series {
            let DataRenderType::RgbImage {
                red,
                green,
                blue,
                range,
                opacity,
                ..
            } = &item.render_type
            else {
                continue;
            };
            if green.columns.len() != red.columns.len() || blue.columns.len() != red.columns.len() {
                return Err(crate::RuleKind::ImageChannelColumns);
            }
            if green.orientation != red.orientation
                || blue.orientation != red.orientation
                || green.grid_layout != red.grid_layout
                || blue.grid_layout != red.grid_layout
            {
                return Err(crate::RuleKind::ImageChannelLattice);
            }
            if !range[0].is_finite() || !range[1].is_finite() || !(range[0] < range[1]) {
                return Err(crate::RuleKind::ImageRange);
            }
            if !opacity.is_finite() || !(0.0..=1.0).contains(opacity) {
                return Err(crate::RuleKind::ImageOpacity);
            }
            if self.view_3d().is_some() {
                return Err(crate::RuleKind::View3dImage);
            }
        }
        Ok(())
    }

    pub fn validate_radar(&self, series: &[SeriesConfig]) -> Result<(), crate::RuleKind> {
        use crate::layer::LayerRef;
        let radars = || series.iter().filter(|s| s.render_type.radar().is_some());
        if radars().next().is_none() {
            return Ok(());
        }
        for layer in self.panel_layers() {
            let members = || radars().filter(|s| self.layer_of_series(&s.series_id) == layer);
            if members().next().is_none() {
                continue;
            }
            let subplot = match layer {
                LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                    self.subplot_config(index)
                }
                LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => None,
            };
            let chart = subplot.as_ref().unwrap_or(self);
            let Some(polar) = chart.polar() else {
                return Err(crate::RuleKind::RadarNotPolar);
            };
            let Some(categories) = chart.bottom_x.categories.as_deref() else {
                return Err(crate::RuleKind::RadarAngleAxis);
            };
            if !polar.interpolation.is_chord() {
                return Err(crate::RuleKind::RadarInterpolation);
            }
            for item in members() {
                validate_radar_series(item, categories)?;
            }
        }
        Ok(())
    }
}

/// Write `labels.text` of every [`DataRenderType::Radar`] series in
/// `series` from the values the host holds for it: the edit-time helper
/// the renderer relies on, since it never formats a value.
///
/// `value(k, i)` is the value of category `i` of the `k`-th radar series in
/// `series` (`None` for a missing value). `categories` are the angle axis'
/// categories, whose order the text follows. With `labels.placement`
/// [`RadarLabels::None`] every text is empty; otherwise a vertex's text is
/// its value with `labels.decimals` fixed decimals and
/// `labels.value_suffix`, and a missing or non-finite value's text is
/// empty. Other series are left alone.
pub fn format_radar_labels(
    series: &mut [SeriesConfig],
    categories: &[Category],
    value: impl Fn(usize, usize) -> Option<f64>,
) {
    let mut k = 0;
    for item in series.iter_mut() {
        let DataRenderType::Radar { labels, .. } = &mut item.render_type else {
            continue;
        };
        let number = crate::format::PlainNumberFormat::fixed(labels.decimals, false);
        labels.text.clear();
        for i in 0..categories.len() {
            let text = match (labels.placement, value(k, i)) {
                (RadarLabels::None, _) | (_, None) => String::new(),
                (_, Some(v)) if !v.is_finite() => String::new(),
                (_, Some(v)) => format!("{}{}", number.format(v), labels.value_suffix),
            };
            labels.text.push(text);
        }
        k += 1;
    }
}

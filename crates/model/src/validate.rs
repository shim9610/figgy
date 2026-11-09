//! One place for every rule a chart's configuration and its series list are
//! held to before a renderer accepts them (#160).
//!
//! [`Config::validate_chart`] checks the chart alone: its target, its axes,
//! overlays, subplots, layout, texts, annotations and line offset layout.
//! [`Config::validate_series`] checks a series list against the chart: every
//! chart-wide series rule (category bars, box plots, range bars, radar,
//! ternary, parallel, 3D, hexagonal grids, line offsets, polar axes) and the
//! series ids. Both return a [`ConfigError`] that names the field and, for a
//! series rule, the series it is about. The renderer adds only what needs
//! its column pool (registered columns, lengths, GPU limits).

use crate::config::{AxisOptions, AxisScale, Config};
use crate::data_config::{DataRenderType, SeriesConfig};

/// Why a chart's configuration or series list is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// The series the rule refused, `None` for a rule on the chart itself.
    pub series_id: Option<String>,
    /// The config field or rule family that failed, such as `"bottom_x"`,
    /// `"layout"` or `"category_bars"`.
    pub field: &'static str,
    /// Which rule failed; its `Display` is the sentence (#215).
    pub kind: crate::RuleKind,
}

impl ConfigError {
    fn chart(field: &'static str, kind: crate::RuleKind) -> Self {
        Self {
            series_id: None,
            field,
            kind,
        }
    }

    /// The rule's sentence, for a host that only wants text.
    pub fn reason(&self) -> String {
        self.kind.to_string()
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.series_id {
            Some(id) => write!(f, "series {id:?} ({}): {}", self.field, self.kind),
            None => write!(f, "{}: {}", self.field, self.kind),
        }
    }
}

impl std::error::Error for ConfigError {}

/// An axis range's own rules: categories well formed, a finite crossing, and
/// for a value axis finite, increasing bounds that are positive on a
/// logarithmic scale.
fn validate_axis_range(field: &'static str, axis: &AxisOptions) -> Result<(), ConfigError> {
    let fail = |reason| Err(ConfigError::chart(field, reason));
    if let Err(reason) = axis.validate_categories() {
        return fail(reason);
    }
    if let Err(reason) = axis.validate_breaks() {
        return fail(reason);
    }
    if axis.cross_at.is_some_and(|v| !v.is_finite()) {
        return fail(crate::RuleKind::AxisCrossAt);
    }
    // A categorical axis maps its category range; `min` / `max` are ignored
    // there.
    if axis.categories.is_some() {
        return match axis.category_window {
            Some([from, to]) if !(from.is_finite() && to.is_finite() && from < to) => {
                fail(crate::RuleKind::CategoryWindow)
            }
            _ => Ok(()),
        };
    }
    if !axis.min.is_finite() || !axis.max.is_finite() {
        return fail(crate::RuleKind::AxisBoundsNotFinite);
    }
    if axis.max <= axis.min {
        return fail(crate::RuleKind::AxisBoundsOrder);
    }
    if matches!(axis.scale, AxisScale::Logarithmic) && axis.min <= 0.0 {
        return fail(crate::RuleKind::LogAxisBounds);
    }
    if matches!(axis.scale, AxisScale::Probability(_)) && !(axis.min > 0.0 && axis.max < 1.0) {
        return fail(crate::RuleKind::ProbabilityAxisBounds);
    }
    Ok(())
}

impl Config {
    /// Every rule on the chart itself, in a fixed order: a non-empty target,
    /// the four axes and the colourbar's (finite, increasing, positive on a
    /// log scale, a colourbar axis never categorical), the overlay axes and
    /// layer membership, the subplot rules, the parallel axis count, the
    /// layout (a non-empty data area), the math in every title and legend,
    /// the annotations, the line offset layout, the overlay expansion, and
    /// each subplot held to the same rules with its links applied.
    pub fn validate_chart(&self) -> Result<(), ConfigError> {
        let area = &self.chart_area.0;
        if area.width == 0 || area.height == 0 {
            return Err(ConfigError::chart(
                "chart_area",
                crate::RuleKind::ChartAreaEmpty,
            ));
        }
        // The colourbar's z axis is held to the four chart axes' rules: its
        // range means the same thing. A log colourbar with a non-positive
        // lower bound has no `t` for its lowest colour.
        let colorbar_axis = self.colorbar.as_ref().map(|bar| &bar.axis);
        if colorbar_axis.is_some_and(|axis| axis.categories.is_some()) {
            return Err(ConfigError::chart(
                "colorbar.axis",
                crate::RuleKind::ColorbarCategorical,
            ));
        }
        if colorbar_axis.is_some_and(|axis| matches!(axis.scale, AxisScale::Probability(_))) {
            return Err(ConfigError::chart(
                "colorbar.axis",
                crate::RuleKind::ColorbarProbabilityAxis,
            ));
        }
        let axes = [
            ("top_x", &self.top_x),
            ("bottom_x", &self.bottom_x),
            ("left_y", &self.left_y),
            ("right_y", &self.right_y),
        ];
        for (field, axis) in axes
            .into_iter()
            .chain(colorbar_axis.map(|axis| ("colorbar.axis", axis)))
        {
            validate_axis_range(field, axis)?;
            // Group headers (#203) run along a categorical x axis.
            if axis.category_groups.is_some()
                && (axis.categories.is_none() || !matches!(field, "top_x" | "bottom_x"))
            {
                return Err(ConfigError::chart(
                    field,
                    crate::RuleKind::CategoryGroupsAxis,
                ));
            }
        }
        // Breaks (#200) cut the data axes of a rectangular chart: the x and
        // y the data is placed on, and an overlay's own y. The secondary
        // axes and the colourbar draw no data of their own to cut, and the
        // other coordinate systems have no straight value axis.
        for (field, axis) in [("top_x", &self.top_x), ("right_y", &self.right_y)]
            .into_iter()
            .chain(colorbar_axis.map(|axis| ("colorbar.axis", axis)))
        {
            if axis.breaks.is_some() {
                return Err(ConfigError::chart(field, crate::RuleKind::BreaksAxis));
            }
        }
        let broken = self.bottom_x.breaks.is_some()
            || self.left_y.breaks.is_some()
            || self.overlays.iter().any(|o| o.y_axis.breaks.is_some());
        if broken && !matches!(self.coordinates, crate::coordinates::Coordinates::Cartesian) {
            return Err(ConfigError::chart(
                "coordinates",
                crate::RuleKind::BreaksChart,
            ));
        }
        // Overlay y axes are ranges of the same kind, and the layer
        // membership must be unambiguous before a series is drawn on a panel.
        for overlay in &self.overlays {
            validate_axis_range("overlays.y_axis", &overlay.y_axis)?;
            if overlay.y_axis.category_groups.is_some() {
                return Err(ConfigError::chart(
                    "overlays.y_axis",
                    crate::RuleKind::CategoryGroupsAxis,
                ));
            }
        }
        self.validate_overlays()
            .map_err(|_| ConfigError::chart("overlays", crate::RuleKind::SeriesInManyOverlays))?;
        // The subplot rules come before the layout check, which also expands
        // each subplot, so a broken link or nesting reports as what it is.
        self.validate_subplots()
            .map_err(|reason| ConfigError::chart("subplots", reason))?;
        // The parallel axis count is checked with the chart, so a series
        // checked against it sees at least two axes.
        if let Some(parallel) = self.parallel() {
            parallel
                .validate_axis_count()
                .map_err(|reason| ConfigError::chart("parallel", reason))?;
        }
        self.validate()
            .map_err(|_| ConfigError::chart("layout", crate::RuleKind::LayoutDataAreaEmpty))?;
        // A malformed math segment tree in a title, axis title or legend is
        // refused like any other field (a JSON document already fails to
        // parse); annotation texts are checked with the annotations.
        let texts = [
            ("chart_title", &self.chart_title.text),
            ("legend", &self.legend.content),
        ]
        .into_iter()
        .chain(axes.map(|(field, axis)| (field, &axis.title_option.text)))
        .chain(colorbar_axis.map(|axis| ("colorbar.axis", &axis.title_option.text)))
        .chain(self.overlays.iter().flat_map(|overlay| {
            [("overlays.y_axis", &overlay.y_axis.title_option.text)]
                .into_iter()
                .chain(
                    overlay
                        .legend
                        .as_ref()
                        .map(|legend| ("overlays.legend", &legend.content)),
                )
        }));
        for (field, text) in texts {
            text.validate_math().map_err(|error| {
                ConfigError::chart(field, crate::RuleKind::InvalidMath { error })
            })?;
        }
        // Malformed annotation styles are refused like any other field;
        // non-finite values and anchors are not malformed, the raster skips
        // them.
        self.annotations
            .validate()
            .map_err(|reason| ConfigError::chart("annotations", reason))?;
        self.validate_line_offset_layout()
            .map_err(|reason| ConfigError::chart("line_offsets", reason))?;
        if !self.overlays.is_empty() {
            self.expand()
                .map_err(|_| ConfigError::chart("overlays", crate::RuleKind::OverlayDataArea))?;
        }
        // Every subplot is a chart of its own, held to the same rules with
        // its links applied.
        for index in 0..self.subplots.len() {
            self.subplot_config(index)
                .ok_or(ConfigError::chart(
                    "subplots",
                    crate::RuleKind::SubplotIndex,
                ))?
                .validate_chart()?;
        }
        Ok(())
    }

    /// Every rule on a series list drawn on this chart, in a fixed order:
    /// the series ids (non-empty, unique), the category bar, box plot, range
    /// bar, radar, ternary, parallel, energy-level, 3D, hexagonal grid and
    /// image rules, the line
    /// offset group, and each series against polar axes. A chart-wide rule
    /// names the first series whose addition to the list breaks it; a rule
    /// the empty list already breaks names none.
    pub fn validate_series(&self, series: &[SeriesConfig]) -> Result<(), ConfigError> {
        for (index, item) in series.iter().enumerate() {
            let fail = |reason| ConfigError {
                series_id: Some(item.series_id.clone()),
                field: "series_id",
                kind: reason,
            };
            if item.series_id.is_empty() {
                return Err(fail(crate::RuleKind::SeriesIdEmpty));
            }
            if series[..index]
                .iter()
                .any(|earlier| earlier.series_id == item.series_id)
            {
                return Err(fail(crate::RuleKind::SeriesIdDuplicate));
            }
            // A composed kind draws only the combinations it has a form for.
            let composition = match &item.render_type {
                DataRenderType::Xy(xy) => xy.validate(),
                DataRenderType::Field(field) => field.validate(),
                _ => Ok(()),
            };
            composition.map_err(|reason| ConfigError {
                series_id: Some(item.series_id.clone()),
                field: "render_type",
                kind: reason,
            })?;
        }
        type Rule = fn(&Config, &[SeriesConfig]) -> Result<(), crate::RuleKind>;
        let rules: [(&'static str, Rule); 13] = [
            ("category_bars", Config::validate_category_bars),
            ("box_plots", Config::validate_box_plots),
            ("range_bars", Config::validate_range_bars),
            ("radar", Config::validate_radar),
            ("ternary", Config::validate_ternary_series),
            ("parallel", Config::validate_parallel_series),
            ("energy_levels", Config::validate_energy_levels),
            ("tri_mesh", Config::validate_tri_mesh_series),
            ("feynman", Config::validate_feynman),
            ("view_3d", Config::validate_view3d_series),
            ("hexagonal", Config::validate_hexagonal_series),
            ("rgb_image", Config::validate_rgb_image_series),
            ("line_offsets", Config::validate_line_offsets),
        ];
        for (field, rule) in rules {
            if let Err(reason) = rule(self, series) {
                return Err(ConfigError {
                    series_id: self.first_breaking(series, rule),
                    field,
                    kind: reason,
                });
            }
        }
        for item in series {
            self.validate_series_on_polar(item)
                .map_err(|reason| ConfigError {
                    series_id: Some(item.series_id.clone()),
                    field: "polar",
                    kind: reason,
                })?;
        }
        Ok(())
    }

    /// The series whose addition to the list first breaks `rule`: the
    /// shortest failing prefix ends with it. `None` when the empty list
    /// already breaks it (a rule on the chart's own fields).
    fn first_breaking(
        &self,
        series: &[SeriesConfig],
        rule: fn(&Config, &[SeriesConfig]) -> Result<(), crate::RuleKind>,
    ) -> Option<String> {
        (0..=series.len())
            .find(|&end| rule(self, &series[..end]).is_err())
            .and_then(|end| end.checked_sub(1))
            .map(|last| series[last].series_id.clone())
    }

    /// Whether `item` can be drawn on this chart's polar axes; always `Ok`
    /// on a chart without them. Points, lines, radar series and category
    /// bars (as a rose, #133) are; a polar arc interpolation needs a
    /// linearly interpolated line, since a step draws chords.
    pub fn validate_series_on_polar(&self, item: &SeriesConfig) -> Result<(), crate::RuleKind> {
        let Some(polar) = self.polar() else {
            return Ok(());
        };
        let line = match &item.render_type {
            DataRenderType::Xy(xy) if xy.errors.is_some() => {
                return Err(crate::RuleKind::PolarErrorBars);
            }
            DataRenderType::Xy(xy) if xy.stem.is_some() => {
                return Err(crate::RuleKind::PolarStems);
            }
            DataRenderType::Xy(xy) => xy.line.as_ref().map(|line| line.interpolation.is_linear()),
            DataRenderType::CategoryBars { .. } => None,
            DataRenderType::Radar { .. } => item
                .render_type
                .radar()
                .map(|radar| radar.line_style().interpolation.is_linear()),
            DataRenderType::Histogram { .. } => {
                return Err(crate::RuleKind::PolarHistogram);
            }
            // A polar heatmap (#208): cells are annular sectors between angle
            // and radius boundaries, so the lattice must be rectangular in
            // (angle, radius) and the rings circles.
            DataRenderType::Field(field) => {
                if field.matrix.grid_layout == crate::data_config::GridLayout::Hexagonal {
                    return Err(crate::RuleKind::PolarHexagonalGrid);
                }
                if polar.grid_shape() == crate::polar::GridShape::Polygon {
                    return Err(crate::RuleKind::PolarHeatmapPolygonGrid);
                }
                None
            }
            DataRenderType::RgbImage { .. } => {
                return Err(crate::RuleKind::PolarImage);
            }
            DataRenderType::Vector { .. } => {
                return Err(crate::RuleKind::PolarVector);
            }
            DataRenderType::BoxPlot { .. } => {
                return Err(crate::RuleKind::PolarBoxPlot);
            }
            DataRenderType::RangeBars { .. } => {
                return Err(crate::RuleKind::PolarRangeBars);
            }
            DataRenderType::Streamlines { .. } => {
                return Err(crate::RuleKind::PolarStreamlines);
            }
            DataRenderType::Parallel { .. } => {
                return Err(crate::RuleKind::PolarParallel);
            }
            DataRenderType::EnergyLevels(_) => {
                return Err(crate::RuleKind::PolarEnergyLevels);
            }
            DataRenderType::TriMesh(_) => {
                return Err(crate::RuleKind::PolarTriMesh);
            }
            DataRenderType::Feynman(_) => {
                return Err(crate::RuleKind::PolarFeynman);
            }
            DataRenderType::Ternary { .. } => {
                return Err(crate::RuleKind::PolarTernary);
            }
        };
        if line == Some(false)
            && matches!(
                polar.interpolation,
                crate::polar::PolarInterpolation::Arc { .. }
            )
        {
            return Err(crate::RuleKind::PolarArcStepLine);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::data_config::{DataScatterStyleConfig, ScatterShape};
    #[cfg(feature = "serde")]
    use crate::polar::PolarOptions;

    fn scatter(id: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::Xy(crate::data_config::XySeries {
                scatter: Some(DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: ScatterShape::CircleFilled,
                    point_size: 3.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                }),
                line: None,
                errors: None,
                stem: None,
            }),
        }
    }

    #[test]
    fn series_ids_are_named_by_the_series_that_breaks_them() {
        let config = crate::defaults::config();
        assert_eq!(
            config.validate_series(&[scatter("a"), scatter("b")]),
            Ok(())
        );
        let error = config
            .validate_series(&[scatter("a"), scatter("b"), scatter("a")])
            .unwrap_err();
        assert_eq!(error.series_id.as_deref(), Some("a"));
        assert_eq!(error.field, "series_id");
        let error = config.validate_series(&[scatter("")]).unwrap_err();
        assert_eq!(error.series_id.as_deref(), Some(""));
    }

    /// A stem needs markers, a finite baseline and a finite width of at
    /// least 0, and it is refused on polar and 3D charts (#206). The colour
    /// cycle recolours it with the rest of the series.
    #[test]
    fn stem_rules_are_checked() {
        use crate::data_config::{CategoryBarDirection, Stem, XySeries};
        let stem = Stem {
            baseline: 0.0,
            direction: CategoryBarDirection::Vertical,
            color: Color::BLACK,
            width: 1.0,
            line_style: crate::line::LineStylePreset::Solid,
        };
        let with = |stem: Stem| {
            let mut series = scatter("s");
            if let DataRenderType::Xy(xy) = &mut series.render_type {
                xy.stem = Some(stem);
            }
            series
        };
        let config = crate::defaults::config();
        assert_eq!(config.validate_series(&[with(stem.clone())]), Ok(()));
        let reason = |series: SeriesConfig| config.validate_series(&[series]).unwrap_err().kind;
        assert_eq!(
            reason(with(Stem {
                baseline: f64::NAN,
                ..stem.clone()
            })),
            crate::RuleKind::StemBaseline
        );
        for width in [-1.0, f32::INFINITY, f32::NAN] {
            assert_eq!(
                reason(with(Stem {
                    width,
                    ..stem.clone()
                })),
                crate::RuleKind::StemWidth
            );
        }
        let mut bare = with(stem.clone());
        if let DataRenderType::Xy(XySeries { scatter, line, .. }) = &mut bare.render_type {
            *scatter = None;
            *line = Some(crate::defaults::data_line_style_config());
        }
        assert_eq!(reason(bare), crate::RuleKind::StemsWithoutMarkers);

        let mut polar = crate::defaults::config();
        polar.bottom_x.min = 0.0;
        polar.bottom_x.max = 360.0;
        polar.set_polar(Some(crate::defaults::polar_options()));
        assert_eq!(
            polar.validate_series_on_polar(&with(stem.clone())),
            Err(crate::RuleKind::PolarStems)
        );
        let mut three_d = crate::defaults::config();
        three_d.set_view_3d(Some(crate::defaults::view3d()));
        let mut tall = with(stem.clone());
        tall.z_column = Some("z".into());
        assert_eq!(
            three_d.validate_view3d_series(&[tall]),
            Err(crate::RuleKind::View3dStem)
        );

        let mut cycled = with(stem);
        crate::preset::ColorCycle::Balanced.apply_to_series(&mut cycled, 3);
        let DataRenderType::Xy(XySeries {
            stem: Some(stem), ..
        }) = &cycled.render_type
        else {
            panic!("the stem stays");
        };
        assert_eq!(stem.color, crate::preset::ColorCycle::Balanced.color(3));
    }

    #[test]
    fn a_polar_chart_names_the_series_it_does_not_draw() {
        let mut config = crate::defaults::config();
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 360.0;
        config.set_polar(Some(crate::defaults::polar_options()));
        let mut vector = scatter("v");
        vector.render_type = DataRenderType::Streamlines {
            line: crate::defaults::data_line_style_config(),
            arrows: None,
            color_map: None,
        };
        let error = config.validate_series(&[scatter("a"), vector]);
        let error = error.unwrap_err();
        assert_eq!(error.series_id.as_deref(), Some("v"));
        assert_eq!(error.field, "polar");
        assert_eq!(
            crate::defaults::config().validate_series_on_polar(&scatter("a")),
            Ok(())
        );
    }

    fn polar_field(layout: crate::data_config::GridLayout, contour: bool) -> SeriesConfig {
        use crate::data_config::{
            ContourConfig, FieldFillConfig, FieldSeries, FillMode, MatrixOrientation, MatrixRef,
            Shading,
        };
        let mut series = scatter("f");
        series.render_type = DataRenderType::Field(FieldSeries {
            matrix: MatrixRef {
                columns: vec!["z0".into(), "z1".into()],
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: layout,
            },
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            }),
            contour: contour.then(|| ContourConfig {
                levels: vec![0.5],
                line: crate::defaults::data_line_style_config(),
                per_level_color: None,
                labels: None,
            }),
            surface: None,
        });
        series
    }

    /// A polar heatmap (#208): fills and contours draw on circular rings; a
    /// hexagonal lattice and a polygon grid are refused with their reasons.
    #[test]
    fn a_polar_heatmap_is_held_to_its_rules() {
        use crate::data_config::GridLayout;
        let mut config = crate::defaults::config();
        config.set_polar(Some(crate::defaults::polar_options()));
        for layout in [GridLayout::Edges, GridLayout::Centers] {
            for contour in [false, true] {
                assert_eq!(
                    config.validate_series_on_polar(&polar_field(layout.clone(), contour)),
                    Ok(())
                );
            }
        }
        assert_eq!(
            config.validate_series_on_polar(&polar_field(GridLayout::Hexagonal, false)),
            Err(crate::RuleKind::PolarHexagonalGrid)
        );
        config.polar_mut().unwrap().grid_shape = Some(crate::polar::GridShape::Polygon);
        assert_eq!(
            config.validate_series_on_polar(&polar_field(GridLayout::Edges, false)),
            Err(crate::RuleKind::PolarHeatmapPolygonGrid)
        );
    }

    /// A polar heatmap document has no new key: it parses and re-serializes
    /// byte-identically.
    #[cfg(feature = "serde")]
    #[test]
    fn a_polar_heatmap_document_round_trips() {
        use crate::data_config::GridLayout;
        let mut config = crate::defaults::config();
        config.set_polar(Some(PolarOptions {
            inner_radius_fraction: 0.1,
            clockwise: true,
            ..crate::defaults::polar_options()
        }));
        let series = vec![polar_field(GridLayout::Edges, true)];
        let config_json = serde_json::to_string(&config).unwrap();
        let series_json = serde_json::to_string(&series).unwrap();
        let config_back: crate::config::Config = serde_json::from_str(&config_json).unwrap();
        let series_back: Vec<SeriesConfig> = serde_json::from_str(&series_json).unwrap();
        assert_eq!(config_back, config);
        assert_eq!(series_back, series);
        assert_eq!(serde_json::to_string(&config_back).unwrap(), config_json);
        assert_eq!(serde_json::to_string(&series_back).unwrap(), series_json);
        assert_eq!(config_back.validate_series(&series_back), Ok(()));
    }

    #[test]
    fn the_chart_rules_name_the_field() {
        let mut config = crate::defaults::config();
        assert_eq!(config.validate_chart(), Ok(()));
        config.bottom_x.max = config.bottom_x.min;
        let error = config.validate_chart().unwrap_err();
        assert_eq!((error.series_id, error.field), (None, "bottom_x"));
    }
}

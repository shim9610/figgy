//! Parallel coordinates for the Cartesian chart: `Config.parallel` (#141).
//!
//! With `parallel: Some(..)` the chart draws one vertical axis per category
//! of `bottom_x`, in order, each with its own range
//! ([`ParallelAxis`]), and every [`DataRenderType::Parallel`] series draws
//! row `r` of its columns as the polyline through
//! `(axis i, (v_i − min_i) / (max_i − min_i))` for every axis: the
//! categorical `bottom_x` places the axes and names them, `left_y` spans
//! the normalized band `0..1` the values are placed on. The normalization
//! only places a value on its axis: it is computed in the vertex stage on
//! every draw and never stored, and hover, selection and the reported
//! values are the original columns. A row with a non-finite value breaks
//! at that axis.
//!
//! The axis names, their drag and the title stay the categorical axis's;
//! each parallel axis's tick labels are their own element
//! ([`Element::ParallelAxisLabel`]), dragged into the axis's
//! `label_offset`. The `left_y` axis is read and not drawn.
//!
//! A parallel chart draws only parallel series, in the Precise draw style;
//! overlays, line offsets, the polar, ternary and 3D systems, subplot axis
//! links and streamed sources are rejected. `None` (the default, omitted
//! from JSON) draws the rectangular chart.
//!
//! [`Element::ParallelAxisLabel`]: crate::layout::Element::ParallelAxisLabel

use crate::config::{AxisOptions, AxisScale, Config};
use crate::data_config::{DataRenderType, LineInterpolation, SeriesConfig};
use crate::line::LineStylePreset;

/// One axis of a parallel chart: its own range and scale, the direction
/// its values grow in, and the offset its tick labels were dragged by.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParallelAxis {
    /// The value at the bottom of the axis (the top when `inverted`).
    pub min: f64,
    /// The value at the top of the axis (the bottom when `inverted`).
    pub max: f64,
    /// Values grow downward.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_false"))]
    pub inverted: bool,
    /// Linear or logarithmic placement along the axis.
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::parallel_axis_scale",
            skip_serializing_if = "is_linear"
        )
    )]
    pub scale: AxisScale,
    /// The tick labels' offset from their drawn place, logical pixels, as
    /// an axis's `label_offset_x` / `label_offset_y`; written by a drag of
    /// [`Element::ParallelAxisLabel`](crate::layout::Element::ParallelAxisLabel).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "is_zero_offset")
    )]
    pub label_offset: [f32; 2],
}

#[cfg(feature = "serde")]
fn is_false(v: &bool) -> bool {
    !*v
}

#[cfg(feature = "serde")]
fn is_linear(scale: &AxisScale) -> bool {
    *scale == AxisScale::Linear
}

#[cfg(feature = "serde")]
fn is_zero_offset(offset: &[f32; 2]) -> bool {
    *offset == [0.0, 0.0]
}

impl ParallelAxis {
    /// A linear axis over `min..max`, not inverted, its labels in place.
    pub fn new(min: f64, max: f64) -> Self {
        Self {
            min,
            max,
            inverted: false,
            scale: AxisScale::Linear,
            label_offset: [0.0, 0.0],
        }
    }

    /// Where `value` sits along the axis: 0 at `min`, 1 at `max` (the other
    /// way round when inverted), linearly or in `log10`, not clamped, so a
    /// value past the range places past the axis's end. `None` for a
    /// non-finite value, or a non-positive one on a logarithmic axis.
    pub fn fraction(&self, value: f64) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        let f = match self.scale {
            AxisScale::Linear => (value - self.min) / (self.max - self.min),
            AxisScale::Logarithmic => {
                if value <= 0.0 {
                    return None;
                }
                (value.log10() - self.min.log10()) / (self.max.log10() - self.min.log10())
            }
            // Refused on a parallel axis (#209).
            AxisScale::Probability(_) => return None,
        };
        f.is_finite()
            .then_some(if self.inverted { 1.0 - f } else { f })
    }

    /// The axis options the tick planner and the label raster read for
    /// this axis: `like`'s look (its label style, tick lengths, line and
    /// title) with this axis's range, scale and direction, no categories,
    /// the tick labels moved by `label_offset` and the major spacing chosen
    /// for the range.
    pub fn axis_options(&self, like: &AxisOptions) -> AxisOptions {
        let mut axis = like.clone();
        axis.min = self.min;
        axis.max = self.max;
        axis.scale = self.scale.clone();
        axis.inverted = self.inverted;
        axis.categories = None;
        axis.major_spacing = crate::tick::axis_major_spacing(&self.scale, self.min, self.max);
        axis.label_style.label_offset_x = self.label_offset[0];
        axis.label_style.label_offset_y = self.label_offset[1];
        axis
    }

    fn validate(&self) -> Result<(), crate::RuleKind> {
        if !self.min.is_finite() || !self.max.is_finite() || !(self.max > self.min) {
            return Err(crate::RuleKind::ParallelAxisRange);
        }
        if self.scale == AxisScale::Logarithmic && self.min <= 0.0 {
            return Err(crate::RuleKind::ParallelLogAxisRange);
        }
        if matches!(self.scale, AxisScale::Probability(_)) {
            return Err(crate::RuleKind::ParallelAxisScale);
        }
        if self.label_offset.iter().any(|v| !v.is_finite()) {
            return Err(crate::RuleKind::ParallelAxisLabelOffset);
        }
        Ok(())
    }
}

/// The parallel coordinate system: one axis per category of `bottom_x`,
/// in order. Later options join it additively.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default = "crate::defaults::parallel_options")
)]
pub struct ParallelOptions {
    pub axes: Vec<ParallelAxis>,
}

impl ParallelOptions {
    /// The axis count every reader of a parallel chart relies on: at least
    /// two. Checked when a chart is set (the renderer's registration), where
    /// the other rules of [`Self::validate`] wait for the draw.
    pub fn validate_axis_count(&self) -> Result<(), crate::RuleKind> {
        if self.axes.len() < 2 {
            return Err(crate::RuleKind::ParallelAxisCount);
        }
        Ok(())
    }

    /// Validate against the axes the coordinate system reads.
    pub fn validate(&self, config: &Config) -> Result<(), crate::RuleKind> {
        self.validate_axis_count()?;
        for axis in &self.axes {
            axis.validate()?;
        }
        let categories = config
            .bottom_x
            .categories
            .as_deref()
            .ok_or(crate::RuleKind::ParallelBottomXCategorical)?;
        if categories.len() != self.axes.len() {
            return Err(crate::RuleKind::ParallelBottomXCategories);
        }
        let y = &config.left_y;
        if y.categories.is_some()
            || y.scale != AxisScale::Linear
            || y.inverted
            || y.min != 0.0
            || y.max != 1.0
        {
            return Err(crate::RuleKind::ParallelLeftY);
        }
        if !config.overlays.is_empty() {
            return Err(crate::RuleKind::ParallelOverlay);
        }
        if config
            .line_offsets
            .as_ref()
            .is_some_and(|offsets| !offsets.series.is_empty())
        {
            return Err(crate::RuleKind::ParallelLineOffsets);
        }
        if !config.draw_style.is_precise() {
            return Err(crate::RuleKind::ParallelDrawStyle);
        }
        if config
            .subplots
            .iter()
            .any(|s| s.link_x.is_some() || s.link_y.is_some())
        {
            return Err(crate::RuleKind::ParallelSubplotLinks);
        }
        Ok(())
    }
}

impl Config {
    /// Validate `parallel` against the axes it reads; `None` always passes.
    pub fn validate_parallel(&self) -> Result<(), crate::RuleKind> {
        match self.parallel() {
            Some(parallel) => parallel.validate(self),
            None => Ok(()),
        }
    }

    /// The rules the series of a chart are held to with the parallel
    /// system: a parallel chart draws only [`DataRenderType::Parallel`]
    /// series, and a parallel series needs a parallel chart. A parallel
    /// series names one column per axis, its `x_column` and `y_column`
    /// being its first two (so every reader of a series' coordinate
    /// columns sees registered columns); its line is solid and linear
    /// without a fill; a colour column maps through `Config.colorbar`.
    pub fn validate_parallel_series(&self, series: &[SeriesConfig]) -> Result<(), crate::RuleKind> {
        for item in series {
            match (&item.render_type, self.parallel()) {
                (DataRenderType::Parallel { .. }, None) => {
                    return Err(crate::RuleKind::ParallelSeriesNotParallel);
                }
                (
                    DataRenderType::Parallel {
                        columns,
                        line,
                        color_column,
                    },
                    Some(parallel),
                ) => {
                    if columns.len() != parallel.axes.len() {
                        return Err(crate::RuleKind::ParallelSeriesColumnCount);
                    }
                    let [first, second, ..] = columns.as_slice() else {
                        return Err(crate::RuleKind::ParallelSeriesTooFewColumns);
                    };
                    if columns.iter().any(String::is_empty) {
                        return Err(crate::RuleKind::ParallelSeriesColumnNames);
                    }
                    if *first != item.x_column || *second != item.y_column {
                        return Err(crate::RuleKind::ParallelSeriesXY);
                    }
                    if line.interpolation != LineInterpolation::Linear {
                        return Err(crate::RuleKind::ParallelSeriesStepLine);
                    }
                    if line.fill.is_some() {
                        return Err(crate::RuleKind::ParallelSeriesLineFill);
                    }
                    if line.line_style != LineStylePreset::Solid {
                        return Err(crate::RuleKind::ParallelSeriesDashed);
                    }
                    if color_column.as_deref().is_some_and(str::is_empty) {
                        return Err(crate::RuleKind::ParallelSeriesColorColumn);
                    }
                    if color_column.is_some() && self.colorbar.is_none() {
                        return Err(crate::RuleKind::ParallelSeriesColorbar);
                    }
                }
                (_, Some(_)) => {
                    return Err(crate::RuleKind::ParallelChartSeriesKind);
                }
                (_, None) => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;
    use crate::config::Category;
    use crate::data_config::DataLineStyleConfig;

    fn line() -> DataLineStyleConfig {
        DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 1.0,
            fill: None,
            fill_color: None,
            interpolation: LineInterpolation::Linear,
        }
    }

    fn series(columns: &[&str]) -> SeriesConfig {
        SeriesConfig {
            series_id: "rows".into(),
            source_id: None,
            label: None,
            x_column: columns[0].into(),
            y_column: columns[1].into(),
            z_column: None,
            render_type: DataRenderType::Parallel {
                columns: columns.iter().map(|c| c.to_string()).collect(),
                line: line(),
                color_column: None,
            },
        }
    }

    fn chart() -> Config {
        let mut config = crate::defaults::config();
        config.bottom_x.categories = Some(
            ["a", "b", "c"]
                .map(|id| Category::new(id, id.to_uppercase()))
                .to_vec(),
        );
        config.left_y.min = 0.0;
        config.left_y.max = 1.0;
        config.set_parallel(Some(ParallelOptions {
            axes: vec![
                ParallelAxis::new(0.0, 10.0),
                ParallelAxis {
                    inverted: true,
                    ..ParallelAxis::new(-1.0, 1.0)
                },
                ParallelAxis {
                    scale: AxisScale::Logarithmic,
                    ..ParallelAxis::new(1.0, 1000.0)
                },
            ],
        }));
        config
    }

    #[test]
    fn fractions_place_values_linearly_inverted_and_logarithmically() {
        let axes = chart().parallel().unwrap().axes.clone();
        assert_eq!(axes[0].fraction(2.5), Some(0.25));
        assert_eq!(
            axes[0].fraction(12.0),
            Some(1.2),
            "past the range, not clamped"
        );
        assert_eq!(axes[1].fraction(0.5), Some(0.25), "inverted");
        assert_eq!(axes[2].fraction(10.0), Some(1.0 / 3.0));
        assert_eq!(axes[2].fraction(0.0), None, "no log of zero");
        assert_eq!(axes[0].fraction(f64::NAN), None);
        let twin = axes[2].axis_options(&chart().left_y);
        assert_eq!((twin.min, twin.max), (1.0, 1000.0));
        assert_eq!(twin.scale, AxisScale::Logarithmic);
        assert!(twin.categories.is_none());
    }

    #[test]
    fn the_rules_name_each_rejection() {
        let config = chart();
        assert_eq!(config.validate_parallel(), Ok(()));
        assert_eq!(
            config.validate_parallel_series(&[series(&["a", "b", "c"])]),
            Ok(())
        );
        let reject = |edit: &dyn Fn(&mut Config)| {
            let mut c = chart();
            edit(&mut c);
            c.validate_parallel().unwrap_err()
        };
        assert_eq!(
            reject(&|c| c.parallel_mut().unwrap().axes.truncate(1)),
            crate::RuleKind::ParallelAxisCount
        );
        assert_eq!(
            reject(&|c| c.parallel_mut().unwrap().axes[0].max = 0.0),
            crate::RuleKind::ParallelAxisRange
        );
        assert_eq!(
            reject(&|c| c.parallel_mut().unwrap().axes[2].min = 0.0),
            crate::RuleKind::ParallelLogAxisRange
        );
        assert_eq!(
            reject(&|c| {
                let axis = &mut c.parallel_mut().unwrap().axes[0];
                axis.scale = AxisScale::Probability(crate::config::ProbabilityScale::Normal);
                axis.min = 0.1;
                axis.max = 0.9;
            }),
            crate::RuleKind::ParallelAxisScale
        );
        assert_eq!(
            reject(&|c| c.bottom_x.categories = None),
            crate::RuleKind::ParallelBottomXCategorical
        );
        assert_eq!(
            reject(&|c| c
                .bottom_x
                .categories
                .as_mut()
                .unwrap()
                .pop()
                .map(|_| ())
                .unwrap()),
            crate::RuleKind::ParallelBottomXCategories
        );
        assert_eq!(
            reject(&|c| c.left_y.max = 2.0),
            crate::RuleKind::ParallelLeftY
        );
        assert_eq!(
            reject(&|c| c.overlays.push(crate::defaults::overlay_layer())),
            crate::RuleKind::ParallelOverlay
        );
        let reject_series = |edit: &dyn Fn(&mut SeriesConfig)| {
            let mut s = series(&["a", "b", "c"]);
            edit(&mut s);
            config.validate_parallel_series(&[s]).unwrap_err()
        };
        assert_eq!(
            reject_series(&|s| {
                if let DataRenderType::Parallel { columns, .. } = &mut s.render_type {
                    columns.pop();
                }
            }),
            crate::RuleKind::ParallelSeriesColumnCount
        );
        assert_eq!(
            reject_series(&|s| s.y_column = "c".into()),
            crate::RuleKind::ParallelSeriesXY
        );
        assert_eq!(
            reject_series(&|s| {
                if let DataRenderType::Parallel { line, .. } = &mut s.render_type {
                    line.line_style = LineStylePreset::Dash;
                }
            }),
            crate::RuleKind::ParallelSeriesDashed
        );
        assert_eq!(
            reject_series(&|s| {
                if let DataRenderType::Parallel { color_column, .. } = &mut s.render_type {
                    *color_column = Some("class".into());
                }
            }),
            crate::RuleKind::ParallelSeriesColorbar
        );
        let mut coloured = chart();
        coloured.colorbar = Some(crate::defaults::color_bar_options());
        let mut s = series(&["a", "b", "c"]);
        if let DataRenderType::Parallel { color_column, .. } = &mut s.render_type {
            *color_column = Some("class".into());
        }
        assert_eq!(coloured.validate_parallel_series(&[s]), Ok(()));
        // The other way round: a plain chart refuses a parallel series, a
        // parallel chart refuses a plain one.
        let plain = crate::defaults::config();
        assert_eq!(
            plain.validate_parallel_series(&[series(&["a", "b", "c"])]),
            Err(crate::RuleKind::ParallelSeriesNotParallel)
        );
        let mut scatter = series(&["a", "b", "c"]);
        scatter.render_type = DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(line()),
            errors: None,
            stem: None,
        });
        assert_eq!(
            config.validate_parallel_series(&[scatter]),
            Err(crate::RuleKind::ParallelChartSeriesKind)
        );
    }

    /// A parallel chart with fewer than two axes is malformed, and a series
    /// checked against it directly (without `validate_parallel` first) is
    /// refused with an error rather than indexing its first two columns
    /// (#152).
    #[test]
    fn a_series_on_fewer_than_two_axes_is_refused_without_panicking() {
        for count in [0usize, 1] {
            let mut config = chart();
            config.parallel_mut().unwrap().axes.truncate(count);
            assert_eq!(
                config.validate_parallel(),
                Err(crate::RuleKind::ParallelAxisCount)
            );
            let columns: Vec<String> = ["a"].iter().take(count).map(|c| c.to_string()).collect();
            let item = SeriesConfig {
                series_id: "rows".into(),
                source_id: None,
                label: None,
                x_column: "a".into(),
                y_column: "b".into(),
                z_column: None,
                render_type: DataRenderType::Parallel {
                    columns,
                    line: line(),
                    color_column: None,
                },
            };
            assert_eq!(
                config.validate_parallel_series(&[item]),
                Err(crate::RuleKind::ParallelSeriesTooFewColumns),
                "{count} axes"
            );
        }
    }

    #[cfg(feature = "serde")]
    // The label gap is a layout constant, so a scaled chart moves each
    // axis's labels outward by the extra gap through `label_offset`, as the
    // Cartesian axes' `scale_label_gap` does.
    #[test]
    fn scaling_keeps_each_axis_label_gap_proportional() {
        use crate::layout::LABEL_GAP;
        let mut config = chart();
        config.parallel_mut().unwrap().axes[1].label_offset = [3.0, 5.0];
        let scaled = config.scaled(2.0);
        let axes = &scaled.parallel().unwrap().axes;
        assert_eq!(axes[0].label_offset, [-LABEL_GAP, 0.0]);
        assert_eq!(axes[1].label_offset, [6.0 - LABEL_GAP, 10.0]);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn parallel_options_round_trip_with_defaults_omitted() {
        let config = chart();
        let json = serde_json::to_value(&config).unwrap();
        let axes = &json["parallel"]["axes"];
        assert!(axes[0].get("inverted").is_none(), "{axes}");
        assert!(axes[0].get("scale").is_none());
        assert!(axes[0].get("label_offset").is_none());
        assert_eq!(axes[1]["inverted"], true);
        assert_eq!(axes[2]["scale"], "Logarithmic");
        let back: Config = serde_json::from_value(json).unwrap();
        assert_eq!(back, config);
        let plain = serde_json::to_value(crate::defaults::config()).unwrap();
        assert!(plain.get("parallel").is_none());
        let series_json = serde_json::to_value(series(&["a", "b", "c"])).unwrap();
        assert!(
            series_json["render_type"]["Parallel"]
                .get("color_column")
                .is_none()
        );
        let back: SeriesConfig = serde_json::from_value(series_json).unwrap();
        assert_eq!(back, series(&["a", "b", "c"]));
    }
}

//! Parallel coordinates for the Cartesian chart: `Config.parallel` (#141).
//!
//! With `parallel: Some(..)` the chart draws one vertical axis per category
//! of `bottom_x`, in order, each with its own range
//! ([`ParallelAxis`]), and every [`DataRenderType::Parallel`] series draws
//! row `r` of its columns as the polyline through
//! `(axis i, (v_i − min_i) / (max_i − min_i))` for every axis: the
//! categorical `bottom_x` places the axes and names them, `left_y` spans
//! the normalized band `0..1` the values are placed on. Normalizing a
//! value only to place it on its axis is appearance (`CLAUDE.md` rule 1,
//! as the ternary normalization), computed in the vertex stage on every
//! draw and never stored; hover, selection and the reported values are the
//! original columns. A row with a non-finite value breaks at that axis.
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
        serde(default = "linear_scale", skip_serializing_if = "is_linear")
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
fn linear_scale() -> AxisScale {
    AxisScale::Linear
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

    fn validate(&self) -> Result<(), &'static str> {
        if !self.min.is_finite() || !self.max.is_finite() || !(self.max > self.min) {
            return Err("a parallel axis's range is finite with a smaller min");
        }
        if self.scale == AxisScale::Logarithmic && self.min <= 0.0 {
            return Err("a logarithmic parallel axis's range is positive");
        }
        if self.label_offset.iter().any(|v| !v.is_finite()) {
            return Err("a parallel axis's label offset is finite");
        }
        Ok(())
    }
}

/// The parallel coordinate system: one axis per category of `bottom_x`,
/// in order. Later options join it additively.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ParallelOptions {
    pub axes: Vec<ParallelAxis>,
}

impl ParallelOptions {
    /// Validate against the axes the coordinate system reads.
    pub fn validate(&self, config: &Config) -> Result<(), &'static str> {
        if self.axes.len() < 2 {
            return Err("a parallel chart has at least two axes");
        }
        for axis in &self.axes {
            axis.validate()?;
        }
        let categories = config
            .bottom_x
            .categories
            .as_deref()
            .ok_or("a parallel chart's bottom_x is categorical: one category per axis")?;
        if categories.len() != self.axes.len() {
            return Err("a parallel chart's bottom_x has one category per axis");
        }
        let y = &config.left_y;
        if y.categories.is_some()
            || y.scale != AxisScale::Linear
            || y.inverted
            || y.min != 0.0
            || y.max != 1.0
        {
            return Err(
                "a parallel chart's left_y spans the normalized band: linear 0..1, not inverted",
            );
        }
        if config.polar.is_some() || config.ternary.is_some() || config.view_3d.is_some() {
            return Err("a chart is parallel, polar, ternary or 3D: one of them");
        }
        if !config.overlays.is_empty() {
            return Err("parallel charts have no overlay layers");
        }
        if config
            .line_offsets
            .as_ref()
            .is_some_and(|offsets| !offsets.series.is_empty())
        {
            return Err("parallel charts have no line offsets");
        }
        if !config.draw_style.is_precise() {
            return Err("parallel charts are drawn in the Precise style only");
        }
        if config
            .subplots
            .iter()
            .any(|s| s.link_x.is_some() || s.link_y.is_some())
        {
            return Err("parallel charts take no subplot axis links");
        }
        Ok(())
    }
}

impl Config {
    /// Validate `parallel` against the axes it reads; `None` always passes.
    pub fn validate_parallel(&self) -> Result<(), &'static str> {
        match &self.parallel {
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
    pub fn validate_parallel_series(&self, series: &[SeriesConfig]) -> Result<(), &'static str> {
        for item in series {
            match (&item.render_type, &self.parallel) {
                (DataRenderType::Parallel { .. }, None) => {
                    return Err("a parallel series is drawn on a parallel chart (Config.parallel)");
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
                        return Err("a parallel series names one column per axis");
                    }
                    if columns.iter().any(String::is_empty) {
                        return Err("a parallel series' axis columns are named");
                    }
                    if columns[0] != item.x_column || columns[1] != item.y_column {
                        return Err(
                            "a parallel series' x_column and y_column are its first two axis \
                             columns",
                        );
                    }
                    if line.interpolation != LineInterpolation::Linear {
                        return Err("a parallel series' line is linear: no step interpolation");
                    }
                    if line.fill.is_some() {
                        return Err("a parallel series' line has no area fill");
                    }
                    if line.line_style != LineStylePreset::Solid {
                        return Err("a parallel series' line is solid");
                    }
                    if color_column.as_deref().is_some_and(str::is_empty) {
                        return Err("a parallel series' colour column is named");
                    }
                    if color_column.is_some() && self.colorbar.is_none() {
                        return Err("a parallel series coloured by a column needs Config.colorbar");
                    }
                }
                (_, Some(_)) => {
                    return Err("a parallel chart draws only parallel series");
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
        let mut config = crate::default::default_config();
        config.bottom_x.categories = Some(
            ["a", "b", "c"]
                .map(|id| Category::new(id, id.to_uppercase()))
                .to_vec(),
        );
        config.left_y.min = 0.0;
        config.left_y.max = 1.0;
        config.parallel = Some(ParallelOptions {
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
        });
        config
    }

    #[test]
    fn fractions_place_values_linearly_inverted_and_logarithmically() {
        let axes = chart().parallel.unwrap().axes;
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
            reject(&|c| c.parallel.as_mut().unwrap().axes.truncate(1)),
            "a parallel chart has at least two axes"
        );
        assert_eq!(
            reject(&|c| c.parallel.as_mut().unwrap().axes[0].max = 0.0),
            "a parallel axis's range is finite with a smaller min"
        );
        assert_eq!(
            reject(&|c| c.parallel.as_mut().unwrap().axes[2].min = 0.0),
            "a logarithmic parallel axis's range is positive"
        );
        assert_eq!(
            reject(&|c| c.bottom_x.categories = None),
            "a parallel chart's bottom_x is categorical: one category per axis"
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
            "a parallel chart's bottom_x has one category per axis"
        );
        assert_eq!(
            reject(&|c| c.left_y.max = 2.0),
            "a parallel chart's left_y spans the normalized band: linear 0..1, not inverted"
        );
        assert_eq!(
            reject(&|c| c.polar = Some(Default::default())),
            "a chart is parallel, polar, ternary or 3D: one of them"
        );
        assert_eq!(
            reject(&|c| c.view_3d = Some(Default::default())),
            "a chart is parallel, polar, ternary or 3D: one of them"
        );
        assert_eq!(
            reject(&|c| c.overlays.push(Default::default())),
            "parallel charts have no overlay layers"
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
            "a parallel series names one column per axis"
        );
        assert_eq!(
            reject_series(&|s| s.y_column = "c".into()),
            "a parallel series' x_column and y_column are its first two axis columns"
        );
        assert_eq!(
            reject_series(&|s| {
                if let DataRenderType::Parallel { line, .. } = &mut s.render_type {
                    line.line_style = LineStylePreset::Dash;
                }
            }),
            "a parallel series' line is solid"
        );
        assert_eq!(
            reject_series(&|s| {
                if let DataRenderType::Parallel { color_column, .. } = &mut s.render_type {
                    *color_column = Some("class".into());
                }
            }),
            "a parallel series coloured by a column needs Config.colorbar"
        );
        let mut coloured = chart();
        coloured.colorbar = Some(crate::default::default_colorbar_options());
        let mut s = series(&["a", "b", "c"]);
        if let DataRenderType::Parallel { color_column, .. } = &mut s.render_type {
            *color_column = Some("class".into());
        }
        assert_eq!(coloured.validate_parallel_series(&[s]), Ok(()));
        // The other way round: a plain chart refuses a parallel series, a
        // parallel chart refuses a plain one.
        let plain = crate::default::default_config();
        assert_eq!(
            plain.validate_parallel_series(&[series(&["a", "b", "c"])]),
            Err("a parallel series is drawn on a parallel chart (Config.parallel)")
        );
        let mut scatter = series(&["a", "b", "c"]);
        scatter.render_type = DataRenderType::Line { line: line() };
        assert_eq!(
            config.validate_parallel_series(&[scatter]),
            Err("a parallel chart draws only parallel series")
        );
    }

    #[cfg(feature = "serde")]
    // The label gap is a layout constant, so a scaled chart moves each
    // axis's labels outward by the extra gap through `label_offset`, as the
    // Cartesian axes' `scale_label_gap` does.
    #[test]
    fn scaling_keeps_each_axis_label_gap_proportional() {
        use crate::layout::LABEL_GAP;
        let mut config = chart();
        config.parallel.as_mut().unwrap().axes[1].label_offset = [3.0, 5.0];
        let scaled = config.scaled(2.0);
        let axes = &scaled.parallel.as_ref().unwrap().axes;
        assert_eq!(axes[0].label_offset, [-LABEL_GAP, 0.0]);
        assert_eq!(axes[1].label_offset, [6.0 - LABEL_GAP, 10.0]);
    }

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
        let plain = serde_json::to_value(crate::default::default_config()).unwrap();
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

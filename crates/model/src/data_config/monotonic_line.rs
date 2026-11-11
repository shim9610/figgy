//! The monotonic-x line series (#271): one channel of a monotonic group,
//! drawn as a line through the group's M4 envelope.

use super::*;

/// One channel of a monotonic group (#271), drawn as a line.
///
/// A monotonic group is evenly spaced data, `x = start + i * interval`,
/// that the host lends to the renderer through a
/// [`MonotonicSource`](crate::monotonic::MonotonicSource) on every call
/// (`set_monotonic`, `append_monotonic`); the document never holds it. The
/// renderer reduces each pixel column to its M4 points (first, lowest,
/// highest, last) on the CPU and keeps only that envelope, sized by the plot
/// width, never by the sample count. See `MONOTONIC.md` in this crate.
///
/// The series names its group and the channel of the group it draws. Its
/// `x_column` and `y_column` are the ids of the two pool columns the
/// renderer keeps the drawn envelope in, written by every call and read by
/// the line pipeline like any line's columns. They are renderer-owned: the
/// host never uploads them, and one pair belongs to one series of one chart
/// (the helper `SeriesConfig::monotonic_line` names them after the series
/// id, [`SeriesConfig::monotonic_column_ids`]).
///
/// The line is linear and has no fill: `line.interpolation` must be
/// `Linear` and `line.fill` `None`. The chart's x axis must be linear,
/// numeric and unbroken (#271); the series is refused on polar, ternary,
/// parallel and 3D charts.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MonotonicLineSeries {
    /// The group the series reads: the name its calls supply the source
    /// under.
    pub group: String,
    /// The channel of the group's source this series draws, `0`-based.
    pub channel: usize,
    /// The line the envelope is drawn with.
    pub line: DataLineStyleConfig,
}

impl MonotonicLineSeries {
    /// The rules one monotonic series is held to on its own: a named group,
    /// a linear line without a fill, and two distinct, named envelope
    /// columns. The chart rules (the x axis, the chart kind) are checked
    /// with the chart ([`crate::config::Config::validate_monotonic_series`]).
    pub fn validate(&self, series: &SeriesConfig) -> Result<(), crate::RuleKind> {
        if self.group.is_empty() {
            return Err(crate::RuleKind::MonotonicGroupEmpty);
        }
        if !self.line.interpolation.is_linear() {
            return Err(crate::RuleKind::MonotonicInterpolation);
        }
        if self.line.fill.is_some() {
            return Err(crate::RuleKind::MonotonicFill);
        }
        if series.x_column.is_empty()
            || series.y_column.is_empty()
            || series.x_column == series.y_column
        {
            return Err(crate::RuleKind::MonotonicColumns);
        }
        Ok(())
    }
}

impl DataRenderType {
    /// The monotonic line series' fields (#271), `None` for every other
    /// kind.
    pub fn monotonic_line(&self) -> Option<&MonotonicLineSeries> {
        match self {
            DataRenderType::MonotonicLine(series) => Some(series),
            _ => None,
        }
    }
}

impl SeriesConfig {
    /// The monotonic line series' fields (#271), `None` for every other
    /// kind.
    pub fn monotonic(&self) -> Option<&MonotonicLineSeries> {
        self.render_type.monotonic_line()
    }

    /// The pool column ids the helper `SeriesConfig::monotonic_line` gives
    /// the envelope of series `series_id`: `<series_id>.monotonic.x` and
    /// `<series_id>.monotonic.y`.
    pub fn monotonic_column_ids(series_id: &str) -> (String, String) {
        (
            format!("{series_id}.monotonic.x"),
            format!("{series_id}.monotonic.y"),
        )
    }
}

impl crate::config::Config {
    /// The chart rules of monotonic series (#271): each series' own rules
    /// ([`MonotonicLineSeries::validate`]), envelope columns no other
    /// series of the chart reads, and an x axis the source can be drawn on:
    /// linear, numeric (not categorical) and unbroken, on a rectangular 2D
    /// chart (not ternary, parallel or 3D; polar is refused by
    /// [`Self::validate_series_on_polar`]). A subplot's series is checked
    /// against the subplot's x axis.
    pub fn validate_monotonic_series(
        &self,
        series: &[SeriesConfig],
    ) -> Result<(), crate::RuleKind> {
        for (index, item) in series.iter().enumerate() {
            let Some(monotonic) = item.monotonic() else {
                continue;
            };
            monotonic.validate(item)?;
            let shared = series.iter().enumerate().any(|(other, s)| {
                let mut reads = false;
                if other != index {
                    s.visit_columns(&mut |id| {
                        reads |= id == item.x_column || id == item.y_column;
                    });
                }
                reads
            });
            if shared {
                return Err(crate::RuleKind::MonotonicColumnsShared);
            }
            if self.ternary().is_some() || self.parallel().is_some() || self.view_3d().is_some() {
                return Err(crate::RuleKind::MonotonicChartKind);
            }
            let x_axis = match self.layer_of_series(&item.series_id) {
                crate::layer::LayerRef::Subplot(i)
                | crate::layer::LayerRef::SubplotOverlay(i, _) => {
                    self.subplots.get(i).map(|subplot| &subplot.config.bottom_x)
                }
                _ => None,
            }
            .unwrap_or(&self.bottom_x);
            if x_axis.scale != crate::config::AxisScale::Linear
                || x_axis.categories.is_some()
                || x_axis
                    .breaks
                    .as_ref()
                    .is_some_and(|breaks| !breaks.ranges.is_empty())
            {
                return Err(crate::RuleKind::MonotonicXAxis);
            }
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "helpers"))]
mod tests {
    use super::*;
    use crate::RuleKind;

    fn series() -> SeriesConfig {
        SeriesConfig::monotonic_line("a", "daq", 1)
    }

    fn with(edit: impl FnOnce(&mut MonotonicLineSeries)) -> SeriesConfig {
        let mut item = series();
        if let DataRenderType::MonotonicLine(monotonic) = &mut item.render_type {
            edit(monotonic);
        }
        item
    }

    #[test]
    fn the_helper_names_the_group_channel_and_envelope_columns() {
        let item = series();
        let monotonic = item.monotonic().expect("monotonic");
        assert_eq!((monotonic.group.as_str(), monotonic.channel), ("daq", 1));
        assert_eq!(
            (item.x_column.to_string(), item.y_column.to_string()),
            SeriesConfig::monotonic_column_ids("a")
        );
        assert_eq!(monotonic.validate(&item), Ok(()));
        assert!(SeriesConfig::line("b", "x", "y").monotonic().is_none());
    }

    #[test]
    fn a_series_is_refused_without_a_group_a_linear_line_or_its_columns() {
        let empty = with(|m| m.group.clear());
        let step = with(|m| m.line.interpolation = LineInterpolation::StepPre);
        let fill = with(|m| m.line.fill = Some(LineFill::ToBaseline(0.0)));
        let check = |item: &SeriesConfig| item.monotonic().expect("monotonic").validate(item);
        assert_eq!(check(&empty), Err(RuleKind::MonotonicGroupEmpty));
        assert_eq!(check(&step), Err(RuleKind::MonotonicInterpolation));
        assert_eq!(check(&fill), Err(RuleKind::MonotonicFill));
        let mut same = series();
        same.y_column = same.x_column.clone();
        assert_eq!(check(&same), Err(RuleKind::MonotonicColumns));
    }

    #[test]
    fn the_chart_refuses_shared_columns_and_an_unsupported_x_axis() {
        let config = crate::default::default_config();
        assert_eq!(config.validate_monotonic_series(&[series()]), Ok(()));
        let item = series();
        let reader = SeriesConfig::line("b", item.x_column.clone(), "y");
        assert_eq!(
            config.validate_monotonic_series(&[item, reader]),
            Err(RuleKind::MonotonicColumnsShared)
        );
        let mut log = config.clone();
        log.bottom_x.scale = crate::config::AxisScale::Logarithmic;
        assert_eq!(
            log.validate_monotonic_series(&[series()]),
            Err(RuleKind::MonotonicXAxis)
        );
        let error = log.validate_series(&[series()]).expect_err("log x");
        assert_eq!(error.kind, RuleKind::MonotonicXAxis);
        assert_eq!(error.field, "monotonic");
    }

    /// The kind round-trips byte-identically, and a document without a
    /// monotonic series serializes as before (the variant is additive).
    #[cfg(feature = "serde")]
    #[test]
    fn the_series_round_trips_byte_identically() {
        let item = series().width(2.0);
        let json = serde_json::to_string(&item).expect("serialize");
        assert!(json.contains("MonotonicLine"), "{json}");
        assert!(json.contains("\"group\":\"daq\""), "{json}");
        let back: SeriesConfig = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, item);
        assert_eq!(serde_json::to_string(&back).expect("serialize"), json);
    }
}

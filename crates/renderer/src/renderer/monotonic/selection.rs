//! Source needs, kept points and `step_chart_selection` over the kept points.

use super::*;

impl Renderer {
    /// The monotonic series of chart `chart` that need their group's source
    /// before they can be drawn: series whose group was never supplied
    /// ([`MappingChange::NotBuilt`]), which draw nothing until its first
    /// full call. Every call that moves the x-to-pixel mapping takes the
    /// source or is refused, so no other series is listed; a frame and a
    /// pick never fail for a missing source.
    pub(in crate::renderer) fn needs_source(
        &self,
        chart: ChartId,
    ) -> Result<Vec<MonotonicSourceNeed>> {
        self.health.check()?;
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let mut needs = Vec::new();
        if !state.series.iter().any(|s| s.monotonic().is_some()) {
            return Ok(needs);
        }
        let (config, _, _) = self.chart_display_config(chart)?;
        let panels = DocumentPanels::new(&config)?;
        for cfg in &state.series {
            let Some(monotonic) = cfg.monotonic() else {
                continue;
            };
            if let Err(FiggyError::MonotonicSourceRequired { change, .. }) =
                self.monotonic.check_series(panels.of(&cfg.series_id), cfg)
            {
                needs.push(MonotonicSourceNeed {
                    series_id: cfg.series_id.clone(),
                    group: monotonic.group.clone(),
                    change,
                });
            }
        }
        Ok(needs)
    }

    /// The kept point of monotonic series `series_id` of chart `chart` with
    /// logical sample index `index`, as `(x, y)` in data units: what a pick
    /// on that series reported, read from the envelope without the source.
    /// `None` when the envelope does not keep that sample.
    pub(in crate::renderer) fn kept_point_of(
        &self,
        chart: ChartId,
        series_id: &str,
        index: usize,
    ) -> Result<Option<(f64, f64)>> {
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        Ok(state
            .series
            .iter()
            .find(|s| s.series_id == series_id && s.monotonic().is_some())
            .and_then(|cfg| self.monotonic.kept_point(cfg, index)))
    }

    /// Move chart `chart`'s first typed data selection by `dx` along its
    /// series and `dy` across series, and return whether it moved
    /// ([`Config::step_selection`]). A point of a monotonic series (#271)
    /// steps over the points its envelope keeps, in x order; `dy` moves it
    /// to the previous or next monotonic series of the chart, at the kept
    /// point nearest its index. Every other selection steps as
    /// `Config::step_selection` steps it, over the registered column
    /// lengths.
    pub fn step_chart_selection(&mut self, chart: ChartId, dx: i32, dy: i32) -> Result<bool> {
        let result = self.step_chart_selection_unreported(chart, dx, dy);
        self.reported(result)
    }

    fn step_chart_selection_unreported(
        &mut self,
        chart: ChartId,
        dx: i32,
        dy: i32,
    ) -> Result<bool> {
        self.health.check()?;
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let mut config = state.config.clone();
        let selected = config
            .picked_data
            .as_ref()
            .and_then(|s| s.refs.first())
            .cloned();
        let moved = match selected {
            Some(PickedDataRef::Point {
                series_id,
                point_index,
                ..
            }) if state
                .series
                .iter()
                .any(|s| s.series_id == series_id && s.monotonic().is_some()) =>
            {
                let Some(next) =
                    self.stepped_monotonic(&state.series, &series_id, point_index, dx, dy)
                else {
                    return Ok(false);
                };
                let Some(current) = config.picked_data.as_mut().and_then(|s| s.refs.first_mut())
                else {
                    return Ok(false);
                };
                if *current == next {
                    return Ok(false);
                }
                *current = next;
                true
            }
            _ => {
                let pool = &self.pool;
                let sources = &self.streaming_sources;
                config
                    .step_selection(&state.series, dx, dy, |column| {
                        pool.slot(column)
                            .map(|slot| slot.len_values)
                            .or_else(|| {
                                sources
                                    .get(column)
                                    .and_then(|source| usize::try_from(source.len).ok())
                            })
                            .or_else(|| {
                                pool.triangles(column)
                                    .map(|triangles| triangles.stats().triangles as usize * 3)
                            })
                    })
                    .map_err(|kind| FiggyError::InvalidConfig {
                        field: "picked_data",
                        kind,
                    })?
            }
        };
        if moved {
            self.set_chart_config_unreported(chart, config)?;
        }
        Ok(moved)
    }

    /// The selection a monotonic point steps to.
    fn stepped_monotonic(
        &self,
        series: &[SeriesConfig],
        series_id: &str,
        point_index: usize,
        dx: i32,
        dy: i32,
    ) -> Option<PickedDataRef> {
        let members = || series.iter().filter(|s| s.monotonic().is_some());
        let position = members().position(|s| s.series_id == series_id)?;
        let target = position
            .saturating_add_signed(dy as isize)
            .min(members().count().saturating_sub(1));
        let next = members().nth(target)?;
        let (envelope, channel) = self.monotonic.envelope_of(next)?;
        // The kept point at or nearest the current index, then `dx` kept
        // points on.
        let kept = || kept_indices(envelope, channel);
        let count = kept().count();
        let here = kept()
            .enumerate()
            .min_by_key(|(_, index)| index.abs_diff(point_index))
            .map(|(k, _)| k)?;
        let step = if next.series_id == series_id { dx } else { 0 };
        let at = here
            .saturating_add_signed(step as isize)
            .min(count.saturating_sub(1));
        Some(PickedDataRef::Point {
            series_id: next.series_id.clone(),
            source_id: next.source_id.clone(),
            point_index: kept().nth(at)?,
        })
    }

    /// The monotonic series of chart `chart` that name group `group`.
    pub(super) fn monotonic_members(&self, chart: ChartId, group: &str) -> Result<Vec<Member>> {
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let mut members = Vec::new();
        for cfg in &state.series {
            let Some(monotonic) = cfg.monotonic().filter(|m| m.group == group) else {
                continue;
            };
            members.push(Member {
                series_id: cfg.series_id.clone(),
                x_column: cfg.x_column.clone(),
                y_column: cfg.y_column.clone(),
                channel: monotonic.channel,
            });
        }
        if members.is_empty() {
            return Err(FiggyError::UnknownMonotonicGroup {
                group: group.to_owned(),
            });
        }
        Ok(members)
    }
}

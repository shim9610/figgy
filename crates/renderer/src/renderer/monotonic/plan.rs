//! Planning, applying and writing a reduction, and the append.

use super::*;

impl Renderer {
    /// The full call with the caller's typed source behind `build`: the x
    /// range `x` goes through the shared step with the group's source, which
    /// reduces the group at the new mapping; with `autoscale` the y range is
    /// then fitted to what that reduction kept, a change that moves no
    /// mapping.
    pub(super) fn set_monotonic_unreported(
        &mut self,
        chart: ChartId,
        group: &str,
        (x_min, x_max): (f64, f64),
        autoscale: bool,
        build: &mut FullCall<'_>,
    ) -> Result<MonotonicUpdate> {
        self.health.check()?;
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let members: Vec<(String, crate::LayerRef)> = state
            .series
            .iter()
            .filter(|cfg| cfg.monotonic().is_some_and(|m| m.group == group))
            .map(|cfg| {
                (
                    cfg.series_id.clone(),
                    state.config.layer_of_series(&cfg.series_id),
                )
            })
            .collect();
        if members.is_empty() {
            return Err(FiggyError::UnknownMonotonicGroup {
                group: group.to_owned(),
            });
        }
        // An autoscaled range of one sample has no width: the axis stays.
        let fixed = !autoscale;
        if fixed && !(x_min.is_finite() && x_max.is_finite() && x_min < x_max) {
            return Err(FiggyError::InvalidConfig {
                field: "bottom_x",
                kind: crate::RuleKind::MonotonicRange,
            });
        }
        let x_extent = crate::chart::FitExtent {
            min: x_min,
            max: x_max,
            min_positive: (x_min > 0.0).then_some(x_min),
        };
        let mut config = state.config.clone();
        for (series_id, layer) in &members {
            let own_x = config
                .overlay_of_layer(*layer)
                .is_some_and(|overlay| !overlay.link_x);
            if autoscale && own_x {
                // An overlay's own x axis, fitted alone (it has no partner).
                if let Some(axis) = x_axis_mut(&mut config, series_id) {
                    crate::chart::apply_axis_fit(axis, &x_extent, 0.0);
                }
            } else if autoscale {
                // Fitted as the general auto-fit fits an axis, its tick
                // plan with it.
                if let Some(panel) = panel_config_mut(&mut config, *layer) {
                    crate::chart::apply_auto_fit_all(
                        panel,
                        &x_extent,
                        &crate::chart::FitExtent::EMPTY,
                        0.0,
                    );
                }
            } else if let Some(axis) = x_axis_mut(&mut config, series_id) {
                // The host's range, as a slide writes it: the rest of the
                // axis stays.
                axis.min = x_min;
                axis.max = x_max;
            }
        }
        config.sync_subplot_links(None);
        let mut update = None;
        let mut lend =
            |name: &str, envelope: &mut MonotonicEnvelope, mapping: &MonotonicMapping| {
                (name == group).then(|| {
                    let built = build(envelope, mapping);
                    if let Ok(built) = &built {
                        update.get_or_insert(*built);
                    }
                    built
                })
            };
        self.set_chart_config_lent(chart, config, &mut lend)?;
        let update = update.ok_or(FiggyError::internal("a monotonic group has a member"))?;
        if !autoscale {
            return Ok(update);
        }
        // The y range: what the reduction kept, per axis.
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let mut config = state.config.clone();
        // One fit per y axis the group draws on, over every monotonic series
        // drawn on it, this group's and any other group's: fitting member by
        // member would leave the axis at the last member's range, and an
        // axis two groups share (an overlay that follows its chart's y axis)
        // must keep both in view. Another group's envelope is at its
        // current mapping: a call that moved it was refused above.
        let axis_of = |layer: &crate::LayerRef| y_axis_layer(&state.config, *layer);
        for (index, (_, layer)) in members.iter().enumerate() {
            let axis = axis_of(layer);
            if members[..index]
                .iter()
                .any(|(_, seen)| axis_of(seen) == axis)
            {
                continue;
            }
            let mut extent = crate::chart::FitExtent::EMPTY;
            for cfg in state.series.iter().filter(|cfg| {
                cfg.monotonic().is_some()
                    && axis_of(&state.config.layer_of_series(&cfg.series_id)) == axis
            }) {
                if let Some((envelope, channel)) = self.monotonic.envelope_of(cfg) {
                    extent.union(&plot_extent(envelope, channel));
                }
            }
            fit_y_axis(&mut config, axis, &extent);
        }
        self.set_chart_config_unreported(chart, config)?;
        Ok(update)
    }

    /// The shared step of every call that changes a registered chart
    /// (#271): plans the monotonic series of the document the call leaves
    /// (`config` and `series`, or the stored ones), placed on the display as
    /// `placement` places it, and writes their envelopes. Each series keeps
    /// the envelope of its group at its mapping, or one built from the
    /// source the call lends (`lend`); a series whose group has data but no
    /// envelope at its new mapping, with no source lent, refuses the call
    /// with [`FiggyError::MonotonicSourceRequired`] before anything is
    /// written. Returns whether anything of the chart's monotonic state was
    /// touched, so the caller can drop it if a later step fails.
    pub(in crate::renderer) fn reconcile_monotonic(
        &mut self,
        id: ChartId,
        config: Option<&Config>,
        series: Option<&[SeriesConfig]>,
        placement: Placement,
        lend: &mut Lender<'_>,
    ) -> Result<bool> {
        let plan = {
            let state = self
                .chart_states
                .get(&id)
                .ok_or(FiggyError::UnknownChart { id })?;
            let series = series.unwrap_or(&state.series);
            if !series.iter().any(|cfg| cfg.monotonic().is_some()) && !self.monotonic.has_chart(id)
            {
                return Ok(false);
            }
            let config = config.unwrap_or(&state.config);
            let placed = self.placed_for(id, config, placement)?;
            self.plan_monotonic(id, &placed, series, lend)?
        };
        self.apply_monotonic_plan(plan)?;
        Ok(true)
    }

    /// `config` placed as chart `id`'s display would place it after a
    /// change of `placement`; `config` itself where nothing places it.
    fn placed_for<'c>(
        &self,
        id: ChartId,
        config: &'c Config,
        placement: Placement,
    ) -> Result<std::borrow::Cow<'c, Config>> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let placed = match (placement, &state.display) {
            (Placement::Surface(None), _) | (Placement::Current | Placement::Hold(_), None) => {
                std::borrow::Cow::Borrowed(config)
            }
            (Placement::Surface(Some(surface)), _) => {
                std::borrow::Cow::Owned(display_config_for_surface(config, surface).0)
            }
            // A release fits the document to the surface again; a hold
            // keeps the placement it begins at, the current one.
            (Placement::Hold(false), Some(display)) if display.hold.is_some() => {
                std::borrow::Cow::Owned(display_config_for_surface(config, display.surface).0)
            }
            (Placement::Current | Placement::Hold(_), Some(display)) => {
                std::borrow::Cow::Owned(display.place(config).0)
            }
        };
        Ok(placed)
    }

    /// Plans the monotonic series of `series`, drawn on `placed`, for chart
    /// `chart`. A group whose source `lend` lends is
    /// reduced again from it at every mapping its series are drawn at; any
    /// other group keeps its envelope at an unchanged mapping, and a series
    /// of it whose mapping moved refuses the call. Reads only: a refusal
    /// comes before anything is written.
    fn plan_monotonic(
        &self,
        chart: ChartId,
        placed: &Config,
        series: &[SeriesConfig],
        lend: &mut Lender<'_>,
    ) -> Result<MonotonicPlan> {
        let panels = DocumentPanels::new(placed)?;
        let current = self.monotonic.groups.get(&chart);
        // Each group's envelopes, and whether the call lends its source.
        let mut groups: Vec<(String, Vec<Arc<MonotonicEnvelope>>, bool)> = Vec::new();
        let mut members = Vec::new();
        let mut build =
            |group: &str, mapping: &MonotonicMapping| -> Result<Option<Arc<MonotonicEnvelope>>> {
                let mut envelope = MonotonicEnvelope::empty();
                match lend(group, &mut envelope, mapping) {
                    Some(Ok(_)) => Ok(Some(Arc::new(envelope))),
                    Some(Err(error)) => Err(FiggyError::Monotonic {
                        group: group.to_owned(),
                        error,
                    }),
                    None => Ok(None),
                }
            };
        for cfg in series {
            let Some(monotonic) = cfg.monotonic() else {
                continue;
            };
            let group = monotonic.group.as_str();
            let mapping = panel_mapping(panels.of(&cfg.series_id));
            let old = current.and_then(|groups| groups.get(group));
            let at = match groups.iter().position(|(name, ..)| name == group) {
                Some(at) => at,
                None => {
                    // The group's first series asks for its source.
                    let built = build(group, &mapping)?;
                    let lent = built.is_some();
                    groups.push((group.to_owned(), built.into_iter().collect(), lent));
                    groups.len() - 1
                }
            };
            let (_, envelopes, lent) = &mut groups[at];
            let found = envelopes
                .iter()
                .position(|envelope| envelope.check_mapping(&mapping).is_ok());
            let panel = match found {
                Some(panel) => panel,
                None if *lent => {
                    let built = build(group, &mapping)?
                        .ok_or(FiggyError::internal("a lent source lends at every mapping"))?;
                    envelopes.push(built);
                    envelopes.len() - 1
                }
                None => {
                    let kept = old.and_then(|state| {
                        state
                            .panels
                            .iter()
                            .find(|envelope| envelope.check_mapping(&mapping).is_ok())
                    });
                    match (kept, old) {
                        (Some(kept), _) => envelopes.push(Arc::clone(kept)),
                        (None, Some(_)) => {
                            return Err(FiggyError::MonotonicSourceRequired {
                                series_id: cfg.series_id.clone(),
                                change: self.monotonic.change_of(cfg, old, &mapping),
                            });
                        }
                        // A group without data: the series draws nothing
                        // until its first full call.
                        (None, None) => continue,
                    }
                    envelopes.len() - 1
                }
            };
            let envelope = &envelopes[panel];
            let channels = envelope.channel_count();
            if monotonic.channel >= channels {
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    kind: crate::RuleKind::MonotonicChannel {
                        channel: monotonic.channel,
                        channels,
                    },
                });
            }
            for id in [&cfg.x_column, &cfg.y_column] {
                let ours = self.monotonic.columns.get(id).is_some_and(|record| {
                    record.chart == chart && record.series_id == cfg.series_id
                });
                let taken = self.monotonic.columns.contains_key(id)
                    || self.pool.slot(id).is_some()
                    || self.streaming_sources.contains_key(id);
                if taken && !ours {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: cfg.series_id.clone(),
                        kind: crate::RuleKind::MonotonicColumnTaken {
                            column: id.to_string(),
                        },
                    });
                }
            }
            let write = !self
                .monotonic
                .holds(chart, cfg, group, monotonic.channel, envelope);
            members.push(Planned {
                member: Member {
                    series_id: cfg.series_id.clone(),
                    x_column: cfg.x_column.clone(),
                    y_column: cfg.y_column.clone(),
                    channel: monotonic.channel,
                },
                group: group.to_owned(),
                panel,
                write,
            });
        }
        Ok(MonotonicPlan {
            chart,
            groups: groups
                .into_iter()
                .map(|(name, envelopes, _)| (name, envelopes))
                .collect(),
            members,
        })
    }

    /// Writes a plan: removes the envelope columns no series of it draws
    /// from, uploads the columns that change and records the envelopes. A
    /// failed write drops the chart's monotonic state, so its series draw
    /// nothing rather than an envelope of another mapping.
    fn apply_monotonic_plan(&mut self, plan: MonotonicPlan) -> Result<()> {
        let chart = plan.chart;
        let result = self.write_monotonic_plan(&plan);
        if result.is_err() {
            self.forget_monotonic_chart(chart);
            return result;
        }
        // The chart's records and groups are the plan's; an export's
        // transient ones (their own key) are not touched.
        self.monotonic
            .columns
            .retain(|_, record| !(record.chart == chart && record.key == record.group));
        for planned in &plan.members {
            for id in [&planned.member.x_column, &planned.member.y_column] {
                self.monotonic.columns.insert(
                    id.clone(),
                    MonotonicColumn {
                        chart,
                        key: planned.group.clone(),
                        group: planned.group.clone(),
                        series_id: planned.member.series_id.clone(),
                        panel: planned.panel,
                        channel: planned.member.channel,
                    },
                );
            }
        }
        let groups = self.monotonic.groups.entry(chart).or_default();
        groups.retain(|key, _| key.contains('\u{1}'));
        for (name, panels) in plan.groups {
            if !panels.is_empty() {
                groups.insert(name, MonotonicGroupState { panels });
            }
        }
        Ok(())
    }

    /// The pool writes of a plan.
    fn write_monotonic_plan(&mut self, plan: &MonotonicPlan) -> Result<()> {
        let chart = plan.chart;
        let mut orphans: Vec<ColumnId> = self
            .monotonic
            .columns
            .iter()
            .filter(|(id, record)| {
                record.chart == chart
                    && record.key == record.group
                    && !plan.members.iter().any(|planned| {
                        planned.member.x_column == **id || planned.member.y_column == **id
                    })
            })
            .map(|(id, _)| id.clone())
            .collect();
        orphans.sort();
        for id in &orphans {
            self.monotonic.columns.remove(id);
            self.remove_envelope_column(id)?;
        }
        for planned in plan.members.iter().filter(|planned| planned.write) {
            let envelope = plan
                .groups
                .iter()
                .find(|(name, _)| *name == planned.group)
                .and_then(|(_, panels)| panels.get(planned.panel))
                .ok_or(FiggyError::internal("a planned series' envelope exists"))?;
            for (id, x) in [
                (&planned.member.x_column, true),
                (&planned.member.y_column, false),
            ] {
                let rows = EnvelopeRows {
                    envelope,
                    channel: planned.member.channel,
                    x,
                };
                self.upload_envelope_column(id, &rows)?;
            }
        }
        Ok(())
    }

    /// The append call with the caller's typed source behind `append`;
    /// `sliding` as its options say.
    pub(super) fn append_monotonic_unreported(
        &mut self,
        chart: ChartId,
        group: &str,
        sliding: bool,
        append: &mut AppendCall<'_>,
    ) -> Result<MonotonicUpdate> {
        self.health.check()?;
        let members = self.monotonic_members(chart, group)?;
        let not_built = || FiggyError::Monotonic {
            group: group.to_owned(),
            error: MonotonicError::NotBuilt,
        };
        // Each member's envelope, as the last full call placed it.
        let mut member_panels = Vec::new();
        member_panels
            .try_reserve_exact(members.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "monotonic member panels",
                reason: error.to_string(),
            })?;
        for member in &members {
            let record = self
                .monotonic
                .columns
                .get(&member.x_column)
                .filter(|record| record.chart == chart && record.series_id == member.series_id)
                .ok_or_else(not_built)?;
            member_panels.push(record.panel);
        }
        let state = self
            .monotonic
            .groups
            .get_mut(&chart)
            .and_then(|groups| groups.get_mut(group))
            .ok_or_else(not_built)?;
        let mut update = None;
        for (index, envelope) in state.panels.iter_mut().enumerate() {
            // The core refuses an append before it changes anything, so a
            // refused first envelope leaves the group as it was; a later
            // one leaves the group needing its source.
            match append(Arc::make_mut(envelope)) {
                Ok(done) => {
                    update.get_or_insert(done);
                }
                Err(error) => {
                    if index > 0 {
                        self.forget_monotonic_group(chart, group);
                    }
                    return Err(FiggyError::Monotonic {
                        group: group.to_owned(),
                        error,
                    });
                }
            }
        }
        let update = update.ok_or_else(not_built)?;
        let panels: Vec<Arc<MonotonicEnvelope>> = state.panels.clone();
        if sliding {
            self.write_slid_ranges(chart, &members, &member_panels, &panels)?;
        }
        if let Err(error) = self.upload_members(&members, &member_panels, &panels) {
            self.forget_monotonic_group(chart, group);
            return Err(error);
        }
        Ok(update)
    }

    /// Writes the x range each slid envelope now covers to the axis its
    /// members are drawn against: the parent's `bottom_x`, or a subplot's
    /// own (the root of its link chain when it is linked).
    fn write_slid_ranges(
        &mut self,
        chart: ChartId,
        members: &[Member],
        member_panels: &[usize],
        panels: &[Arc<MonotonicEnvelope>],
    ) -> Result<()> {
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let mut config = state.config.clone();
        for (member, &panel) in members.iter().zip(member_panels) {
            let Some(mapping) = panels.get(panel).and_then(|envelope| envelope.mapping()) else {
                continue;
            };
            if let Some(axis) = x_axis_mut(&mut config, &member.series_id) {
                axis.min = mapping.x_min;
                axis.max = mapping.x_max;
            }
        }
        self.set_chart_config_unreported(chart, config)
    }

    /// Uploads every member's envelope channel to its two pool columns.
    fn upload_members(
        &mut self,
        members: &[Member],
        member_panels: &[usize],
        panels: &[Arc<MonotonicEnvelope>],
    ) -> Result<()> {
        for (member, &panel) in members.iter().zip(member_panels) {
            let envelope = panels
                .get(panel)
                .ok_or(FiggyError::internal("a member's envelope exists"))?;
            for (id, x) in [(&member.x_column, true), (&member.y_column, false)] {
                let rows = EnvelopeRows {
                    envelope,
                    channel: member.channel,
                    x,
                };
                self.upload_envelope_column(id, &rows)?;
            }
        }
        Ok(())
    }
}

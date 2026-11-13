//! Chart state, displays, view state, selection and external invalidations.

use super::*;

impl Renderer {
    /// Atomically replace the renderer-owned config and ordered series.
    ///
    /// This is the mutation boundary for hosts whose one logical edit affects
    /// both declarations, such as an auto-managed legend update. Validation
    /// and every revision successor complete before either value is replaced.
    pub fn set_chart_state(
        &mut self,
        id: ChartId,
        config: Config,
        series: Vec<SeriesConfig>,
    ) -> Result<()> {
        let result = refuse_monotonic_series(&series)
            .and_then(|()| self.set_chart_state_unreported(id, config, series));
        self.reported(result)
    }

    pub(super) fn set_chart_state_unreported(
        &mut self,
        id: ChartId,
        mut config: Config,
        series: Vec<SeriesConfig>,
    ) -> Result<()> {
        crate::chart::validate_renderer_config(&config)?;
        validate_renderer_series(&(&self.pool, &self.streaming_sources), &config, &series)?;
        self.require_resident_overlay_series(&config, &series)?;
        config.set_active_layer(config.active_layer);
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        config.sync_subplot_links(Some(&state.config));
        let touched = self.reconcile_monotonic(
            id,
            Some(&config),
            Some(&series),
            Placement::Current,
            &mut lend_none,
        )?;
        let result = self.commit_chart_state(id, config, series);
        if result.is_err() && touched {
            self.forget_monotonic_chart(id);
        }
        result
    }

    /// The revisions and the write of a validated config and series.
    fn commit_chart_state(
        &mut self,
        id: ChartId,
        config: Config,
        series: Vec<SeriesConfig>,
    ) -> Result<()> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let config_changed = state.config != config;
        let series_changed = state.series != series;
        let clear_selection =
            state.config.active_layer() != config.active_layer() && state.selected.is_some();
        let ChartRevisions {
            desired: desired_before,
            config: config_before,
            series: series_before,
            data: data_before,
            view: view_before,
            selection: selection_before,
            raster: raster_before,
            annotation: annotation_before,
        } = state.revisions;
        let desired = if config_changed || series_changed {
            desired_before.successor("chart desired revision")?
        } else {
            desired_before
        };
        let config_revision = if config_changed {
            config_before.successor("chart config revision")?
        } else {
            config_before
        };
        let series_revision = if series_changed {
            series_before.successor("chart series revision")?
        } else {
            series_before
        };
        let data = if data_series_declarations_equal(&state.series, &series) {
            data_before
        } else {
            data_before.successor("chart data revision")?
        };
        let view = if !stream_config_equal(&state.config, &config, &state.series) {
            view_before.successor("chart view revision")?
        } else {
            view_before
        };
        // Series declarations feed the legend and the raster's series-aware
        // tick guards, so a series change re-rasterizes like a chrome edit.
        let raster =
            if raster_config_equal(&state.config, &config) && !series_changed && !clear_selection {
                raster_before
            } else {
                raster_before.successor("chart raster revision")?
            };
        let visual = self.visual_revision.successor("renderer visual revision")?;
        let mut selection = selection_config_revision(selection_before, &state.config, &config)?;
        if clear_selection {
            selection = selection.successor("chart selection revision")?;
        }
        let next_revisions = ChartRevisions {
            desired,
            config: config_revision,
            series: series_revision,
            data,
            view,
            selection,
            raster,
            annotation: annotation_config_revision(annotation_before, &state.config, &config)?,
        };

        let chart_state = self.chart_states.get_mut(&id).ok_or(FiggyError::internal(
            "validated chart remains registered during state preparation",
        ))?;
        let picker = self
            .picker
            .prepare_active_chart_mutation(&self.pool, id, &config, &series)?;
        if clear_selection {
            chart_state.selected = None;
        }
        PreparedChartMutation {
            chart_state,
            visual_revision: &mut self.visual_revision,
            next_revisions,
            next_visual_revision: visual,
            values: ChartMutationValues::State { config, series },
            picker,
        }
        .commit();
        Ok(())
    }

    /// Draw chart `id` scaled and letterboxed onto a surface of `surface`
    /// physical pixels in [`Self::prepare_registered`] and
    /// [`WindowedRenderer::draw_registered`] (the document fitted as
    /// [`display_config_for_surface`] fits it), or as stored for `None`. The
    /// stored config is unchanged: this is how the chart is shown, not what it
    /// is, so a fit token taken before the call stays current. The scaled
    /// config and the series' styles at that scale are built when the chart is
    /// next prepared, and again only when the config, the series or the scale
    /// change (#160).
    pub fn set_chart_display(&mut self, id: ChartId, surface: Option<(u32, u32)>) -> Result<()> {
        let result = self.set_chart_display_unreported(id, surface);
        self.reported(result)
    }

    pub(super) fn set_chart_display_unreported(
        &mut self,
        id: ChartId,
        surface: Option<(u32, u32)>,
    ) -> Result<()> {
        self.set_chart_display_lent(id, surface, &mut lend_none)
    }

    /// [`Self::set_chart_display`] with the sources `lend` lends (#271): a
    /// surface that changes the plot's width in device pixels rebuilds the
    /// envelopes from them or is refused.
    pub(in crate::renderer) fn set_chart_display_lent(
        &mut self,
        id: ChartId,
        surface: Option<(u32, u32)>,
        lend: &mut Lender<'_>,
    ) -> Result<()> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        if state.display.as_ref().map(|display| display.surface) == surface {
            return Ok(());
        }
        let touched =
            self.reconcile_monotonic(id, None, None, Placement::Surface(surface), lend)?;
        let result = self.commit_chart_display(id, surface);
        if result.is_err() && touched {
            self.forget_monotonic_chart(id);
        }
        result
    }

    /// The revisions and the write of a new display.
    pub(in crate::renderer) fn commit_chart_display(
        &mut self,
        id: ChartId,
        surface: Option<(u32, u32)>,
    ) -> Result<()> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let desired = state
            .revisions
            .desired
            .successor("chart desired revision")?;
        let raster = state.revisions.raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;
        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        state.display = surface.map(|surface| ChartDisplay {
            surface,
            placed: None,
            styles: None,
            hold: None,
        });
        state.revisions.desired = desired;
        state.revisions.raster = raster;
        self.visual_revision = visual;
        Ok(())
    }

    /// Hold chart `id`'s display placement (`true`) or release it (`false`)
    /// (#205). While held, the document keeps the scale and the position of
    /// its parent chart it had when the hold began, so a subplot dragged past
    /// the document's edge (which grows the document) does not rescale the
    /// page under the pointer; parts that leave the surface are cut. The
    /// release fits the document to the surface again. A host holds on a
    /// press that starts a drag and releases on the pointer's release. A
    /// chart without a display ([`Self::set_chart_display`]) ignores it.
    pub fn hold_chart_display(&mut self, id: ChartId, hold: bool) -> Result<()> {
        let result = self.hold_chart_display_unreported(id, hold);
        self.reported(result)
    }

    pub(super) fn hold_chart_display_unreported(&mut self, id: ChartId, hold: bool) -> Result<()> {
        let unchanged = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?
            .display
            .as_ref()
            .is_none_or(|display| display.hold.is_some() == hold);
        if unchanged {
            return Ok(());
        }
        let touched =
            self.reconcile_monotonic(id, None, None, Placement::Hold(hold), &mut lend_none)?;
        let result = self.commit_chart_hold(id, hold);
        if result.is_err() && touched {
            self.forget_monotonic_chart(id);
        }
        result
    }

    /// The revisions and the write of a hold or a release.
    fn commit_chart_hold(&mut self, id: ChartId, hold: bool) -> Result<()> {
        let held = {
            let state = self
                .chart_states
                .get(&id)
                .ok_or(FiggyError::UnknownChart { id })?;
            let Some(display) = &state.display else {
                return Ok(());
            };
            if display.hold.is_some() == hold {
                return Ok(());
            }
            if hold {
                let (placed, _, scale) = self.chart_display_config(id)?;
                let origin = placed.chart_area.0;
                Some(DisplayHold {
                    scale,
                    origin: [i64::from(origin.x), i64::from(origin.y)],
                })
            } else {
                None
            }
        };
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let desired = state
            .revisions
            .desired
            .successor("chart desired revision")?;
        let raster = state.revisions.raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;
        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let Some(display) = state.display.as_mut() else {
            return Ok(());
        };
        display.hold = held;
        display.placed = None;
        state.revisions.desired = desired;
        state.revisions.raster = raster;
        self.visual_revision = visual;
        Ok(())
    }

    /// Chart `id`'s config as [`Self::prepare_registered`] draws it, the rect
    /// its parent panel lands on, and the scale: the placed config of
    /// [`Self::set_chart_display`], borrowed when it is current; or the
    /// stored config at scale 1 when no display is set.
    pub fn chart_display_config(
        &self,
        id: ChartId,
    ) -> Result<(std::borrow::Cow<'_, Config>, Rect, f32)> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let Some(display) = &state.display else {
            return Ok((
                std::borrow::Cow::Borrowed(&state.config),
                state.config.chart_area.0,
                1.0,
            ));
        };
        if let Some(placed) = display
            .placed
            .as_ref()
            .filter(|placed| placed.config_revision == state.revisions.config)
        {
            return Ok((
                std::borrow::Cow::Borrowed(&placed.config),
                placed.panel_rect,
                placed.scale,
            ));
        }
        let (config, panel_rect, scale) = display.place(&state.config);
        Ok((std::borrow::Cow::Owned(config), panel_rect, scale))
    }

    /// Bring chart `id`'s placed config and display-scale styles up to date
    /// with its config, series and surface. A chart without a display has
    /// neither.
    pub(in crate::renderer) fn ensure_registered_display(&mut self, id: ChartId) -> Result<()> {
        let allocation =
            |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource: "registered chart display styles",
                reason: error.to_string(),
            };
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let Some(display) = &state.display else {
            return Ok(());
        };
        let placed_current = display
            .placed
            .as_ref()
            .is_some_and(|placed| placed.config_revision == state.revisions.config);
        let placed = if placed_current {
            None
        } else {
            let (config, panel_rect, scale) = display.place(&state.config);
            Some(PlacedDisplay {
                config_revision: state.revisions.config,
                config,
                panel_rect,
                scale,
            })
        };
        let scale = match (&placed, &display.placed) {
            (Some(placed), _) | (None, Some(placed)) => placed.scale,
            (None, None) => return Err(FiggyError::internal("a current display is placed")),
        };
        let styles_current = display.styles.as_ref().is_some_and(|(bits, styles)| {
            *bits == scale.to_bits() && styles.series_revision == state.revisions.series
        });
        let styles = if styles_current {
            None
        } else {
            let mut styles = Vec::new();
            styles
                .try_reserve_exact(state.series.len())
                .map_err(allocation)?;
            for series in &state.series {
                styles.push(self.create_style_for_series_scaled_inner(series, scale)?);
            }
            Some((
                scale.to_bits(),
                RegisteredChartStyles {
                    series_revision: state.revisions.series,
                    styles,
                },
            ))
        };
        let display = self
            .chart_states
            .get_mut(&id)
            .and_then(|state| state.display.as_mut())
            .ok_or(FiggyError::UnknownChart { id })?;
        if placed.is_some() {
            display.placed = placed;
        }
        if styles.is_some() {
            display.styles = styles;
        }
        Ok(())
    }

    /// Whether the styles of `series` fit the memory budget and the device's
    /// storage limits, without creating them: a host checks an edit with it
    /// before committing, and the styles are created when the chart is next
    /// prepared.
    pub fn preflight_chart_styles(&self, series: &[SeriesConfig]) -> Result<()> {
        let mut bytes = 0u64;
        for item in series {
            bytes = bytes
                .checked_add(self.style_allocation_bytes(item)?)
                .ok_or(FiggyError::GpuResourceLimit {
                    resource: "chart style bytes",
                    requested: u64::MAX,
                    limit: self.gpu_ledger.budget_limit(),
                })?;
        }
        let limit = self.gpu_ledger.budget_limit();
        let requested = self
            .gpu_memory_usage()
            .checked_total_bytes()
            .and_then(|used| used.checked_add(bytes))
            .ok_or(FiggyError::GpuResourceLimit {
                resource: "chart style budget",
                requested: u64::MAX,
                limit,
            })?;
        if requested > limit {
            return Err(FiggyError::GpuResourceLimit {
                resource: "chart style budget",
                requested,
                limit,
            });
        }
        Ok(())
    }

    pub fn set_chart_view_state(&mut self, id: ChartId, view: ChartViewState) -> Result<()> {
        let result = self.set_chart_view_state_unreported(id, view);
        self.reported(result)
    }

    pub(super) fn set_chart_view_state_unreported(
        &mut self,
        id: ChartId,
        view: ChartViewState,
    ) -> Result<()> {
        validate_chart_view_state(&view)?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        if view.overlay_y.len() != state.config.overlays.len()
            || view.overlay_x.len() != state.config.overlays.len()
        {
            return Err(FiggyError::InvalidConfig {
                field: "overlays",
                kind: crate::RuleKind::ViewStateOverlayRanges,
            });
        }
        if view.subplots.len() != state.config.subplots.len()
            || view
                .subplots
                .iter()
                .zip(&state.config.subplots)
                .any(|(view, subplot)| {
                    !view.subplots.is_empty()
                        || view.overlay_y.len() != subplot.config.overlays.len()
                        || view.overlay_x.len() != subplot.config.overlays.len()
                })
        {
            return Err(FiggyError::InvalidConfig {
                field: "subplots",
                kind: crate::RuleKind::ViewStateSubplots,
            });
        }
        let mut prospective_config = state.config.clone();
        view.apply_to(&mut prospective_config);
        prospective_config.sync_subplot_links(Some(&state.config));
        crate::chart::validate_renderer_config(&prospective_config)?;
        let touched = self.reconcile_monotonic(
            id,
            Some(&prospective_config),
            None,
            Placement::Current,
            &mut lend_none,
        )?;
        let result = self.commit_chart_view_state(id, prospective_config);
        if result.is_err() && touched {
            self.forget_monotonic_chart(id);
        }
        result
    }

    /// The revisions and the write of a validated view.
    fn commit_chart_view_state(&mut self, id: ChartId, prospective_config: Config) -> Result<()> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let ChartRevisions {
            desired,
            config,
            // A view edit moves axis ranges only: no declaration, column,
            // selection or annotation changes.
            series,
            data,
            view,
            selection,
            raster,
            annotation,
        } = state.revisions;
        let desired = desired.successor("chart desired revision")?;
        let config = config.successor("chart config revision")?;
        let view_revision = view.successor("chart view revision")?;
        let raster = raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;

        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        // The validated document, links already carried: one write.
        state.config = prospective_config;
        state.stream_auto_fit_padding = None;
        state.revisions = ChartRevisions {
            desired,
            config,
            series,
            data,
            view: view_revision,
            selection,
            raster,
            annotation,
        };
        self.visual_revision = visual;
        Ok(())
    }

    pub fn set_chart_selection(&mut self, id: ChartId, selected: Option<HitId>) -> Result<()> {
        let result = self.set_chart_selection_unreported(id, selected);
        self.reported(result)
    }

    pub(super) fn set_chart_selection_unreported(
        &mut self,
        id: ChartId,
        selected: Option<HitId>,
    ) -> Result<()> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let ChartRevisions {
            desired,
            // An element selection is chrome only: the raster and the
            // selection overlay redraw, nothing is re-declared.
            config,
            series,
            data,
            view,
            selection,
            raster,
            annotation,
        } = state.revisions;
        let desired = desired.successor("chart desired revision")?;
        let selection = selection.successor("chart selection revision")?;
        let raster = raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;

        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        state.selected = selected;
        state.revisions = ChartRevisions {
            desired,
            config,
            series,
            data,
            view,
            selection,
            raster,
            annotation,
        };
        self.visual_revision = visual;
        Ok(())
    }

    /// Observe global font registration without adding a lock to frame or
    /// raster hot paths. All chart revision successors are preflighted before
    /// any state is changed.
    pub fn sync_external_invalidations(&mut self) -> Result<bool> {
        let result = self.sync_external_invalidations_unreported();
        self.reported(result)
    }

    pub(super) fn sync_external_invalidations_unreported(&mut self) -> Result<bool> {
        let generation = crate::text_render::font_generation();
        if generation == self.observed_font_generation {
            return Ok(false);
        }
        if self.chart_count() == 0 {
            self.observed_font_generation = generation;
            return Ok(false);
        }

        let visual = self.visual_revision.successor("renderer visual revision")?;
        let mut updates = Vec::new();
        updates.try_reserve(self.chart_count()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "font invalidation revisions",
                reason: error.to_string(),
            }
        })?;
        for id in self.every_chart() {
            let state = self
                .chart_states
                .get(id)
                .ok_or(FiggyError::UnknownChart { id: *id })?;
            updates.push((
                *id,
                state
                    .revisions
                    .desired
                    .successor("chart desired revision")?,
                state.revisions.raster.successor("chart raster revision")?,
                state
                    .revisions
                    .annotation
                    .successor("chart annotation revision")?,
            ));
        }

        for (id, desired, raster, annotation) in updates {
            let state = self
                .chart_states
                .get_mut(&id)
                .ok_or(FiggyError::UnknownChart { id })?;
            // A font change re-rasterizes text: the axis chrome and the
            // annotation layer, which draws its text with the same fonts.
            let ChartRevisions {
                desired: desired_slot,
                config: _,
                series: _,
                data: _,
                view: _,
                selection: _,
                raster: raster_slot,
                annotation: annotation_slot,
            } = &mut state.revisions;
            *desired_slot = desired;
            *raster_slot = raster;
            *annotation_slot = annotation;
        }
        self.visual_revision = visual;
        self.observed_font_generation = generation;
        Ok(true)
    }
}

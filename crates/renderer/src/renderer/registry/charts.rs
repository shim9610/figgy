//! Registering, reading and replacing charts, their configs and their series.

use super::*;

impl Renderer {
    /// Current renderer-owned visual state identity.
    pub fn visual_revision(&self) -> RenderRevision {
        self.visual_revision
    }

    pub fn visual_stamp(&self) -> RendererVisualStamp {
        RendererVisualStamp {
            revision: self.visual_revision,
            font_generation: crate::text_render::font_generation(),
        }
    }

    /// Chart composition order. This slice, never `HashMap` iteration order,
    /// defines panel ordering for renderer-owned composition.
    pub fn chart_order(&self) -> &[ChartId] {
        &self.chart_order
    }

    pub fn chart_config(&self, id: ChartId) -> Result<&Config> {
        self.chart_states
            .get(&id)
            .map(|state| &state.config)
            .ok_or(FiggyError::UnknownChart { id })
    }

    pub fn chart_series(&self, id: ChartId) -> Result<&[SeriesConfig]> {
        self.chart_states
            .get(&id)
            .map(|state| state.series.as_slice())
            .ok_or(FiggyError::UnknownChart { id })
    }

    /// What one series of a renderer-owned chart draws right now — and whether
    /// the data made it smaller than the declaration asked for.
    ///
    /// The reporting window for the smallest-common-extent rule: a length mismatch is drawn to the
    /// smallest common extent, never raised as an error, so this is how a host
    /// learns that its 11-edge / 9-count histogram describes 9 bars. Reads current
    /// logical lengths, so the answer follows a replacement without a re-prepare.
    /// This is the complete declared extent, not streaming progress.
    pub fn series_draw_info(&self, id: ChartId, series_id: &str) -> Result<SeriesDrawInfo> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let series = state
            .series
            .iter()
            .find(|series| series.series_id == series_id)
            .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                series_id: series_id.to_string(),
                kind: crate::RuleKind::SeriesNotInChart,
            })?;
        Ok(series_draw_info_from_pool(
            &(&self.pool, &self.streaming_sources),
            series,
            state.config.drawn_z_column(series),
        ))
    }

    pub fn chart_selection(&self, id: ChartId) -> Result<Option<HitId>> {
        self.chart_states
            .get(&id)
            .map(|state| state.selected)
            .ok_or(FiggyError::UnknownChart { id })
    }

    pub fn chart_render_stamp(&self, id: ChartId) -> Result<ChartRenderStamp> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        Ok(ChartRenderStamp {
            renderer_identity: self.renderer_identity,
            chart_id: id,
            desired: state.revisions.desired,
            raster: state.revisions.raster,
            annotation: state.revisions.annotation,
        })
    }

    pub fn chart_view_state(&self, id: ChartId) -> Result<ChartViewState> {
        self.chart_states
            .get(&id)
            .map(|state| ChartViewState::from_config(&state.config))
            .ok_or(FiggyError::UnknownChart { id })
    }

    /// Register one renderer-owned chart. The id issuer and visual revision
    /// are checked before any state is published.
    pub fn register_chart(&mut self, config: Config, series: Vec<SeriesConfig>) -> Result<ChartId> {
        let result = self.register_chart_unreported(config, series);
        self.reported(result)
    }

    pub(super) fn register_chart_unreported(
        &mut self,
        mut config: Config,
        series: Vec<SeriesConfig>,
    ) -> Result<ChartId> {
        crate::chart::validate_renderer_config(&config)?;
        validate_renderer_series(&(&self.pool, &self.streaming_sources), &config, &series)?;
        self.require_resident_overlay_series(&config, &series)?;
        config.set_active_layer(config.active_layer);
        // Linked subplot axes are stored equal to their targets.
        config.sync_subplot_links(None);
        let id_value = self
            .next_chart_id
            .checked_add(1)
            .ok_or(FiggyError::CounterExhausted {
                counter: "chart id",
            })?;
        let visual_revision = self.visual_revision.successor("renderer visual revision")?;
        let chart_revision =
            RenderRevision::initial(self.renderer_identity).successor("chart revision")?;
        self.chart_states
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "chart registry",
                reason: error.to_string(),
            })?;
        self.chart_order
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "chart order",
                reason: error.to_string(),
            })?;

        let id = ChartId {
            renderer_identity: self.renderer_identity,
            sequence: id_value,
        };
        if self.chart_states.contains_key(&id) {
            return Err(FiggyError::internal("a new chart id is never registered"));
        }
        self.chart_states.insert(
            id,
            ChartRenderState {
                config,
                series,
                prepared_styles: None,
                selected: None,
                stream_auto_fit_padding: None,
                revisions: ChartRevisions::initial(chart_revision),
                fit_extents: HashMap::new(),
                display: None,
            },
        );
        self.chart_order.push(id);
        self.next_chart_id = id_value;
        self.visual_revision = visual_revision;
        Ok(id)
    }

    pub fn remove_chart(&mut self, id: ChartId) -> Result<()> {
        let result = self.remove_chart_unreported(id);
        self.reported(result)
    }

    pub(super) fn remove_chart_unreported(&mut self, id: ChartId) -> Result<()> {
        if !self.chart_states.contains_key(&id) {
            return Err(FiggyError::UnknownChart { id });
        }
        let visual_revision = self.visual_revision.successor("renderer visual revision")?;
        self.chart_states.remove(&id);
        self.chart_order.retain(|candidate| *candidate != id);
        if self
            .picker
            .active
            .as_ref()
            .is_some_and(|active| active.chart_id == id)
        {
            self.picker.active = None;
        }
        self.visual_revision = visual_revision;
        Ok(())
    }

    pub fn set_chart_config(&mut self, id: ChartId, config: Config) -> Result<()> {
        let result = self.set_chart_config_unreported(id, config);
        self.reported(result)
    }

    pub(super) fn set_chart_config_unreported(
        &mut self,
        id: ChartId,
        mut config: Config,
    ) -> Result<()> {
        crate::chart::validate_renderer_config(&config)?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        validate_renderer_series(
            &(&self.pool, &self.streaming_sources),
            &config,
            &state.series,
        )?;
        self.require_resident_overlay_series(&config, &state.series)?;
        // The registry never holds a layer address past the overlay list: an
        // out-of-range `Overlay(i)` recovers to `Base` here, as the setter does.
        config.set_active_layer(config.active_layer);
        // A linked subplot axis edited in this call moves its whole link
        // group here, inside this one `&mut self` call: no frame can show one
        // member moved and another not.
        config.sync_subplot_links(Some(&state.config));
        // Each revision follows exactly the fields it depends on: an identical
        // config changes nothing, a layer-only or selection-only edit draws
        // again without re-rasterizing the axis chrome.
        let changed = state.config != config;
        // Element selection lives one level below the layer: a `HitId` names
        // an entry of the hit map registered for the previous layer, so a
        // layer change clears it, and its selection chrome must be redrawn.
        let layer_changed = state.config.active_layer() != config.active_layer();
        let clear_selection = layer_changed && state.selected.is_some();
        let ChartRevisions {
            desired: desired_before,
            config: config_before,
            // A config edit leaves the series declarations and their data.
            series,
            data,
            view: view_before,
            selection: selection_before,
            raster: raster_before,
            annotation: annotation_before,
        } = state.revisions;
        let desired = if changed {
            desired_before.successor("chart desired revision")?
        } else {
            desired_before
        };
        let config_revision = if changed {
            config_before.successor("chart config revision")?
        } else {
            config_before
        };
        let view = if !stream_config_equal(&state.config, &config, &state.series) {
            view_before.successor("chart view revision")?
        } else {
            view_before
        };
        let raster = if raster_config_equal(&state.config, &config) && !clear_selection {
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
            series,
            data,
            view,
            selection,
            raster,
            annotation: annotation_config_revision(annotation_before, &state.config, &config)?,
        };

        let chart_state = self.chart_states.get_mut(&id).ok_or(FiggyError::internal(
            "validated chart remains registered during config preparation",
        ))?;
        let picker = self.picker.prepare_active_chart_mutation(
            &self.pool,
            id,
            &config,
            &chart_state.series,
        )?;
        if clear_selection {
            chart_state.selected = None;
        }
        PreparedChartMutation {
            chart_state,
            visual_revision: &mut self.visual_revision,
            next_revisions,
            next_visual_revision: visual,
            values: ChartMutationValues::Config(config),
            picker,
        }
        .commit();
        Ok(())
    }

    /// Set the layer that receives pointer interaction and the hit map
    /// registers (`HitMap::for_active_layer`). An `Overlay` index past the
    /// end of the overlay list is stored as `Base`; the stored layer is
    /// returned. When the layer actually changes, the element selection is
    /// cleared (a `HitId` belongs to the previous layer's map) and, if there
    /// was one, the raster revision advances so the selection chrome is
    /// redrawn on the next refresh; the data revision is untouched. This is
    /// the document's `active_layer` field, so `chart_config` reflects it.
    pub fn set_active_layer(
        &mut self,
        id: ChartId,
        layer: crate::LayerRef,
    ) -> Result<crate::LayerRef> {
        let result = self.set_active_layer_unreported(id, layer);
        self.reported(result)
    }

    pub(super) fn set_active_layer_unreported(
        &mut self,
        id: ChartId,
        layer: crate::LayerRef,
    ) -> Result<crate::LayerRef> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let mut config = state.config.clone();
        let stored = config.set_active_layer(layer);
        if config != state.config {
            self.set_chart_config(id, config)?;
        }
        Ok(stored)
    }

    /// The layer that receives pointer interaction, with the out-of-range
    /// fallback applied.
    pub fn active_layer(&self, id: ChartId) -> Result<crate::LayerRef> {
        let result = self.active_layer_unreported(id);
        self.reported(result)
    }

    pub(super) fn active_layer_unreported(&self, id: ChartId) -> Result<crate::LayerRef> {
        Ok(self.chart_config(id)?.active_layer())
    }

    /// A series an overlay layer scales or a subplot draws must be resident:
    /// the nonresident stream pass draws one panel, so a streamed series on
    /// another panel is an explicit error rather than a silently mis-scaled
    /// drawing. A line offset member is new and not streamed at all (rule
    /// 8): a nonresident member is an explicit error too.
    pub(in crate::renderer) fn require_resident_overlay_series(
        &self,
        config: &Config,
        series: &[SeriesConfig],
    ) -> Result<()> {
        for item in series {
            if config.line_offset_of(&item.series_id).is_some()
                && self.require_resident_series(item).is_err()
            {
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: item.series_id.clone(),
                    kind: crate::RuleKind::LineOffsetMemberStreamed,
                });
            }
            let layer = config.layer_of_series(&item.series_id);
            if !matches!(layer, crate::LayerRef::Base)
                && self.require_resident_series(item).is_err()
            {
                let reason = if matches!(layer, crate::LayerRef::Overlay(_)) {
                    crate::RuleKind::NonresidentOverlaySeries
                } else {
                    crate::RuleKind::NonresidentSubplotSeries
                };
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: item.series_id.clone(),
                    kind: reason,
                });
            }
        }
        Ok(())
    }

    /// Commit the automatic layout of the Feynman series `series_id` of
    /// chart `id` (#234): every vertex without a position gets the one it
    /// is drawn at ([`crate::feynman::FeynmanSeries::auto_layout`]), so the
    /// diagram stays where it is once drawn. The auto-fit pattern: the
    /// renderer writes the display parameter into the registered SSOT and
    /// the host reads it back through [`Self::chart_series`]. Returns
    /// whether anything was written. Synchronous: the layout is CPU work.
    pub fn auto_layout(&mut self, id: ChartId, series_id: &str) -> Result<bool> {
        let result = self.auto_layout_unreported(id, series_id);
        self.reported(result)
    }

    pub(super) fn auto_layout_unreported(&mut self, id: ChartId, series_id: &str) -> Result<bool> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let Some(position) = state
            .series
            .iter()
            .position(|s| s.series_id == series_id && s.feynman().is_some())
        else {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series_id.into(),
                kind: crate::RuleKind::AutoLayoutNotFeynman,
            });
        };
        if !state.series[position]
            .feynman()
            .is_some_and(|d| d.has_unplaced())
        {
            return Ok(false);
        }
        // Configuration only is copied (rule 2).
        let mut series = state.series.clone();
        let laid_out = match series[position].feynman_mut() {
            Some(diagram) => diagram
                .auto_layout()
                .map_err(|error| crate::renderer::feynman_error(series_id, error))?,
            None => false,
        };
        self.set_chart_series_unreported(id, series)?;
        Ok(laid_out)
    }

    /// The anchor space of the panel that draws series `series_id` of chart
    /// `id`, in document logical pixels: where a Feynman drag (#234)
    /// converts its delta, as an annotation drag does. A layered document
    /// answers the series' own panel.
    pub(crate) fn series_panel_space(
        &self,
        id: ChartId,
        series_id: &str,
    ) -> Result<crate::annotation::PixelSpace> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let panels = super::expand_for_render(&state.config)?;
        let index = super::series_panel_index(&state.config, panels.len(), series_id);
        Ok(crate::annotation::pixel_space_for_config(&panels[index]))
    }

    /// Drag a part of the Feynman series `series_id` of chart `id` by
    /// `(dx, dy)` document logical pixels (#234), in the series' panel
    /// space: a vertex moves through the axes
    /// ([`crate::feynman::FeynmanSeries::drag_vertex`]; the first drag
    /// commits the layout) and a propagator bends
    /// ([`crate::feynman::FeynmanSeries::drag_bend`]). A label moves in
    /// logical pixels through the value-label drag
    /// ([`crate::feynman::FeynmanSeries::drag_label`]), which needs no
    /// panel space, and is refused here. The registered series is replaced
    /// on a move; the host reads it back through [`Self::chart_series`].
    pub fn drag_feynman(
        &mut self,
        id: ChartId,
        series_id: &str,
        drag: crate::feynman::FeynmanHit,
        dx: f32,
        dy: f32,
    ) -> Result<crate::layout::NudgeResult> {
        let result = self.drag_feynman_unreported(id, series_id, drag, dx, dy);
        self.reported(result)
    }

    pub(super) fn drag_feynman_unreported(
        &mut self,
        id: ChartId,
        series_id: &str,
        drag: crate::feynman::FeynmanHit,
        dx: f32,
        dy: f32,
    ) -> Result<crate::layout::NudgeResult> {
        use crate::feynman::FeynmanHit;
        if let FeynmanHit::Label(..) = drag {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series_id.into(),
                kind: crate::RuleKind::FeynmanLabelDrag,
            });
        }
        let space = self.series_panel_space(id, series_id)?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let Some(position) = state
            .series
            .iter()
            .position(|s| s.series_id == series_id && s.feynman().is_some())
        else {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series_id.into(),
                kind: crate::RuleKind::DragFeynmanNotFeynman,
            });
        };
        // Configuration only is copied (rule 2).
        let mut series = state.series.clone();
        let Some(diagram) = series[position].feynman_mut() else {
            return Err(FiggyError::internal("the found series is a Feynman series"));
        };
        let moved = match drag {
            FeynmanHit::Vertex(index) => diagram.drag_vertex(index, dx, dy, &space),
            FeynmanHit::Propagator(index) => diagram.drag_bend(index, dx, dy, &space),
            FeynmanHit::Label(..) => Ok(crate::layout::NudgeResult::Rejected(
                crate::layout::NudgeReject::OutOfBounds,
            )),
        }
        .map_err(|error| crate::renderer::feynman_error(series_id, error))?;
        if moved == crate::layout::NudgeResult::Moved {
            self.set_chart_series_unreported(id, series)?;
        }
        Ok(moved)
    }

    pub fn set_chart_series(&mut self, id: ChartId, series: Vec<SeriesConfig>) -> Result<()> {
        let result = self.set_chart_series_unreported(id, series);
        self.reported(result)
    }

    pub(super) fn set_chart_series_unreported(
        &mut self,
        id: ChartId,
        series: Vec<SeriesConfig>,
    ) -> Result<()> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        // Validated against the chart's *current* config: this call replaces the
        // series only, so that config is what the new series will be drawn with.
        validate_renderer_series(
            &(&self.pool, &self.streaming_sources),
            &state.config,
            &series,
        )?;
        self.require_resident_overlay_series(&state.config, &series)?;
        let ChartRevisions {
            desired: desired_before,
            // A series edit leaves the config and everything derived from it
            // alone; the raster follows the series (legend, tick guards).
            config,
            series: series_before,
            data: data_before,
            view,
            selection,
            raster: raster_before,
            annotation,
        } = state.revisions;
        let desired = desired_before.successor("chart desired revision")?;
        let series_revision = series_before.successor("chart series revision")?;
        let data = if data_series_declarations_equal(&state.series, &series) {
            data_before
        } else {
            data_before.successor("chart data revision")?
        };
        let raster = raster_before.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;
        let next_revisions = ChartRevisions {
            desired,
            config,
            series: series_revision,
            data,
            view,
            selection,
            raster,
            annotation,
        };

        let chart_state = self.chart_states.get_mut(&id).ok_or(FiggyError::internal(
            "validated chart remains registered during series preparation",
        ))?;
        let picker = self.picker.prepare_active_chart_mutation(
            &self.pool,
            id,
            &chart_state.config,
            &series,
        )?;
        PreparedChartMutation {
            chart_state,
            visual_revision: &mut self.visual_revision,
            next_revisions,
            next_visual_revision: visual,
            values: ChartMutationValues::Series(series),
            picker,
        }
        .commit();
        Ok(())
    }
}

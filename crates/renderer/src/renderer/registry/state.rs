//! Per-chart registry state: revisions, displays, mutation, replacement and removal plans.

use super::*;

pub(in crate::renderer) fn selection_config_revision(
    current: RenderRevision,
    before: &Config,
    after: &Config,
) -> Result<RenderRevision> {
    if before.picked_data != after.picked_data {
        current.successor("chart selection revision")
    } else {
        Ok(current)
    }
}

/// Whether two configs rasterize the same axis chrome (grid and decoration
/// textures). This is the single classification of `Config` fields for the
/// raster revision: a field listed as `_` here never touches the CPU raster,
/// every other field does. Exhaustive so a new field must be classified.
///
/// - `picked_data` is the GPU selection overlay
///   (`selection_config_revision`).
/// - `active_layer` only routes interaction; `annotations` is the dedicated
///   annotation layer, rasterized by its own pass under its own revision
///   (`annotation_config_revision`).
pub(in crate::renderer) fn raster_config_equal(left: &Config, right: &Config) -> bool {
    let Config {
        chart_area,
        top_x,
        bottom_x,
        left_y,
        right_y,
        chart_title,
        grid,
        legend,
        picked_data: _,
        colorbar,
        draw_style,
        coordinates,
        overlays,
        subplots,
        active_layer: _,
        annotations: _,
        // The bars' layout is drawn by the GPU pass, not rasterized.
        bar_groups: _,
        line_offsets,
        plot_background,
        background,
    } = left;
    chart_area == &right.chart_area
        && top_x == &right.top_x
        && bottom_x == &right.bottom_x
        && left_y == &right.left_y
        && right_y == &right.right_y
        && chart_title == &right.chart_title
        && grid == &right.grid
        && legend == &right.legend
        && colorbar == &right.colorbar
        && draw_style == &right.draw_style
        // The grid, axes and labels are rings and spokes, a triangle, a box
        // that follows the camera, parallel axes with their own ranges, or
        // a rectangle.
        && coordinates == &right.coordinates
        // Every overlay field is chrome of its own panel (its axis, its
        // legend) or decides which panel draws a series.
        && overlays == &right.overlays
        // A subplot is a chart of its own: every field is its panels' chrome,
        // its series list or its links.
        && subplots == &right.subplots
        // Each member is a panel of its own with textures of its own, like
        // an overlay: the group decides how many panels the view holds and
        // which panel draws a series.
        && line_offsets == &right.line_offsets
        // The plot fill is the grid raster's first part (#203).
        && plot_background == &right.plot_background
        // The chart fill is the grid raster's first part (#265).
        && background == &right.background
}

/// Whether two configs map data to the same pixels on every layer: the
/// overlay count, each overlay's Y range and the series each one scales.
/// Decoration-only overlay edits (axis colour, legend) are not geometry.
pub(in crate::renderer) fn overlay_geometry_equal(left: &Config, right: &Config) -> bool {
    left.overlays.len() == right.overlays.len()
        && left.overlays.iter().zip(&right.overlays).all(|(a, b)| {
            AxisViewState::from_axis(&a.y_axis) == AxisViewState::from_axis(&b.y_axis)
                && break_geometry_equal(&a.y_axis, &b.y_axis)
                && a.series == b.series
        })
}

/// Whether two axes' breaks map data to the same pixels (#200): the same
/// cut ranges, style, gap and piece sizing (#205). The mark and the piece
/// frames are chrome.
pub(in crate::renderer) fn break_geometry_equal(left: &AxisOptions, right: &AxisOptions) -> bool {
    fn geometry(
        axis: &AxisOptions,
    ) -> Option<(
        &[crate::config::AxisBreak],
        crate::config::AxisBreakStyle,
        f32,
        &crate::config::PieceSizing,
    )> {
        axis.breaks.as_ref().map(|breaks| {
            let crate::config::AxisBreaks {
                ranges,
                style,
                gap_px,
                // Drawn on the axis line, not part of the mapping.
                mark: _,
                frame_each_panel: _,
                sizing,
            } = breaks;
            (ranges.as_slice(), *style, *gap_px, sizing)
        })
    }
    geometry(left) == geometry(right)
}

/// Whether two configs map data to the same pixels in every subplot: the
/// subplot count, the series each one draws and, per subplot chart with its
/// links applied, the same geometry `stream_config_equal` compares for the
/// parent.
pub(in crate::renderer) fn subplot_geometry_equal(
    left: &Config,
    right: &Config,
    series: &[SeriesConfig],
) -> bool {
    left.subplots.len() == right.subplots.len()
        && (0..left.subplots.len()).all(|index| {
            left.subplots[index].series == right.subplots[index].series
                && match (left.subplot_config(index), right.subplot_config(index)) {
                    (Some(a), Some(b)) => stream_config_equal(&a, &b, series),
                    _ => false,
                }
        })
}

/// The annotation raster revision follows the layer's items only. Its
/// `visible` flag skips the composite without touching any texture, and the
/// anchor space (axes, data area) is chrome, so a change there already bumps
/// the raster revision whose refresh redoes the annotation rasters too.
pub(in crate::renderer) fn annotation_config_revision(
    current: RenderRevision,
    before: &Config,
    after: &Config,
) -> Result<RenderRevision> {
    if before.annotations.items != after.annotations.items {
        current.successor("chart annotation revision")
    } else {
        Ok(current)
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::renderer) struct ChartRevisions {
    pub(in crate::renderer) desired: RenderRevision,
    pub(in crate::renderer) config: RenderRevision,
    pub(in crate::renderer) series: RenderRevision,
    pub(in crate::renderer) data: RenderRevision,
    pub(in crate::renderer) view: RenderRevision,
    pub(in crate::renderer) selection: RenderRevision,
    pub(in crate::renderer) raster: RenderRevision,
    /// The annotation layer's own raster revision: follows
    /// `annotations.items` only, so an annotation edit re-rasterizes the two
    /// annotation textures and nothing else, and a `visible` toggle
    /// re-rasterizes nothing.
    pub(in crate::renderer) annotation: RenderRevision,
}

impl ChartRevisions {
    pub(in crate::renderer) fn initial(revision: RenderRevision) -> Self {
        Self {
            desired: revision,
            config: revision,
            series: revision,
            data: revision,
            view: revision,
            selection: revision,
            raster: revision,
            annotation: revision,
        }
    }
}

pub(in crate::renderer) struct ChartRenderState {
    pub(in crate::renderer) config: Config,
    pub(in crate::renderer) series: Vec<SeriesConfig>,
    pub(in crate::renderer) prepared_styles: Option<RegisteredChartStyles>,
    pub(in crate::renderer) selected: Option<HitId>,
    /// One-shot renderer-owned request consumed by the next automatic stream.
    /// `None` preserves the caller-specified axis ranges.
    pub(in crate::renderer) stream_auto_fit_padding: Option<f64>,
    pub(in crate::renderer) revisions: ChartRevisions,
    /// Each series' fit extent from the last committed whole-chart auto-fit,
    /// by its fit stamp. A stamp names the series' columns by allocation, so a
    /// repeated fit over unchanged columns reuses the extent instead of
    /// rerunning the reduction; a fit replaces the whole map (#160).
    pub(in crate::renderer) fit_extents:
        HashMap<FitSeriesStamp, Option<crate::gpu_extent::GpuSeriesExtent>>,
    /// The surface a host draws this chart on through
    /// [`Renderer::prepare_registered`], when it set one with
    /// [`Renderer::set_chart_display`]; `None` draws the document as stored.
    pub(in crate::renderer) display: Option<ChartDisplay>,
}

/// A registered chart drawn scaled onto a surface (#160). The placed config
/// and the styles at the display scale are display parameters (rule 1): built
/// from the stored config and series when they change, never edited.
pub(in crate::renderer) struct ChartDisplay {
    pub(in crate::renderer) surface: (u32, u32),
    pub(in crate::renderer) placed: Option<PlacedDisplay>,
    /// The series' styles at `placed.scale`, with that scale's bits.
    pub(in crate::renderer) styles: Option<(u32, RegisteredChartStyles)>,
    /// The placement held by [`Renderer::hold_chart_display`] while the
    /// host drags a subplot (#205), instead of the surface fit.
    pub(in crate::renderer) hold: Option<DisplayHold>,
}

/// A held placement: the scale and the surface position of the parent's
/// `chart_area` origin, as they were when the hold began.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::renderer) struct DisplayHold {
    pub(in crate::renderer) scale: f32,
    pub(in crate::renderer) origin: [i64; 2],
}

impl ChartDisplay {
    /// `config` placed on this display: fitted to the surface
    /// ([`display_config_for_surface`]), or at the held scale and parent
    /// origin while a hold is on.
    pub(in crate::renderer) fn place(&self, config: &Config) -> (Config, Rect, f32) {
        match self.hold {
            Some(hold) => super::frame::held_display_config(config, hold.scale, hold.origin),
            None => display_config_for_surface(config, self.surface),
        }
    }
}

/// The stored config scaled and placed on the display surface, for one
/// config revision.
pub(in crate::renderer) struct PlacedDisplay {
    pub(in crate::renderer) config_revision: RenderRevision,
    pub(in crate::renderer) config: Config,
    pub(in crate::renderer) panel_rect: Rect,
    pub(in crate::renderer) scale: f32,
}

// GPU-only derivative; authoritative options stay in ChartRenderState.series.
// Style bindings use stable layouts and scale 1, independent of target format.
pub(in crate::renderer) struct RegisteredChartStyles {
    pub(in crate::renderer) series_revision: RenderRevision,
    pub(in crate::renderer) styles: Vec<ChartStyle>,
}

pub(in crate::renderer) enum ChartMutationValues {
    Config(Config),
    Series(Vec<SeriesConfig>),
    State {
        config: Config,
        series: Vec<SeriesConfig>,
    },
}

pub(in crate::renderer) struct PreparedChartMutation<'a> {
    pub(in crate::renderer) chart_state: &'a mut ChartRenderState,
    pub(in crate::renderer) visual_revision: &'a mut RenderRevision,
    pub(in crate::renderer) next_revisions: ChartRevisions,
    pub(in crate::renderer) next_visual_revision: RenderRevision,
    pub(in crate::renderer) values: ChartMutationValues,
    pub(in crate::renderer) picker: Option<PreparedActivePickerMutation<'a>>,
}

impl PreparedChartMutation<'_> {
    pub(in crate::renderer) fn commit(self) {
        if let Some(picker) = self.picker {
            picker.commit();
        }
        // Hosts may submit the full SSOT after a config-only edit or repeat
        // an identical state. Keep valid GPU style bindings when the ordered
        // declarations are unchanged, without altering mutation revisions.
        let same_series = match &self.values {
            ChartMutationValues::Config(_) => true,
            ChartMutationValues::Series(series) | ChartMutationValues::State { series, .. } => {
                self.chart_state.series == *series
            }
        };
        if same_series {
            if let Some(styles) = &mut self.chart_state.prepared_styles {
                if styles.series_revision == self.chart_state.revisions.series {
                    styles.series_revision = self.next_revisions.series;
                }
            }
            if let Some((_, styles)) = self
                .chart_state
                .display
                .as_mut()
                .and_then(|display| display.styles.as_mut())
            {
                if styles.series_revision == self.chart_state.revisions.series {
                    styles.series_revision = self.next_revisions.series;
                }
            }
        }
        match self.values {
            ChartMutationValues::Config(config) => self.chart_state.config = config,
            ChartMutationValues::Series(series) => self.chart_state.series = series,
            ChartMutationValues::State { config, series } => {
                self.chart_state.config = config;
                self.chart_state.series = series;
            }
        }
        // A fit belongs to the declarations that requested it. Replacing a
        // document (including a tab restore) must not inherit that request.
        self.chart_state.stream_auto_fit_padding = None;
        self.chart_state.revisions = self.next_revisions;
        *self.visual_revision = self.next_visual_revision;
    }
}

pub(in crate::renderer) struct ChartColumnInvalidation {
    pub(in crate::renderer) id: ChartId,
    pub(in crate::renderer) desired: RenderRevision,
    pub(in crate::renderer) data: RenderRevision,
}

pub(in crate::renderer) struct ColumnInvalidationPlan {
    pub(in crate::renderer) charts: Vec<ChartColumnInvalidation>,
    pub(in crate::renderer) visual: Option<RenderRevision>,
}

pub(in crate::renderer) struct ChartReplacementRevision {
    pub(in crate::renderer) id: ChartId,
    pub(in crate::renderer) revisions: ChartRevisions,
}

pub(in crate::renderer) struct ChartReplacementPlan {
    pub(in crate::renderer) target_id: ChartId,
    pub(in crate::renderer) config: Config,
    pub(in crate::renderer) series: Vec<SeriesConfig>,
    pub(in crate::renderer) revisions: Vec<ChartReplacementRevision>,
    pub(in crate::renderer) visual: RenderRevision,
}

impl ChartReplacementPlan {
    pub(in crate::renderer) fn publish(
        self,
        chart_states: &mut HashMap<ChartId, ChartRenderState>,
        visual_revision: &mut RenderRevision,
    ) {
        // The plan's ids were read from this map, which the guard has held
        // exclusively since: every chart is still registered, and a chart
        // that were not would have nothing to publish to.
        if let Some(target) = chart_states.get_mut(&self.target_id) {
            target.config = self.config;
            target.series = self.series;
        }
        for update in self.revisions {
            if let Some(state) = chart_states.get_mut(&update.id) {
                state.revisions = update.revisions;
            }
        }
        *visual_revision = self.visual;
    }
}

impl ColumnInvalidationPlan {
    pub(in crate::renderer) fn prospective_desired_revision(
        &self,
        id: ChartId,
        current: RenderRevision,
    ) -> RenderRevision {
        self.charts
            .iter()
            .find(|update| update.id == id)
            .map_or(current, |update| update.desired)
    }

    pub(in crate::renderer) fn publish(
        self,
        chart_states: &mut HashMap<ChartId, ChartRenderState>,
        visual_revision: &mut RenderRevision,
    ) {
        for update in self.charts {
            // The plan's ids were read from this map, which the guard has
            // held exclusively since; see `ChartReplacementPlan::publish`.
            let Some(state) = chart_states.get_mut(&update.id) else {
                continue;
            };
            // A column edit changes what is drawn, not any declaration.
            let ChartRevisions {
                desired,
                config: _,
                series: _,
                data,
                view: _,
                selection: _,
                raster: _,
                annotation: _,
            } = &mut state.revisions;
            *desired = update.desired;
            *data = update.data;
        }
        if let Some(visual) = self.visual {
            *visual_revision = visual;
        }
    }
}

pub(in crate::renderer) struct ChartSeriesRemoval {
    pub(in crate::renderer) id: ChartId,
    /// Set when the chart's series that reference the column are removed.
    pub(in crate::renderer) referencing: Option<SeriesRemovalRevisions>,
    /// Set when the chart's config is replaced with the removal.
    pub(in crate::renderer) replacement: Option<ChartConfigReplacement>,
    pub(in crate::renderer) desired: RenderRevision,
    pub(in crate::renderer) raster: RenderRevision,
}

/// The revisions a chart publishes when its referencing series are removed.
pub(in crate::renderer) struct SeriesRemovalRevisions {
    pub(in crate::renderer) series: RenderRevision,
    pub(in crate::renderer) data: RenderRevision,
}

/// A chart config published with a column removal, and its revisions.
pub(in crate::renderer) struct ChartConfigReplacement {
    pub(in crate::renderer) config: Config,
    pub(in crate::renderer) config_revision: RenderRevision,
    pub(in crate::renderer) selection: RenderRevision,
    pub(in crate::renderer) annotation: RenderRevision,
    pub(in crate::renderer) view: Option<RenderRevision>,
}

pub(in crate::renderer) struct ColumnRemovalPlan {
    pub(in crate::renderer) charts: Vec<ChartSeriesRemoval>,
    pub(in crate::renderer) visual: Option<RenderRevision>,
}

impl ColumnRemovalPlan {
    pub(in crate::renderer) fn publish(
        self,
        chart_states: &mut HashMap<ChartId, ChartRenderState>,
        visual_revision: &mut RenderRevision,
        removed_column: &str,
    ) {
        for update in self.charts {
            // The plan's ids were read from this map, which the guard has
            // held exclusively since; see `ChartReplacementPlan::publish`.
            let Some(state) = chart_states.get_mut(&update.id) else {
                continue;
            };
            let ChartRevisions {
                desired,
                config: config_revision,
                series: series_revision,
                data,
                view,
                selection,
                raster,
                annotation,
            } = &mut state.revisions;
            if let Some(referencing) = update.referencing {
                state
                    .series
                    .retain(|series| !series_dropped_with_column(series, removed_column));
                *series_revision = referencing.series;
                *data = referencing.data;
            }
            if let Some(replacement) = update.replacement {
                state.config = replacement.config;
                *config_revision = replacement.config_revision;
                *selection = replacement.selection;
                *annotation = replacement.annotation;
                if let Some(next_view) = replacement.view {
                    *view = next_view;
                }
            }
            *desired = update.desired;
            *raster = update.raster;
        }
        if let Some(visual) = self.visual {
            *visual_revision = visual;
        }
    }
}

pub(in crate::renderer) fn series_after_column_removal(
    series: &[SeriesConfig],
    removed_column: &str,
) -> Result<Vec<SeriesConfig>> {
    let retained = series
        .iter()
        .filter(|series| !series_dropped_with_column(series, removed_column))
        .count();
    let mut candidate = Vec::new();
    candidate
        .try_reserve_exact(retained)
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "column removal series candidate",
            reason: error.to_string(),
        })?;
    candidate.extend(
        series
            .iter()
            .filter(|series| !series_dropped_with_column(series, removed_column))
            .cloned(),
    );
    Ok(candidate)
}

//! The chart and column registry: registered chart state and its revisions,
//! chart and view-state edits, column registration, upsert, invalidation and
//! removal plans, the demo loader and the web-derived snapshot.

use super::*;
use crate::{OverlayLayer, Subplot};

/// Logical data-to-screen state derived from the renderer-owned `Config`.
///
/// This value is never stored as a parallel truth source. The getter derives
/// it from `Config`; the setter applies it back to those same four axes.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartViewState {
    pub chart_area: Rect,
    pub top_x: AxisViewState,
    pub bottom_x: AxisViewState,
    pub left_y: AxisViewState,
    pub right_y: AxisViewState,
    /// One entry per overlay layer, in `Config::overlays` order. A view state
    /// applied to a chart must carry exactly as many entries as the chart has
    /// overlays; the X range is shared, so one call moves every layer.
    pub overlay_y: Vec<AxisViewState>,
    /// One entry per subplot, in `Config::subplots` order, each the view
    /// state of that subplot's chart (its own `subplots` empty). A linked
    /// axis reads as its target's range; writing a new range to it moves the
    /// whole link group, in the same call as everything else.
    pub subplots: Vec<ChartViewState>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AxisViewState {
    pub scale: AxisScale,
    pub min: f64,
    pub max: f64,
    pub inverted: bool,
}

impl AxisViewState {
    pub(super) fn from_axis(axis: &AxisOptions) -> Self {
        let AxisOptions {
            scale,
            min,
            max,
            // Re-derived from the range when a view state is applied.
            major_spacing: _,
            minor_count: _,
            inverted,
            // Style and band: not part of the data-to-screen mapping.
            label_style: _,
            tick: _,
            title_option: _,
            out_margin: _,
            line_offset: _,
            line_visible: _,
            line_color: _,
            line_width: _,
            line_style: _,
            major_tick_length: _,
            minor_tick_length: _,
            // The range of a categorical axis (`data_range`).
            categories: _,
            // Where the line is drawn, not how data maps.
            cross_at: _,
            // Which ticks a log axis draws, not how data maps.
            log_ticks: _,
        } = axis;
        let (min, max) = match axis.categories {
            Some(_) => axis.data_range(),
            None => (*min, *max),
        };
        Self {
            scale: scale.clone(),
            min,
            max,
            inverted: *inverted,
        }
    }

    pub(super) fn apply_to(&self, axis: &mut AxisOptions) {
        let Self {
            scale,
            min,
            max,
            inverted,
        } = self;
        axis.scale = scale.clone();
        crate::chart::apply_axis_range(axis, *min, *max, matches!(scale, AxisScale::Logarithmic));
        axis.inverted = *inverted;
    }
}

impl ChartViewState {
    pub(super) fn from_config(config: &Config) -> Self {
        let Config {
            chart_area,
            top_x,
            bottom_x,
            left_y,
            right_y,
            // Chrome, selections, z scale, style and routing: not part of
            // the data-to-screen mapping.
            chart_title: _,
            grid: _,
            legend: _,
            picked_points: _,
            picked_data: _,
            colorbar: _,
            draw_style: _,
            polar: _,
            ternary: _,
            view_3d: _,
            overlays,
            subplots,
            active_layer: _,
            annotations: _,
            // How bars share a slot, not how data maps to the screen.
            bar_groups: _,
            // The member shifts are display parameters of the group, kept
            // as they are when a view state moves the ranges.
            line_offsets: _,
        } = config;
        Self {
            chart_area: chart_area.0,
            top_x: AxisViewState::from_axis(top_x),
            bottom_x: AxisViewState::from_axis(bottom_x),
            left_y: AxisViewState::from_axis(left_y),
            right_y: AxisViewState::from_axis(right_y),
            overlay_y: overlays
                .iter()
                .map(|overlay| {
                    let OverlayLayer {
                        y_axis,
                        series: _,
                        legend: _,
                    } = overlay;
                    AxisViewState::from_axis(y_axis)
                })
                .collect(),
            // Each subplot as it draws, its linked axes following their
            // targets.
            subplots: (0..subplots.len())
                .filter_map(|index| config.subplot_config(index))
                .map(|subplot| Self::from_config(&subplot))
                .collect(),
        }
    }

    pub(super) fn apply_to(&self, config: &mut Config) {
        let Self {
            chart_area,
            top_x,
            bottom_x,
            left_y,
            right_y,
            overlay_y,
            subplots,
        } = self;
        let Config {
            chart_area: config_area,
            top_x: config_top_x,
            bottom_x: config_bottom_x,
            left_y: config_left_y,
            right_y: config_right_y,
            // Not part of a view state (see `from_config`).
            chart_title: _,
            grid: _,
            legend: _,
            picked_points: _,
            picked_data: _,
            colorbar: _,
            draw_style: _,
            polar: _,
            ternary: _,
            view_3d: _,
            overlays: config_overlays,
            subplots: config_subplots,
            active_layer: _,
            annotations: _,
            bar_groups: _,
            line_offsets: _,
        } = config;
        config_area.0 = *chart_area;
        top_x.apply_to(config_top_x);
        bottom_x.apply_to(config_bottom_x);
        left_y.apply_to(config_left_y);
        right_y.apply_to(config_right_y);
        for (state, overlay) in overlay_y.iter().zip(config_overlays) {
            let OverlayLayer {
                y_axis,
                series: _,
                legend: _,
            } = overlay;
            state.apply_to(y_axis);
        }
        for (state, subplot) in subplots.iter().zip(config_subplots) {
            let Subplot {
                config,
                series: _,
                link_x: _,
                link_y: _,
            } = subplot;
            state.apply_to(config);
        }
    }
}

pub(super) fn selection_config_revision(
    current: RenderRevision,
    before: &Config,
    after: &Config,
) -> Result<RenderRevision> {
    if before.picked_points != after.picked_points || before.picked_data != after.picked_data {
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
/// - `picked_points` / `picked_data` are the GPU selection overlay
///   (`selection_config_revision`).
/// - `active_layer` only routes interaction; `annotations` is the dedicated
///   annotation layer, rasterized by its own pass under its own revision
///   (`annotation_config_revision`).
pub(super) fn raster_config_equal(left: &Config, right: &Config) -> bool {
    let Config {
        chart_area,
        top_x,
        bottom_x,
        left_y,
        right_y,
        chart_title,
        grid,
        legend,
        picked_points: _,
        picked_data: _,
        colorbar,
        draw_style,
        polar,
        ternary,
        view_3d,
        overlays,
        subplots,
        active_layer: _,
        annotations: _,
        // The bars' layout is drawn by the GPU pass, not rasterized.
        bar_groups: _,
        line_offsets,
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
        // The grid, axes and labels are rings and spokes or a rectangle.
        && polar == &right.polar
        && ternary == &right.ternary
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
}

/// Whether two configs map data to the same pixels on every layer: the
/// overlay count, each overlay's Y range and the series each one scales.
/// Decoration-only overlay edits (axis colour, legend) are not geometry.
pub(super) fn overlay_geometry_equal(left: &Config, right: &Config) -> bool {
    left.overlays.len() == right.overlays.len()
        && left.overlays.iter().zip(&right.overlays).all(|(a, b)| {
            AxisViewState::from_axis(&a.y_axis) == AxisViewState::from_axis(&b.y_axis)
                && a.series == b.series
        })
}

/// Whether two configs map data to the same pixels in every subplot: the
/// subplot count, the series each one draws and, per subplot chart with its
/// links applied, the same geometry `stream_config_equal` compares for the
/// parent.
pub(super) fn subplot_geometry_equal(
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
pub(super) fn annotation_config_revision(
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ColumnContentStamp {
    pub(super) id: ColumnId,
    pub(super) allocation_epoch: u64,
}

/// Atomic identity for all renderer state used by web-owned derived caches.
///
/// Fields stay private so the web wrapper cannot reconstruct or increment
/// renderer counters independently.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WebDerivedStamp {
    pub(super) renderer_identity: u64,
    pub(super) chart_id: ChartId,
    pub(super) view: RenderRevision,
    pub(super) data: RenderRevision,
    pub(super) pool_layout_generation: u64,
    pub(super) columns: Vec<ColumnContentStamp>,
}

/// One coherent, owned read of renderer state for browser-only derived work.
#[derive(Clone, Debug)]
pub struct WebDerivedSnapshot {
    pub(super) stamp: WebDerivedStamp,
    pub(super) fit_token: FitCommitToken,
    pub(super) config: Config,
    pub(super) series: Vec<SeriesConfig>,
}

impl WebDerivedSnapshot {
    pub fn stamp(&self) -> &WebDerivedStamp {
        &self.stamp
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn series(&self) -> &[SeriesConfig] {
        &self.series
    }

    pub fn fit_token(&self) -> &FitCommitToken {
        &self.fit_token
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ChartRevisions {
    pub(super) desired: RenderRevision,
    pub(super) config: RenderRevision,
    pub(super) series: RenderRevision,
    pub(super) data: RenderRevision,
    pub(super) view: RenderRevision,
    pub(super) selection: RenderRevision,
    pub(super) raster: RenderRevision,
    /// The annotation layer's own raster revision: follows
    /// `annotations.items` only, so an annotation edit re-rasterizes the two
    /// annotation textures and nothing else, and a `visible` toggle
    /// re-rasterizes nothing.
    pub(super) annotation: RenderRevision,
}

impl ChartRevisions {
    pub(super) fn initial(revision: RenderRevision) -> Self {
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

pub(super) struct ChartRenderState {
    pub(super) config: Config,
    pub(super) series: Vec<SeriesConfig>,
    pub(super) prepared_styles: Option<RegisteredChartStyles>,
    pub(super) selected: Option<HitId>,
    /// One-shot renderer-owned request consumed by the next automatic stream.
    /// `None` preserves the caller-specified axis ranges.
    pub(super) stream_auto_fit_padding: Option<f64>,
    pub(super) revisions: ChartRevisions,
}

// GPU-only derivative; authoritative options stay in ChartRenderState.series.
// Style bindings use stable layouts and scale 1, independent of target format.
pub(super) struct RegisteredChartStyles {
    pub(super) series_revision: RenderRevision,
    pub(super) styles: Vec<ChartStyle>,
}

pub(super) enum ChartMutationValues {
    Config(Config),
    Series(Vec<SeriesConfig>),
    State {
        config: Config,
        series: Vec<SeriesConfig>,
    },
}

pub(super) struct PreparedChartMutation<'a> {
    pub(super) chart_state: &'a mut ChartRenderState,
    pub(super) visual_revision: &'a mut RenderRevision,
    pub(super) next_revisions: ChartRevisions,
    pub(super) next_visual_revision: RenderRevision,
    pub(super) values: ChartMutationValues,
    pub(super) picker: Option<PreparedActivePickerMutation<'a>>,
}

impl PreparedChartMutation<'_> {
    pub(super) fn commit(self) {
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

pub(super) struct ChartColumnInvalidation {
    pub(super) id: ChartId,
    pub(super) desired: RenderRevision,
    pub(super) data: RenderRevision,
}

pub(super) struct ColumnInvalidationPlan {
    pub(super) charts: Vec<ChartColumnInvalidation>,
    pub(super) visual: Option<RenderRevision>,
}

pub(super) struct LoadDemoChartRevision {
    pub(super) id: ChartId,
    pub(super) revisions: ChartRevisions,
}

pub(super) struct LoadDemoChartPlan {
    pub(super) target_id: ChartId,
    pub(super) config: Config,
    pub(super) series: Vec<SeriesConfig>,
    pub(super) revisions: Vec<LoadDemoChartRevision>,
    pub(super) visual: RenderRevision,
}

impl LoadDemoChartPlan {
    pub(super) fn publish(
        self,
        chart_states: &mut HashMap<ChartId, ChartRenderState>,
        visual_revision: &mut RenderRevision,
    ) {
        let target = chart_states
            .get_mut(&self.target_id)
            .expect("load-demo target chart remains registered");
        target.config = self.config;
        target.series = self.series;
        for update in self.revisions {
            chart_states
                .get_mut(&update.id)
                .expect("load-demo affected chart remains registered")
                .revisions = update.revisions;
        }
        *visual_revision = self.visual;
    }
}

impl ColumnInvalidationPlan {
    pub(super) fn prospective_desired_revision(
        &self,
        id: ChartId,
        current: RenderRevision,
    ) -> RenderRevision {
        self.charts
            .iter()
            .find(|update| update.id == id)
            .map_or(current, |update| update.desired)
    }

    pub(super) fn prospective_data_revision(
        &self,
        id: ChartId,
        current: RenderRevision,
    ) -> RenderRevision {
        self.charts
            .iter()
            .find(|update| update.id == id)
            .map_or(current, |update| update.data)
    }

    pub(super) fn publish(
        self,
        chart_states: &mut HashMap<ChartId, ChartRenderState>,
        visual_revision: &mut RenderRevision,
    ) {
        for update in self.charts {
            let state = chart_states
                .get_mut(&update.id)
                .expect("column invalidation chart remains registered");
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

pub(super) struct ChartSeriesRemoval {
    pub(super) id: ChartId,
    pub(super) remove_referencing_series: bool,
    pub(super) next_config: Option<Config>,
    pub(super) desired: RenderRevision,
    pub(super) config_revision: Option<RenderRevision>,
    pub(super) selection_revision: Option<RenderRevision>,
    pub(super) annotation_revision: Option<RenderRevision>,
    pub(super) series_revision: Option<RenderRevision>,
    pub(super) data: Option<RenderRevision>,
    pub(super) view: Option<RenderRevision>,
    pub(super) raster: RenderRevision,
}

pub(super) struct ColumnRemovalPlan {
    pub(super) charts: Vec<ChartSeriesRemoval>,
    pub(super) visual: Option<RenderRevision>,
}

impl ColumnRemovalPlan {
    pub(super) fn publish(
        self,
        chart_states: &mut HashMap<ChartId, ChartRenderState>,
        visual_revision: &mut RenderRevision,
        removed_column: &str,
    ) {
        for update in self.charts {
            let state = chart_states
                .get_mut(&update.id)
                .expect("column removal chart remains registered");
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
            if update.remove_referencing_series {
                state
                    .series
                    .retain(|series| !series_references_column(series, removed_column));
                *series_revision = update
                    .series_revision
                    .expect("series-removal revision was prepared");
                *data = update.data.expect("data revision was prepared");
            }
            if let Some(config) = update.next_config {
                state.config = config;
                *config_revision = update
                    .config_revision
                    .expect("config replacement revision was prepared");
                *selection = update
                    .selection_revision
                    .expect("selection revision was prepared");
                *annotation = update
                    .annotation_revision
                    .expect("annotation revision was prepared");
                if let Some(next_view) = update.view {
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

pub(super) fn series_after_column_removal(
    series: &[SeriesConfig],
    removed_column: &str,
) -> Result<Vec<SeriesConfig>> {
    let retained = series
        .iter()
        .filter(|series| !series_references_column(series, removed_column))
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
            .filter(|series| !series_references_column(series, removed_column))
            .cloned(),
    );
    Ok(candidate)
}

/// Renderer-level guard for an atomic scalar or hi/lo column upsert.
///
/// The provisional pool is available for picker/bind-group preparation.  A
/// dropped guard restores the previous column mapping; `commit` is infallible
/// and allocation-free.
#[must_use = "dropping a RendererColumnUpsert rolls the provisional pool state back"]
pub struct RendererColumnUpsert<'a> {
    pub(super) streaming_sources: &'a HashMap<ColumnId, RegisteredStreamColumn>,
    pub(super) inner: Option<data_render::column_pool::ColumnUpsert<'a>>,
    pub(super) chart_states: &'a mut HashMap<ChartId, ChartRenderState>,
    pub(super) visual_revision: &'a mut RenderRevision,
    pub(super) pending_defrag: &'a mut bool,
    pub(super) invalidation: Option<ColumnInvalidationPlan>,
    pub(super) picker: Option<PreparedActivePickerMutation<'a>>,
    pub(super) pending_defrag_after_commit: Option<bool>,
    pub(super) renderer_identity: u64,
    pub(super) device: &'a wgpu::Device,
    pub(super) queue: &'a wgpu::Queue,
    pub(super) errorbar_extent_engine: Option<&'a crate::gpu_errorbar::GpuErrorbarExtentEngine>,
}

/// Prepared renderer side of the bundled demo transaction.
#[must_use = "dropping a RendererLoadDemo leaves renderer authority unchanged"]
pub struct RendererLoadDemo<'a> {
    pub(super) inner: Option<data_render::column_pool::ColumnBatchUpsert<'a>>,
    pub(super) chart_states: &'a mut HashMap<ChartId, ChartRenderState>,
    pub(super) visual_revision: &'a mut RenderRevision,
    pub(super) pending_defrag: &'a mut bool,
    pub(super) chart_plan: Option<LoadDemoChartPlan>,
    pub(super) picker: Option<PreparedActivePickerMutation<'a>>,
}

impl RendererLoadDemo<'_> {
    /// Publish the four demo columns, chart state, picker, and maintenance bit.
    pub fn commit(mut self) -> [ColumnHandle; 4] {
        let handles = self
            .inner
            .take()
            .expect("renderer load-demo candidate remains live")
            .commit();
        self.chart_plan
            .take()
            .expect("renderer load-demo chart plan remains live")
            .publish(self.chart_states, self.visual_revision);
        if let Some(picker) = self.picker.take() {
            picker.commit();
        }
        *self.pending_defrag = false;
        handles
    }
}

impl RendererColumnUpsert<'_> {
    pub(super) fn inner(&self) -> &data_render::column_pool::ColumnUpsert<'_> {
        self.inner
            .as_ref()
            .expect("renderer column upsert remains live")
    }

    pub fn pool(&self) -> &ColumnPool {
        self.inner().pool()
    }

    pub fn handle(&self) -> ColumnHandle {
        self.inner().handle()
    }

    pub fn replaced_existing(&self) -> bool {
        self.inner().replaced_existing()
    }

    /// Coherent prospective snapshot for cache preparation before commit.
    ///
    /// The stamp reflects the provisional pool mapping and the checked data
    /// revision that `commit` will publish. Dropping this guard rolls the pool
    /// back, so the returned stamp never becomes current.
    pub fn web_derived_snapshot(&self, id: ChartId) -> Result<WebDerivedSnapshot> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        for series in &state.series {
            require_resident_sources(self.streaming_sources, series)?;
        }
        let data_revision = self
            .invalidation
            .as_ref()
            .map_or(state.revisions.data, |invalidation| {
                invalidation.prospective_data_revision(id, state.revisions.data)
            });
        build_web_derived_snapshot_from_state(
            self.pool(),
            self.streaming_sources,
            self.renderer_identity,
            id,
            state,
            data_revision,
        )
    }

    /// Begin an extent job against the provisional mappings.  The column
    /// upload was submitted before this guard was returned, so queue ordering
    /// makes the new bytes visible to the reduction.
    pub fn begin_errorbar_extent(
        &self,
        value_id: &str,
        lower_id: &str,
        upper_id: &str,
    ) -> std::result::Result<
        crate::gpu_errorbar::GpuErrorbarExtentTicket,
        crate::gpu_errorbar::GpuErrorbarError,
    > {
        for id in [value_id, lower_id, upper_id] {
            if self.streaming_sources.contains_key(id) {
                return Err(crate::gpu_errorbar::GpuErrorbarError::ColumnNotResident {
                    id: id.to_owned(),
                });
            }
        }
        begin_errorbar_extent_from_pool(
            prepared_errorbar_extent_engine(self.errorbar_extent_engine)?,
            self.device,
            self.queue,
            self.pool(),
            value_id,
            lower_id,
            upper_id,
        )
    }

    /// Begin a drawable-series fit-bound job against the provisional
    /// mappings. Inactive error directions are `None`; active directions
    /// require both lower and upper ids.
    pub fn begin_series_extent(
        &self,
        mode: crate::gpu_errorbar::GpuSeriesExtentMode,
        columns: crate::gpu_errorbar::GpuSeriesExtentColumnIds<'_>,
    ) -> std::result::Result<
        crate::gpu_errorbar::GpuSeriesExtentTicket,
        crate::gpu_errorbar::GpuErrorbarError,
    > {
        for id in [
            Some(columns.x),
            Some(columns.y),
            columns.x_lower,
            columns.x_upper,
            columns.y_lower,
            columns.y_upper,
        ]
        .into_iter()
        .flatten()
        {
            if self.streaming_sources.contains_key(id) {
                return Err(crate::gpu_errorbar::GpuErrorbarError::ColumnNotResident {
                    id: id.to_owned(),
                });
            }
        }
        begin_series_extent_from_pool(
            prepared_errorbar_extent_engine(self.errorbar_extent_engine)?,
            self.device,
            self.queue,
            self.pool(),
            mode,
            columns,
        )
    }

    /// Begin the exact GPU fit domain for a complete series declaration against
    /// the provisional pool. Matrix-backed series resolve only their cell counts
    /// on the CPU; coordinate values and edge/sample arithmetic stay on the GPU.
    pub fn begin_series_fit_extent(
        &self,
        series: &SeriesConfig,
    ) -> std::result::Result<
        Option<crate::gpu_errorbar::GpuSeriesExtentTicket>,
        crate::gpu_errorbar::GpuErrorbarError,
    > {
        if let Err(FiggyError::ColumnNotResident { id }) =
            require_resident_sources(self.streaming_sources, series)
        {
            return Err(crate::gpu_errorbar::GpuErrorbarError::ColumnNotResident { id });
        }
        begin_series_fit_extent_from_pool(
            prepared_errorbar_extent_engine(self.errorbar_extent_engine)?,
            self.device,
            self.queue,
            self.pool(),
            series,
        )
    }

    pub fn commit(mut self) -> ColumnHandle {
        let handle = self
            .inner
            .take()
            .expect("renderer column upsert remains live")
            .commit();
        if let Some(invalidation) = self.invalidation.take() {
            invalidation.publish(self.chart_states, self.visual_revision);
        }
        if let Some(picker) = self.picker.take() {
            picker.commit();
        }
        if let Some(pending) = self.pending_defrag_after_commit {
            *self.pending_defrag = pending;
        }
        handle
    }
}

/// Rebuild the active picker against a pool that has just been mutated, when it
/// has to be rebuilt.
///
/// Two reasons force it: the pool relocated its columns, so every recorded
/// offset is stale, or the picker reads one of the columns the mutation touched,
/// so its slots have to be rebuilt from the new bytes. When only the first
/// applies, `rebuild_columns` stays empty and matching slots are reused.
///
/// Shared by the single-column upsert and the batch insert so "when must the
/// picker be rebuilt" has one answer.
pub(super) fn prepare_picker_for_pool_mutation<'picker>(
    picker: &'picker mut RendererPicker,
    pool: &ColumnPool,
    chart_states: &HashMap<ChartId, ChartRenderState>,
    relocated: bool,
    changed_columns: &[&str],
) -> Result<Option<PreparedActivePickerMutation<'picker>>> {
    let Some((chart_id, rebuild)) = picker.active.as_ref().map(|active| {
        (
            active.chart_id,
            active.references_any_column(changed_columns),
        )
    }) else {
        return Ok(None);
    };
    if !relocated && !rebuild {
        return Ok(None);
    }
    let state = chart_states
        .get(&chart_id)
        .ok_or(FiggyError::UnknownChart { id: chart_id })?;
    let plan = PickChartPlan::for_active_layer(&state.config, &state.series)?;
    Ok(picker.prepare_active_pool_mutation(
        pool,
        chart_id,
        plan,
        if rebuild { changed_columns } else { &[] },
    )?)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_renderer_column_upsert<'a>(
    inner: data_render::column_pool::ColumnUpsert<'a>,
    streaming_sources: &'a HashMap<ColumnId, RegisteredStreamColumn>,
    changed_column: &str,
    old_layout_generation: u64,
    chart_states: &'a mut HashMap<ChartId, ChartRenderState>,
    visual_revision: &'a mut RenderRevision,
    pending_defrag: &'a mut bool,
    picker: &'a mut RendererPicker,
    invalidation: Option<ColumnInvalidationPlan>,
    renderer_identity: u64,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    errorbar_extent_engine: Option<&'a crate::gpu_errorbar::GpuErrorbarExtentEngine>,
) -> Result<RendererColumnUpsert<'a>> {
    let relocated = inner.pool().layout_generation() != old_layout_generation;
    let changed_columns = [changed_column];
    let picker = prepare_picker_for_pool_mutation(
        picker,
        inner.pool(),
        &*chart_states,
        relocated,
        &changed_columns,
    )?;
    let pending_defrag_after_commit = if relocated {
        Some(false)
    } else if inner.replaced_existing() {
        Some(true)
    } else {
        None
    };

    Ok(RendererColumnUpsert {
        streaming_sources,
        inner: Some(inner),
        chart_states,
        visual_revision,
        pending_defrag,
        invalidation,
        picker,
        pending_defrag_after_commit,
        renderer_identity,
        device,
        queue,
        errorbar_extent_engine,
    })
}

#[must_use = "dropping a PreparedColumnRemoval rolls the composite removal back"]
pub(super) struct PreparedColumnRemoval<'a> {
    pub(super) inner: Option<data_render::column_pool::ColumnRemoval<'a>>,
    pub(super) chart_states: &'a mut HashMap<ChartId, ChartRenderState>,
    pub(super) visual_revision: &'a mut RenderRevision,
    pub(super) pending_defrag: &'a mut bool,
    pub(super) removal: Option<ColumnRemovalPlan>,
    pub(super) picker: Option<PreparedActivePickerMutation<'a>>,
    pub(super) removed_column: ColumnId,
}

impl PreparedColumnRemoval<'_> {
    pub(super) fn commit(mut self) -> bool {
        self.inner
            .take()
            .expect("renderer column removal remains live")
            .commit();
        self.removal
            .take()
            .expect("renderer column removal plan remains live")
            .publish(
                self.chart_states,
                self.visual_revision,
                &self.removed_column,
            );
        if let Some(picker) = self.picker.take() {
            picker.commit();
        }
        *self.pending_defrag = true;
        true
    }
}

pub(super) fn require_resident_sources(
    sources: &HashMap<ColumnId, RegisteredStreamColumn>,
    series: &SeriesConfig,
) -> Result<()> {
    let mut result = Ok(());
    visit_series_columns(series, &mut |id| {
        if result.is_ok() && sources.contains_key(id) {
            result = Err(FiggyError::ColumnNotResident { id: id.to_owned() });
        }
    });
    result
}

pub(super) trait ColumnRegistration {
    fn column_len(&self, id: &str) -> Option<u64>;

    /// Whether `id` names a resident record (a box plot's `y_column`) or a
    /// resident ragged column (its outliers). Records and ragged columns are
    /// resident only.
    fn contains_composite(&self, id: &str) -> bool;

    fn contains_column(&self, id: &str) -> bool {
        self.column_len(id).is_some()
    }
}

impl ColumnRegistration for ColumnPool {
    /// A record's or ragged column's member columns are not columns a series
    /// can reference: they belong to their record and change id on every
    /// replacement.
    fn column_len(&self, id: &str) -> Option<u64> {
        self.slot(id)
            .filter(|_| !self.is_member_column(id))
            .map(|slot| slot.len_values as u64)
    }

    fn contains_composite(&self, id: &str) -> bool {
        self.record(id).is_some() || self.ragged(id).is_some()
    }
}

impl ColumnRegistration for (&ColumnPool, &HashMap<ColumnId, RegisteredStreamColumn>) {
    fn column_len(&self, id: &str) -> Option<u64> {
        self.1
            .get(id)
            .map(|source| source.len)
            .or_else(|| self.0.column_len(id))
    }

    fn contains_composite(&self, id: &str) -> bool {
        self.0.contains_composite(id)
    }
}

pub(super) fn validate_registered_column(pool: &impl ColumnRegistration, id: &str) -> Result<()> {
    if pool.contains_column(id) {
        Ok(())
    } else {
        Err(FiggyError::UnknownColumn { id: id.to_owned() })
    }
}

pub(super) fn validate_axis_view_state(field: &'static str, axis: &AxisViewState) -> Result<()> {
    if !axis.min.is_finite() || !axis.max.is_finite() {
        return Err(FiggyError::InvalidConfig {
            field,
            reason: "axis bounds must be finite",
        });
    }
    if axis.max <= axis.min {
        return Err(FiggyError::InvalidConfig {
            field,
            reason: "axis max must be greater than min",
        });
    }
    if matches!(axis.scale, AxisScale::Logarithmic) && axis.min <= 0.0 {
        return Err(FiggyError::InvalidConfig {
            field,
            reason: "logarithmic axis bounds must be positive",
        });
    }
    Ok(())
}

pub(super) fn validate_chart_view_state(view: &ChartViewState) -> Result<()> {
    if view.chart_area.width == 0 || view.chart_area.height == 0 {
        return Err(FiggyError::InvalidChartArea {
            width: view.chart_area.width,
            height: view.chart_area.height,
        });
    }
    validate_axis_view_state("top_x", &view.top_x)?;
    validate_axis_view_state("bottom_x", &view.bottom_x)?;
    validate_axis_view_state("left_y", &view.left_y)?;
    validate_axis_view_state("right_y", &view.right_y)?;
    for axis in &view.overlay_y {
        validate_axis_view_state("overlays.y_axis", axis)?;
    }
    for subplot in &view.subplots {
        validate_chart_view_state(subplot)?;
    }
    Ok(())
}

pub(super) fn validate_error_ref_columns(
    pool: &impl ColumnRegistration,
    error: &ErrorRef,
) -> Result<()> {
    match error {
        ErrorRef::Symmetric { column } => validate_registered_column(pool, column),
        ErrorRef::Asymmetric { lower, upper } => {
            validate_registered_column(pool, lower)?;
            validate_registered_column(pool, upper)
        }
    }
}

/// Whether this series reads `id` from the pool.
///
/// Asks [`visit_series_columns`] rather than enumerating the columns a second
/// time. The two used to be hand-kept copies of the same list, which is a live
/// hazard without any new variant at all: give an existing variant another
/// column-bearing field, update one function and not the other, and column
/// removal and validation quietly disagree about what a series needs. One
/// enumeration cannot disagree with itself.
pub(super) fn series_references_column(series: &SeriesConfig, id: &str) -> bool {
    let mut found = false;
    visit_series_columns(series, &mut |column| found |= column == id);
    found
}

pub(super) fn data_series_declarations_equal(
    left: &[SeriesConfig],
    right: &[SeriesConfig],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.series_id == right.series_id
                && left.source_id == right.source_id
                && left.x_column == right.x_column
                && left.y_column == right.y_column
                && left.render_type == right.render_type
        })
}

/// Whether any of `ids` is read by any series in the list.
///
/// Takes an iterator rather than a slice so a caller that only has the ids as
/// part of a larger structure — a batch of `(id, source)` pairs, for one — does
/// not have to collect them into a `Vec` first.
pub(super) fn series_list_references_any_column<'i>(
    series: &[SeriesConfig],
    mut ids: impl Iterator<Item = &'i str>,
) -> bool {
    ids.any(|id| {
        series
            .iter()
            .any(|series| series_references_column(series, id))
    })
}

pub(super) fn try_insert_column_id(
    ids: &mut std::collections::HashSet<ColumnId>,
    id: &str,
    resource: &'static str,
) -> Result<bool> {
    if ids.contains(id) {
        return Ok(false);
    }
    ids.try_reserve(1)
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource,
            reason: error.to_string(),
        })?;
    let mut owned = String::new();
    owned
        .try_reserve_exact(id.len())
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource,
            reason: error.to_string(),
        })?;
    owned.push_str(id);
    Ok(ids.insert(owned))
}

pub(super) fn prepare_load_demo_chart_plan(
    chart_states: &HashMap<ChartId, ChartRenderState>,
    chart_order: &[ChartId],
    visual_revision: RenderRevision,
    target_id: ChartId,
    config: Config,
    series: Vec<SeriesConfig>,
) -> Result<LoadDemoChartPlan> {
    chart_states
        .get(&target_id)
        .ok_or(FiggyError::UnknownChart { id: target_id })?;
    let mut revisions = Vec::new();
    revisions.try_reserve(chart_order.len()).map_err(|error| {
        FiggyError::StateAllocationFailed {
            resource: "load-demo chart revisions",
            reason: error.to_string(),
        }
    })?;
    for id in chart_order {
        let state = chart_states
            .get(id)
            .ok_or(FiggyError::UnknownChart { id: *id })?;
        let is_target = *id == target_id;
        if !is_target
            && !series_list_references_any_column(&state.series, DEMO_COLUMN_IDS.iter().copied())
        {
            continue;
        }

        let ChartRevisions {
            desired,
            config: config_revision,
            series: series_revision,
            data,
            view,
            selection,
            raster,
            annotation,
        } = state.revisions;
        let desired = desired.successor("chart desired revision")?;
        let next = if is_target {
            ChartRevisions {
                desired,
                config: config_revision.successor("chart config revision")?,
                selection: selection_config_revision(selection, &state.config, &config)?,
                series: series_revision.successor("chart series revision")?,
                data: if !data_series_declarations_equal(&state.series, &series)
                    || series_list_references_any_column(
                        &state.series,
                        DEMO_COLUMN_IDS.iter().copied(),
                    )
                    || series_list_references_any_column(&series, DEMO_COLUMN_IDS.iter().copied())
                {
                    data.successor("chart data revision")?
                } else {
                    data
                },
                view: if !stream_config_equal(&state.config, &config, &state.series) {
                    view.successor("chart view revision")?
                } else {
                    view
                },
                raster: raster.successor("chart raster revision")?,
                annotation: annotation_config_revision(annotation, &state.config, &config)?,
            }
        } else {
            // A chart that only reads the demo columns: its data changed.
            ChartRevisions {
                desired,
                config: config_revision,
                selection,
                series: series_revision,
                data: data.successor("chart data revision")?,
                view,
                raster,
                annotation,
            }
        };
        revisions.push(LoadDemoChartRevision {
            id: *id,
            revisions: next,
        });
    }
    debug_assert!(revisions.iter().any(|update| update.id == target_id));
    let visual = visual_revision.successor("renderer visual revision")?;
    Ok(LoadDemoChartPlan {
        target_id,
        config,
        series,
        revisions,
        visual,
    })
}

pub(super) fn visit_error_ref_columns(error: &ErrorRef, visit: &mut impl FnMut(&str)) {
    match error {
        ErrorRef::Symmetric { column } => visit(column),
        ErrorRef::Asymmetric { lower, upper } => {
            visit(lower);
            visit(upper);
        }
    }
}

/// Every pool column this series reads, visited once per occurrence.
///
/// The single enumeration: column removal, validation, pick invalidation, and
/// the derived-data stamps all reach the pool through here, so a column this
/// function does not name is a column the renderer believes nobody uses.
///
/// The tail match exists to make that a compile error rather than a discovery.
/// Every column above is reached through an `extract_*` helper, so a variant
/// whose columns live in a *new* field — the design's matrix bundle, say — would
/// be invisible here and nothing would fail to compile: a chart keeps a series
/// pointing at a removed column, and every later `prepare` fails in `lookup`
/// with `UnknownColumn`, permanently. Listing the variants forces the author of
/// the next one to answer "does it read anything else?".
pub(super) fn visit_series_columns(series: &SeriesConfig, visit: &mut impl FnMut(&str)) {
    visit(&series.x_column);
    // A violin without boxes names no record.
    if !(series.y_column.is_empty() && series.render_type.box_plot().is_some()) {
        visit(&series.y_column);
    }
    if let Some(scatter) = extract_scatter(&series.render_type) {
        if let Some(column) = scatter.point_style_index_column.as_deref() {
            visit(column);
        }
        if let Some(map) = scatter.point_color_map.as_ref() {
            visit(&map.column);
        }
        if let Some(map) = scatter.point_size_map.as_ref() {
            visit(&map.column);
        }
    }
    if let Some(column) = extract_errorbar_style(&series.render_type)
        .and_then(|errorbar| errorbar.error_bar_style_index_column.as_deref())
    {
        visit(column);
    }
    if let Some(error) = extract_err_x(&series.render_type) {
        visit_error_ref_columns(error, visit);
    }
    if let Some(error) = extract_err_y(&series.render_type) {
        visit_error_ref_columns(error, visit);
    }
    match &series.render_type {
        // Nothing beyond what the extractors above already reached.
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        // Category positions and values are its x and y columns.
        | DataRenderType::CategoryBars { .. }
        // Category indices and values too.
        | DataRenderType::Radar { .. } => {}
        // Its first two components are its x and y columns; the third is a
        // referenced column, so replacing it redraws the series and
        // removing it invalidates it.
        DataRenderType::Ternary { c_column, .. } => visit(c_column),
        // The polylines are its x and y columns; the colour column is
        // referenced like a scatter's colour map.
        DataRenderType::Streamlines { color_map, .. } => {
            if let Some(map) = color_map {
                visit(&map.column);
            }
        }
        // Its positions are its x column and its record its y column; the
        // outliers are a referenced ragged column like a matrix' columns, so
        // replacing or removing them invalidates the series.
        // So are a violin's curves and the density and raw point columns.
        DataRenderType::BoxPlot { .. } => {
            if let Some(boxes) = series.render_type.box_plot() {
                for id in boxes.ragged_columns().chain(boxes.plain_columns()) {
                    visit(id);
                }
            }
        }
        // Its positions and end values are its x and y columns; the start,
        // the whiskers, the colour flag or state and the progress are
        // referenced columns, so replacing one redraws the series and
        // removing one invalidates it.
        DataRenderType::RangeBars { .. } => {
            if let Some(bars) = series.render_type.range_bars() {
                for id in bars.extra_columns() {
                    visit(id);
                }
            }
        }
        // A matrix' constituent columns are referenced columns like any other:
        // remove-column cascade, validation, and the derived-identity stamp all
        // reach them through here, so a grid column cannot be dropped from under
        // a live field.
        DataRenderType::Heatmap { matrix, .. }
        | DataRenderType::Contour { matrix, .. }
        | DataRenderType::HeatmapContour { matrix, .. } => {
            for column in &matrix.columns {
                visit(column);
            }
        }
        // The components and the colour-map value are referenced columns in
        // both forms, for the same reasons as a matrix' constituents.
        DataRenderType::Vector { components, style } => {
            match components {
                crate::data_config::VectorComponents::Points { u, v } => {
                    visit(u);
                    visit(v);
                }
                crate::data_config::VectorComponents::Grid { u, v } => {
                    for column in u.columns.iter().chain(&v.columns) {
                        visit(column);
                    }
                }
            }
            match style.color_map.as_ref() {
                Some(crate::data_config::VectorColorMap::Column(column)) => visit(column),
                Some(crate::data_config::VectorColorMap::Grid(matrix)) => {
                    for column in &matrix.columns {
                        visit(column);
                    }
                }
                None => {}
            }
        }
    }
}

/// Validate a series list against the pool and the chart's own `Config`.
///
/// The config is needed because a field series is only fully declared with it:
/// `Config.colorbar` owns the z range and colormap.
pub(super) fn validate_renderer_series(
    pool: &impl ColumnRegistration,
    config: &Config,
    series: &[SeriesConfig],
) -> Result<()> {
    // The category bar rules are the chart's: its bar groups, their axes and
    // every bar series' style, labels and overrides.
    if let Err(reason) = config.validate_category_bars(series) {
        return Err(FiggyError::InvalidSeriesConfig {
            series_id: series
                .iter()
                .find(|s| s.render_type.category_bars().is_some())
                .map_or_else(String::new, |s| s.series_id.clone()),
            reason: reason.to_string(),
        });
    }
    // The box plot rules are the chart's too: its box groups, their axes and
    // every box series' style, labels and overrides.
    if let Err(reason) = config.validate_box_plots(series) {
        return Err(FiggyError::InvalidSeriesConfig {
            series_id: series
                .iter()
                .find(|s| s.render_type.box_plot().is_some())
                .map_or_else(String::new, |s| s.series_id.clone()),
            reason: reason.to_string(),
        });
    }
    // The range bar rules are the chart's too: their groups, their axes,
    // and every range series' columns, style and labels.
    if let Err(reason) = config.validate_range_bars(series) {
        return Err(FiggyError::InvalidSeriesConfig {
            series_id: series
                .iter()
                .find(|s| s.render_type.range_bars().is_some())
                .map_or_else(String::new, |s| s.series_id.clone()),
            reason: reason.to_string(),
        });
    }
    validate_radar_rules(config, series)?;
    // A ternary chart draws only ternary series, and a ternary series needs
    // a ternary chart; its line is linear and unfilled.
    if let Err(reason) = config.validate_ternary_series(series) {
        return Err(FiggyError::InvalidSeriesConfig {
            series_id: series
                .iter()
                .find(|s| s.render_type.ternary_c_column().is_some())
                .or(series.first())
                .map_or_else(String::new, |s| s.series_id.clone()),
            reason: reason.to_string(),
        });
    }
    // The line offset group is the chart's too: its members, their kinds
    // and the fills between them.
    config
        .validate_line_offsets(series)
        .map_err(|reason| FiggyError::InvalidConfig {
            field: "line_offsets",
            reason,
        })?;
    for (index, item) in series.iter().enumerate() {
        if item.series_id.is_empty() {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: item.series_id.clone(),
                reason: "series_id must not be empty".to_string(),
            });
        }
        if series[..index]
            .iter()
            .any(|earlier| earlier.series_id == item.series_id)
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: item.series_id.clone(),
                reason: "series_id must be unique within a chart".to_string(),
            });
        }

        if let Some(Err(e)) = item.label.as_ref().map(|label| label.validate_math()) {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: item.series_id.clone(),
                reason: format!("label: {e}"),
            });
        }
        validate_registered_column(pool, &item.x_column)?;
        if let Some(boxes) = item.render_type.box_plot() {
            // A box plot reads a record and ragged columns, not columns,
            // besides its violin's densities and its raw points.
            let record = (!item.y_column.is_empty()).then_some(item.y_column.as_str());
            for id in record.into_iter().chain(boxes.ragged_columns()) {
                if !pool.contains_composite(id) {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: item.series_id.clone(),
                        reason: format!(
                            "box plot {id:?} is not a resident record or ragged column: upload \
                             the record with `upsert_record` and the outliers and violin \
                             curves with `upsert_ragged`"
                        ),
                    });
                }
            }
            for id in boxes.plain_columns() {
                validate_registered_column(pool, id)?;
            }
        } else {
            validate_registered_column(pool, &item.y_column)?;
        }
        if let Some(bars) = item.render_type.range_bars() {
            for id in bars.extra_columns() {
                validate_registered_column(pool, id)?;
            }
        }
        if let Some(c) = item.render_type.ternary_c_column() {
            validate_registered_column(pool, c)?;
        }
        if let Some(scatter) = extract_scatter(&item.render_type) {
            if let Some(column) = scatter.point_style_index_column.as_deref() {
                validate_registered_column(pool, column)?;
            }
            if let Some(map) = scatter.point_color_map.as_ref() {
                validate_registered_column(pool, &map.column)?;
                if config.colorbar.is_none() {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: item.series_id.clone(),
                        reason: "a colour-mapped scatter needs Config.colorbar: it owns the z \
                                 range and colormap, so without it there is no colour to draw"
                            .to_string(),
                    });
                }
            }
            if let Some(map) = scatter.point_size_map.as_ref() {
                validate_registered_column(pool, &map.column)?;
                map.validate()
                    .map_err(|reason| FiggyError::InvalidSeriesConfig {
                        series_id: item.series_id.clone(),
                        reason: reason.to_string(),
                    })?;
            }
        }
        if let Some(errorbar) = extract_errorbar_style(&item.render_type)
            && let Some(column) = errorbar.error_bar_style_index_column.as_deref()
        {
            validate_registered_column(pool, column)?;
        }
        if let Some(error) = extract_err_x(&item.render_type) {
            validate_error_ref_columns(pool, error)?;
        }
        if let Some(error) = extract_err_y(&item.render_type) {
            validate_error_ref_columns(pool, error)?;
        }
        if requires_colorbar(&item.render_type) && config.colorbar.is_none() {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: item.series_id.clone(),
                reason: "a field series needs Config.colorbar: it owns the z range and \
                         colormap, so without it there is no value to draw"
                    .to_string(),
            });
        }
        validate_contour_level_count(&item.series_id, &item.render_type)?;
        validate_contour_label_config(&item.series_id, &item.render_type)?;
        validate_line_fill(pool, series.iter(), item)?;
        validate_polar_series(config, item)?;
        if let Some(matrix) = extract_matrix(&item.render_type) {
            // A grid column that is not registered would read whatever else
            // happens to be at that offset. Count mismatches between the
            // declaration and the data are *not* checked here — those are drawn
            // to the smallest common extent and reported, never rejected.
            for column in &matrix.columns {
                validate_registered_column(pool, column)?;
            }
        }
        if let Some((components, style)) = extract_vector(&item.render_type) {
            validate_vector_series(pool, config, item, components, style)?;
        }
        validate_streamline_series(pool, config, item)?;
    }
    Ok(())
}

/// A streamline series' rules: its own (`data_config::validate_streamline_series`:
/// a linear line without a fill and valid arrows), a registered colour
/// column, and the colourbar a colour map maps through.
pub(super) fn validate_streamline_series(
    pool: &impl ColumnRegistration,
    config: &Config,
    item: &SeriesConfig,
) -> Result<()> {
    let Some(streamlines) = item.render_type.streamlines() else {
        return Ok(());
    };
    let invalid = |reason: &str| FiggyError::InvalidSeriesConfig {
        series_id: item.series_id.clone(),
        reason: reason.to_owned(),
    };
    crate::data_config::validate_streamline_series(item).map_err(invalid)?;
    if let Some(map) = streamlines.color_map {
        validate_registered_column(pool, &map.column)?;
        if config.colorbar.is_none() {
            return Err(invalid(
                "colour-mapped streamlines need Config.colorbar: it owns the z range and \
                 colormap, so without it there is no colour to draw",
            ));
        }
    }
    Ok(())
}

/// The polar and ternary coordinate systems of the chart against its own
/// axes.
pub(super) fn validate_polar_config(config: &Config) -> Result<()> {
    config
        .validate_polar()
        .map_err(|reason| FiggyError::InvalidConfig {
            field: "polar",
            reason,
        })?;
    config
        .validate_ternary()
        .map_err(|reason| FiggyError::InvalidConfig {
            field: "ternary",
            reason,
        })
}

/// The series a polar chart draws: scatter markers, lines (with fills) and
/// radar polygons. Histograms, fields, contours, error bars, vectors,
/// category bars, box plots and streamlines have no polar form and are rejected rather
/// than drawn on the wrong axes; an arc interpolation needs a linearly
/// interpolated line.
/// The radar rules of a chart, at registration and at export: a polar chart
/// with a categorical angle axis, chord edges, any number of series (#107)
/// and every series' style and labels (`Config::validate_radar`).
pub(super) fn validate_radar_rules(config: &Config, series: &[SeriesConfig]) -> Result<()> {
    config
        .validate_radar(series)
        .map_err(|reason| FiggyError::InvalidSeriesConfig {
            series_id: series
                .iter()
                .find(|s| s.render_type.radar().is_some())
                .map_or_else(String::new, |s| s.series_id.clone()),
            reason: reason.to_string(),
        })
}

pub(super) fn validate_polar_series(config: &Config, item: &SeriesConfig) -> Result<()> {
    let Some(polar) = &config.polar else {
        return Ok(());
    };
    let invalid = |reason: &str| FiggyError::InvalidSeriesConfig {
        series_id: item.series_id.clone(),
        reason: reason.to_owned(),
    };
    match &item.render_type {
        DataRenderType::Scatter { .. }
        | DataRenderType::Line { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::Radar { .. } => {}
        DataRenderType::Histogram { .. } => {
            return Err(invalid("histogram series are not drawn on polar axes"));
        }
        DataRenderType::Heatmap { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Contour { .. } => {
            return Err(invalid(
                "field and contour series are not drawn on polar axes",
            ));
        }
        DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. } => {
            return Err(invalid("error bars are not drawn on polar axes"));
        }
        DataRenderType::Vector { .. } => {
            return Err(invalid("vector series are not drawn on polar axes"));
        }
        DataRenderType::CategoryBars { .. } => {
            return Err(invalid("category bars are not drawn on polar axes"));
        }
        DataRenderType::BoxPlot { .. } => {
            return Err(invalid("box plots are not drawn on polar axes"));
        }
        DataRenderType::RangeBars { .. } => {
            return Err(invalid("range bars are not drawn on polar axes"));
        }
        DataRenderType::Streamlines { .. } => {
            return Err(invalid("streamlines are not drawn on polar axes"));
        }
        DataRenderType::Ternary { .. } => {
            return Err(invalid(
                "ternary series are drawn on ternary charts, not polar axes",
            ));
        }
    }
    if let (Some(line), crate::polar::PolarInterpolation::Arc { .. }) =
        (extract_line(&item.render_type), polar.interpolation)
        && !line.interpolation.is_linear()
    {
        return Err(invalid(
            "a polar arc interpolation needs a linearly interpolated line; a step \
             interpolation draws chords",
        ));
    }
    Ok(())
}

/// A vector series' own rules: a valid style, registered component columns,
/// grids that agree on orientation and layout, a colour map in the same form
/// as the components, and the colourbar it maps through.
pub(super) fn validate_vector_series(
    pool: &impl ColumnRegistration,
    config: &Config,
    item: &SeriesConfig,
    components: &crate::data_config::VectorComponents,
    style: &crate::data_config::VectorStyle,
) -> Result<()> {
    use crate::data_config::{VectorColorMap, VectorComponents};
    let invalid = |reason: String| FiggyError::InvalidSeriesConfig {
        series_id: item.series_id.clone(),
        reason,
    };
    style.validate().map_err(|reason| invalid(reason.into()))?;
    let grid = match components {
        VectorComponents::Points { u, v } => {
            validate_registered_column(pool, u)?;
            validate_registered_column(pool, v)?;
            None
        }
        VectorComponents::Grid { u, v } => {
            for column in u.columns.iter().chain(&v.columns) {
                validate_registered_column(pool, column)?;
            }
            if u.orientation != v.orientation || u.grid_layout != v.grid_layout {
                return Err(invalid(
                    "vector u and v grids must share orientation and grid layout".into(),
                ));
            }
            Some(u)
        }
    };
    match (style.color_map.as_ref(), grid) {
        (None, _) => {}
        (Some(VectorColorMap::Column(column)), None) => {
            validate_registered_column(pool, column)?;
        }
        (Some(VectorColorMap::Grid(matrix)), Some(u)) => {
            for column in &matrix.columns {
                validate_registered_column(pool, column)?;
            }
            if matrix.orientation != u.orientation || matrix.grid_layout != u.grid_layout {
                return Err(invalid(
                    "vector colour grid must share the u grid's orientation and grid layout".into(),
                ));
            }
        }
        (Some(_), _) => {
            return Err(invalid(
                "vector colour map must take the components' form: a column for points, \
                 a grid for grids"
                    .into(),
            ));
        }
    }
    if style.color_map.is_some() && config.colorbar.is_none() {
        return Err(invalid(
            "a colour-mapped vector needs Config.colorbar: it owns the z range and colormap, \
             so without it there is no colour to draw"
                .into(),
        ));
    }
    Ok(())
}

/// A line fill's declaration must resolve before any GPU work: a finite
/// baseline, or a referenced series that exists in the same chart, is not the
/// series itself, shares its x column and has a registered y column. A
/// different x column would need resampling — renderer-derived data — so it
/// is an error, not a fallback.
pub(super) fn validate_line_fill<'s>(
    pool: &impl ColumnRegistration,
    series: impl IntoIterator<Item = &'s SeriesConfig>,
    item: &SeriesConfig,
) -> Result<()> {
    let Some(line) = extract_line(&item.render_type) else {
        return Ok(());
    };
    let invalid = |reason: String| FiggyError::InvalidSeriesConfig {
        series_id: item.series_id.clone(),
        reason,
    };
    match line.fill.as_ref() {
        None => {}
        Some(LineFill::ToBaseline(baseline)) => {
            if !baseline.is_finite() {
                return Err(invalid("line fill baseline must be finite".into()));
            }
        }
        Some(LineFill::ToSeries(target)) => {
            if *target == item.series_id {
                return Err(invalid(
                    "line fill cannot reference the series itself".into(),
                ));
            }
            let target = series
                .into_iter()
                .find(|other| other.series_id == *target)
                .ok_or_else(|| {
                    invalid(format!(
                        "line fill references series `{target}`, which is not in this chart"
                    ))
                })?;
            if target.x_column != item.x_column {
                return Err(invalid(format!(
                    "line fill to series `{}` requires the same x column: `{}` vs `{}`",
                    target.series_id, item.x_column, target.x_column
                )));
            }
            validate_registered_column(pool, &target.y_column)?;
        }
    }
    Ok(())
}

pub(super) fn build_web_derived_stamp_from_state(
    pool: &ColumnPool,
    renderer_identity: u64,
    chart_id: ChartId,
    state: &ChartRenderState,
    data_revision: RenderRevision,
) -> Result<WebDerivedStamp> {
    let mut ids: Vec<String> = Vec::new();
    ids.try_reserve(state.series.len().saturating_mul(8))
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "derived column identities",
            reason: error.to_string(),
        })?;
    for series in &state.series {
        visit_series_columns(series, &mut |column| {
            if !ids.iter().any(|existing| existing == column) {
                ids.push(column.to_string());
            }
        });
    }

    let mut columns = Vec::new();
    columns
        .try_reserve(ids.len())
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "derived column stamps",
            reason: error.to_string(),
        })?;
    for column in ids {
        let allocation_epoch = pool
            .allocation_epoch(&column)
            .ok_or_else(|| FiggyError::UnknownColumn { id: column.clone() })?;
        columns.push(ColumnContentStamp {
            id: column,
            allocation_epoch,
        });
    }

    Ok(WebDerivedStamp {
        renderer_identity,
        chart_id,
        view: state.revisions.view,
        data: data_revision,
        pool_layout_generation: pool.layout_generation(),
        columns,
    })
}

pub(super) fn build_web_derived_snapshot_from_state(
    pool: &ColumnPool,
    streaming_sources: &HashMap<ColumnId, RegisteredStreamColumn>,
    renderer_identity: u64,
    chart_id: ChartId,
    state: &ChartRenderState,
    data_revision: RenderRevision,
) -> Result<WebDerivedSnapshot> {
    let stamp = build_web_derived_stamp_from_state(
        pool,
        renderer_identity,
        chart_id,
        state,
        data_revision,
    )?;
    let fit_token =
        build_fit_token_from_state(pool, streaming_sources, renderer_identity, chart_id, state)?;
    let mut series = Vec::new();
    series
        .try_reserve(state.series.len())
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "web derived series snapshot",
            reason: error.to_string(),
        })?;
    series.extend(state.series.iter().cloned());
    Ok(WebDerivedSnapshot {
        stamp,
        fit_token,
        config: state.config.clone(),
        series,
    })
}

pub(super) const DEMO_COLUMN_IDS: [&str; 4] = ["demo_x", "demo_sin", "demo_t", "demo_rc"];

impl Renderer {
    /// Check whether an exact resident working set fits every declared static
    /// resource constraint without allocating or mutating renderer state.
    ///
    /// The check includes ALIGN-padded column storage, upload staging, a pool
    /// compaction/growth candidate, persistent derived bytes, caller-declared
    /// transition headroom, current live resources, and resources retired but
    /// not yet released by the GPU. It also applies the current pool policies,
    /// the device's combined buffer/storage-binding ceiling, the working-set
    /// cap, and [`Self::memory_budget`].
    ///
    /// Without a host memory budget the result is fail-closed as
    /// [`ResidentAdmissionStatus::MemoryBudgetUnset`]: WebGPU exposes device
    /// limits but not free VRAM. An admissible result is not a reservation, so
    /// the eventual atomic transition must repeat allocation-time checks and
    /// remain failure-atomic.
    pub fn resident_admission(&self, request: ResidentAdmissionRequest<'_>) -> ResidentAdmission {
        use crate::data::COLUMN_VALUE_BYTES;
        use crate::data_render::column_pool::{ALIGN, ResidentBatchCapacityError};

        let usage = self.gpu_memory_usage();
        let Some(current_gpu_bytes) = usage.checked_total_bytes() else {
            return ResidentAdmission {
                status: ResidentAdmissionStatus::ArithmeticOverflow,
                resident_column_bytes: 0,
                resident_working_set_bytes: 0,
                derived_resident_bytes: 0,
                upload_staging_bytes: 0,
                pool_capacity_before: self.pool.capacity(),
                pool_capacity_after: self.pool.capacity(),
                pool_transition_bytes: 0,
                pool_free_bytes: self.pool.free_bytes(),
                pool_largest_free_bytes: self.pool.largest_free_region(),
                current_gpu_bytes: u64::MAX,
                transition_peak_bytes: u64::MAX,
                working_set_limit_bytes: request.working_set_limit_bytes,
                memory_budget_bytes: self.memory_budget,
                device_buffer_limit_bytes: self
                    .caps
                    .max_buffer_size
                    .min(self.caps.max_storage_buffer_binding_size),
            };
        };
        let device_buffer_limit_bytes = self
            .caps
            .max_buffer_size
            .min(self.caps.max_storage_buffer_binding_size);
        let mut report = ResidentAdmission {
            status: ResidentAdmissionStatus::ArithmeticOverflow,
            resident_column_bytes: 0,
            resident_working_set_bytes: 0,
            derived_resident_bytes: 0,
            upload_staging_bytes: 0,
            pool_capacity_before: self.pool.capacity(),
            pool_capacity_after: self.pool.capacity(),
            pool_transition_bytes: 0,
            pool_free_bytes: self.pool.free_bytes(),
            pool_largest_free_bytes: self.pool.largest_free_region(),
            current_gpu_bytes,
            transition_peak_bytes: current_gpu_bytes,
            working_set_limit_bytes: request.working_set_limit_bytes,
            memory_budget_bytes: self.memory_budget,
            device_buffer_limit_bytes,
        };

        for (column, &length) in request.column_value_counts.iter().enumerate() {
            if length > u64::from(u32::MAX) || usize::try_from(length).is_err() {
                report.status =
                    ResidentAdmissionStatus::ColumnLengthLimitExceeded { column, length };
                return report;
            }
            if length == 0 {
                continue;
            }
            let Some(raw_bytes) = length.checked_mul(COLUMN_VALUE_BYTES as u64) else {
                return report;
            };
            let Some(region_bytes) = raw_bytes
                .checked_add(ALIGN - 1)
                .map(|bytes| bytes & !(ALIGN - 1))
            else {
                return report;
            };
            if region_bytes > device_buffer_limit_bytes {
                report.status = ResidentAdmissionStatus::DeviceBufferLimitExceeded;
                return report;
            }
            let Some(total) = report.resident_column_bytes.checked_add(region_bytes) else {
                return report;
            };
            report.resident_column_bytes = total;
        }
        report.upload_staging_bytes = report.resident_column_bytes;
        for (buffer, &bytes) in request.derived_buffer_sizes.iter().enumerate() {
            if bytes > device_buffer_limit_bytes {
                report.status =
                    ResidentAdmissionStatus::DerivedBufferLimitExceeded { buffer, bytes };
                return report;
            }
            let Some(total) = report.derived_resident_bytes.checked_add(bytes) else {
                return report;
            };
            report.derived_resident_bytes = total;
        }
        let Some(working_set_bytes) = report
            .resident_column_bytes
            .checked_add(report.derived_resident_bytes)
            .and_then(|bytes| bytes.checked_add(request.already_resident_working_set_bytes))
        else {
            return report;
        };
        report.resident_working_set_bytes = working_set_bytes;
        if working_set_bytes > request.working_set_limit_bytes {
            report.status = ResidentAdmissionStatus::WorkingSetLimitExceeded;
            return report;
        }

        let pool_plan = match self
            .pool
            .plan_resident_batch_capacity(report.resident_column_bytes, device_buffer_limit_bytes)
        {
            Ok(plan) => plan,
            Err(ResidentBatchCapacityError::Overflow) => return report,
            Err(ResidentBatchCapacityError::DeviceLimit) => {
                report.status = ResidentAdmissionStatus::DeviceBufferLimitExceeded;
                return report;
            }
            Err(ResidentBatchCapacityError::Policy) => {
                report.status = ResidentAdmissionStatus::PoolCapacityUnavailable;
                return report;
            }
        };
        report.pool_capacity_after = pool_plan.capacity_after;
        report.pool_transition_bytes = pool_plan.transition_bytes;

        let Some(transition_peak_bytes) = current_gpu_bytes
            .checked_add(pool_plan.transition_bytes)
            .and_then(|bytes| bytes.checked_add(report.upload_staging_bytes))
            .and_then(|bytes| bytes.checked_add(report.derived_resident_bytes))
            .and_then(|bytes| bytes.checked_add(request.transition_headroom_bytes))
        else {
            return report;
        };
        report.transition_peak_bytes = transition_peak_bytes;
        let Some(memory_budget) = self.memory_budget else {
            report.status = ResidentAdmissionStatus::MemoryBudgetUnset;
            return report;
        };
        report.status = if transition_peak_bytes > memory_budget {
            ResidentAdmissionStatus::MemoryBudgetExceeded
        } else {
            ResidentAdmissionStatus::Admissible
        };
        report
    }

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
                reason: "no series with this id in the chart".to_string(),
            })?;
        Ok(series_draw_info_from_pool(
            &(&self.pool, &self.streaming_sources),
            series,
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
    pub fn register_chart(
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
        let previous = self.chart_states.insert(
            id,
            ChartRenderState {
                config,
                series,
                prepared_styles: None,
                selected: None,
                stream_auto_fit_padding: None,
                revisions: ChartRevisions::initial(chart_revision),
            },
        );
        debug_assert!(previous.is_none());
        self.chart_order.push(id);
        self.next_chart_id = id_value;
        self.visual_revision = visual_revision;
        Ok(id)
    }

    pub fn remove_chart(&mut self, id: ChartId) -> Result<()> {
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

    pub fn set_chart_config(&mut self, id: ChartId, mut config: Config) -> Result<()> {
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

        let chart_state = self
            .chart_states
            .get_mut(&id)
            .expect("validated chart remains registered during config preparation");
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
        Ok(self.chart_config(id)?.active_layer())
    }

    /// A series an overlay layer scales or a subplot draws must be resident:
    /// the nonresident stream pass draws one panel, so a streamed series on
    /// another panel is an explicit error rather than a silently mis-scaled
    /// drawing. A line offset member is new and not streamed at all (rule
    /// 8): a nonresident member is an explicit error too.
    pub(super) fn require_resident_overlay_series(
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
                    reason: "a line offset member must be resident: streamed sources are not \
                             drawn with line offsets"
                        .into(),
                });
            }
            let layer = config.layer_of_series(&item.series_id);
            if !matches!(layer, crate::LayerRef::Base)
                && self.require_resident_series(item).is_err()
            {
                let reason = if matches!(layer, crate::LayerRef::Overlay(_)) {
                    "a nonresident streaming series cannot be scaled by an overlay layer"
                } else {
                    "a nonresident streaming series cannot be drawn by a subplot"
                };
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: item.series_id.clone(),
                    reason: reason.into(),
                });
            }
        }
        Ok(())
    }

    pub fn set_chart_series(&mut self, id: ChartId, series: Vec<SeriesConfig>) -> Result<()> {
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

        let chart_state = self
            .chart_states
            .get_mut(&id)
            .expect("validated chart remains registered during series preparation");
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

    /// Atomically replace the renderer-owned config and ordered series.
    ///
    /// This is the mutation boundary for hosts whose one logical edit affects
    /// both declarations, such as an auto-managed legend update. Validation
    /// and every revision successor complete before either value is replaced.
    pub fn set_chart_state(
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

        let chart_state = self
            .chart_states
            .get_mut(&id)
            .expect("validated chart remains registered during state preparation");
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

    pub fn set_chart_view_state(&mut self, id: ChartId, view: ChartViewState) -> Result<()> {
        validate_chart_view_state(&view)?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        if view.overlay_y.len() != state.config.overlays.len() {
            return Err(FiggyError::InvalidConfig {
                field: "overlays",
                reason: "a view state must carry one Y range per overlay layer",
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
                })
        {
            return Err(FiggyError::InvalidConfig {
                field: "subplots",
                reason: "a view state must carry one view per subplot, each with one Y range per overlay layer",
            });
        }
        let mut prospective_config = state.config.clone();
        view.apply_to(&mut prospective_config);
        prospective_config.sync_subplot_links(Some(&state.config));
        crate::chart::validate_renderer_config(&prospective_config)?;
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
        let generation = crate::text_render::font_generation();
        if generation == self.observed_font_generation {
            return Ok(false);
        }
        if self.chart_order.is_empty() {
            self.observed_font_generation = generation;
            return Ok(false);
        }

        let visual = self.visual_revision.successor("renderer visual revision")?;
        let mut updates = Vec::new();
        updates
            .try_reserve(self.chart_order.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "font invalidation revisions",
                reason: error.to_string(),
            })?;
        for id in &self.chart_order {
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

    pub fn web_derived_snapshot(&self, id: ChartId) -> Result<WebDerivedSnapshot> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        for series in &state.series {
            self.require_resident_series(series)?;
        }
        build_web_derived_snapshot_from_state(
            &self.pool,
            &self.streaming_sources,
            self.renderer_identity,
            id,
            state,
            state.revisions.data,
        )
    }

    pub(super) fn prepare_column_invalidation(&self, id: &str) -> Result<ColumnInvalidationPlan> {
        self.prepare_columns_invalidation(std::iter::once(id))
    }

    /// The invalidation plan for a mutation that touches several columns at
    /// once. One pass over the charts, whatever the batch size.
    pub(super) fn prepare_columns_invalidation<'i>(
        &self,
        ids: impl Iterator<Item = &'i str> + Clone,
    ) -> Result<ColumnInvalidationPlan> {
        let mut charts = Vec::new();
        charts
            .try_reserve(self.chart_order.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "column invalidation revisions",
                reason: error.to_string(),
            })?;
        for chart_id in &self.chart_order {
            let state = self
                .chart_states
                .get(chart_id)
                .ok_or(FiggyError::UnknownChart { id: *chart_id })?;
            if series_list_references_any_column(&state.series, ids.clone()) {
                charts.push(ChartColumnInvalidation {
                    id: *chart_id,
                    desired: state
                        .revisions
                        .desired
                        .successor("chart desired revision")?,
                    data: state.revisions.data.successor("chart data revision")?,
                });
            }
        }
        let visual = if charts.is_empty() {
            None
        } else {
            Some(self.visual_revision.successor("renderer visual revision")?)
        };
        Ok(ColumnInvalidationPlan { charts, visual })
    }

    pub(super) fn prepare_column_removal(
        &self,
        id: &str,
        config_override: Option<(ChartId, Config)>,
    ) -> Result<ColumnRemovalPlan> {
        let mut charts = Vec::new();
        charts
            .try_reserve(self.chart_order.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "column removal candidates",
                reason: error.to_string(),
            })?;
        let mut config_override = config_override;
        for chart_id in &self.chart_order {
            let state = self
                .chart_states
                .get(chart_id)
                .ok_or(FiggyError::UnknownChart { id: *chart_id })?;
            let remove_referencing_series = state
                .series
                .iter()
                .any(|series| series_references_column(series, id));
            let replace_config = config_override
                .as_ref()
                .is_some_and(|(override_chart, _)| override_chart == chart_id);
            if !remove_referencing_series && !replace_config {
                continue;
            }

            let desired = state
                .revisions
                .desired
                .successor("chart desired revision")?;
            let series_revision = remove_referencing_series
                .then(|| state.revisions.series.successor("chart series revision"))
                .transpose()?;
            let data = remove_referencing_series
                .then(|| state.revisions.data.successor("chart data revision"))
                .transpose()?;
            let config_revision = replace_config
                .then(|| state.revisions.config.successor("chart config revision"))
                .transpose()?;
            let selection_revision = config_override
                .as_ref()
                .filter(|(override_chart, _)| override_chart == chart_id)
                .map(|(_, config)| {
                    selection_config_revision(state.revisions.selection, &state.config, config)
                })
                .transpose()?;
            let annotation_revision = config_override
                .as_ref()
                .filter(|(override_chart, _)| override_chart == chart_id)
                .map(|(_, config)| {
                    annotation_config_revision(state.revisions.annotation, &state.config, config)
                })
                .transpose()?;
            let view = config_override
                .as_ref()
                .filter(|(override_chart, _)| override_chart == chart_id)
                .and_then(|(_, config)| {
                    (!stream_config_equal(&state.config, config, &state.series))
                        .then(|| state.revisions.view.successor("chart view revision"))
                })
                .transpose()?;
            let raster = state.revisions.raster.successor("chart raster revision")?;
            charts.push(ChartSeriesRemoval {
                id: *chart_id,
                remove_referencing_series,
                next_config: replace_config.then(|| {
                    config_override
                        .take()
                        .expect("matching chart config override remains available")
                        .1
                }),
                desired,
                config_revision,
                selection_revision,
                annotation_revision,
                series_revision,
                data,
                view,
                raster,
            });
        }
        debug_assert!(config_override.is_none());
        let visual = if charts.is_empty() {
            None
        } else {
            Some(self.visual_revision.successor("renderer visual revision")?)
        };
        Ok(ColumnRemovalPlan { charts, visual })
    }

    pub(super) fn remove_column_impl(
        &mut self,
        id: &str,
        config_override: Option<(ChartId, Config)>,
    ) -> Result<bool> {
        if self.pool.slot(id).is_none() && !self.streaming_sources.contains_key(id) {
            return Ok(false);
        }
        if self.pool.is_member_column(id) {
            return Err(FiggyError::ReservedColumnId { id: id.to_owned() });
        }
        if let Some((chart_id, config)) = config_override.as_ref() {
            crate::chart::validate_renderer_config(config)?;
            if !self.chart_states.contains_key(chart_id) {
                return Err(FiggyError::UnknownChart { id: *chart_id });
            }
        }

        let active_plan = match self.picker.active.as_ref() {
            Some(active) => {
                let state =
                    self.chart_states
                        .get(&active.chart_id)
                        .ok_or(FiggyError::UnknownChart {
                            id: active.chart_id,
                        })?;
                let removes_series = state
                    .series
                    .iter()
                    .any(|series| series_references_column(series, id));
                let override_config = config_override
                    .as_ref()
                    .filter(|(chart_id, _)| *chart_id == active.chart_id)
                    .map(|(_, config)| config);
                if removes_series || override_config.is_some() {
                    let candidate = removes_series
                        .then(|| series_after_column_removal(&state.series, id))
                        .transpose()?;
                    Some((
                        active.chart_id,
                        PickChartPlan::for_active_layer(
                            override_config.unwrap_or(&state.config),
                            candidate.as_deref().unwrap_or(&state.series),
                        )?,
                    ))
                } else {
                    None
                }
            }
            None => None,
        };
        let removed_column = id.to_string();
        let removal = self.prepare_column_removal(id, config_override)?;
        if self.streaming_sources.contains_key(id) {
            let picker = if let Some((chart_id, plan)) = active_plan {
                self.picker
                    .prepare_active_pool_mutation(&self.pool, chart_id, plan, &[])?
            } else {
                None
            };
            self.streaming_sources.remove(id);
            self.stream_revision_lineage.remove(id);
            removal.publish(
                &mut self.chart_states,
                &mut self.visual_revision,
                &removed_column,
            );
            if let Some(picker) = picker {
                picker.commit();
            }
            return Ok(true);
        }
        let Some(inner) = self.pool.begin_remove_column(id)? else {
            return Ok(false);
        };
        let picker = if let Some((chart_id, plan)) = active_plan {
            self.picker
                .prepare_active_pool_mutation(inner.pool(), chart_id, plan, &[])?
        } else {
            None
        };
        let removed = PreparedColumnRemoval {
            inner: Some(inner),
            chart_states: &mut self.chart_states,
            visual_revision: &mut self.visual_revision,
            pending_defrag: &mut self.pending_defrag,
            removal: Some(removal),
            picker,
            removed_column,
        }
        .commit();
        if removed {
            self.stream_revision_lineage.remove(id);
        }
        Ok(removed)
    }

    /// Look up either logical host metadata or the authoritative resident slot.
    pub fn logical_column(&self, id: &str) -> Option<crate::LogicalColumn<'_>> {
        self.streaming_sources
            .get(id)
            .map(|source| crate::LogicalColumn::Streamed(&source.column))
            .or_else(|| self.pool.slot(id).map(crate::LogicalColumn::Resident))
    }

    /// Prepare the renderer-owned portion of the bundled demo as one commit.
    ///
    /// `prepare_state` receives the complete candidate pool so the final axis
    /// ranges can be derived without publishing columns or submitting extent
    /// work. Dropping the returned guard leaves all renderer authority intact.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_load_demo<F>(
        &mut self,
        chart_id: ChartId,
        demo_x: &dyn ColumnSource,
        demo_sin: &dyn ColumnSource,
        demo_t: &dyn ColumnSource,
        demo_rc: &dyn ColumnSource,
        prepare_state: F,
    ) -> Result<RendererLoadDemo<'_>>
    where
        F: FnOnce(&ColumnPool) -> Result<(Config, Vec<SeriesConfig>)>,
    {
        for id in DEMO_COLUMN_IDS {
            self.reject_streamed_id(id)?;
        }
        let Renderer {
            device,
            queue,
            pool,
            chart_states,
            chart_order,
            visual_revision,
            pending_defrag,
            picker,
            gpu_ledger,
            memory_budget,
            ..
        } = self;
        let memory_budget = *memory_budget;
        let inner = pool.begin_demo_batch_upsert(
            [
                (DEMO_COLUMN_IDS[0].to_string(), demo_x),
                (DEMO_COLUMN_IDS[1].to_string(), demo_sin),
                (DEMO_COLUMN_IDS[2].to_string(), demo_t),
                (DEMO_COLUMN_IDS[3].to_string(), demo_rc),
            ],
            pool_alloc_ctx(
                device.as_ref(),
                queue.as_ref(),
                memory_budget,
                gpu_ledger.as_ref(),
            ),
        )?;
        let (config, series) = prepare_state(inner.pool())?;
        crate::chart::validate_renderer_config(&config)?;
        validate_renderer_series(inner.pool(), &config, &series)?;
        let chart_plan = prepare_load_demo_chart_plan(
            chart_states,
            chart_order,
            *visual_revision,
            chart_id,
            config,
            series,
        )?;

        let picker = match picker.active.as_ref().map(|active| active.chart_id) {
            Some(active_id) => {
                let plan = if active_id == chart_id {
                    PickChartPlan::for_active_layer(&chart_plan.config, &chart_plan.series)?
                } else {
                    let state = chart_states
                        .get(&active_id)
                        .ok_or(FiggyError::UnknownChart { id: active_id })?;
                    PickChartPlan::for_active_layer(&state.config, &state.series)?
                };
                picker.prepare_active_pool_mutation(
                    inner.pool(),
                    active_id,
                    plan,
                    &DEMO_COLUMN_IDS,
                )?
            }
            None => None,
        };

        Ok(RendererLoadDemo {
            inner: Some(inner),
            chart_states,
            visual_revision,
            pending_defrag,
            chart_plan: Some(chart_plan),
            picker,
        })
    }

    /// Add a column to the pool. `AllocError` is converted to `FiggyError::Pool`.
    pub fn add_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        let id = id.into();
        if self.pool.slot(&id).is_some() {
            return Err(data_render::AllocError::DuplicateId(id).into());
        }
        Ok(self.begin_upsert_column(id, source)?.commit())
    }

    pub fn add_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        let id = id.into();
        if self.pool.slot(&id).is_some() {
            return Err(data_render::AllocError::DuplicateId(id).into());
        }
        Ok(self.begin_upsert_hilo_column(id, source)?.commit())
    }

    /// Upload many **new** columns in one transaction: one pool region, one
    /// staging buffer, one copy, one submit — instead of one of each per column.
    ///
    /// This is the path a matrix declaration takes. At the thousands of columns
    /// a grid has, per-column [`Self::add_column`] spends the whole upload on
    /// first-fit searches, staging buffers and `queue.submit` calls. Nothing
    /// about the batch is matrix-specific, so any host with many columns gets it.
    ///
    /// Every id must be new to the pool and distinct within the batch —
    /// `DuplicateId` otherwise, with nothing uploaded. An empty slice is a
    /// no-op. No handles come back: a batch may relocate the pool, so callers
    /// re-acquire them through [`Self::handle_for`] exactly as after any other
    /// invalidating mutation.
    ///
    /// Failure-atomic end to end. The pool guard holds the columns provisionally
    /// while the chart revisions and the picker are prepared against them; if
    /// any of that fails the guard is dropped and no column is published.
    ///
    /// One thing a failure does not undo: a compaction or growth that had to
    /// happen first to make one contiguous region. That relayout is not part of
    /// the batch — the pool is simply laid out differently with the same columns
    /// in it — so handles and prepared frames are re-acquired exactly as after a
    /// [`Self::defragment`], and an active picker reports `StaleColumn` until it
    /// is prepared again.
    pub fn add_columns(&mut self, columns: &[(&str, &dyn ColumnSource)]) -> Result<()> {
        for (id, _) in columns {
            self.reject_streamed_id(id)?;
        }
        if columns.is_empty() {
            return Ok(());
        }
        let invalidation = self.prepare_columns_invalidation(columns.iter().map(|(id, _)| *id))?;
        let old_layout_generation = self.pool.layout_generation();
        let ctx = pool_alloc_ctx(
            self.device.as_ref(),
            self.queue.as_ref(),
            self.memory_budget,
            self.gpu_ledger.as_ref(),
        );
        let batch = self.pool.begin_add_columns(columns, ctx)?;
        let relocated = batch.pool().layout_generation() != old_layout_generation;
        // No changed-column list: every id here is new to the pool, and
        // `validate_renderer_series` refuses a series that reads a column the
        // pool does not have (test:
        // `a_matrix_column_must_be_registered_and_is_a_referenced_column`), so
        // no picker slot can already be reading one of them. Relocation is the
        // only reason a batch insert can force a rebuild — and it needs no list.
        let picker = prepare_picker_for_pool_mutation(
            &mut self.picker,
            batch.pool(),
            &self.chart_states,
            relocated,
            &[],
        )?;

        batch.commit();
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        if relocated {
            // The batch relaid the pool out, so the maintenance defrag the
            // upsert path would have asked for is already paid for.
            self.pending_defrag = false;
        }
        Ok(())
    }

    /// Begin a failure-atomic scalar insert or same-id replacement.
    pub fn begin_upsert_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        let id = id.into();
        self.reject_streamed_id(&id)?;
        let invalidation = self.prepare_column_invalidation(&id)?;
        let old_layout_generation = self.pool.layout_generation();
        let inner = self.pool.begin_upsert_column(
            id.clone(),
            source,
            pool_alloc_ctx(
                self.device.as_ref(),
                self.queue.as_ref(),
                self.memory_budget,
                self.gpu_ledger.as_ref(),
            ),
        )?;
        finish_renderer_column_upsert(
            inner,
            &self.streaming_sources,
            &id,
            old_layout_generation,
            &mut self.chart_states,
            &mut self.visual_revision,
            &mut self.pending_defrag,
            &mut self.picker,
            Some(invalidation),
            self.renderer_identity,
            self.device.as_ref(),
            self.queue.as_ref(),
            self.errorbar_extent_engine.as_ref(),
        )
    }

    /// Begin a failure-atomic hi/lo insert or same-id replacement.
    pub fn begin_upsert_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        let id = id.into();
        self.reject_streamed_id(&id)?;
        let invalidation = self.prepare_column_invalidation(&id)?;
        let old_layout_generation = self.pool.layout_generation();
        let inner = self.pool.begin_upsert_hilo_column(
            id.clone(),
            source,
            pool_alloc_ctx(
                self.device.as_ref(),
                self.queue.as_ref(),
                self.memory_budget,
                self.gpu_ledger.as_ref(),
            ),
        )?;
        finish_renderer_column_upsert(
            inner,
            &self.streaming_sources,
            &id,
            old_layout_generation,
            &mut self.chart_states,
            &mut self.visual_revision,
            &mut self.pending_defrag,
            &mut self.picker,
            Some(invalidation),
            self.renderer_identity,
            self.device.as_ref(),
            self.queue.as_ref(),
            self.errorbar_extent_engine.as_ref(),
        )
    }

    /// Failure-atomic scalar upsert without an intermediate batch-preparation
    /// step. Hosts that must prepare additional derived state against the
    /// provisional pool can use [`Self::begin_upsert_column`] and commit its
    /// guard explicitly.
    pub fn upsert_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        Ok(self.begin_upsert_column(id, source)?.commit())
    }

    /// Failure-atomic hi/lo upsert without an intermediate batch-preparation
    /// step.
    pub fn upsert_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        Ok(self.begin_upsert_hilo_column(id, source)?.commit())
    }

    /// Insert or replace the record `id`: named scalar fields of one length,
    /// checked by name against `source.schema` and uploaded as one batch
    /// transaction ([`ColumnPool::upsert_record`]). Records are resident
    /// only: a streamed id is rejected, and so is an id that names a column
    /// or a ragged column. Failure-atomic, as [`Self::add_columns`]; the
    /// active picker is prepared against the final pool before publication.
    pub fn upsert_record(
        &mut self,
        id: impl Into<RecordId>,
        source: &RecordSource<'_>,
    ) -> Result<RecordId> {
        let id = id.into();
        self.reject_streamed_composite(&id, "a record")?;
        let old = self
            .pool
            .record(&id)
            .map(|record| record.slot().fields.clone())
            .unwrap_or_default();
        let mut changed = [""; data_render::MAX_RECORD_FIELDS + 1];
        changed[0] = id.as_str();
        let mut count = 1usize;
        for member in old.iter().flatten() {
            changed[count] = member;
            count += 1;
        }
        let replaced = count > 1;
        let changed = &changed[..count];
        let invalidation = self.prepare_columns_invalidation(changed.iter().copied())?;
        let ctx = pool_alloc_ctx(
            self.device.as_ref(),
            self.queue.as_ref(),
            self.memory_budget,
            self.gpu_ledger.as_ref(),
        );
        let (picker_state, chart_states) = (&mut self.picker, &self.chart_states);
        let (relocated, picker) =
            self.pool
                .upsert_record_with(id.clone(), source, ctx, move |pool, relocated| {
                    let picker = prepare_picker_for_pool_mutation(
                        picker_state,
                        pool,
                        chart_states,
                        relocated,
                        changed,
                    )?;
                    Ok((relocated, picker))
                })?;
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        self.settle_composite_defrag(relocated, replaced);
        Ok(id)
    }

    /// Insert or replace the ragged column `id`: groups of values with
    /// offsets, uploaded as one batch transaction whose write observer
    /// rejects offsets that do not start at 0, decrease, or do not end at the
    /// value count ([`ColumnPool::upsert_ragged`]). Resident only, as
    /// [`Self::upsert_record`].
    pub fn upsert_ragged(
        &mut self,
        id: impl Into<RaggedId>,
        source: &RaggedSource<'_>,
    ) -> Result<RaggedId> {
        let id = id.into();
        self.reject_streamed_composite(&id, "a ragged column")?;
        let old = self
            .pool
            .ragged(&id)
            .map(|ragged| (ragged.slot().values.clone(), ragged.slot().offsets.clone()));
        let changed_ids = match old.as_ref() {
            Some((values, offsets)) => [id.as_str(), values.as_str(), offsets.as_str()],
            None => [id.as_str(), "", ""],
        };
        let replaced = old.is_some();
        let changed = &changed_ids[..if replaced { 3 } else { 1 }];
        let invalidation = self.prepare_columns_invalidation(changed.iter().copied())?;
        let ctx = pool_alloc_ctx(
            self.device.as_ref(),
            self.queue.as_ref(),
            self.memory_budget,
            self.gpu_ledger.as_ref(),
        );
        let (picker_state, chart_states) = (&mut self.picker, &self.chart_states);
        let (relocated, picker) =
            self.pool
                .upsert_ragged_with(id.clone(), source, ctx, move |pool, relocated| {
                    let picker = prepare_picker_for_pool_mutation(
                        picker_state,
                        pool,
                        chart_states,
                        relocated,
                        changed,
                    )?;
                    Ok((relocated, picker))
                })?;
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        self.settle_composite_defrag(relocated, replaced);
        Ok(id)
    }

    /// Delete the record `id` and free its member columns. `false` when no
    /// such record is resident.
    pub fn remove_record(&mut self, id: &str) -> Result<bool> {
        let Some(record) = self.pool.record(id) else {
            return Ok(false);
        };
        let old = record.slot().fields.clone();
        let mut changed = [""; data_render::MAX_RECORD_FIELDS + 1];
        changed[0] = id;
        let mut count = 1usize;
        for member in old.iter().flatten() {
            changed[count] = member;
            count += 1;
        }
        let changed = &changed[..count];
        let invalidation = self.prepare_columns_invalidation(changed.iter().copied())?;
        let (picker_state, chart_states) = (&mut self.picker, &self.chart_states);
        let picker = self.pool.remove_record_with(id, move |pool| {
            prepare_picker_for_pool_mutation(picker_state, pool, chart_states, false, changed)
        })?;
        let Some(picker) = picker else {
            return Ok(false);
        };
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        self.pending_defrag = true;
        Ok(true)
    }

    /// Delete the ragged column `id` and free its two member columns.
    /// `false` when no such ragged column is resident.
    pub fn remove_ragged(&mut self, id: &str) -> Result<bool> {
        let Some(ragged) = self.pool.ragged(id) else {
            return Ok(false);
        };
        let (values, offsets) = (ragged.slot().values.clone(), ragged.slot().offsets.clone());
        let changed_ids = [id, values.as_str(), offsets.as_str()];
        let changed = &changed_ids[..];
        let invalidation = self.prepare_columns_invalidation(changed.iter().copied())?;
        let (picker_state, chart_states) = (&mut self.picker, &self.chart_states);
        let picker = self.pool.remove_ragged_with(id, move |pool| {
            prepare_picker_for_pool_mutation(picker_state, pool, chart_states, false, changed)
        })?;
        let Some(picker) = picker else {
            return Ok(false);
        };
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        self.pending_defrag = true;
        Ok(true)
    }

    /// Records and ragged columns are resident only: streaming is not
    /// extended to them (CLAUDE.md rule 8).
    fn reject_streamed_composite(&self, id: &str, kind: &str) -> Result<()> {
        if self.streaming_sources.contains_key(id) {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: id.to_owned(),
                reason: format!(
                    "the id is a streamed column; streaming is not extended to records and \
                     ragged columns, so {kind} needs a resident id"
                ),
            });
        }
        Ok(())
    }

    /// Maintenance after a record or ragged upsert: freed old members leave
    /// holes worth compacting; a relayout without them already compacted.
    fn settle_composite_defrag(&mut self, relocated: bool, replaced: bool) {
        if replaced {
            self.pending_defrag = true;
        } else if relocated {
            self.pending_defrag = false;
        }
    }

    /// Remove a column and cascade-remove every renderer-owned series that
    /// references it, so no registered chart can retain a freed data id.
    ///
    /// The core renderer does not rewrite `Config::legend`; legend document
    /// policy belongs to the host. The web wrapper removes rows for its
    /// auto-managed legend and preserves freely edited legend text.
    pub fn remove_column(&mut self, id: &str) -> Result<bool> {
        self.remove_column_impl(id, None)
    }

    /// Remove a column while atomically replacing one chart's config.
    ///
    /// This is the host boundary for a derived legend update caused by the
    /// series cascade. A missing column returns `false` without validating or
    /// publishing `next_config`. All fallible validation, revision, pool, and
    /// picker preparation completes before either authority is published.
    pub fn remove_column_with_chart_config(
        &mut self,
        id: &str,
        chart_id: ChartId,
        next_config: Config,
    ) -> Result<bool> {
        self.remove_column_impl(id, Some((chart_id, next_config)))
    }

    /// Compact every live column to the start of the pool. `true` iff
    /// anything actually moved.
    pub fn defragment(&mut self) -> Result<bool> {
        let defragment = self.pool.begin_defragment(pool_alloc_ctx(
            self.device.as_ref(),
            self.queue.as_ref(),
            self.memory_budget,
            self.gpu_ledger.as_ref(),
        ))?;
        let picker = if defragment.relocated() {
            match self.picker.active.as_ref().map(|active| active.chart_id) {
                Some(chart_id) => {
                    let state = self
                        .chart_states
                        .get(&chart_id)
                        .ok_or(FiggyError::UnknownChart { id: chart_id })?;
                    let plan = PickChartPlan::for_active_layer(&state.config, &state.series)?;
                    self.picker.prepare_active_pool_mutation(
                        defragment.pool(),
                        chart_id,
                        plan,
                        &[],
                    )?
                }
                None => None,
            }
        } else {
            None
        };
        let moved = defragment.commit();
        if let Some(picker) = picker {
            picker.commit();
        }
        self.pending_defrag = false;
        Ok(moved)
    }

    /// Release the defrag backup and shrink the resident pool to the live
    /// columns (never below `minimum_capacity`). This never reads columns
    /// back to the CPU. It waits for submitted GPU work before releasing an
    /// old slab and again before reporting retired bytes as reusable.
    ///
    /// Call only at a host quiescence point: no unsubmitted command buffer may
    /// still reference the old pool. A failed candidate leaves live columns
    /// and picker bindings on the previous primary.
    pub async fn release_unused_pool_memory(&mut self, minimum_capacity: u64) -> Result<()> {
        self.wait_submitted_work().await;
        self.pool.release_backup();
        self.end_gpu_frame();
        self.wait_submitted_work().await;

        let target = self.pool.used_bytes().max(minimum_capacity);
        if target < self.pool.capacity() {
            let result = (|| -> Result<()> {
                let relayout = self.pool.begin_relayout(
                    pool_alloc_ctx(
                        self.device.as_ref(),
                        self.queue.as_ref(),
                        self.memory_budget,
                        self.gpu_ledger.as_ref(),
                    ),
                    target,
                )?;
                let picker = if relayout.relocated() {
                    match self.picker.active.as_ref().map(|active| active.chart_id) {
                        Some(chart_id) => {
                            let state = self
                                .chart_states
                                .get(&chart_id)
                                .ok_or(FiggyError::UnknownChart { id: chart_id })?;
                            let plan =
                                PickChartPlan::for_active_layer(&state.config, &state.series)?;
                            self.picker.prepare_active_pool_mutation(
                                relayout.pool(),
                                chart_id,
                                plan,
                                &[],
                            )?
                        }
                        None => None,
                    }
                } else {
                    None
                };
                relayout.commit();
                if let Some(picker) = picker {
                    picker.commit();
                }
                self.pending_defrag = false;
                Ok(())
            })();
            self.end_gpu_frame();
            self.wait_submitted_work().await;
            result?;
        }
        Ok(())
    }

    pub fn has_pending_maintenance(&self) -> bool {
        self.pending_defrag
    }

    /// Settle queued stream cancellations before a host chooses its draw path,
    /// then perform pending pool compaction. The return value reports only
    /// whether compaction changed the pool; stream retirement may also occur
    /// when it is false.
    pub fn process_pending_maintenance(&mut self) -> Result<bool> {
        self.service_stream_requests();
        if !self.pending_defrag {
            return Ok(false);
        }
        self.defragment()
    }

    pub fn set_defrag_policy(&mut self, policy: DefragPolicy) {
        self.pool.defrag_policy = policy;
    }

    /// Whether an upload that does not fit may enlarge the pool.
    pub fn pool_growth_policy(&self) -> data_render::GrowthPolicy {
        self.pool.growth_policy
    }

    /// Let uploads that do not fit enlarge the pool, or keep the fixed-capacity
    /// behavior.
    ///
    /// `GrowthPolicy::OnAllocFailure` is the default (#107): an upload that
    /// does not fit relayouts the pool into a larger buffer first, which
    /// invalidates every live [`ColumnHandle`] and `PreparedFrame` exactly as a
    /// defragmentation does. Growth is bounded only by the device ceiling and
    /// by [`Self::set_memory_budget`]; when it cannot happen the original
    /// `OutOfSpace` is what the caller sees. `GrowthPolicy::Fixed` keeps the
    /// capacity `try_new` was given: an upload with nowhere to go reports
    /// `OutOfSpace` and the host decides.
    pub fn set_pool_growth_policy(&mut self, policy: data_render::GrowthPolicy) {
        self.pool.growth_policy = policy;
    }

    pub fn handle_for(&self, id: &str) -> Result<ColumnHandle> {
        if self.streaming_sources.contains_key(id) {
            return Err(FiggyError::ColumnNotResident { id: id.to_owned() });
        }
        self.pool
            .handle_for(id)
            .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })
    }

    pub fn is_valid_handle(&self, h: &ColumnHandle) -> bool {
        self.pool.is_valid_handle(h)
    }
}

//! The monotonic chart handle (#271): a chart with monotonic-x line series
//! is registered by [`Renderer::register_monotonic_chart`] and reached only
//! through its [`MonotonicChartId`]. No general chart call takes that id, so
//! none can move a monotonic chart's x-to-pixel mapping: the type keeps
//! them apart. The handle's calls reuse the registry's own steps, so a
//! monotonic chart draws, picks and exports through the same paths as any
//! chart.

use super::*;
use crate::monotonic::{MonotonicAppend, MonotonicConfig, MonotonicSource, MonotonicUpdate};
use crate::renderer::monotonic::{MonotonicGroupRef, MonotonicRange, MonotonicSourceNeed};

/// A registered monotonic chart (#271). Issued by
/// [`Renderer::register_monotonic_chart`]; the general chart calls, which
/// take a [`ChartId`], cannot be given one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MonotonicChartId(pub(crate) ChartId);

/// A monotonic chart and the view it draws into, for
/// [`Renderer::prepare_monotonic`].
pub struct MonotonicChartDrawItem<'a> {
    pub chart: MonotonicChartId,
    pub view: &'a mut ChartView,
}

/// The calls of one monotonic chart (#271), from
/// [`Renderer::monotonic_chart`]. Four move its x-to-pixel mapping, and each
/// takes the groups' source: [`Self::set_monotonic`],
/// [`Self::append_monotonic`], [`Self::resize`] and [`Self::fit_layout`].
/// Everything else (the document through [`Self::edit_config`], the series,
/// the selection, picks) works with no source.
pub struct MonotonicChart<'r> {
    renderer: &'r mut Renderer,
    id: ChartId,
}

impl Renderer {
    /// Registers a monotonic chart (#271): `config` and `series`, which may
    /// hold monotonic line series ([`SeriesConfig::monotonic_line`]) and any
    /// other series. Its monotonic series draw nothing until their group's
    /// first [`MonotonicChart::set_monotonic`].
    pub fn register_monotonic_chart(
        &mut self,
        config: Config,
        series: Vec<SeriesConfig>,
    ) -> Result<MonotonicChartId> {
        let result = self
            .register_chart_unreported(config, series, true)
            .map(MonotonicChartId);
        self.reported(result)
    }

    /// The calls of monotonic chart `id`.
    pub fn monotonic_chart(&mut self, id: MonotonicChartId) -> Result<MonotonicChart<'_>> {
        if !self.chart_states.contains_key(&id.0) {
            return Err(FiggyError::UnknownChart { id: id.0 });
        }
        Ok(MonotonicChart {
            renderer: self,
            id: id.0,
        })
    }

    /// Monotonic chart `id`'s document, read only.
    pub fn monotonic_config(&self, id: MonotonicChartId) -> Result<&Config> {
        self.chart_config(id.0)
    }

    /// Monotonic chart `id`'s series, read only.
    pub fn monotonic_series(&self, id: MonotonicChartId) -> Result<&[SeriesConfig]> {
        self.chart_series(id.0)
    }

    /// The series of monotonic chart `id` whose group was never supplied:
    /// they draw nothing until its first [`MonotonicChart::set_monotonic`].
    pub fn monotonic_needs_source(&self, id: MonotonicChartId) -> Result<Vec<MonotonicSourceNeed>> {
        self.needs_source(id.0)
    }

    /// The kept point of series `series_id` of monotonic chart `id` with
    /// logical sample index `index`, `(x, y)` in data units, read from its
    /// envelope; `None` when the envelope does not keep it.
    pub fn monotonic_kept_point(
        &self,
        id: MonotonicChartId,
        series_id: &str,
        index: usize,
    ) -> Result<Option<(f64, f64)>> {
        self.kept_point_of(id.0, series_id, index)
    }

    /// Monotonic chart `id`'s selected element ([`Self::chart_selection`]).
    pub fn monotonic_selection(&self, id: MonotonicChartId) -> Result<Option<HitId>> {
        self.chart_selection(id.0)
    }

    /// Monotonic chart `id`'s document as drawn on its display
    /// ([`Self::chart_display_config`]).
    pub fn monotonic_display_config(
        &self,
        id: MonotonicChartId,
    ) -> Result<(std::borrow::Cow<'_, Config>, Rect, f32)> {
        self.chart_display_config(id.0)
    }

    /// Monotonic chart `id`'s render stamp ([`Self::chart_render_stamp`]).
    pub fn monotonic_render_stamp(&self, id: MonotonicChartId) -> Result<ChartRenderStamp> {
        self.chart_render_stamp(id.0)
    }

    /// What one series of monotonic chart `id` draws
    /// ([`Self::series_draw_info`]).
    pub fn monotonic_series_draw_info(
        &self,
        id: MonotonicChartId,
        series_id: &str,
    ) -> Result<SeriesDrawInfo> {
        self.series_draw_info(id.0, series_id)
    }

    /// [`Self::prepare_registered`] for monotonic charts: each item's chart
    /// drawn into its view, at its display placement.
    pub fn prepare_monotonic(
        &mut self,
        items: &mut [MonotonicChartDrawItem<'_>],
    ) -> Result<PreparedFrame> {
        let mut registered = Vec::new();
        let result = registered
            .try_reserve_exact(items.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "monotonic draw items",
                reason: error.to_string(),
            })
            .and_then(|()| {
                registered.extend(items.iter_mut().map(|item| RegisteredChartDrawItem {
                    chart_id: item.chart.0,
                    view: &mut *item.view,
                }));
                self.prepare_registered_items_unreported(&mut registered)
            });
        self.reported(result)
    }
}

impl MonotonicChart<'_> {
    /// The chart's handle.
    pub fn id(&self) -> MonotonicChartId {
        MonotonicChartId(self.id)
    }

    /// The document, read only.
    pub fn config(&self) -> Result<&Config> {
        self.renderer.chart_config(self.id)
    }

    /// The series, read only.
    pub fn series(&self) -> Result<&[SeriesConfig]> {
        self.renderer.chart_series(self.id)
    }

    /// Edits the document through a [`MonotonicConfig`]: everything but the
    /// x-to-pixel mapping (#271). The edit applies when it keeps the mapping
    /// ([`crate::monotonic::same_mapping`]); otherwise it is refused with
    /// `RuleKind::MonotonicMappingEdit` and changes nothing.
    pub fn edit_config<R>(
        &mut self,
        edit: impl FnOnce(&mut MonotonicConfig<'_>) -> R,
    ) -> Result<R> {
        let result = self.edit_config_unreported(edit);
        self.renderer.reported(result)
    }

    fn edit_config_unreported<R>(
        &mut self,
        edit: impl FnOnce(&mut MonotonicConfig<'_>) -> R,
    ) -> Result<R> {
        let current = self.renderer.chart_config(self.id)?;
        let mut config = current.clone();
        let value = edit(&mut MonotonicConfig::new(&mut config));
        if !crate::monotonic::same_mapping(current, &config) {
            return Err(FiggyError::InvalidConfig {
                field: "config",
                kind: crate::RuleKind::MonotonicMappingEdit,
            });
        }
        self.renderer.set_chart_config_unreported(self.id, config)?;
        Ok(value)
    }

    /// Replaces the series: a style, a channel, a group, series added or
    /// removed. A series of a group never supplied draws nothing until its
    /// first [`Self::set_monotonic`].
    pub fn set_series(&mut self, series: Vec<SeriesConfig>) -> Result<()> {
        let result = self.renderer.set_chart_series_unreported(self.id, series);
        self.renderer.reported(result)
    }

    /// The full call of group `group` (#271): every series of the chart that
    /// names the group is reduced in full from `source` (its values, `start`
    /// and `interval`) at the mapping it is drawn at, and its envelope
    /// uploaded. The only call that sets the x range of the group's axes:
    /// [`MonotonicRange::X`] writes it (the rest of the axis stays),
    /// [`MonotonicRange::Autoscale`] fits it to the samples' span and each y
    /// axis the group draws on, padded by 5 %, to the values every monotonic
    /// series on that axis keeps (this group's and any other's). Reads only
    /// the visible samples and one on each side; keeps no reference to the
    /// source.
    pub fn set_monotonic<S: MonotonicSource + ?Sized>(
        &mut self,
        group: &str,
        source: &S,
        range: MonotonicRange,
    ) -> Result<MonotonicUpdate> {
        self.renderer.set_monotonic(self.id, group, source, range)
    }

    /// The append call of group `group` (#271): `new` holds only the new
    /// samples, which continue the full call's grid (its `start` and
    /// `interval` are not read). With [`MonotonicAppend::sliding`] the x
    /// range moves by exactly their span. Never reads earlier samples.
    pub fn append_monotonic<S: MonotonicSource>(
        &mut self,
        group: &str,
        new: &S,
        options: MonotonicAppend,
    ) -> Result<MonotonicUpdate> {
        self.renderer.append_monotonic(self.id, group, new, options)
    }

    /// The resize (#271): the chart area, the plot area (data area, in
    /// document pixels) a plot-area resize proposes (`None` keeps the
    /// margins), and the display surface (`None`: no display). Each group
    /// whose series it moves to other device pixels is reduced again from
    /// its source in `monotonic`. Atomic: without a source a moved group
    /// needs, refused with [`FiggyError::MonotonicSourceRequired`] and
    /// nothing changes; a plot rect the margins cannot place is refused
    /// with `RuleKind::NoFeasibleLayout`.
    pub fn resize(
        &mut self,
        chart_area: crate::layout::Rect,
        plot_area: Option<crate::layout::Rect>,
        surface: Option<(u32, u32)>,
        monotonic: &[MonotonicGroupRef<'_>],
    ) -> Result<()> {
        self.renderer
            .resize_monotonic(self.id, chart_area, plot_area, surface, monotonic)
    }

    /// Fits the layout to the labels and titles (`layout::auto_layout`,
    /// measured with the chart's font) and applies it (#271). It can move
    /// the plot's edges, so it takes the sources of the groups it moves,
    /// refused as [`Self::resize`] is without them.
    pub fn fit_layout(&mut self, monotonic: &[MonotonicGroupRef<'_>]) -> Result<()> {
        self.renderer.fit_layout(self.id, monotonic)
    }

    /// Steps the selection ([`Renderer::step_chart_selection`]): a monotonic
    /// point over the kept points.
    pub fn step_selection(&mut self, dx: i32, dy: i32) -> Result<bool> {
        self.renderer.step_chart_selection(self.id, dx, dy)
    }

    /// Selects a chart element ([`Renderer::set_chart_selection`]).
    pub fn set_selection(&mut self, selected: Option<HitId>) -> Result<()> {
        self.renderer.set_chart_selection(self.id, selected)
    }

    /// A typed data pick ([`Renderer::pick_chart_data`]): a monotonic
    /// series reports the logical sample index of a kept point.
    pub fn pick_data(&mut self, request: GpuPickRequest) -> Result<crate::GpuDataPickTicket> {
        self.renderer.pick_chart_data(self.id, request)
    }

    /// A point pick ([`Renderer::pick_chart`]).
    pub fn pick(
        &mut self,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        self.renderer.pick_chart(self.id, request)
    }

    /// Builds the chart's GPU picker
    /// ([`Renderer::prepare_gpu_picking_for_chart`]).
    pub fn prepare_picking(&mut self) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.renderer.prepare_gpu_picking_for_chart(self.id)
    }

    /// Removes the chart, its envelopes and their columns
    /// ([`Renderer::remove_chart`]).
    pub fn remove(self) -> Result<()> {
        self.renderer.remove_chart(self.id)
    }
}

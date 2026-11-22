//! The calls that move the x-to-pixel mapping: the full call, `resize`,
//! `fit_layout` and `append_monotonic`.

use super::*;

impl Renderer {
    /// The full call of a monotonic group (#271): reduce `source` (the
    /// group's whole data, with its `start` and `interval`) in full for every
    /// series of chart `chart` that names group `group`, at the x-to-pixel
    /// mapping each is drawn at, and upload each series' envelope to its two
    /// pool columns. With it `range` sets the x range of the axes the
    /// group's series are drawn against: the only call that does
    /// ([`MonotonicRange::X`]), or fits it to the samples and the y range to
    /// the values this reduction keeps ([`MonotonicRange::Autoscale`]: the
    /// x range `start ..= start + (len - 1) × interval` by arithmetic, the y
    /// range padded by 5 % on each side; a display parameter, rule 1, read
    /// with no other pass over the data). An extent without width (one
    /// sample, a constant channel) leaves its axis as it is.
    ///
    /// Reads only the visible samples and one on each side; keeps no
    /// reference to the source. Fails with [`FiggyError::UnknownMonotonicGroup`]
    /// when no series of the chart names the group, with
    /// [`FiggyError::Monotonic`] for the core's errors (an invalid grid, an x
    /// axis that is not linear and unbroken), with
    /// [`FiggyError::InvalidConfig`] for an x range that is not finite and
    /// increasing, and with [`FiggyError::InvalidSeriesConfig`] for a channel
    /// past the source's or envelope columns another series or chart owns.
    pub(in crate::renderer) fn set_monotonic<S: MonotonicSource + ?Sized>(
        &mut self,
        chart: ChartId,
        group: &str,
        source: &S,
        range: MonotonicRange,
    ) -> Result<MonotonicUpdate> {
        let x = match range {
            MonotonicRange::X { min, max } => (min, max),
            MonotonicRange::Autoscale => (
                source.start(),
                source.start() + source.len().saturating_sub(1) as f64 * source.interval(),
            ),
        };
        let autoscale = range == MonotonicRange::Autoscale;
        let result =
            self.set_monotonic_unreported(chart, group, x, autoscale, &mut |envelope, mapping| {
                envelope.set_monotonic(source, mapping)
            });
        self.reported(result)
    }

    /// Resizes chart `chart` (#271): its document's chart area to
    /// `chart_area`, its plot (data area, in document pixels) to `plot_area`
    /// (`None` keeps the margins: the plot follows the chart area), and the
    /// display surface it is shown on to `surface` (`None`: no display, as
    /// [`Self::set_chart_display`] takes it). The resize call of a chart with
    /// monotonic series, for a canvas or chart-area resize and for a
    /// plot-area resize alike: each monotonic group whose series this moves
    /// to another plot width is reduced again from its source in `monotonic`
    /// in the same call; a group whose mapping stays needs none.
    ///
    /// Atomic: the document, the display and the envelopes are planned and
    /// checked together before any is written. Without a source a moved
    /// group needs, the call is refused the same way and changes nothing; a
    /// plot rect the margins cannot place inside the chart area is refused
    /// with [`FiggyError::InvalidConfig`] (`RuleKind::NoFeasibleLayout`).
    pub(in crate::renderer) fn resize_monotonic(
        &mut self,
        chart: ChartId,
        chart_area: crate::layout::Rect,
        plot_area: Option<crate::layout::Rect>,
        surface: Option<(u32, u32)>,
        monotonic: &[MonotonicGroupRef<'_>],
    ) -> Result<()> {
        let result =
            self.resize_monotonic_unreported(chart, chart_area, plot_area, surface, monotonic);
        self.reported(result)
    }

    fn resize_monotonic_unreported(
        &mut self,
        chart: ChartId,
        chart_area: crate::layout::Rect,
        plot_area: Option<crate::layout::Rect>,
        surface: Option<(u32, u32)>,
        monotonic: &[MonotonicGroupRef<'_>],
    ) -> Result<()> {
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let mut config = state.config.clone();
        config.chart_area = crate::layout::ChartArea(chart_area);
        if let Some(plot) = plot_area {
            place_plot_area(&mut config, plot)?;
        }
        // Its insets keep their place in the plot they are nested in (#392).
        if state.config.has_insets() {
            config.follow_parent_areas(&state.config, None);
        }
        let config = self.prepared_chart_config(chart, config)?;
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let config_changes = state.config != config;
        let surface_changes = state.display.as_ref().map(|display| display.surface) != surface;
        if !config_changes && !surface_changes {
            return Ok(());
        }
        // One plan for the document and the display together: a refusal
        // comes before anything is written.
        let placement = if surface_changes {
            Placement::Surface(surface)
        } else {
            Placement::Current
        };
        let touched = self.reconcile_monotonic(
            chart,
            Some(&config),
            None,
            placement,
            &mut lend_refs(monotonic),
        )?;
        let result = self.commit_resize(
            chart,
            config_changes.then_some(config),
            surface_changes.then_some(surface),
        );
        if result.is_err() && touched {
            self.forget_monotonic_chart(chart);
        }
        result
    }

    /// Writes a resize's display and document. A commit fails only before it
    /// writes, so a document that fails after the display was written puts
    /// the display back: no half-applied resize remains.
    fn commit_resize(
        &mut self,
        chart: ChartId,
        config: Option<Config>,
        surface: Option<Option<(u32, u32)>>,
    ) -> Result<()> {
        let mut previous = None;
        if let Some(surface) = surface {
            let display = self
                .chart_states
                .get_mut(&chart)
                .ok_or(FiggyError::UnknownChart { id: chart })?
                .display
                .take();
            if let Err(error) = self.commit_chart_display(chart, surface) {
                self.restore_display(chart, display);
                return Err(error);
            }
            previous = Some(display);
        }
        if let Some(config) = config
            && let Err(error) = self.commit_chart_config(chart, config)
        {
            if let Some(display) = previous {
                self.restore_display(chart, display);
            }
            return Err(error);
        }
        Ok(())
    }

    fn restore_display(&mut self, chart: ChartId, display: Option<ChartDisplay>) {
        if let Some(state) = self.chart_states.get_mut(&chart) {
            state.display = display;
        }
    }

    /// Fits chart `chart`'s layout to its labels and titles
    /// ([`crate::layout::auto_layout`], measured with the font its draw
    /// style renders) and applies it (#271): the auto-layout of a registered
    /// chart. It can move the plot's edges, so each monotonic group whose
    /// series it moves is reduced again from its source in `monotonic` in
    /// the same call; without a source a moved group needs, the call is
    /// refused with [`FiggyError::MonotonicSourceRequired`] and changes
    /// nothing. A chart without monotonic series takes no source.
    pub(in crate::renderer) fn fit_layout(
        &mut self,
        chart: ChartId,
        monotonic: &[MonotonicGroupRef<'_>],
    ) -> Result<()> {
        let result = self.fit_layout_unreported(chart, monotonic);
        self.reported(result)
    }

    fn fit_layout_unreported(
        &mut self,
        chart: ChartId,
        monotonic: &[MonotonicGroupRef<'_>],
    ) -> Result<()> {
        let config = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?
            .config
            .clone();
        let measure = crate::CpuTextMeasure::for_style(&config.draw_style);
        let laid_out = crate::layout::auto_layout(&config, &measure);
        self.set_chart_config_lent(chart, laid_out, &mut lend_refs(monotonic))
    }

    /// The append call of a monotonic group (#271): `new` holds only the new
    /// samples of every channel; sample `j` of it is the group's logical
    /// sample `len + j`. The renderer compares nothing and never changes the
    /// scale: the new samples inside the kept columns are merged into the
    /// envelope and later ones are counted, not kept. With
    /// [`MonotonicAppend::sliding`] the window keeps its length: the chart's
    /// x range moves by exactly `new.len() × interval`, written to the
    /// document (the x axis of each panel the group is drawn on), and the
    /// columns that left are dropped. Never reads earlier samples, so the
    /// host may drop them; the new samples continue the grid of the full
    /// call, so `new`'s `start` and `interval` are not read.
    pub(in crate::renderer) fn append_monotonic<S: MonotonicSource>(
        &mut self,
        chart: ChartId,
        group: &str,
        new: &S,
        options: MonotonicAppend,
    ) -> Result<MonotonicUpdate> {
        let result =
            self.append_monotonic_unreported(chart, group, options.sliding, &mut |envelope| {
                envelope.append_monotonic(new, options)
            });
        self.reported(result)
    }
}

//! The envelope state per group and mapping, the document helpers and the
//! envelope rows uploaded to the pool.

use super::*;

/// Rows of a series' envelope columns per envelope slot: each of a cell's
/// 4 points and its join marker.
pub(crate) const ROWS_PER_SLOT: usize = 2 * CELL_POINTS;

/// The whole pixel columns an envelope keeps beyond each edge of the plot
/// ([`MonotonicMapping::margin_px`]), fixed when `set_monotonic` builds it:
/// a stroke paints into the plot from samples outside it by up to half its
/// width plus its cap and antialiasing, so strokes up to about 120 device
/// pixels wide draw their edges as a full draw does; a wider one may miss
/// ink at the plot's edges. A display parameter, not a count of data, and
/// no style edit changes it: a stroke width is a plain style edit.
pub(crate) const MARGIN_PX: u32 = 64;

/// A monotonic series that draws nothing until its group's first full call
/// ([`Renderer::monotonic_needs_source`]).
#[derive(Clone, Debug, PartialEq)]
pub struct MonotonicSourceNeed {
    /// The series.
    pub series_id: String,
    /// The group whose `set_monotonic` call draws it.
    pub group: String,
    /// Why it draws nothing: [`MappingChange::NotBuilt`], since every call
    /// that changes the mapping rebuilds the envelope or is refused.
    pub change: MappingChange,
}

/// A monotonic group's source, lent to a call that reduces it again at
/// another mapping: a resize ([`crate::renderer::MonotonicChart::resize`]), a layout fit
/// ([`crate::renderer::MonotonicChart::fit_layout`]) or an export at another scale
/// ([`ExportSource::Monotonic`]). It borrows the source for that call
/// only.
#[derive(Clone, Copy, Debug)]
pub struct MonotonicGroupRef<'a> {
    /// The group's name, as its series name it.
    pub group: &'a str,
    /// The samples, with their `start` and `interval`.
    pub source: MonotonicSourceRef<'a>,
}

impl<'a> MonotonicGroupRef<'a> {
    /// `group`'s `source`. On native targets the source is `Sync`, so an
    /// export that borrows it stays `Send`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new<S: MonotonicSource + Sync + 'a>(group: &'a str, source: &'a S) -> Self {
        Self {
            group,
            source: MonotonicSourceRef::new(source),
        }
    }

    /// `group`'s `source`.
    #[cfg(target_arch = "wasm32")]
    pub fn new<S: MonotonicSource + 'a>(group: &'a str, source: &'a S) -> Self {
        Self {
            group,
            source: MonotonicSourceRef::new(source),
        }
    }
}

/// The x range [`crate::renderer::MonotonicChart::set_monotonic`] sets: the only call that sets
/// the x range of a monotonic series' axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MonotonicRange {
    /// This x range, in data units, written to the x axis the group's
    /// series are drawn against as a slide writes it: the rest of the axis
    /// (its ticks and labels) and the y range stay as they are.
    X { min: f64, max: f64 },
    /// The x range fitted to the samples' span, `start ..= start + (len -
    /// 1) × interval`, and the y range, padded by 5 % on each side, to the
    /// lowest and highest value the reduction keeps in the plot.
    Autoscale,
}

/// How a change places the chart's document on its display.
#[derive(Clone, Copy, Debug)]
pub(in crate::renderer) enum Placement {
    /// As the chart's display places it now.
    Current,
    /// On a display of this surface, unheld (`None`: no display).
    Surface(Option<(u32, u32)>),
    /// The display's placement held, or fitted to its surface again.
    Hold(bool),
}

/// Builds a group's envelope at a mapping from the source a call lends;
/// `None` when the call lends none for the group.
pub(in crate::renderer) type Lender<'c> = dyn FnMut(
        &str,
        &mut MonotonicEnvelope,
        &MonotonicMapping,
    ) -> Option<std::result::Result<MonotonicUpdate, MonotonicError>>
    + 'c;

/// A call that lends no source: a plain change.
pub(in crate::renderer) fn lend_none(
    _: &str,
    _: &mut MonotonicEnvelope,
    _: &MonotonicMapping,
) -> Option<std::result::Result<MonotonicUpdate, MonotonicError>> {
    None
}

/// A call that lends the sources in `monotonic`, each by its group.
pub(super) fn lend_refs<'m>(
    monotonic: &'m [MonotonicGroupRef<'m>],
) -> impl FnMut(
    &str,
    &mut MonotonicEnvelope,
    &MonotonicMapping,
) -> Option<std::result::Result<MonotonicUpdate, MonotonicError>>
+ 'm {
    move |group, envelope, mapping| {
        monotonic
            .iter()
            .find(|data| data.group == group)
            .map(|data| envelope.set_monotonic_ref(data.source, mapping))
    }
}

/// What a change leaves of a chart's monotonic series: the envelopes each
/// group with data keeps, one per mapping its series are drawn at, and
/// every series drawn from one.
pub(in crate::renderer) struct MonotonicPlan {
    pub(super) chart: ChartId,
    pub(super) groups: Vec<(String, Vec<Arc<MonotonicEnvelope>>)>,
    pub(super) members: Vec<Planned>,
}

/// A series of a plan: its envelope and whether its columns are written.
pub(super) struct Planned {
    pub(super) member: Member,
    pub(super) group: String,
    pub(super) panel: usize,
    pub(super) write: bool,
}

/// A full call on one envelope at one mapping: the core's
/// `set_monotonic` on the caller's typed source.
pub(super) type FullCall<'c> = dyn FnMut(
        &mut MonotonicEnvelope,
        &MonotonicMapping,
    ) -> std::result::Result<MonotonicUpdate, MonotonicError>
    + 'c;

/// An append on one envelope: the core's `append_monotonic` on the
/// caller's typed source.
pub(super) type AppendCall<'c> =
    dyn FnMut(&mut MonotonicEnvelope) -> std::result::Result<MonotonicUpdate, MonotonicError> + 'c;

/// The renderer's monotonic state: the envelopes of every group of every
/// registered chart, and the pool columns monotonic series draw from.
pub(in crate::renderer) struct MonotonicState {
    /// By chart and group name.
    pub(super) groups: HashMap<ChartId, HashMap<String, MonotonicGroupState>>,
    /// The envelope columns, x and y of every series, by pool column id.
    pub(super) columns: HashMap<ColumnId, MonotonicColumn>,
}

/// The envelopes of one group: one per x-to-pixel mapping its series are
/// drawn at, shared with the picks in flight (copied on write only while
/// one holds it).
pub(super) struct MonotonicGroupState {
    pub(super) panels: Vec<Arc<MonotonicEnvelope>>,
}

/// Whose envelope a pool column holds.
#[derive(Clone, Debug)]
pub(super) struct MonotonicColumn {
    pub(super) chart: ChartId,
    /// The key of the envelopes in the chart's group map: the group's name,
    /// or an export's transient key.
    pub(super) key: String,
    /// The group the series reads.
    pub(super) group: String,
    pub(super) series_id: String,
    /// Index into the group's envelopes.
    pub(super) panel: usize,
    pub(super) channel: usize,
}

/// One series of a call: what it draws and where.
pub(super) struct Member {
    pub(super) series_id: String,
    pub(super) x_column: ColumnId,
    pub(super) y_column: ColumnId,
    pub(super) channel: usize,
}

/// The rows of a pick's monotonic series (#271): the envelope its columns
/// were written from when the pick was submitted, which maps the picked row
/// back to a logical sample index without the source.
#[derive(Clone)]
pub(crate) struct MonotonicPickRows {
    pub(crate) series_id: String,
    pub(crate) envelope: Arc<MonotonicEnvelope>,
    pub(crate) channel: usize,
}

impl MonotonicPickRows {
    /// The logical sample index of `row`, `None` for a row that holds no
    /// kept point.
    pub(crate) fn logical(&self, row: usize) -> Option<usize> {
        row_logical(&self.envelope, self.channel, row)
    }
}

impl MonotonicState {
    pub(in crate::renderer) fn new() -> Self {
        Self {
            groups: HashMap::new(),
            columns: HashMap::new(),
        }
    }

    /// Whether pool column `id` holds a monotonic envelope: the renderer
    /// writes it, a host cannot.
    pub(in crate::renderer) fn owns_column(&self, id: &str) -> bool {
        self.columns.contains_key(id)
    }

    /// The envelope and channel series `cfg` is drawn from, when it was
    /// built.
    pub(super) fn envelope_of(&self, cfg: &SeriesConfig) -> Option<(&Arc<MonotonicEnvelope>, usize)> {
        let record = self.columns.get(&cfg.x_column)?;
        let monotonic = cfg.monotonic()?;
        // The columns hold this series' channel of this group, written for
        // both of its columns; an edit of the group, the channel or a column
        // id leaves the series needing its source.
        let same = |other: &MonotonicColumn| {
            other.series_id == cfg.series_id
                && other.key == record.key
                && other.group == monotonic.group
                && other.channel == monotonic.channel
                && other.chart == record.chart
                && other.panel == record.panel
        };
        if !same(record) || !self.columns.get(&cfg.y_column).is_some_and(same) {
            return None;
        }
        let envelope = self
            .groups
            .get(&record.chart)?
            .get(&record.key)?
            .panels
            .get(record.panel)?;
        Some((envelope, record.channel))
    }

    /// Fails with [`FiggyError::MonotonicSourceRequired`] when monotonic
    /// series `cfg`, drawn on `panel` (its own panel's config, as drawn), has
    /// no envelope built for that panel's x-to-pixel mapping:
    /// the series is never drawn from a stale envelope.
    pub(super) fn check_series(&self, panel: &Config, cfg: &SeriesConfig) -> Result<()> {
        let change = match self.envelope_of(cfg) {
            None => Some(MappingChange::NotBuilt),
            Some((envelope, _)) => match envelope.check_mapping(&panel_mapping(panel)) {
                Ok(()) => None,
                Err(MonotonicError::SourceRequired { change }) => Some(change),
                Err(_) => Some(MappingChange::NotBuilt),
            },
        };
        match change {
            None => Ok(()),
            Some(change) => Err(FiggyError::MonotonicSourceRequired {
                series_id: cfg.series_id.clone(),
                change,
            }),
        }
    }

    /// [`Self::check_series`] for every monotonic series of the expanded
    /// panels a frame or an export draws. A series whose group has no data
    /// yet has no columns and draws nothing. A registered chart's envelopes
    /// always match the mapping it is drawn at, since every call that moves
    /// it rebuilds them or is refused; only a host-held draw of a
    /// registered chart's series at another placement can fail here.
    pub(in crate::renderer) fn check_panels(&self, items: &[PanelDraw<'_>]) -> Result<()> {
        for item in items {
            for series in item.series {
                if series.config.monotonic().is_none() {
                    continue;
                }
                match self.check_series(item.chart_config, series.config) {
                    Err(FiggyError::MonotonicSourceRequired {
                        change: MappingChange::NotBuilt,
                        ..
                    }) => {}
                    other => other?,
                }
            }
        }
        Ok(())
    }

    /// The selection visuals of monotonic series `cfg` with each logical
    /// sample index replaced by the row its kept point was uploaded to; a
    /// point the envelope does not keep is not highlighted.
    pub(in crate::renderer) fn rows_of_visuals(
        &self,
        cfg: &SeriesConfig,
        visuals: &mut Vec<selection_prepare::PointSelectionVisual>,
    ) {
        let Some((envelope, channel)) = self.envelope_of(cfg) else {
            visuals.clear();
            return;
        };
        visuals.retain_mut(
            |visual| match logical_row(envelope, channel, visual.point_index) {
                Some(row) => {
                    visual.point_index = row;
                    true
                }
                None => false,
            },
        );
    }

    /// The rows a pick on chart `chart` maps back through: one entry per
    /// monotonic series of `series` with a built envelope.
    pub(in crate::renderer) fn pick_rows(
        &self,
        series: &[SeriesConfig],
    ) -> Result<Vec<MonotonicPickRows>> {
        let count = series.iter().filter(|s| s.monotonic().is_some()).count();
        let mut rows = Vec::new();
        if count == 0 {
            return Ok(rows);
        }
        rows.try_reserve_exact(count)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "monotonic pick rows",
                reason: error.to_string(),
            })?;
        for cfg in series {
            if let Some((envelope, channel)) = self.envelope_of(cfg) {
                rows.push(MonotonicPickRows {
                    series_id: cfg.series_id.clone(),
                    envelope: Arc::clone(envelope),
                    channel,
                });
            }
        }
        Ok(rows)
    }

    /// Whether the last call of `cfg`'s group took the common extent of
    /// channels of different lengths.
    pub(in crate::renderer) fn truncated(&self, cfg: &SeriesConfig) -> bool {
        self.envelope_of(cfg)
            .is_some_and(|(envelope, _)| envelope.truncated())
    }

    /// The kept point of monotonic series `cfg` with logical sample index
    /// `index`, as `(x, y)` in data units.
    pub(super) fn kept_point(&self, cfg: &SeriesConfig, index: usize) -> Option<(f64, f64)> {
        let (envelope, channel) = self.envelope_of(cfg)?;
        let row = logical_row(envelope, channel, index)?;
        let (i, value) = row_point(envelope, channel, row)?;
        Some((envelope.grid()?.x(i), value))
    }

    /// Drops chart `chart`'s monotonic state and returns the envelope
    /// columns it held, for removal from the pool.
    pub(super) fn forget_chart(&mut self, chart: ChartId) -> Vec<ColumnId> {
        self.groups.remove(&chart);
        let mut ids: Vec<ColumnId> = self
            .columns
            .iter()
            .filter(|(_, record)| record.chart == chart)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        for id in &ids {
            self.columns.remove(id);
        }
        ids
    }

    /// Whether chart `chart` has monotonic envelopes or columns.
    pub(super) fn has_chart(&self, chart: ChartId) -> bool {
        self.groups
            .get(&chart)
            .is_some_and(|groups| !groups.is_empty())
            || self.columns.values().any(|record| record.chart == chart)
    }

    /// Whether series `cfg`'s columns already hold channel `channel` of
    /// `envelope`, the envelope of group `group` of chart `chart` at its
    /// recorded place: then a change writes nothing.
    pub(super) fn holds(
        &self,
        chart: ChartId,
        cfg: &SeriesConfig,
        group: &str,
        channel: usize,
        envelope: &Arc<MonotonicEnvelope>,
    ) -> bool {
        let held = |id: &ColumnId| {
            self.columns.get(id).is_some_and(|record| {
                record.chart == chart
                    && record.key == group
                    && record.group == group
                    && record.series_id == cfg.series_id
                    && record.channel == channel
                    && self
                        .groups
                        .get(&chart)
                        .and_then(|groups| groups.get(group))
                        .and_then(|state| state.panels.get(record.panel))
                        .is_some_and(|held| Arc::ptr_eq(held, envelope))
            })
        };
        held(&cfg.x_column) && held(&cfg.y_column)
    }

    /// What moved for series `cfg` of group state `old` to be drawn at
    /// `mapping`: from the envelope its columns hold, or else from the
    /// group's first envelope.
    pub(super) fn change_of(
        &self,
        cfg: &SeriesConfig,
        old: Option<&MonotonicGroupState>,
        mapping: &MonotonicMapping,
    ) -> MappingChange {
        self.envelope_of(cfg)
            .map(|(envelope, _)| envelope)
            .or_else(|| old.and_then(|state| state.panels.first()))
            .and_then(|envelope| envelope.mapping())
            .and_then(|built| built.change_to(mapping))
            .unwrap_or(MappingChange::NotBuilt)
    }

    /// Drops group `group` of chart `chart` and returns its envelope
    /// columns, for removal from the pool: its series draw nothing until
    /// the next full call.
    pub(super) fn forget_group(&mut self, chart: ChartId, group: &str) -> Vec<ColumnId> {
        if let Some(groups) = self.groups.get_mut(&chart) {
            groups.remove(group);
        }
        let mut ids: Vec<ColumnId> = self
            .columns
            .iter()
            .filter(|(_, record)| record.chart == chart && record.key == group)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        for id in &ids {
            self.columns.remove(id);
        }
        ids
    }
}

/// The y padding of [`crate::renderer::MonotonicChart::set_monotonic`]'s autoscale: a twentieth of
/// the kept values' span on each side. A display parameter.
const MONOTONIC_AUTOSCALE_Y_PADDING: f64 = 0.05;

/// The lowest and highest value channel `channel` of `envelope` keeps in
/// the plot's own columns (not its margins): every column's extremes are
/// kept points, so this is the extent of the samples the reduction read
/// there.
/// The x axis series `series_id` of `config` is drawn against: its
/// overlay's own (`link_x` off), its panel's `bottom_x`, or the one a linked
/// subplot's x follows.
pub(super) fn x_axis_mut<'c>(
    config: &'c mut Config,
    series_id: &str,
) -> Option<&'c mut crate::config::AxisOptions> {
    let layer = config.layer_of_series(series_id);
    if config
        .overlay_of_layer(layer)
        .is_some_and(|overlay| !overlay.link_x)
    {
        return overlay_mut(config, layer).map(|overlay| &mut overlay.x_axis);
    }
    match layer {
        crate::LayerRef::Subplot(index) | crate::LayerRef::SubplotOverlay(index, _) => {
            match config.subplot_link_root(index, true) {
                crate::CellRef::Subplot(root) => config
                    .subplots
                    .get_mut(root)
                    .map(|subplot| &mut subplot.config.bottom_x),
                crate::CellRef::Parent => Some(&mut config.bottom_x),
            }
        }
        _ => Some(&mut config.bottom_x),
    }
}

/// Moves `config`'s plot edges, the margins its data-area edges nudge, so
/// that its data area is `target` (#271): what a plot-area resize writes.
/// Refused with `RuleKind::NoFeasibleLayout` when the margins cannot place
/// it exactly.
pub(super) fn place_plot_area(config: &mut Config, target: crate::layout::Rect) -> Result<()> {
    use crate::layout::{Element, NudgeResult, Side};
    let refused = || FiggyError::InvalidConfig {
        field: "plot_area",
        kind: crate::RuleKind::NoFeasibleLayout,
    };
    let current = config.data_area().map_err(|_| refused())?.0;
    let measure = crate::CpuTextMeasure::for_style(&config.draw_style);
    let end = |at: u32, length: u32| f64::from(at) + f64::from(length);
    let left = (f64::from(target.x) - f64::from(current.x)) as f32;
    let right = (end(target.x, target.width) - end(current.x, current.width)) as f32;
    let top = (f64::from(target.y) - f64::from(current.y)) as f32;
    let bottom = (end(target.y, target.height) - end(current.y, current.height)) as f32;
    // The edge that moves away from the other goes first, so the plot keeps
    // a width and a height between the two moves.
    let horizontal = if left > 0.0 {
        [(Side::Right, right), (Side::Left, left)]
    } else {
        [(Side::Left, left), (Side::Right, right)]
    };
    let vertical = if top > 0.0 {
        [(Side::Bottom, bottom), (Side::Top, top)]
    } else {
        [(Side::Top, top), (Side::Bottom, bottom)]
    };
    for (side, delta) in horizontal {
        if delta != 0.0
            && config.nudge(Element::DataAreaEdge(side), &measure, delta, 0.0) != NudgeResult::Moved
        {
            return Err(refused());
        }
    }
    for (side, delta) in vertical {
        if delta != 0.0
            && config.nudge(Element::DataAreaEdge(side), &measure, 0.0, delta) != NudgeResult::Moved
        {
            return Err(refused());
        }
    }
    match config.data_area() {
        Ok(area) if area.0 == target => Ok(()),
        _ => Err(refused()),
    }
}

pub(super) fn plot_extent(envelope: &MonotonicEnvelope, channel: usize) -> crate::chart::FitExtent {
    let mut extent = crate::chart::FitExtent::EMPTY;
    let Some(grid) = envelope.grid() else {
        return extent;
    };
    // Slot `c + 1` holds envelope column `c`; the plot's columns follow the
    // margin.
    let cells = envelope
        .channel_cells(channel)
        .skip(grid.margin() + 1)
        .take(grid.plot_width());
    for cell in cells {
        for &value in &cell.value[..usize::from(cell.count)] {
            if value.is_finite() {
                extent.union(&crate::chart::FitExtent::point(value));
            }
        }
    }
    extent
}

/// The overlay `layer` names, editable (see `Config::overlay_of_layer`).
fn overlay_mut(config: &mut Config, layer: crate::LayerRef) -> Option<&mut crate::OverlayLayer> {
    match layer {
        crate::LayerRef::Overlay(index) => config.overlays.get_mut(index),
        crate::LayerRef::SubplotOverlay(index, overlay) => config
            .subplots
            .get_mut(index)
            .and_then(|subplot| subplot.config.overlays.get_mut(overlay)),
        _ => None,
    }
}

/// The layer whose y axis series of `layer` are drawn against: `layer`
/// itself, or for an overlay with `link_y` on the chart it follows (`Base`
/// or `Subplot(i)`). Members with the same one share one axis and one fit.
pub(super) fn y_axis_layer(config: &Config, layer: crate::LayerRef) -> crate::LayerRef {
    if !config
        .overlay_of_layer(layer)
        .is_some_and(|overlay| overlay.link_y)
    {
        return layer;
    }
    match layer {
        crate::LayerRef::SubplotOverlay(index, _) => crate::LayerRef::Subplot(index),
        _ => crate::LayerRef::Base,
    }
}

/// The config of the panel layer `layer` is drawn on: the document's, or
/// a subplot's (the root of its x link chain, whose x axis it shows).
pub(super) fn panel_config_mut(config: &mut Config, layer: crate::LayerRef) -> Option<&mut Config> {
    match layer {
        crate::LayerRef::Subplot(index) | crate::LayerRef::SubplotOverlay(index, _) => {
            match config.subplot_link_root(index, true) {
                crate::CellRef::Subplot(root) => config
                    .subplots
                    .get_mut(root)
                    .map(|subplot| &mut subplot.config),
                crate::CellRef::Parent => Some(config),
            }
        }
        _ => Some(config),
    }
}

/// Fits the y axis layer `layer` is drawn against to `extent`: the
/// panel's `left_y` (with its partner), or an overlay's own axis.
pub(super) fn fit_y_axis(config: &mut Config, layer: crate::LayerRef, extent: &crate::chart::FitExtent) {
    let padding = MONOTONIC_AUTOSCALE_Y_PADDING;
    let empty = crate::chart::FitExtent::EMPTY;
    match layer {
        crate::LayerRef::Overlay(index) => {
            if let Some(overlay) = config.overlays.get_mut(index) {
                crate::chart::apply_axis_fit(&mut overlay.y_axis, extent, padding);
            }
        }
        crate::LayerRef::Subplot(index) => {
            if let Some(subplot) = config.subplots.get_mut(index) {
                crate::chart::apply_auto_fit_all(&mut subplot.config, &empty, extent, padding);
            }
        }
        crate::LayerRef::SubplotOverlay(index, overlay) => {
            if let Some(overlay) = config
                .subplots
                .get_mut(index)
                .and_then(|subplot| subplot.config.overlays.get_mut(overlay))
            {
                crate::chart::apply_axis_fit(&mut overlay.y_axis, extent, padding);
            }
        }
        _ => crate::chart::apply_auto_fit_all(config, &empty, extent, padding),
    }
}

/// The x-to-pixel mapping series of panel `panel` (its own config, as
/// drawn) are drawn at: the x range, the data area's width in device
/// pixels, the x scale, breaks and direction, and the fixed margin.
pub(in crate::renderer) fn panel_mapping(panel: &Config) -> MonotonicMapping {
    let axis = &panel.bottom_x;
    let (x_min, x_max) = axis.data_range();
    MonotonicMapping {
        x_min,
        x_max,
        // A data area the layout cannot place has no width: the mapping is
        // refused, never drawn.
        plot_width_px: panel.data_area().map_or(0, |area| area.0.width),
        x_scale: axis.scale.clone(),
        x_breaks: crate::layout::BreakMap::cuts(axis),
        x_inverted: axis.inverted,
        margin_px: MARGIN_PX,
    }
}

/// The panels a document draws as, to find the panel config of each series:
/// the document itself unless it is layered.
pub(super) struct DocumentPanels<'c> {
    document: &'c Config,
    expanded: Vec<Config>,
}

impl<'c> DocumentPanels<'c> {
    pub(super) fn new(document: &'c Config) -> Result<Self> {
        let expanded = if is_layered(document) {
            expand_for_render(document)?
        } else {
            Vec::new()
        };
        Ok(Self { document, expanded })
    }

    /// The config of the panel series `series_id` is drawn on.
    pub(super) fn of(&self, series_id: &str) -> &Config {
        if self.expanded.is_empty() {
            return self.document;
        }
        let index = series_panel_index(self.document, self.expanded.len(), series_id);
        self.expanded.get(index).unwrap_or(self.document)
    }
}

/// The point uploaded at `row` of a channel's envelope columns: point
/// `(row % 8) / 2` of slot `row / 8`, a short cell's last point past its
/// count, and the last point before an empty slot.
fn row_point(envelope: &MonotonicEnvelope, channel: usize, row: usize) -> Option<(usize, f64)> {
    let slot = row / ROWS_PER_SLOT;
    let at = (row % ROWS_PER_SLOT) / 2;
    let mut current = slot;
    loop {
        let cell = envelope.cell(channel, current)?;
        if cell.count > 0 {
            let k = if current == slot {
                at.min(usize::from(cell.count) - 1)
            } else {
                usize::from(cell.count) - 1
            };
            return cell.points().nth(k);
        }
        current = current.checked_sub(1)?;
    }
}

/// The logical sample index of the kept point uploaded at `row`; `None` for
/// a gap or a row before any point.
pub(crate) fn row_logical(
    envelope: &MonotonicEnvelope,
    channel: usize,
    row: usize,
) -> Option<usize> {
    row_point(envelope, channel, row)
        .filter(|(_, value)| value.is_finite())
        .map(|(index, _)| index)
}

/// The row the kept point with logical sample index `index` was uploaded
/// to: the slot its column says, else either edge slot, else any slot (a
/// slide moves edge samples). `None` when the envelope keeps no such finite
/// point.
fn logical_row(envelope: &MonotonicEnvelope, channel: usize, index: usize) -> Option<usize> {
    let slots = envelope.slots();
    let last = slots.checked_sub(1)?;
    let guess = match envelope.grid()?.column(index) {
        None => 0,
        Some(column) => column.saturating_add(1).min(last),
    };
    let find = |slot: usize| {
        let cell = envelope.cell(channel, slot)?;
        cell.points()
            .position(|(i, value)| i == index && value.is_finite())
            .map(|k| slot * ROWS_PER_SLOT + 2 * k)
    };
    find(guess)
        .or_else(|| find(0))
        .or_else(|| find(last))
        .or_else(|| (0..slots).find_map(find))
}

/// The kept finite points of a channel in x order, as logical sample
/// indices.
pub(super) fn kept_indices(envelope: &MonotonicEnvelope, channel: usize) -> impl Iterator<Item = usize> + '_ {
    envelope.channel_cells(channel).flat_map(|cell| {
        (0..usize::from(cell.count)).filter_map(move |k| {
            let value = cell.value[k];
            value.is_finite().then_some(cell.index[k])
        })
    })
}

/// One channel's envelope as one of its two pool columns, written straight
/// into the upload's mapped staging (rule 3): see the module's upload
/// layout.
pub(super) struct EnvelopeRows<'a> {
    pub(super) envelope: &'a MonotonicEnvelope,
    pub(super) channel: usize,
    /// The x column (`start + index × interval`) or the y column.
    pub(super) x: bool,
}

impl EnvelopeRows<'_> {
    /// The value a row holds for point `(index, value)`.
    fn coordinate(&self, grid: &crate::monotonic::MonotonicGrid, point: (usize, f64)) -> f64 {
        if self.x { grid.x(point.0) } else { point.1 }
    }
}

impl ColumnSource for EnvelopeRows<'_> {
    fn len(&self) -> usize {
        self.envelope.slots().saturating_mul(ROWS_PER_SLOT)
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        let slots = self.envelope.slots();
        let Some(grid) = self.envelope.grid() else {
            return;
        };
        let cell = |slot: usize| {
            self.envelope
                .cell(self.channel, slot)
                .unwrap_or(EnvelopeCell::EMPTY)
        };
        let nan = |dst: &mut ColumnPairWriter<'_>, row: usize| dst.write_value(row, f64::NAN);
        let mut slot = 0;
        // Empty slots before the first point draw nothing.
        while slot < slots && cell(slot).count == 0 {
            for row in slot * ROWS_PER_SLOT..(slot + 1) * ROWS_PER_SLOT {
                nan(dst, row);
            }
            slot += 1;
        }
        while slot < slots {
            let here = cell(slot);
            let count = usize::from(here.count);
            // The next slot with a point, and whether the line runs on to it.
            let next = (slot + 1..slots).find(|&s| cell(s).count > 0);
            let last = (here.index[count - 1], here.value[count - 1]);
            let joins = next
                .and_then(|s| cell(s).first())
                .is_some_and(|first| EnvelopeCell::joins(last, first));
            let base = slot * ROWS_PER_SLOT;
            for k in 0..CELL_POINTS {
                let point = if k < count {
                    (here.index[k], here.value[k])
                } else {
                    last
                };
                dst.write_value(base + 2 * k, self.coordinate(grid, point));
                // The marker after point k: the line goes on to point k + 1,
                // breaks there, or (after the last point) runs on to the
                // next slot.
                let goes_on = if k + 1 < count {
                    here.linked(k)
                } else if k + 1 == CELL_POINTS {
                    joins
                } else {
                    true
                };
                if goes_on {
                    dst.write_value(base + 2 * k + 1, self.coordinate(grid, point));
                } else {
                    nan(dst, base + 2 * k + 1);
                }
            }
            // The empty slots up to the next point carry the line across
            // or draw nothing.
            let end = next.unwrap_or(slots);
            for row in (slot + 1) * ROWS_PER_SLOT..end * ROWS_PER_SLOT {
                if joins {
                    dst.write_value(row, self.coordinate(grid, last));
                } else {
                    nan(dst, row);
                }
            }
            slot = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monotonic::MonotonicSlices;

    /// The rows `EnvelopeRows` writes, decoded from their `(hi, lo)` pairs.
    fn rows(envelope: &MonotonicEnvelope, channel: usize, x: bool) -> Vec<f64> {
        let source = EnvelopeRows {
            envelope,
            channel,
            x,
        };
        let mut bytes = vec![0u8; source.len() * crate::data::COLUMN_VALUE_BYTES];
        let out = std::cell::Cell::new(false);
        let mut writer = ColumnPairWriter::from_bytes_for_test(&mut bytes, &out);
        source.write_pairs(&mut writer);
        assert!(!out.get());
        bytes
            .chunks_exact(8)
            .map(|pair| {
                let hi = f32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]);
                let lo = f32::from_le_bytes([pair[4], pair[5], pair[6], pair[7]]);
                f64::from(hi) + f64::from(lo)
            })
            .collect()
    }

    /// A dense envelope (samples with NaN gaps) at the renderer's margin.
    fn envelope(values: &[f64], width: u32) -> MonotonicEnvelope {
        let channels = [values];
        let source = MonotonicSlices::new(0.0, 1.0, &channels);
        let mut mapping = MonotonicMapping::linear(0.0, (values.len() - 1) as f64, width);
        mapping.margin_px = MARGIN_PX;
        let mut envelope = MonotonicEnvelope::empty();
        envelope.set_monotonic(&source, &mapping).expect("set");
        envelope
    }

    fn signal(n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| {
                let t = i as f64 * 1e-3;
                if i % 9_973 == 17 {
                    f64::NAN
                } else {
                    (t * 3.0).sin() + 0.3 * (t * 37.0).sin()
                }
            })
            .collect()
    }

    /// The rows draw every plot column's span (up to the `(hi, lo)` f32
    /// pair's precision): the segments between finite
    /// consecutive rows that lie in a column cover the lowest and highest
    /// drawn sample of that column, and no segment joins across a NaN row.
    #[test]
    fn the_rows_cover_every_columns_span() {
        let n = 200_000usize;
        let width = 440;
        let values = signal(n);
        let envelope = envelope(&values, width);
        let xs = rows(&envelope, 0, true);
        let ys = rows(&envelope, 0, false);
        assert_eq!(xs.len(), ROWS_PER_SLOT * envelope.slots());
        assert!(envelope.slots() >= (width + 2 * MARGIN_PX) as usize);
        let xpp = (n - 1) as f64 / f64::from(width);
        let column = |x: f64| (x / xpp).floor();
        for c in 0..width as usize {
            // The drawn samples of column c: finite with a finite neighbour
            // in the column.
            let (a, b) = (
                (c as f64 * xpp).ceil() as usize,
                (((c + 1) as f64) * xpp).ceil() as usize,
            );
            let inside = |i: usize| i >= a && i < b.min(n);
            let drawn = (a..b.min(n)).filter(|&i| {
                values[i].is_finite()
                    && ((i > 0 && inside(i - 1) && values[i - 1].is_finite())
                        || (inside(i + 1) && values[i + 1].is_finite()))
            });
            let (lo, hi) = drawn.fold((f64::MAX, f64::MIN), |(l, h), i| {
                (l.min(values[i]), h.max(values[i]))
            });
            let mut span = (f64::MAX, f64::MIN);
            for r in 0..xs.len() - 1 {
                let (x0, x1, y0, y1) = (xs[r], xs[r + 1], ys[r], ys[r + 1]);
                if [x0, x1, y0, y1].iter().all(|v| v.is_finite())
                    && column(x0) == c as f64
                    && column(x1) == c as f64
                {
                    span = (span.0.min(y0).min(y1), span.1.max(y0).max(y1));
                }
            }
            assert!(
                span.0 <= lo + 1e-12 && span.1 >= hi - 1e-12,
                "column {c}: drawn samples {lo}..{hi}, rows {span:?}"
            );
        }
    }

    /// Every row maps to the kept point it draws, and every kept point's
    /// index maps back to its row: what a pick and the highlight read.
    #[test]
    fn rows_and_kept_indices_map_both_ways() {
        let values = signal(50_000);
        let envelope = envelope(&values, 300);
        let xs = rows(&envelope, 0, true);
        let ys = rows(&envelope, 0, false);
        // A finite row draws the point it maps to. NaN rows (breaks, gaps
        // and empty slots) draw nothing, so their mapping is never read.
        for (row, (&x, &y)) in xs.iter().zip(&ys).enumerate() {
            if !(x.is_finite() && y.is_finite()) {
                continue;
            }
            let index = row_logical(&envelope, 0, row).expect("a finite row maps");
            assert_eq!(x, index as f64, "row {row}");
            // The `(hi, lo)` f32 pair carries about 48 bits.
            assert!((y - values[index]).abs() < 1e-12, "row {row}");
        }
        let kept: Vec<usize> = kept_indices(&envelope, 0).collect();
        assert!(kept.windows(2).all(|w| w[0] < w[1]), "x order");
        for &index in &kept {
            let row = logical_row(&envelope, 0, index).expect("kept");
            assert_eq!(row_logical(&envelope, 0, row), Some(index));
        }
        let dropped = (0..values.len())
            .find(|i| !kept.contains(i) && values[*i].is_finite())
            .expect("a dense envelope drops samples");
        assert_eq!(logical_row(&envelope, 0, dropped), None);
    }
}

//! Monotonic-x line series (#271): the renderer side of the M4 envelope.
//!
//! The CPU core ([`crate::monotonic`]) reduces a group's samples to its
//! envelope. This module keeps one envelope per group and per x-to-pixel
//! mapping its series are drawn at (one, unless a subplot shows the group at
//! another range or width), uploads each series' channel to its two pool
//! columns, and checks wherever a frame, an export or a pick is prepared
//! that the envelope was built for the mapping it is drawn at. The source is
//! an argument to every call and is never retained (M1, M3).
//!
//! # Upload layout
//!
//! A series' envelope is drawn by the line pipeline as it draws any line:
//! from two pool columns, x and y, one segment per pair of consecutive rows.
//! Each envelope slot (the lead edge, the `width + 1` columns, the trail
//! edge) takes [`ROWS_PER_SLOT`] rows: each of the cell's 4 points followed
//! by its join marker. A marker repeats its point when the line goes on to
//! the next point and is NaN when it breaks there; a cell with fewer than 4
//! points repeats its last one. A repeated point makes a zero-length
//! segment, which the stroke collapses to nothing, and a NaN row masks both
//! segments that touch it, as a NaN sample does in any line. An empty slot
//! repeats the last point before it while the line runs across it and is
//! NaN otherwise. So the rows are a fixed function of the slots: row `r`
//! belongs to slot `r / 8` and point `(r % 8) / 2`, which is how a pick's
//! row maps back to a logical sample index and a selection's index to a row,
//! by arithmetic and one look at the envelope. x is `start + index ×
//! interval` in data units, so the projection places every kept point
//! exactly where a full draw places it, slide offset or not.

use std::sync::Arc;

use super::*;
use crate::data::ColumnPairWriter;
use crate::monotonic::{
    CELL_POINTS, EnvelopeCell, MappingChange, MonotonicAppend, MonotonicEnvelope, MonotonicError,
    MonotonicMapping, MonotonicSource, MonotonicSourceRef, MonotonicUpdate,
};

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
fn lend_refs<'m>(
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
    chart: ChartId,
    groups: Vec<(String, Vec<Arc<MonotonicEnvelope>>)>,
    members: Vec<Planned>,
}

/// A series of a plan: its envelope and whether its columns are written.
struct Planned {
    member: Member,
    group: String,
    panel: usize,
    write: bool,
}

/// A full call on one envelope at one mapping: the core's
/// `set_monotonic` on the caller's typed source.
type FullCall<'c> = dyn FnMut(
        &mut MonotonicEnvelope,
        &MonotonicMapping,
    ) -> std::result::Result<MonotonicUpdate, MonotonicError>
    + 'c;

/// An append on one envelope: the core's `append_monotonic` on the
/// caller's typed source.
type AppendCall<'c> =
    dyn FnMut(&mut MonotonicEnvelope) -> std::result::Result<MonotonicUpdate, MonotonicError> + 'c;

/// The renderer's monotonic state: the envelopes of every group of every
/// registered chart, and the pool columns monotonic series draw from.
pub(in crate::renderer) struct MonotonicState {
    /// By chart and group name.
    groups: HashMap<ChartId, HashMap<String, MonotonicGroupState>>,
    /// The envelope columns, x and y of every series, by pool column id.
    columns: HashMap<ColumnId, MonotonicColumn>,
}

/// The envelopes of one group: one per x-to-pixel mapping its series are
/// drawn at, shared with the picks in flight (copied on write only while
/// one holds it).
struct MonotonicGroupState {
    panels: Vec<Arc<MonotonicEnvelope>>,
}

/// Whose envelope a pool column holds.
#[derive(Clone, Debug)]
struct MonotonicColumn {
    chart: ChartId,
    /// The key of the envelopes in the chart's group map: the group's name,
    /// or an export's transient key.
    key: String,
    /// The group the series reads.
    group: String,
    series_id: String,
    /// Index into the group's envelopes.
    panel: usize,
    channel: usize,
}

/// One series of a call: what it draws and where.
struct Member {
    series_id: String,
    x_column: ColumnId,
    y_column: ColumnId,
    channel: usize,
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
    fn envelope_of(&self, cfg: &SeriesConfig) -> Option<(&Arc<MonotonicEnvelope>, usize)> {
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
    fn check_series(&self, panel: &Config, cfg: &SeriesConfig) -> Result<()> {
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
    fn kept_point(&self, cfg: &SeriesConfig, index: usize) -> Option<(f64, f64)> {
        let (envelope, channel) = self.envelope_of(cfg)?;
        let row = logical_row(envelope, channel, index)?;
        let (i, value) = row_point(envelope, channel, row)?;
        Some((envelope.grid()?.x(i), value))
    }

    /// Drops chart `chart`'s monotonic state and returns the envelope
    /// columns it held, for removal from the pool.
    fn forget_chart(&mut self, chart: ChartId) -> Vec<ColumnId> {
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
    fn has_chart(&self, chart: ChartId) -> bool {
        self.groups
            .get(&chart)
            .is_some_and(|groups| !groups.is_empty())
            || self.columns.values().any(|record| record.chart == chart)
    }

    /// Whether series `cfg`'s columns already hold channel `channel` of
    /// `envelope`, the envelope of group `group` of chart `chart` at its
    /// recorded place: then a change writes nothing.
    fn holds(
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
    fn change_of(
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
    fn forget_group(&mut self, chart: ChartId, group: &str) -> Vec<ColumnId> {
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
/// The x axis series `series_id` of `config` is drawn against: its panel's
/// `bottom_x`, or the one a linked subplot's x follows.
fn x_axis_mut<'c>(
    config: &'c mut Config,
    series_id: &str,
) -> Option<&'c mut crate::config::AxisOptions> {
    match config.layer_of_series(series_id) {
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
fn place_plot_area(config: &mut Config, target: crate::layout::Rect) -> Result<()> {
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

fn plot_extent(envelope: &MonotonicEnvelope, channel: usize) -> crate::chart::FitExtent {
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

/// The config of the panel layer `layer` is drawn on: the document's, or
/// a subplot's (the root of its x link chain, whose x axis it shows).
fn panel_config_mut(config: &mut Config, layer: crate::LayerRef) -> Option<&mut Config> {
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
fn fit_y_axis(config: &mut Config, layer: crate::LayerRef, extent: &crate::chart::FitExtent) {
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
struct DocumentPanels<'c> {
    document: &'c Config,
    expanded: Vec<Config>,
}

impl<'c> DocumentPanels<'c> {
    fn new(document: &'c Config) -> Result<Self> {
        let expanded = if is_layered(document) {
            expand_for_render(document)?
        } else {
            Vec::new()
        };
        Ok(Self { document, expanded })
    }

    /// The config of the panel series `series_id` is drawn on.
    fn of(&self, series_id: &str) -> &Config {
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
fn kept_indices(envelope: &MonotonicEnvelope, channel: usize) -> impl Iterator<Item = usize> + '_ {
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
struct EnvelopeRows<'a> {
    envelope: &'a MonotonicEnvelope,
    channel: usize,
    /// The x column (`start + index × interval`) or the y column.
    x: bool,
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
    fn monotonic_members(&self, chart: ChartId, group: &str) -> Result<Vec<Member>> {
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

    /// The full call with the caller's typed source behind `build`: the x
    /// range `x` goes through the shared step with the group's source, which
    /// reduces the group at the new mapping; with `autoscale` the y range is
    /// then fitted to what that reduction kept, a change that moves no
    /// mapping.
    fn set_monotonic_unreported(
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
            if autoscale {
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
        for (series_id, layer) in &members {
            let Some(cfg) = state.series.iter().find(|cfg| cfg.series_id == *series_id) else {
                continue;
            };
            let Some(extent) = self
                .monotonic
                .envelope_of(cfg)
                .map(|(envelope, channel)| plot_extent(envelope, channel))
            else {
                continue;
            };
            fit_y_axis(&mut config, *layer, &extent);
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
    /// change of `placement`.
    fn placed_for(&self, id: ChartId, config: &Config, placement: Placement) -> Result<Config> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let placed = match (placement, &state.display) {
            (Placement::Surface(None), _) | (Placement::Current | Placement::Hold(_), None) => {
                config.clone()
            }
            (Placement::Surface(Some(surface)), _) => display_config_for_surface(config, surface).0,
            // A release fits the document to the surface again; a hold
            // keeps the placement it begins at, the current one.
            (Placement::Hold(false), Some(display)) if display.hold.is_some() => {
                display_config_for_surface(config, display.surface).0
            }
            (Placement::Current | Placement::Hold(_), Some(display)) => display.place(config).0,
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
    fn append_monotonic_unreported(
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
        let mut member_panels = Vec::with_capacity(members.len());
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

    /// Replaces pool column `id` with `rows` through the renderer's upsert
    /// (chart revisions, picker and resident display caches follow). An
    /// envelope of the same width replaces one of the same size: the hole
    /// it leaves is the next one's, so it asks for no defragmentation.
    fn upload_envelope_column(&mut self, id: &str, rows: &EnvelopeRows<'_>) -> Result<()> {
        let same_size = self
            .pool
            .slot(id)
            .is_some_and(|slot| slot.len_values == rows.len());
        let pending = self.pending_defrag;
        let invalidation = self.prepare_column_invalidation(id)?;
        let old_layout_generation = self.pool.layout_generation();
        let inner = self.pool.begin_upsert_column(
            id.to_owned(),
            rows,
            pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger),
        )?;
        finish_renderer_column_upsert(
            inner,
            &self.streaming_sources,
            id,
            old_layout_generation,
            &mut self.chart_states,
            &mut self.visual_revision,
            &mut self.pending_defrag,
            &mut self.picker,
            Some(invalidation),
            &self.device,
            &self.queue,
            self.errorbar_extent_engine.as_ref(),
        )?
        .commit();
        if same_size {
            self.pending_defrag = pending && self.pending_defrag;
        }
        Ok(())
    }

    /// Builds, for an export at another mapping than the screen's, the
    /// envelopes of `series` (the export's copy of the chart's series) at the
    /// export's panels (`export_config`, as the export places them) from `sources`,
    /// uploads them to transient columns and points the copies at them. Returns the transient column ids, which
    /// [`Self::end_monotonic_export`] removes. A series whose envelope
    /// already matches the export's mapping keeps its columns; one that
    /// matches nothing and has no source in `sources` fails with
    /// [`FiggyError::MonotonicSourceRequired`].
    pub(in crate::renderer) fn begin_monotonic_export(
        &mut self,
        chart: ChartId,
        export_config: &Config,
        series: &mut [SeriesConfig],
        sources: &[MonotonicGroupRef<'_>],
    ) -> Result<Vec<ColumnId>> {
        let mut transient = Vec::new();
        if !series.iter().any(|s| s.monotonic().is_some()) {
            return Ok(transient);
        }
        let panels = DocumentPanels::new(export_config)?;
        for index in 0..series.len() {
            let cfg = &series[index];
            let Some(monotonic) = cfg.monotonic() else {
                continue;
            };
            let panel = panels.of(&cfg.series_id);
            if self.monotonic.check_series(panel, cfg).is_ok() {
                continue;
            }
            let mapping = panel_mapping(panel);
            let Some(data) = sources.iter().find(|s| s.group == monotonic.group) else {
                let change = self
                    .monotonic
                    .check_series(panel, cfg)
                    .err()
                    .and_then(|error| match error {
                        FiggyError::MonotonicSourceRequired { change, .. } => Some(change),
                        _ => None,
                    })
                    .unwrap_or(MappingChange::NotBuilt);
                // A group without data draws nothing, at any scale.
                if change == MappingChange::NotBuilt {
                    continue;
                }
                return Err(FiggyError::MonotonicSourceRequired {
                    series_id: cfg.series_id.clone(),
                    change,
                });
            };
            let channel = monotonic.channel;
            let channels = data.source.channel_count();
            if channel >= channels {
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    kind: crate::RuleKind::MonotonicChannel { channel, channels },
                });
            }
            let mut envelope = MonotonicEnvelope::empty();
            envelope
                .set_monotonic_ref(data.source, &mapping)
                .map_err(|error| FiggyError::Monotonic {
                    group: monotonic.group.clone(),
                    error,
                })?;
            let group = monotonic.group.clone();
            let series_id = cfg.series_id.clone();
            let ids = [
                format!("{}\u{1}export", cfg.x_column),
                format!("{}\u{1}export", cfg.y_column),
            ];
            let envelope = Arc::new(envelope);
            for (id, x) in [(&ids[0], true), (&ids[1], false)] {
                if self.pool.slot(id).is_some() || self.monotonic.owns_column(id) {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: series_id.clone(),
                        kind: crate::RuleKind::MonotonicColumnTaken { column: id.clone() },
                    });
                }
                let rows = EnvelopeRows {
                    envelope: &envelope,
                    channel,
                    x,
                };
                let upload = self.upload_envelope_column(id, &rows);
                if let Err(error) = upload {
                    self.end_monotonic_export(&transient);
                    return Err(error);
                }
                transient.push(id.clone());
            }
            // The transient columns are drawn as one more group of this
            // chart, under its own name, and checked like any other.
            let key = format!("{group}\u{1}export");
            let panel_index = {
                let groups = self.monotonic.groups.entry(chart).or_default();
                let state = groups
                    .entry(key.clone())
                    .or_insert_with(|| MonotonicGroupState { panels: Vec::new() });
                state.panels.push(envelope);
                state.panels.len() - 1
            };
            for id in &ids {
                self.monotonic.columns.insert(
                    id.clone(),
                    MonotonicColumn {
                        chart,
                        key: key.clone(),
                        group: group.clone(),
                        series_id: series_id.clone(),
                        panel: panel_index,
                        channel,
                    },
                );
            }
            let [x, y] = ids;
            series[index].x_column = x;
            series[index].y_column = y;
        }
        Ok(transient)
    }

    /// Removes an export's transient envelope columns and their groups.
    pub(in crate::renderer) fn end_monotonic_export(&mut self, transient: &[ColumnId]) {
        for id in transient {
            if let Some(record) = self.monotonic.columns.remove(id) {
                if let Some(groups) = self.monotonic.groups.get_mut(&record.chart) {
                    groups.remove(&record.key);
                }
            }
            // A failed removal leaves an unreferenced column the next
            // export's id check refuses; the export's own result stands.
            if self.remove_column_impl(id, None).is_err() {
                continue;
            }
        }
    }

    /// Drops chart `chart`'s monotonic state and its envelope columns, when
    /// the chart is removed.
    /// Drops group `group` of chart `chart` and removes its envelope
    /// columns: after a failed append its series draw nothing rather than
    /// an envelope the group no longer has.
    fn forget_monotonic_group(&mut self, chart: ChartId, group: &str) {
        for id in self.monotonic.forget_group(chart, group) {
            if self.remove_envelope_column(&id).is_err() {
                continue;
            }
        }
    }

    /// Removes envelope column `id`, whose record is already dropped. A
    /// series that still names it keeps its place in the document and draws
    /// nothing; the active pick registry, which would read the column, is
    /// rebuilt at the next pick.
    fn remove_envelope_column(&mut self, id: &str) -> Result<bool> {
        let named = self.picker.active.as_ref().is_some_and(|active| {
            self.chart_states
                .get(&active.chart_id)
                .is_some_and(|state| {
                    state
                        .series
                        .iter()
                        .any(|series| series_references_column(series, id))
                })
        });
        if named {
            self.picker.active = None;
        }
        self.remove_column_impl(id, None)
    }

    pub(in crate::renderer) fn forget_monotonic_chart(&mut self, chart: ChartId) {
        for id in self.monotonic.forget_chart(chart) {
            if self.remove_envelope_column(&id).is_err() {
                continue;
            }
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

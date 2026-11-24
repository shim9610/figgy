//! Value label preparation: the label atlas of each category bar, box plot
//! and radar series that draws value labels, cached by what it is baked
//! from, and the per-frame label layer over it.

use super::*;

/// One series' value label atlases in the scratch cache
/// ([`ScratchEntry::ValueLabels`]).
///
/// A host-held chart or an export draws from `held`, keyed as a contour's
/// label atlas is: by the [`data_render::ContourLabelAtlasInput`] it was
/// baked from (the SSOT strings, the face, the font size at the render
/// scale, the colour, the font policy and the absent background:
/// everything the bake reads) and the font generation, which invalidates
/// an atlas when a host registers a font. Its series configs are the
/// host's, so nothing could tell a changed string from a revision (#443,
/// option A1).
///
/// A registered chart draws from `registered`, by chart: each entry is
/// keyed by the chart's labels revision ([`ChartRevisions::labels`]), the
/// face and the font generation, with no copy of the strings (#443). Its
/// draw and its pick keep an entry each, so a pick at another font size
/// does not evict the draw's atlas, and a pick at the same size shares
/// it. The entries are bounded by the charts and the two roles that draw
/// the series, never by a count of labels (rule 10).
pub(in crate::renderer) struct ValueLabelScratch {
    held: Option<HeldValueLabelAtlas>,
    registered: HashMap<ChartId, RegisteredValueLabels>,
}

impl ValueLabelScratch {
    /// Drop the entries of registered chart `id`, once it is removed.
    pub(in crate::renderer) fn forget_chart(&mut self, id: ChartId) {
        self.registered.remove(&id);
    }
}

/// The atlas of a host-held chart's or an export's series.
struct HeldValueLabelAtlas {
    atlas_input: data_render::ContourLabelAtlasInput,
    font_generation: u64,
    /// `None` when the strings measure to nothing.
    atlas: Option<Arc<data_render::ValueLabelAtlas>>,
}

/// A registered chart's entries for one series: the draw's and the pick's.
#[derive(Default)]
struct RegisteredValueLabels {
    draw: Option<RegisteredValueLabelAtlas>,
    pick: Option<RegisteredValueLabelAtlas>,
}

impl RegisteredValueLabels {
    fn slot(&self, role: ValueLabelRole) -> Option<&RegisteredValueLabelAtlas> {
        match role {
            ValueLabelRole::Draw => self.draw.as_ref(),
            ValueLabelRole::Pick => self.pick.as_ref(),
        }
    }

    fn slot_mut(&mut self, role: ValueLabelRole) -> &mut Option<RegisteredValueLabelAtlas> {
        match role {
            ValueLabelRole::Draw => &mut self.draw,
            ValueLabelRole::Pick => &mut self.pick,
        }
    }
}

/// One registered atlas and the label cells last built over it.
struct RegisteredValueLabelAtlas {
    labels: RenderRevision,
    face: ValueLabelFace,
    font_generation: u64,
    /// `None` when the strings measure to nothing.
    atlas: Option<Arc<data_render::ValueLabelAtlas>>,
    /// The label uniform, the 16 B cell per label and their group 1, kept
    /// across frames while [`ValueLabelCellsKey`] holds (#443). Only the
    /// draw's entry holds them; a pick at the draw's scale reuses them, and
    /// a pick at another scale's are owned by the picker's prepared-pick
    /// cache (#445).
    cells: Option<(ValueLabelCellsKey, data_render::ValueLabelData)>,
}

impl RegisteredValueLabelAtlas {
    fn matches(&self, labels: RenderRevision, face: ValueLabelFaceRef<'_>, font: u64) -> bool {
        self.labels == labels && self.font_generation == font && self.face.as_ref() == face
    }
}

/// What a registered series' label cells and uniform were built from,
/// besides the atlas they sit in (whose entry is keyed by the labels
/// revision, which also follows the categories): the series and selection
/// revisions (texts, offsets, placements and a radar group's members), the
/// scale, and the uniform itself.
#[derive(Clone, Copy, PartialEq)]
struct ValueLabelCellsKey {
    series: RenderRevision,
    selection: RenderRevision,
    scale_bits: u32,
    params: data_render::ValueLabelParamsGpu,
}

/// The registered chart a series' value labels are drawn or picked for
/// and the revisions its atlas and cells are keyed by (#443): an issued
/// chart id and generations ("Identity and staleness checks").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::renderer) struct ValueLabelSource {
    pub(in crate::renderer) chart: ChartId,
    pub(in crate::renderer) labels: RenderRevision,
    pub(in crate::renderer) series: RenderRevision,
    pub(in crate::renderer) selection: RenderRevision,
}

impl ValueLabelSource {
    pub(in crate::renderer) fn of(chart: ChartId, revisions: &ChartRevisions) -> Self {
        Self {
            chart,
            labels: revisions.labels,
            series: revisions.series,
            selection: revisions.selection,
        }
    }
}

/// Who asks for a registered series' labels: its draw or its pick. Each
/// keeps its own entry ([`ValueLabelScratch`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::renderer) enum ValueLabelRole {
    Draw,
    Pick,
}

impl ValueLabelRole {
    fn other(self) -> Self {
        match self {
            Self::Draw => Self::Pick,
            Self::Pick => Self::Draw,
        }
    }
}

/// A registered series' labels and who asks for them; `None` on a
/// host-held chart or an export.
pub(in crate::renderer) type ValueLabelSlot = Option<(ValueLabelSource, ValueLabelRole)>;

/// The registered value label source of each panel of a preparation: the
/// sources of its items, in item order (one per item, or none at all), and
/// how many panels each item drew as (`group_sizes`, empty when none is
/// layered).
#[derive(Clone, Copy)]
pub(in crate::renderer) struct ValueLabelSources<'s> {
    pub(in crate::renderer) items: &'s [ValueLabelSource],
    pub(in crate::renderer) group_sizes: &'s [usize],
}

impl ValueLabelSources<'_> {
    pub(in crate::renderer) const NONE: ValueLabelSources<'static> = ValueLabelSources {
        items: &[],
        group_sizes: &[],
    };

    /// The source of expanded panel `panel` among `panels`.
    pub(in crate::renderer) fn of(&self, panel: usize, panels: usize) -> Option<ValueLabelSource> {
        if self.group_sizes.is_empty() {
            return (self.items.len() == panels)
                .then(|| self.items.get(panel).copied())
                .flatten();
        }
        if self.group_sizes.len() != self.items.len() {
            return None;
        }
        let mut first = 0usize;
        for (item, &count) in self.items.iter().zip(self.group_sizes) {
            if panel < first + count {
                return Some(*item);
            }
            first += count;
        }
        None
    }
}

/// The face a series' value labels bake in, as the stored entry holds it.
#[derive(Clone, Debug, PartialEq)]
struct ValueLabelFace {
    font_size_bits: u32,
    color_bits: [u32; 4],
    policy: crate::text_render::FontPolicy,
    family: String,
}

impl ValueLabelFace {
    fn as_ref(&self) -> ValueLabelFaceRef<'_> {
        ValueLabelFaceRef {
            font_size_bits: self.font_size_bits,
            color_bits: self.color_bits,
            policy: self.policy,
            family: &self.family,
        }
    }
}

/// The face a series' value labels bake in, borrowed from the config: the
/// value axis' tick label face and size (in render pixels in a scaled
/// config), the label colour and the font policy. What
/// [`data_render::ContourLabelAtlasInput::for_value_labels`] reads besides
/// the strings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::renderer) struct ValueLabelFaceRef<'c> {
    font_size_bits: u32,
    color_bits: [u32; 4],
    policy: crate::text_render::FontPolicy,
    family: &'c str,
}

impl ValueLabelFaceRef<'_> {
    fn to_owned(self) -> ValueLabelFace {
        ValueLabelFace {
            font_size_bits: self.font_size_bits,
            color_bits: self.color_bits,
            policy: self.policy,
            family: self.family.to_owned(),
        }
    }
}

/// Series `cfg`'s label strings and face under the panel `config`,
/// borrowed: `None` when the series draws no value labels or their font
/// size is not positive. Copies nothing (#443).
pub(in crate::renderer) fn value_label_face<'c>(
    config: &'c Config,
    cfg: &'c SeriesConfig,
) -> Option<(&'c [String], ValueLabelFaceRef<'c>)> {
    let selected = series_selected(config, cfg);
    let (_, text) = data_render::value_label_kind(&cfg.render_type, selected)?;
    let axis = value_label_axis(config, &cfg.render_type);
    let size = axis.label_style.font_size;
    if !(size > 0.0) {
        return None;
    }
    let c = value_label_color(config, &cfg.render_type, axis);
    Some((
        text,
        ValueLabelFaceRef {
            font_size_bits: size.to_bits(),
            color_bits: [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()],
            policy: crate::text_render::FontPolicy::for_style(&config.draw_style),
            family: &axis.label_style.label_font,
        },
    ))
}

#[cfg(test)]
thread_local! {
    /// Value label atlas bakes on this thread (#443 tests).
    static VALUE_LABEL_BAKES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The value label atlas bakes this thread has run (#443 tests).
#[cfg(test)]
pub(in crate::renderer) fn value_label_bakes() -> u64 {
    VALUE_LABEL_BAKES.with(std::cell::Cell::get)
}

/// The axis whose tick label face a series' value labels take: the value
/// axis of a category bar or box plot series (`left_y` for vertical groups,
/// `bottom_x` for horizontal ones), the radius axis of a radar series. When
/// that axis shows no tick labels and its opposite one does (`right_y`,
/// `top_x`: an overlay panel's own axis), the opposite one: the labels take
/// the face of the value axis as it is drawn.
pub(in crate::renderer) fn value_label_axis<'c>(
    config: &'c Config,
    render_type: &DataRenderType,
) -> &'c crate::config::AxisOptions {
    let horizontal = render_type.radar().is_none()
        && config.bar_groups.as_ref().is_some_and(|groups| {
            groups.direction == crate::data_config::CategoryBarDirection::Horizontal
        });
    let (axis, opposite) = if horizontal {
        (&config.bottom_x, &config.top_x)
    } else {
        (&config.left_y, &config.right_y)
    };
    let shows =
        |a: &crate::config::AxisOptions| a.label_style.visible && a.label_style.label_visible;
    if !shows(axis) && shows(opposite) && render_type.radar().is_none() {
        opposite
    } else {
        axis
    }
}

/// The categories a series' value labels follow, in axis order: the
/// categorical axis of a bar or box group's direction, a radar chart's
/// angle axis.
pub(in crate::renderer) fn value_label_categories<'c>(
    config: &'c Config,
    render_type: &DataRenderType,
) -> &'c [crate::config::Category] {
    if render_type.radar().is_some() {
        config.bottom_x.categories.as_deref().unwrap_or(&[])
    } else {
        category_bar_categories(config).unwrap_or(&[])
    }
}

/// The contrast ink of series `render_type`'s labels in the panel
/// `config` (#357): a range bar series with `labels.contrast_ink`, over the
/// plot's background (`plot_background`, else the chart's `background`,
/// else white). `None` draws the labels in their baked colour.
pub(in crate::renderer) fn value_label_ink(
    config: &Config,
    render_type: &DataRenderType,
) -> Option<data_render::ValueLabelInk> {
    let bars = render_type.range_bars()?;
    let [dark, light] = bars.labels.contrast_ink?;
    Some(data_render::ValueLabelInk {
        dark,
        light,
        back: config
            .plot_background
            .or(config.background)
            .unwrap_or(Color::WHITE),
        base: bars.color,
    })
}

/// The colour a series' labels are baked in: the value axis' tick label
/// colour, or white over a dark bar its labels sit inside (`Inside`, or
/// `Auto` in a stack), as the former categorical chart wrote them. Labels
/// in contrast ink ([`value_label_ink`]) are baked in white: the shader
/// reads the atlas' alpha as coverage and picks the ink.
fn value_label_color(
    config: &Config,
    render_type: &DataRenderType,
    axis: &crate::config::AxisOptions,
) -> Color {
    if value_label_ink(config, render_type).is_some() {
        return Color::WHITE;
    }
    if let Some(bars) = render_type.category_bars() {
        let stacked = config
            .bar_groups
            .as_ref()
            .is_some_and(|groups| groups.mode != crate::data_config::CategoryBarMode::Grouped);
        let inside = match bars.labels.placement {
            crate::data_config::CategoryBarLabels::Inside => true,
            crate::data_config::CategoryBarLabels::Auto => stacked,
            _ => false,
        };
        let c = bars.color;
        if inside && c.r * 0.2126 + c.g * 0.7152 + c.b * 0.0722 < 0.55 {
            return Color::WHITE;
        }
    }
    axis.label_style.color
}

/// Whether the visible typed selection of `config` names series `cfg` (a
/// vertex, a label or any other datum of it): a radar series' `Emphasized`
/// labels show only then.
fn series_selected(config: &Config, cfg: &SeriesConfig) -> bool {
    config
        .picked_data
        .as_ref()
        .filter(|picked| picked.visible)
        .is_some_and(|picked| {
            picked
                .refs
                .iter()
                .any(|r| picked_data_ref_matches_series(cfg, r))
        })
}

/// The selected value label of series `cfg`, when the visible typed
/// selection holds one.
pub(in crate::renderer) fn selected_value_label(
    config: &Config,
    cfg: &SeriesConfig,
) -> Option<u32> {
    let picked = config
        .picked_data
        .as_ref()
        .filter(|picked| picked.visible)?;
    picked.refs.iter().find_map(|r| match r {
        crate::config::PickedDataRef::ValueLabel { category_index, .. }
            if picked_data_ref_matches_series(cfg, r) =>
        {
            u32::try_from(*category_index).ok()
        }
        _ => None,
    })
}

/// The members of a radar label group in the panel `config` (#272): the
/// radar series of `series` that draw value labels, in series order, each
/// with its index in `series`. Their labels at one spoke are placed
/// together; the group's rank, lane table and label cells all follow this
/// one order.
pub(in crate::renderer) fn radar_label_members<'s>(
    config: &Config,
    series: impl IntoIterator<Item = &'s SeriesConfig>,
) -> impl Iterator<Item = (usize, &'s SeriesConfig)> {
    series
        .into_iter()
        .enumerate()
        .filter(move |(_, s)| s.render_type.radar().is_some() && draws_value_labels(config, s))
}

/// Series `cfg`'s label group among `series` in the panel `config`: for a
/// radar series that draws value labels, its rank among the group's
/// members ([`radar_label_members`]) and their count, the visual gap two
/// labels keep (#272, the label font's `visual_gap_for`), the radius
/// axis' ray and the box of the drawn ring labels ([`ring_label_band`]);
/// [`data_render::RadarLabelGroup::NONE`] for any other series.
pub(in crate::renderer) fn radar_label_group<'s>(
    config: &Config,
    series: impl IntoIterator<Item = &'s SeriesConfig>,
    cfg: &SeriesConfig,
) -> data_render::RadarLabelGroup {
    let mut group = data_render::RadarLabelGroup::NONE;
    let Some(polar) = config.polar().filter(|_| cfg.render_type.radar().is_some()) else {
        return group;
    };
    group.count = 0;
    for (_, s) in radar_label_members(config, series) {
        if s.series_id == cfg.series_id {
            group.rank = group.count;
        }
        group.count += 1;
    }
    group.count = group.count.max(1);
    let font = value_label_axis(config, &cfg.render_type)
        .label_style
        .font_size;
    group.visual_gap = crate::chrome::visual_gap_for(font, font);
    group.ray = polar.ray_direction();
    group.ring = ring_label_band(config);
    group
}

/// The box of the ring (radius) labels the polar panel `config` draws, in
/// the frame of its ray from the centre as fractions of the outer radius
/// (`RadarLabelGroup::ring`): the drawn boxes the chrome emits, so a moved,
/// hidden or thinned label counts as drawn. Empty without ring labels.
fn ring_label_band(config: &Config) -> [f32; 4] {
    let empty = data_render::RadarLabelGroup::NONE.ring;
    let (Some(polar), Some(geometry)) = (config.polar(), config.polar_geometry()) else {
        return empty;
    };
    if !(geometry.radius > 0.0) {
        return empty;
    }
    let [dx, dy] = polar.ray_direction();
    let [sx, sy] = polar.ray_side();
    let [cx, cy] = geometry.center;
    let measure = crate::text_render::CpuTextMeasure::for_style(&config.draw_style);
    let mut band = [
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ];
    crate::select::emit_element(
        config,
        &crate::layout::Element::AxisLabel(crate::layout::Side::Left),
        &measure,
        &mut crate::sink::Parts(&mut |part| {
            let r = part.extent();
            for (x, y) in [
                (r.x, r.y),
                (r.x + r.width, r.y),
                (r.x, r.y + r.height),
                (r.x + r.width, r.y + r.height),
            ] {
                let along = (x - cx) * dx + (y - cy) * dy;
                let across = (x - cx) * sx + (y - cy) * sy;
                band = [
                    band[0].min(along),
                    band[1].max(along),
                    band[2].min(across),
                    band[3].max(across),
                ];
            }
        }),
    );
    if band.iter().all(|v| v.is_finite()) {
        band.map(|v| v / geometry.radius)
    } else {
        empty
    }
}

/// Whether series `cfg` draws value labels under the panel `config`: the
/// value-label step of its panel paints only then.
pub(crate) fn draws_value_labels(config: &Config, cfg: &SeriesConfig) -> bool {
    data_render::value_label_kind(&cfg.render_type, series_selected(config, cfg)).is_some()
}

/// What series `cfg`'s labels bake from under the panel `config`, or
/// `None` when the series draws no value labels.
pub(in crate::renderer) fn value_label_atlas_input(
    config: &Config,
    cfg: &SeriesConfig,
) -> Option<data_render::ContourLabelAtlasInput> {
    let selected = series_selected(config, cfg);
    let (_, text) = data_render::value_label_kind(&cfg.render_type, selected)?;
    let axis = value_label_axis(config, &cfg.render_type);
    let color = value_label_color(config, &cfg.render_type, axis);
    // The atlas holds a cell per string, in axis order: label `i` samples
    // cell `i`. The SSOT strings themselves are the key, as a contour's
    // are; any number of them (#107 R11).
    let texts = text.to_vec();
    data_render::ContourLabelAtlasInput::for_value_labels(config, axis, texts, color)
}

impl PrepareContext<'_> {
    /// The atlas series `cfg`'s value labels draw from under `config`,
    /// baked or reused; `None` when the series draws no value labels.
    /// `slot` names the registered chart and role ([`ValueLabelScratch`]);
    /// `None` keys the atlas by its strings.
    pub(in crate::renderer) fn ensure_value_label_scratch(
        &mut self,
        config: &Config,
        cfg: &SeriesConfig,
        slot: ValueLabelSlot,
    ) -> Result<Option<Arc<data_render::ValueLabelAtlas>>> {
        let Some((source, role)) = slot else {
            return self.ensure_held_value_label_atlas(config, cfg);
        };
        let Some((text, face)) = value_label_face(config, cfg) else {
            return Ok(None);
        };
        let font_generation = crate::text_render::font_generation();
        if let Some(ScratchEntry::ValueLabels(scratch)) =
            self.contour_cache.get_mut(cfg.series_id.as_str())
            && let Some(entries) = scratch.registered.get_mut(&source.chart)
        {
            if let Some(own) = entries
                .slot(role)
                .filter(|entry| entry.matches(source.labels, face, font_generation))
            {
                return Ok(own.atlas.clone());
            }
            // The other role's atlas at the same font size: shared, so a
            // pick at the draw's scale bakes nothing.
            if let Some(other) = entries
                .slot(role.other())
                .filter(|entry| entry.matches(source.labels, face, font_generation))
            {
                let shared = RegisteredValueLabelAtlas {
                    labels: other.labels,
                    face: other.face.clone(),
                    font_generation,
                    atlas: other.atlas.clone(),
                    cells: None,
                };
                let atlas = shared.atlas.clone();
                *entries.slot_mut(role) = Some(shared);
                return Ok(atlas);
            }
        }
        // The strings are copied into the bake's input for the bake only;
        // the entry keeps none of them.
        let input = data_render::ContourLabelAtlasInput {
            texts: text.to_vec(),
            font_size_bits: face.font_size_bits,
            color_bits: face.color_bits,
            policy: face.policy,
            family: face.family.to_owned(),
            bg_color_bits: None,
            bg_padding_bits: 0.0f32.to_bits(),
        };
        let atlas = self.bake_value_label_atlas(cfg, &input)?;
        drop(input);
        let entry = RegisteredValueLabelAtlas {
            labels: source.labels,
            face: face.to_owned(),
            font_generation,
            atlas: atlas.clone(),
            cells: None,
        };
        let scratch = self.value_label_scratch_mut(cfg)?;
        scratch
            .registered
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "value label atlas entries",
                reason: error.to_string(),
            })?;
        *scratch
            .registered
            .entry(source.chart)
            .or_default()
            .slot_mut(role) = Some(entry);
        Ok(atlas)
    }

    /// A host-held chart's or an export's atlas, keyed by its strings.
    fn ensure_held_value_label_atlas(
        &mut self,
        config: &Config,
        cfg: &SeriesConfig,
    ) -> Result<Option<Arc<data_render::ValueLabelAtlas>>> {
        let Some(atlas_input) = value_label_atlas_input(config, cfg) else {
            return Ok(None);
        };
        let font_generation = crate::text_render::font_generation();
        if let Some(ScratchEntry::ValueLabels(scratch)) =
            self.contour_cache.get(cfg.series_id.as_str())
            && let Some(held) = &scratch.held
            && held.atlas_input == atlas_input
            && held.font_generation == font_generation
        {
            return Ok(held.atlas.clone());
        }
        let atlas = self.bake_value_label_atlas(cfg, &atlas_input)?;
        self.value_label_scratch_mut(cfg)?.held = Some(HeldValueLabelAtlas {
            atlas_input,
            font_generation,
            atlas: atlas.clone(),
        });
        Ok(atlas)
    }

    /// Bake and upload `input`; `None` when the strings measure to nothing.
    fn bake_value_label_atlas(
        &mut self,
        cfg: &SeriesConfig,
        input: &data_render::ContourLabelAtlasInput,
    ) -> Result<Option<Arc<data_render::ValueLabelAtlas>>> {
        #[cfg(test)]
        VALUE_LABEL_BAKES.with(|bakes| bakes.set(bakes.get() + 1));
        let baked =
            axis_render::bake_contour_label_atlas(input, self.caps.max_texture_dimension_2d)
                .map_err(|error| FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    kind: crate::RuleKind::ValueLabels {
                        detail: error.to_string(),
                    },
                })?;
        Ok(match baked {
            Some(baked) => Some(Arc::new(data_render::ValueLabelAtlas::upload(
                self.device,
                self.queue,
                self.gpu_ledger,
                self.caps.max_texture_dimension_2d,
                baked,
            )?)),
            None => None,
        })
    }

    /// Series `cfg`'s entry in the scratch cache, made when it holds none
    /// (or another kind's): the contour scratch cache, one map, one
    /// eviction rule, one limit.
    fn value_label_scratch_mut(&mut self, cfg: &SeriesConfig) -> Result<&mut ValueLabelScratch> {
        let id = cfg.series_id.as_str();
        if !matches!(
            self.contour_cache.get(id),
            Some(ScratchEntry::ValueLabels(_))
        ) {
            if !self.in_frame {
                evict_if_full(self.contour_cache, id, CONTOUR_SCRATCH_CACHE_LIMIT);
            }
            self.contour_cache.try_reserve(1).map_err(|error| {
                FiggyError::StateAllocationFailed {
                    resource: "contour scratch cache",
                    reason: error.to_string(),
                }
            })?;
            self.contour_cache.insert(
                cfg.series_id.clone(),
                ScratchEntry::ValueLabels(ValueLabelScratch {
                    held: None,
                    registered: HashMap::new(),
                }),
            );
        }
        match self.contour_cache.get_mut(id) {
            Some(ScratchEntry::ValueLabels(scratch)) => Ok(scratch),
            _ => Err(FiggyError::internal("value label scratch was just made")),
        }
    }

    /// The atlas series `cfg`'s value labels draw from under `config` when
    /// the scratch cache holds it as baked for them now (what
    /// [`Self::ensure_value_label_scratch`] resolved for `slot`), else
    /// `None`.
    pub(in crate::renderer) fn resident_value_label_atlas(
        &self,
        config: &Config,
        cfg: &SeriesConfig,
        slot: ValueLabelSlot,
    ) -> Option<Arc<data_render::ValueLabelAtlas>> {
        let Some(ScratchEntry::ValueLabels(scratch)) =
            self.contour_cache.get(cfg.series_id.as_str())
        else {
            return None;
        };
        let font_generation = crate::text_render::font_generation();
        match slot {
            Some((source, role)) => {
                let (_, face) = value_label_face(config, cfg)?;
                let entries = scratch.registered.get(&source.chart)?;
                [role, role.other()]
                    .into_iter()
                    .filter_map(|role| entries.slot(role))
                    .find(|entry| entry.matches(source.labels, face, font_generation))
                    .and_then(|entry| entry.atlas.clone())
            }
            None => {
                let atlas_input = value_label_atlas_input(config, cfg)?;
                scratch
                    .held
                    .as_ref()
                    .filter(|held| {
                        held.atlas_input == atlas_input && held.font_generation == font_generation
                    })
                    .and_then(|held| held.atlas.clone())
            }
        }
    }

    /// Series `cfg`'s value label layer data as [`Self::value_label_data`]
    /// builds it, kept in its registered entry (`slot`) and reused while
    /// the series and selection revisions, the scale, the uniform and the
    /// atlas are the same (#443): an idle frame, a pan and a repeated pick
    /// build no label buffer. Without a registered slot it is built anew.
    /// `member_atlas` resolves a radar group member's atlas, as
    /// [`Self::value_label_data`]'s does, with this context lent.
    pub(in crate::renderer) fn ensure_value_label_data<'s>(
        &mut self,
        config: &Config,
        series: impl IntoIterator<Item = &'s SeriesConfig> + Clone,
        cfg: &SeriesConfig,
        atlas: &Arc<data_render::ValueLabelAtlas>,
        scale: f32,
        slot: ValueLabelSlot,
        mut member_atlas: impl FnMut(
            &Self,
            usize,
            &SeriesConfig,
        ) -> Result<Option<Arc<data_render::ValueLabelAtlas>>>,
    ) -> Result<Option<data_render::ValueLabelData>> {
        let Some((source, role)) = slot else {
            let this = &*self;
            return this.value_label_data(config, series, cfg, atlas, scale, |index, member| {
                member_atlas(this, index, member)
            });
        };
        let Some(params) = self.value_label_uniform(config, series.clone(), cfg, atlas, scale)
        else {
            return Ok(None);
        };
        let key = ValueLabelCellsKey {
            series: source.series,
            selection: source.selection,
            scale_bits: scale.to_bits(),
            params,
        };
        // The entry whose atlas this is: the role's own, else the other's
        // (shared storage, `Arc::ptr_eq`).
        let holds = |entry: &RegisteredValueLabelAtlas| {
            entry
                .atlas
                .as_ref()
                .is_some_and(|held| Arc::ptr_eq(held, atlas))
        };
        if let Some(ScratchEntry::ValueLabels(scratch)) =
            self.contour_cache.get(cfg.series_id.as_str())
            && let Some(entries) = scratch.registered.get(&source.chart)
            && let Some(data) = [role, role.other()]
                .into_iter()
                .filter_map(|role| entries.slot(role))
                .filter(|entry| holds(entry))
                .find_map(|entry| entry.cells.as_ref().filter(|(k, _)| *k == key))
                .map(|(_, data)| data.clone())
        {
            return Ok(Some(data));
        }
        let this = &*self;
        let data = this.value_label_data(config, series, cfg, atlas, scale, |index, member| {
            member_atlas(this, index, member)
        })?;
        // The draw's entry owns the data it keeps. A pick's is owned by
        // the picker's prepared-pick cache, which keys it by the chart's
        // desired revision and the display (#445): one owner each, so the
        // pick entry keeps only its atlas.
        if role == ValueLabelRole::Draw
            && let Some(data) = &data
            && let Some(ScratchEntry::ValueLabels(scratch)) =
                self.contour_cache.get_mut(cfg.series_id.as_str())
            && let Some(entries) = scratch.registered.get_mut(&source.chart)
            && let Some(entry) = entries.slot_mut(role).as_mut().filter(|entry| holds(entry))
        {
            entry.cells = Some((key, data.clone()));
        }
        Ok(data)
    }

    /// Series `cfg`'s label uniform in the panel `config` over `atlas` at
    /// `scale`, as [`Self::value_label_data`] writes it.
    fn value_label_uniform<'s>(
        &self,
        config: &Config,
        series: impl IntoIterator<Item = &'s SeriesConfig>,
        cfg: &SeriesConfig,
        atlas: &data_render::ValueLabelAtlas,
        scale: f32,
    ) -> Option<data_render::ValueLabelParamsGpu> {
        let selected = series_selected(config, cfg);
        let (kind, text) = data_render::value_label_kind(&cfg.render_type, selected)?;
        let axis = value_label_axis(config, &cfg.render_type);
        let categories = value_label_categories(config, &cfg.render_type);
        let count = data_render::value_label_count(kind, text, categories);
        let group = radar_label_group(config, series, cfg);
        Some(data_render::value_label_params(
            kind,
            count,
            atlas,
            axis.label_style.font_size,
            scale,
            &group,
            value_label_ink(config, &cfg.render_type),
        ))
    }

    /// Series `cfg`'s value label layer data in the panel `config` at
    /// `scale` render pixels per logical pixel: its label uniform, its
    /// label cells (one per label, any number) and group 1 over `atlas`.
    /// A radar series' cells hold a block per member of its label group
    /// among `series` ([`radar_label_group`]), in member order, each
    /// written from that member's atlas (`member_atlas`, given the member's
    /// index in `series`), so its labels are placed with the others at
    /// their spokes.
    pub(in crate::renderer) fn value_label_data<'s>(
        &self,
        config: &Config,
        series: impl IntoIterator<Item = &'s SeriesConfig> + Clone,
        cfg: &SeriesConfig,
        atlas: &data_render::ValueLabelAtlas,
        scale: f32,
        mut member_atlas: impl FnMut(
            usize,
            &SeriesConfig,
        ) -> Result<Option<Arc<data_render::ValueLabelAtlas>>>,
    ) -> Result<Option<data_render::ValueLabelData>> {
        let selected = series_selected(config, cfg);
        let Some((kind, text)) = data_render::value_label_kind(&cfg.render_type, selected) else {
            return Ok(None);
        };
        let axis = value_label_axis(config, &cfg.render_type);
        let categories = value_label_categories(config, &cfg.render_type);
        let count = data_render::value_label_count(kind, text, categories);
        let group = radar_label_group(config, series.clone(), cfg);
        let params = data_render::value_label_params(
            kind,
            count,
            atlas,
            axis.label_style.font_size,
            scale,
            &group,
            value_label_ink(config, &cfg.render_type),
        );
        data_render::create_value_label_data(
            self.device,
            self.gpu_ledger,
            self.value_label_bgl,
            atlas,
            self.sampler,
            &params,
            |push| {
                if kind != data_render::VALUE_LABEL_KIND_RADAR || group.count <= 1 {
                    data_render::value_label_cells(
                        &cfg.render_type,
                        text,
                        cfg.value_label_offsets().unwrap_or(&[]),
                        categories,
                        count,
                        atlas,
                        scale,
                        push,
                    );
                    return Ok(());
                }
                // One block of `count` cells per member, in member order:
                // a member's own cells, padded with empty ones (no label).
                for (index, member) in radar_label_members(config, series) {
                    let own = member.series_id == cfg.series_id;
                    let resolved = if own {
                        None
                    } else {
                        member_atlas(index, member)?
                    };
                    let member_text = data_render::value_label_kind(
                        &member.render_type,
                        series_selected(config, member),
                    )
                    .map_or(&[][..], |(_, text)| text);
                    let mut written = 0usize;
                    let mut pad = |cell: [f32; 4]| {
                        if written < count {
                            push(cell);
                            written += 1;
                        }
                    };
                    if let Some(member_atlas) = resolved.as_deref().or(own.then_some(atlas)) {
                        data_render::value_label_cells(
                            &member.render_type,
                            member_text,
                            member.value_label_offsets().unwrap_or(&[]),
                            categories,
                            count,
                            member_atlas,
                            scale,
                            &mut pad,
                        );
                    }
                    while written < count {
                        push([0.0; 4]);
                        written += 1;
                    }
                }
                Ok(())
            },
        )
        .map(Some)
    }

    /// The category bar group 3 a radar series' value labels read: its
    /// category column, one label per row, and the value columns of its
    /// label group's members among `series` in the panel `config`
    /// ([`radar_label_group`]), its own at its rank. Each member holds one
    /// value per category, as its polygon does.
    pub(in crate::renderer) fn radar_label_columns<'s>(
        &self,
        config: &Config,
        series: impl IntoIterator<Item = &'s SeriesConfig> + Clone,
        cfg: &SeriesConfig,
        column_buffer: &wgpu::Buffer,
        scale: f32,
        lookup: &mut impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<data_render::CategoryBarData> {
        let x = lookup(&cfg.x_column)?;
        let y = lookup(&cfg.y_column)?;
        let lane = |handle: &ColumnHandle| -> Result<u32> {
            u32::try_from(handle.offset / 4).map_err(|_| FiggyError::GpuResourceLimit {
                resource: "radar label column lane",
                requested: handle.offset / 4,
                limit: u64::from(u32::MAX),
            })
        };
        let count = u32::try_from(x.len_values.min(y.len_values)).unwrap_or(u32::MAX);
        let group = radar_label_group(config, series.clone(), cfg);
        let params = data_render::radar_label_params(lane(&x)?, count, &group, scale);
        let style_map = data_render::create_bar_style_map_with_slots(
            self.device,
            self.per_point_style_map_bgl,
            &[],
            &[],
        )?;
        let own = lane(&y)?;
        data_render::create_category_bar_data_bind_group(
            self.device,
            self.gpu_ledger,
            self.category_bar_data_bgl,
            column_buffer,
            &params,
            group.count.max(1) as usize,
            |push| {
                if group.count <= 1 {
                    push(own);
                    return Ok(());
                }
                for (_, member) in radar_label_members(config, series) {
                    let mx = lookup(&member.x_column)?;
                    let my = lookup(&member.y_column)?;
                    radar_columns_fit(config, member, mx.len_values, my.len_values)?;
                    push(lane(&my)?);
                }
                Ok(())
            },
            style_map,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_config::CategoryBarSeries;
    use crate::data_config::{CategoryBarLabels, CategoryBarSeriesLabels, CategoryBarStyle};

    fn chart() -> Config {
        let mut config = crate::default::default_config();
        config.bottom_x.categories = Some(
            vec![
                crate::config::Category::new("a", "A"),
                crate::config::Category::new("b", "B"),
            ]
            .into(),
        );
        config
    }

    fn bars(placement: CategoryBarLabels, color: Color) -> SeriesConfig {
        SeriesConfig {
            series_id: "s".into(),
            source_id: None,
            label: None,
            x_column: "pos".into(),
            y_column: "v".into(),
            z_column: None,
            render_type: DataRenderType::CategoryBars(CategoryBarSeries {
                color,
                style: CategoryBarStyle::default(),
                labels: CategoryBarSeriesLabels {
                    placement,
                    text: vec!["1.5".into(), "2".into()],
                    ..CategoryBarSeriesLabels::default()
                },
                label_format: Default::default(),
                overrides: None,
            }),
        }
    }

    fn text_mut(series: &mut SeriesConfig) -> &mut Vec<String> {
        let DataRenderType::CategoryBars(CategoryBarSeries { labels, .. }) =
            &mut series.render_type
        else {
            unreachable!()
        };
        &mut labels.text
    }

    /// The atlas key names every input of the bake (#66): changing any of
    /// the strings, the face, the size, the scale, the colour (the axis'
    /// or the white over a dark bar a label sits inside) or the font policy
    /// is a new key, so the cache bakes again; what the bake does not read
    /// (a drag offset, the category axis' face) keeps the key, and a series
    /// that shows no label has none.
    #[test]
    fn the_value_label_atlas_key_names_every_bake_input() {
        let config = chart();
        let series = bars(CategoryBarLabels::Outside, Color::from_rgb8(250, 220, 40));
        let key = |c: &Config, s: &SeriesConfig| value_label_atlas_input(c, s).expect("labels");
        let base = key(&config, &series);
        assert_eq!(base.texts, ["1.5", "2"]);

        let mut changed = series.clone();
        text_mut(&mut changed)[1] = "20".into();
        assert_ne!(key(&config, &changed), base, "a string");
        for (what, edit) in [
            (
                "the font size",
                (|c: &mut Config| c.left_y.label_style.font_size += 1.0) as fn(&mut Config),
            ),
            ("the colour", |c| {
                c.left_y.label_style.color = Color::from_rgb8(1, 2, 3)
            }),
            ("the face", |c| {
                c.left_y.label_style.label_font = "serif".into()
            }),
            ("the font policy", |c| {
                c.draw_style = crate::config::DrawStyle::Sketch(Default::default())
            }),
        ] {
            let mut c = config.clone();
            edit(&mut c);
            assert_ne!(key(&c, &series), base, "{what}");
        }
        assert_ne!(key(&config.scaled(2.0), &series), base, "the scale");
        // Inside a dark bar the labels are white; over a light one, the
        // axis' colour.
        let light = key(
            &config,
            &bars(CategoryBarLabels::Inside, Color::from_rgb8(250, 220, 40)),
        );
        let dark = key(
            &config,
            &bars(CategoryBarLabels::Inside, Color::from_rgb8(20, 30, 90)),
        );
        assert_ne!(light, dark, "the ink over a dark bar");
        assert_eq!(light, base);

        let mut moved = series.clone();
        let _ = moved.drag_value_label(1, 2, 5.0, 5.0);
        assert_eq!(key(&config, &moved), base, "a drag offset");
        let mut c = config.clone();
        c.bottom_x.label_style.font_size += 3.0;
        assert_eq!(key(&c, &series), base, "the category axis' face");
        assert_eq!(
            value_label_atlas_input(&config, &bars(CategoryBarLabels::None, Color::BLACK)),
            None
        );
        let mut empty = series.clone();
        text_mut(&mut empty).iter_mut().for_each(String::clear);
        assert_eq!(value_label_atlas_input(&config, &empty), None);
    }
}

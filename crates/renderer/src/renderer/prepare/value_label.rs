//! Value label preparation: the label atlas of each category bar, box plot
//! and radar series that draws value labels, cached by what it is baked
//! from, and the per-frame label layer over it.

use super::*;

/// One series' value label atlas in the scratch cache
/// ([`ScratchEntry::ValueLabels`]), keyed as a contour's label atlas is: by
/// the [`data_render::ContourLabelAtlasInput`] it was baked from (the SSOT
/// strings, the face, the font size at the render scale, the colour, the
/// font policy and the absent background: everything the bake reads, so
/// the compare covers the bake by construction) and the font generation,
/// which invalidates an atlas when a host registers a font.
pub(in crate::renderer) struct ValueLabelScratch {
    pub(in crate::renderer) atlas_input: data_render::ContourLabelAtlasInput,
    pub(in crate::renderer) font_generation: u64,
    /// `None` when the strings measure to nothing.
    pub(in crate::renderer) atlas: Option<Arc<data_render::ValueLabelAtlas>>,
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

/// The colour a series' labels are baked in: the value axis' tick label
/// colour, or white over a dark bar its labels sit inside (`Inside`, or
/// `Auto` in a stack), as the former categorical chart wrote them.
fn value_label_color(
    config: &Config,
    render_type: &DataRenderType,
    axis: &crate::config::AxisOptions,
) -> Color {
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
    pub(in crate::renderer) fn ensure_value_label_scratch(
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
            && scratch.atlas_input == atlas_input
            && scratch.font_generation == font_generation
        {
            return Ok(scratch.atlas.clone());
        }
        let baked =
            axis_render::bake_contour_label_atlas(&atlas_input, self.caps.max_texture_dimension_2d)
                .map_err(|error| FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    kind: crate::RuleKind::ValueLabels {
                        detail: error.to_string(),
                    },
                })?;
        let atlas = match baked {
            Some(baked) => Some(Arc::new(data_render::ValueLabelAtlas::upload(
                self.device,
                self.queue,
                self.gpu_ledger,
                self.caps.max_texture_dimension_2d,
                baked,
            )?)),
            None => None,
        };
        // The contour scratch cache: one map, one eviction rule, one limit.
        if !self.in_frame {
            evict_if_full(
                &mut self.contour_cache,
                cfg.series_id.as_str(),
                CONTOUR_SCRATCH_CACHE_LIMIT,
            );
        }
        self.contour_cache
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "contour scratch cache",
                reason: error.to_string(),
            })?;
        self.contour_cache.insert(
            cfg.series_id.clone(),
            ScratchEntry::ValueLabels(ValueLabelScratch {
                atlas_input,
                font_generation,
                atlas: atlas.clone(),
            }),
        );
        Ok(atlas)
    }

    /// The atlas series `cfg`'s value labels draw from under `config` when
    /// the scratch cache holds it as baked for them now (what
    /// [`Self::ensure_value_label_scratch`] resolved), else `None`.
    pub(in crate::renderer) fn resident_value_label_atlas(
        &self,
        config: &Config,
        cfg: &SeriesConfig,
    ) -> Option<Arc<data_render::ValueLabelAtlas>> {
        let atlas_input = value_label_atlas_input(config, cfg)?;
        match self.contour_cache.get(cfg.series_id.as_str()) {
            Some(ScratchEntry::ValueLabels(scratch))
                if scratch.atlas_input == atlas_input
                    && scratch.font_generation == crate::text_render::font_generation() =>
            {
                scratch.atlas.clone()
            }
            _ => None,
        }
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
        let style_map =
            data_render::create_bar_style_map(self.device, self.per_point_style_map_bgl, &[])?;
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
    use crate::data_config::{CategoryBarLabels, CategoryBarSeriesLabels, CategoryBarStyle};

    fn chart() -> Config {
        let mut config = crate::default::default_config();
        config.bottom_x.categories = Some(vec![
            crate::config::Category::new("a", "A"),
            crate::config::Category::new("b", "B"),
        ]);
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
            render_type: DataRenderType::CategoryBars {
                color,
                style: CategoryBarStyle::default(),
                labels: CategoryBarSeriesLabels {
                    placement,
                    text: vec!["1.5".into(), "2".into()],
                    ..CategoryBarSeriesLabels::default()
                },
                label_format: Default::default(),
                overrides: None,
            },
        }
    }

    fn text_mut(series: &mut SeriesConfig) -> &mut Vec<String> {
        let DataRenderType::CategoryBars { labels, .. } = &mut series.render_type else {
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

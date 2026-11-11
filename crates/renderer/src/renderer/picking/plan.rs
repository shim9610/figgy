//! The pick plan of a chart: its layers, subplot routes and pick style rows.

use super::*;

pub(in crate::renderer) struct PickChartPlan {
    pub(in crate::renderer) descriptors: Vec<OwnedGpuPickSeriesDescriptor>,
}

/// The panel and series a pick addresses: the active layer's. Picking a
/// layered document reads the active layer's internal panel (its Y range and
/// the group data area) and only the series that layer scales; the base
/// layer answers while the annotation layer is active, which has no series.
/// A document without overlays is its own panel.
///
/// The line offset members the layer scales are drawn by panels of their
/// own (`Config::expand`). They follow the layer's other series, in stacking
/// order, and `member_panels` holds each one's panel, the expansion's own
/// panels kept in place, however many members there are (#107): the pick
/// tests each member where its panel draws it, from the top of the stack
/// first, and the layer's own panel last (`GpuPickEngine::pick_with_arcs`).
pub(in crate::renderer) struct LayerPickInputs {
    pub(in crate::renderer) panel: Config,
    pub(in crate::renderer) series: Vec<SeriesConfig>,
    pub(in crate::renderer) member_panels: Vec<Config>,
}

impl LayerPickInputs {
    /// The number of members at the end of `series`.
    pub(in crate::renderer) fn member_count(&self) -> usize {
        self.member_panels.len()
    }

    /// The transform of member `k`'s panel.
    pub(in crate::renderer) fn member_transform(&self, k: usize) -> data_render::ScatterTransform {
        data_render::scatter_transform_from_config(&self.member_panels[k])
    }
}

/// The config a base-layer pick at `cursor` (in `displayed`'s pixels) runs
/// on: `config` with the subplot drawn on top there as its active layer
/// ([`Config::subplot_at`], #205). `None` off every subplot, or on any
/// other active layer, where the host has chosen the layer itself.
pub(in crate::renderer) fn subplot_route(
    config: &Config,
    displayed: &Config,
    cursor: [f32; 2],
) -> Option<Config> {
    if config.active_layer() != crate::LayerRef::Base {
        return None;
    }
    let index = displayed.subplot_at(cursor[0], cursor[1])?;
    let mut routed = config.clone();
    routed.active_layer = crate::LayerRef::Subplot(index);
    Some(routed)
}

pub(in crate::renderer) fn active_layer_pick_inputs(
    config: &Config,
    series: &[SeriesConfig],
) -> std::result::Result<LayerPickInputs, crate::gpu_pick::GpuPickError> {
    if !is_layered(config) {
        return Ok(LayerPickInputs {
            panel: config.clone(),
            series: series.to_vec(),
            member_panels: Vec::new(),
        });
    }
    let layer = match config.active_layer() {
        crate::LayerRef::Annotations => crate::LayerRef::Base,
        layer => layer,
    };
    let mut panels =
        expand_for_render(config).map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
            resource: "renderer pick layer expansion",
        })?;
    let index = panel_index(config, panels.len(), layer);
    let layers = config.panel_layers();
    let layer = layers[index];
    let mut subset: Vec<SeriesConfig> = series
        .iter()
        .filter(|item| series_panel_index(config, panels.len(), &item.series_id) == index)
        .cloned()
        .collect();
    // The layer's own panel, then its members' panels in stacking order,
    // kept in the expansion's list (the layer's panel is not a member).
    let panel = panels.remove(index);
    let mut kept = 0usize;
    panels.retain(|_| {
        let panel_at = if kept < index { kept } else { kept + 1 };
        kept += 1;
        if layers.get(panel_at) != Some(&layer) {
            return false;
        }
        let Some(member) = config.panel_member(panel_at) else {
            return false;
        };
        let Some(item) = series.iter().find(|item| item.series_id == member) else {
            return false;
        };
        subset.push(item.clone());
        true
    });
    Ok(LayerPickInputs {
        panel,
        series: subset,
        member_panels: panels,
    })
}

/// The series a pick plans: every series but a monotonic series before its
/// group's first full call, which has no columns in `pool` and draws
/// nothing (#271). Borrowed unless there is one to leave out.
pub(in crate::renderer) fn pickable_series<'s>(
    pool: &ColumnPool,
    series: &'s [SeriesConfig],
) -> std::borrow::Cow<'s, [SeriesConfig]> {
    let drawn =
        |item: &SeriesConfig| item.monotonic().is_none() || pool.slot(&item.x_column).is_some();
    if series.iter().all(drawn) {
        std::borrow::Cow::Borrowed(series)
    } else {
        std::borrow::Cow::Owned(series.iter().filter(|item| drawn(item)).cloned().collect())
    }
}

impl PickChartPlan {
    /// [`Self::for_active_layer`] of a chart whose columns are in `pool`:
    /// a monotonic series before its group's first full call has none and
    /// is not planned (#271).
    pub(in crate::renderer) fn for_active_layer_in(
        pool: &ColumnPool,
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<Self, crate::gpu_pick::GpuPickError> {
        Self::for_active_layer(config, &pickable_series(pool, series))
    }

    /// The plan for the active layer of `config`; see
    /// [`active_layer_pick_inputs`].
    pub(in crate::renderer) fn for_active_layer(
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<Self, crate::gpu_pick::GpuPickError> {
        let inputs = active_layer_pick_inputs(config, series)?;
        Self::new(&inputs.panel, &inputs.series)
    }

    pub(in crate::renderer) fn new(
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<Self, crate::gpu_pick::GpuPickError> {
        let mut descriptors = Vec::new();
        descriptors.try_reserve_exact(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick descriptor plan",
            }
        })?;

        let all_series = series;
        for series in series {
            // A box plot's raw points are picked as markers placed in their
            // category slot; its boxes and violins are data picks.
            let primitives = effective_series_primitives(&config.draw_style, &series.render_type);
            if let Some(points) = series
                .render_type
                .box_plot()
                .and_then(|b| b.points)
                .filter(|_| primitives.box_points)
            {
                descriptors.push(box_points_pick_descriptor(
                    config, all_series, series, points,
                ));
                continue;
            }
            // A point vector is picked on the arrows it draws, through the
            // scatter primitive slot.
            let vector = extract_vector(&series.render_type)
                .filter(|_| primitives.vector)
                .and_then(|(components, style)| gpu_pick_vector(components, style));
            let vector_points = vector.is_some();
            // A ternary region fill is picked where it paints, with or
            // without markers and a line.
            let ternary_region =
                config.ternary().is_some() && series.render_type.ternary_region().is_some();
            if !primitives.line && !primitives.scatter && !vector_points && !ternary_region {
                continue;
            }

            let scatter = if let Some((_, style)) =
                extract_vector(&series.render_type).filter(|_| vector_points)
            {
                // The marker slot carries the anchor's highlight radius; the
                // pick itself walks the drawn arrow (`GpuPickVector`).
                let base_radius_px = crate::data_render::vector::vector_anchor_radius_px(style);
                let base_shape_id = data_render::shape_id(&ScatterShape::CircleFilled);
                Some(OwnedGpuPickScatter {
                    base_radius_px,
                    base_shape_id,
                    style_index_column: None,
                    style_slots: Vec::new(),
                    style_overrides: Vec::new(),
                    style_meta: None,
                    size_map: None,
                })
            } else if primitives.scatter {
                let scatter = extract_scatter(&series.render_type).ok_or(
                    crate::gpu_pick::GpuPickError::Internal(
                        "effective scatter primitive has scatter configuration",
                    ),
                )?;
                let scatter = scatter.as_ref();
                let base_radius_px = scatter.point_size;
                let base_shape_id = data_render::shape_id(&scatter.point_shape);
                if config.draw_style.is_precise() {
                    let style_table = scatter.point_style_table.as_deref().unwrap_or(&[]);
                    let overrides = scatter.point_style_overrides.as_deref().unwrap_or(&[]);
                    // The error bars' rows follow the markers' in the same
                    // vectors (`errorbar_style_rows` below).
                    let (bar_table, bar_overrides) = extract_errorbar_style(&series.render_type)
                        .map_or((&[][..], &[][..]), |style| {
                            (
                                style.error_bar_style_table.as_deref().unwrap_or(&[]),
                                style.error_bar_style_overrides.as_deref().unwrap_or(&[]),
                            )
                        });
                    let mut style_slots = Vec::new();
                    style_slots
                        .try_reserve_exact(style_table.len() + bar_table.len())
                        .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                            resource: "renderer pick style slots",
                        })?;
                    style_slots.extend(style_table.iter().map(pick_style_row));

                    let mut style_overrides = Vec::new();
                    style_overrides
                        .try_reserve_exact(overrides.len() + bar_overrides.len())
                        .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                            resource: "renderer pick style overrides",
                        })?;
                    for override_ in overrides {
                        let Ok(point_index) = u32::try_from(override_.index) else {
                            continue;
                        };
                        let slot = pick_style_row(&override_.style);
                        style_overrides.push(data_render::ScatterStyleOverrideGpu {
                            point_index,
                            _pad: [0; 3],
                            color_premul: slot.color_premul,
                            meta: slot.meta,
                        });
                    }

                    let style_index_column = scatter.point_style_index_column.clone();
                    let has_index = style_index_column.is_some();
                    let style_meta =
                        (has_index || !style_slots.is_empty() || !style_overrides.is_empty())
                            .then_some(data_render::ScatterStyleMapMeta {
                                style_count: style_slots.len().min(u32::MAX as usize) as u32,
                                override_count: style_overrides.len().min(u32::MAX as usize) as u32,
                                has_index: u32::from(has_index),
                                _pad: 0,
                            });
                    errorbar_style_rows(
                        &series.render_type,
                        &mut style_slots,
                        &mut style_overrides,
                    );
                    let size_map = scatter.point_size_map.as_ref().map(|map| {
                        crate::gpu_pick::GpuPickSizeMap {
                            column: map.column.clone(),
                            domain_min: map.domain_min,
                            domain_max: map.domain_max,
                            min_px: map.min_px,
                            max_px: map.max_px,
                            area: matches!(map.scale, SizeScale::Area),
                        }
                    });
                    Some(OwnedGpuPickScatter {
                        base_radius_px,
                        base_shape_id,
                        style_index_column,
                        style_slots,
                        style_overrides,
                        style_meta,
                        size_map,
                    })
                } else {
                    Some(OwnedGpuPickScatter {
                        base_radius_px,
                        base_shape_id,
                        style_index_column: None,
                        style_slots: Vec::new(),
                        style_overrides: Vec::new(),
                        style_meta: None,
                        size_map: None,
                    })
                }
            } else {
                None
            };

            let line = if primitives.line {
                Some(extract_line(&series.render_type).ok_or(
                    crate::gpu_pick::GpuPickError::Internal(
                        "effective line primitive has line configuration",
                    ),
                )?)
            } else {
                None
            };
            let line = line.as_deref();
            let line_width_px = line.map(|line| line.line_width);
            let line_interpolation =
                line.map_or(LineInterpolation::Linear, |line| line.interpolation);
            let line_style = line.map_or(LineStylePreset::Solid, |line| line.line_style);
            let errorbar = scatter
                .as_ref()
                .filter(|_| !vector_points)
                .and_then(|_| pick_errorbar(config, &series.render_type));
            let stem = scatter
                .as_ref()
                .and_then(|_| pick_stem(config, &series.render_type));
            // A radar outline is closed, and so is its fill on a full
            // circle, as `build_series_layers` draws them.
            let line_closed = line.is_some() && series.render_type.radar().is_some();
            let area = line.and_then(|line| {
                pick_area(
                    line,
                    series,
                    all_series,
                    line_closed && polar_fill_closes(config),
                )
            });
            descriptors.push(OwnedGpuPickSeriesDescriptor {
                series_id: series.series_id.clone(),
                source_id: series.source_id.clone(),
                resource: PickSeriesResource {
                    x_column: series.x_column.clone(),
                    y_column: series.y_column.clone(),
                    scatter,
                    line_width_px,
                    line_interpolation,
                    line_style,
                    vector,
                    errorbar,
                    area,
                    stem,
                    line_closed,
                    // A streamline's arrowheads pick with its line.
                    path_arrows: line.and_then(|_| {
                        series
                            .render_type
                            .streamlines()
                            .and_then(|streamlines| streamlines.arrows.copied())
                    }),
                    category_jitter: None,
                    // A ternary series' markers and line are placed by
                    // their third component too.
                    ternary_c: series.render_type.ternary_c_column().map(ColumnId::from),
                    ternary_region,
                },
            });
        }

        Ok(Self { descriptors })
    }

    pub(in crate::renderer) fn registry_slots<'a>(
        &'a self,
        current: Option<&[OwnedGpuPickSeriesDescriptor]>,
    ) -> std::result::Result<
        Vec<crate::gpu_pick::GpuPickRegistrySlot<'a>>,
        crate::gpu_pick::GpuPickError,
    > {
        self.registry_slots_rebuilding(current, &[])
    }

    pub(in crate::renderer) fn registry_slots_rebuilding<'a>(
        &'a self,
        current: Option<&[OwnedGpuPickSeriesDescriptor]>,
        rebuild_columns: &[&str],
    ) -> std::result::Result<
        Vec<crate::gpu_pick::GpuPickRegistrySlot<'a>>,
        crate::gpu_pick::GpuPickError,
    > {
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(self.descriptors.len())
            .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick registry transition",
            })?;
        for descriptor in &self.descriptors {
            let reusable = current.and_then(|current| {
                current.iter().position(|slot| {
                    !descriptor.resource.references_any_column(rebuild_columns)
                        && slot.series_id == descriptor.series_id
                        && slot.resource == descriptor.resource
                })
            });
            if let Some(current_index) = reusable {
                slots.push(crate::gpu_pick::GpuPickRegistrySlot::Reuse {
                    current_index,
                    source_id: descriptor.source_id.clone(),
                    series_id: descriptor.series_id.clone(),
                });
            } else {
                slots.push(crate::gpu_pick::GpuPickRegistrySlot::Build(
                    descriptor.descriptor(),
                ));
            }
        }
        Ok(slots)
    }
}

/// The error bars a series draws with its markers under `config`'s draw
/// style (`prepare/layers.rs`): every style that draws the markers but
/// Constellation, which drops error-bar series. The sizes are the logical
/// style's; whether the colour paints is the precise and sketch rule (alpha
/// above zero), while a Milkyway jet's white core paints at any alpha.
/// The stems a marker series draws (#206), as `vs_stem` draws them.
/// Constellation drops a stem series, so it picks none.
pub(super) fn pick_stem(
    config: &Config,
    render_type: &DataRenderType,
) -> Option<crate::gpu_pick::GpuPickStem> {
    if matches!(config.draw_style, DrawStyle::Constellation(_)) {
        return None;
    }
    let DataRenderType::Xy(crate::data_config::XySeries {
        stem: Some(stem), ..
    }) = render_type
    else {
        return None;
    };
    Some(crate::gpu_pick::GpuPickStem {
        baseline: stem.baseline,
        horizontal: stem.direction == crate::data_config::CategoryBarDirection::Horizontal,
        width_px: stem.width,
        dash: stem.line_style,
        paints: stem.color.a > 0.0,
    })
}

pub(super) fn pick_errorbar(
    config: &Config,
    render_type: &DataRenderType,
) -> Option<OwnedGpuPickErrorBar> {
    let style = extract_errorbar_style(render_type)?;
    if matches!(config.draw_style, DrawStyle::Constellation(_)) {
        return None;
    }
    let columns = |error: &ErrorRef| match error {
        ErrorRef::Symmetric { column } => [column.clone(), column.clone()],
        ErrorRef::Asymmetric { lower, upper } => [lower.clone(), upper.clone()],
    };
    let y = extract_err_y(render_type).map(columns);
    let x = extract_err_x(render_type).map(columns);
    if y.is_none() && x.is_none() {
        return None;
    }
    let DataErrorBarStyleConfig {
        error_bar_color,
        error_bar_width,
        error_bar_cap_size,
        cap_width,
        error_bar_style_table,
        error_bar_style_index_column,
        error_bar_style_overrides,
    } = style;
    // The precise draw maps per-point styles exactly when the renderer builds
    // an error-bar style map (`create_errorbar_style_map_for`).
    let mapped = config.draw_style.is_precise()
        && (error_bar_style_index_column.is_some()
            || error_bar_style_table
                .as_ref()
                .is_some_and(|table| !table.is_empty())
            || error_bar_style_overrides
                .as_ref()
                .is_some_and(|overrides| !overrides.is_empty()));
    Some(OwnedGpuPickErrorBar {
        y,
        x,
        stem_width_px: *error_bar_width,
        cap_half_px: *error_bar_cap_size,
        cap_width_px: *cap_width,
        paints: matches!(config.draw_style, DrawStyle::Milkyway(_)) || error_bar_color.a > 0.0,
        style_map: mapped.then(|| error_bar_style_index_column.clone()),
    })
}

/// Appends a series' error-bar style rows after the marker rows, in logical
/// pixels. The pick reads no colour, only whether a row's colour paints:
/// `color_premul.w` is 1 where the row sets a colour with alpha above zero.
pub(super) fn errorbar_style_rows(
    render_type: &DataRenderType,
    slots: &mut Vec<data_render::ScatterStyleSlotGpu>,
    overrides: &mut Vec<data_render::ScatterStyleOverrideGpu>,
) {
    let Some(style) = extract_errorbar_style(render_type) else {
        return;
    };
    let row = |slot: &DataErrorBarPointStyleConfig| {
        // The draw's mask bits (`create_errorbar_style_map_for`).
        const MASK_COLOR: u32 = 1;
        const MASK_STEM_WIDTH: u32 = 2;
        const MASK_CAP_HALF: u32 = 4;
        const MASK_CAP_WIDTH: u32 = 8;
        let row = errorbar_style_slot_gpu(
            slot,
            1.0,
            MASK_COLOR,
            MASK_STEM_WIDTH,
            MASK_CAP_HALF,
            MASK_CAP_WIDTH,
        );
        data_render::ScatterStyleSlotGpu {
            color_premul: [
                0.0,
                0.0,
                0.0,
                if row.color_premul[3] > 0.0 { 1.0 } else { 0.0 },
            ],
            meta: row.meta,
        }
    };
    slots.extend(
        style
            .error_bar_style_table
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(row),
    );
    for override_ in style.error_bar_style_overrides.as_deref().unwrap_or(&[]) {
        let Ok(point_index) = u32::try_from(override_.index) else {
            continue;
        };
        let slot = row(&override_.style);
        overrides.push(data_render::ScatterStyleOverrideGpu {
            point_index,
            _pad: [0; 3],
            color_premul: slot.color_premul,
            meta: slot.meta,
        });
    }
}

/// A line's fill as `prepare/layers.rs` draws it: to a baseline, or to the
/// y column of the series it names in the same series list with the same x
/// column (otherwise the draw rejects the chart and nothing is picked).
/// `paints` is whether the fill colour's alpha is above zero.
pub(super) fn pick_area(
    line: &DataLineStyleConfig,
    series: &SeriesConfig,
    all_series: &[SeriesConfig],
    closed: bool,
) -> Option<crate::gpu_pick::GpuPickArea> {
    let paints = line.effective_fill_color().a > 0.0;
    match line.fill.as_ref()? {
        LineFill::ToBaseline(baseline) => Some(crate::gpu_pick::GpuPickArea {
            to_series: None,
            baseline: *baseline,
            paints,
            closed,
        }),
        LineFill::ToSeries(target) => {
            let target = all_series
                .iter()
                .find(|other| other.series_id == *target)
                .filter(|other| other.x_column == series.x_column)?;
            Some(crate::gpu_pick::GpuPickArea {
                to_series: Some(target.y_column.clone()),
                baseline: 0.0,
                paints,
                closed,
            })
        }
    }
}

/// A style table row or override as the pick reads it: its radius and
/// shape at scale 1. The pick never reads colour, so the colour lanes and
/// the colour mask bit stay zero and a colour-only edit leaves the row's
/// bytes, and with them the slot's resource, equal.
pub(super) fn pick_style_row(
    style: &DataScatterPointStyleConfig,
) -> data_render::ScatterStyleSlotGpu {
    let mut row = scatter_style_slot_gpu(
        style,
        1.0,
        0,
        data_render::STYLE_MASK_RADIUS,
        data_render::STYLE_MASK_SHAPE,
    );
    row.color_premul = [0.0; 4];
    row
}

//! Selection preparation shared by resident columns and bounded selected rows.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SelectionRefFilter {
    All,
    Legacy(usize),
    Typed(usize),
}

impl SelectionRefFilter {
    fn accepts_legacy(self, ordinal: usize) -> bool {
        matches!(self, Self::All) || self == Self::Legacy(ordinal)
    }

    fn accepts_typed(self, ordinal: usize) -> bool {
        matches!(self, Self::All) || self == Self::Typed(ordinal)
    }
}

/// Ref identity stays global; only GPU instance/column addressing is rebased.
#[derive(Clone, Copy, Debug)]
pub(super) struct SelectionRows {
    pub global_start: usize,
    pub refs: SelectionRefFilter,
    /// A packed view page is sparse in source-index space. The selected
    /// source row is already resolved to this page's GPU-local instance.
    pub packed_instance: Option<u32>,
}

impl SelectionRows {
    pub const RESIDENT: Self = Self {
        global_start: 0,
        refs: SelectionRefFilter::All,
        packed_instance: None,
    };

    fn instance(self, global: usize, local_count: usize) -> Option<u32> {
        if let Some(local) = self.packed_instance {
            return (global == self.global_start && (local as usize) < local_count).then_some(local);
        }
        let local = global.checked_sub(self.global_start)?;
        (local < local_count)
            .then(|| u32::try_from(local).ok())
            .flatten()
    }
}

pub(super) struct SelectionColumns<'a> {
    pub buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    pub bar: Option<(ColumnHandle, ColumnHandle)>,
    pub rows: SelectionRows,
    /// A precise streamed ring uses ScatterStyleMap::stream_bind_group with
    /// rows.global_start. Its style-index vertex column still uses local rows.
    pub mapped_point_bg: Option<&'a wgpu::BindGroup>,
}

pub(super) struct SelectionLayers<'a> {
    pub picked: Vec<ColumnPickRingLayer<'a>>,
    pub selected_bars: Vec<ColumnBarSelectionLayer<'a>>,
    pub selected_fields: Vec<ColumnFieldSelectionLayer<'a>>,
}

impl<'a> SelectionLayers<'a> {
    pub fn into_series_layers(self) -> data_render::SeriesLayers<'a> {
        data_render::SeriesLayers {
            field: None,
            bar: None,
            contour: None,
            errorbar: None,
            line: None,
            line_extra: None,
            scatter: None,
            selected_bars: self.selected_bars,
            selected_fields: self.selected_fields,
            picked: self.picked,
        }
    }
}

#[derive(Debug)]
struct PointSelectionVisual {
    point_index: usize,
    color: Color,
    width_px: f32,
    radius_extra_px: f32,
}

fn point_selection_visuals(
    chart_config: &Config,
    cfg: &SeriesConfig,
    refs: SelectionRefFilter,
) -> Vec<PointSelectionVisual> {
    let mut point_visuals = Vec::new();
    if let Some(picked_cfg) = chart_config
        .picked_points
        .as_ref()
        .filter(|config| config.visible && !config.refs.is_empty())
    {
        point_visuals.extend(
            picked_cfg
                .refs
                .iter()
                .enumerate()
                .filter(|(ordinal, picked_ref)| {
                    refs.accepts_legacy(*ordinal) && picked_ref_matches_series(cfg, picked_ref)
                })
                .map(|(_, picked_ref)| PointSelectionVisual {
                    point_index: picked_ref.point_index,
                    color: picked_cfg.ring_color,
                    width_px: picked_cfg.ring_width_px,
                    radius_extra_px: picked_cfg.radius_extra_px,
                }),
        );
    }
    let typed_selections = chart_config
        .picked_data
        .as_ref()
        .filter(|config| config.visible && !config.refs.is_empty());
    if let Some(selection_cfg) = typed_selections {
        point_visuals.extend(selection_cfg.refs.iter().enumerate().filter_map(
            |(ordinal, picked_ref)| {
                if !refs.accepts_typed(ordinal) || !picked_data_ref_matches_series(cfg, picked_ref)
                {
                    return None;
                }
                let PickedDataRef::Point { point_index, .. } = picked_ref else {
                    return None;
                };
                Some(PointSelectionVisual {
                    point_index: *point_index,
                    color: selection_cfg.highlight_color,
                    width_px: selection_cfg.outline_width_px,
                    radius_extra_px: selection_cfg.point_radius_extra_px,
                })
            },
        ));
    }

    point_visuals
}

impl PrepareContext<'_> {
    /// Reads selection policy from Config; callers retain all supplied GPU
    /// handles and charges until the resulting prepared packet is dropped.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_selection_layers<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &Config,
        series: &Series<'a>,
        pipelines: &'a TargetPipelines,
        columns: SelectionColumns<'a>,
        contour_scratch: Option<&PreparedContourScratch>,
        use_scatter_style_mapping: bool,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<SelectionLayers<'a>> {
        let cfg = series.config;
        let rt = &cfg.render_type;
        let primitives = effective_series_primitives(&chart_config.draw_style, rt);
        let (x_h, y_h, column_buffer) = (columns.x, columns.y, columns.buffer);
        let point_visuals = point_selection_visuals(chart_config, cfg, columns.rows.refs);
        let typed_selections = chart_config
            .picked_data
            .as_ref()
            .filter(|config| config.visible && !config.refs.is_empty());
        let mut picked = Vec::new();
        if primitives.line || primitives.scatter {
            for visual in point_visuals {
                let Some(instance) = columns
                    .rows
                    .instance(visual.point_index, x_h.len_values.min(y_h.len_values))
                else {
                    continue;
                };

                let scatter_cfg = extract_scatter(rt);
                let has_line_anchor = extract_line(rt).is_some();
                let has_scatter_anchor = scatter_cfg.is_some_and(|scatter| {
                    scatter_pick_anchor_may_be_visible(
                        scatter,
                        visual.point_index,
                        use_scatter_style_mapping,
                    )
                });
                if !has_line_anchor && !has_scatter_anchor {
                    continue;
                }

                let precise_pick_style_map = if use_scatter_style_mapping {
                    series.style.scatter_map.as_ref()
                } else {
                    None
                };
                if precise_pick_style_map.is_some()
                    && columns.rows.global_start != 0
                    && columns.mapped_point_bg.is_none()
                {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: cfg.series_id.clone(),
                        reason: "rebased selection needs a global-index style map binding".into(),
                    });
                }
                let style_index = match (precise_pick_style_map, scatter_cfg) {
                    (Some(map), Some(scatter)) if map.has_index => {
                        let Some(column) = scatter.point_style_index_column.as_ref() else {
                            return Err(FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: "scatter style map expects an index column".into(),
                            });
                        };
                        let h = lookup(column)?;
                        let count = x_h.len_values.min(y_h.len_values);
                        if h.len_values < count {
                            return Err(FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: format!(
                                    "style index column {column:?} has {} values, but scatter uses {count}",
                                    h.len_values
                                ),
                            });
                        }
                        Some(h)
                    }
                    _ => None,
                };

                let mut ring_style =
                    PrimitiveStyle::from_color_with_width(visual.color, visual.width_px);
                let uses_mapped_pick = precise_pick_style_map.is_some();
                if uses_mapped_pick {
                    ring_style.point_radius_px = series.style.scatter_radius_px;
                    ring_style.cap_half_px = visual.radius_extra_px;
                } else {
                    let scatter_radius = scatter_cfg
                        .map(|scatter| {
                            scatter_config_radius_px(
                                scatter,
                                visual.point_index,
                                use_scatter_style_mapping,
                            )
                        })
                        .unwrap_or(0.0);
                    ring_style.point_radius_px = (scatter_radius + visual.radius_extra_px).max(0.0);
                }
                ring_style.shape_id = data_render::shape_id(&ScatterShape::Circle);
                let ring_buf = data_render::create_style_uniform_buffer(&self.device, &ring_style);
                let style_bg =
                    data_render::create_style_bind_group(&self.device, &self.style_bgl, &ring_buf);
                picked.push(ColumnPickRingLayer {
                    pipeline: if uses_mapped_pick {
                        pipelines
                            .pick_ring_mapped
                            .as_ref()
                            .expect("prepare ensured mapped pick ring pipeline")
                    } else {
                        pipelines
                            .pick_ring
                            .as_ref()
                            .expect("prepare ensured pick ring pipeline")
                    },
                    transform_bg: &view.transform_bg,
                    style_bg,
                    style_map_bg: precise_pick_style_map
                        .map(|map| columns.mapped_point_bg.unwrap_or(&map.bind_group)),
                    quad_vb: &self.quad_vb,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    style_index,
                    instance,
                });
            }
        }

        let mut selected_bars = Vec::new();
        let mut selected_fields = Vec::new();
        if let Some(selection_cfg) = typed_selections {
            for (ordinal, picked_ref) in selection_cfg.refs.iter().enumerate() {
                if !columns.rows.refs.accepts_typed(ordinal)
                    || !picked_data_ref_matches_series(cfg, picked_ref)
                {
                    continue;
                }
                match picked_ref {
                    PickedDataRef::HistogramBin { bin_index, .. } if primitives.bar => {
                        let (edges, values) =
                            columns.bar.expect("bar primitive has selection columns");
                        let count =
                            data_render::bar_instance_count(edges.len_values, values.len_values);
                        let Some(instance) = columns.rows.instance(*bin_index, count as usize)
                        else {
                            continue;
                        };
                        let mut selection = data_render::DataSelectionGpu::from_color(
                            selection_cfg.highlight_color,
                        );
                        selection.metrics[0] = selection_cfg.outline_width_px;
                        selection.indices = [
                            data_render::DATA_SELECTION_KIND_HISTOGRAM_BIN,
                            instance,
                            0,
                            0,
                        ];
                        let (selection_bg, selection_charge) =
                            data_render::create_data_selection_bind_group(
                                &self.gpu_ledger,
                                &self.device,
                                &self.data_selection_bgl,
                                &selection,
                            );
                        // The selection outline must follow the selected
                        // bin's overridden gap/width exactly. Resolve only
                        // style metadata on the CPU; edge/value geometry
                        // remains in the shared GPU columns.
                        let bar_cfg =
                            extract_bar(rt).expect("histogram selection has a bar config");
                        let resolved_style = resolved_bar_primitive_style(
                            bar_cfg,
                            *bin_index,
                            series.style.display_scale,
                        );
                        let resolved_style_buffer =
                            data_render::create_style_uniform_buffer(&self.device, &resolved_style);
                        let resolved_style_bg = data_render::create_style_bind_group(
                            &self.device,
                            &self.style_bgl,
                            &resolved_style_buffer,
                        );
                        selected_bars.push(ColumnBarSelectionLayer {
                            pipeline: pipelines
                                .bar_selection
                                .as_ref()
                                .expect("prepare ensured histogram selection pipeline"),
                            transform_bg: &view.transform_bg,
                            style_bg: resolved_style_bg,
                            selection_bg,
                            selection_charge,
                            pool_buffer: column_buffer,
                            edges,
                            values,
                            instance,
                        });
                    }
                    PickedDataRef::MatrixCell {
                        x_index, y_index, ..
                    } if primitives.field => {
                        let (Some(x_index), Some(y_index)) =
                            (u32::try_from(*x_index).ok(), u32::try_from(*y_index).ok())
                        else {
                            continue;
                        };
                        let scratch = self
                            .field_cache
                            .get(cfg.series_id.as_str())
                            .expect("prepare ensured selected field scratch");
                        let mut selection = data_render::DataSelectionGpu::from_color(
                            selection_cfg.highlight_color,
                        );
                        selection.metrics[0] = selection_cfg.outline_width_px;
                        selection.indices = [
                            data_render::DATA_SELECTION_KIND_MATRIX_CELL,
                            0,
                            x_index,
                            y_index,
                        ];
                        let (selection_bg, selection_charge) =
                            data_render::create_data_selection_bind_group(
                                &self.gpu_ledger,
                                &self.device,
                                &self.data_selection_bgl,
                                &selection,
                            );
                        selected_fields.push(ColumnFieldSelectionLayer {
                            pipeline: pipelines
                                .field_selection
                                .as_ref()
                                .expect("prepare ensured field selection pipeline"),
                            transform_bg: &view.transform_bg,
                            selection_bg,
                            selection_charge,
                            field_bg: scratch.field_bg.clone(),
                            charge: Arc::clone(&scratch.charge),
                            drawable: scratch.drawable,
                        });
                    }
                    PickedDataRef::ContourLevel {
                        level_index,
                        x_index,
                        y_index,
                        ..
                    } if primitives.contour => {
                        let (Some(level_index), Some(x_index), Some(y_index)) = (
                            u32::try_from(*level_index).ok(),
                            u32::try_from(*x_index).ok(),
                            u32::try_from(*y_index).ok(),
                        ) else {
                            continue;
                        };
                        if usize::try_from(level_index).ok().is_none_or(|index| {
                            extract_contour(rt).is_none_or(|contour| index >= contour.levels.len())
                        }) {
                            continue;
                        }
                        let scratch =
                            contour_scratch.expect("prepare captured selected contour scratch");
                        let mut selection = data_render::DataSelectionGpu::from_color(
                            selection_cfg.highlight_color,
                        );
                        selection.metrics[1] = selection_cfg.contour_width_extra_px;
                        selection.indices = [
                            data_render::DATA_SELECTION_KIND_CONTOUR_LEVEL,
                            level_index,
                            x_index,
                            y_index,
                        ];
                        let (selection_bg, selection_charge) =
                            data_render::create_data_selection_bind_group(
                                &self.gpu_ledger,
                                &self.device,
                                &self.data_selection_bgl,
                                &selection,
                            );
                        selected_fields.push(ColumnFieldSelectionLayer {
                            pipeline: pipelines
                                .field_selection
                                .as_ref()
                                .expect("prepare ensured contour selection pipeline"),
                            transform_bg: &view.transform_bg,
                            selection_bg,
                            selection_charge,
                            field_bg: scratch.field_bg.clone(),
                            charge: Arc::clone(&scratch.charge),
                            drawable: scratch.drawable,
                        });
                    }
                    _ => {}
                }
            }
        }

        Ok(SelectionLayers {
            picked,
            selected_bars,
            selected_fields,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DataSelectionsConfig, PickedPointsConfig};
    use crate::data_config::{BarOrientation, DataBarStyleOverride, DataScatterPointStyleOverride};

    fn declaration() -> SeriesConfig {
        SeriesConfig {
            series_id: "selected".into(),
            source_id: Some("source".into()),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Scatter {
                scatter: DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: ScatterShape::CircleFilled,
                    point_size: 0.0,
                    point_style_index_column: Some("index".into()),
                    point_style_table: Some(vec![DataScatterPointStyleConfig {
                        point_size: Some(9.0),
                        ..Default::default()
                    }]),
                    point_style_overrides: Some(vec![DataScatterPointStyleOverride {
                        index: 2,
                        style: DataScatterPointStyleConfig {
                            point_size: Some(17.0),
                            ..Default::default()
                        },
                    }]),
                },
            },
        }
    }

    fn legacy(index: usize) -> PickedPointRef {
        PickedPointRef {
            source_id: Some("source".into()),
            series_id: "selected".into(),
            point_index: index,
        }
    }

    fn point(index: usize) -> PickedDataRef {
        PickedDataRef::Point {
            source_id: Some("source".into()),
            series_id: "selected".into(),
            point_index: index,
        }
    }

    #[test]
    fn selection_ordinals_preserve_duplicates_order_and_provenance() {
        let series = declaration();
        let mut config = crate::default::default_config();
        config.picked_points = Some(PickedPointsConfig {
            refs: vec![legacy(10), legacy(2), legacy(10)],
            ring_color: Color::new(1.0, 0.0, 0.0, 0.5),
            ring_width_px: 3.0,
            radius_extra_px: 7.0,
            ..Default::default()
        });
        config.picked_data = Some(DataSelectionsConfig {
            refs: vec![
                point(10),
                PickedDataRef::HistogramBin {
                    source_id: None,
                    series_id: "selected".into(),
                    bin_index: 2,
                },
                point(2),
                point(10),
            ],
            highlight_color: Color::new(0.0, 1.0, 0.0, 0.5),
            outline_width_px: 5.0,
            point_radius_extra_px: 11.0,
            ..Default::default()
        });
        let visuals = point_selection_visuals(&config, &series, SelectionRefFilter::All);
        assert_eq!(
            visuals.iter().map(|v| v.point_index).collect::<Vec<_>>(),
            [10, 2, 10, 10, 2, 10]
        );
        assert_eq!(
            (
                visuals[0].color.r,
                visuals[0].width_px,
                visuals[0].radius_extra_px
            ),
            (1.0, 3.0, 7.0)
        );
        assert_eq!(
            (
                visuals[3].color.g,
                visuals[3].width_px,
                visuals[3].radius_extra_px
            ),
            (1.0, 5.0, 11.0)
        );
        for (filter, expected) in [
            (SelectionRefFilter::Legacy(0), vec![10]),
            (SelectionRefFilter::Legacy(1), vec![2]),
            (SelectionRefFilter::Legacy(2), vec![10]),
            (SelectionRefFilter::Typed(0), vec![10]),
            (SelectionRefFilter::Typed(1), vec![]),
            (SelectionRefFilter::Typed(2), vec![2]),
            (SelectionRefFilter::Typed(3), vec![10]),
            (SelectionRefFilter::Typed(4), vec![]),
        ] {
            assert_eq!(
                point_selection_visuals(&config, &series, filter)
                    .iter()
                    .map(|v| v.point_index)
                    .collect::<Vec<_>>(),
                expected
            );
        }
        config.picked_points.as_mut().unwrap().refs[0].source_id = Some("other".into());
        assert!(
            point_selection_visuals(&config, &series, SelectionRefFilter::Legacy(0)).is_empty()
        );
        config.picked_points.as_mut().unwrap().refs[0].source_id = None;
        assert_eq!(
            point_selection_visuals(&config, &series, SelectionRefFilter::Legacy(0)).len(),
            1
        );
        config.picked_points.as_mut().unwrap().visible = false;
        config.picked_data.as_mut().unwrap().visible = false;
        assert!(point_selection_visuals(&config, &series, SelectionRefFilter::All).is_empty());
    }

    #[test]
    fn selection_rows_rebase_only_gpu_addressing_and_reject_outside_rows() {
        let rows = SelectionRows {
            global_start: 10,
            refs: SelectionRefFilter::Typed(2),
            packed_instance: None,
        };
        assert_eq!(rows.instance(10, 1), Some(0));
        assert_eq!(rows.instance(9, 1), None);
        assert_eq!(rows.instance(11, 1), None);
        assert_eq!(rows.instance(11, 2), Some(1));
        assert_eq!(rows.instance(usize::MAX, 1), None);
        assert_eq!(
            SelectionRows {
                global_start: usize::MAX,
                ..rows
            }
            .instance(usize::MAX, 1),
            Some(0)
        );
        assert_eq!(SelectionRows::RESIDENT.instance(10, 11), Some(10));
        assert_eq!(SelectionRows::RESIDENT.instance(10, 10), None);
        let packed = SelectionRows { global_start: 20, packed_instance: Some(7), ..rows };
        assert_eq!(packed.instance(20, 8), Some(7));
        assert_eq!(packed.instance(20, 7), None);
        assert_eq!(packed.instance(21, 8), None);
    }

    fn narrow(mut handle: ColumnHandle, start: usize, count: usize) -> ColumnHandle {
        assert!(start + count <= handle.len_values);
        handle.offset += (start * crate::data::COLUMN_VALUE_BYTES) as u64;
        handle.byte_size = (count * crate::data::COLUMN_VALUE_BYTES) as u64;
        handle.len_values = count;
        handle
    }

    fn pixels(renderer: &Renderer, frame: &PreparedFrame) -> Vec<u8> {
        let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("selection helper parity"),
            size: wgpu::Extent3d {
                width: 320,
                height: 240,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("selection helper parity readback"),
            size: 1280 * 240,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let view = target.create_view(&Default::default());
        let mut encoder = renderer.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            renderer
                .paint_prepared(&mut pass, (320, 240), frame)
                .unwrap();
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(1280),
                    rows_per_image: Some(240),
                },
            },
            target.size(),
        );
        renderer.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        slice.get_mapped_range().unwrap().to_vec()
    }

    #[test]
    fn selected_row_geometry_matches_resident_global_point_and_bin_overrides() {
        let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
            .lock()
            .unwrap();
        let (device, queue) = data_render::shared_device().expect("selection parity GPU required");
        let mut renderer = Renderer::try_new_with_sample_count(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            8192,
            1,
        )
        .unwrap();
        for (id, values) in [
            ("x", vec![0.1, 0.3, 0.55, 0.88]),
            ("y", vec![0.22, 0.5, 0.72]),
            ("index", vec![0.0; 3]),
        ] {
            renderer
                .add_column(
                    id,
                    &crate::Column {
                        data: values,
                        min: 0.0,
                        max: 1.0,
                    },
                )
                .unwrap();
        }
        for kind in 0..3 {
            let mut declaration = declaration();
            if kind != 0 {
                declaration.render_type = DataRenderType::Histogram {
                    bar: DataBarStyleConfig {
                        fill_color: Color::BLACK,
                        border_color: Color::BLACK,
                        border_width: 1.0,
                        baseline: 0.0,
                        gap_px: 0.0,
                        width_ratio: 1.0,
                        orientation: if kind == 1 {
                            BarOrientation::Vertical
                        } else {
                            BarOrientation::Horizontal
                        },
                        bar_style_overrides: Some(vec![DataBarStyleOverride {
                            index: 2,
                            style: DataBarBinStyleConfig {
                                gap_px: Some(2.0),
                                width_ratio: Some(0.31),
                                ..Default::default()
                            },
                        }]),
                    },
                };
                if kind == 2 {
                    std::mem::swap(&mut declaration.x_column, &mut declaration.y_column);
                }
            }
            let mut config = crate::default::default_config();
            config.chart_area = crate::layout::ChartArea(Rect {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            });
            config.picked_data = Some(DataSelectionsConfig {
                refs: vec![if kind == 0 {
                    point(2)
                } else {
                    PickedDataRef::HistogramBin {
                        source_id: Some("source".into()),
                        series_id: "selected".into(),
                        bin_index: 2,
                    }
                }],
                highlight_color: Color::new(0.0, 1.0, 0.0, 1.0),
                outline_width_px: 3.0,
                point_radius_extra_px: 4.0,
                ..Default::default()
            });
            let mut chart = Chart::new(config);
            chart.set_x_range(0.0, 1.0);
            chart.set_y_range(0.0, 1.0);
            let view = renderer
                .create_chart_view(&chart, chart.config().chart_area.0)
                .unwrap();
            let style = renderer.create_style_for_series(&declaration).unwrap();
            let series = [Series {
                config: &declaration,
                style: &style,
            }];
            let mut frame = renderer
                .prepare(&[ChartDrawItem {
                    view: &view,
                    chart_config: chart.config(),
                    series: &series,
                }])
                .unwrap();
            let mut images = Vec::new();
            for local in [false, true] {
                let packet = {
                    let (_, preparation) = renderer.preparation_parts();
                    let x = preparation.pool.handle_for(&declaration.x_column).unwrap();
                    let y = preparation.pool.handle_for(&declaration.y_column).unwrap();
                    let edges = preparation.pool.handle_for("x").unwrap();
                    let values = preparation.pool.handle_for("y").unwrap();
                    let narrow_point = |h| if local { narrow(h, 2, 1) } else { h };
                    let tally = crate::gpu_memory::ChargeTally::new();
                    let map = local
                        .then(|| {
                            style.scatter_map.as_ref().map(|map| {
                                map.stream_bind_group(
                                    preparation.device,
                                    preparation.per_point_style_map_bgl,
                                    2,
                                    &tally,
                                )
                            })
                        })
                        .flatten();
                    let layers = preparation
                        .build_selection_layers(
                            &view,
                            chart.config(),
                            &series[0],
                            preparation.pipelines,
                            SelectionColumns {
                                buffer: preparation.pool.buffer(),
                                x: narrow_point(x),
                                y: narrow_point(y),
                                bar: (kind != 0).then(|| {
                                    if local {
                                        (narrow(edges, 2, 2), narrow(values, 2, 1))
                                    } else {
                                        (edges, values)
                                    }
                                }),
                                rows: SelectionRows {
                                    global_start: if local { 2 } else { 0 },
                                    refs: SelectionRefFilter::Typed(0),
                                    packed_instance: None,
                                },
                                mapped_point_bg: map.as_ref(),
                            },
                            None,
                            true,
                            |id| {
                                preparation
                                    .pool
                                    .handle_for(id)
                                    .map(narrow_point)
                                    .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
                            },
                        )
                        .unwrap();
                    if kind == 0 {
                        assert_eq!(layers.picked.len(), 1);
                        assert_eq!(layers.picked[0].instance, if local { 0 } else { 2 });
                    } else {
                        assert_eq!(layers.selected_bars.len(), 1);
                        assert_eq!(layers.selected_bars[0].instance, if local { 0 } else { 2 });
                    }
                    PreparedSeries::from_layers(
                        layers.into_series_layers(), None, Some(Arc::clone(&style._charge)),
                    )
                };
                frame.items[0].series = vec![packet];
                images.push(pixels(&renderer, &frame));
            }
            assert!(
                images[0]
                    .chunks_exact(4)
                    .filter(|p| p[1] > 180 && p[0] < 80 && p[2] < 80)
                    .count()
                    > 20,
                "selection fixture must draw visible green geometry, kind {kind}"
            );
            assert_eq!(
                images[0], images[1],
                "global/local selected geometry differs, kind {kind}"
            );
            drop(frame);
            renderer.end_gpu_frame();
        }
    }
}

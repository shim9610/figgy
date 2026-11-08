//! Selection preparation shared by resident columns and bounded selected rows.

mod bars;
mod fields;
mod rings;

use super::*;
use crate::data_config::ViolinSide;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SelectionRefFilter {
    All,
    /// Only the ref at this index of `picked_data.refs`.
    Typed(usize),
}

impl SelectionRefFilter {
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
            return (global == self.global_start && (local as usize) < local_count)
                .then_some(local);
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
    /// A category bar, box plot or range bar series' data and style map, as
    /// its draw binds them; its picked bar's or box's highlight reads the
    /// same.
    pub category: Option<data_render::CategoryBarData>,
    /// A box plot series' violin data and segment count, and its raw
    /// points' data, as their draws bind them.
    pub violin: Option<(data_render::CategoryBarData, u32)>,
    pub box_points: Option<data_render::CategoryBarData>,
    pub rows: SelectionRows,
    /// A precise streamed ring uses ScatterStyleMap::stream_bind_group with
    /// rows.global_start. Its style-index vertex column still uses local rows.
    pub mapped_point_bg: Option<&'a wgpu::BindGroup>,
    /// A 3D series' height column and camera group (#132): its ring draws
    /// through the 3D ring entries in the live slot.
    pub view3d: Option<(ColumnHandle, data_render::surface_mesh::View3dBinding)>,
}

pub(super) struct SelectionLayers<'a> {
    pub picked: Vec<ColumnPickRingLayer<'a>>,
    pub selected_bars: Vec<ColumnBarSelectionLayer<'a>>,
    pub selected_fields: Vec<ColumnFieldSelectionLayer<'a>>,
}

impl<'a> SelectionLayers<'a> {
    pub fn into_series_layers(self) -> data_render::SeriesLayers<'a> {
        data_render::SeriesLayers {
            area: None,
            field: None,
            mesh: None,
            mesh_contour: None,
            bar: None,
            violin: None,
            category_bars: None,
            box_points: None,
            contour: None,
            vector: None,
            errorbar: None,
            stem: None,
            line: None,
            parallel: None,
            line_extra: None,
            scatter: None,
            line_3d: None,
            scatter_3d: None,
            vector_3d: None,
            selected_bars: self.selected_bars,
            selected_fields: self.selected_fields,
            picked: self.picked,
            value_labels: None,
            energy_levels: None,
            surface: None,
            selected_surface: None,
            floor: None,
            region: None,
        }
    }
}

#[derive(Debug)]
pub(super) struct PointSelectionVisual {
    pub(super) point_index: usize,
    pub(super) color: Color,
    pub(super) width_px: f32,
    pub(super) radius_extra_px: f32,
}

/// The point selections of `cfg` with the colour and width each is drawn
/// in.
pub(super) fn point_selection_visuals(
    chart_config: &Config,
    cfg: &SeriesConfig,
    refs: SelectionRefFilter,
) -> Vec<PointSelectionVisual> {
    let mut point_visuals = Vec::new();
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

/// One series' inputs to the selection builders: the panel's view, config
/// and pipelines, the series and the columns its draws bind.
#[derive(Clone, Copy)]
struct SelectionInputs<'a, 'c> {
    view: &'a ChartView,
    chart_config: &'c Config,
    series: &'c Series<'a>,
    pipelines: &'a TargetPipelines,
    columns: &'c SelectionColumns<'a>,
    contour_scratch: Option<&'c PreparedContourScratch>,
    use_scatter_style_mapping: bool,
    cfg: &'a SeriesConfig,
    rt: &'a DataRenderType,
    x_h: ColumnHandle,
    y_h: ColumnHandle,
    column_buffer: &'a wgpu::Buffer,
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
        // A point vector's anchors take the ring like markers; a grid vector's
        // cells take the field highlight below.
        let vector_points = extract_vector(rt)
            .filter(|_| primitives.vector)
            .and_then(|(components, style)| components.grid().is_none().then_some(style));
        let mut picked = Vec::new();
        let s = SelectionInputs {
            view,
            chart_config,
            series,
            pipelines,
            columns: &columns,
            contour_scratch,
            use_scatter_style_mapping,
            cfg,
            rt,
            x_h,
            y_h,
            column_buffer,
        };
        if primitives.line || primitives.scatter || vector_points.is_some() {
            for visual in point_visuals {
                self.point_ring_layer(&s, visual, vector_points, &mut picked, &mut lookup)?;
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
                        self.histogram_bin_selection(
                            &s,
                            selection_cfg,
                            bin_index,
                            &mut selected_bars,
                        )?;
                    }
                    // A box is a bin whose index is its row: the highlight
                    // builds the same body from the same group 3 and style
                    // map as the draw.
                    PickedDataRef::HistogramBin {
                        bin_index, half, ..
                    } if primitives.box_plot => {
                        self.box_selection(&s, selection_cfg, bin_index, half, &mut selected_bars)?;
                    }
                    // A range bar is a bin whose index is its row: the
                    // highlight builds the same body from the same group 3
                    // and style map as the draw.
                    // A box plot's raw point: its ring, built from the same
                    // group 3 and slot as its draw places it with.
                    PickedDataRef::Point { point_index, .. } if primitives.box_points => {
                        self.box_point_selection(
                            &s,
                            selection_cfg,
                            point_index,
                            &mut selected_bars,
                        )?;
                    }
                    PickedDataRef::HistogramBin { bin_index, .. } if primitives.range_bars => {
                        self.range_bar_selection(&s, selection_cfg, bin_index, &mut selected_bars)?;
                    }
                    // A category bar is a bin whose index is its row: the
                    // highlight builds the same bar from the same group 3
                    // and override table as the draw.
                    PickedDataRef::HistogramBin { bin_index, .. } if primitives.category_bars => {
                        self.category_bar_selection(
                            &s,
                            selection_cfg,
                            bin_index,
                            &mut selected_bars,
                        )?;
                    }
                    // A 3D chart's picked cell is its surface patch
                    // (`surface_layers`), not the flat cell outline.
                    PickedDataRef::MatrixCell {
                        x_index, y_index, ..
                    } if primitives.field && chart_config.view_3d().is_none() => {
                        self.field_cell_selection(
                            &s,
                            selection_cfg,
                            x_index,
                            y_index,
                            &mut selected_fields,
                        )?;
                    }
                    // A grid vector's selected cell: the `u` grid is bound as a
                    // field, so the field selection pipeline outlines the cell.
                    PickedDataRef::MatrixCell {
                        x_index, y_index, ..
                    } if primitives.vector => {
                        self.vector_cell_selection(
                            &s,
                            selection_cfg,
                            x_index,
                            y_index,
                            &mut selected_fields,
                        )?;
                    }
                    PickedDataRef::ContourLevel {
                        level_index,
                        x_index,
                        y_index,
                        ..
                    } if primitives.contour => {
                        self.contour_level_selection(
                            &s,
                            selection_cfg,
                            level_index,
                            x_index,
                            y_index,
                            &mut selected_fields,
                        )?;
                    }
                    // A triangle mesh's picked triangle (#210): that one
                    // instance redrawn through the mesh's selection entry,
                    // outlined inside its edges.
                    // A 3D chart's picked triangle is its surface patch
                    // (`mesh_surface_layers`), not the flat outline.
                    PickedDataRef::MeshTriangle { triangle_index, .. }
                        if primitives.mesh && chart_config.view_3d().is_none() =>
                    {
                        self.mesh_triangle_selection(
                            &s,
                            selection_cfg,
                            triangle_index,
                            &mut selected_fields,
                        )?;
                    }
                    // A triangle mesh's picked level: every triangle redrawn,
                    // painting that level's stroke widened. On a 3D chart
                    // the contours are on the floor, which draws no
                    // highlight, as a heatmap's floor does not.
                    PickedDataRef::ContourLevel { level_index, .. }
                        if primitives.mesh_contour && chart_config.view_3d().is_none() =>
                    {
                        self.mesh_contour_selection(
                            &s,
                            selection_cfg,
                            level_index,
                            &mut selected_fields,
                        )?;
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
    use crate::config::DataSelectionsConfig;
    use crate::data_config::{BarOrientation, DataBarStyleOverride, DataScatterPointStyleOverride};

    fn declaration() -> SeriesConfig {
        SeriesConfig {
            series_id: "selected".into(),
            source_id: Some("source".into()),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::Xy(crate::data_config::XySeries {
                scatter: Some(DataScatterStyleConfig {
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
                    point_color_map: None,
                    point_size_map: None,
                }),
                line: None,
                errors: None,
                stem: None,
            }),
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
        config.picked_data = Some(DataSelectionsConfig {
            refs: vec![
                point(10),
                PickedDataRef::HistogramBin {
                    source_id: None,
                    series_id: "selected".into(),
                    bin_index: 2,
                    half: None,
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
            [10, 2, 10]
        );
        assert_eq!(
            (
                visuals[0].color.g,
                visuals[0].width_px,
                visuals[0].radius_extra_px
            ),
            (1.0, 5.0, 11.0)
        );
        for (filter, expected) in [
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
        let refs = &mut config.picked_data.as_mut().unwrap().refs;
        refs[0] = PickedDataRef::Point {
            source_id: Some("other".into()),
            series_id: "selected".into(),
            point_index: 10,
        };
        assert!(point_selection_visuals(&config, &series, SelectionRefFilter::Typed(0)).is_empty());
        config.picked_data.as_mut().unwrap().refs[0] = PickedDataRef::Point {
            source_id: None,
            series_id: "selected".into(),
            point_index: 10,
        };
        assert_eq!(
            point_selection_visuals(&config, &series, SelectionRefFilter::Typed(0)).len(),
            1
        );
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
        let packed = SelectionRows {
            global_start: 20,
            packed_instance: Some(7),
            ..rows
        };
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
            RendererDevice::new(device, queue).expect("figgy poll thread"),
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
                .add_column(id, &crate::Column { data: values })
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
                        half: None,
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
                                .unwrap()
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
                                category: None,
                                violin: None,
                                box_points: None,
                                rows: SelectionRows {
                                    global_start: if local { 2 } else { 0 },
                                    refs: SelectionRefFilter::Typed(0),
                                    packed_instance: None,
                                },
                                mapped_point_bg: map.as_ref(),
                                view3d: None,
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
                        layers.into_series_layers(),
                        None,
                        Some(Arc::clone(&style._charge)),
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

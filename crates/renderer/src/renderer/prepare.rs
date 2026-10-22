//! Series preparation for a frame: PrepareContext, the per-kind scratch
//! caches it fills (field, value map, vector, contour and arc prefix) and
//! their signatures. #46 splits this per data kind into data_render/.

mod arc;
mod contour;
mod field;
mod layers;
mod stream;
mod value_map;
mod vector;
use super::*;
pub(in crate::renderer) use arc::*;
pub(in crate::renderer) use contour::*;
pub(in crate::renderer) use field::*;
pub(in crate::renderer) use value_map::*;
pub(in crate::renderer) use vector::*;

/// Clear a per-series scratch cache that is about to exceed its bound.
///
/// Same rule as `arc_cache`: a host cycling through thousands of series ids must
/// not grow a cache without limit, and dropping the lot is simpler than a
/// recency policy nothing has asked for.
pub(super) fn evict_if_full<V>(cache: &mut HashMap<String, V>, series_id: &str, limit: usize) {
    let known = cache.contains_key(series_id);
    if cache.len() > limit || (!known && cache.len() >= limit) {
        cache.clear();
    }
}

// Disjoint GPU borrows let registered preparation read the registry in place.
// This context never owns chart Config or SeriesConfig values.
pub(super) struct PrepareContext<'a> {
    pub(super) device: &'a Arc<wgpu::Device>,
    pub(super) queue: &'a Arc<wgpu::Queue>,
    pub(super) caps: &'a RendererDeviceCaps,
    pub(super) pool: &'a ColumnPool,
    pub(super) gpu_ledger: &'a Arc<GpuLedger>,
    pub(super) style_bgl: &'a wgpu::BindGroupLayout,
    pub(super) field_bgl: &'a wgpu::BindGroupLayout,
    pub(super) vector_bgl: &'a wgpu::BindGroupLayout,
    pub(super) field_cell_tables: &'a data_render::FieldCellTables,
    pub(super) transform_bgl: &'a wgpu::BindGroupLayout,
    pub(super) per_point_style_map_bgl: &'a wgpu::BindGroupLayout,
    pub(super) point_value_map_bgl: &'a wgpu::BindGroupLayout,
    pub(super) neutral_point_value_map_bg: &'a wgpu::BindGroup,
    pub(super) data_selection_bgl: &'a wgpu::BindGroupLayout,
    pub(super) star_data_bgl: &'a wgpu::BindGroupLayout,
    pub(super) sampler: &'a wgpu::Sampler,
    pub(super) quad_vb: &'a TrackedBuffer,
    pub(super) pipelines: &'a mut TargetPipelines,
    pub(super) field_cache: &'a mut HashMap<String, FieldScratch>,
    pub(super) value_map_cache: &'a mut HashMap<String, ValueMapScratch>,
    pub(super) vector_cache: &'a mut HashMap<String, VectorScratch>,
    pub(super) contour_cache: &'a mut HashMap<String, ContourScratchEntry>,
    pub(super) arc_cache: &'a mut HashMap<String, Vec<ArcResultEntry>>,
    pub(super) texture_bgl: &'a wgpu::BindGroupLayout,
    pub(super) target_sample_count: u32,
    pub(super) resident_displays: &'a mut Vec<ResidentDisplay>,
    pub(super) resident_display_serial: &'a mut u64,
    pub(super) contour_label_pipelines: &'a mut Option<crate::gpu_contour::ContourLabelPipelines>,
    pub(super) arc_pipelines: &'a mut Option<data_render::line_arc::ArcScanPipelines>,
    pub(super) surface_format: wgpu::TextureFormat,
    pub(super) renderer_identity: u64,
    pub(super) target_pipeline_generation: u64,
    #[cfg(test)]
    pub(super) arc_chunk_override: Option<u32>,
}

impl PrepareContext<'_> {
    pub(super) fn prepare(&mut self, items: &[ChartDrawItem<'_>]) -> Result<PreparedFrame> {
        // One expansion path: a layered document becomes one item per panel
        // here, so everything below sees single-layer panels only.
        // A frame without layered items is drawn from the caller's slice as
        // it is, so it pays no allocation for expansion.
        let layered = items.iter().any(|item| is_layered(item.chart_config));
        let storage = if layered {
            Some(PanelDrawStorage::expand(items)?)
        } else {
            None
        };
        let group_sizes = storage
            .as_ref()
            .map(|storage| storage.group_sizes(items))
            .unwrap_or_default();
        let expanded = storage.as_ref().map(|storage| storage.items(items));
        let items: &[ChartDrawItem<'_>] = match &expanded {
            Some(expanded) => expanded.as_slice(),
            None => items,
        };
        let mut contour_spacings = Vec::with_capacity(items.len());
        for item in items {
            let mut per_series = Vec::with_capacity(item.series.len());
            for series in item.series {
                validate_contour_level_count(
                    series.config.series_id.as_str(),
                    &series.config.render_type,
                )?;
                validate_line_fill(
                    self.pool,
                    item.series.iter().map(|series| series.config),
                    series.config,
                )?;
                let contour_enabled = effective_series_primitives(
                    &item.chart_config.draw_style,
                    &series.config.render_type,
                )
                .contour;
                per_series.push(preflight_contour_label_spacing(
                    series.config.series_id.as_str(),
                    &series.config.render_type,
                    contour_enabled,
                    1.0,
                )?);
            }
            contour_spacings.push(per_series);
        }
        // The label draw's module and group-1 layout live on the renderer, and
        // the target pipeline is compiled from them — so they have to exist
        // before the one place allowed to compile new pipelines runs.
        self.ensure_contour_label_pipelines(items);
        self.pipelines.ensure_precise_variants_for_items(
            &self.device,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            self.point_value_map_bgl,
            &self.data_selection_bgl,
            &self.field_bgl,
            self.vector_bgl,
            self.contour_label_pipelines.as_ref(),
            self.surface_format,
            items,
        );
        self.pipelines.ensure_styles_for_items(
            &self.device,
            &self.queue,
            &self.transform_bgl,
            &self.style_bgl,
            &self.star_data_bgl,
            self.surface_format,
            items,
        );
        let mut grid_sources = self.ensure_field_scratches(items)?;
        self.ensure_value_map_scratches(items, 1.0)?;
        grid_sources.append(&mut self.ensure_vector_scratches(items, 1.0)?);
        let (prepared_contours, mut contour_sources) =
            self.ensure_contour_scratches(items, 1.0, &contour_spacings)?;
        grid_sources.append(&mut contour_sources);
        let prepared_arcs = self.prepare_arc_items(items)?;
        let (mut prepared, column_sources) = self.resolve_prepared_items(
            items,
            &prepared_arcs,
            &prepared_contours,
            &self.pipelines,
        )?;
        if layered {
            lift_annotation_overlays(&mut prepared, &group_sizes);
        }
        self.attach_resident_displays(items, &mut prepared);
        Ok(PreparedFrame {
            renderer_identity: self.renderer_identity,
            items: prepared,
            column_sources,
            pool_layout_generation: self.pool.layout_generation(),
            target_pipeline_generation: self.target_pipeline_generation,
            grid_sources,
            registered_charts: Vec::new(),
        })
    }

    /// Give every qualifying resident panel its cached composite, drawing it
    /// again only where its [`ResidentDisplayKey`] changed. `inputs` and
    /// `prepared` are the same panels in the same order. All composites drawn
    /// here go out in one submission, ordered before the host's pass.
    pub(super) fn attach_resident_displays(
        &mut self,
        inputs: &[ChartDrawItem<'_>],
        prepared: &mut [PreparedItem],
    ) {
        self.resident_displays
            .retain(|display| display.view.strong_count() > 0);
        let max = self.caps.max_texture_dimension_2d;
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        for (input, item) in inputs.iter().zip(prepared.iter_mut()) {
            if item.stream_display.is_some()
                || item.series.is_empty()
                || !input.chart_config.draw_style.is_precise()
            {
                continue;
            }
            let panel = item.view.panel_rect;
            let (w, h) = (panel.width, panel.height);
            let (Some(data_x), Some(data_y)) = (
                item.data_area.x.checked_sub(panel.x),
                item.data_area.y.checked_sub(panel.y),
            ) else {
                continue;
            };
            if w == 0 || h == 0 || w > max || h > max || data_x >= w || data_y >= h {
                continue;
            }
            let data_area = Rect {
                x: data_x,
                y: data_y,
                width: item.data_area.width,
                height: item.data_area.height,
            };
            let probe = ResidentDisplayProbe {
                size: (w, h),
                data_area,
                transform: data_render::scatter_transform_from_config(input.chart_config),
                chart_config: input.chart_config,
                series: input.series,
                font_generation: crate::text_render::font_generation(),
                columns: self.panel_columns(item.column_fingerprint, input.series),
                pool_layout_generation: self.pool.layout_generation(),
                target_pipeline_generation: self.target_pipeline_generation,
            };
            let view_id = Arc::as_ptr(&item.view.content_revision) as usize;
            let slot = self
                .resident_displays
                .iter()
                .position(|display| display.view.as_ptr() as usize == view_id);
            let current = slot.filter(|&i| self.resident_displays[i].key.matches(&probe));
            let index = match current {
                Some(i) => i,
                None => {
                    let Some(serial) = self.resident_display_serial.checked_add(1) else {
                        continue;
                    };
                    *self.resident_display_serial = serial;
                    let key = ResidentDisplayKey::from_probe(&probe);
                    let reusable = slot.filter(|&i| {
                        let display = &self.resident_displays[i];
                        display.texture.width() == w
                            && display.texture.height() == h
                            && display.format == self.surface_format
                            && display.sample_count == self.target_sample_count
                    });
                    let i = match reusable {
                        Some(i) => {
                            let display = &mut self.resident_displays[i];
                            display.key = key;
                            display.serial = serial;
                            i
                        }
                        None => {
                            let display = self.create_resident_display(
                                Arc::downgrade(&item.view.content_revision),
                                key,
                                serial,
                                w,
                                h,
                            );
                            match slot {
                                Some(i) => {
                                    self.resident_displays[i] = display;
                                    i
                                }
                                None => {
                                    self.resident_displays.push(display);
                                    self.resident_displays.len() - 1
                                }
                            }
                        }
                    };
                    let display = &self.resident_displays[i];
                    let target = display.texture.create_view(&Default::default());
                    let msaa = display
                        .msaa
                        .as_ref()
                        .map(|texture| texture.create_view(&Default::default()));
                    let encoder = encoder.get_or_insert_with(|| {
                        self.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("resident panel composite"),
                            })
                    });
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("resident panel composite"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: msaa.as_ref().unwrap_or(&target),
                            depth_slice: None,
                            resolve_target: msaa.as_ref().map(|_| &target),
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: if msaa.is_some() {
                                    wgpu::StoreOp::Discard
                                } else {
                                    wgpu::StoreOp::Store
                                },
                            },
                        })],
                        ..Default::default()
                    });
                    let series_list: Vec<_> =
                        item.series.iter().map(PreparedSeries::layers).collect();
                    pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
                    pass.set_scissor_rect(
                        data_area.x,
                        data_area.y,
                        data_area.width.min(w - data_area.x),
                        data_area.height.min(h - data_area.y),
                    );
                    data_render::issue_series_bodies(&mut pass, &series_list);
                    i
                }
            };
            let display = &self.resident_displays[index];
            item.resident_display = Some(ResidentDisplayPacket {
                view: view_id,
                serial: display.serial,
                bind_group: display.bind_group.clone(),
                _charge: display.texture.shared_charge(),
            });
        }
        if let Some(encoder) = encoder {
            self.queue.submit([encoder.finish()]);
        }
    }

    /// `looked_up` plus the grid columns of the panel's field, contour and
    /// vector series, from the receipts their scratches were built against.
    pub(super) fn panel_columns(&self, looked_up: u64, series: &[Series<'_>]) -> u64 {
        let receipt = |sources: &GridSources| {
            column_fingerprint(
                sources
                    .ids
                    .iter()
                    .zip(&sources.epochs)
                    .map(|(id, epoch)| (id.as_str(), *epoch)),
            )
        };
        series.iter().fold(looked_up, |sum, series| {
            let id = series.config.series_id.as_str();
            [
                self.field_cache.get(id).map(|scratch| &*scratch.sources),
                self.vector_cache.get(id).map(|scratch| &*scratch.sources),
                self.contour_cache.get(id).map(|entry| &*entry.sources),
            ]
            .into_iter()
            .flatten()
            .fold(sum, |sum, sources| sum.wrapping_add(receipt(sources)))
        })
    }

    /// A panel-sized composite target in the surface format and sample
    /// count, bound for the blit like a raster layer.
    pub(super) fn create_resident_display(
        &self,
        view: std::sync::Weak<std::sync::atomic::AtomicU64>,
        key: ResidentDisplayKey,
        serial: u64,
        w: u32,
        h: u32,
    ) -> ResidentDisplay {
        let target = |sample_count: u32, usage: wgpu::TextureUsages| {
            TrackedTexture::new(
                self.gpu_ledger,
                if sample_count > 1 {
                    GpuResourceKind::MsaaTarget
                } else {
                    GpuResourceKind::PanelTexture
                },
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("resident panel composite"),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.surface_format,
                    usage,
                    view_formats: &[],
                }),
            )
        };
        let texture = target(
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let msaa = (self.target_sample_count > 1).then(|| {
            target(
                self.target_sample_count,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let bind_group = data_render::create_texture_bind_group(
            self.device,
            self.texture_bgl,
            &texture.create_view(&Default::default()),
            self.sampler,
        );
        ResidentDisplay {
            view,
            key,
            serial,
            format: self.surface_format,
            sample_count: self.target_sample_count,
            texture,
            msaa,
            bind_group,
        }
    }

    pub(super) fn resolve_prepared_items(
        &self,
        items: &[ChartDrawItem<'_>],
        prepared_arcs: &[PreparedArcItem],
        prepared_contours: &[PreparedContourItem],
        pipelines: &TargetPipelines,
    ) -> Result<(Vec<PreparedItem>, HashMap<ColumnId, u64>)> {
        let mut prepared = Vec::with_capacity(items.len());
        let mut column_sources = HashMap::new();

        for ((item, prepared_arc), prepared_contour) in
            items.iter().zip(prepared_arcs).zip(prepared_contours)
        {
            let styled = pipelines.style_set(&item.chart_config.draw_style);
            let products = PreparedSeriesProducts {
                arcs: &prepared_arc.series,
                contours: &prepared_contour.series,
            };
            let (layers, sources) = self.build_series_layers(
                item.view,
                item.chart_config,
                item.series,
                pipelines,
                products,
                styled,
            )?;

            let column_fingerprint =
                column_fingerprint(sources.iter().map(|(id, epoch)| (id.as_str(), *epoch)));
            for (id, allocation_epoch) in sources {
                column_sources.entry(id).or_insert(allocation_epoch);
            }

            prepared.push(PreparedItem {
                view: PreparedView::capture(
                    item.view,
                    prepared_arc.view_revision,
                    item.chart_config.annotations.visible,
                ),
                data_area: item
                    .chart_config
                    .data_area()
                    .map(|area| area.0)
                    .unwrap_or(item.view.panel_rect),
                axis_pipeline: pipelines.axis.clone(),
                series: layers
                    .into_iter()
                    .zip(&prepared_arc.series)
                    .zip(item.series)
                    .map(|((layers, arc), series)| {
                        PreparedSeries::from_layers(
                            layers,
                            arc.arc.as_ref().map(|arc| Arc::clone(&arc.charge)),
                            Some(Arc::clone(&series.style._charge)),
                        )
                    })
                    .collect(),
                stream_display: None,
                resident_display: None,
                column_fingerprint,
            });
        }
        Ok((prepared, column_sources))
    }

    /// The value columns and group-3 bind group a mapped scatter draw binds.
    ///
    /// `None` everywhere when the series draws through the plain pipeline. With
    /// a style map but no value map the renderer's neutral group is bound; with
    /// a value map the columns are looked up and length-checked the way the
    /// style-index column is, and the bind group is the one
    /// `ensure_value_map_scratch` resolved.
    #[allow(clippy::type_complexity)]
    pub(super) fn scatter_value_map_columns<'a>(
        &'a self,
        cfg: &SeriesConfig,
        mapped: bool,
        count: usize,
        lookup: &mut impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<(
        Option<ColumnHandle>,
        Option<ColumnHandle>,
        Option<&'a wgpu::BindGroup>,
    )> {
        if !mapped {
            return Ok((None, None, None));
        }
        let scatter = extract_scatter(&cfg.render_type).expect("a mapped series has points");
        if scatter.point_color_map.is_none() && scatter.point_size_map.is_none() {
            return Ok((None, None, Some(self.neutral_point_value_map_bg)));
        }
        let mut value_column = |column: &ColumnId, what: &str| -> Result<ColumnHandle> {
            let handle = lookup(column)?;
            if handle.len_values < count {
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    reason: format!(
                        "{what} column {column:?} has {} values, but scatter uses {count}",
                        handle.len_values
                    ),
                });
            }
            Ok(handle)
        };
        let color_value = scatter
            .point_color_map
            .as_ref()
            .map(|map| value_column(&map.column, "point colour map"))
            .transpose()?;
        let size_value = scatter
            .point_size_map
            .as_ref()
            .map(|map| value_column(&map.column, "point size map"))
            .transpose()?;
        let scratch = self
            .value_map_cache
            .get(cfg.series_id.as_str())
            .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                series_id: cfg.series_id.clone(),
                reason: "point value maps require GPU-resident columns; streamed sources are \
                         not supported"
                    .into(),
            })?;
        Ok((color_value, size_value, Some(&scratch.bind_group)))
    }
}

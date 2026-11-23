//! Series preparation for a frame: PrepareContext, the per-kind scratch
//! caches it fills (field, value map, vector, contour and arc prefix) and
//! their signatures. #46 splits this per data kind into data_render/.

mod arc;
mod contour;
pub(in crate::renderer) mod energy_levels;
mod field;
mod layers;
mod stream;
pub(in crate::renderer) mod value_label;
mod value_map;
mod vector;
use super::*;
pub(in crate::renderer) use arc::*;
pub(in crate::renderer) use contour::*;
pub(in crate::renderer) use energy_levels::*;
pub(in crate::renderer) use field::*;
pub(in crate::renderer) use value_label::*;
pub(in crate::renderer) use value_map::*;
pub(in crate::renderer) use vector::*;

/// A count or pool offset a shader addresses as a 32-bit lane index.
pub(in crate::renderer) fn lane_u32(value: u64, resource: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| FiggyError::GpuResourceLimit {
        resource,
        requested: value,
        limit: u64::from(u32::MAX),
    })
}

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

impl PrepareContext<'_> {
    /// Run `body` as one frame's batch (`in_frame` set, restored after).
    pub(super) fn frame_batch<T>(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let outer = std::mem::replace(&mut self.in_frame, true);
        let result = body(self);
        self.in_frame = outer;
        result
    }
}

/// Bound a per-series scratch cache before a frame resolves `items`: when
/// it is full, keep only the entries the frame's series use. Inside the
/// frame nothing is evicted, so a frame of more series than `limit` keeps
/// every scratch its layers read (rule 10: the bound evicts, it never caps
/// what one frame draws); the cache then holds the frame's series and no
/// more.
pub(super) fn retain_frame_series<V>(
    cache: &mut HashMap<String, V>,
    items: &[PanelDraw<'_>],
    limit: usize,
) {
    if cache.len() < limit {
        return;
    }
    cache.retain(|id, _| {
        items
            .iter()
            .any(|item| item.series.iter().any(|s| s.config.series_id == *id))
    });
}

// Disjoint GPU borrows let registered preparation read the registry in place.
// This context never owns chart Config or SeriesConfig values.
pub(super) struct PrepareContext<'a> {
    /// True while one frame's batch (`ensure_*_scratches`) resolves its
    /// items: a scratch cache was bounded before the batch began
    /// (`retain_frame_series`) and evicts nothing while it runs, so every
    /// scratch the frame's layers read is still there (rule 10). A lone
    /// preparation (a pick) evicts as before.
    pub(super) in_frame: bool,
    pub(super) device: &'a wgpu::Device,
    pub(super) queue: &'a wgpu::Queue,
    pub(super) caps: &'a RendererDeviceCaps,
    pub(super) pool: &'a ColumnPool,
    /// The monotonic envelopes (#271): a monotonic series is drawn only
    /// from an envelope built for its panel's mapping, and its selection
    /// rows map through it.
    pub(super) monotonic: &'a monotonic::MonotonicState,
    pub(super) gpu_ledger: &'a Arc<GpuLedger>,
    pub(super) style_bgl: &'a wgpu::BindGroupLayout,
    pub(super) field_bgl: &'a wgpu::BindGroupLayout,
    pub(super) surface_bgl: &'a wgpu::BindGroupLayout,
    pub(super) region_bgl: &'a wgpu::BindGroupLayout,
    pub(super) vector_bgl: &'a wgpu::BindGroupLayout,
    pub(super) category_bar_data_bgl: &'a wgpu::BindGroupLayout,
    /// Group 3 of the parallel entries (`create_parallel_data_bind_group_layout`).
    pub(super) parallel_bgl: &'a wgpu::BindGroupLayout,
    /// Group 1 of the value label pipelines (`create_value_label_bind_group_layout`).
    pub(super) value_label_bgl: &'a wgpu::BindGroupLayout,
    pub(super) field_cell_tables: &'a data_render::FieldCellTables,
    pub(super) transform_bgl: &'a wgpu::BindGroupLayout,
    pub(super) per_point_style_map_bgl: &'a wgpu::BindGroupLayout,
    pub(super) point_value_map_bgl: &'a wgpu::BindGroupLayout,
    pub(super) neutral_point_value_map_bg: &'a wgpu::BindGroup,
    pub(super) neutral_point_value_map_buffers: &'a data_render::PointValueMapBuffers,
    pub(super) view3d_value_map_bgl: &'a wgpu::BindGroupLayout,
    pub(super) data_selection_bgl: &'a wgpu::BindGroupLayout,
    pub(super) star_data_bgl: &'a wgpu::BindGroupLayout,
    pub(super) sampler: &'a wgpu::Sampler,
    pub(super) quad_vb: &'a TrackedBuffer,
    pub(super) pipelines: &'a mut TargetPipelines,
    pub(super) field_cache: &'a mut HashMap<String, FieldScratch>,
    pub(super) value_map_cache: &'a mut HashMap<String, ValueMapScratch>,
    pub(super) vector_cache: &'a mut HashMap<String, VectorScratch>,
    pub(super) contour_cache: &'a mut HashMap<String, ScratchEntry>,
    pub(super) arc_cache: &'a mut HashMap<String, Vec<ArcResultEntry>>,
    pub(super) texture_bgl: &'a wgpu::BindGroupLayout,
    pub(super) target_sample_count: u32,
    pub(super) resident_displays: &'a mut Vec<ResidentDisplay>,
    pub(super) resident_display_serial: &'a mut u64,
    pub(super) ink_masks: &'a mut Vec<InkMask>,
    pub(super) ink_mask_pipeline: &'a mut Option<InkMaskPipeline>,
    /// The registered chart each item of an on-screen prepare draws, in
    /// item order: its panels' ink masks are drawn (#381). Empty for every
    /// other preparation (a pick's, a host-owned chart's, an export's).
    pub(super) ink_charts: &'a [ChartId],
    pub(super) contour_label_pipelines: &'a mut Option<crate::gpu_contour::ContourLabelPipelines>,
    pub(super) arc_pipelines: &'a mut Option<data_render::line_arc::ArcScanPipelines>,
    pub(super) surface_format: wgpu::TextureFormat,
    pub(super) renderer_identity: u64,
    pub(super) target_pipeline_generation: u64,
    #[cfg(test)]
    pub(super) arc_chunk_override: Option<u32>,
}

impl PrepareContext<'_> {
    pub(super) fn prepare(&mut self, items: &[PanelDraw<'_>]) -> Result<PreparedFrame> {
        let source = items;
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
            .transpose()?
            // No layered item: no groups to lift annotations across.
            .unwrap_or_default();
        let expanded = storage
            .as_ref()
            .map(|storage| storage.items(items))
            .transpose()?;
        let items: &[PanelDraw<'_>] = match &expanded {
            Some(expanded) => expanded.as_slice(),
            None => items,
        };
        // A monotonic series is drawn only from an envelope built for its
        // panel's x-to-pixel mapping (#271).
        self.monotonic.check_panels(items)?;
        // One Vec of per-panel products for the whole frame; each phase below
        // fills its part in place.
        let mut panels = Vec::with_capacity(items.len());
        for item in items {
            let mut per_series = Vec::with_capacity(item.series.len());
            for series in item.series {
                validate_contour_level_count(
                    series.config.series_id.as_str(),
                    &series.config.render_type,
                )?;
                // The host-owned path checks polar axes too, as registration
                // and export do (#160).
                item.chart_config
                    .validate_series_on_polar(series.config)
                    .map_err(|reason| FiggyError::InvalidSeriesConfig {
                        series_id: series.config.series_id.clone(),
                        kind: reason,
                    })?;
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
            panels.push(PreparedPanelProducts::with_contour_spacings(per_series));
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
            [self.surface_bgl, self.view3d_value_map_bgl],
            &self.data_selection_bgl,
            &self.field_bgl,
            self.vector_bgl,
            self.star_data_bgl,
            self.category_bar_data_bgl,
            self.parallel_bgl,
            self.contour_label_pipelines.as_ref(),
            self.surface_format,
            items,
        );
        self.pipelines.ensure_surface_pipelines(
            &self.device,
            &self.transform_bgl,
            &self.style_bgl,
            &self.field_bgl,
            self.surface_bgl,
            self.surface_format,
            items,
        );
        self.pipelines.ensure_ternary_region_pipelines(
            &self.device,
            &self.transform_bgl,
            self.region_bgl,
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
        grid_sources.append(&mut self.ensure_vector_scratches(items, None)?);
        let mut contour_sources = self.ensure_contour_scratches(items, 1.0, &mut panels)?;
        grid_sources.append(&mut contour_sources);
        grid_sources.append(&mut self.ensure_mesh_scratches(items, 1.0)?);
        self.prepare_arc_items(items, &mut panels)?;
        let (mut prepared, column_sources) =
            self.resolve_prepared_items(items, &panels, &self.pipelines)?;
        self.attach_resident_displays(items, &mut prepared);
        self.attach_ink_masks(source, &group_sizes, items, &prepared);
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
        inputs: &[PanelDraw<'_>],
        prepared: &mut [PreparedItem],
    ) {
        self.resident_displays
            .retain(|display| display.view.strong_count() > 0);
        let max = self.caps.max_texture_dimension_2d;
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        for (input, item) in inputs.iter().zip(prepared.iter_mut()) {
            if !input.chart_config.draw_style.is_precise() {
                continue;
            }
            let Some((probe, pieces)) = self.panel_probe(input, item, max) else {
                continue;
            };
            let (w, h) = probe.size;
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
                            && display.depth.is_some() == probe.needs_depth_stencil()
                    });
                    let i = match reusable {
                        Some(i) => {
                            let display = &mut self.resident_displays[i];
                            display.key = key;
                            display.serial = serial;
                            i
                        }
                        None => {
                            let depth = key.camera.is_some() || key.stencil;
                            let display = self.create_resident_display(
                                Arc::downgrade(&item.view.content_revision),
                                key,
                                serial,
                                w,
                                h,
                                depth,
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
                    let depth = display
                        .depth
                        .as_ref()
                        .map(|texture| texture.create_view(&Default::default()));
                    let series_list: Vec<_> =
                        item.series.iter().map(PreparedSeries::layers).collect();
                    let own = item.own_piece_layers();
                    draw_panel_composite(
                        encoder,
                        PanelCompositeTargets {
                            color: msaa.as_ref().unwrap_or(&target),
                            resolve: msaa.as_ref().map(|_| &target),
                            depth: depth.as_ref(),
                            depth_carried: None,
                        },
                        (w, h),
                        &pieces,
                        super::composite::PieceSeries::of(&series_list, &own),
                        CompositeSteps::All,
                    );
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

    /// Draw the ink mask of every panel of each registered chart in
    /// `source` ([`Self::ink_charts`]) whose data layer changed since its
    /// mask was drawn (#381): each series of the panel on its own into a
    /// transient transparent target, as the resident display draws it, and
    /// after each the resolve pass that writes its slot where it inked, so
    /// the topmost owner is left. `inputs` and `prepared` are
    /// the expanded panels, `group_sizes` how many each source item drew as
    /// (empty when none is layered). A panel that draws no data layer has
    /// no mask. All masks drawn here go out in one submission.
    pub(super) fn attach_ink_masks(
        &mut self,
        source: &[PanelDraw<'_>],
        group_sizes: &[usize],
        inputs: &[PanelDraw<'_>],
        prepared: &[PreparedItem],
    ) {
        if self.ink_charts.len() != source.len() || inputs.len() != prepared.len() {
            return;
        }
        let max = self.caps.max_texture_dimension_2d;
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        // Series passes recorded into the current command buffer.
        let mut batched = 0usize;
        let mut first = 0;
        for (index, chart) in self.ink_charts.iter().enumerate() {
            let count = group_sizes.get(index).copied().unwrap_or(1);
            let panels = first..first + count;
            first += count;
            // Panels the chart no longer draws as lose their masks.
            self.ink_masks
                .retain(|mask| mask.chart != *chart || mask.panel < count);
            for (panel, position) in panels.enumerate() {
                let (input, item) = (&inputs[position], &prepared[position]);
                let slot = self
                    .ink_masks
                    .iter()
                    .position(|mask| mask.chart == *chart && mask.panel == panel);
                let Some((probe, pieces)) = self.panel_probe(input, item, max) else {
                    if let Some(slot) = slot {
                        self.ink_masks.swap_remove(slot);
                    }
                    continue;
                };
                let panel_rect = item.view.panel_rect;
                if let Some(slot) = slot.filter(|&slot| self.ink_masks[slot].key.matches(&probe)) {
                    self.ink_masks[slot].panel_rect = panel_rect;
                    continue;
                }
                let (w, h) = probe.size;
                let key = ResidentDisplayKey::from_probe(&probe);
                let reusable = slot.filter(|&slot| {
                    let texture = &self.ink_masks[slot].texture;
                    texture.width() == w && texture.height() == h
                });
                // The series each slot names, in drawing order (#357: a
                // range link series draws under its bars).
                let owners: Arc<[crate::gpu_pick::InkSeries]> =
                    crate::data_config::series_draw_order(input.series.len(), |i| {
                        input.series[i].config
                    })
                    .map(|i| &input.series[i])
                    .map(|series| crate::gpu_pick::InkSeries {
                        source_id: series.config.source_id.clone(),
                        series_id: series.config.series_id.clone(),
                    })
                    .collect();
                let mask = match reusable {
                    Some(slot) => {
                        let mask = &mut self.ink_masks[slot];
                        mask.key = key;
                        mask.panel_rect = panel_rect;
                        mask.owners = owners;
                        slot
                    }
                    None => {
                        let mask = InkMask {
                            chart: *chart,
                            panel,
                            key,
                            panel_rect,
                            owners,
                            texture: self.panel_texture(
                                "panel ink mask",
                                (w, h),
                                1,
                                INK_MASK_FORMAT,
                                wgpu::TextureUsages::RENDER_ATTACHMENT
                                    | wgpu::TextureUsages::COPY_SRC,
                            ),
                        };
                        match slot {
                            Some(slot) => {
                                self.ink_masks[slot] = mask;
                                slot
                            }
                            None => {
                                self.ink_masks.push(mask);
                                self.ink_masks.len() - 1
                            }
                        }
                    }
                };
                // The data layer, drawn as the resident display draws it,
                // into targets that live until the submission completes.
                let layer = self.panel_texture(
                    "panel ink layer",
                    (w, h),
                    1,
                    self.surface_format,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                );
                let msaa = (self.target_sample_count > 1).then(|| {
                    self.panel_texture(
                        "panel ink layer",
                        (w, h),
                        self.target_sample_count,
                        self.surface_format,
                        wgpu::TextureUsages::RENDER_ATTACHMENT,
                    )
                });
                let depth = probe.needs_depth_stencil().then(|| {
                    create_composite_depth(
                        self.device,
                        self.gpu_ledger,
                        (w, h),
                        self.target_sample_count,
                    )
                });
                let layer_view = layer.create_view(&Default::default());
                let msaa_view = msaa
                    .as_ref()
                    .map(|texture| texture.create_view(&Default::default()));
                let depth_view = depth
                    .as_ref()
                    .map(|texture| texture.create_view(&Default::default()));
                let encoder = encoder.get_or_insert_with(|| {
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("panel ink masks"),
                        })
                });
                let series_list: Vec<_> = item.series.iter().map(PreparedSeries::layers).collect();
                let own = item.own_piece_layers();
                let all = super::composite::PieceSeries::of(&series_list, &own);
                let pipeline = self
                    .ink_mask_pipeline
                    .get_or_insert_with(|| InkMaskPipeline::new(self.device));
                let source = pipeline.source(self.device, &layer_view);
                let mask_view = self.ink_masks[mask]
                    .texture
                    .create_view(&Default::default());
                // A panel without series is cleared to no ink.
                if series_list.is_empty() {
                    pipeline.record(encoder, &source, &mask_view, 0, true);
                }
                // Each series alone, in the frame's order: every series'
                // fills, then every series' bodies. Its slot overwrites what
                // was recorded before wherever it inks, so a marker over a
                // later series' fill stays the owner. A 3D panel keeps its
                // depth from pass to pass. A series with no fill skips the
                // fill step.
                let mut first = true;
                for steps in [CompositeSteps::Fills, CompositeSteps::Bodies] {
                    for (index, slot) in (0..series_list.len()).zip(1u32..) {
                        let layers = &series_list[index];
                        if steps == CompositeSteps::Fills
                            && layers.area.is_none()
                            && layers.region.is_none()
                        {
                            continue;
                        }
                        draw_panel_composite(
                            encoder,
                            PanelCompositeTargets {
                                color: msaa_view.as_ref().unwrap_or(&layer_view),
                                resolve: msaa_view.as_ref().map(|_| &layer_view),
                                depth: depth_view.as_ref(),
                                depth_carried: Some(!first),
                            },
                            (w, h),
                            &pieces,
                            all.series(index),
                            steps,
                        );
                        pipeline.record(encoder, &source, &mask_view, slot, first);
                        first = false;
                        // The passes go out in batches, so no command buffer
                        // grows with the series count: a long one can trip
                        // a driver's per-command-buffer time limit (Metal's
                        // watchdog lost the device at 5,000 series in one).
                        // The batch size is not a cap; every series is
                        // recorded.
                        batched += 1;
                        if batched == INK_MASK_PASSES_PER_SUBMIT {
                            let full = std::mem::replace(
                                encoder,
                                self.device.create_command_encoder(
                                    &wgpu::CommandEncoderDescriptor {
                                        label: Some("panel ink masks"),
                                    },
                                ),
                            );
                            self.queue.submit([full.finish()]);
                            batched = 0;
                        }
                    }
                }
            }
        }
        if let Some(encoder) = encoder {
            self.queue.submit([encoder.finish()]);
        }
    }

    /// A panel-sized texture charged to the device ledger: a multisampled
    /// target when `sample_count` is above 1, a panel texture otherwise.
    fn panel_texture(
        &self,
        label: &'static str,
        (w, h): (u32, u32),
        sample_count: u32,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> TrackedTexture {
        TrackedTexture::new(
            self.gpu_ledger,
            if sample_count > 1 {
                GpuResourceKind::MsaaTarget
            } else {
                GpuResourceKind::PanelTexture
            },
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            }),
        )
    }

    /// The data layer of `item` (drawn from `input`) as a
    /// [`ResidentDisplayProbe`], with its data pieces relative to the
    /// panel: `None` for a streamed display, a panel without series, or a
    /// panel past `max` or whose data area falls outside it. Both panel
    /// caches of the data layer (the resident display and the ink mask)
    /// are keyed by it.
    pub(super) fn panel_probe<'p>(
        &self,
        input: &PanelDraw<'p>,
        item: &PreparedItem,
        max: u32,
    ) -> Option<(ResidentDisplayProbe<'p>, super::composite::DataPieces)> {
        if item.stream_display.is_some() || item.series.is_empty() {
            return None;
        }
        let panel = item.view.panel_rect;
        let (w, h) = (panel.width, panel.height);
        let data_x = item.data_area.x.checked_sub(panel.x)?;
        let data_y = item.data_area.y.checked_sub(panel.y)?;
        if w == 0 || h == 0 || w > max || h > max || data_x >= w || data_y >= h {
            return None;
        }
        let data_area = Rect {
            x: data_x,
            y: data_y,
            width: item.data_area.width,
            height: item.data_area.height,
        };
        let pieces = item.data_pieces.relative_to(panel.x, panel.y)?;
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
            camera: panel_camera(input.chart_config, item.view.panel_rect),
            stencil: panel_stencil(input.chart_config, input.series),
        };
        Some((probe, pieces))
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
                self.contour_cache
                    .get(id)
                    .and_then(ScratchEntry::contour)
                    .map(|entry| &*entry.sources),
            ]
            .into_iter()
            .flatten()
            .fold(sum, |sum, sources| sum.wrapping_add(receipt(sources)))
        })
    }

    /// A panel-sized composite target in the surface format and sample
    /// count, bound for the blit like a raster layer, with a depth-stencil
    /// attachment when `depth` (a 3D panel, or a ternary panel with a
    /// region fill).
    pub(super) fn create_resident_display(
        &self,
        view: std::sync::Weak<std::sync::atomic::AtomicU64>,
        key: ResidentDisplayKey,
        serial: u64,
        w: u32,
        h: u32,
        depth: bool,
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
        let depth = depth.then(|| {
            create_composite_depth(
                self.device,
                self.gpu_ledger,
                (w, h),
                self.target_sample_count,
            )
        });
        let bind_group = data_render::create_texture_bind_group(
            self.device,
            self.texture_bgl,
            &texture.create_view(&Default::default()),
            self.sampler,
        );
        ResidentDisplay {
            depth,
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
        items: &[PanelDraw<'_>],
        panels: &[PreparedPanelProducts],
        pipelines: &TargetPipelines,
    ) -> Result<(Vec<PreparedItem>, HashMap<ColumnId, u64>)> {
        if items.len() != panels.len() {
            return Err(FiggyError::internal("one prepared panel per draw item"));
        }
        let mut prepared = Vec::with_capacity(items.len());
        let mut column_sources = HashMap::new();

        for (item, panel) in items.iter().zip(panels) {
            // A chart with an axis break, a probability axis or polar
            // coordinates is drawn with the full shaders only (#299). Every entry admits its charts first
            // (`Renderer::admit_full_shaders`), so this fails only for an entry
            // that did not, instead of drawing the chart without them.
            if pipelines.shaders.set == ShaderSet::Basic
                && config_needs_full_shaders(item.chart_config)
            {
                return Err(FiggyError::internal(
                    "a chart with an axis break, a probability axis or polar coordinates is drawn with the full shaders",
                ));
            }
            let (prepared_arc, prepared_contour) = (&panel.arc, &panel.contour);
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

            let data_pieces = super::composite::DataPieces::of(
                item.chart_config,
                item.chart_config
                    .data_area()
                    .map(|area| area.0)
                    .unwrap_or(item.view.panel_rect),
            );
            let piece_transforms = self.piece_transforms(item.chart_config, &data_pieces);
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
                data_pieces,
                piece_transforms,
                axis_pipeline: pipelines.axis.clone(),
                // In drawing order (#357): a range link series listed
                // after its bars draws under them. The ink mask's owners
                // and the pick's paint order follow the same order.
                series: {
                    let mut layers = layers;
                    if layers.len() != item.series.len()
                        || prepared_arc.series.len() < item.series.len()
                    {
                        return Err(FiggyError::internal("one prepared layer set per series"));
                    }
                    crate::data_config::series_draw_order(item.series.len(), |i| {
                        item.series[i].config
                    })
                    .map(|i| {
                        PreparedSeries::from_layers(
                            std::mem::replace(&mut layers[i], data_render::SeriesLayers::empty()),
                            prepared_arc.series[i]
                                .arc
                                .as_ref()
                                .map(|arc| Arc::clone(&arc.charge)),
                            Some(Arc::clone(&item.series[i].style._charge)),
                        )
                    })
                    .collect()
                },
                stream_display: None,
                resident_display: None,
                column_fingerprint,
            });
        }
        Ok((prepared, column_sources))
    }

    /// The transform of each shifted piece of a `Panels` axis (#200): the
    /// panel's transform with its offset moved by the piece's shift, in the
    /// same panel `t` the offset is in.
    fn piece_transforms(
        &self,
        config: &crate::config::Config,
        pieces: &super::composite::DataPieces,
    ) -> [Option<super::frame::PieceTransform>; super::composite::MAX_DATA_PIECES] {
        let mut out = super::frame::no_piece_transforms();
        if !pieces.shifted() {
            return out;
        }
        let base = data_render::scatter_transform_from_config(config);
        for (slot, shift) in out.iter_mut().zip(pieces.shifts()) {
            if shift[0] == 0.0 && shift[1] == 0.0 {
                continue;
            }
            let mut transform = base;
            for axis in 0..2 {
                transform.data_to_panel_offset[axis] +=
                    (shift[axis] * f64::from(base.data_to_panel_scale[axis])) as f32;
            }
            let buffer = TrackedBuffer::new(
                self.gpu_ledger,
                GpuResourceKind::Uniform,
                data_render::create_scatter_transform_uniform_buffer(self.device, &transform),
            );
            let bind_group = data_render::create_scatter_transform_bind_group(
                self.device,
                self.transform_bgl,
                &buffer,
            );
            *slot = Some(super::frame::PieceTransform {
                _buffer: buffer,
                bind_group,
            });
        }
        out
    }

    /// The value-map buffers a mapped scatter series on a 3D chart binds
    /// beside the camera (#132): the series' own when it has a value map,
    /// else the renderer's neutral ones; `None` for a plain series.
    pub(super) fn scatter_value_map_buffers(
        &self,
        cfg: &SeriesConfig,
        mapped: bool,
    ) -> Option<&data_render::PointValueMapBuffers> {
        if !mapped {
            return None;
        }
        let scatter = extract_scatter(&cfg.render_type)?;
        if scatter.point_color_map.is_none() && scatter.point_size_map.is_none() {
            return Some(self.neutral_point_value_map_buffers);
        }
        self.value_map_cache
            .get(cfg.series_id.as_str())
            .map(|scratch| &scratch.buffers)
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
        let scatter = extract_scatter(&cfg.render_type)
            .ok_or(FiggyError::internal("a mapped series has points"))?;
        if scatter.point_color_map.is_none() && scatter.point_size_map.is_none() {
            return Ok((None, None, Some(self.neutral_point_value_map_bg)));
        }
        let mut value_column = |column: &ColumnId, what: &str| -> Result<ColumnHandle> {
            let handle = lookup(column)?;
            if handle.len_values < count {
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    kind: crate::RuleKind::PointMapColumnShort {
                        what: what.to_string(),
                        column: column.to_string(),
                        len: handle.len_values as u64,
                        need: count as u64,
                    },
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
                kind: crate::RuleKind::StreamedPointValueMap,
            })?;
        Ok((color_value, size_value, Some(&scratch.bind_group)))
    }
}

/// A 3D panel's camera uniform for a composite target the size of `panel`,
/// or `None` for a flat panel: the resident key compares it, and the
/// composite's surfaces draw with it.
pub(super) fn panel_camera(
    config: &Config,
    panel: Rect,
) -> Option<data_render::surface_mesh::View3dGpu> {
    let target = (
        (f64::from(panel.x), f64::from(panel.y)),
        (f64::from(panel.width), f64::from(panel.height)),
    );
    data_render::surface_mesh::view3d_gpu(config, target, target)
}

/// Whether a panel's composite needs the stencil attachment: a ternary
/// chart with a region fill among its series.
pub(super) fn panel_stencil(config: &Config, series: &[Series<'_>]) -> bool {
    config.ternary().is_some()
        && series
            .iter()
            .any(|series| series.config.render_type.ternary_region().is_some())
}

/// The depth-stencil attachment of a 3D panel's composite, charged with
/// the composite.
pub(super) fn create_composite_depth(
    device: &wgpu::Device,
    ledger: &Arc<crate::gpu_memory::GpuLedger>,
    (w, h): (u32, u32),
    sample_count: u32,
) -> TrackedTexture {
    TrackedTexture::new(
        ledger,
        if sample_count > 1 {
            GpuResourceKind::MsaaTarget
        } else {
            GpuResourceKind::PanelTexture
        },
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("panel composite depth-stencil"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: data_render::surface_mesh::DEPTH_STENCIL_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        }),
    )
}

/// How many series passes of the ink masks (#381) one command buffer holds
/// before it is submitted and the next one begins: a batching parameter of
/// the recording, so a mask's work does not grow one command buffer with
/// the series count. Every series is recorded whatever the count (rule 10).
const INK_MASK_PASSES_PER_SUBMIT: usize = 64;

/// Which steps of a panel's data a composite pass draws: every step, or
/// one of the two the frame draws in order, the fills (with the ternary
/// region fills under them) and then the bodies (with a 3D panel's
/// surfaces). The ink mask records each step series by series, so a
/// marker over a later series' fill stays the owner (#381).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompositeSteps {
    All,
    Fills,
    Bodies,
}

/// The attachments of a panel composite pass.
pub(super) struct PanelCompositeTargets<'t> {
    pub(super) color: &'t wgpu::TextureView,
    pub(super) resolve: Option<&'t wgpu::TextureView>,
    /// A 3D panel's depth-stencil attachment.
    pub(super) depth: Option<&'t wgpu::TextureView>,
    /// The ink mask's series passes (#381): `Some(carried)` keeps a 3D
    /// panel's depth for the next series, and `carried` loads it from the
    /// series before instead of clearing it. `None` (the resident display
    /// and the export) clears it and discards it after the pass. A flat
    /// panel's stencil is always cleared and discarded.
    pub(super) depth_carried: Option<bool>,
}

/// Draw one panel's bodies into its composite: cleared, scissored to the
/// data area. A 3D panel draws its surfaces with the depth test; a flat one
/// its series bodies, after the ternary region fills when its attachment
/// carries the stencil. Shared by the resident display and the export.
pub(super) fn draw_panel_composite(
    encoder: &mut wgpu::CommandEncoder,
    targets: PanelCompositeTargets<'_>,
    (w, h): (u32, u32),
    data: &super::composite::DataPieces,
    series: super::composite::PieceSeries<'_, '_>,
    steps: CompositeSteps,
) {
    let fills = steps != CompositeSteps::Bodies;
    let bodies = steps != CompositeSteps::Fills;
    let issue_bodies =
        |pass: &mut wgpu::RenderPass<'_>, list: &[data_render::SeriesLayers<'_>]| match steps {
            CompositeSteps::All => data_render::issue_series_bodies(pass, list),
            CompositeSteps::Fills => data_render::issue_series_fills(pass, list),
            CompositeSteps::Bodies => data_render::issue_series_bodies_without_fills(pass, list),
        };
    let three_d = series.piece(0).iter().any(|s| {
        s.surface.is_some()
            || s.floor.is_some()
            || s.line_3d.is_some()
            || s.scatter_3d.is_some()
            || s.vector_3d.is_some()
    });
    // A flat panel with the stencil attachment (a ternary chart with a
    // region fill) draws in two passes: the region fills into the stencil,
    // then its bodies with the 2D pipelines, which carry no depth-stencil
    // state and so cannot join a pass that has the attachment. The colour
    // target is kept between the two and resolved by the second.
    let two_passes = targets.depth.is_some() && !three_d;
    // Each piece of the data area in turn (#200): the whole area on an
    // unbroken chart.
    let scissor = |pass: &mut wgpu::RenderPass<'_>, piece: &Rect| {
        pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        pass.set_scissor_rect(
            piece.x,
            piece.y,
            piece.width.min(w.saturating_sub(piece.x)),
            piece.height.min(h.saturating_sub(piece.y)),
        );
    };
    // Each piece with the series layers it is drawn with.
    let pieces = || {
        data.rects()
            .iter()
            .enumerate()
            .filter(|(_, piece)| piece.x < w && piece.y < h)
            .map(|(k, piece)| (piece, series.piece(k)))
    };
    let final_store = if targets.resolve.is_some() {
        wgpu::StoreOp::Discard
    } else {
        wgpu::StoreOp::Store
    };
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("panel composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: targets.color,
                depth_slice: None,
                resolve_target: if two_passes { None } else { targets.resolve },
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: if two_passes {
                        wgpu::StoreOp::Store
                    } else {
                        final_store
                    },
                },
            })],
            depth_stencil_attachment: targets.depth.map(|view| {
                // A 3D panel's ink mask draws one series per pass and keeps
                // the depth between them (#381).
                let carried = targets.depth_carried.filter(|_| three_d);
                wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: if carried == Some(true) {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(1.0)
                        },
                        store: if carried.is_some() {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }
            }),
            ..Default::default()
        });
        // The ternary region fills first, into the stencil, under every
        // fill and body; a 3D panel then draws its surfaces, a flat one
        // its bodies (in the second pass when the stencil is attached).
        for (piece, series_list) in pieces() {
            scissor(&mut pass, piece);
            if targets.depth.is_some() && fills {
                data_render::issue_series_regions(&mut pass, series_list);
            }
            if three_d {
                if bodies {
                    data_render::issue_series_surfaces(&mut pass, series_list);
                }
            } else if !two_passes {
                issue_bodies(&mut pass, series_list);
            }
        }
    }
    if two_passes {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("panel composite bodies"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: targets.color,
                depth_slice: None,
                resolve_target: targets.resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: final_store,
                },
            })],
            ..Default::default()
        });
        for (piece, series_list) in pieces() {
            scissor(&mut pass, piece);
            issue_bodies(&mut pass, series_list);
        }
    }
}

//! Series preparation for a frame: PrepareContext, the per-kind scratch
//! caches it fills (field, value map, vector, contour and arc prefix) and
//! their signatures. #46 splits this per data kind into data_render/.

mod contour;
mod field;
mod vector;
use super::*;
pub(in crate::renderer) use contour::*;
pub(in crate::renderer) use field::*;
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

/// Per-series point value-map scratch cache limit, mirroring the field limit.
pub(super) const VALUE_MAP_SCRATCH_CACHE_LIMIT: usize = 256;

/// What a cached point value-map bind group (group 3) was built from: the
/// params uniform, compared by its bytes so a NaN `nan_color` still compares
/// equal to itself, and the colourbar's colormap (SSOT config, copied). Equality
/// means the cached bind group is still exactly right, so a prepare that
/// changes nothing uploads nothing — the same contract as [`FieldSignature`].
#[derive(Debug)]
pub(super) struct ValueMapSignature {
    pub(super) params: data_render::PointValueMapParamsGpu,
    pub(super) colormap: Option<crate::colormap::ColorMap>,
}

impl PartialEq for ValueMapSignature {
    fn eq(&self, other: &Self) -> bool {
        bytemuck::bytes_of(&self.params) == bytemuck::bytes_of(&other.params)
            && self.colormap == other.colormap
    }
}

/// One scatter series' resolved value-map state: the group-3 bind group of
/// the mapped scatter and pick-ring pipelines.
pub(super) struct ValueMapScratch {
    pub(super) signature: ValueMapSignature,
    pub(super) bind_group: wgpu::BindGroup,
    pub(super) _charge: crate::gpu_memory::SharedCharge,
}

/// A dashed series' GPU arc-length prefix: the buffer (bound as the line
/// pipeline's vertex slots 4/5) and its used byte length.
pub(super) type ArcPrefix = (Arc<wgpu::Buffer>, u64);

pub(super) const ARC_PREFIX_CACHE_LIMIT: usize = 256;

pub(super) const ARC_RESULT_CACHE_LIMIT_PER_SERIES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ArcResultKey {
    pub(super) n: u32,
    pub(super) x_base: u32,
    pub(super) y_base: u32,
    pub(super) pool_layout_generation: u64,
    pub(super) x_allocation_epoch: u64,
    pub(super) y_allocation_epoch: u64,
    /// The arc scan reads the first 64 bytes of `ScatterTransform` and its
    /// polar words (`polar_angle`, `polar_extent`, the last 32 bytes); the
    /// style parameters between them are left out, so a style change does
    /// not retraverse the data.
    pub(super) geometry_transform_bits: [u32; 16],
    pub(super) polar_transform_bits: [u32; 8],
    pub(super) star_pitch_bits: Option<u32>,
    /// Step interpolation mode: a stepped path has a different arc length.
    pub(super) step_mode: u32,
}

pub(super) struct ArcResultEntry {
    pub(super) key: ArcResultKey,
    pub(super) scratch: data_render::line_arc::ArcScratch,
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

    /// Per-item body of the prepare phase (also reused by export). For each
    /// panel: computes ONE transform snapshot from `item.chart_config`,
    /// writes it to the panel's transform uniform, dispatches the arc-length
    /// scan of every line series that needs one with that same snapshot —
    /// dashed lines (dash phase) and every line of a style with
    /// `needs_arc_prefix` (sketch: the wobble is parameterized by arc
    /// length, so solid sketch lines need the prefix too). The immutable draw
    /// phase later reads the owned result, never the live `arc_cache`.
    /// Resolve every field series' group-2 state, reusing what has not changed.
    ///
    /// The prepare-phase `&mut self` half of the field path, next to
    /// `prepare_arc_items` and for the same reason: `build_series_layers` is
    /// `&self`, so anything that has to be built has to be built here. Returns
    /// the per-field source receipts in `items` order for the token to hold.
    pub(super) fn ensure_value_map_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<()> {
        for item in items {
            if !item.chart_config.draw_style.is_precise() {
                // Stylized draw styles ignore per-point style mappings, value
                // maps included, exactly as the style table is ignored.
                continue;
            }
            for series in item.series {
                self.ensure_value_map_scratch(item.chart_config, series.config, scale)?;
            }
        }
        Ok(())
    }

    /// Build or reuse one scatter series' value-map bind group (group 3).
    ///
    /// The value-map twin of `ensure_field_scratch`: the colourbar lives in
    /// the chart `Config`, not in the series, so it has to be read here where
    /// both are in hand, and `build_series_layers` is `&self`. Everything the
    /// shader derives per point — the colour from the colourbar ramp, the radius
    /// from the size domain — is computed on every draw from the value columns;
    /// this uploads only the parameters and the ramp's control points, and only
    /// when the signature changed. `scale` is the export DPI multiplier applied
    /// to the size map's pixel range.
    pub(super) fn ensure_value_map_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<()> {
        let Some(scatter) = extract_scatter(&cfg.render_type) else {
            return Ok(());
        };
        if scatter.point_color_map.is_none() && scatter.point_size_map.is_none() {
            return Ok(());
        }
        let pair = |value: f64| {
            let (hi, lo) = crate::data::split_f64_to_f32_pair(value);
            [hi, lo]
        };
        let mut params = data_render::PointValueMapParamsGpu::NEUTRAL;
        let mut colormap = None;
        if scatter.point_color_map.is_some() {
            let bar =
                chart_config
                    .colorbar
                    .as_ref()
                    .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                        series_id: cfg.series_id.clone(),
                        reason: "a colour-mapped scatter needs Config.colorbar: it owns the z \
                                 range and colormap"
                            .into(),
                    })?;
            let colorbar = colorbar_gpu(bar);
            params.z_min = colorbar.z_min;
            params.z_max = colorbar.z_max;
            params.flags |= data_render::VALUE_MAP_COLOR;
            if colorbar.log_z {
                params.flags |= data_render::VALUE_MAP_LOG_Z;
            }
            params.stop_count = u32::try_from(bar.colormap.stops().len()).unwrap_or(u32::MAX);
            params.nan_color_premul = colorbar.nan_color_premul;
            colormap = Some(bar.colormap.clone());
        }
        if let Some(map) = scatter.point_size_map.as_ref() {
            map.validate()
                .map_err(|reason| FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    reason: reason.into(),
                })?;
            params.size_domain_min = pair(map.domain_min);
            params.size_domain_max = pair(map.domain_max);
            params.size_px = [map.min_px * scale, map.max_px * scale];
            params.flags |= data_render::VALUE_MAP_SIZE;
            if matches!(map.scale, SizeScale::Area) {
                params.flags |= data_render::VALUE_MAP_SIZE_AREA;
            }
        }
        let signature = ValueMapSignature { params, colormap };
        if let Some(scratch) = self.value_map_cache.get(cfg.series_id.as_str())
            && scratch.signature == signature
        {
            return Ok(());
        }
        let stops: &[Color] = signature
            .colormap
            .as_ref()
            .map_or(&[], |colormap| colormap.stops());
        let (bind_group, charge) = data_render::create_point_value_map_bind_group(
            &self.device,
            &self.gpu_ledger,
            &self.point_value_map_bgl,
            &signature.params,
            stops,
        );
        evict_if_full(
            &mut self.value_map_cache,
            cfg.series_id.as_str(),
            VALUE_MAP_SCRATCH_CACHE_LIMIT,
        );
        self.value_map_cache
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "point value map scratch cache",
                reason: error.to_string(),
            })?;
        self.value_map_cache.insert(
            cfg.series_id.clone(),
            ValueMapScratch {
                signature,
                bind_group,
                _charge: charge,
            },
        );
        Ok(())
    }

    pub(super) fn prepare_arc_items(
        &mut self,
        items: &[ChartDrawItem<'_>],
    ) -> Result<Vec<PreparedArcItem>> {
        let mut prepared = Vec::with_capacity(items.len());
        for item in items {
            let t = data_render::scatter_transform_from_config(item.chart_config);
            // Same snapshot for both GPU consumers of the transform: the
            // uniform the vertex shaders read and the arc dispatch below.
            // Written here (not via `update_transform`) so they cannot
            // desync within a frame.
            item.view.advance_stream_revision()?;
            let view_revision = item.view.advance_content_revision()?;
            data_render::update_scatter_transform(&self.queue, &item.view.transform_buffer, &t);
            let variant = style_variant(&item.chart_config.draw_style);
            // Milkyway lines also get the arc-driven star pass: the
            // candidate-slot pitch the indirect kernel sizes the dispatch
            // with (the star VS derives the same value from the transform).
            let star_pitch = match &item.chart_config.draw_style {
                crate::config::DrawStyle::Milkyway(c) => Some(
                    (data_render::line_arc::STAR_SLOT_PITCH_FACTOR * c.structure_scale).max(1e-3),
                ),
                _ => None,
            };
            let mut per_series = Vec::with_capacity(item.series.len());
            for series in item.series {
                let cfg = series.config;
                let line =
                    effective_series_primitives(&item.chart_config.draw_style, &cfg.render_type)
                        .line;
                let dashed = line
                    && extract_line(&cfg.render_type)
                        .is_some_and(|l| !matches!(l.line_style, LineStylePreset::Solid));
                let arc_wanted = dashed || (variant.is_some_and(|v| v.needs_arc_prefix) && line);
                let arc = if arc_wanted {
                    self.ensure_arc_prefix(
                        &cfg.series_id,
                        &cfg.x_column,
                        &cfg.y_column,
                        &t,
                        if line { star_pitch } else { None },
                        extract_line(&cfg.render_type)
                            .map_or(LineInterpolation::Linear, |l| l.interpolation),
                    )
                } else {
                    None
                };
                per_series.push(PreparedArcSeries { arc });
            }
            prepared.push(PreparedArcItem {
                view_revision,
                series: per_series,
            });
        }
        Ok(prepared)
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

    /// Convert one panel's `Series` list into `SeriesLayers` ready for
    /// drawing. The `config.render_type` enum variant decides which of
    /// line/scatter/errorbar each series needs; column ids are resolved to
    /// handles via the pool. `prepared` supplies prepare-time arc and star
    /// products without a live `arc_cache` read. `styled` is the panel's
    /// resolved [`StyleSet`] — it selects the stylized pipeline variants
    /// (and the line strip's vertex count); `None` (precise mode, or the
    /// theoretically-impossible cache miss) selects the precise pipelines.
    /// Everything else is identical between the modes.
    pub(super) fn build_series_layers<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series_specs: &[Series<'a>],
        pipelines: &'a TargetPipelines,
        products: PreparedSeriesProducts<'a>,
        styled: Option<&'a StyleSet>,
    ) -> Result<(Vec<data_render::SeriesLayers<'a>>, HashMap<ColumnId, u64>)> {
        let pool = &self.pool;
        let mut column_sources = HashMap::new();
        let mut lookup = |id: &str| -> Result<ColumnHandle> {
            let handle = pool
                .handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
            let allocation_epoch = pool
                .allocation_epoch(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
            column_sources.entry(id.into()).or_insert(allocation_epoch);
            Ok(handle)
        };

        let layers = self.build_series_layers_with_columns(
            view,
            chart_config,
            series_specs,
            pipelines,
            products,
            styled,
            pool.buffer(),
            &mut lookup,
            None,
        )?;
        Ok((layers, column_sources))
    }

    /// A single exact pass; composite series and global-context primitives must
    /// be scheduled separately rather than repeated once per source chunk.
    pub(super) fn validate_stream_series(config: &Config, series: &SeriesConfig) -> Result<()> {
        let unsupported = || FiggyError::InvalidSeriesConfig {
            series_id: series.series_id.clone(),
            reason: "stream pass does not yet support contours or arc-driven stars".into(),
        };
        let primitives = effective_series_primitives(&config.draw_style, &series.render_type);
        if primitives.contour
            || (primitives.field && !streaming_runtime::is_stream_heatmap(&series.render_type))
            || (primitives.line && matches!(config.draw_style, DrawStyle::Milkyway(_)))
        {
            return Err(unsupported());
        }
        // Streaming is not extended to polar charts.
        if config.polar.is_some() {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "polar charts require GPU-resident columns; streamed sources are \
                         not supported"
                    .into(),
            });
        }
        // A fill between two series needs index-aligned chunks of two streams,
        // which nonresident streaming cannot guarantee, and a stepped line's
        // arc prefix is not part of the streamed canonical scan. Both are
        // resident-only by design, rejected here rather than drawn wrong.
        if primitives.line
            && let Some(line) = extract_line(&series.render_type)
            && (line.fill.is_some() || !line.interpolation.is_linear())
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "line fill and step interpolation require GPU-resident columns; \
                         streamed sources are not supported"
                    .into(),
            });
        }
        // Streaming is not extended to vector series: their shader reads the
        // resident pool as storage.
        if has_vector(&series.render_type) {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "vector series require GPU-resident columns; streamed sources are \
                         not supported"
                    .into(),
            });
        }
        // The value maps read index-aligned columns on every draw; streaming
        // is not extended to them.
        if let Some(scatter) = extract_scatter(&series.render_type)
            && (scatter.point_color_map.is_some() || scatter.point_size_map.is_some())
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "point colour and size maps require GPU-resident columns; streamed \
                         sources are not supported"
                    .into(),
            });
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn build_stream_series(
        &self,
        view: &ChartView,
        config: &Config,
        series: &Series<'_>,
        chunk: &crate::streaming_upload::RecordedChunk,
        columns: &[crate::streaming::ColumnRange],
        phase: streaming_runtime::StreamDrawPhase,
    ) -> Result<PreparedSeries> {
        self.build_stream_series_with_pipelines(
            view,
            config,
            series,
            chunk,
            columns,
            phase,
            self.pipelines,
        )
    }

    pub(super) fn build_stream_arc_series(
        &self,
        view: &ChartView,
        config: &Config,
        series: &Series<'_>,
        pool: &wgpu::Buffer,
        x: ColumnHandle,
        y: ColumnHandle,
        arc: ArcPrefix,
        charge: crate::gpu_memory::SharedCharge,
        pipelines: &TargetPipelines,
    ) -> Result<PreparedSeries> {
        Self::validate_stream_series(config, series.config)?;
        let specs = [Series {
            config: series.config,
            style: series.style,
        }];
        let mut layers = self.build_series_layers_with_columns(
            view,
            config,
            &specs,
            pipelines,
            PreparedSeriesProducts {
                arcs: &[],
                contours: &[],
            },
            pipelines.style_set(&config.draw_style),
            pool,
            |id| {
                if id == series.config.x_column {
                    Ok(x)
                } else if id == series.config.y_column {
                    Ok(y)
                } else {
                    Err(FiggyError::UnknownColumn { id: id.into() })
                }
            },
            Some(streaming_runtime::StreamDrawPhase::Line),
        )?;
        let mut packet = PreparedSeries::from_layers(
            layers.remove(0),
            Some(charge),
            Some(Arc::clone(&series.style._charge)),
        );
        if let Some(line) = &mut packet.line {
            line.arc = Some(arc);
        }
        Ok(packet)
    }

    pub(super) fn build_stream_series_with_pipelines(
        &self,
        view: &ChartView,
        config: &Config,
        series: &Series<'_>,
        chunk: &crate::streaming_upload::RecordedChunk,
        columns: &[crate::streaming::ColumnRange],
        phase: streaming_runtime::StreamDrawPhase,
        pipelines: &TargetPipelines,
    ) -> Result<PreparedSeries> {
        Self::validate_stream_series(config, series.config)?;
        if phase == streaming_runtime::StreamDrawPhase::Field {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed field requires the bounded tile executor".into(),
            });
        }
        if phase == streaming_runtime::StreamDrawPhase::Line
            && extract_line(&series.config.render_type).is_some_and(|line| {
                !matches!(line.line_style, LineStylePreset::Solid)
                    || style_variant(&config.draw_style)
                        .is_some_and(|variant| variant.needs_arc_prefix)
            })
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed line requires a prepared canonical arc prefix".into(),
            });
        }
        let expected = streaming_runtime::stream_phase_columns(series.config, phase);
        let first = columns
            .first()
            .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed primitive has no supplied columns".into(),
            })?;
        let bins = if phase == streaming_runtime::StreamDrawPhase::Histogram {
            if streaming_runtime::histogram_edge_column(series.config)
                == Some(series.config.x_column.as_str())
            {
                first.len.saturating_sub(1)
            } else {
                first.len
            }
        } else {
            first.len.saturating_sub(phase.halo())
        };
        if columns.len() != expected.count
            || columns.iter().enumerate().any(|(index, column)| {
                column.column != index as u64
                    || column.offset != first.offset
                    || column.len
                        != if phase == streaming_runtime::StreamDrawPhase::Histogram {
                            bins.saturating_add(u64::from(
                                streaming_runtime::histogram_edge_column(series.config)
                                    == Some(expected.ids[index]),
                            ))
                        } else {
                            bins.saturating_add(phase.halo())
                        }
            })
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed primitive ranges must describe the same global rows".into(),
            });
        }
        let specs = [Series {
            config: series.config,
            style: series.style,
        }];
        let mut layers = self.build_series_layers_with_columns(
            view,
            config,
            &specs,
            pipelines,
            PreparedSeriesProducts {
                arcs: &[],
                contours: &[],
            },
            pipelines.style_set(&config.draw_style),
            &chunk.work,
            |id| {
                let index = expected.ids[..expected.count]
                    .iter()
                    .position(|column| *column == id)
                    .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
                let range = columns[index];
                chunk
                    .column_handle(range)
                    .map_err(|error| FiggyError::InvalidSeriesConfig {
                        series_id: series.config.series_id.clone(),
                        reason: format!("invalid streamed column span: {error:?}"),
                    })
            },
            Some(phase),
        )?;
        let mut prepared = PreparedSeries::from_layers(
            layers.remove(0),
            None,
            Some(Arc::clone(&series.style._charge)),
        );
        prepared._column_charge = Some(chunk.work.shared_charge());
        Ok(prepared)
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

    /// Column locations and their lifetime authority are resolved by the caller;
    /// geometry and style interpretation remain shared with resident rendering.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_series_layers_with_columns<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series_specs: &[Series<'a>],
        pipelines: &'a TargetPipelines,
        products: PreparedSeriesProducts<'a>,
        styled: Option<&'a StyleSet>,
        column_buffer: &'a wgpu::Buffer,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
        phase: Option<streaming_runtime::StreamDrawPhase>,
    ) -> Result<Vec<data_render::SeriesLayers<'a>>> {
        // Resolve the style set into per-primitive picks once. `stars` is the
        // constellation's arc-driven indirect star pass over the same polyline.
        #[derive(Clone, Copy)]
        struct LinePick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            verts: u32,
            texture_bg: Option<&'p wgpu::BindGroup>,
        }
        #[derive(Clone, Copy)]
        struct StarsPick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            texture_bg: &'p wgpu::BindGroup,
        }
        #[derive(Clone, Copy)]
        struct ScatterPick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            texture_bg: Option<&'p wgpu::BindGroup>,
        }
        let (line_pick, stars_pick, scatter_pick, errorbar_pipe) = match styled {
            None => (
                pipelines.line.as_ref().map(|pipeline| LinePick {
                    pipeline,
                    verts: 4,
                    texture_bg: None,
                }),
                None,
                pipelines.scatter.as_ref().map(|pipeline| ScatterPick {
                    pipeline,
                    texture_bg: None,
                }),
                pipelines.errorbar.as_ref(),
            ),
            Some(StyleSet::Sketch {
                line,
                scatter,
                errorbar,
                line_verts,
            }) => (
                Some(LinePick {
                    pipeline: line,
                    verts: *line_verts,
                    texture_bg: None,
                }),
                None,
                Some(ScatterPick {
                    pipeline: scatter,
                    texture_bg: None,
                }),
                Some(errorbar),
            ),
            // Milkyway: line = ribbon + star pass, scatter = ringed
            // planets (premultiplied — they occlude the star field),
            // errorbar = bipolar jets with terminal shock knots.
            Some(StyleSet::Milkyway(c)) => (
                Some(LinePick {
                    pipeline: &c.ribbon,
                    verts: data_render::MILKYWAY_RIBBON_VERTICES,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                Some(StarsPick {
                    pipeline: &c.stars,
                    texture_bg: &c.star_tex_bg,
                }),
                Some(ScatterPick {
                    pipeline: &c.planets,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                Some(&c.jets),
            ),
            // Constellation: only ScatterLine is supported. The line is the
            // regular columnar stroke with style-level alpha; the scatter
            // layer renders PSF stars at the data points.
            Some(StyleSet::Constellation(c)) => (
                Some(LinePick {
                    pipeline: &c.line,
                    verts: 4,
                    texture_bg: None,
                }),
                None,
                Some(ScatterPick {
                    pipeline: &c.stars,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                None,
            ),
        };

        let mut out = Vec::with_capacity(series_specs.len());
        for (idx, series) in series_specs.iter().enumerate() {
            let cfg = series.config;
            let rt = &cfg.render_type;
            let constellation_only = matches!(styled, Some(StyleSet::Constellation(_)));
            let primitives = effective_series_primitives(&chart_config.draw_style, rt);
            let x_h = lookup(&cfg.x_column)?;
            let y_h = lookup(&cfg.y_column)?;

            // A field: the group-2 state was resolved in `ensure_field_scratches`
            // (the prepare phase's `&mut self` half) and is reused whole while
            // nothing it depends on changes.
            let field = if primitives.field {
                let scratch = self
                    .field_cache
                    .get(cfg.series_id.as_str())
                    .expect("prepare ensured this field's scratch");
                Some(data_render::ColumnFieldLayer {
                    pipeline: pipelines
                        .field
                        .as_ref()
                        .expect("prepare ensured field pipeline"),
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    field_bg: scratch.field_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    drawable: scratch.drawable,
                })
            } else {
                None
            };

            // Contour lines: the same quad the field draws, through
            // `fs_contour`. The group-2 tables were resolved in
            // `ensure_contour_scratches`; the isolines come out of the grid
            // itself, so there is nothing per-level about the draw call.
            let contour = if primitives.contour {
                let scratch = products
                    .contours
                    .get(idx)
                    .and_then(|series| series.scratch.as_ref())
                    .expect("prepare captured this contour occurrence");
                let label = scratch
                    .label
                    .as_ref()
                    .zip(pipelines.contour_label.as_ref())
                    .map(|(snapshot, pipeline)| data_render::ColumnContourLabel {
                        pipeline,
                        snapshot: snapshot.clone(),
                    });
                let pipeline = if label.is_some() {
                    pipelines
                        .contour_labelled
                        .as_ref()
                        .expect("prepare ensured labelled contour pipeline")
                } else {
                    pipelines
                        .contour
                        .as_ref()
                        .expect("prepare ensured contour pipeline")
                };
                Some(data_render::ColumnContourLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    field_bg: scratch.field_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    drawable: scratch.drawable,
                    // The label pipeline is compiled only when some series wants
                    // labels, so a `None` here means this series' labels are not
                    // drawable this frame — never that the state is missing.
                    label,
                })
            } else {
                None
            };

            // Arrows: the group-2 state was resolved in `ensure_vector_scratches`
            // and is reused whole while nothing it depends on changes.
            let vector = if primitives.vector && phase.is_none() {
                let scratch = self
                    .vector_cache
                    .get(cfg.series_id.as_str())
                    .expect("prepare ensured this vector's scratch");
                let pipeline = match scratch.input {
                    data_render::vector::VectorInput::Points => pipelines.vector_points.as_ref(),
                    data_render::vector::VectorInput::Grid => pipelines.vector_grid.as_ref(),
                }
                .expect("prepare ensured vector pipeline");
                Some(data_render::vector::ColumnVectorLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    vector_bg: scratch.vector_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    count: scratch.count,
                })
            } else {
                None
            };

            // Bars: the edge column and the value column are picked from the
            // bar's own orientation — never from the column lengths (design
            // §B.3). `Vertical` reads x as edges, `Horizontal` reads y.
            let bar = if primitives.bar
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Histogram)
            {
                let bar_cfg = extract_bar(rt).expect("primitives.bar implies a bar config");
                let (edges, values) = match bar_cfg.orientation {
                    crate::data_config::BarOrientation::Vertical => (x_h, y_h),
                    crate::data_config::BarOrientation::Horizontal => (y_h, x_h),
                };
                let style_map = series.style.bar_map.as_ref();
                Some(data_render::ColumnBarLayer {
                    envelope: phase.is_none().then(|| {
                        pipelines
                            .bar_envelope
                            .as_ref()
                            .expect("prepare ensured histogram envelope pipeline")
                            .prepare(
                                &self.device,
                                &self.queue,
                                &self.gpu_ledger,
                                column_buffer,
                                edges,
                                values,
                                match bar_cfg.orientation {
                                    crate::data_config::BarOrientation::Vertical => {
                                        view.panel_rect.width
                                    }
                                    crate::data_config::BarOrientation::Horizontal => {
                                        view.panel_rect.height
                                    }
                                },
                                &view.transform_bg,
                                &series.style.bar_bg,
                                style_map.map(|map| &map.bind_group),
                            )
                    }),
                    pipeline: style_map.map_or_else(
                        || {
                            pipelines
                                .bar
                                .as_ref()
                                .expect("prepare ensured bar pipeline")
                        },
                        |_| {
                            pipelines
                                .bar_mapped
                                .as_ref()
                                .expect("prepare ensured mapped bar pipeline")
                        },
                    ),
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.bar_bg,
                    style_map_bg: style_map.map(|map| &map.bind_group),
                    pool_buffer: column_buffer,
                    edges,
                    values,
                })
            } else {
                None
            };

            let interpolation =
                extract_line(rt).map_or(LineInterpolation::Linear, |line| line.interpolation);
            let line = if primitives.line
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Line)
            {
                let line_pick = line_pick.expect("prepare ensured line pipeline");
                let arc = products
                    .arcs
                    .get(idx)
                    .and_then(|series| series.arc.as_ref())
                    .map(|arc| arc.prefix.clone());
                Some(ColumnLineLayer {
                    pipeline: line_pick.pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.line_bg,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    arc,
                    // One strip per step piece (or polar arc piece), joined
                    // per `strip_slot`.
                    verts_per_instance: crate::line_step::strip_vertex_count(
                        line_pick.verts,
                        crate::line_step::mode_word(
                            interpolation,
                            crate::line_step::arc_pieces(chart_config.polar.as_ref()),
                        ),
                    ),
                    texture_bg: line_pick.texture_bg,
                })
            } else {
                None
            };

            // The area fill under / beside the line. Resident only: the fill
            // between two series needs index-aligned columns that no streamed
            // chunking can guarantee, and `validate_stream_series` has already
            // rejected a streamed fill before this point. The second y column
            // is the referenced series' own, resolved from the same series
            // list; `validate_line_fill` checked the reference and its x
            // column before any GPU work.
            let area = if line.is_some()
                && phase.is_none()
                && let Some(fill) = extract_line(rt).and_then(|line| line.fill.as_ref())
            {
                let base = match fill {
                    LineFill::ToBaseline(_) => y_h,
                    LineFill::ToSeries(target) => {
                        let target = series_specs
                            .iter()
                            .map(|other| other.config)
                            .find(|other| other.series_id == *target)
                            .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: format!(
                                    "line fill references series `{target}`, which is not in \
                                     this chart"
                                ),
                            })?;
                        if target.x_column != cfg.x_column {
                            return Err(FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: format!(
                                    "line fill to series `{}` requires the same x column: \
                                     `{}` vs `{}`",
                                    target.series_id, cfg.x_column, target.x_column
                                ),
                            });
                        }
                        lookup(&target.y_column)?
                    }
                };
                Some(data_render::ColumnAreaLayer {
                    pipeline: pipelines
                        .area
                        .as_ref()
                        .expect("prepare ensured line area pipeline"),
                    transform_bg: &view.transform_bg,
                    style_bg: series.style.area_bg.as_ref().ok_or_else(|| {
                        FiggyError::InvalidSeriesConfig {
                            series_id: cfg.series_id.clone(),
                            reason: "series style was created without its line fill; \
                                     recreate the style for the current series config"
                                .into(),
                        }
                    })?,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    base,
                    verts_per_instance: crate::line_step::strip_vertex_count(
                        data_render::LINE_PRECISE_VERTICES_PER_INSTANCE,
                        crate::line_step::mode_word(
                            interpolation,
                            crate::line_step::arc_pieces(chart_config.polar.as_ref()),
                        ),
                    ),
                })
            } else {
                None
            };

            // Star pass: per-series GPU state is the owned snapshot the
            // prepare phase cloned out of the arc scratch (indirect args +
            // VS bind group) — read from the token, never from `arc_cache`,
            // so an eviction/rebuild between prepare and paint cannot drop
            // or desync it. A missing star (n < 2, or a
            // theoretically-impossible prepare miss) just skips the stars —
            // the ribbon still draws.
            let line_extra = match (&line, &stars_pick) {
                (Some(_), Some(sp)) => products
                    .arcs
                    .get(idx)
                    .and_then(|series| series.arc.as_ref())
                    .and_then(|a| a.star.as_ref())
                    .map(|star| data_render::ColumnStarLayer {
                        pipeline: sp.pipeline,
                        transform_bg: &view.transform_bg,
                        style_bg: &series.style.line_bg,
                        texture_bg: sp.texture_bg,
                        star_bg: &star.vs_bg,
                        indirect: &star.indirect,
                    }),
                _ => None,
            };

            let scatter = if primitives.scatter
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Scatter)
            {
                let precise_style_map = if styled.is_none() {
                    series.style.scatter_map.as_ref()
                } else {
                    None
                };
                let style_index = match (precise_style_map, extract_scatter(rt)) {
                    (Some(map), Some(sc)) if map.has_index => {
                        let Some(column) = sc.point_style_index_column.as_ref() else {
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
                let (color_value, size_value, value_map_bg) = self.scatter_value_map_columns(
                    cfg,
                    precise_style_map.is_some(),
                    x_h.len_values.min(y_h.len_values),
                    &mut lookup,
                )?;
                let scatter_pick = scatter_pick.expect("prepare ensured scatter pipeline");
                Some(ColumnScatterLayer {
                    pipeline: precise_style_map.map_or(scatter_pick.pipeline, |_| {
                        pipelines
                            .scatter_mapped
                            .as_ref()
                            .expect("prepare ensured mapped scatter pipeline")
                    }),
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.scatter_bg,
                    style_map_bg: precise_style_map.map(|m| &m.bind_group),
                    value_map_bg,
                    quad_vb: &self.quad_vb,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    style_index,
                    color_value,
                    size_value,
                    texture_bg: scatter_pick.texture_bg,
                })
            } else {
                None
            };

            let errorbar = if constellation_only
                || phase.is_some_and(|phase| phase != streaming_runtime::StreamDrawPhase::Errorbar)
            {
                None
            } else {
                match (extract_err_y(rt), extract_err_x(rt)) {
                    (None, None) => None,
                    (ey_opt, ex_opt) => {
                        let (ey_lo, ey_hi) = match ey_opt {
                            Some(ErrorRef::Symmetric { column }) => {
                                let h = lookup(column)?;
                                (h, h)
                            }
                            Some(ErrorRef::Asymmetric { lower, upper }) => {
                                (lookup(lower)?, lookup(upper)?)
                            }
                            // The direction flag disables these attributes
                            // before the shader reads them. Reuse the anchor
                            // column so the bound slice still covers every draw.
                            None => (y_h, y_h),
                        };
                        let (ex_lo, ex_hi) = match ex_opt {
                            Some(ErrorRef::Symmetric { column }) => {
                                let h = lookup(column)?;
                                (h, h)
                            }
                            Some(ErrorRef::Asymmetric { lower, upper }) => {
                                (lookup(lower)?, lookup(upper)?)
                            }
                            None => (x_h, x_h),
                        };
                        let precise_errorbar_style_map = if styled.is_none() {
                            series.style.errorbar_map.as_ref()
                        } else {
                            None
                        };
                        let errorbar_style_index = match (
                            precise_errorbar_style_map,
                            extract_errorbar_style(rt),
                        ) {
                            (Some(map), Some(eb)) if map.has_index => {
                                let Some(column) = eb.error_bar_style_index_column.as_ref() else {
                                    return Err(FiggyError::InvalidSeriesConfig {
                                        series_id: cfg.series_id.clone(),
                                        reason: "errorbar style map expects an index column".into(),
                                    });
                                };
                                let h = lookup(column)?;
                                let count = [
                                    x_h.len_values,
                                    y_h.len_values,
                                    ey_lo.len_values,
                                    ey_hi.len_values,
                                    ex_lo.len_values,
                                    ex_hi.len_values,
                                ]
                                .into_iter()
                                .min()
                                .unwrap_or(0);
                                if h.len_values < count {
                                    return Err(FiggyError::InvalidSeriesConfig {
                                        series_id: cfg.series_id.clone(),
                                        reason: format!(
                                            "errorbar style index column {column:?} has {} values, but errorbar uses {count}",
                                            h.len_values
                                        ),
                                    });
                                }
                                Some(h)
                            }
                            _ => None,
                        };
                        Some(ColumnErrorBarDraw {
                            pipeline: precise_errorbar_style_map.map_or(
                                errorbar_pipe.expect("prepare ensured errorbar pipeline"),
                                |_| {
                                    pipelines
                                        .errorbar_mapped
                                        .as_ref()
                                        .expect("prepare ensured mapped errorbar pipeline")
                                },
                            ),
                            transform_bg: &view.transform_bg,
                            style_bg: &series.style.errorbar_bg,
                            style_map_bg: precise_errorbar_style_map.map(|m| &m.bind_group),
                            pool_buffer: column_buffer,
                            x: x_h,
                            y: y_h,
                            err_y_lo: ey_lo,
                            err_y_hi: ey_hi,
                            err_x_lo: ex_lo,
                            err_x_hi: ex_hi,
                            style_index: errorbar_style_index,
                        })
                    }
                }
            };

            let selection_prepare::SelectionLayers {
                picked,
                selected_bars,
                selected_fields,
            } = if phase.is_some() {
                selection_prepare::SelectionLayers {
                    picked: Vec::new(),
                    selected_bars: Vec::new(),
                    selected_fields: Vec::new(),
                }
            } else {
                self.build_selection_layers(
                    view,
                    chart_config,
                    series,
                    pipelines,
                    selection_prepare::SelectionColumns {
                        buffer: column_buffer,
                        x: x_h,
                        y: y_h,
                        bar: bar.as_ref().map(|layer| (layer.edges, layer.values)),
                        rows: selection_prepare::SelectionRows::RESIDENT,
                        mapped_point_bg: None,
                    },
                    products
                        .contours
                        .get(idx)
                        .and_then(|series| series.scratch.as_ref()),
                    styled.is_none(),
                    &mut lookup,
                )?
            };

            out.push(data_render::SeriesLayers {
                area,
                field,
                bar,
                contour,
                vector,
                errorbar,
                line,
                line_extra,
                scatter,
                selected_bars,
                selected_fields,
                picked,
            });
        }
        Ok(out)
    }

    /// The `(x_base, y_base, n)` source layout for an arc scan over these
    /// columns right now — `None` when a column is missing or shorter than
    /// two points.
    pub(super) fn arc_source_layout(&self, x_id: &str, y_id: &str) -> Option<(u32, u32, u32)> {
        let (x_offset, x_len) = {
            let s = self.pool.slot(x_id)?;
            (s.offset, s.len_values)
        };
        let (y_offset, y_len) = {
            let s = self.pool.slot(y_id)?;
            (s.offset, s.len_values)
        };
        let n = x_len.min(y_len);
        if n < 2 {
            return None;
        }
        let n = u32::try_from(n).ok()?;
        // Pool offsets are 256-aligned bytes → exact f32 element indices.
        // Arc WGSL views the pool as f32 lanes; each logical value occupies
        // two lanes `(hi, lo)`, so `point_px(i)` applies the `i * 2` stride.
        let x_base = u32::try_from(x_offset / 4).ok()?;
        let y_base = u32::try_from(y_offset / 4).ok()?;
        Some((x_base, y_base, n))
    }

    /// Ensure the GPU arc-length prefix for one dashed line series and return
    /// the token-side snapshot: the prefix buffer slice, the owned star-pass
    /// handles (when the scratch has one), and the source-column layout the
    /// scan ran over. The whole computation runs on the GPU
    /// (`line_arc.wgsl` compute scan over the pool columns) — the data never
    /// returns to the CPU, keeping the pool's no-CPU-copy contract intact.
    /// Series longer than one chunk's dispatch capacity scan as sequential
    /// chunks linked by a carry buffer, so any pool-resident `n` is fine.
    ///
    /// A distinct scan input is dispatched once into new immutable buffers.
    /// An exact key hit reuses that result without traversing the columns. An
    /// old result is never rewritten: after `paint_prepared` records a buffer,
    /// the host may drop its [`PreparedFrame`] before submitting the command
    /// buffer, and the late submission must still see the recorded generation.
    ///
    /// `star_pitch`: `Some` adds the arc-driven star pass to the scratch
    /// (indirect args + VS bind group) and runs its kernel after the scan.
    /// Prepare passes `Some` only for Milkyway lines; `DrawStyle::Constellation`
    /// has no arc-prefix star pass.
    pub(super) fn ensure_arc_prefix(
        &mut self,
        series_id: &str,
        x_id: &str,
        y_id: &str,
        t: &data_render::ScatterTransform,
        star_pitch: Option<f32>,
        interpolation: LineInterpolation,
    ) -> Option<PreparedArc> {
        // The mode word: the step mode and, on a polar chart, the arc pieces
        // the transform carries.
        let step_mode = crate::line_step::mode_word_for(interpolation, t);
        if self.arc_pipelines.is_none() {
            *self.arc_pipelines = Some(data_render::line_arc::create_arc_scan_pipelines(
                &self.device,
            ));
        }
        let (x_base, y_base, n) = self.arc_source_layout(x_id, y_id)?;
        let layout_generation = self.pool.layout_generation();
        let x_allocation_epoch = self.pool.allocation_epoch(x_id)?;
        let y_allocation_epoch = self.pool.allocation_epoch(y_id)?;
        let transform_bits: [u32; std::mem::size_of::<data_render::ScatterTransform>() / 4] =
            bytemuck::cast(*t);
        let mut geometry_transform_bits = [0u32; 16];
        geometry_transform_bits.copy_from_slice(&transform_bits[..16]);
        let mut polar_transform_bits = [0u32; 8];
        polar_transform_bits.copy_from_slice(&transform_bits[28..]);
        let key = ArcResultKey {
            n,
            x_base,
            y_base,
            pool_layout_generation: layout_generation,
            x_allocation_epoch,
            y_allocation_epoch,
            geometry_transform_bits,
            polar_transform_bits,
            star_pitch_bits: star_pitch.map(f32::to_bits),
            step_mode,
        };

        // Runaway-churn backstop: ids of long-removed series would otherwise
        // pin GPU memory forever. Exact-key results are immutable; new-key
        // inserts evict only through this bounded cache policy.
        let has_cached_series = self.arc_cache.contains_key(series_id);
        if self.arc_cache.len() > ARC_PREFIX_CACHE_LIMIT
            || (!has_cached_series && self.arc_cache.len() >= ARC_PREFIX_CACHE_LIMIT)
        {
            self.arc_cache.clear();
        }
        let results = self.arc_cache.entry(series_id.to_string()).or_default();
        if let Some(hit) = results.iter().find(|entry| entry.key == key) {
            let star = hit.scratch.star.as_ref().map(|star| PreparedStar {
                indirect: star.indirect.clone(),
                vs_bg: star.vs_bg.clone(),
            });
            return Some(PreparedArc {
                prefix: (Arc::clone(&hit.scratch.arc), u64::from(n) * 4),
                star,
                charge: hit.scratch.charge(),
            });
        }

        #[cfg(test)]
        let chunk_override = self.arc_chunk_override;
        #[cfg(not(test))]
        let chunk_override = None;
        let scratch = data_render::line_arc::ArcScratch::build(
            &self.device,
            &self.gpu_ledger,
            self.arc_pipelines.as_ref().expect("arc pipelines ensured"),
            self.pool.buffer(),
            n,
            x_base,
            y_base,
            self.caps.max_compute_workgroups_per_dimension,
            chunk_override,
            star_pitch.is_some().then_some(&self.star_data_bgl),
            step_mode,
        )?; // None only on a zero-dispatch-limit adapter; try_new rejects those.

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy line arc encoder"),
            });
        scratch.dispatch(
            &self.queue,
            &mut encoder,
            self.arc_pipelines.as_ref().expect("arc pipelines ensured"),
            t,
            star_pitch,
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        // Snapshot after submission. The cache never mutates this scratch;
        // dropping a prepared token therefore cannot make a future prepare
        // overwrite a host command buffer that already recorded these handles.
        let star = scratch.star.as_ref().map(|s| PreparedStar {
            indirect: s.indirect.clone(),
            vs_bg: s.vs_bg.clone(),
        });
        let prepared = PreparedArc {
            prefix: (Arc::clone(&scratch.arc), u64::from(n) * 4),
            star,
            charge: scratch.charge(),
        };
        if results.len() >= ARC_RESULT_CACHE_LIMIT_PER_SERIES {
            results.remove(0);
        }
        results.push(ArcResultEntry { key, scratch });
        Some(prepared)
    }
}

impl Renderer {
    pub(super) fn ensure_value_map_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<()> {
        self.preparation_parts()
            .1
            .ensure_value_map_scratches(items, scale)
    }

    /// Ensure the GPU arc-length prefix for one dashed line series and return
    /// the token-side snapshot: the prefix buffer slice, the owned star-pass
    /// handles (when the scratch has one), and the source-column layout the
    /// scan ran over. The whole computation runs on the GPU
    /// (`line_arc.wgsl` compute scan over the pool columns) — the data never
    /// returns to the CPU, keeping the pool's no-CPU-copy contract intact.
    /// Series longer than one chunk's dispatch capacity scan as sequential
    /// chunks linked by a carry buffer, so any pool-resident `n` is fine.
    ///
    /// A distinct scan input is dispatched once into new immutable buffers.
    /// An exact key hit reuses that result without traversing the columns. An
    /// old result is never rewritten: after `paint_prepared` records a buffer,
    /// the host may drop its [`PreparedFrame`] before submitting the command
    /// buffer, and the late submission must still see the recorded generation.
    ///
    /// `star_pitch`: `Some` adds the arc-driven star pass to the scratch
    /// (indirect args + VS bind group) and runs its kernel after the scan.
    /// Prepare passes `Some` only for Milkyway lines; `DrawStyle::Constellation`
    /// has no arc-prefix star pass.
    #[cfg(test)]
    pub(super) fn ensure_arc_prefix(
        &mut self,
        series_id: &str,
        x_id: &str,
        y_id: &str,
        t: &data_render::ScatterTransform,
        star_pitch: Option<f32>,
        interpolation: LineInterpolation,
    ) -> Option<PreparedArc> {
        self.preparation_parts().1.ensure_arc_prefix(
            series_id,
            x_id,
            y_id,
            t,
            star_pitch,
            interpolation,
        )
    }
}

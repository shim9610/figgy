//! Series preparation for a frame: PrepareContext, the per-kind scratch
//! caches it fills (field, value map, vector, contour and arc prefix) and
//! their signatures. #46 splits this per data kind into data_render/.

mod field;
use super::*;
pub(in crate::renderer) use field::*;

pub(super) fn contour_levels(rt: &DataRenderType) -> Result<Vec<f32>> {
    let empty: [f64; 0] = [];
    let source = extract_contour(rt).map_or(&empty[..], |contour| &contour.levels);
    let mut levels =
        crate::gpu_contour::try_collect("contour levels", source.iter().map(|l| *l as f32))?;
    if levels.is_empty() {
        levels.push(0.0);
    }
    Ok(levels)
}

/// How many levels the declaration actually names — the padding
/// `contour_levels` adds is not one of them.
pub(super) fn declared_level_count(rt: &DataRenderType) -> usize {
    extract_contour(rt).map_or(0, |contour| contour.levels.len())
}

pub(super) fn validate_contour_level_count(series_id: &str, rt: &DataRenderType) -> Result<()> {
    let count = declared_level_count(rt);
    if count > crate::data_config::MAX_CONTOUR_LEVELS {
        return Err(FiggyError::InvalidSeriesConfig {
            series_id: series_id.to_owned(),
            reason: format!(
                "contour level count {count} exceeds the supported maximum {}",
                crate::data_config::MAX_CONTOUR_LEVELS
            ),
        });
    }
    Ok(())
}

pub(super) fn validate_contour_label_config(series_id: &str, rt: &DataRenderType) -> Result<()> {
    let Some(labels) = extract_contour(rt).and_then(|contour| contour.labels.as_ref()) else {
        return Ok(());
    };
    labels
        .validate()
        .map_err(|reason| FiggyError::InvalidSeriesConfig {
            series_id: series_id.to_owned(),
            reason: reason.to_owned(),
        })
}

pub(super) fn contour_labels_use_automatic(rt: &DataRenderType) -> bool {
    let Some(contour) = extract_contour(rt) else {
        return false;
    };
    let Some(labels) = contour.labels.as_ref() else {
        return false;
    };
    labels.visible
        && labels.font_size > 0.0
        && !contour.levels.is_empty()
        && !labels
            .anchors
            .iter()
            .any(|anchor| (anchor.level_index as usize) < contour.levels.len())
}

/// Validate the frame-dependent spacing product and return the exact value the
/// automatic dispatch must consume. Explicit placement never multiplies it.
pub(super) fn preflight_contour_label_spacing(
    series_id: &str,
    rt: &DataRenderType,
    contour_enabled: bool,
    scale: f32,
) -> Result<Option<f32>> {
    validate_contour_label_config(series_id, rt)?;
    if !contour_enabled || !contour_labels_use_automatic(rt) {
        return Ok(None);
    }
    let labels = extract_contour(rt)
        .and_then(|contour| contour.labels.as_ref())
        .expect("automatic contour labels have a label config");
    let spacing_px = labels.spacing_px * scale;
    if !spacing_px.is_finite() || spacing_px <= 0.0 {
        return Err(FiggyError::InvalidSeriesConfig {
            series_id: series_id.to_owned(),
            reason:
                "automatic contour label spacing_px * scale must be finite and greater than zero"
                    .to_owned(),
        });
    }
    Ok(Some(spacing_px))
}

/// Whether a series declares contour labels that would actually be drawn.
///
/// Hidden labels and a non-positive font size are the same answer as no label
/// declaration at all — nothing to bake, so nothing to compile a pipeline for.
pub(super) fn series_wants_contour_labels(rt: &DataRenderType) -> bool {
    extract_contour(rt)
        .and_then(|contour| contour.labels.as_ref())
        .is_some_and(|labels| labels.visible && labels.font_size > 0.0)
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

/// Per-series contour scratch cache limit, mirroring `ARC_PREFIX_CACHE_LIMIT`.
pub(super) const CONTOUR_SCRATCH_CACHE_LIMIT: usize = 256;

/// What a cached contour's GPU state was resolved from.
///
/// Equality means the same isolines would be drawn: the levels, their colours,
/// the grid, the coordinate columns and the pool layout are all in it. Shaped
/// like [`FieldSignature`] because since design B.4.9 a contour *is* a field
/// draw — the same quad and the same group-2 tables, through `fs_contour`.
///
/// The colourmap stops are absent on purpose: `fs_contour` takes its colour from
/// `level_colors` and never samples the ramp, so the stop table is padding for
/// the layout and cannot change what is drawn.
#[derive(Debug, PartialEq)]
pub(super) struct ContourSignature {
    pub(super) pool_layout_generation: u64,
    pub(super) params: data_render::FieldParamsGpu,
    pub(super) grid: Vec<data_render::GridColumnGpu>,
    pub(super) epochs: Vec<u64>,
    pub(super) levels: Vec<f32>,
    /// Per-level stroke colour, premultiplied — as drawn, so comparing
    /// signatures compares pixels.
    pub(super) level_colors: Vec<[f32; 4]>,
    /// `None` when the series draws no labels.
    pub(super) label: Option<ContourLabelSignature>,
}

/// What a contour series' baked label atlas and anchor buffer were resolved
/// from — the part of it not already in [`ContourSignature`].
///
/// The level GPU values and stroke colours are in the outer signature already,
/// so this holds only what the atlas adds: the final strings, the independent
/// label colour, the face, the background, and any host anchor override.
/// `spacing_px` is
/// deliberately absent — it is a screen distance the anchor pass reads every
/// frame, not something an atlas is baked for.
#[derive(Debug, PartialEq)]
pub(super) struct ContourLabelSignature {
    /// Final formatted strings are the atlas' exact content authority. This
    /// captures f64 level distinctions and colourbar/level-spacing changes
    /// without accidentally keying the atlas to an unrelated chart axis.
    pub(super) texts: Vec<String>,
    /// `font_size * scale` — an export at 2x re-bakes rather than magnifying.
    pub(super) font_bits: u32,
    pub(super) color: [u32; 4],
    pub(super) policy: crate::text_render::FontPolicy,
    pub(super) family: String,
    pub(super) bg: Option<[u32; 4]>,
    pub(super) bg_padding_bits: u32,
    /// Runtime font registration invalidates a previously baked atlas even
    /// when every chart-owned formatting field is unchanged.
    pub(super) font_generation: u64,
    /// The resolved host override. Empty means the GPU picks the anchors.
    pub(super) anchors: Vec<crate::gpu_contour::LabelAnchorGpu>,
}

/// One contour series' resolved GPU state — the same shape a field's is.
pub(super) struct ContourScratchEntry {
    pub(super) signature: ContourSignature,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) field_bg: wgpu::BindGroup,
    pub(super) charge: crate::gpu_memory::SharedCharge,
    pub(super) sources: Arc<GridSources>,
    /// False when the grid has no cell to interpolate a contour across.
    pub(super) drawable: bool,
    /// `None` when the series draws no labels — hidden, no label declaration, or
    /// every formatted string empty. Atlas/device-limit failures are errors.
    pub(super) label: Option<crate::gpu_contour::ContourLabelState>,
}

/// Per-series vector scratch cache limit, mirroring `FIELD_SCRATCH_CACHE_LIMIT`.
pub(super) const VECTOR_SCRATCH_CACHE_LIMIT: usize = 256;

/// Everything a vector series' group-2 state depends on. Compared before any
/// upload, so a steady frame builds nothing.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct VectorSignature {
    pub(super) pool_layout_generation: u64,
    pub(super) params: data_render::vector::VectorParamsGpu,
    pub(super) grid: Vec<data_render::GridColumnGpu>,
    pub(super) epochs: Vec<u64>,
    pub(super) stops: Vec<[f32; 4]>,
    /// Series colour bits: the group-1 style is built here too.
    pub(super) color: [u32; 4],
}

/// One vector series' resolved GPU state.
pub(super) struct VectorScratch {
    pub(super) signature: VectorSignature,
    pub(super) input: data_render::vector::VectorInput,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) vector_bg: wgpu::BindGroup,
    pub(super) charge: crate::gpu_memory::SharedCharge,
    /// Grid form only: the `u` grid as a field's group 2, so the selected
    /// cell highlight runs the field pipeline unchanged. The grid's arrow
    /// pick binds `vector_bg` (`pick_vector_cell`).
    pub(super) field: Option<(wgpu::BindGroup, crate::gpu_memory::SharedCharge)>,
    pub(super) sources: Arc<GridSources>,
    /// Arrows the draw issues; zero draws nothing.
    pub(super) count: u32,
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

    /// Prepare every vector series' group-2 state, returning the column
    /// receipts for the token the way `ensure_field_scratches` does.
    pub(super) fn ensure_vector_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<Vec<Arc<GridSources>>> {
        let mut receipts = Vec::new();
        for item in items {
            for series in item.series {
                let cfg = series.config;
                if !effective_series_primitives(&item.chart_config.draw_style, &cfg.render_type)
                    .vector
                {
                    continue;
                }
                let Some(sources) = self.ensure_vector_scratch(item.chart_config, cfg, scale)?
                else {
                    continue;
                };
                receipts
                    .try_reserve(1)
                    .map_err(|error| FiggyError::StateAllocationFailed {
                        resource: "vector source receipts",
                        reason: error.to_string(),
                    })?;
                receipts.push(sources);
            }
        }
        Ok(receipts)
    }

    /// Build or reuse one vector series' group-2 bind group (and, for the grid
    /// form, the field bind group its cell highlight uses).
    ///
    /// Same contract as `ensure_field_scratch`: the signature is compared
    /// before anything is created, so a steady frame uploads nothing, and a
    /// live token holds its own refcounted clones. `scale` is the export DPI
    /// multiplier applied to every pixel size, like `point_radius_px`.
    pub(super) fn ensure_vector_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<Option<Arc<GridSources>>> {
        use crate::data_config::{VectorColorMap, VectorComponents, VectorPivot, VectorScale};
        use data_render::vector as vector_render;
        let Some((components, style)) = extract_vector(&cfg.render_type) else {
            return Ok(None);
        };
        let invalid = |reason: String| FiggyError::InvalidSeriesConfig {
            series_id: cfg.series_id.clone(),
            reason,
        };
        style.validate().map_err(|reason| invalid(reason.into()))?;
        let alloc = |resource: &'static str| {
            move |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource,
                reason: error.to_string(),
            }
        };
        let handle = |id: &str| {
            self.pool
                .handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })
        };
        let epoch = |id: &str| {
            self.pool
                .allocation_epoch(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })
        };
        let lane = |handle: &ColumnHandle| {
            u32::try_from(handle.offset / 4)
                .map_err(|_| invalid("column offset exceeds a 32-bit lane index".into()))
        };
        let len32 = |handle: &ColumnHandle| u32::try_from(handle.len_values).unwrap_or(u32::MAX);

        let x = handle(&cfg.x_column)?;
        let y = handle(&cfg.y_column)?;
        let mut params = vector_render::VectorParamsGpu {
            x_base: lane(&x)?,
            y_base: lane(&y)?,
            x_len: len32(&x),
            y_len: len32(&y),
            u_base: 0,
            u_len: 0,
            v_base: 0,
            v_len: 0,
            c_base: 0,
            c_len: 0,
            cols: 0,
            rows: 0,
            flags: 0,
            count: 0,
            stop_count: 0,
            pivot: match style.pivot {
                VectorPivot::Tail => vector_render::VECTOR_PIVOT_TAIL,
                VectorPivot::Middle => vector_render::VECTOR_PIVOT_MIDDLE,
                VectorPivot::Tip => vector_render::VECTOR_PIVOT_TIP,
            },
            scale: 0.0,
            width_px: style.width_px * scale,
            head_length_px: style.head_length_px * scale,
            head_width_px: style.head_width_px * scale,
            min_length_px: style.min_length_px * scale,
            aa_px: scale,
            pad0: 0.0,
            pad1: 0.0,
            z_min: [0.0; 2],
            z_max: [0.0; 2],
            nan_color_premul: [0.0; 4],
        };
        match style.scale {
            VectorScale::Data { factor } => params.scale = factor as f32,
            VectorScale::Pixels { px_per_unit } => {
                params.scale = px_per_unit as f32 * scale;
                params.flags |= vector_render::VECTOR_FLAG_SCALE_PIXELS;
            }
        }

        let mut grid = Vec::new();
        let mut ids = Vec::new();
        let mut epochs = Vec::new();
        // Grid form: the `u` grid's field params and its table length, for
        // the field bind group the selection reuses.
        let mut field_params = None;
        let input = match components {
            VectorComponents::Points { u, v } => {
                let u_h = handle(u)?;
                let v_h = handle(v)?;
                params.u_base = lane(&u_h)?;
                params.u_len = len32(&u_h);
                params.v_base = lane(&v_h)?;
                params.v_len = len32(&v_h);
                let mut count = x
                    .len_values
                    .min(y.len_values)
                    .min(u_h.len_values)
                    .min(v_h.len_values);
                ids.try_reserve_exact(3)
                    .map_err(alloc("vector source ids"))?;
                epochs
                    .try_reserve_exact(3)
                    .map_err(alloc("vector source epochs"))?;
                ids.push(u.clone());
                epochs.push(epoch(u)?);
                ids.push(v.clone());
                epochs.push(epoch(v)?);
                if let Some(VectorColorMap::Column(column)) = style.color_map.as_ref() {
                    let c_h = handle(column)?;
                    params.c_base = lane(&c_h)?;
                    params.c_len = len32(&c_h);
                    count = count.min(c_h.len_values);
                    ids.push(column.clone());
                    epochs.push(epoch(column)?);
                }
                params.count = u32::try_from(count).unwrap_or(u32::MAX);
                // A storage binding cannot be zero-sized: one padding entry.
                grid.push(data_render::GridColumnGpu { base: 0, len: 0 });
                vector_render::VectorInput::Points
            }
            VectorComponents::Grid { u, v } => {
                let mut extent: Option<(usize, usize)> = None;
                let mut append = |matrix: &crate::data_config::MatrixRef,
                                  grid: &mut Vec<data_render::GridColumnGpu>,
                                  ids: &mut Vec<ColumnId>,
                                  epochs: &mut Vec<u64>|
                 -> Result<(u32, u32, ResolvedGrid)> {
                    let resolved = self.resolve_grid(cfg, matrix)?;
                    let base = u32::try_from(grid.len()).unwrap_or(u32::MAX);
                    let entries = u32::try_from(resolved.grid.len()).unwrap_or(u32::MAX);
                    grid.try_reserve(resolved.grid.len())
                        .map_err(alloc("vector grid table"))?;
                    grid.extend_from_slice(&resolved.grid);
                    ids.try_reserve(matrix.columns.len())
                        .map_err(alloc("vector source ids"))?;
                    ids.extend(matrix.columns.iter().cloned());
                    epochs
                        .try_reserve(resolved.epochs.len())
                        .map_err(alloc("vector source epochs"))?;
                    epochs.extend_from_slice(&resolved.epochs);
                    let cells = (resolved.extent.cols, resolved.extent.rows);
                    extent = Some(match extent {
                        None => cells,
                        Some((cols, rows)) => (cols.min(cells.0), rows.min(cells.1)),
                    });
                    Ok((base, entries, resolved))
                };
                let (u_base, u_len, u_grid) = append(u, &mut grid, &mut ids, &mut epochs)?;
                params.u_base = u_base;
                params.u_len = u_len;
                let (v_base, v_len, _) = append(v, &mut grid, &mut ids, &mut epochs)?;
                params.v_base = v_base;
                params.v_len = v_len;
                if let Some(VectorColorMap::Grid(matrix)) = style.color_map.as_ref() {
                    let (c_base, c_len, _) = append(matrix, &mut grid, &mut ids, &mut epochs)?;
                    params.c_base = c_base;
                    params.c_len = c_len;
                }
                let (cols, rows) = extent.unwrap_or((0, 0));
                params.cols = u32::try_from(cols).unwrap_or(u32::MAX);
                params.rows = u32::try_from(rows).unwrap_or(u32::MAX);
                params.count = cols
                    .checked_mul(rows)
                    .and_then(|count| u32::try_from(count).ok())
                    .ok_or_else(|| {
                        invalid("vector grid cell count exceeds the u32 instance space".into())
                    })?;
                params.flags |= vector_render::VECTOR_FLAG_GRID;
                let mut field_flags = 0u32;
                if u_grid.columns_are_y {
                    params.flags |= vector_render::VECTOR_FLAG_COLUMNS_ARE_Y;
                    field_flags |= data_render::FIELD_FLAG_COLUMNS_ARE_Y;
                }
                if u_grid.centers {
                    params.flags |= vector_render::VECTOR_FLAG_CENTERS;
                    field_flags |= data_render::FIELD_FLAG_CENTERS;
                }
                // The `u` grid's lattice as a field, for the selected-cell
                // outline; that pass reads only the lattice.
                field_params = Some(data_render::FieldParamsGpu::cell_geometry(
                    params.x_base,
                    params.y_base,
                    params.x_len,
                    params.y_len,
                    params.cols,
                    params.rows,
                    field_flags,
                ));
                vector_render::VectorInput::Grid
            }
        };

        let mut stops = Vec::new();
        if style.color_map.is_some() {
            let bar = chart_config.colorbar.as_ref().ok_or_else(|| {
                invalid(
                    "a colour-mapped vector needs Config.colorbar: it owns the z range and \
                     colormap"
                        .into(),
                )
            })?;
            let colorbar = colorbar_gpu(bar);
            params.z_min = colorbar.z_min;
            params.z_max = colorbar.z_max;
            params.flags |= vector_render::VECTOR_FLAG_COLOR_MAP;
            if colorbar.log_z {
                params.flags |= vector_render::VECTOR_FLAG_LOG_Z;
            }
            params.nan_color_premul = colorbar.nan_color_premul;
            let table = colorbar_stops("vector colourmap stops", bar)?;
            stops = table.stops;
            params.stop_count = table.stop_count;
        }
        if stops.is_empty() {
            // No colour map: one padding entry keeps the binding non-empty.
            stops.push([0.0; 4]);
        }

        let signature = VectorSignature {
            pool_layout_generation: self.pool.layout_generation(),
            params,
            grid,
            epochs,
            stops,
            color: [
                style.color.r.to_bits(),
                style.color.g.to_bits(),
                style.color.b.to_bits(),
                style.color.a.to_bits(),
            ],
        };
        if let Some(scratch) = self.vector_cache.get(cfg.series_id.as_str())
            && scratch.signature == signature
        {
            return Ok(Some(Arc::clone(&scratch.sources)));
        }

        let sources = Arc::new(GridSources {
            ids,
            epochs: signature.epochs.clone(),
        });
        let style_buf = data_render::create_style_uniform_buffer(
            &self.device,
            &PrimitiveStyle::from_color_with_width(style.color, style.width_px * scale),
        );
        let style_bg =
            data_render::create_style_bind_group(&self.device, &self.style_bgl, &style_buf);
        let vector_render::VectorDataBindGroup {
            bind_group: vector_bg,
            grid: grid_buffer,
            charge,
        } = vector_render::create_vector_data_bind_group(
            &self.device,
            &self.gpu_ledger,
            &self.vector_bgl,
            self.pool.buffer(),
            vector_render::VectorTables {
                grid: &signature.grid,
                stops: &signature.stops,
                params: &signature.params,
            },
        );
        // The grid table starts with the `u` grid's columns, which is all the
        // field lattice indexes; the same buffer serves both groups.
        let field = field_params.map(|field_params| {
            data_render::create_field_cell_bind_group(
                &self.device,
                &self.gpu_ledger,
                &self.field_bgl,
                self.pool.buffer(),
                &grid_buffer,
                self.field_cell_tables,
                &field_params,
            )
        });

        evict_if_full(
            &mut self.vector_cache,
            cfg.series_id.as_str(),
            VECTOR_SCRATCH_CACHE_LIMIT,
        );
        self.vector_cache
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "vector scratch cache",
                reason: error.to_string(),
            })?;
        let receipt = Arc::clone(&sources);
        let count = signature.params.count;
        self.vector_cache.insert(
            cfg.series_id.clone(),
            VectorScratch {
                signature,
                input,
                style_bg,
                vector_bg,
                charge,
                field,
                sources,
                count,
            },
        );
        Ok(Some(receipt))
    }

    /// Prepare every contour series' reusable GPU state.
    ///
    /// Returns the grid receipts for the token, the same way
    /// `ensure_field_scratches` does. Automatic label anchors are the only
    /// contour compute dispatch and remain frame-dependent.
    /// `scale` is the export DPI multiplier — the same one
    /// `create_style_for_series_scaled` applies to every other pixel dimension.
    /// It rides in the signature, so switching between a window frame and an
    /// export rebuilds the level styles rather than drawing one at the other's
    /// stroke width.
    pub(super) fn ensure_contour_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
        contour_spacings: &[Vec<Option<f32>>],
    ) -> Result<(Vec<PreparedContourItem>, Vec<Arc<GridSources>>)> {
        let mut prepared = Vec::with_capacity(items.len());
        let mut receipts = Vec::new();
        debug_assert_eq!(items.len(), contour_spacings.len());
        for (item, item_spacings) in items.iter().zip(contour_spacings) {
            debug_assert_eq!(item.series.len(), item_spacings.len());
            let transform = data_render::scatter_transform_from_config(item.chart_config);
            let ca = item.chart_config.chart_area.0;
            let da = item
                .chart_config
                .data_area()
                .map(|area| area.0)
                .unwrap_or(ca);
            let mut encoder = None;
            let mut per_series = Vec::with_capacity(item.series.len());
            for (series, effective_spacing_px) in item.series.iter().zip(item_spacings) {
                let cfg = series.config;
                if !effective_series_primitives(&item.chart_config.draw_style, &cfg.render_type)
                    .contour
                {
                    per_series.push(PreparedContourSeries { scratch: None });
                    continue;
                }
                let sources = self.ensure_contour_scratch(cfg, item.chart_config, scale)?;
                receipts
                    .try_reserve(1)
                    .map_err(|error| FiggyError::StateAllocationFailed {
                        resource: "contour source receipts",
                        reason: error.to_string(),
                    })?;
                receipts.push(sources);

                let automatic = self
                    .contour_cache
                    .get(cfg.series_id.as_str())
                    .and_then(|entry| entry.label.as_ref())
                    .is_some_and(crate::gpu_contour::ContourLabelState::is_automatic);
                if automatic && encoder.is_none() {
                    encoder = Some(self.device.create_command_encoder(
                        &wgpu::CommandEncoderDescriptor {
                            label: Some("figgy contour label encoder"),
                        },
                    ));
                }

                let label = self.contour_label_snapshot(
                    cfg.series_id.as_str(),
                    &mut encoder,
                    &transform,
                    (ca, da),
                    *effective_spacing_px,
                );
                let entry = self
                    .contour_cache
                    .get(cfg.series_id.as_str())
                    .expect("contour scratch was just ensured");
                per_series.push(PreparedContourSeries {
                    scratch: Some(PreparedContourScratch {
                        style_bg: entry.style_bg.clone(),
                        field_bg: entry.field_bg.clone(),
                        charge: Arc::clone(&entry.charge),
                        drawable: entry.drawable,
                        label,
                    }),
                });
            }
            if let Some(encoder) = encoder {
                self.queue.submit(std::iter::once(encoder.finish()));
            }
            prepared.push(PreparedContourItem { series: per_series });
        }
        Ok((prepared, receipts))
    }

    /// The label snapshot a contour series draws with at `transform`: the
    /// explicit placement, or the automatic one for this exact transform and
    /// anchor plan (dispatched into `encoder` on a miss, reused on a hit).
    /// `None` without labels. Shared by the draw and the contour pick, which
    /// leaves the same label gaps unpicked.
    pub(super) fn contour_label_snapshot(
        &mut self,
        series_id: &str,
        encoder: &mut Option<wgpu::CommandEncoder>,
        transform: &data_render::ScatterTransform,
        (ca, da): (Rect, Rect),
        effective_spacing_px: Option<f32>,
    ) -> Option<crate::gpu_contour::ContourLabelSnapshot> {
        let entry = self.contour_cache.get_mut(series_id)?;
        let label = entry.label.as_mut()?;
        if !label.is_automatic() {
            return label.explicit_snapshot();
        }
        let effective_spacing_px =
            effective_spacing_px.expect("automatic contour spacing was preflighted");
        let params = &entry.signature.params;
        let plan = crate::gpu_contour::ContourLabelState::anchor_plan(
            da.width as f32,
            da.height as f32,
            effective_spacing_px,
            params.level_count,
        );
        let encoder = encoder.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy contour label encoder"),
                })
        });
        Some(
            label.dispatch_automatic(
                &self.device,
                &self.queue,
                &self.gpu_ledger,
                encoder,
                self.contour_label_pipelines
                    .as_ref()
                    .expect("contour label pipelines ensured"),
                transform,
                crate::gpu_contour::AnchorParamsGpu {
                    lattice_x: plan.lattice_x,
                    lattice_y: plan.lattice_y,
                    level_count: plan.levels,
                    kept_capacity: crate::gpu_contour::MAX_CONTOUR_LABELS_TOTAL,
                    area_x: ca.x as f32,
                    area_y: ca.y as f32,
                    area_w: ca.width as f32,
                    area_h: ca.height as f32,
                    clip_x: da.x as f32,
                    clip_y: da.y as f32,
                    clip_w: da.width as f32,
                    clip_h: da.height as f32,
                    spacing_px: effective_spacing_px,
                    label_h_px: label.cell_h_px(),
                    label_vertices: crate::gpu_contour::LABEL_VERTICES,
                    _pad0: 0,
                },
                &entry.field_bg,
            ),
        )
    }

    /// The label snapshot a contour pick of `cfg` sees: the one the draw at
    /// `chart_config` (the pick's display configuration) and `scale` paints
    /// its label gaps from, through [`Self::contour_label_snapshot`]. Any
    /// anchor dispatch is submitted before the pick that reads it.
    pub(super) fn contour_pick_label(
        &mut self,
        cfg: &SeriesConfig,
        chart_config: &Config,
        scale: f32,
    ) -> Result<Option<crate::gpu_contour::ContourLabelSnapshot>> {
        if !series_wants_contour_labels(&cfg.render_type) {
            return Ok(None);
        }
        let spacing =
            preflight_contour_label_spacing(cfg.series_id.as_str(), &cfg.render_type, true, scale)?;
        if self.contour_label_pipelines.is_none() {
            *self.contour_label_pipelines = Some(crate::gpu_contour::ContourLabelPipelines::new(
                &self.device,
                &self.field_bgl,
            ));
        }
        let transform = data_render::scatter_transform_from_config(chart_config);
        let ca = chart_config.chart_area.0;
        let da = chart_config.data_area().map(|area| area.0).unwrap_or(ca);
        let mut encoder = None;
        let label = self.contour_label_snapshot(
            cfg.series_id.as_str(),
            &mut encoder,
            &transform,
            (ca, da),
            spacing,
        );
        if let Some(encoder) = encoder {
            self.queue.submit(std::iter::once(encoder.finish()));
        }
        Ok(label)
    }

    /// Build or reuse one contour series' GPU state.
    ///
    /// Since design B.4.9 this resolves the same group-2 tables a field does: the
    /// isolines are drawn from the grid itself by `fs_contour`, so there is no
    /// trace to dispatch and nothing here depends on the panel transform. The
    /// label anchors do — `ensure_contour_scratches` captures one placement
    /// snapshot per panel/item + series occurrence.
    ///
    /// `scale` is the export DPI multiplier, applied to the stroke width and the
    /// label font exactly as `create_style_for_series_scaled` applies it
    /// everywhere else.
    pub(super) fn ensure_contour_scratch(
        &mut self,
        cfg: &SeriesConfig,
        chart_config: &Config,
        scale: f32,
    ) -> Result<Arc<GridSources>> {
        let rt = &cfg.render_type;
        let matrix = extract_matrix(rt).expect("a contour series has a matrix");
        let contour = extract_contour(rt).expect("a contour series has a contour config");
        validate_contour_level_count(cfg.series_id.as_str(), rt)?;
        let resolved = self.resolve_grid(cfg, matrix)?;
        let levels = contour_levels(rt)?;
        let level_count = u32::try_from(declared_level_count(rt)).unwrap_or(u32::MAX);

        let mut flags = 0u32;
        if resolved.columns_are_y {
            flags |= data_render::FIELD_FLAG_COLUMNS_ARE_Y;
        }
        if resolved.centers {
            flags |= data_render::FIELD_FLAG_CENTERS;
        }
        // The contour's lattice is the sample-point lattice — that is where z
        // lives, and an isoline does not move because a fill decided to draw flat
        // cells. `fs_contour` asks for `LATTICE_SAMPLES` explicitly, so this flag
        // is redundant for it; set anyway so this bind group's params describe the
        // lattice its shader actually walks.
        flags |= data_render::FIELD_FLAG_INTERPOLATED;

        // Per-level stroke: `per_level_color` when given, the line's own colour
        // otherwise. Colours are not derived from the colormap here — a helper
        // that wants that writes them into the config (design B.3).
        //
        // Premultiplied, because that is what the shader composites and what the
        // blend state expects; storing it as drawn also makes a signature
        // comparison a comparison of pixels.
        let level_colors: Vec<[f32; 4]> = crate::gpu_contour::try_collect(
            "contour level colours",
            (0..level_count as usize).map(|index| {
                let color = contour
                    .per_level_color
                    .as_ref()
                    .and_then(|colors| colors.get(index))
                    .copied()
                    .unwrap_or(contour.line.line_color);
                let a = color.a.clamp(0.0, 1.0);
                [color.r * a, color.g * a, color.b * a, a]
            }),
        )?;

        let params = data_render::FieldParamsGpu {
            x_base: u32::try_from(resolved.x.offset / 4).unwrap_or(0),
            y_base: u32::try_from(resolved.y.offset / 4).unwrap_or(0),
            x_len: u32::try_from(resolved.x.len_values).unwrap_or(u32::MAX),
            y_len: u32::try_from(resolved.y.len_values).unwrap_or(u32::MAX),
            cols: u32::try_from(resolved.extent.cols).unwrap_or(u32::MAX),
            rows: u32::try_from(resolved.extent.rows).unwrap_or(u32::MAX),
            level_count,
            // One padding stop: `fs_contour` never samples the ramp, and a
            // zero-sized storage binding is invalid.
            stop_count: 1,
            flags,
            // Fill-only; the contour pass never reads it.
            opacity: 1.0,
            line_width_px: contour.line.line_width * scale,
            level_color_count: u32::try_from(level_colors.len()).unwrap_or(u32::MAX),
            // Colourbar bounds are the *fill*'s business. Levels are in data
            // units (design B.5), so the lines need no z range at all.
            z_min: [0.0, 0.0],
            z_max: [0.0, 0.0],
        };

        // What the labels are baked from. `spacing_px` is not in here: it is a
        // screen distance the anchor pass reads every frame, so changing it moves
        // the labels without rebaking a single glyph.
        let mut label_sig = None;
        if let Some(labels) = contour.labels.as_ref()
            && labels.visible
            && labels.font_size > 0.0
            && level_count > 0
        {
            let mut anchors = crate::gpu_contour::try_collect(
                "contour label anchor overrides",
                labels.anchors.iter().map(|anchor| {
                    let (x_hi, x_lo) = crate::data::split_f64_to_f32_pair(anchor.x);
                    let (y_hi, y_lo) = crate::data::split_f64_to_f32_pair(anchor.y);
                    crate::gpu_contour::LabelAnchorGpu {
                        x: [x_hi, x_lo],
                        y: [y_hi, y_lo],
                        // A direction, so it needs no pair split.
                        dir: [anchor.tx as f32, anchor.ty as f32],
                        level: anchor.level_index,
                        // Resolved from the baked atlas immediately before the
                        // explicit anchors are uploaded.
                        width_px: 0.0,
                    }
                }),
            )?;
            // An anchor naming a level the declaration no longer has is stale,
            // not fatal: it draws nothing and the rest still do.
            anchors.retain(|anchor| anchor.level < level_count);
            anchors.truncate(crate::gpu_contour::MAX_CONTOUR_LABELS_TOTAL as usize);
            label_sig = Some(ContourLabelSignature {
                texts: axis_render::contour_label_texts(chart_config, labels, &contour.levels),
                font_bits: (labels.font_size * scale).to_bits(),
                color: [
                    labels.color.r.to_bits(),
                    labels.color.g.to_bits(),
                    labels.color.b.to_bits(),
                    labels.color.a.to_bits(),
                ],
                policy: crate::text_render::FontPolicy::for_style(&chart_config.draw_style),
                family: chart_config.bottom_x.label_style.label_font.clone(),
                bg: labels
                    .bg_color
                    .as_ref()
                    .map(|c| [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()]),
                bg_padding_bits: labels.bg_padding_px.to_bits(),
                font_generation: crate::text_render::font_generation(),
                anchors,
            });
        }

        let signature = ContourSignature {
            pool_layout_generation: self.pool.layout_generation(),
            params,
            grid: resolved.grid,
            epochs: resolved.epochs,
            levels,
            level_colors,
            label: label_sig,
        };
        if let Some(entry) = self.contour_cache.get(cfg.series_id.as_str())
            && entry.signature == signature
        {
            return Ok(Arc::clone(&entry.sources));
        }

        // A contour interpolates point-to-point, so it needs two samples along
        // each axis before it has a cell to place a line in.
        let drawable = resolved.extent.cols >= 2 && resolved.extent.rows >= 2;
        let sources = Self::grid_receipt(matrix, &signature.epochs)?;

        // `fs_contour` does not read `Style`, but the layout is shared with the
        // fill and group 1 has to be bound. Transparent, so a future entry point
        // that did read it would draw nothing rather than something arbitrary.
        let style_buf = data_render::create_style_uniform_buffer(
            &self.device,
            &PrimitiveStyle::from_color(Color::new(0.0, 0.0, 0.0, 0.0)),
        );
        let style_bg =
            data_render::create_style_bind_group(&self.device, &self.style_bgl, &style_buf);
        let lookup_tables = data_render::build_field_lookup_tables(
            &[[0.0; 4]],
            &signature.levels[..level_count as usize],
        )?;
        let (field_bg, charge) = data_render::create_field_data_bind_group(
            &self.device,
            &self.gpu_ledger,
            &self.field_bgl,
            self.pool.buffer(),
            data_render::FieldTables {
                grid: &signature.grid,
                levels: &signature.levels,
                stops: &lookup_tables.stops,
                level_colors: if signature.level_colors.is_empty() {
                    &[[0.0; 4]]
                } else {
                    &signature.level_colors
                },
                lookup_metadata: &lookup_tables.metadata,
                params: &signature.params,
            },
        );

        let label = match signature.label.as_ref() {
            Some(label_sig) => self.build_contour_label(
                cfg.series_id.as_str(),
                chart_config,
                contour,
                label_sig,
                scale,
            )?,
            None => None,
        };

        evict_if_full(
            &mut self.contour_cache,
            cfg.series_id.as_str(),
            CONTOUR_SCRATCH_CACHE_LIMIT,
        );
        self.contour_cache
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "contour scratch cache",
                reason: error.to_string(),
            })?;
        let receipt = Arc::clone(&sources);
        self.contour_cache.insert(
            cfg.series_id.clone(),
            ContourScratchEntry {
                signature,
                style_bg,
                field_bg,
                charge,
                sources,
                drawable,
                label,
            },
        );
        Ok(receipt)
    }

    /// Bake one contour series' label atlas and allocate its anchor state.
    ///
    /// `None` when nothing is bakeable because every string is empty. A requested
    /// atlas that cannot fit the device limit is an explicit series error rather
    /// than silently dropping labels.
    pub(super) fn build_contour_label(
        &mut self,
        series_id: &str,
        chart_config: &Config,
        contour: &crate::data_config::ContourConfig,
        label_sig: &ContourLabelSignature,
        scale: f32,
    ) -> Result<Option<crate::gpu_contour::ContourLabelState>> {
        let Some(labels) = contour.labels.as_ref() else {
            return Ok(None);
        };
        let Some(baked) = axis_render::bake_contour_label_atlas(
            chart_config,
            labels,
            &label_sig.texts,
            scale,
            self.caps.max_texture_dimension_2d,
        )
        .map_err(|error| FiggyError::InvalidSeriesConfig {
            series_id: series_id.to_owned(),
            reason: error.to_string(),
        })?
        else {
            return Ok(None);
        };
        let cell_w = crate::gpu_contour::try_collect(
            "contour label cell widths",
            baked.cells.iter().map(|cell| cell.width),
        )?;
        if self.contour_label_pipelines.is_none() {
            *self.contour_label_pipelines = Some(crate::gpu_contour::ContourLabelPipelines::new(
                &self.device,
                &self.field_bgl,
            ));
        }
        let pipelines = self
            .contour_label_pipelines
            .as_ref()
            .expect("contour label pipelines ensured");
        let state = crate::gpu_contour::ContourLabelState::build(
            &self.device,
            &self.queue,
            &self.gpu_ledger,
            pipelines,
            &self.sampler,
            self.caps.max_texture_dimension_2d,
            crate::gpu_contour::LabelAtlas {
                rgba: &baked.rgba,
                width: baked.width,
                height: baked.height,
                cell_h: baked.cell_h,
                cell_stride_w: baked.cell_stride_w,
                cell_stride_h: baked.cell_stride_h,
                columns: baked.columns,
                rows: baked.rows,
                gutter: baked.gutter,
                cell_w: &cell_w,
            },
            (!label_sig.anchors.is_empty()).then_some(label_sig.anchors.as_slice()),
        )?;
        Ok(Some(state))
    }

    /// Compile the label shader's module and layouts if any item wants labels.
    ///
    /// Separate from `ensure_precise_variants_for_items` because that function
    /// may only *use* them: it is the single site allowed to compile a target
    /// pipeline (design B.6), and it has no `&mut self` to create the module
    /// with.
    pub(super) fn ensure_contour_label_pipelines(&mut self, items: &[ChartDrawItem<'_>]) {
        if self.contour_label_pipelines.is_some() {
            return;
        }
        let wanted = items.iter().any(|item| {
            item.series.iter().any(|series| {
                effective_series_primitives(
                    &item.chart_config.draw_style,
                    &series.config.render_type,
                )
                .contour
                    && series_wants_contour_labels(&series.config.render_type)
            })
        });
        if wanted {
            *self.contour_label_pipelines = Some(crate::gpu_contour::ContourLabelPipelines::new(
                &self.device,
                &self.field_bgl,
            ));
        }
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

    pub(super) fn ensure_vector_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<Vec<Arc<GridSources>>> {
        self.preparation_parts()
            .1
            .ensure_vector_scratches(items, scale)
    }

    pub(super) fn ensure_vector_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<Option<Arc<GridSources>>> {
        self.preparation_parts()
            .1
            .ensure_vector_scratch(chart_config, cfg, scale)
    }

    /// Prepare every contour series' reusable GPU state.
    ///
    /// Returns the grid receipts for the token, the same way
    /// `ensure_field_scratches` does. Automatic label anchors are the only
    /// contour compute dispatch and remain frame-dependent.
    /// `scale` is the export DPI multiplier — the same one
    /// `create_style_for_series_scaled` applies to every other pixel dimension.
    /// It rides in the signature, so switching between a window frame and an
    /// export rebuilds the level styles rather than drawing one at the other's
    /// stroke width.
    pub(super) fn ensure_contour_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
        contour_spacings: &[Vec<Option<f32>>],
    ) -> Result<(Vec<PreparedContourItem>, Vec<Arc<GridSources>>)> {
        self.preparation_parts()
            .1
            .ensure_contour_scratches(items, scale, contour_spacings)
    }

    /// Build or reuse one contour series' GPU state.
    ///
    /// Since design B.4.9 this resolves the same group-2 tables a field does: the
    /// isolines are drawn from the grid itself by `fs_contour`, so there is no
    /// trace to dispatch and nothing here depends on the panel transform. The
    /// label anchors do — `ensure_contour_scratches` captures one placement
    /// snapshot per panel/item + series occurrence.
    ///
    /// `scale` is the export DPI multiplier, applied to the stroke width and the
    /// label font exactly as `create_style_for_series_scaled` applies it
    /// everywhere else.
    pub(super) fn ensure_contour_scratch(
        &mut self,
        cfg: &SeriesConfig,
        chart_config: &Config,
        scale: f32,
    ) -> Result<Arc<GridSources>> {
        self.preparation_parts()
            .1
            .ensure_contour_scratch(cfg, chart_config, scale)
    }

    /// Compile the label shader's module and layouts if any item wants labels.
    ///
    /// Separate from `ensure_precise_variants_for_items` because that function
    /// may only *use* them: it is the single site allowed to compile a target
    /// pipeline (design B.6), and it has no `&mut self` to create the module
    /// with.
    pub(super) fn ensure_contour_label_pipelines(&mut self, items: &[ChartDrawItem<'_>]) {
        self.preparation_parts()
            .1
            .ensure_contour_label_pipelines(items)
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

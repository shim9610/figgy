//! Contour preparation: level and label validation, contour signatures and
//! scratch, label snapshots and the ensure_contour_scratch path.

use super::*;

pub(in crate::renderer) fn contour_levels(rt: &DataRenderType) -> Result<Vec<f32>> {
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
pub(in crate::renderer) fn declared_level_count(rt: &DataRenderType) -> usize {
    extract_contour(rt).map_or(0, |contour| contour.levels.len())
}

pub(in crate::renderer) fn validate_contour_level_count(
    series_id: &str,
    rt: &DataRenderType,
) -> Result<()> {
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

pub(in crate::renderer) fn validate_contour_label_config(
    series_id: &str,
    rt: &DataRenderType,
) -> Result<()> {
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

pub(in crate::renderer) fn contour_labels_use_automatic(rt: &DataRenderType) -> bool {
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
pub(in crate::renderer) fn preflight_contour_label_spacing(
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
pub(in crate::renderer) fn series_wants_contour_labels(rt: &DataRenderType) -> bool {
    extract_contour(rt)
        .and_then(|contour| contour.labels.as_ref())
        .is_some_and(|labels| labels.visible && labels.font_size > 0.0)
}

/// Per-series contour scratch cache limit, mirroring `ARC_PREFIX_CACHE_LIMIT`.
pub(in crate::renderer) const CONTOUR_SCRATCH_CACHE_LIMIT: usize = 256;

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
pub(in crate::renderer) struct ContourSignature {
    pub(in crate::renderer) pool_layout_generation: u64,
    pub(in crate::renderer) params: data_render::FieldParamsGpu,
    pub(in crate::renderer) grid: Vec<data_render::GridColumnGpu>,
    pub(in crate::renderer) epochs: Vec<u64>,
    pub(in crate::renderer) levels: Vec<f32>,
    /// Per-level stroke colour, premultiplied — as drawn, so comparing
    /// signatures compares pixels.
    pub(in crate::renderer) level_colors: Vec<[f32; 4]>,
    /// `None` when the series draws no labels.
    pub(in crate::renderer) label: Option<ContourLabelSignature>,
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
pub(in crate::renderer) struct ContourLabelSignature {
    /// Final formatted strings are the atlas' exact content authority. This
    /// captures f64 level distinctions and colourbar/level-spacing changes
    /// without accidentally keying the atlas to an unrelated chart axis.
    pub(in crate::renderer) texts: Vec<String>,
    /// `font_size * scale` — an export at 2x re-bakes rather than magnifying.
    pub(in crate::renderer) font_bits: u32,
    pub(in crate::renderer) color: [u32; 4],
    pub(in crate::renderer) policy: crate::text_render::FontPolicy,
    pub(in crate::renderer) family: String,
    pub(in crate::renderer) bg: Option<[u32; 4]>,
    pub(in crate::renderer) bg_padding_bits: u32,
    /// Runtime font registration invalidates a previously baked atlas even
    /// when every chart-owned formatting field is unchanged.
    pub(in crate::renderer) font_generation: u64,
    /// The resolved host override. Empty means the GPU picks the anchors.
    pub(in crate::renderer) anchors: Vec<crate::gpu_contour::LabelAnchorGpu>,
}

/// One contour series' resolved GPU state — the same shape a field's is.
pub(in crate::renderer) struct ContourScratchEntry {
    pub(in crate::renderer) signature: ContourSignature,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) field_bg: wgpu::BindGroup,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) sources: Arc<GridSources>,
    /// False when the grid has no cell to interpolate a contour across.
    pub(in crate::renderer) drawable: bool,
    /// `None` when the series draws no labels — hidden, no label declaration, or
    /// every formatted string empty. Atlas/device-limit failures are errors.
    pub(in crate::renderer) label: Option<crate::gpu_contour::ContourLabelState>,
}

impl PrepareContext<'_> {
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
    pub(in crate::renderer) fn ensure_contour_scratches(
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
    pub(in crate::renderer) fn contour_label_snapshot(
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
    pub(in crate::renderer) fn contour_pick_label(
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
    pub(in crate::renderer) fn ensure_contour_scratch(
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
    pub(in crate::renderer) fn build_contour_label(
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
    pub(in crate::renderer) fn ensure_contour_label_pipelines(
        &mut self,
        items: &[ChartDrawItem<'_>],
    ) {
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
}

impl Renderer {
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
    pub(in crate::renderer) fn ensure_contour_scratches(
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
    pub(in crate::renderer) fn ensure_contour_scratch(
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
    pub(in crate::renderer) fn ensure_contour_label_pipelines(
        &mut self,
        items: &[ChartDrawItem<'_>],
    ) {
        self.preparation_parts()
            .1
            .ensure_contour_label_pipelines(items)
    }
}

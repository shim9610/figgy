//! Field preparation: the resolved grid, field signatures and scratch, and
//! the ensure_field_scratch path.

use super::*;

/// A grid declaration resolved against the pool — see `Renderer::resolve_grid`.
pub(in crate::renderer) struct ResolvedGrid {
    pub(in crate::renderer) x: ColumnHandle,
    pub(in crate::renderer) y: ColumnHandle,
    pub(in crate::renderer) grid: Vec<data_render::GridColumnGpu>,
    pub(in crate::renderer) epochs: Vec<u64>,
    pub(in crate::renderer) extent: MatrixExtent,
    pub(in crate::renderer) columns_are_y: bool,
    pub(in crate::renderer) centers: bool,
}

/// Per-series field scratch cache limit, mirroring `ARC_PREFIX_CACHE_LIMIT`.
pub(in crate::renderer) const FIELD_SCRATCH_CACHE_LIMIT: usize = 256;

/// The grid columns a prepared field or contour read, and the allocation epochs
/// they had.
///
/// Shared with the `PreparedFrame` by `Arc`, so validating a token walks it
/// without allocating and a matrix' thousands of columns cost one pointer in the
/// token rather than a `HashMap` entry each.
#[derive(Debug)]
pub(in crate::renderer) struct GridSources {
    pub(in crate::renderer) ids: Vec<ColumnId>,
    pub(in crate::renderer) epochs: Vec<u64>,
}

/// What a cached field bind group was built from.
///
/// Everything the group-2 buffers hold, plus the pool layout it was resolved
/// against. Equality means the cached bind group is still exactly right, so a
/// prepare that changes nothing uploads nothing — which is the difference
/// between a 40 KB grid table per frame and one per change.
#[derive(Debug, PartialEq)]
pub(in crate::renderer) struct FieldSignature {
    pub(in crate::renderer) pool_layout_generation: u64,
    pub(in crate::renderer) params: data_render::FieldParamsGpu,
    pub(in crate::renderer) grid: Vec<data_render::GridColumnGpu>,
    pub(in crate::renderer) epochs: Vec<u64>,
    pub(in crate::renderer) levels: Vec<f32>,
    pub(in crate::renderer) stops: Vec<[f32; 4]>,
    pub(in crate::renderer) nan_color: [u32; 4],
}

/// One field series' resolved GPU state.
pub(in crate::renderer) struct FieldScratch {
    pub(in crate::renderer) signature: FieldSignature,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) field_bg: wgpu::BindGroup,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) sources: Arc<GridSources>,
    pub(in crate::renderer) drawable: bool,
}

/// The contour levels a render type declares, as the f32s both the fill's band
/// quantization and the implicit contour shader compare against.
///
/// Padded to one entry when there are none: a storage binding cannot be
/// zero-sized, and the real count travels in the params uniform.
/// What every colour-mapped primitive's GPU params take from the chart
/// colourbar: the z bounds as hi/lo pairs, already log10 under a logarithmic
/// bar, and the premultiplied `nan_color`.
///
/// The log bounds are taken here, in f64: doing it in the shader would lose
/// the precision the bounds were written with, and `validate_renderer_config`
/// has already refused a log colourbar whose lower bound is not positive. One
/// place for the field, the point colour map and the vector colour map, so
/// their shaders (which share the ramp, SHADER_COMMON.md §5.4) read bounds
/// computed the same way.
pub(in crate::renderer) struct ColorbarGpu {
    pub(in crate::renderer) log_z: bool,
    pub(in crate::renderer) z_min: [f32; 2],
    pub(in crate::renderer) z_max: [f32; 2],
    pub(in crate::renderer) nan_color_premul: [f32; 4],
}

pub(in crate::renderer) fn colorbar_gpu(bar: &crate::config::ColorBarOptions) -> ColorbarGpu {
    let log_z = matches!(bar.axis.scale, AxisScale::Logarithmic);
    let bound = |value: f64| {
        let transformed = if log_z { value.log10() } else { value };
        let (hi, lo) = crate::data::split_f64_to_f32_pair(transformed);
        [hi, lo]
    };
    let nan = bar.nan_color;
    let alpha = nan.a.clamp(0.0, 1.0);
    ColorbarGpu {
        log_z,
        z_min: bound(bar.axis.min),
        z_max: bound(bar.axis.max),
        nan_color_premul: [nan.r * alpha, nan.g * alpha, nan.b * alpha, alpha],
    }
}

/// The colourbar's ramp control points as straight RGBA for a storage
/// binding. `stop_count` is the real count; the table keeps one padding entry
/// when the map has none, because a storage binding cannot be zero-sized.
pub(in crate::renderer) struct ColorbarStops {
    pub(in crate::renderer) stops: Vec<[f32; 4]>,
    pub(in crate::renderer) stop_count: u32,
}

pub(in crate::renderer) fn colorbar_stops(
    label: &'static str,
    bar: &crate::config::ColorBarOptions,
) -> Result<ColorbarStops> {
    let mut stops = crate::gpu_contour::try_collect(
        label,
        bar.colormap
            .stops()
            .iter()
            .map(|color| [color.r, color.g, color.b, color.a]),
    )?;
    let stop_count = u32::try_from(stops.len()).unwrap_or(u32::MAX);
    if stops.is_empty() {
        stops.push([0.0; 4]);
    }
    Ok(ColorbarStops { stops, stop_count })
}

impl PrepareContext<'_> {
    pub(in crate::renderer) fn ensure_field_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
    ) -> Result<Vec<Arc<GridSources>>> {
        let mut receipts = Vec::new();
        for item in items {
            for series in item.series {
                let cfg = series.config;
                if !effective_series_primitives(&item.chart_config.draw_style, &cfg.render_type)
                    .field
                {
                    continue;
                }
                let sources = self.ensure_field_scratch(item.chart_config, cfg)?;
                receipts
                    .try_reserve(1)
                    .map_err(|error| FiggyError::StateAllocationFailed {
                        resource: "field source receipts",
                        reason: error.to_string(),
                    })?;
                receipts.push(sources);
            }
        }
        Ok(receipts)
    }

    /// A grid declaration resolved against the pool: the coordinate handles, the
    /// `(base, len)` table, the per-column allocation epochs, and the extent.
    ///
    /// Shared by the field and the contour paths so that "what does this
    /// declaration point at right now" has one answer. Both need every part of
    /// it, and a second copy of the resolution is a second place for the two to
    /// disagree about which cell holds which value.
    pub(in crate::renderer) fn resolve_grid(
        &self,
        cfg: &SeriesConfig,
        matrix: &crate::data_config::MatrixRef,
    ) -> Result<ResolvedGrid> {
        let alloc = |resource: &'static str| {
            move |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource,
                reason: error.to_string(),
            }
        };
        let x = self
            .pool
            .handle_for(&cfg.x_column)
            .ok_or_else(|| FiggyError::UnknownColumn {
                id: cfg.x_column.clone(),
            })?;
        let y = self
            .pool
            .handle_for(&cfg.y_column)
            .ok_or_else(|| FiggyError::UnknownColumn {
                id: cfg.y_column.clone(),
            })?;

        let mut grid = Vec::new();
        grid.try_reserve_exact(matrix.columns.len().max(1))
            .map_err(alloc("field grid table"))?;
        let mut epochs = Vec::new();
        epochs
            .try_reserve_exact(matrix.columns.len())
            .map_err(alloc("field grid epochs"))?;
        let mut lengths = Vec::new();
        lengths
            .try_reserve_exact(matrix.columns.len())
            .map_err(alloc("field grid column lengths"))?;
        for id in &matrix.columns {
            let handle = self
                .pool
                .handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.clone() })?;
            let epoch = self
                .pool
                .allocation_epoch(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.clone() })?;
            lengths.push(handle.len_values);
            epochs.push(epoch);
            grid.push(data_render::GridColumnGpu {
                base: u32::try_from(handle.offset / 4).map_err(|_| {
                    FiggyError::InvalidSeriesConfig {
                        series_id: cfg.series_id.clone(),
                        reason: "grid column offset exceeds a 32-bit lane index".into(),
                    }
                })?,
                len: u32::try_from(handle.len_values).unwrap_or(u32::MAX),
            });
        }
        let mut index = 0usize;
        let extent = matrix_extent(matrix, x.len_values, y.len_values, |_| {
            let len = lengths.get(index).copied().unwrap_or(0);
            index += 1;
            len
        });
        // A storage binding cannot be zero-sized, so an empty table is padded.
        // The real count travels in the params uniform.
        if grid.is_empty() {
            grid.push(data_render::GridColumnGpu { base: 0, len: 0 });
        }
        Ok(ResolvedGrid {
            x,
            y,
            grid,
            epochs,
            extent,
            columns_are_y: matches!(
                matrix.orientation,
                crate::data_config::MatrixOrientation::ColumnsAreY
            ),
            centers: matches!(matrix.grid_layout, crate::data_config::GridLayout::Centers),
        })
    }

    /// The grid columns a resolved declaration read, as a shared receipt.
    pub(in crate::renderer) fn grid_receipt(
        matrix: &crate::data_config::MatrixRef,
        epochs: &[u64],
    ) -> Result<Arc<GridSources>> {
        let alloc = |resource: &'static str| {
            move |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource,
                reason: error.to_string(),
            }
        };
        let mut ids = Vec::new();
        ids.try_reserve_exact(matrix.columns.len())
            .map_err(alloc("grid source ids"))?;
        ids.extend(matrix.columns.iter().cloned());
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(epochs.len())
            .map_err(alloc("grid source epochs"))?;
        owned.extend_from_slice(epochs);
        Ok(Arc::new(GridSources { ids, epochs: owned }))
    }

    /// Build or reuse one field series' group-2 bind group.
    ///
    /// The signature is compared before anything is created, so a steady frame
    /// uploads nothing at all. When it does differ, the new buffers replace the
    /// old ones outright — a live `PreparedFrame` holds its own refcounted clone
    /// of the bind group, so nothing it can still draw with is disturbed. That is
    /// why this needs none of `arc_cache`'s immutable keyed results: the field
    /// buffers themselves are already immutable snapshots.
    pub(in crate::renderer) fn ensure_field_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
    ) -> Result<Arc<GridSources>> {
        let rt = &cfg.render_type;
        let matrix = extract_matrix(rt).expect("a field series has a matrix");
        let fill = extract_field_fill(rt).expect("a field series has a fill config");
        validate_contour_level_count(cfg.series_id.as_str(), rt)?;
        let bar =
            chart_config
                .colorbar
                .as_ref()
                .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    reason: "a field series needs Config.colorbar".into(),
                })?;
        let mut resolved = self.resolve_grid(cfg, matrix)?;
        // A surface coloured by a second grid on a 3D chart: its columns
        // follow the height columns in the same table (`color_base` is the
        // height column count), so the surface reads both through one group.
        let color_matrix = chart_config
            .view_3d
            .as_ref()
            .and(rt.surface_style())
            .and_then(|style| style.color_matrix.as_ref());
        if let Some(color) = color_matrix {
            let colors = self.resolve_grid(cfg, color)?;
            if !matrix.columns.is_empty() {
                resolved.grid.truncate(matrix.columns.len());
            }
            resolved
                .grid
                .try_reserve(color.columns.len())
                .and_then(|()| resolved.epochs.try_reserve(color.columns.len()))
                .map_err(|error| FiggyError::StateAllocationFailed {
                    resource: "surface colour grid table",
                    reason: error.to_string(),
                })?;
            resolved
                .grid
                .extend(colors.grid.iter().take(color.columns.len()).copied());
            resolved.epochs.extend_from_slice(&colors.epochs);
        }

        let mut flags = 0u32;
        if resolved.columns_are_y {
            flags |= data_render::FIELD_FLAG_COLUMNS_ARE_Y;
        }
        if resolved.centers {
            flags |= data_render::FIELD_FLAG_CENTERS;
        }
        let interpolated = matches!(fill.shading, crate::data_config::Shading::Interpolated);
        if interpolated {
            flags |= data_render::FIELD_FLAG_INTERPOLATED;
        }
        if matches!(fill.mode, crate::data_config::FillMode::Bands) {
            flags |= data_render::FIELD_FLAG_BANDS;
        }
        let colorbar = colorbar_gpu(bar);
        if colorbar.log_z {
            flags |= data_render::FIELD_FLAG_LOG_Z;
        }

        let levels = contour_levels(rt)?;
        let level_count = u32::try_from(declared_level_count(rt)).unwrap_or(u32::MAX);
        let ColorbarStops { stops, stop_count } = colorbar_stops("field colourmap stops", bar)?;

        let params = data_render::FieldParamsGpu {
            x_base: u32::try_from(resolved.x.offset / 4).unwrap_or(0),
            y_base: u32::try_from(resolved.y.offset / 4).unwrap_or(0),
            x_len: u32::try_from(resolved.x.len_values).unwrap_or(u32::MAX),
            y_len: u32::try_from(resolved.y.len_values).unwrap_or(u32::MAX),
            cols: u32::try_from(resolved.extent.cols).unwrap_or(u32::MAX),
            rows: u32::try_from(resolved.extent.rows).unwrap_or(u32::MAX),
            level_count,
            stop_count,
            flags,
            opacity: fill.opacity.clamp(0.0, 1.0),
            // Contour-only; a fill draws no strokes and reads no level colours.
            line_width_px: 0.0,
            level_color_count: 0,
            z_min: colorbar.z_min,
            z_max: colorbar.z_max,
        };
        let nan = bar.nan_color;
        let signature = FieldSignature {
            pool_layout_generation: self.pool.layout_generation(),
            params,
            grid: resolved.grid,
            epochs: resolved.epochs,
            levels,
            stops,
            // Bits, not floats: the signature has to be comparable, and two
            // NaN alphas that differ in bits are two different configs.
            nan_color: [
                nan.r.to_bits(),
                nan.g.to_bits(),
                nan.b.to_bits(),
                nan.a.to_bits(),
            ],
        };

        if let Some(scratch) = self.field_cache.get(cfg.series_id.as_str())
            && scratch.signature == signature
        {
            return Ok(Arc::clone(&scratch.sources));
        }

        let sources = match color_matrix {
            Some(color) => {
                let mut both = matrix.clone();
                both.columns
                    .try_reserve(color.columns.len())
                    .map_err(|error| FiggyError::StateAllocationFailed {
                        resource: "surface colour grid receipt",
                        reason: error.to_string(),
                    })?;
                both.columns.extend(color.columns.iter().cloned());
                Self::grid_receipt(&both, &signature.epochs)?
            }
            None => Self::grid_receipt(matrix, &signature.epochs)?,
        };
        let scratch = self.build_field_scratch(signature, sources)?;

        // Bounded like `arc_cache`: a host cycling through thousands of series
        // ids must not grow this without limit.
        evict_if_full(
            &mut self.field_cache,
            cfg.series_id.as_str(),
            FIELD_SCRATCH_CACHE_LIMIT,
        );
        self.field_cache
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "field scratch cache",
                reason: error.to_string(),
            })?;
        let receipt = Arc::clone(&scratch.sources);
        self.field_cache.insert(cfg.series_id.clone(), scratch);
        Ok(receipt)
    }

    /// Build the group-2 state a signature describes.
    ///
    /// The signature (and the column receipt, which names the same columns
    /// the signature's epochs stamp) is this function's only input besides
    /// the device: the cache compares signatures, so whatever the build could
    /// read beyond one would be a change the compare cannot see. Keeping the
    /// build to the signature makes the compare complete by construction.
    fn build_field_scratch(
        &self,
        signature: FieldSignature,
        sources: Arc<GridSources>,
    ) -> Result<FieldScratch> {
        let params = &signature.params;
        // Interpolated shading spans point-to-point, so it needs two samples
        // along each axis before it has a quad to draw.
        let interpolated = params.flags & data_render::FIELD_FLAG_INTERPOLATED != 0;
        let minimum = if interpolated { 2 } else { 1 };
        let drawable = params.cols >= minimum && params.rows >= minimum;

        let [r, g, b, a] = signature.nan_color;
        let nan_color = Color::new(
            f32::from_bits(r),
            f32::from_bits(g),
            f32::from_bits(b),
            f32::from_bits(a),
        );
        let nan_buf = data_render::create_style_uniform_buffer(
            &self.device,
            &PrimitiveStyle::from_color(nan_color),
        );
        let style_bg =
            data_render::create_style_bind_group(&self.device, &self.style_bgl, &nan_buf);
        let lookup_tables = data_render::build_field_lookup_tables(
            &signature.stops,
            &signature.levels[..params.level_count as usize],
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
                // Fill-only bind group: the stroke colour table is padding.
                level_colors: &[[0.0; 4]],
                lookup_metadata: &lookup_tables.metadata,
                params,
            },
        );
        Ok(FieldScratch {
            signature,
            style_bg,
            field_bg,
            charge,
            sources,
            drawable,
        })
    }
}

impl Renderer {
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
    pub(in crate::renderer) fn ensure_field_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
    ) -> Result<Vec<Arc<GridSources>>> {
        self.preparation_parts().1.ensure_field_scratches(items)
    }

    /// Build or reuse one field series' group-2 bind group.
    ///
    /// The signature is compared before anything is created, so a steady frame
    /// uploads nothing at all. When it does differ, the new buffers replace the
    /// old ones outright — a live `PreparedFrame` holds its own refcounted clone
    /// of the bind group, so nothing it can still draw with is disturbed. That is
    /// why this needs none of `arc_cache`'s immutable keyed results: the field
    /// buffers themselves are already immutable snapshots.
    pub(in crate::renderer) fn ensure_field_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
    ) -> Result<Arc<GridSources>> {
        self.preparation_parts()
            .1
            .ensure_field_scratch(chart_config, cfg)
    }
}

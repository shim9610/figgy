//! Vector field preparation: signatures, scratch and the ensure_vector_scratch path.

use super::*;

/// Per-series vector scratch cache limit, mirroring `FIELD_SCRATCH_CACHE_LIMIT`.
pub(in crate::renderer) const VECTOR_SCRATCH_CACHE_LIMIT: usize = 256;

/// Everything a vector series' group-2 state depends on. Compared before any
/// upload, so a steady frame builds nothing.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::renderer) struct VectorSignature {
    pub(in crate::renderer) pool_layout_generation: u64,
    /// Which input fragment the module is assembled with.
    pub(in crate::renderer) input: data_render::vector::VectorInput,
    pub(in crate::renderer) params: data_render::vector::VectorParamsGpu,
    pub(in crate::renderer) grid: Vec<data_render::GridColumnGpu>,
    pub(in crate::renderer) epochs: Vec<u64>,
    pub(in crate::renderer) stops: Vec<[f32; 4]>,
    /// Series colour bits: the group-1 style is built here too.
    pub(in crate::renderer) color: [u32; 4],
}

/// One vector series' resolved GPU state.
pub(in crate::renderer) struct VectorScratch {
    pub(in crate::renderer) signature: VectorSignature,
    pub(in crate::renderer) input: data_render::vector::VectorInput,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) vector_bg: wgpu::BindGroup,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    /// Grid form only: the `u` grid as a field's group 2, so the selected
    /// cell highlight runs the field pipeline unchanged. The grid's arrow
    /// pick binds `vector_bg` (`pick_vector_cell`).
    pub(in crate::renderer) field: Option<(wgpu::BindGroup, crate::gpu_memory::SharedCharge)>,
    pub(in crate::renderer) sources: Arc<GridSources>,
    /// Arrows the draw issues; zero draws nothing.
    pub(in crate::renderer) count: u32,
}

impl PrepareContext<'_> {
    /// Prepare every vector series' group-2 state, returning the column
    /// receipts for the token the way `ensure_field_scratches` does.
    pub(in crate::renderer) fn ensure_vector_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<Vec<Arc<GridSources>>> {
        let mut receipts = Vec::new();
        for item in items {
            for series in item.series {
                let cfg = series.config;
                let primitives =
                    effective_series_primitives(&item.chart_config.draw_style, &cfg.render_type);
                // A streamline series' arrowheads draw with its line.
                let heads = primitives.line
                    && cfg
                        .render_type
                        .streamlines()
                        .is_some_and(|streamlines| streamlines.arrows.is_some());
                if !primitives.vector && !heads {
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
    pub(in crate::renderer) fn ensure_vector_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<Option<Arc<GridSources>>> {
        use crate::data_config::{VectorColorMap, VectorComponents, VectorPivot, VectorScale};
        use data_render::vector as vector_render;
        // A vector series, or the arrowheads along a streamline series'
        // drawn path (the path form: heads only, pixel-scaled to one head
        // length, centred on their arc slots).
        let vector = extract_vector(&cfg.render_type);
        let streamline = cfg
            .render_type
            .streamlines()
            .and_then(|streamlines| streamlines.arrows.map(|arrows| (streamlines, arrows)));
        if vector.is_none() && streamline.is_none() {
            return Ok(None);
        }
        let invalid = |reason: String| FiggyError::InvalidSeriesConfig {
            series_id: cfg.series_id.clone(),
            reason,
        };
        if let Some((_, style)) = vector {
            style.validate().map_err(|reason| invalid(reason.into()))?;
        }
        if let Some((_, arrows)) = streamline {
            arrows.validate().map_err(|reason| invalid(reason.into()))?;
        }
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
            pivot: vector_render::VECTOR_PIVOT_MIDDLE,
            scale: 0.0,
            width_px: 0.0,
            head_length_px: 0.0,
            head_width_px: 0.0,
            min_length_px: 0.0,
            aa_px: scale,
            spacing_px: 0.0,
            pad1: 0.0,
            z_min: [0.0; 2],
            z_max: [0.0; 2],
            nan_color_premul: [0.0; 4],
        };
        if let Some((_, style)) = vector {
            params.pivot = match style.pivot {
                VectorPivot::Tail => vector_render::VECTOR_PIVOT_TAIL,
                VectorPivot::Middle => vector_render::VECTOR_PIVOT_MIDDLE,
                VectorPivot::Tip => vector_render::VECTOR_PIVOT_TIP,
            };
            params.width_px = style.width_px * scale;
            params.head_length_px = style.head_length_px * scale;
            params.head_width_px = style.head_width_px * scale;
            params.min_length_px = style.min_length_px * scale;
            match style.scale {
                VectorScale::Data { factor } => params.scale = factor as f32,
                VectorScale::Pixels { px_per_unit } => {
                    params.scale = px_per_unit as f32 * scale;
                    params.flags |= vector_render::VECTOR_FLAG_SCALE_PIXELS;
                }
            }
        }
        if let Some((_, arrows)) = streamline {
            // A head with no shaft: the arrow is one head length long along
            // the unit tangent, centred on its slot.
            params.head_length_px = arrows.head_length_px * scale;
            params.head_width_px = arrows.head_width_px * scale;
            params.scale = params.head_length_px;
            params.spacing_px = arrows.spacing_px * scale;
            params.flags |= vector_render::VECTOR_FLAG_SCALE_PIXELS;
        }
        let color_map = match (vector, streamline) {
            (Some((_, style)), _) => style.color_map.is_some(),
            (None, Some((streamlines, _))) => streamlines.color_map.is_some(),
            (None, None) => false,
        };

        let mut grid = Vec::new();
        let mut ids = Vec::new();
        let mut epochs = Vec::new();
        let input = match vector {
            // The arrowheads read the coordinates and the colour column
            // index-aligned; the arc slot data (group 3) is the line's arc
            // scan, bound at draw time.
            None => {
                let mut count = x.len_values.min(y.len_values);
                ids.try_reserve_exact(1)
                    .map_err(alloc("vector source ids"))?;
                epochs
                    .try_reserve_exact(1)
                    .map_err(alloc("vector source epochs"))?;
                if let Some(map) = streamline.and_then(|(streamlines, _)| streamlines.color_map) {
                    let c_h = handle(&map.column)?;
                    params.c_base = lane(&c_h)?;
                    params.c_len = len32(&c_h);
                    count = count.min(c_h.len_values);
                    ids.push(map.column.clone());
                    epochs.push(epoch(&map.column)?);
                }
                params.count = u32::try_from(count).unwrap_or(u32::MAX);
                // A storage binding cannot be zero-sized: one padding entry.
                grid.push(data_render::GridColumnGpu { base: 0, len: 0 });
                vector_render::VectorInput::Path
            }
            Some((VectorComponents::Points { u, v }, style)) => {
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
            Some((VectorComponents::Grid { u, v }, style)) => {
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
                if u_grid.columns_are_y {
                    params.flags |= vector_render::VECTOR_FLAG_COLUMNS_ARE_Y;
                }
                if u_grid.centers {
                    params.flags |= vector_render::VECTOR_FLAG_CENTERS;
                }
                vector_render::VectorInput::Grid
            }
        };

        let mut stops = Vec::new();
        if color_map {
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

        // The heads take the line colour without a colour map.
        let color = match (vector, streamline) {
            (Some((_, style)), _) => style.color,
            (None, Some((streamlines, _))) => streamlines.line.line_color,
            (None, None) => Color::BLACK,
        };
        let signature = VectorSignature {
            pool_layout_generation: self.pool.layout_generation(),
            input,
            params,
            grid,
            epochs,
            stops,
            color: [
                color.r.to_bits(),
                color.g.to_bits(),
                color.b.to_bits(),
                color.a.to_bits(),
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
        let scratch = self.build_vector_scratch(signature, sources);

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
        let receipt = Arc::clone(&scratch.sources);
        self.vector_cache.insert(cfg.series_id.clone(), scratch);
        Ok(Some(receipt))
    }

    /// Build the group-1 and group-2 state a signature describes.
    ///
    /// The signature (and the column receipt, which names the same columns
    /// the signature's epochs stamp) is this function's only input besides
    /// the device: the cache compares signatures, so whatever the build could
    /// read beyond one would be a change the compare cannot see. Keeping the
    /// build to the signature makes the compare complete by construction.
    fn build_vector_scratch(
        &self,
        signature: VectorSignature,
        sources: Arc<GridSources>,
    ) -> VectorScratch {
        use data_render::vector as vector_render;
        let params = &signature.params;
        let [r, g, b, a] = signature.color;
        let color = Color::new(
            f32::from_bits(r),
            f32::from_bits(g),
            f32::from_bits(b),
            f32::from_bits(a),
        );
        let style_buf = data_render::create_style_uniform_buffer(
            &self.device,
            &PrimitiveStyle::from_color_with_width(color, params.width_px),
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
                params,
            },
        );
        // Grid form only: the `u` grid's lattice as a field, for the selected
        // cell outline; that pass reads only the lattice. The grid table
        // starts with the `u` grid's columns, which is all the field lattice
        // indexes; the same buffer serves both groups.
        let field = (signature.input == vector_render::VectorInput::Grid).then(|| {
            let mut field_flags = 0u32;
            if params.flags & vector_render::VECTOR_FLAG_COLUMNS_ARE_Y != 0 {
                field_flags |= data_render::FIELD_FLAG_COLUMNS_ARE_Y;
            }
            if params.flags & vector_render::VECTOR_FLAG_CENTERS != 0 {
                field_flags |= data_render::FIELD_FLAG_CENTERS;
            }
            let field_params = data_render::FieldParamsGpu::cell_geometry(
                params.x_base,
                params.y_base,
                params.x_len,
                params.y_len,
                params.cols,
                params.rows,
                field_flags,
            );
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
        let input = signature.input;
        let count = params.count;
        VectorScratch {
            signature,
            input,
            style_bg,
            vector_bg,
            charge,
            field,
            sources,
            count,
        }
    }
}

impl Renderer {
    pub(in crate::renderer) fn ensure_vector_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<Vec<Arc<GridSources>>> {
        self.preparation_parts()
            .1
            .ensure_vector_scratches(items, scale)
    }

    pub(in crate::renderer) fn ensure_vector_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<Option<Arc<GridSources>>> {
        self.preparation_parts()
            .1
            .ensure_vector_scratch(chart_config, cfg, scale)
    }
}

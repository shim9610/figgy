//! Point value map preparation: signatures, scratch and the
//! ensure_value_map_scratch path.

use super::*;

/// Per-series point value-map scratch cache limit, mirroring the field limit.
pub(in crate::renderer) const VALUE_MAP_SCRATCH_CACHE_LIMIT: usize = 256;

/// What a cached point value-map bind group (group 3) was built from: the
/// params uniform, compared by its bytes so a NaN `nan_color` still compares
/// equal to itself, and the colourbar's colormap (SSOT config, copied). Equality
/// means the cached bind group is still exactly right, so a prepare that
/// changes nothing uploads nothing — the same contract as [`FieldSignature`].
#[derive(Debug)]
pub(in crate::renderer) struct ValueMapSignature {
    pub(in crate::renderer) params: data_render::PointValueMapParamsGpu,
    pub(in crate::renderer) colormap: Option<crate::colormap::ColorMap>,
}

impl PartialEq for ValueMapSignature {
    fn eq(&self, other: &Self) -> bool {
        bytemuck::bytes_of(&self.params) == bytemuck::bytes_of(&other.params)
            && self.colormap == other.colormap
    }
}

/// One scatter series' resolved value-map state: the group-3 bind group of
/// the mapped scatter and pick-ring pipelines.
pub(in crate::renderer) struct ValueMapScratch {
    pub(in crate::renderer) signature: ValueMapSignature,
    pub(in crate::renderer) bind_group: wgpu::BindGroup,
    /// The group's buffers, bound again with the camera on a 3D chart
    /// (#132).
    pub(in crate::renderer) buffers: data_render::PointValueMapBuffers,
    pub(in crate::renderer) _charge: crate::gpu_memory::SharedCharge,
}

impl PrepareContext<'_> {
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
    pub(in crate::renderer) fn ensure_value_map_scratches(
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
    pub(in crate::renderer) fn ensure_value_map_scratch(
        &mut self,
        chart_config: &Config,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<()> {
        // A scatter's value maps, or the colour column a streamline series'
        // stroke is mapped by (the same colour map, read by the mapped line
        // pipeline as its group 3).
        // A parallel series' colour column is mapped the same way (#141).
        let scatter = extract_scatter(&cfg.render_type);
        let (color_mapped, size_map) = match (
            &scatter,
            cfg.render_type.streamlines(),
            cfg.render_type.parallel(),
        ) {
            (Some(scatter), _, _) => (
                scatter.point_color_map.is_some(),
                scatter.point_size_map.as_ref(),
            ),
            (None, Some(streamlines), _) => (streamlines.color_map.is_some(), None),
            (None, None, Some(parallel)) => (parallel.color_column.is_some(), None),
            (None, None, None) => return Ok(()),
        };
        if !color_mapped && size_map.is_none() {
            return Ok(());
        }
        let pair = |value: f64| {
            let (hi, lo) = crate::data::split_f64_to_f32_pair(value);
            [hi, lo]
        };
        let mut params = data_render::PointValueMapParamsGpu::NEUTRAL;
        let mut colormap = None;
        if color_mapped {
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
        if let Some(map) = size_map {
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
        let scratch = self.build_value_map_scratch(signature);
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
        self.value_map_cache.insert(cfg.series_id.clone(), scratch);
        Ok(())
    }

    /// Build the group-3 state a signature describes.
    ///
    /// The signature is this function's only input besides the device: the
    /// cache compares signatures, so whatever the build could read beyond
    /// one would be a change the compare cannot see. Keeping the build to
    /// the signature makes the compare complete by construction.
    fn build_value_map_scratch(&self, signature: ValueMapSignature) -> ValueMapScratch {
        let stops: &[Color] = signature
            .colormap
            .as_ref()
            .map_or(&[], |colormap| colormap.stops());
        let (buffers, charge) = data_render::create_point_value_map_buffers(
            &self.device,
            &self.gpu_ledger,
            &signature.params,
            stops,
        );
        let bind_group =
            data_render::bind_point_value_map(&self.device, &self.point_value_map_bgl, &buffers);
        ValueMapScratch {
            signature,
            bind_group,
            buffers,
            _charge: charge,
        }
    }
}

impl Renderer {
    pub(in crate::renderer) fn ensure_value_map_scratches(
        &mut self,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<()> {
        self.preparation_parts()
            .1
            .ensure_value_map_scratches(items, scale)
    }
}

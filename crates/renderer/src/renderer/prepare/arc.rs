//! Arc prefix preparation for dashed and stylized lines: the prefix cache,
//! its keys and the ensure_arc_prefix path.

use super::*;

/// A dashed series' GPU arc-length prefix: the buffer (bound as the line
/// pipeline's vertex slots 4/5) and its used byte length.
pub(in crate::renderer) type ArcPrefix = (Arc<wgpu::Buffer>, u64);

pub(in crate::renderer) const ARC_PREFIX_CACHE_LIMIT: usize = 256;

pub(in crate::renderer) const ARC_RESULT_CACHE_LIMIT_PER_SERIES: usize = 8;

/// The arc slot pitch of a streamline series' arrowheads in render pixels:
/// their spacing at `display_scale`, or `None` without arrowheads. The draw
/// and the pick scan the arc with it, so they share the cache entry.
pub(in crate::renderer) fn streamline_arrow_pitch(
    render_type: &DataRenderType,
    display_scale: f32,
) -> Option<f32> {
    let arrows = render_type.streamlines()?.arrows?;
    Some((arrows.spacing_px * display_scale).max(1e-3))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::renderer) struct ArcResultKey {
    pub(in crate::renderer) n: u32,
    pub(in crate::renderer) x_base: u32,
    pub(in crate::renderer) y_base: u32,
    pub(in crate::renderer) pool_layout_generation: u64,
    pub(in crate::renderer) x_allocation_epoch: u64,
    pub(in crate::renderer) y_allocation_epoch: u64,
    /// The arc scan reads the first 64 bytes of `ScatterTransform` and its
    /// polar words (`polar_angle`, `polar_extent`, the last 32 bytes); the
    /// style parameters between them are left out, so a style change does
    /// not retraverse the data.
    pub(in crate::renderer) geometry_transform_bits: [u32; 16],
    pub(in crate::renderer) polar_transform_bits: [u32; 8],
    pub(in crate::renderer) star_pitch_bits: Option<u32>,
    /// Step interpolation mode: a stepped path has a different arc length.
    pub(in crate::renderer) step_mode: u32,
}

pub(in crate::renderer) struct ArcResultEntry {
    pub(in crate::renderer) key: ArcResultKey,
    pub(in crate::renderer) scratch: data_render::line_arc::ArcScratch,
}

impl PrepareContext<'_> {
    pub(in crate::renderer) fn prepare_arc_items(
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
                // Streamline arrowheads sit on arc slots: the scan runs with
                // their spacing in render pixels as the slot pitch, and its
                // indirect kernel counts the slots.
                let arrow_pitch = line
                    .then(|| streamline_arrow_pitch(&cfg.render_type, series.style.display_scale))
                    .flatten();
                let arc_wanted = dashed
                    || arrow_pitch.is_some()
                    || (variant.is_some_and(|v| v.needs_arc_prefix) && line);
                let arc = if arc_wanted {
                    self.ensure_arc_prefix(
                        &cfg.series_id,
                        &cfg.x_column,
                        &cfg.y_column,
                        &t,
                        if line {
                            arrow_pitch.or(star_pitch)
                        } else {
                            None
                        },
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

    /// The `(x_base, y_base, n)` source layout for an arc scan over these
    /// columns right now — `None` when a column is missing or shorter than
    /// two points.
    pub(in crate::renderer) fn arc_source_layout(
        &self,
        x_id: &str,
        y_id: &str,
    ) -> Option<(u32, u32, u32)> {
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
    pub(in crate::renderer) fn ensure_arc_prefix(
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
    pub(in crate::renderer) fn ensure_arc_prefix(
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

//! Point selection rings.

use super::*;

impl PrepareContext<'_> {
    /// The ring of one selected point: a marker's, a line anchor's or a
    /// point vector's anchor, mapped, ternary or 3D like its draw.
    pub(super) fn point_ring_layer<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        visual: PointSelectionVisual,
        vector_points: Option<&crate::data_config::VectorStyle>,
        picked: &mut Vec<ColumnPickRingLayer<'a>>,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            chart_config,
            series,
            pipelines,
            columns,
            use_scatter_style_mapping,
            cfg,
            rt,
            x_h,
            y_h,
            column_buffer,
            ..
        } = *s;
        let Some(instance) = columns
            .rows
            .instance(visual.point_index, x_h.len_values.min(y_h.len_values))
        else {
            return Ok(());
        };

        let scatter_owned = extract_scatter(rt);
        let scatter_cfg = scatter_owned.as_deref();
        let has_line_anchor = extract_line(rt).is_some();
        // A zero-size marker still draws its dot and picks, so it
        // takes the ring too; a size-mapped point the map hides
        // (negative radius) is dropped by the mapped ring entry.
        let has_scatter_anchor = scatter_cfg.is_some();
        let has_vector_anchor = vector_points
            .is_some_and(|style| crate::data_render::vector::vector_anchor_radius_px(style) > 0.0);
        if !has_line_anchor && !has_scatter_anchor && !has_vector_anchor {
            return Ok(());
        }

        let precise_pick_style_map = if use_scatter_style_mapping {
            series.style.scatter_map.as_ref()
        } else {
            None
        };
        if precise_pick_style_map.is_some()
            && columns.rows.global_start != 0
            && columns.mapped_point_bg.is_none()
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: cfg.series_id.clone(),
                kind: crate::RuleKind::RebasedSelectionStyleMap,
            });
        }
        let style_index = match (precise_pick_style_map, scatter_cfg) {
            (Some(map), Some(scatter)) if map.has_index => {
                let Some(column) = scatter.point_style_index_column.as_ref() else {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: cfg.series_id.clone(),
                        kind: crate::RuleKind::ScatterStyleMapIndex,
                    });
                };
                let h = lookup(column)?;
                let count = x_h.len_values.min(y_h.len_values);
                if h.len_values < count {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: cfg.series_id.clone(),
                        kind: crate::RuleKind::ScatterStyleIndexShort {
                            column: column.to_string(),
                            len: h.len_values as u64,
                            need: count as u64,
                        },
                    });
                }
                Some(h)
            }
            _ => None,
        };

        let (color_value, size_value, value_map_bg) = self.scatter_value_map_columns(
            cfg,
            precise_pick_style_map.is_some(),
            x_h.len_values.min(y_h.len_values),
            &mut lookup,
        )?;
        // The ring sits `radius_extra_px` outside the marker drawn at
        // the point: the shader measures that marker from its radius
        // and shape at the render scale (`ring_marker_extent_px`),
        // under the chart's draw style. A mapped ring resolves both
        // per point like the mapped marker. A line-only anchor has no
        // marker; a point vector's anchor is the glyph's half width.
        let mut ring_style = PrimitiveStyle::from_color_with_width(visual.color, visual.width_px);
        let uses_mapped_pick = precise_pick_style_map.is_some();
        ring_style.cap_half_px = visual.radius_extra_px;
        ring_style.series_salt = crate::sketch::fnv1a(&cfg.series_id);
        ring_style.primitive_flags =
            crate::gpu_pick::GpuPickDrawStyle::of(&chart_config.draw_style).marker_mode();
        if let Some(scatter) = scatter_cfg {
            // The draw's own base radius in both branches: the mapped
            // ring resolves overrides per point as `vs_mapped` does,
            // and an unmapped ring rings a draw that reads no
            // overrides (a stylized style, or none exist).
            ring_style.point_radius_px = series.style.scatter_radius_px;
            ring_style.shape_id = data_render::shape_id(&scatter.point_shape);
        } else {
            ring_style.point_radius_px = vector_points.map_or(0.0, |style| {
                crate::data_render::vector::vector_anchor_radius_px(style)
                    * series.style.display_scale
            });
            ring_style.shape_id = data_render::shape_id(&ScatterShape::CircleFilled);
            ring_style.primitive_flags = 0;
        }
        let ring_buf = data_render::create_style_uniform_buffer(&self.device, &ring_style);
        let style_bg =
            data_render::create_style_bind_group(&self.device, &self.style_bgl, &ring_buf);
        // A ternary point's ring: the ternary entries with its third
        // component.
        let ternary_c = rt.ternary_c_column().map(&mut lookup).transpose()?;
        let ternary = |kind: TernaryPipeline| {
            pipelines.ternary[kind as usize]
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured the ternary pick ring pipeline",
                ))
        };
        // A 3D point's ring: the 3D ring entries with its height and
        // camera (#132).
        let view3d_ring = |kind: View3dPipeline| {
            pipelines.view3d[kind as usize]
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured the 3D pick ring pipeline",
                ))
        };
        let (third, view3d, pipeline) = match &columns.view3d {
            Some((z, binding)) => (
                Some(*z),
                Some(binding.clone()),
                if uses_mapped_pick {
                    view3d_ring(View3dPipeline::RingMapped)?
                } else {
                    view3d_ring(View3dPipeline::Ring)?
                },
            ),
            None => (
                ternary_c,
                None,
                match (uses_mapped_pick, ternary_c.is_some()) {
                    (true, true) => ternary(TernaryPipeline::RingMapped)?,
                    (false, true) => ternary(TernaryPipeline::Ring)?,
                    (true, false) => {
                        pipelines
                            .pick_ring_mapped
                            .as_ref()
                            .ok_or(FiggyError::internal(
                                "prepare ensured mapped pick ring pipeline",
                            ))?
                    }
                    (false, false) => pipelines
                        .pick_ring
                        .as_ref()
                        .ok_or(FiggyError::internal("prepare ensured pick ring pipeline"))?,
                },
            ),
        };
        picked.push(ColumnPickRingLayer {
            pipeline,
            third,
            view3d,
            transform_bg: &view.transform_bg,
            style_bg,
            style_map_bg: precise_pick_style_map
                .map(|map| columns.mapped_point_bg.unwrap_or(&map.bind_group)),
            value_map_bg,
            quad_vb: &self.quad_vb,
            pool_buffer: column_buffer,
            x: x_h,
            y: y_h,
            style_index,
            color_value,
            size_value,
            instance,
        });
        Ok(())
    }
}

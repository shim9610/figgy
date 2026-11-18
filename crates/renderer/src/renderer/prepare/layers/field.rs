//! Field family layers: fields, meshes, surfaces, floors and contours.

use super::*;

impl PrepareContext<'_> {
    /// A series' field family: the flat field, a triangle mesh's fill and
    /// contours, a 3D chart's surface, its picked patch and floor, and the
    /// contour lines.
    pub(super) fn field_layers<'a>(&'a self, s: &SeriesInputs<'a, '_>) -> Result<FieldLayers<'a>> {
        let SeriesInputs {
            view,
            chart_config,
            pipelines,
            products,
            phase,
            idx,
            cfg,
            rt,
            primitives,
            ..
        } = *s;
        let series = &s.series;
        // A field: the group-2 state was resolved in `ensure_field_scratches`
        // (the prepare phase's `&mut self` half) and is reused whole while
        // nothing it depends on changes.
        let field = if primitives.field {
            let scratch = self
                .field_cache
                .get(cfg.series_id.as_str())
                .ok_or(FiggyError::internal("prepare ensured this field's scratch"))?;
            Some(data_render::ColumnFieldLayer {
                pipeline: pipelines
                    .field
                    .as_ref()
                    .ok_or(FiggyError::internal("prepare ensured field pipeline"))?,
                transform_bg: &view.transform_bg,
                style_bg: scratch.style_bg.clone(),
                field_bg: scratch.field_bg.clone(),
                charge: Arc::clone(&scratch.charge),
                drawable: scratch.drawable,
            })
        } else {
            None
        };

        // A triangle mesh (#210): its fill and its contours draw the one
        // group 2 `ensure_mesh_scratches` resolved, instancing the
        // triangle count (the shader rechecks every index on every draw).
        // On a 3D chart the fill is a surface into the panel composite
        // and the contours draw on the box floor, as a heatmap's do.
        let mut mesh = None;
        let mut mesh_contour = None;
        let mut mesh_surface = None;
        let mut mesh_selected = None;
        let mut mesh_floor = None;
        if (primitives.mesh || primitives.mesh_contour) && phase.is_none() {
            let scratch = self
                .field_cache
                .get(cfg.series_id.as_str())
                .ok_or(FiggyError::internal("prepare ensured this mesh's scratch"))?;
            let triangles = if scratch.drawable {
                scratch.signature.params.rows
            } else {
                0
            };
            if chart_config.view_3d().is_some() {
                if primitives.mesh {
                    let (surface, selected) = self.mesh_surface_layers(
                        view,
                        chart_config,
                        series,
                        pipelines,
                        scratch,
                        idx,
                    )?;
                    mesh_surface = surface;
                    mesh_selected = selected;
                }
                if primitives.mesh_contour {
                    mesh_floor = self
                        .mesh_floor_layer(view, chart_config, series, pipelines, scratch, idx)
                        .transpose()?;
                }
            }
            for (wanted, pass, slot) in [
                (
                    primitives.mesh,
                    data_render::tri_mesh::TriMeshPass::Fill,
                    &mut mesh,
                ),
                (
                    primitives.mesh_contour,
                    data_render::tri_mesh::TriMeshPass::Contour,
                    &mut mesh_contour,
                ),
            ] {
                if !wanted || chart_config.view_3d().is_some() {
                    continue;
                }
                *slot = Some(data_render::tri_mesh::ColumnTriMeshLayer {
                    pipeline: pipelines.tri_mesh[pass as usize]
                        .as_ref()
                        .ok_or(FiggyError::internal("prepare ensured the mesh pipeline"))?,
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    field_bg: scratch.field_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    triangles,
                });
            }
        }

        // A heatmap on a 3D chart draws as a surface into the panel
        // composite, not as the flat field: the same group 2, its own
        // camera and look as group 3.
        let (field, surface, selected_surface) = match field {
            Some(flat)
                if chart_config.view_3d().is_some()
                    && crate::renderer::device::draws_surface(rt) =>
            {
                let scratch = self
                    .field_cache
                    .get(cfg.series_id.as_str())
                    .ok_or(FiggyError::internal("prepare ensured this field's scratch"))?;
                let (surface, selected) =
                    self.surface_layers(view, chart_config, series, pipelines, scratch, idx, flat)?;
                (None, surface, selected)
            }
            other => (other, None, None),
        };
        let surface = surface.or(mesh_surface);
        let selected_surface = selected_surface.or(mesh_selected);

        // A contour on a 3D chart draws on the box floor
        // (`vs_floor`), never as flat isolines over the box.
        let floor_only = chart_config.view_3d().is_some();

        // Contour lines: the same quad the field draws, through
        // `fs_contour`. The group-2 tables were resolved in
        // `ensure_contour_scratches`; the isolines come out of the grid
        // itself, so there is nothing per-level about the draw call.
        // The floor's contour: the contour's own group 2 under the floor
        // pipeline, the panel camera as group 3.
        let floor = if primitives.contour && floor_only {
            let scratch = products
                .contours
                .get(idx)
                .and_then(|series| series.scratch.as_ref())
                .ok_or(FiggyError::internal(
                    "prepare captured this contour occurrence",
                ))?;
            self.floor_layer(view, chart_config, series, pipelines, scratch, idx)
                .transpose()?
        } else {
            None
        };
        let floor = floor.or(mesh_floor);

        let contour = if primitives.contour && !floor_only {
            let scratch = products
                .contours
                .get(idx)
                .and_then(|series| series.scratch.as_ref())
                .ok_or(FiggyError::internal(
                    "prepare captured this contour occurrence",
                ))?;
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
                    .ok_or(FiggyError::internal(
                        "prepare ensured labelled contour pipeline",
                    ))?
            } else {
                pipelines
                    .contour
                    .as_ref()
                    .ok_or(FiggyError::internal("prepare ensured contour pipeline"))?
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
        Ok(FieldLayers {
            field,
            mesh,
            mesh_contour,
            surface,
            selected_surface,
            floor,
            contour,
        })
    }
}

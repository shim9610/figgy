//! 3D surface and floor layers of heatmaps, meshes and contours.

use super::*;

impl PrepareContext<'_> {
    /// Convert one panel's `Series` list into `SeriesLayers` ready for
    /// drawing. The `config.render_type` enum variant decides which of
    /// line/scatter/errorbar each series needs; column ids are resolved to
    /// handles via the pool. `prepared` supplies prepare-time arc and star
    /// products without a live `arc_cache` read. `styled` is the panel's
    /// resolved [`StyleSet`] — it selects the stylized pipeline variants
    /// (and the line strip's vertex count); `None` (precise mode, or the
    /// theoretically-impossible cache miss) selects the precise pipelines.
    /// Everything else is identical between the modes.
    /// The surface of one heatmap on a 3D chart and, when the chart's
    /// selection names one of its cells, that sample's patch: the field's
    /// group 2 snapshot under the surface pipelines, with the camera of the
    /// panel and the surface's look as group 3.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::renderer) fn surface_layers<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series: &Series<'a>,
        pipelines: &'a TargetPipelines,
        scratch: &FieldScratch,
        index: usize,
        flat: data_render::ColumnFieldLayer<'a>,
    ) -> Result<(
        Option<data_render::surface_mesh::ColumnSurfaceLayer<'a>>,
        Option<data_render::surface_mesh::ColumnSurfaceLayer<'a>>,
    )> {
        use data_render::surface_mesh::{self as mesh, SurfacePass};
        let cfg = series.config;
        let panel = view.panel_rect;
        let target = (
            (f64::from(panel.x), f64::from(panel.y)),
            (f64::from(panel.width), f64::from(panel.height)),
        );
        let (Some(camera), Some(pipeline)) = (
            mesh::view3d_gpu(chart_config, target, target),
            pipelines.surface[SurfacePass::Draw as usize].as_ref(),
        ) else {
            return Ok((None, None));
        };
        let params = &scratch.signature.params;
        let columns_are_y = params.flags & data_render::FIELD_FLAG_COLUMNS_ARE_Y != 0;
        let (along, across) = if columns_are_y {
            (params.rows, params.cols)
        } else {
            (params.cols, params.rows)
        };
        let quads = if flat.drawable {
            mesh::surface_quad_count(along, across)
        } else {
            0
        };
        let style = cfg.render_type.surface_style();
        let color_base = params.cols;
        let scale = series.style.display_scale;
        let pick_id = u32::try_from(index)
            .unwrap_or(u32::MAX - 1)
            .saturating_add(1);
        let look = mesh::surface_params_gpu(style, color_base, scale, None, pick_id);
        let (surface_bg, surface_charge) = mesh::create_surface_bind_group(
            &self.gpu_ledger,
            &self.device,
            &self.surface_bgl,
            &camera,
            &look,
        )?;
        let surface = mesh::ColumnSurfaceLayer {
            pipeline,
            transform_bg: &view.transform_bg,
            style_bg: flat.style_bg.clone(),
            field_bg: flat.field_bg.clone(),
            surface_bg,
            vertices: mesh::SURFACE_QUAD_VERTICES,
            instances: quads,
            charge: Arc::clone(&flat.charge),
            surface_charge,
        };
        // The picked sample: the chart's matrix cell of this series, in
        // canonical x / y order, as the shader's constituent column and row.
        let picked = chart_config
            .picked_data
            .as_ref()
            .filter(|selection| selection.visible)
            .and_then(|selection| {
                selection.refs.iter().find_map(|picked| match picked {
                    PickedDataRef::MatrixCell {
                        series_id,
                        x_index,
                        y_index,
                        ..
                    } if *series_id == cfg.series_id => Some((selection, *x_index, *y_index)),
                    _ => None,
                })
            });
        let selected = match (
            picked,
            pipelines.surface[SurfacePass::Selected as usize].as_ref(),
        ) {
            (Some((selection, x, y)), Some(pipeline)) if quads > 0 => {
                let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
                    return Ok((Some(surface), None));
                };
                let (c, r) = if columns_are_y { (y, x) } else { (x, y) };
                let look =
                    mesh::surface_params_gpu(style, color_base, scale, Some((c, r)), pick_id);
                let (surface_bg, surface_charge) = mesh::create_surface_bind_group(
                    &self.gpu_ledger,
                    &self.device,
                    &self.surface_bgl,
                    &camera,
                    &look,
                )?;
                let highlight = data_render::create_style_uniform_buffer(
                    &self.device,
                    &PrimitiveStyle::from_color(selection.highlight_color),
                );
                let style_bg =
                    data_render::create_style_bind_group(&self.device, &self.style_bgl, &highlight);
                Some(mesh::ColumnSurfaceLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg,
                    field_bg: flat.field_bg.clone(),
                    surface_bg,
                    vertices: mesh::SURFACE_QUAD_VERTICES,
                    instances: mesh::SURFACE_SELECTED_INSTANCES,
                    charge: Arc::clone(&flat.charge),
                    surface_charge,
                })
            }
            _ => None,
        };
        Ok((Some(surface), selected))
    }

    /// A triangle mesh on a 3D chart (#210): its triangles as a surface
    /// under the mesh surface pipelines, the one group 2 its flat draw
    /// resolves (`ensure_mesh_scratches`, with the height column as the
    /// table's third entry), with the panel camera and the mesh's look as
    /// group 3; and, when the chart's selection names one of its triangles,
    /// that triangle as the picked patch.
    pub(in crate::renderer) fn mesh_surface_layers<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series: &Series<'a>,
        pipelines: &'a TargetPipelines,
        scratch: &FieldScratch,
        index: usize,
    ) -> Result<(
        Option<data_render::surface_mesh::ColumnSurfaceLayer<'a>>,
        Option<data_render::surface_mesh::ColumnSurfaceLayer<'a>>,
    )> {
        use data_render::surface_mesh::{self as mesh, SurfacePass};
        use data_render::tri_mesh::TRI_MESH_VERTICES;
        let cfg = series.config;
        let panel = view.panel_rect;
        let target = (
            (f64::from(panel.x), f64::from(panel.y)),
            (f64::from(panel.width), f64::from(panel.height)),
        );
        let (Some(camera), Some(pipeline)) = (
            mesh::view3d_gpu(chart_config, target, target),
            pipelines.tri_surface[SurfacePass::Draw as usize].as_ref(),
        ) else {
            return Ok((None, None));
        };
        let triangles = if scratch.drawable {
            scratch.signature.params.rows
        } else {
            0
        };
        let style = cfg.tri_mesh().and_then(|m| m.surface.as_ref());
        let scale = series.style.display_scale;
        let pick_id = u32::try_from(index)
            .unwrap_or(u32::MAX - 1)
            .saturating_add(1);
        let look = mesh::surface_params_gpu(style, 0, scale, None, pick_id);
        let (surface_bg, surface_charge) = mesh::create_surface_bind_group(
            &self.gpu_ledger,
            &self.device,
            &self.surface_bgl,
            &camera,
            &look,
        )?;
        let surface = mesh::ColumnSurfaceLayer {
            pipeline,
            transform_bg: &view.transform_bg,
            style_bg: scratch.style_bg.clone(),
            field_bg: scratch.field_bg.clone(),
            surface_bg,
            vertices: TRI_MESH_VERTICES,
            instances: triangles,
            charge: Arc::clone(&scratch.charge),
            surface_charge,
        };
        // The picked triangle of this series.
        let picked = chart_config
            .picked_data
            .as_ref()
            .filter(|selection| selection.visible)
            .and_then(|selection| {
                selection.refs.iter().find_map(|picked| match picked {
                    PickedDataRef::MeshTriangle {
                        series_id,
                        triangle_index,
                        ..
                    } if *series_id == cfg.series_id => Some((selection, *triangle_index)),
                    _ => None,
                })
            });
        let selected = match (
            picked,
            pipelines.tri_surface[SurfacePass::Selected as usize].as_ref(),
        ) {
            (Some((selection, triangle)), Some(pipeline)) if triangles > 0 => {
                let Ok(triangle) = u32::try_from(triangle) else {
                    return Ok((Some(surface), None));
                };
                if triangle >= triangles {
                    return Ok((Some(surface), None));
                }
                let look = mesh::surface_params_gpu(style, 0, scale, Some((triangle, 0)), pick_id);
                let (surface_bg, surface_charge) = mesh::create_surface_bind_group(
                    &self.gpu_ledger,
                    &self.device,
                    &self.surface_bgl,
                    &camera,
                    &look,
                )?;
                let highlight = data_render::create_style_uniform_buffer(
                    &self.device,
                    &PrimitiveStyle::from_color(selection.highlight_color),
                );
                let style_bg =
                    data_render::create_style_bind_group(&self.device, &self.style_bgl, &highlight);
                Some(mesh::ColumnSurfaceLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg,
                    field_bg: scratch.field_bg.clone(),
                    surface_bg,
                    vertices: TRI_MESH_VERTICES,
                    instances: 1,
                    charge: Arc::clone(&scratch.charge),
                    surface_charge,
                })
            }
            _ => None,
        };
        Ok((Some(surface), selected))
    }

    /// A triangle mesh's contours on a 3D chart's floor (#210): every
    /// triangle under the mesh floor pipeline (`vs_tri_floor` /
    /// `fs_tri_floor`), the mesh's group 2, the panel camera as group 3.
    pub(in crate::renderer) fn mesh_floor_layer<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series: &Series<'a>,
        pipelines: &'a TargetPipelines,
        scratch: &FieldScratch,
        index: usize,
    ) -> Option<Result<data_render::surface_mesh::ColumnSurfaceLayer<'a>>> {
        use data_render::surface_mesh::{self as mesh, SurfacePass};
        if !scratch.drawable {
            return None;
        }
        let panel = view.panel_rect;
        let target = (
            (f64::from(panel.x), f64::from(panel.y)),
            (f64::from(panel.width), f64::from(panel.height)),
        );
        let camera = mesh::view3d_gpu(chart_config, target, target)?;
        let pipeline = pipelines.tri_surface[SurfacePass::FloorContour as usize].as_ref()?;
        let pick_id = u32::try_from(index)
            .unwrap_or(u32::MAX - 1)
            .saturating_add(1);
        let look = mesh::surface_params_gpu(None, 0, series.style.display_scale, None, pick_id);
        let (surface_bg, surface_charge) = match mesh::create_surface_bind_group(
            &self.gpu_ledger,
            &self.device,
            &self.surface_bgl,
            &camera,
            &look,
        ) {
            Ok(group) => group,
            Err(error) => return Some(Err(error)),
        };
        Some(Ok(mesh::ColumnSurfaceLayer {
            pipeline,
            transform_bg: &view.transform_bg,
            style_bg: scratch.style_bg.clone(),
            field_bg: scratch.field_bg.clone(),
            surface_bg,
            vertices: data_render::tri_mesh::TRI_MESH_VERTICES,
            instances: scratch.signature.params.rows,
            charge: Arc::clone(&scratch.charge),
            surface_charge,
        }))
    }

    /// A contour on a 3D chart's floor: the contour's group 2 under the
    /// floor pipeline (`vs_floor` / `fs_contour`), one instance, with the
    /// panel camera as group 3.
    pub(in crate::renderer) fn floor_layer<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series: &Series<'a>,
        pipelines: &'a TargetPipelines,
        scratch: &PreparedContourScratch,
        index: usize,
    ) -> Option<Result<data_render::surface_mesh::ColumnSurfaceLayer<'a>>> {
        use data_render::surface_mesh::{self as mesh, SurfacePass};
        if !scratch.drawable {
            return None;
        }
        let panel = view.panel_rect;
        let target = (
            (f64::from(panel.x), f64::from(panel.y)),
            (f64::from(panel.width), f64::from(panel.height)),
        );
        let camera = mesh::view3d_gpu(chart_config, target, target)?;
        let pipeline = pipelines.surface[SurfacePass::FloorContour as usize].as_ref()?;
        let pick_id = u32::try_from(index)
            .unwrap_or(u32::MAX - 1)
            .saturating_add(1);
        let look = mesh::surface_params_gpu(None, 0, series.style.display_scale, None, pick_id);
        let (surface_bg, surface_charge) = match mesh::create_surface_bind_group(
            &self.gpu_ledger,
            &self.device,
            &self.surface_bgl,
            &camera,
            &look,
        ) {
            Ok(group) => group,
            Err(error) => return Some(Err(error)),
        };
        Some(Ok(mesh::ColumnSurfaceLayer {
            pipeline,
            transform_bg: &view.transform_bg,
            style_bg: scratch.style_bg.clone(),
            field_bg: scratch.field_bg.clone(),
            surface_bg,
            vertices: mesh::SURFACE_QUAD_VERTICES,
            instances: 1,
            charge: Arc::clone(&scratch.charge),
            surface_charge,
        }))
    }
}

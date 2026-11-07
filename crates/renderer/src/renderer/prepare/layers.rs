//! Series layer building: build_series_layers and its per-kind arms.

use super::*;

/// The region params a ternary series' draw and its pick scan over: every
/// column's f32 lane and the rows all three columns hold.
fn ternary_region_params(
    series_id: &str,
    color_premul: [f32; 4],
    x: ColumnHandle,
    y: ColumnHandle,
    c: ColumnHandle,
) -> Result<data_render::ternary_region::TernaryRegionParamsGpu> {
    let invalid = |reason: &str| FiggyError::InvalidSeriesConfig {
        series_id: series_id.to_owned(),
        reason: reason.into(),
    };
    let lane = |handle: ColumnHandle| -> Result<u32> {
        if !handle.offset.is_multiple_of(4) {
            return Err(invalid("region column byte offset is not f32-aligned"));
        }
        u32::try_from(handle.offset / 4)
            .map_err(|_| invalid("region column lies past the 32-bit lane range"))
    };
    let count = x.len_values.min(y.len_values).min(c.len_values);
    Ok(data_render::ternary_region::TernaryRegionParamsGpu {
        color_premul,
        x_base: lane(x)?,
        y_base: lane(y)?,
        c_base: lane(c)?,
        count: u32::try_from(count)
            .map_err(|_| invalid("region rows exceed the 32-bit row range"))?,
    })
}

fn ring_scan_too_large(series_id: &str) -> FiggyError {
    FiggyError::InvalidSeriesConfig {
        series_id: series_id.to_owned(),
        reason: "the region's rows exceed the adapter's dispatch grid".into(),
    }
}

impl PrepareContext<'_> {
    /// A region series' ring starts for the pick (#49): the scan its draw
    /// runs, over the same rows. `None` for any other series, or when its
    /// columns are not resident; a scan the draw would fail fails the pick
    /// with the same error.
    pub(in crate::renderer) fn region_ring_starts(
        &mut self,
        config: &Config,
        series: &SeriesConfig,
    ) -> Result<Option<wgpu::Buffer>> {
        let (Some(_), Some(_), Some(c)) = (
            series.render_type.ternary_region(),
            config.ternary(),
            series.render_type.ternary_c_column(),
        ) else {
            return Ok(None);
        };
        let (Some(x), Some(y), Some(c)) = (
            self.pool.handle_for(&series.x_column),
            self.pool.handle_for(&series.y_column),
            self.pool.handle_for(c),
        ) else {
            return Ok(None);
        };
        let params = ternary_region_params(&series.series_id, [0.0; 4], x, y, c)?;
        let scan = self.pipelines.ensure_ternary_region_scan(self.device);
        let (buffer, _charge) = data_render::ternary_region::scan_ring_starts(
            self.device,
            self.queue,
            self.gpu_ledger,
            &scan,
            self.pool.buffer(),
            &params,
            self.caps.max_compute_workgroups_per_dimension,
        )?
        .ok_or_else(|| ring_scan_too_large(&series.series_id))?;
        Ok(Some(buffer))
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

    pub(in crate::renderer) fn build_series_layers<'a>(
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

    /// Column locations and their lifetime authority are resolved by the caller;
    /// geometry and style interpretation remain shared with resident rendering.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::renderer) fn build_series_layers_with_columns<'a>(
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
        // The stem pipeline (#206) per draw style: the precise and sketch
        // stem entries, a Milkyway jet (its `vs_jet` stem branch), none
        // under Constellation, which drops a stem series.
        let stem_pipe = match styled {
            None => pipelines.errorbar_stem.as_ref(),
            Some(StyleSet::Sketch { stem, .. }) => Some(stem),
            Some(StyleSet::Milkyway(c)) => Some(&c.jets),
            Some(StyleSet::Constellation(_)) => None,
        };
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
                ..
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

        // A 3D chart's camera for this panel (#132): the scatter and line
        // series with a height column draw through the 3D entries with it.
        let camera_3d = chart_config.view_3d().and_then(|_| {
            let panel = view.panel_rect;
            let target = (
                (f64::from(panel.x), f64::from(panel.y)),
                (f64::from(panel.width), f64::from(panel.height)),
            );
            data_render::surface_mesh::view3d_gpu(chart_config, target, target)
        });
        let mut out = Vec::with_capacity(series_specs.len());
        for (idx, series) in series_specs.iter().enumerate() {
            let cfg = series.config;
            let rt = &cfg.render_type;
            let constellation_only = matches!(styled, Some(StyleSet::Constellation(_)));
            let primitives = effective_series_primitives(&chart_config.draw_style, rt);
            // A DataLess series (energy levels, a Feynman diagram) reads no
            // column: its only layer is the CPU raster this prepare resolved
            // (#203, #234). A streamed phase never draws one (rule 8).
            if cfg.is_dataless() {
                let energy_levels = products
                    .contours
                    .get(idx)
                    .and_then(|series| series.energy_levels.as_ref())
                    .filter(|_| phase.is_none())
                    .map(|raster| data_render::AxisLayer {
                        pipeline: &pipelines.axis,
                        bind_group: &raster.layer.bind_group,
                    });
                out.push(data_render::SeriesLayers {
                    energy_levels,
                    ..data_render::SeriesLayers::empty()
                });
                continue;
            }
            let x_h = lookup(&cfg.x_column)?;
            // The height column of a scatter or line series on a 3D chart
            // (#132), and its camera group: the surface layout for the
            // precise entries, the combined value map and camera layout for
            // the mapped markers and their ring.
            // A point vector names its height too (#211): its arrows and its
            // picked anchor's ring draw through the camera.
            let z_3d = match (camera_3d.as_ref(), cfg.z_column.as_deref()) {
                (Some(_), Some(z))
                    if (primitives.line || primitives.scatter || primitives.vector)
                        && phase.is_none() =>
                {
                    Some(lookup(z)?)
                }
                _ => None,
            };
            let camera_binding = |mapped: bool| -> Result<_> {
                data_render::surface_mesh::create_view3d_binding(
                    &self.gpu_ledger,
                    &self.device,
                    if mapped {
                        self.view3d_value_map_bgl
                    } else {
                        self.surface_bgl
                    },
                    camera_3d
                        .as_ref()
                        .ok_or(FiggyError::internal("a 3D series has its panel's camera"))?,
                    series.style.display_scale,
                    0,
                    self.scatter_value_map_buffers(cfg, mapped),
                )
            };
            // A box plot's `y_column` is its record, not a column: its
            // member columns are looked up with its params below.
            let y_h = if rt.box_plot().is_some() {
                x_h
            } else {
                lookup(&cfg.y_column)?
            };
            // A radar polygon has one vertex per category: its columns are
            // data, so their lengths are checked here against the pool.
            if rt.radar().is_some() {
                radar_columns_fit(chart_config, cfg, x_h.len_values, y_h.len_values)?;
            }

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
                    let (surface, selected) = self.surface_layers(
                        view,
                        chart_config,
                        series,
                        pipelines,
                        scratch,
                        idx,
                        flat,
                    )?;
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

            // Arrows: the group-2 state was resolved in `ensure_vector_scratches`
            // and is reused whole while nothing it depends on changes.
            let vector =
                if primitives.vector && phase.is_none() {
                    let scratch = self.vector_cache.get(cfg.series_id.as_str()).ok_or(
                        FiggyError::internal("prepare ensured this vector's scratch"),
                    )?;
                    let pipeline = match scratch.input {
                        data_render::vector::VectorInput::Points => {
                            pipelines.vector_points.as_ref()
                        }
                        data_render::vector::VectorInput::Grid => pipelines.vector_grid.as_ref(),
                        data_render::vector::VectorInput::Path => pipelines.vector_path.as_ref(),
                        data_render::vector::VectorInput::Points3d => {
                            pipelines.view3d[View3dPipeline::Vector as usize].as_ref()
                        }
                    }
                    .ok_or(FiggyError::internal("prepare ensured vector pipeline"))?;
                    // A 3D chart's arrows (#211) bind the panel's camera.
                    let view3d = (scratch.input == data_render::vector::VectorInput::Points3d)
                        .then(|| camera_binding(false))
                        .transpose()?;
                    Some(data_render::vector::ColumnVectorLayer {
                        pipeline,
                        transform_bg: &view.transform_bg,
                        style_bg: scratch.style_bg.clone(),
                        vector_bg: scratch.vector_bg.clone(),
                        charge: Arc::clone(&scratch.charge),
                        count: scratch.count,
                        indirect: None,
                        view3d,
                    })
                } else if primitives.line
                    && phase.is_none()
                    && rt
                        .streamlines()
                        .is_some_and(|streamlines| streamlines.arrows.is_some())
                {
                    // Streamline arrowheads: the vector glyph on the arc slots of
                    // the line's arc scan, drawn indirect in the vector slot,
                    // under the stroke (#51, D5). A series too short to scan has
                    // no slots and draws no heads.
                    let scratch = self.vector_cache.get(cfg.series_id.as_str()).ok_or(
                        FiggyError::internal("prepare ensured this streamline's arrowhead scratch"),
                    )?;
                    let pipeline = pipelines.vector_path.as_ref().ok_or(FiggyError::internal(
                        "prepare ensured the path vector pipeline",
                    ))?;
                    products
                        .arcs
                        .get(idx)
                        .and_then(|series| series.arc.as_ref())
                        .and_then(|arc| arc.star.as_ref())
                        .map(|slots| data_render::vector::ColumnVectorLayer {
                            pipeline,
                            transform_bg: &view.transform_bg,
                            style_bg: scratch.style_bg.clone(),
                            vector_bg: scratch.vector_bg.clone(),
                            charge: Arc::clone(&scratch.charge),
                            count: scratch.count,
                            indirect: Some((&slots.indirect, &slots.vs_bg)),
                            view3d: None,
                        })
                } else {
                    None
                };

            // Category bars: the group is this panel's category bar series;
            // the params and the override table are resolved against the
            // panel's categorical axis on every prepare, so a reordered axis
            // or an edited override is drawn at once. No streamed form.
            let category_bars = if primitives.category_bars && phase.is_none() {
                let bars = rt.category_bars().ok_or(FiggyError::internal(
                    "the category bar primitive has a category bar config",
                ))?;
                let group = series_specs
                    .iter()
                    .filter(|s| {
                        effective_series_primitives(&chart_config.draw_style, &s.config.render_type)
                            .category_bars
                    })
                    .map(|s| s.config);
                let params = category_bar_params(
                    chart_config,
                    group.clone(),
                    cfg,
                    series.style.display_scale,
                    &mut lookup,
                    |id| self.pool.slot(id).map(|slot| slot.min),
                )?;
                let style_map = crate::renderer::style::category_bar_style_map(
                    &self.device,
                    self.per_point_style_map_bgl,
                    category_bar_categories(chart_config).unwrap_or(&[]),
                    bars,
                    series.style.display_scale,
                )?;
                let data = data_render::create_category_bar_data_bind_group(
                    &self.device,
                    self.gpu_ledger,
                    self.category_bar_data_bgl,
                    column_buffer,
                    &params,
                    params.members as usize,
                    |push| category_bar_lanes(chart_config, group, cfg, &mut lookup, push),
                    style_map,
                )?;
                Some((
                    data,
                    params.count,
                    data_render::CATEGORY_BAR_VERTICES,
                    pipelines.bar_category.as_ref().ok_or(FiggyError::internal(
                        "prepare ensured category bar pipeline",
                    ))?,
                ))
            } else if primitives.box_plot && phase.is_none() {
                // Box plots: the group is this panel's box plot series; the
                // params read the record's and the outlier column's member
                // lanes, and the style map resolves the overrides against the
                // panel's categorical axis on every prepare. No streamed form.
                let boxes = rt.box_plot().ok_or(FiggyError::internal(
                    "the box plot primitive has a box plot config",
                ))?;
                let (params, lanes, vertices) = box_plot_params(
                    chart_config,
                    series_specs
                        .iter()
                        .filter(|s| {
                            effective_series_primitives(
                                &chart_config.draw_style,
                                &s.config.render_type,
                            )
                            .box_plot
                        })
                        .map(|s| s.config),
                    cfg,
                    series.style.display_scale,
                    self.pool,
                    &mut lookup,
                )?;
                let style_map = crate::renderer::style::box_plot_style_map(
                    &self.device,
                    self.per_point_style_map_bgl,
                    category_bar_categories(chart_config).unwrap_or(&[]),
                    boxes,
                    series.style.display_scale,
                )?;
                let data = data_render::create_category_bar_data_bind_group(
                    &self.device,
                    self.gpu_ledger,
                    self.category_bar_data_bgl,
                    column_buffer,
                    &params,
                    lanes.len(),
                    |push| {
                        lanes.iter().for_each(|&lane| push(lane));
                        Ok(())
                    },
                    style_map,
                )?;
                Some((
                    data,
                    params.count,
                    vertices,
                    pipelines
                        .bar_box_plot
                        .as_ref()
                        .ok_or(FiggyError::internal("prepare ensured box plot pipeline"))?,
                ))
            } else if primitives.range_bars && phase.is_none() {
                // Range bars: the group is this panel's range bar series; the
                // params read every column's lane and the style map holds the
                // part colours. No streamed form.
                let (data, count) = range_bar_data(
                    &self.device,
                    self.gpu_ledger,
                    self.category_bar_data_bgl,
                    self.per_point_style_map_bgl,
                    column_buffer,
                    chart_config,
                    series_specs
                        .iter()
                        .filter(|s| {
                            effective_series_primitives(
                                &chart_config.draw_style,
                                &s.config.render_type,
                            )
                            .range_bars
                        })
                        .map(|s| s.config),
                    cfg,
                    series.style.display_scale,
                    &mut lookup,
                )?;
                Some((
                    data,
                    count,
                    data_render::RANGE_BAR_VERTICES,
                    pipelines
                        .bar_range
                        .as_ref()
                        .ok_or(FiggyError::internal("prepare ensured range bar pipeline"))?,
                ))
            } else {
                None
            };
            // A box plot's violins and raw points: their own group 3 over the
            // box group's layout, under and over its boxes. No streamed form.
            let box_group = || {
                series_specs
                    .iter()
                    .filter(|s| {
                        effective_series_primitives(&chart_config.draw_style, &s.config.render_type)
                            .box_plot
                    })
                    .map(|s| s.config)
            };
            let mut box_extra = |points: bool, pipeline: Option<&'a wgpu::RenderPipeline>| {
                box_extras_data(
                    &self.device,
                    self.gpu_ledger,
                    self.category_bar_data_bgl,
                    self.per_point_style_map_bgl,
                    column_buffer,
                    self.pool,
                    chart_config,
                    box_group(),
                    cfg,
                    series.style.display_scale,
                    &mut lookup,
                    points,
                )
                .and_then(|(data, count)| {
                    Ok(data_render::ColumnCategoryBarLayer {
                        pipeline: pipeline
                            .ok_or(FiggyError::internal("prepare ensured the violin pipelines"))?,
                        transform_bg: &view.transform_bg,
                        style_bg: &series.style.bar_bg,
                        data,
                        count,
                        vertices: data_render::VIOLIN_VERTICES,
                    })
                })
            };
            let violin = (primitives.violin && phase.is_none())
                .then(|| box_extra(false, pipelines.bar_violin.as_ref()))
                .transpose()?;
            let box_points = (primitives.box_points && phase.is_none())
                .then(|| box_extra(true, pipelines.bar_box_points.as_ref()))
                .transpose()?;

            // Bars: the edge column and the value column are picked from the
            // bar's own orientation — never from the column lengths (design
            // §B.3). `Vertical` reads x as edges, `Horizontal` reads y.
            let bar = if primitives.bar
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Histogram)
            {
                let bar_cfg = extract_bar(rt)
                    .ok_or(FiggyError::internal("primitives.bar implies a bar config"))?;
                let (edges, values) = match bar_cfg.orientation {
                    crate::data_config::BarOrientation::Vertical => (x_h, y_h),
                    crate::data_config::BarOrientation::Horizontal => (y_h, x_h),
                };
                let style_map = series.style.bar_map.as_ref();
                Some(data_render::ColumnBarLayer {
                    envelope: if phase.is_none() {
                        Some(
                            pipelines
                                .bar_envelope
                                .as_ref()
                                .ok_or(FiggyError::internal(
                                    "prepare ensured histogram envelope pipeline",
                                ))?
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
                                )?,
                        )
                    } else {
                        None
                    },
                    pipeline: match style_map {
                        None => pipelines
                            .bar
                            .as_ref()
                            .ok_or(FiggyError::internal("prepare ensured bar pipeline"))?,
                        Some(_) => pipelines
                            .bar_mapped
                            .as_ref()
                            .ok_or(FiggyError::internal("prepare ensured mapped bar pipeline"))?,
                    },
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
            // A radar polygon closes its outline, and its fill on a full
            // circle (in a sector the closing trapezoid would cover the
            // others).
            let closed = rt.radar().is_some();
            let closed_fill = closed && polar_fill_closes(chart_config);
            let line = if primitives.line
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Line)
            {
                let line_pick =
                    line_pick.ok_or(FiggyError::internal("prepare ensured line pipeline"))?;
                let arc = products
                    .arcs
                    .get(idx)
                    .and_then(|series| series.arc.as_ref())
                    .map(|arc| arc.prefix.clone());
                // A streamline series' colour column: its stroke draws through
                // the mapped line pipeline with the value map
                // `ensure_value_map_scratch` resolved (precise only, never
                // streamed).
                let value_map = match rt.streamlines().and_then(|s| s.color_map) {
                    Some(map) if styled.is_none() && phase.is_none() => {
                        let scratch = self.value_map_cache.get(cfg.series_id.as_str()).ok_or(
                            FiggyError::internal(
                                "prepare ensured this streamline colour map's value map",
                            ),
                        )?;
                        Some(data_render::LineValueMap {
                            column: lookup(&map.column)?,
                            neutral_bg: self.neutral_point_value_map_bg,
                            value_map_bg: &scratch.bind_group,
                        })
                    }
                    _ => None,
                };
                // A ternary series' stroke: the ternary entry with its third
                // component (never mapped, never styled).
                let ternary_c = rt.ternary_c_column().map(&mut lookup).transpose()?;
                // A 3D series' stroke: the 3D entry with its height and the
                // panel's camera (#132).
                let line_3d = z_3d
                    .map(|z| camera_binding(false).map(|binding| (z, binding)))
                    .transpose()?;
                Some(ColumnLineLayer {
                    pipeline: if line_3d.is_some() {
                        pipelines.view3d[View3dPipeline::Line as usize]
                            .as_ref()
                            .ok_or(FiggyError::internal("prepare ensured the 3D line pipeline"))?
                    } else if value_map.is_some() {
                        pipelines
                            .line_mapped
                            .as_ref()
                            .ok_or(FiggyError::internal("prepare ensured mapped line pipeline"))?
                    } else if ternary_c.is_some() {
                        pipelines.ternary[TernaryPipeline::Line as usize]
                            .as_ref()
                            .ok_or(FiggyError::internal(
                                "prepare ensured the ternary line pipeline",
                            ))?
                    } else {
                        line_pick.pipeline
                    },
                    third: line_3d.as_ref().map(|(z, _)| *z).or(ternary_c),
                    view3d: line_3d.map(|(_, binding)| binding),
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
                            crate::line_step::arc_pieces(chart_config.polar()),
                        ),
                    ),
                    texture_bg: line_pick.texture_bg,
                    closed,
                    value_map,
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
                && let Some(line_config) = extract_line(rt)
                && let Some(fill) = line_config.fill.as_ref()
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
                        .ok_or(FiggyError::internal("prepare ensured line area pipeline"))?,
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
                            crate::line_step::arc_pieces(chart_config.polar()),
                        ),
                    ),
                    closed: closed_fill,
                })
            } else {
                None
            };

            // A ternary series' region fill: the closed polygons its samples
            // outline, drawn into the composite's stencil attachment before
            // every series' fill and body. Resident only (streamed ternary
            // sources are rejected).
            let region = match (rt.ternary_region(), rt.ternary_c_column()) {
                (Some(style), Some(c)) if chart_config.ternary().is_some() && phase.is_none() => {
                    let c_h = lookup(c)?;
                    let params =
                        ternary_region_params(&cfg.series_id, style.color_premul(), x_h, y_h, c_h)?;
                    // Each ring's first row, scanned before the draw reads it
                    // for the closing edges (#49).
                    let scan =
                        pipelines
                            .ternary_region_scan
                            .as_ref()
                            .ok_or(FiggyError::internal(
                                "prepare ensured the ternary region pipelines",
                            ))?;
                    let (ring_starts, ring_charge) = data_render::ternary_region::scan_ring_starts(
                        &self.device,
                        &self.queue,
                        &self.gpu_ledger,
                        scan,
                        column_buffer,
                        &params,
                        self.caps.max_compute_workgroups_per_dimension,
                    )?
                    .ok_or_else(|| ring_scan_too_large(&cfg.series_id))?;
                    let (region_bg, charge) =
                        data_render::ternary_region::create_ternary_region_bind_group(
                            &self.gpu_ledger,
                            &self.device,
                            &self.region_bgl,
                            column_buffer,
                            &params,
                            &ring_starts,
                        )?;
                    let pipeline = |pass: data_render::ternary_region::TernaryRegionPass| {
                        pipelines.ternary_region[pass as usize].as_ref().ok_or(
                            FiggyError::internal("prepare ensured the ternary region pipelines"),
                        )
                    };
                    Some(data_render::ternary_region::ColumnTernaryRegionLayer {
                        stencil: pipeline(data_render::ternary_region::TernaryRegionPass::Stencil)?,
                        cover: pipeline(data_render::ternary_region::TernaryRegionPass::Cover)?,
                        transform_bg: &view.transform_bg,
                        region_bg,
                        count: params.count,
                        charge,
                        ring_charge,
                    })
                }
                _ => None,
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
                let style_index = match (precise_style_map, extract_scatter(rt).as_deref()) {
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
                let scatter_pick =
                    scatter_pick.ok_or(FiggyError::internal("prepare ensured scatter pipeline"))?;
                // A ternary series' markers: the ternary entries with its
                // third component.
                let ternary_c = rt.ternary_c_column().map(&mut lookup).transpose()?;
                let ternary = |kind: TernaryPipeline| {
                    pipelines.ternary[kind as usize]
                        .as_ref()
                        .ok_or(FiggyError::internal(
                            "prepare ensured the ternary scatter pipeline",
                        ))
                };
                // A 3D series' markers: the 3D entries with its height and
                // the panel's camera, beside the value map when mapped
                // (#132).
                let view3d = |kind: View3dPipeline| {
                    pipelines.view3d[kind as usize]
                        .as_ref()
                        .ok_or(FiggyError::internal(
                            "prepare ensured the 3D scatter pipeline",
                        ))
                };
                let scatter_3d = z_3d
                    .map(|z| {
                        camera_binding(precise_style_map.is_some()).map(|binding| (z, binding))
                    })
                    .transpose()?;
                Some(ColumnScatterLayer {
                    pipeline: match (precise_style_map, ternary_c, scatter_3d.is_some()) {
                        (Some(_), _, true) => view3d(View3dPipeline::ScatterMapped)?,
                        (None, _, true) => view3d(View3dPipeline::Scatter)?,
                        (Some(_), Some(_), false) => ternary(TernaryPipeline::ScatterMapped)?,
                        (None, Some(_), false) => ternary(TernaryPipeline::Scatter)?,
                        (Some(_), None, false) => {
                            pipelines
                                .scatter_mapped
                                .as_ref()
                                .ok_or(FiggyError::internal(
                                    "prepare ensured mapped scatter pipeline",
                                ))?
                        }
                        (None, None, false) => scatter_pick.pipeline,
                    },
                    third: scatter_3d.as_ref().map(|(z, _)| *z).or(ternary_c),
                    view3d: scatter_3d.map(|(_, binding)| binding),
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
                            pipeline: match precise_errorbar_style_map {
                                None => errorbar_pipe.ok_or(FiggyError::internal(
                                    "prepare ensured errorbar pipeline",
                                ))?,
                                Some(_) => pipelines.errorbar_mapped.as_ref().ok_or(
                                    FiggyError::internal(
                                        "prepare ensured mapped errorbar pipeline",
                                    ),
                                )?,
                            },
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

            // The stems (#206): the error-bar module's second draw, the
            // error slots bound to the anchor columns as filler (the stem
            // reads only x and y). Every row is drawn (rule 10). A streamed
            // phase never draws one (rule 8: refused before a cursor).
            let stem = match (series.style.stem_bg.as_ref(), phase) {
                (Some(stem_bg), None) if !constellation_only && primitives.scatter => {
                    Some(ColumnErrorBarDraw {
                        pipeline: stem_pipe
                            .ok_or(FiggyError::internal("prepare ensured stem pipeline"))?,
                        transform_bg: &view.transform_bg,
                        style_bg: stem_bg,
                        style_map_bg: None,
                        pool_buffer: column_buffer,
                        x: x_h,
                        y: y_h,
                        err_y_lo: y_h,
                        err_y_hi: y_h,
                        err_x_lo: x_h,
                        err_x_hi: x_h,
                        style_index: None,
                    })
                }
                _ => None,
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
                        category: category_bars.as_ref().map(|(data, ..)| data.clone()),
                        violin: violin
                            .as_ref()
                            .map(|layer| (layer.data.clone(), layer.count)),
                        box_points: box_points.as_ref().map(|layer| layer.data.clone()),
                        rows: selection_prepare::SelectionRows::RESIDENT,
                        mapped_point_bg: None,
                        view3d: z_3d
                            .filter(|_| primitives.scatter || primitives.vector)
                            .map(|z| {
                                camera_binding(
                                    styled.is_none() && series.style.scatter_map.is_some(),
                                )
                                .map(|binding| (z, binding))
                            })
                            .transpose()?,
                    },
                    products
                        .contours
                        .get(idx)
                        .and_then(|series| series.scratch.as_ref()),
                    styled.is_none(),
                    &mut lookup,
                )?
            };

            // Value labels: the atlas this prepare resolved for the series,
            // placed over the series' own category bar group 3 (a bar or
            // box series') or its category and value columns (a radar
            // series'), with its uniform at the style's display scale. Only
            // a series that draws its bars, boxes or polygon draws labels.
            let atlas = products
                .contours
                .get(idx)
                .and_then(|series| series.value_labels.as_ref());
            let value_labels = match atlas {
                Some(atlas) if phase.is_none() => {
                    let data = match &category_bars {
                        Some((data, ..)) => Some(data.clone()),
                        None if rt.radar().is_some() && primitives.line => {
                            Some(self.radar_label_group(
                                cfg,
                                column_buffer,
                                series.style.display_scale,
                                &mut lookup,
                            )?)
                        }
                        None => None,
                    };
                    match (
                        data,
                        self.value_label_data(
                            chart_config,
                            cfg,
                            atlas,
                            series.style.display_scale,
                        )?,
                        pipelines.bar_value_label.as_ref(),
                    ) {
                        (Some(data), Some(labels), Some(pipeline)) => {
                            Some(data_render::ColumnValueLabelLayer {
                                pipeline,
                                transform_bg: &view.transform_bg,
                                labels,
                                data,
                                selected: selected_value_label(chart_config, cfg)
                                    .zip(pipelines.bar_value_label_selection.as_ref()),
                            })
                        }
                        _ => None,
                    }
                }
                _ => None,
            };

            let category_bars = category_bars.map(|(data, count, vertices, pipeline)| {
                data_render::ColumnCategoryBarLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.bar_bg,
                    data,
                    count,
                    vertices,
                }
            });
            // A parallel coordinates series' rows (#141): the axis lane and
            // range tables over the chart's parallel axes, the rows coloured
            // through the value map `ensure_value_map_scratch` resolved when
            // a colour column is named. Resident and precise only; the
            // picked row (a point selection of this series) is redrawn in
            // the selection's style over every body.
            let parallel = if primitives.parallel && phase.is_none() {
                let value_map = match rt.parallel().and_then(|p| p.color_column) {
                    Some(_) => {
                        &self
                            .value_map_cache
                            .get(cfg.series_id.as_str())
                            .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: "a colour-mapped parallel series needs its value map \
                                         prepared; recreate the chart's preparation"
                                    .into(),
                            })?
                            .buffers
                    }
                    None => self.neutral_point_value_map_buffers,
                };
                let data = build_parallel_data(
                    &self.device,
                    self.gpu_ledger,
                    self.parallel_bgl,
                    column_buffer,
                    value_map,
                    chart_config,
                    cfg,
                    &mut lookup,
                )?;
                let selected = selection_prepare::point_selection_visuals(
                    chart_config,
                    cfg,
                    selection_prepare::SelectionRows::RESIDENT.refs,
                )
                .into_iter()
                .next()
                .zip(pipelines.line_parallel_selection.as_ref())
                .and_then(|(visual, pipeline)| {
                    let row = u32::try_from(visual.point_index).ok()?;
                    let style = PrimitiveStyle::from_color_with_width(
                        visual.color,
                        visual.width_px * series.style.display_scale,
                    );
                    let buffer = data_render::create_style_uniform_buffer(&self.device, &style);
                    Some(data_render::ParallelSelection {
                        pipeline,
                        style_bg: data_render::create_style_bind_group(
                            &self.device,
                            self.style_bgl,
                            &buffer,
                        ),
                        row,
                    })
                });
                Some(data_render::ColumnParallelLayer {
                    pipeline: pipelines
                        .line_parallel
                        .as_ref()
                        .ok_or(FiggyError::internal(
                            "prepare ensured the parallel pipeline",
                        ))?,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.line_bg,
                    data,
                    selected,
                })
            } else {
                None
            };
            // A 3D series' line and markers draw with the surfaces, not
            // among the flat bodies.
            let (line, line_3d) = match z_3d {
                Some(_) => (None, line),
                None => (line, None),
            };
            let (scatter, scatter_3d) = match z_3d {
                Some(_) => (None, scatter),
                None => (scatter, None),
            };
            let (vector, vector_3d) = match &vector {
                Some(layer) if layer.view3d.is_some() => (None, vector),
                _ => (vector, None),
            };
            out.push(data_render::SeriesLayers {
                area,
                field,
                mesh,
                mesh_contour,
                bar,
                violin,
                category_bars,
                box_points,
                contour,
                vector,
                errorbar,
                stem,
                line,
                parallel,
                line_extra,
                scatter,
                line_3d,
                scatter_3d,
                vector_3d,
                selected_bars,
                selected_fields,
                picked,
                value_labels,
                energy_levels: None,
                surface,
                selected_surface,
                floor,
                region,
            });
        }
        Ok(out)
    }
}

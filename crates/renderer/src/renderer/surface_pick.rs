//! The ID pick of a 3D chart (#49 D3): the panel's surfaces drawn once more
//! into a 1×1 `Rgba32Uint` target with a 1×1 depth-stencil, through the
//! draw's own vertex entry and depth rule, with the camera premultiplied by
//! a matrix that maps the cursor pixel onto the target. The pixel the
//! cursor is over picks exactly what it shows: the nearest quad's nearest
//! grid sample, hidden quads never.

use super::*;

/// The GPU style of one series a 3D point pick draws with (#132): the
/// marker and stroke styles the draw bound, cloned from the chart's
/// prepared styles so the pick needs no borrow of the chart state.
pub(super) struct PickStyle3d {
    pub(super) scatter_bg: wgpu::BindGroup,
    pub(super) line_bg: wgpu::BindGroup,
    /// The per-point style map and whether it binds an index column.
    pub(super) scatter_map: Option<(wgpu::BindGroup, bool)>,
    pub(super) display_scale: f32,
}

impl PickStyle3d {
    pub(super) fn of(style: &ChartStyle) -> Self {
        PickStyle3d {
            scatter_bg: style.scatter_bg.clone(),
            line_bg: style.line_bg.clone(),
            scatter_map: style
                .scatter_map
                .as_ref()
                .map(|map| (map.bind_group.clone(), map.has_index)),
            display_scale: style.display_scale,
        }
    }
}

/// One 3D scatter or line series' pick inputs, resolved before the pass.
struct PointPick3d {
    x: ColumnHandle,
    y: ColumnHandle,
    z: ColumnHandle,
    /// The camera group with this series' pick id, for the stroke.
    line: Option<data_render::surface_mesh::View3dBinding>,
    /// The camera group (with the value map when mapped), for the markers.
    scatter: Option<data_render::surface_mesh::View3dBinding>,
    arc: Option<(Arc<wgpu::Buffer>, u64)>,
    style_index: Option<ColumnHandle>,
    color_value: Option<ColumnHandle>,
    size_value: Option<ColumnHandle>,
}

impl Renderer {
    /// Pick the surface sample under `request`'s cursor on the 3D chart
    /// `config` (the active layer's panel as displayed) drawing `series`;
    /// `styles` are the series' prepared styles (`None` for a series not
    /// drawn yet), which the 3D scatter and line picks bind (#132).
    pub(super) fn pick_surface(
        &mut self,
        config: &Config,
        series: &[SeriesConfig],
        styles: &[Option<PickStyle3d>],
        request: &GpuPickRequest,
    ) -> Result<crate::GpuDataPickTicket> {
        use data_render::surface_mesh::{self as mesh, SurfacePass};
        for pass in [SurfacePass::Pick, SurfacePass::FloorContourPick] {
            self.pipelines.ensure_surface_pass(
                &self.device,
                [
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.field_bgl,
                    &self.surface_bgl,
                ],
                self.surface_format,
                pass,
            );
        }
        for kind in [
            View3dPipeline::ScatterPick,
            View3dPipeline::ScatterMappedPick,
            View3dPipeline::LinePick,
        ] {
            self.pipelines.ensure_view3d_pass(
                &self.device,
                [
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.per_point_style_map_bgl,
                    &self.surface_bgl,
                    &self.view3d_value_map_bgl,
                ],
                self.surface_format,
                kind,
            );
        }
        // The 3D scatter and line series: those with a height column and a
        // prepared style, by series position.
        let points = || {
            series.iter().zip(styles).filter_map(|(cfg, style)| {
                let style = style.as_ref()?;
                let z = cfg.z_column.as_deref()?;
                let primitives = effective_series_primitives(&config.draw_style, &cfg.render_type);
                (primitives.line || primitives.scatter).then_some((cfg, style, z, primitives))
            })
        };
        let floors = || {
            series
                .iter()
                .filter(|cfg| crate::renderer::device::draws_floor(&cfg.render_type))
        };
        let surfaces = || {
            series
                .iter()
                .filter(|cfg| crate::renderer::device::draws_surface(&cfg.render_type))
        };
        {
            let (_, mut ctx) = self.preparation_parts();
            for cfg in surfaces() {
                ctx.ensure_field_scratch(config, cfg)?;
            }
            for cfg in floors() {
                ctx.ensure_contour_scratch(cfg, config, request.display_scale)?;
            }
        }
        let (px, py) = (
            f64::from(request.canvas_position_px[0]),
            f64::from(request.canvas_position_px[1]),
        );
        // The cursor's pixel is the whole 1×1 target.
        let target = ((px - 0.5, py - 0.5), (1.0, 1.0));
        let camera = mesh::view3d_gpu(config, target, target);
        // The draw's camera of this panel: the arc prefix of a dashed 3D
        // line is the draw's, measured in the panel's pixels, which the pick
        // target also counts in (one chart pixel per target).
        let panel = config.chart_area.0;
        let panel_target = (
            (f64::from(panel.x), f64::from(panel.y)),
            (f64::from(panel.width), f64::from(panel.height)),
        );
        let draw_camera = mesh::view3d_gpu(config, panel_target, panel_target);
        // The 3D point series' columns, arcs and camera groups, resolved
        // with the preparation half; their pick ids follow the floors' and
        // surfaces' in `identities` (floors, surfaces, then these).
        let point_count = points().count();
        let first_point_id = floors().count() + surfaces().count();
        let mut point_picks: Vec<PointPick3d> = Vec::new();
        point_picks
            .try_reserve_exact(point_count)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "3D point pick inputs",
                reason: error.to_string(),
            })?;
        if point_count > 0 {
            let (_, mut ctx) = self.preparation_parts();
            for (k, (cfg, style, z, primitives)) in points().enumerate() {
                let pick_id = u32::try_from(first_point_id + k + 1).unwrap_or(u32::MAX);
                let Some(mut cam) = camera else {
                    continue;
                };
                cam.flags[3] = pick_id as f32;
                let handle = |id: &str| -> Result<ColumnHandle> {
                    ctx.pool
                        .handle_for(id)
                        .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
                };
                let (x, y, z_h) = (handle(&cfg.x_column)?, handle(&cfg.y_column)?, handle(z)?);
                let mapped = style.scatter_map.is_some();
                let scatter_cfg = extract_scatter(&cfg.render_type);
                let style_index = match (&style.scatter_map, scatter_cfg.as_deref()) {
                    (Some((_, true)), Some(sc)) => sc
                        .point_style_index_column
                        .as_deref()
                        .map(&handle)
                        .transpose()?,
                    _ => None,
                };
                let (color_value, size_value) = match scatter_cfg.as_deref() {
                    Some(sc) if mapped => (
                        sc.point_color_map
                            .as_ref()
                            .map(|map| handle(&map.column))
                            .transpose()?,
                        sc.point_size_map
                            .as_ref()
                            .map(|map| handle(&map.column))
                            .transpose()?,
                    ),
                    _ => (None, None),
                };
                if mapped && primitives.scatter {
                    ctx.ensure_value_map_scratch(config, cfg, request.display_scale)?;
                }
                let arc = if primitives.line
                    && extract_line(&cfg.render_type)
                        .is_some_and(|l| !matches!(l.line_style, LineStylePreset::Solid))
                {
                    let t = data_render::scatter_transform_from_config(config);
                    ctx.ensure_arc_prefix(
                        &cfg.series_id,
                        &cfg.x_column,
                        &cfg.y_column,
                        draw_camera
                            .as_ref()
                            .map(|camera| super::prepare::ArcThird::Height(z, camera)),
                        &t,
                        None,
                        LineInterpolation::Linear,
                    )
                    .map(|arc| arc.prefix)
                } else {
                    None
                };
                let binding = |mapped: bool| {
                    mesh::create_view3d_binding(
                        &ctx.gpu_ledger,
                        &ctx.device,
                        if mapped {
                            ctx.view3d_value_map_bgl
                        } else {
                            ctx.surface_bgl
                        },
                        &cam,
                        style.display_scale,
                        pick_id,
                        ctx.scatter_value_map_buffers(cfg, mapped),
                    )
                };
                point_picks.push(PointPick3d {
                    x,
                    y,
                    z: z_h,
                    line: primitives.line.then(|| binding(false)),
                    scatter: primitives.scatter.then(|| binding(mapped)),
                    arc,
                    style_index,
                    color_value,
                    size_value,
                });
            }
        }
        let tally = crate::gpu_memory::ChargeTally::new();
        let transform_buf = crate::gpu_memory::charged_buffer_init(
            &tally,
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("figgy surface pick transform"),
                contents: bytemuck::bytes_of(&data_render::scatter_transform_from_config(config)),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let transform_bg = data_render::create_scatter_transform_bind_group(
            &self.device,
            &self.transform_bgl,
            &transform_buf,
        );
        // The point entries size markers and strokes in pixels through
        // `pixel_to_ndc`: one chart pixel is the whole 1×1 target.
        let mut point_transform = data_render::scatter_transform_from_config(config);
        point_transform.pixel_to_ndc = [2.0, 2.0];
        let point_transform_buf = crate::gpu_memory::charged_buffer_init(
            &tally,
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("figgy 3D point pick transform"),
                contents: bytemuck::bytes_of(&point_transform),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let point_transform_bg = data_render::create_scatter_transform_bind_group(
            &self.device,
            &self.transform_bgl,
            &point_transform_buf,
        );
        let texture = |format, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("figgy surface pick target"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            mesh::SURFACE_PICK_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            mesh::DEPTH_STENCIL_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("figgy surface pick readback"),
            size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut identities = Vec::new();
        let mut charges = Vec::new();
        identities
            .try_reserve_exact(surfaces().count() + floors().count() + point_count)
            .and_then(|()| charges.try_reserve_exact(surfaces().count() + floors().count() + 1))
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "surface pick identities",
                reason: error.to_string(),
            })?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy surface pick"),
            });
        {
            let color_view = color.create_view(&Default::default());
            let depth_view = depth.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy surface pick"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let pipeline = self.pipelines.surface[SurfacePass::Pick as usize]
                .as_ref()
                .expect("the surface pick pipeline was just ensured");
            // The floor contours first, as the draw paints them: a surface
            // over a floor contour answers for its pixels.
            let floor_pipeline = self.pipelines.surface[SurfacePass::FloorContourPick as usize]
                .as_ref()
                .expect("the floor pick pipeline was just ensured");
            for cfg in floors() {
                let Some(camera) = camera.as_ref() else {
                    continue;
                };
                let Some(scratch) = self
                    .contour_cache
                    .get(cfg.series_id.as_str())
                    .and_then(ScratchEntry::contour)
                else {
                    continue;
                };
                let pick_id = u32::try_from(identities.len() + 1).unwrap_or(u32::MAX);
                let mut floor_camera = *camera;
                floor_camera.flags[3] = pick_id as f32;
                let look = mesh::surface_params_gpu(None, 0, request.display_scale, None, pick_id);
                let (surface_bg, charge) = mesh::create_surface_bind_group(
                    &self.gpu_ledger,
                    &self.device,
                    &self.surface_bgl,
                    &floor_camera,
                    &look,
                );
                charges.push(charge);
                identities.push(crate::gpu_data_pick::SurfacePickIdentity {
                    source_id: cfg.source_id.clone(),
                    series_id: cfg.series_id.clone(),
                    columns_are_y: false,
                });
                if !scratch.drawable {
                    continue;
                }
                pass.set_pipeline(floor_pipeline);
                pass.set_bind_group(0, &transform_bg, &[]);
                pass.set_bind_group(1, &scratch.style_bg, &[]);
                pass.set_bind_group(2, &scratch.field_bg, &[]);
                pass.set_bind_group(3, &surface_bg, &[]);
                pass.draw(0..mesh::SURFACE_QUAD_VERTICES, 0..1);
            }
            for cfg in surfaces() {
                let (Some(camera), Some(scratch)) = (
                    camera.as_ref(),
                    self.field_cache.get(cfg.series_id.as_str()),
                ) else {
                    continue;
                };
                let params = &scratch.signature.params;
                let columns_are_y = params.flags & data_render::FIELD_FLAG_COLUMNS_ARE_Y != 0;
                let (along, across) = if columns_are_y {
                    (params.rows, params.cols)
                } else {
                    (params.cols, params.rows)
                };
                let quads = if scratch.drawable {
                    mesh::surface_quad_count(along, across)
                } else {
                    0
                };
                let pick_id = u32::try_from(identities.len() + 1).unwrap_or(u32::MAX);
                let look = mesh::surface_params_gpu(
                    cfg.render_type.surface_style(),
                    params.cols,
                    request.display_scale,
                    None,
                    pick_id,
                );
                let (surface_bg, charge) = mesh::create_surface_bind_group(
                    &self.gpu_ledger,
                    &self.device,
                    &self.surface_bgl,
                    camera,
                    &look,
                );
                charges.push(charge);
                identities.push(crate::gpu_data_pick::SurfacePickIdentity {
                    source_id: cfg.source_id.clone(),
                    series_id: cfg.series_id.clone(),
                    columns_are_y,
                });
                if quads == 0 {
                    continue;
                }
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &transform_bg, &[]);
                pass.set_bind_group(1, &scratch.style_bg, &[]);
                pass.set_bind_group(2, &scratch.field_bg, &[]);
                pass.set_bind_group(3, &surface_bg, &[]);
                pass.draw(0..mesh::SURFACE_QUAD_VERTICES, 0..quads);
            }
            // The 3D scatter and line series after the surfaces, as the
            // draw paints them (#132): every stroke, then every marker,
            // depth tested, each writing kind 1 and the point's index.
            debug_assert_eq!(identities.len(), first_point_id);
            let view3d = |kind: View3dPipeline| {
                self.pipelines.view3d[kind as usize]
                    .as_ref()
                    .expect("the 3D pick pipelines were just ensured")
            };
            for (cfg, _, _, _) in points() {
                identities.push(crate::gpu_data_pick::SurfacePickIdentity {
                    source_id: cfg.source_id.clone(),
                    series_id: cfg.series_id.clone(),
                    columns_are_y: false,
                });
            }
            let pool_buffer = self.pool.buffer();
            for ((cfg, style, _, _), pick) in points().zip(&point_picks) {
                let Some(line) = pick.line.clone() else {
                    continue;
                };
                let _ = cfg;
                data_render::draw_line_layer(
                    &mut pass,
                    &data_render::ColumnLineLayer {
                        pipeline: view3d(View3dPipeline::LinePick),
                        transform_bg: &point_transform_bg,
                        style_bg: &style.line_bg,
                        pool_buffer,
                        x: pick.x,
                        y: pick.y,
                        arc: pick.arc.clone(),
                        verts_per_instance: crate::line_step::strip_vertex_count(
                            data_render::LINE_PRECISE_VERTICES_PER_INSTANCE,
                            crate::line_step::mode_word(
                                LineInterpolation::Linear,
                                crate::line_step::arc_pieces(None),
                            ),
                        ),
                        texture_bg: None,
                        closed: false,
                        value_map: None,
                        third: Some(pick.z),
                        view3d: Some(line),
                    },
                );
            }
            for ((_, style, _, _), pick) in points().zip(&point_picks) {
                let Some(scatter) = pick.scatter.clone() else {
                    continue;
                };
                let mapped = style.scatter_map.is_some();
                data_render::draw_scatter_layer(
                    &mut pass,
                    &data_render::ColumnScatterLayer {
                        pipeline: view3d(if mapped {
                            View3dPipeline::ScatterMappedPick
                        } else {
                            View3dPipeline::ScatterPick
                        }),
                        transform_bg: &point_transform_bg,
                        style_bg: &style.scatter_bg,
                        style_map_bg: style.scatter_map.as_ref().map(|(bg, _)| bg),
                        value_map_bg: None,
                        quad_vb: &self.quad_vb,
                        pool_buffer,
                        x: pick.x,
                        y: pick.y,
                        style_index: pick.style_index,
                        color_value: pick.color_value,
                        size_value: pick.size_value,
                        texture_bg: None,
                        third: Some(pick.z),
                        view3d: Some(scatter),
                    },
                );
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let (sender, receiver) = futures_channel::oneshot::channel();
        readback
            .slice(..16)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        charges.push(crate::gpu_memory::shared_charge(
            tally,
            &self.gpu_ledger,
            crate::gpu_memory::GpuResourceKind::Uniform,
        ));
        Ok(crate::GpuDataPickTicket::surface(
            crate::gpu_data_pick::SurfacePickTicket {
                device: Arc::clone(&self.device),
                readback,
                receiver,
                identities,
                _charges: charges,
            },
        ))
    }
}

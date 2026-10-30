//! The ID pick of a 3D chart (#49 D3): the panel's surfaces drawn once more
//! into a 1×1 `Rgba32Uint` target with a 1×1 depth-stencil, through the
//! draw's own vertex entry and depth rule, with the camera premultiplied by
//! a matrix that maps the cursor pixel onto the target. The pixel the
//! cursor is over picks exactly what it shows: the nearest quad's nearest
//! grid sample, hidden quads never.

use super::*;

impl Renderer {
    /// Pick the surface sample under `request`'s cursor on the 3D chart
    /// `config` (the active layer's panel as displayed) drawing `series`.
    pub(super) fn pick_surface(
        &mut self,
        config: &Config,
        series: &[SeriesConfig],
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
            .try_reserve_exact(surfaces().count() + floors().count())
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

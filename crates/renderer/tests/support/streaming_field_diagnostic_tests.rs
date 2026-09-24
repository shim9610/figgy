//! Explicitly opted-in backend diagnosis; no normal parity assertion is relaxed.
use super::*;

fn compare(label: &str, a: &RasterImage, b: &RasterImage) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let differences: Vec<_> = a
        .rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (a, b))| (i, a.to_vec(), b.to_vec()))
        .collect();
    let bytes = a.rgba.iter().zip(&b.rgba).filter(|(a, b)| a != b).count();
    let maximum = a
        .rgba
        .iter()
        .zip(&b.rgba)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    eprintln!(
        "FIELD DIAGNOSTIC {label}: pixels={} bytes={bytes} maxDelta={maximum}",
        differences.len()
    );
    for (i, av, bv) in differences.iter().take(40) {
        eprintln!(
            "  ({},{}) {:?} -> {:?}",
            *i as u32 % a.width,
            *i as u32 / a.width,
            av,
            bv
        );
    }
    if let Some(path) = std::env::var_os("FIGGY_FIELD_DIAGNOSTIC_ARTIFACT_DIR") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path).unwrap();
        for (suffix, image) in [("a", a), ("b", b)] {
            std::fs::write(
                path.join(format!("{label}-{suffix}.png")),
                encode_png(image).unwrap(),
            )
            .unwrap();
        }
        let mut mask = vec![255; a.rgba.len()];
        for (i, _, _) in &differences {
            mask[*i * 4..*i * 4 + 4].copy_from_slice(&[255, 0, 255, 255]);
        }
        std::fs::write(
            path.join(format!("{label}-difference-mask.png")),
            encode_png(&RasterImage {
                width: a.width,
                height: a.height,
                rgba: mask,
            })
            .unwrap(),
        )
        .unwrap();
    }
}

fn read_target(r: &Renderer, target: &wgpu::Texture, resolved: &wgpu::Texture) -> RasterImage {
    let (w, h) = (target.width(), target.height());
    let row = (w * 4).div_ceil(256) * 256;
    let readback = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("field diagnostic readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    {
        let view = target.create_view(&Default::default());
        let out = resolved.create_view(&Default::default());
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: Some(&out),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: resolved,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        resolved.size(),
    );
    r.queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    r.wait_idle();
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    let mut rgba = Vec::new();
    for bytes in mapped.chunks_exact(row as usize) {
        rgba.extend_from_slice(&bytes[..w as usize * 4]);
    }
    RasterImage {
        width: w,
        height: h,
        rgba,
    }
}

fn pass_control(
    r: &mut Renderer,
    config: &Config,
    series: &[SeriesConfig],
    scale: f32,
) -> [RasterImage; 2] {
    let mut config = config.scaled(scale);
    let (w, h) = (config.chart_area.0.width, config.chart_area.0.height);
    config.chart_area.0.x = 0;
    config.chart_area.0.y = 0;
    r.ensure_target(wgpu::TextureFormat::Rgba8Unorm, 4).unwrap();
    let id = r.register_chart(config.clone(), series.to_vec()).unwrap();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let frame = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id: id,
            view: &view,
        }])
        .unwrap();
    let item = &frame.items[0];
    [false, true].map(|split| {
        let create = |samples| {
            r.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("field diagnostic target"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let target = create(4);
        let resolved = create(1);
        let target_view = target.create_view(&Default::default());
        let mut encoder = r.device.create_command_encoder(&Default::default());
        let issue = |pass: &mut wgpu::RenderPass<'_>, stage| {
            let p = item.view.panel_rect;
            pass.set_viewport(
                p.x as f32,
                p.y as f32,
                p.width as f32,
                p.height as f32,
                0.0,
                1.0,
            );
            if stage == 0 || stage == 3 {
                pass.set_scissor_rect(p.x, p.y, p.width, p.height);
                pass.set_pipeline(&item.axis_pipeline);
                pass.set_bind_group(
                    0,
                    if stage == 0 {
                        &item.view.grid_bind_group
                    } else {
                        &item.view.decoration_bind_group
                    },
                    &[],
                );
                pass.draw(0..3, 0..1);
            } else {
                let d = item.data_area;
                pass.set_scissor_rect(d.x, d.y, d.width, d.height);
                let layers = item.series[0].layers();
                if stage == 1 {
                    data_render::issue_series_data(pass, &layers);
                } else {
                    data_render::issue_series_picked(pass, &layers);
                }
            }
        };
        for part in 0..if split { 4 } else { 1 } {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if part == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::WHITE)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if split {
                issue(&mut pass, part);
            } else {
                for stage in 0..4 {
                    issue(&mut pass, stage);
                }
            }
        }
        r.queue.submit([encoder.finish()]);
        read_target(r, &target, &resolved)
    })
}

fn pump_export(f: &mut Fixture, operation: StreamingOperation) {
    for _ in 0..10000 {
        match f
            .renderer
            .request_stream_operation_ranges(operation)
            .unwrap()
        {
            crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                let values: Vec<_> = ranges
                    .iter()
                    .map(|range| {
                        let source = &f.columns.iter().find(|(id, _)| *id == range.id).unwrap().1;
                        column(
                            source.data[range.offset as usize..(range.offset + range.len) as usize]
                                .to_vec(),
                        )
                    })
                    .collect();
                let bindings: Vec<_> = ranges
                    .iter()
                    .zip(&values)
                    .map(|(r, v)| crate::StreamRangeSourceBinding {
                        id: &r.id,
                        revision: r.revision,
                        source_len: r.source_len,
                        offset: r.offset,
                        source: crate::StreamColumnSource::HiLo(v),
                    })
                    .collect();
                f.renderer
                    .submit_stream_operation_ranges(operation, &bindings)
                    .unwrap();
            }
            crate::AutoStreamingRangeRequest::Backpressure { .. }
            | crate::AutoStreamingRangeRequest::AllSubmitted { .. } => f.renderer.wait_idle(),
            crate::AutoStreamingRangeRequest::Complete { .. } => return,
        }
    }
    panic!("diagnostic export did not finish");
}

#[test]
#[ignore = "opt-in backend diagnostic; honors WGPU_BACKEND and emits exact comparison artifacts"]
fn heatmap_browser_fixture_backend_diagnostic() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    eprintln!("FIELD compiler default={:?}, environment={:?}",
        wgpu::InstanceDescriptor::new_without_display_handle().backend_options.dx12.shader_compiler,
        wgpu::InstanceDescriptor::new_without_display_handle_from_env().backend_options.dx12.shader_compiler);
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = data_render::request_adapter(&instance).unwrap();
    eprintln!("FIELD adapter {:?}", adapter.get_info());
    let (device, queue) = data_render::request_device(&adapter).unwrap();
    let (device, queue) = (Arc::new(device), Arc::new(queue));
    let make = || {
        Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4096,
        )
        .unwrap()
    };
    for selected in [false, true] {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 160,
            height: 120,
        });
        config.chart_title.visible = false;
        config.chart_title.top_margin = 0.0;
        config.legend.visible = false;
        for (axis, is_x) in [
            (&mut config.bottom_x, true),
            (&mut config.top_x, true),
            (&mut config.left_y, false),
            (&mut config.right_y, false),
        ] {
            axis.min = if is_x { 1e12 + 0.1 } else { 0.1 };
            axis.max = if is_x { 1e12 + 4.0 } else { 4.0 };
            axis.scale = AxisScale::Linear;
            axis.inverted = false;
            axis.out_margin = 12.0;
            axis.label_style.label_visible = false;
            axis.title_option.visible = false;
        }
        let mut bar = crate::default::default_colorbar_options();
        bar.visible = false;
        bar.axis = config.left_y.clone();
        bar.nan_color = Color::new(0.9, 0.1, 0.4, 0.45);
        config.colorbar = Some(bar);
        if selected {
            config.picked_data = Some(crate::config::DataSelectionsConfig {
                visible: true,
                refs: vec![PickedDataRef::MatrixCell {
                    source_id: Some("matrix".into()),
                    series_id: "heat".into(),
                    x_index: 1,
                    y_index: 1,
                }],
                highlight_color: Color::new(0.9, 0.1, 0.6, 0.65),
                outline_width_px: 3.0,
                point_radius_extra_px: 3.0,
                contour_width_extra_px: 2.0,
            });
        }
        let columns = vec![
            ("x", column(vec![1e12 + 0.5, 1e12 + 1.5, 1e12 + 3.0])),
            ("y", column(vec![0.5, 1.5, 3.0])),
            ("z0", column(vec![0.2, f64::NAN, 1.4, 9.0])),
            ("z1", column(vec![2.1, 3.8, 2.6])),
        ];
        let series = vec![SeriesConfig {
            series_id: "heat".into(),
            source_id: Some("matrix".into()),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Heatmap {
                matrix: crate::data_config::MatrixRef {
                    columns: vec!["z0".into(), "z1".into(), "z0".into()],
                    orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                    grid_layout: crate::data_config::GridLayout::Centers,
                },
                fill: crate::data_config::FieldFillConfig {
                    mode: crate::data_config::FillMode::Continuous,
                    shading: crate::data_config::Shading::Interpolated,
                    opacity: 0.63,
                },
            },
        }];
        let mut resident = make();
        for (id, values) in &columns {
            resident.add_hilo_column(*id, values).unwrap();
        }
        if std::env::var_os("FIGGY_FIELD_DIAGNOSTIC_PASS_ONLY").is_some() {
            for scale in [1.0, 2.0] {
                let expected = pollster::block_on(resident.export_panel_rgba_with_clear_async(
                    &Chart::new(config.clone()),
                    &series,
                    scale,
                    Color::WHITE,
                ))
                .unwrap();
                let [single, split] = pass_control(&mut resident, &config, &series, scale);
                compare(
                    &format!("selected-{selected}-scale-{scale}-export-single"),
                    &expected,
                    &single,
                );
                compare(
                    &format!("selected-{selected}-scale-{scale}-same-packets-pass-split"),
                    &single,
                    &split,
                );
            }
            continue;
        }
        let mut renderer = make();
        renderer
            .register_streamed_columns(
                columns
                    .iter()
                    .map(|(id, v)| crate::StreamColumn {
                        id: (*id).into(),
                        len: v.data.len() as u64,
                        revision: 1,
                        encoding: crate::StreamEncoding::HiLoF32,
                        replay: crate::StreamReplay::RandomAccess,
                        statistics: crate::StreamStatistics::Unknown,
                    })
                    .collect(),
            )
            .unwrap();
        renderer
            .configure_streaming(crate::StreamingLimits {
                max_active_charts: 1,
                max_in_flight_chunks: 2,
                max_columns_per_chunk: 8,
                max_chunk_input_bytes: 512,
                max_in_flight_gpu_bytes: 4 * 1024 * 1024,
            })
            .unwrap();
        let chart = Chart::new(config.clone());
        let id = renderer
            .register_chart(config.clone(), series.clone())
            .unwrap();
        let view = renderer
            .create_chart_view(&chart, config.chart_area.0)
            .unwrap();
        renderer
            .request_auto_streaming_chart(
                id,
                &view,
                crate::StreamingChartOptions {
                    size: (160, 120),
                    clear_color: Color::WHITE,
                    max_primitives_per_chunk: 2,
                },
            )
            .unwrap();
        let mut f = Fixture {
            renderer,
            chart,
            id,
            view,
            series,
            columns,
        };
        finish_display(&mut f);
        for scale in [1.0, 2.0] {
            let expected = pollster::block_on(resident.export_panel_rgba_with_clear_async(
                &f.chart,
                &f.series,
                scale,
                Color::WHITE,
            ))
            .unwrap();
            let op = f
                .renderer
                .begin_stream_export(f.id, scale, Color::WHITE, 2)
                .unwrap();
            pump_export(&mut f, op);
            let actual = pollster::block_on(f.renderer.finish_stream_export(op)).unwrap();
            compare(
                &format!("selected-{selected}-scale-{scale}-resident-stream"),
                &expected,
                &actual,
            );
            let [single, split] = pass_control(&mut resident, &config, &f.series, scale);
            compare(
                &format!("selected-{selected}-scale-{scale}-export-single"),
                &expected,
                &single,
            );
            compare(
                &format!("selected-{selected}-scale-{scale}-same-packets-pass-split"),
                &single,
                &split,
            );
        }
    }
}

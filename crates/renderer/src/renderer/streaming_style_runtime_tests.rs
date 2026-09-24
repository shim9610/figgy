//! End-to-end style parity through the public borrowed-source streaming API.

use super::*;
use crate::config::{ConstellationOptions, MilkywayOptions, SketchOptions};
use crate::data_config::{
    ContourConfig, DataErrorBarPointStyleOverride, DataScatterPointStyleOverride, FieldFillConfig,
    FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
};
use std::cell::{Cell, RefCell};

struct RangeOnly<'a> {
    values: &'a [f32],
    calls: Cell<usize>,
    largest: Cell<usize>,
    last_start: Cell<u64>,
}

impl crate::ColumnSource for RangeOnly<'_> {
    fn len(&self) -> usize {
        self.values.len()
    }
    fn min(&self) -> f64 {
        0.0
    }
    fn max(&self) -> f64 {
        1.0
    }
    fn write_f32_le_into(&self, _: &mut [u8]) {
        panic!("stream called a full-column writer");
    }
    fn write_f32_pair_le_into_with_stats(
        &self,
        _: crate::ColumnPairWriter<'_>,
    ) -> crate::ColumnUploadStats {
        panic!("stream called a full-column pair writer");
    }
    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut writer: crate::ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, crate::ColumnRangeWriteError> {
        let end = (start as usize)
            .checked_add(writer.len())
            .filter(|end| *end <= self.values.len())
            .ok_or(crate::ColumnRangeWriteError::InvalidRange)?;
        self.calls.set(self.calls.get() + 1);
        self.largest.set(self.largest.get().max(writer.len()));
        self.last_start.set(self.last_start.get().max(start));
        for (index, &value) in self.values[start as usize..end].iter().enumerate() {
            writer.write_pair(index, value, 0.0);
        }
        Ok(None)
    }
}

fn test_config(style: DrawStyle) -> Config {
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    config.draw_style = style;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    chart.config().clone()
}

fn styles() -> [DrawStyle; 3] {
    [
        DrawStyle::Sketch(SketchOptions {
            amplitude_px: 2.4,
            wavelength_px: 43.0,
            seed: 73,
        }),
        DrawStyle::Milkyway(MilkywayOptions {
            seed: 291,
            planet_rim: 0.67,
            ..Default::default()
        }),
        DrawStyle::Constellation(ConstellationOptions {
            star_opacity: 0.79,
            line_opacity: 0.37,
        }),
    ]
}

fn scatter(color: Color) -> DataScatterStyleConfig {
    DataScatterStyleConfig {
        point_color: color,
        point_shape: ScatterShape::DiamondFilled,
        point_size: 9.0,
        point_style_index_column: Some("index".into()),
        point_style_table: Some(vec![DataScatterPointStyleConfig {
            point_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
            point_size: Some(23.0),
            point_shape: Some(ScatterShape::SquareFilled),
        }]),
        point_style_overrides: Some(vec![
            DataScatterPointStyleOverride {
                index: 7,
                style: DataScatterPointStyleConfig {
                    point_size: Some(0.0),
                    ..Default::default()
                },
            },
            DataScatterPointStyleOverride {
                index: 15,
                style: DataScatterPointStyleConfig {
                    point_size: Some(31.0),
                    ..Default::default()
                },
            },
        ]),
    }
}

fn error_style(color: Color) -> DataErrorBarStyleConfig {
    DataErrorBarStyleConfig {
        error_bar_color: color,
        error_bar_width: 2.5,
        error_bar_cap_size: 8.0,
        cap_width: 2.0,
        error_bar_style_index_column: Some("index".into()),
        error_bar_style_table: Some(vec![DataErrorBarPointStyleConfig {
            error_bar_color: Some(Color::new(1.0, 0.0, 1.0, 1.0)),
            error_bar_width: Some(9.0),
            error_bar_cap_size: Some(23.0),
            cap_width: Some(7.0),
        }]),
        error_bar_style_overrides: Some(vec![DataErrorBarPointStyleOverride {
            index: 12,
            style: DataErrorBarPointStyleConfig {
                error_bar_width: Some(0.0),
                ..Default::default()
            },
        }]),
    }
}

fn remap_resident(series: &mut SeriesConfig, remove_maps: bool) {
    series.x_column = format!("r{}", series.x_column);
    series.y_column = format!("r{}", series.y_column);
    let (scatter, error) = match &mut series.render_type {
        DataRenderType::Scatter { scatter } | DataRenderType::ScatterLine { scatter, .. } => {
            (scatter, None)
        }
        DataRenderType::ScatterErrorbarXY {
            scatter,
            err_x,
            err_y,
            err_style,
        } => {
            prefix_error_column(err_x);
            prefix_error_column(err_y);
            (scatter, Some(err_style))
        }
        DataRenderType::Line { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. } => unreachable!(),
    };
    scatter.point_style_index_column = (!remove_maps).then(|| "rindex".into());
    if remove_maps {
        scatter.point_style_table = None;
        scatter.point_style_overrides = None;
    }
    if let Some(error) = error {
        error.error_bar_style_index_column = (!remove_maps).then(|| "rindex".into());
        if remove_maps {
            error.error_bar_style_table = None;
            error.error_bar_style_overrides = None;
        }
    }
}

fn draw_registered(r: &mut Renderer, id: ChartId, view: &ChartView, samples: u32) -> Vec<u8> {
    let frame = r
        .prepare_registered(&[RegisteredChartDrawItem { chart_id: id, view }])
        .unwrap();
    let pixels = paint_frame_pixels(r, &frame, samples);
    drop(frame);
    r.end_gpu_frame();
    pixels
}

// Opt-in diagnostic artifacts only. Original PNGs retain the exact readback bytes;
// the crop is nearest-neighbour enlarged, and the mask is explicitly not a render.
fn save_visual_comparison(expected: &[u8], actual: &[u8], label: &str) {
    let Some(directory) = std::env::var_os("FIGGY_STYLE_EXPORT_ARTIFACT_DIR") else { return };
    assert_eq!(expected.len(), 320 * 240 * 4);
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    let name: String = label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let save = |suffix: &str, width, height, rgba: Vec<u8>| {
        std::fs::write(directory.join(format!("{name}-{suffix}.png")),
            encode_png(&RasterImage { width, height, rgba }).unwrap()).unwrap();
    };
    save("resident", 320, 240, expected.to_vec());
    save("streamed", 320, 240, actual.to_vec());
    let mut pair = vec![224; 648 * 240 * 4];
    for pixel in pair.chunks_exact_mut(4) { pixel[3] = 255; }
    for y in 0..240 {
        let row = y * 320 * 4..(y + 1) * 320 * 4;
        pair[y * 648 * 4..(y * 648 + 320) * 4].copy_from_slice(&expected[row.clone()]);
        pair[(y * 648 + 328) * 4..(y + 1) * 648 * 4].copy_from_slice(&actual[row]);
    }
    save("native-pair", 648, 240, pair);
    let mut mask = vec![255; expected.len()];
    let mut histogram = [0usize; 256];
    let mut maximum = (0u8, 0usize);
    let mut mask_difference = 0usize;
    for (i, (a, b)) in expected.chunks_exact(4).zip(actual.chunks_exact(4)).enumerate() {
        let delta = a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        histogram[usize::from(delta)] += 1;
        if delta > maximum.0 { maximum = (delta, i); }
        if delta > 0 { mask[i * 4..i * 4 + 4].copy_from_slice(&[255, 0, 255, 255]); }
        mask_difference += usize::from((a != [255; 4]) != (b != [255; 4]));
    }
    save("difference-locations-not-render", 320, 240, mask);
    let x0 = (maximum.1 % 320).saturating_sub(20).min(280);
    let y0 = (maximum.1 / 320).saturating_sub(20).min(200);
    let mut crop = vec![224; 648 * 320 * 4];
    for pixel in crop.chunks_exact_mut(4) { pixel[3] = 255; }
    for (side, pixels) in [expected, actual].into_iter().enumerate() {
        for y in 0..320 {
            for x in 0..320 {
                let source = ((y0 + y / 8) * 320 + x0 + x / 8) * 4;
                let dest = (y * 648 + side * 328 + x) * 4;
                crop[dest..dest + 4].copy_from_slice(&pixels[source..source + 4]);
            }
        }
    }
    save("max-delta-crop-8x", 648, 320, crop);
    eprintln!("visual artifacts {name}: max delta {} at ({},{}), crop origin ({x0},{y0}), nonwhite mask differences={mask_difference}, delta histogram={:?}",
        maximum.0, maximum.1 % 320, maximum.1 / 320,
        histogram.iter().enumerate().filter(|(_, n)| **n > 0).collect::<Vec<_>>());
}

fn mismatch(expected: &[u8], actual: &[u8], label: &str) -> Option<String> {
    assert_eq!(expected.len(), actual.len());
    save_visual_comparison(expected, actual, label);
    let count = expected
        .chunks_exact(4)
        .zip(actual.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    (count != 0).then(|| {
        let maximum = expected.iter().zip(actual).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        let first = expected.chunks_exact(4).zip(actual.chunks_exact(4)).position(|(a, b)| a != b).unwrap();
        format!("{label}: {count} differing pixels, max channel delta {maximum}, first ({},{}) {:?} != {:?}",
            first % 320, first / 320, &expected[first * 4..first * 4 + 4], &actual[first * 4..first * 4 + 4])
    })
}

fn compare_isolated_stream_chunks(
    resident: &mut Renderer,
    fixture: &[(&str, crate::Column<f32>)],
    config: &Config,
    declarations: &[SeriesConfig],
) {
    let pairs = std::env::var_os("FIGGY_STYLE_EXPORT_TWO_CHUNKS").is_some();
    let samples = if std::env::var_os("FIGGY_STYLE_EXPORT_NO_MSAA").is_some() { 1 } else { 4 };
    resident.ensure_target(wgpu::TextureFormat::Rgba8Unorm, samples).unwrap();
    let count = fixture[0].1.data.len();
    let mut streamed = Renderer::try_new_with_sample_count(
        RendererDevice::new(Arc::clone(&resident.device), Arc::clone(&resident.queue)),
        wgpu::TextureFormat::Rgba8Unorm, 8192, samples,
    ).unwrap();
    streamed.configure_streaming_runtime(limits(2)).unwrap();
    streamed.register_streamed_columns(fixture.iter().map(|(id, _)| {
        let mut column = source(id, 1);
        column.len = count as u64;
        column
    }).collect()).unwrap();
    let chart = Chart::new(config.clone());
    let resident_view = resident.create_chart_view(&chart, config.chart_area.0).unwrap();
    let stream_view = streamed.create_chart_view(&chart, config.chart_area.0).unwrap();
    let data_area = config.data_area().unwrap().0;
    let original_y = &fixture.iter().find(|(id, _)| *id == "y").unwrap().1;
    let mut failures = Vec::new();
    let mut tested = 0;
    for declaration in declarations {
        let resident_id = resident.register_chart(config.clone(), vec![declaration.clone()]).unwrap();
        let stream_id = streamed.register_chart(config.clone(), vec![declaration.clone()]).unwrap();
        let sizes: &[u64] = if pairs { &[1, 3, 7, 10, 19] } else { &[1, 3, 7, 19] };
        for &chunk_size in sizes {
          for shift in 0..if pairs { 2 } else { 1 } {
            let target = draw_target(&streamed, samples);
            clear_draw_target(&streamed, &target);
            let job = streamed.begin_chart_stream_draw(stream_id, &stream_view, &target, chunk_size).unwrap();
            let mut phases = [0usize; 2];
            let mut pair_start = 0;
            loop {
                streamed.service_stream_requests();
                let ticket = match streamed.request_chart_stream_draw(job).unwrap() {
                    StreamDrawRequestStatus::Ready(ticket) => ticket,
                    StreamDrawRequestStatus::AllSubmitted => break,
                    StreamDrawRequestStatus::Backpressure => panic!("isolated chunk was awaited"),
                };
                let columns: Vec<_> = streamed.stream_request_columns(ticket).unwrap().iter()
                    .map(|column| (column.column.clone(), column.range)).collect();
                let first = columns[0].1;
                let start = first.offset as usize;
                let end = start + first.len as usize;
                let errorbar = columns.iter().any(|(id, _)| id == "ex_lo");
                let phase_index = usize::from(!errorbar);
                let completes_pair = pairs && phases[phase_index] >= shift
                    && (phases[phase_index] - shift) % 2 == 1;
                assert!(columns.iter().all(|(_, range)| range.offset == first.offset && range.len == first.len));
                let inputs: Vec<_> = columns.iter().map(|(id, range)| {
                    let values = &fixture.iter().find(|(key, _)| key == id).unwrap().1.data;
                    StreamInput { column: id, bytes: bytemuck::cast_slice(
                        &values[range.offset as usize..(range.offset + range.len) as usize]) }
                }).collect();

                if pairs && !completes_pair {
                    pair_start = start;
                    clear_draw_target(&streamed, &target);
                    streamed.submit_chart_stream_draw(ticket, &inputs, &stream_view, &target).unwrap();
                    streamed.wait_idle();
                    streamed.end_gpu_frame();
                    phases[phase_index] += 1;
                    continue;
                }
                let start = if pairs { pair_start } else { start };

                // The ordinary resident path keeps all original instance IDs.
                // NaN suppresses rows outside this ticket without rebasing seeds.
                let mut resident_y = original_y.clone();
                resident_y.data[..start].fill(f32::NAN);
                resident_y.data[end..].fill(f32::NAN);
                resident.upsert_column("y", &resident_y).unwrap();
                let frame = resident.prepare_registered(&[RegisteredChartDrawItem {
                    chart_id: resident_id, view: &resident_view,
                }]).unwrap();
                let reference = draw_target(resident, samples);
                clear_draw_target(resident, &reference);
                let reference_view = reference.create_view(&Default::default());
                let mut encoder = resident.device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &reference_view, depth_slice: None, resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                        })], ..Default::default()
                    });
                    pass.set_viewport(0.0, 0.0, 320.0, 240.0, 0.0, 1.0);
                    pass.set_scissor_rect(data_area.x, data_area.y, data_area.width, data_area.height);
                    let mut layers = frame.items[0].series[0].layers();
                    if errorbar { layers.scatter = None; } else { layers.errorbar = None; }
                    data_render::issue_series_data(&mut pass, &layers);
                }
                resident.queue.submit([encoder.finish()]);
                let expected = read_draw_target(resident, &reference);
                drop(frame);
                resident.end_gpu_frame();

                // Paired mode retains the first chunk's MSAA samples without an
                // intermediate resolve. Single mode begins with a blank target.
                if !pairs { clear_draw_target(&streamed, &target); }
                streamed.submit_chart_stream_draw(ticket, &inputs, &stream_view, &target).unwrap();
                let actual = read_draw_target(&streamed, &target);
                streamed.end_gpu_frame();
                let label = format!("{} chunk(s) {} {} [{start},{end}) max={chunk_size} MSAA={samples}",
                    if pairs { 2 } else { 1 }, declaration.series_id, if errorbar { "errorbar" } else { "scatter" });
                if let Some(reason) = mismatch(&expected, &actual, &label) {
                    failures.push(reason);
                }
                let expected_ink = expected.chunks_exact(4).filter(|p| *p != [255, 255, 255, 255]).count();
                assert!(expected_ink > 0 || original_y.data[start..end].iter().all(|v| !v.is_finite()),
                    "{label}: reference must actually draw the requested range");
                let mask_diff = expected.chunks_exact(4).zip(actual.chunks_exact(4))
                    .filter(|(a, b)| (**a != [255, 255, 255, 255]) != (**b != [255, 255, 255, 255])).count();
                assert_eq!(mask_diff, 0, "{label}: binary mask differs");
                phases[usize::from(!errorbar)] += 1;
                tested += 1;
            }
            assert_eq!(phases, [count.div_ceil(chunk_size as usize); 2]);
            eprintln!("chunk comparison paired={pairs} shift={shift} {} MSAA={samples} max={chunk_size}: errorbar={}, scatter={}, RGBA failures so far={}",
                declaration.series_id, phases[0], phases[1], failures.len());
            streamed.cancel_chart_stream(stream_id).unwrap();
          }
        }
    }
    eprintln!("isolated real-runtime chunk comparisons={tested}, exact RGBA failures={}", failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn compare_stream_phase_pairs(
    resident: &mut Renderer,
    fixture: &[(&str, crate::Column<f32>)],
    config: &Config,
    declarations: &[SeriesConfig],
) {
    let samples = if std::env::var_os("FIGGY_STYLE_EXPORT_NO_MSAA").is_some() { 1 } else { 4 };
    resident.ensure_target(wgpu::TextureFormat::Rgba8Unorm, samples).unwrap();
    let count = fixture[0].1.data.len();
    let mut streamed = Renderer::try_new_with_sample_count(
        RendererDevice::new(Arc::clone(&resident.device), Arc::clone(&resident.queue)),
        wgpu::TextureFormat::Rgba8Unorm, 8192, samples,
    ).unwrap();
    streamed.configure_streaming_runtime(limits(2)).unwrap();
    streamed.register_streamed_columns(fixture.iter().map(|(id, _)| {
        let mut column = source(id, 1); column.len = count as u64; column
    }).collect()).unwrap();
    let chart = Chart::new(config.clone());
    let resident_view = resident.create_chart_view(&chart, config.chart_area.0).unwrap();
    let stream_view = streamed.create_chart_view(&chart, config.chart_area.0).unwrap();
    let resident_id = resident.register_chart(config.clone(), declarations.to_vec()).unwrap();
    let stream_id = streamed.register_chart(config.clone(), declarations.to_vec()).unwrap();
    let frame = resident.prepare_registered(&[RegisteredChartDrawItem {
        chart_id: resident_id, view: &resident_view,
    }]).unwrap();
    let data = config.data_area().unwrap().0;
    let mut failures = Vec::new();
    for first_stage in 0..declarations.len() * 2 - 1 {
        let reference = draw_target(resident, samples);
        clear_draw_target(resident, &reference);
        let reference_view = reference.create_view(&Default::default());
        let mut encoder = resident.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &reference_view, depth_slice: None, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })], ..Default::default()
            });
            pass.set_viewport(0.0, 0.0, 320.0, 240.0, 0.0, 1.0);
            pass.set_scissor_rect(data.x, data.y, data.width, data.height);
            for stage in first_stage..first_stage + 2 {
                let mut layers = frame.items[0].series[stage / 2].layers();
                if stage % 2 == 0 { layers.scatter = None; } else { layers.errorbar = None; }
                data_render::issue_series_data(&mut pass, &layers);
            }
        }
        resident.queue.submit([encoder.finish()]);
        let expected = read_draw_target(resident, &reference);
        let target = draw_target(&streamed, samples);
        clear_draw_target(&streamed, &target);
        let job = streamed.begin_chart_stream_draw(stream_id, &stream_view, &target, count as u64).unwrap();
        for stage in 0..first_stage + 2 {
            streamed.service_stream_requests();
            let StreamDrawRequestStatus::Ready(ticket) = streamed.request_chart_stream_draw(job).unwrap()
                else { panic!("two-phase request was not ready") };
            let columns: Vec<_> = streamed.stream_request_columns(ticket).unwrap().iter()
                .map(|column| (column.column.clone(), column.range)).collect();
            assert!(columns.iter().all(|(_, range)| range.offset == 0 && range.len == count as u64));
            let inputs: Vec<_> = columns.iter().map(|(id, _)| StreamInput {
                column: id, bytes: bytemuck::cast_slice(&fixture.iter().find(|(key, _)| key == id).unwrap().1.data),
            }).collect();
            if stage == first_stage { clear_draw_target(&streamed, &target); }
            streamed.submit_chart_stream_draw(ticket, &inputs, &stream_view, &target).unwrap();
            streamed.wait_idle();
            streamed.end_gpu_frame();
        }
        let actual = read_draw_target(&streamed, &target);
        let label = format!("two phase chunks {first_stage}->{} MSAA={samples}", first_stage + 1);
        let mask_diff = expected.chunks_exact(4).zip(actual.chunks_exact(4))
            .filter(|(a, b)| (**a != [255, 255, 255, 255]) != (**b != [255, 255, 255, 255])).count();
        if let Some(reason) = mismatch(&expected, &actual, &label) {
            eprintln!("{reason}; binary mask differing={mask_diff}");
            failures.push(reason);
        } else { eprintln!("{label}: exact RGBA match; binary mask differing={mask_diff}"); }
        streamed.cancel_chart_stream(stream_id).unwrap();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn sketch_errorbar_stream_export_matches_browser_fixture() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    const N: usize = 19;
    // JavaScript computes in f64 before Float32Array rounds each completed value.
    let fixture: Vec<_> = ["x", "y", "ex_lo", "ex_hi", "ey_lo", "ey_hi", "index"]
        .into_iter()
        .map(|id| {
            let data = (0..N)
                .map(|i| {
                    (match id {
                        "x" => 0.08 + (i % 9) as f64 * 0.1,
                        "y" if i == 11 && std::env::var_os("FIGGY_STYLE_EXPORT_NO_NAN").is_none() => f64::NAN,
                        "y" => 0.11 + ((i * 7) % 13) as f64 * 0.055,
                        "ex_lo" => 0.03,
                        "ex_hi" => 0.06,
                        "ey_lo" => 0.045,
                        "ey_hi" => 0.025,
                        _ => 0.0,
                    }) as f32
                })
                .collect();
            (
                id,
                crate::Column {
                    data,
                    min: 0.0,
                    max: 1.0,
                },
            )
        })
        .collect();
    let config = test_config(if std::env::var_os("FIGGY_STYLE_EXPORT_PRECISE").is_some() {
        DrawStyle::Precise
    } else { styles()[0].clone() });
    let series: Vec<_> = ["front", "back"]
        .into_iter()
        .take(if std::env::var_os("FIGGY_STYLE_EXPORT_ONE_SERIES").is_some() { 1 } else { 2 })
        .enumerate()
        .map(|(i, id)| {
            let mut tint = if i == 0 {
                Color::new(0.9, 0.2, 0.1, 0.65)
            } else {
                Color::new(0.1, 0.4, 0.9, 0.55)
            };
            if std::env::var_os("FIGGY_STYLE_EXPORT_OPAQUE").is_some() { tint.a = 1.0; }
            let mut point = scatter(tint);
            if std::env::var_os("FIGGY_STYLE_EXPORT_NO_SCATTER").is_some() { point.point_size = 0.0; }
            let mut error = error_style(tint);
            if std::env::var_os("FIGGY_STYLE_EXPORT_NO_ERROR").is_some() {
                error.error_bar_width = 0.0;
                error.cap_width = 0.0;
            }
            let mut cfg = declaration(id, "x", "y");
            cfg.render_type = DataRenderType::ScatterErrorbarXY {
                scatter: point,
                err_x: ErrorRef::Asymmetric {
                    lower: "ex_lo".into(),
                    upper: "ex_hi".into(),
                },
                err_y: ErrorRef::Asymmetric {
                    lower: "ey_lo".into(),
                    upper: "ey_hi".into(),
                },
                err_style: error,
            };
            cfg
        })
        .collect();
    // This focused backend regression deliberately honors WGPU_BACKEND so the
    // browser/Dawn D3D12 result can be compared with native D3D12 and Vulkan.
    let instance = wgpu::Instance::new(
        wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
    );
    let adapter = data_render::request_adapter(&instance).expect("style export GPU required");
    eprintln!("style export adapter: {:?}", adapter.get_info());
    let (device, queue) = data_render::request_device(&adapter).expect("style export device required");
    let (device, queue) = (Arc::new(device), Arc::new(queue));
    let make = || {
        Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            8192,
        )
        .unwrap()
    };
    let mut resident = make();
    for (id, values) in &fixture {
        resident.add_column(*id, values).unwrap();
    }
    if std::env::var_os("FIGGY_STYLE_EXPORT_SINGLE_CHUNK").is_some()
        || std::env::var_os("FIGGY_STYLE_EXPORT_TWO_CHUNKS").is_some() {
        compare_isolated_stream_chunks(&mut resident, &fixture, &config, &series);
        return;
    }
    if std::env::var_os("FIGGY_STYLE_EXPORT_PHASE_PAIR").is_some() {
        compare_stream_phase_pairs(&mut resident, &fixture, &config, &series);
        return;
    }
    if std::env::var_os("FIGGY_STYLE_EXPORT_PASS_CONTROL").is_some() {
        let samples = if std::env::var_os("FIGGY_STYLE_EXPORT_NO_MSAA").is_some() { 1 } else { 4 };
        resident.ensure_target(wgpu::TextureFormat::Rgba8Unorm, samples).unwrap();
        let chart = resident.register_chart(config.clone(), series.clone()).unwrap();
        let view = resident.create_chart_view(&Chart::new(config.clone()), config.chart_area.0).unwrap();
        let frame = resident.prepare_registered(&[RegisteredChartDrawItem { chart_id: chart, view: &view }]).unwrap();
        let item = &frame.items[0];
        let stage_count = item.series.len() * 2 + 2;
        let issued_stages = RefCell::new(Vec::new());
        let draw = |pass: &mut wgpu::RenderPass<'_>, stage: usize| {
            issued_stages.borrow_mut().push(stage);
            let panel = item.view.panel_rect;
            pass.set_viewport(panel.x as f32, panel.y as f32, panel.width as f32, panel.height as f32, 0.0, 1.0);
            if stage == 0 || stage == stage_count - 1 {
                pass.set_scissor_rect(panel.x, panel.y, panel.width, panel.height);
                pass.set_pipeline(&item.axis_pipeline);
                pass.set_bind_group(0, if stage == 0 { &item.view.grid_bind_group } else { &item.view.decoration_bind_group }, &[]);
                pass.draw(0..3, 0..1);
            } else {
                let data = item.data_area;
                pass.set_scissor_rect(data.x, data.y, data.width, data.height);
                let mut layers = item.series[(stage - 1) / 2].layers();
                if stage % 2 == 1 { layers.scatter = None; } else { layers.errorbar = None; }
                data_render::issue_series_data(pass, &layers);
            }
        };
        let pixels: Vec<_> = [false, true].into_iter().map(|split| {
            issued_stages.borrow_mut().clear();
            let target = draw_target(&resident, samples);
            clear_draw_target(&resident, &target);
            let view = target.create_view(&Default::default());
            let mut encoder = resident.device.create_command_encoder(&Default::default());
            for part in 0..if split { stage_count } else { 1 } {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view, depth_slice: None, resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })], ..Default::default()
                });
                if split { draw(&mut pass, part); } else { for stage in 0..stage_count { draw(&mut pass, stage); } }
            }
            assert_eq!(*issued_stages.borrow(), (0..stage_count).collect::<Vec<_>>());
            eprintln!("pass-control split={split} issued stages: {:?}", issued_stages.borrow());
            resident.queue.submit([encoder.finish()]);
            read_draw_target(&resident, &target)
        }).collect();
        for metric in ["nonwhite", "chroma", "darkness"] {
            for threshold in [1u16, 2, 4, 8, 16, 32, 64, 128] {
                let value = |pixel: &[u8]| -> u16 {
                    match metric {
                        "nonwhite" => 255 - u16::from(*pixel[..3].iter().min().unwrap()),
                        "chroma" => u16::from(*pixel[..3].iter().max().unwrap())
                            - u16::from(*pixel[..3].iter().min().unwrap()),
                        "darkness" => 255 - ((2126 * u32::from(pixel[0])
                            + 7152 * u32::from(pixel[1]) + 722 * u32::from(pixel[2]) + 5000) / 10000) as u16,
                        _ => unreachable!(),
                    }
                };
                let mut active = [0usize; 2];
                let mut only = [0usize; 2];
                for (a, b) in pixels[0].chunks_exact(4).zip(pixels[1].chunks_exact(4)) {
                    let (a, b) = (value(a) >= threshold, value(b) >= threshold);
                    active[0] += usize::from(a);
                    active[1] += usize::from(b);
                    only[0] += usize::from(a && !b);
                    only[1] += usize::from(b && !a);
                }
                eprintln!("binary {metric} threshold={threshold}: active={active:?}, only={only:?}, differing={}", only[0] + only[1]);
            }
        }
        assert!(mismatch(&pixels[0], &pixels[1], "same packets, different render pass boundaries").is_none(), "{}", mismatch(&pixels[0], &pixels[1], "same packets, different render pass boundaries").unwrap());
        return;
    }
    let expected = pollster::block_on(resident.export_panel_rgba_with_clear_async(
        &Chart::new(config.clone()),
        &series,
        1.0,
        Color::WHITE,
    ))
    .unwrap();
    let chunks = if std::env::var_os("FIGGY_STYLE_EXPORT_UNSPLIT").is_some() { vec![19] } else { vec![1, 3, 7] };
    for chunk in chunks {
        let mut streamed = make();
        streamed
            .configure_streaming(crate::StreamingLimits {
                max_active_charts: 1,
                max_in_flight_chunks: 2,
                max_columns_per_chunk: 8,
                max_chunk_input_bytes: 4096,
                max_in_flight_gpu_bytes: 64 * 1024 * 1024,
            })
            .unwrap();
        streamed
            .register_streamed_columns(
                fixture
                    .iter()
                    .map(|(id, _)| crate::StreamColumn {
                        id: (*id).into(),
                        len: N as u64,
                        revision: 1,
                        encoding: crate::StreamEncoding::ScalarF32,
                        replay: crate::StreamReplay::RandomAccess,
                        statistics: crate::StreamStatistics::Unknown,
                    })
                    .collect(),
            )
            .unwrap();
        let chart = streamed
            .register_chart(config.clone(), series.clone())
            .unwrap();
        let view = streamed
            .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
            .unwrap();
        streamed
            .request_auto_streaming_chart_with_config(
                chart,
                &view,
                config.clone(),
                crate::StreamingChartOptions {
                    size: (320, 240),
                    clear_color: Color::WHITE,
                    max_primitives_per_chunk: chunk,
                },
            )
            .unwrap();
        let bindings: Vec<_> = fixture
            .iter()
            .map(|(id, values)| crate::StreamSourceBinding {
                id,
                revision: 1,
                source: crate::StreamColumnSource::Scalar(values),
            })
            .collect();
        loop {
            match streamed.auto_stream_chart_step(chart, &bindings).unwrap() {
                crate::AutoStreamingProgress::AllSubmitted { .. }
                | crate::AutoStreamingProgress::Complete { .. } => break,
                crate::AutoStreamingProgress::Backpressure { .. } => streamed.wait_idle(),
                _ => {}
            }
        }
        streamed.wait_idle();
        drop(
            streamed
                .prepare_registered(&[RegisteredChartDrawItem {
                    chart_id: chart,
                    view: &view,
                }])
                .unwrap(),
        );
        streamed.auto_stream_chart_step(chart, &bindings).unwrap();
        let operation = streamed
            .begin_stream_export(chart, 1.0, Color::WHITE, chunk)
            .unwrap();
        loop {
            match streamed.request_stream_operation_ranges(operation).unwrap() {
                crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                    let values: Vec<_> = ranges
                        .iter()
                        .map(|range| {
                            let values = &fixture.iter().find(|(id, _)| *id == range.id).unwrap().1;
                            crate::Column {
                                data: values.data
                                    [range.offset as usize..(range.offset + range.len) as usize]
                                    .to_vec(),
                                min: 0.0,
                                max: 1.0,
                            }
                        })
                        .collect();
                    let bindings: Vec<_> = ranges
                        .iter()
                        .zip(&values)
                        .map(|(range, values)| crate::StreamRangeSourceBinding {
                            id: &range.id,
                            revision: range.revision,
                            source_len: range.source_len,
                            offset: range.offset,
                            source: crate::StreamColumnSource::Scalar(values),
                        })
                        .collect();
                    streamed
                        .submit_stream_operation_ranges(operation, &bindings)
                        .unwrap();
                }
                crate::AutoStreamingRangeRequest::Backpressure { .. } => streamed.wait_idle(),
                crate::AutoStreamingRangeRequest::AllSubmitted { .. }
                | crate::AutoStreamingRangeRequest::Complete { .. } => break,
            }
        }
        let actual = pollster::block_on(streamed.finish_stream_export(operation)).unwrap();
        assert!(
            mismatch(
                &expected.rgba,
                &actual.rgba,
                &format!("Sketch errorbar export chunk={chunk}")
            )
            .is_none(),
            "{}",
            mismatch(
                &expected.rgba,
                &actual.rgba,
                &format!("Sketch errorbar export chunk={chunk}")
            )
            .unwrap()
        );
    }
}

#[test]
fn styled_stream_runtime_matches_resident_pixels_and_ignores_point_maps() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    const N: usize = 19;
    let fixture: Vec<(&str, crate::Column<f32>)> =
        ["x", "y", "ex_lo", "ex_hi", "ey_lo", "ey_hi", "index"]
            .into_iter()
            .map(|id| {
                let data = (0..N)
                    .map(|i| match id {
                        "x" => 0.08 + (i % 9) as f32 * 0.1,
                        "y" if i == 11 => f32::NAN,
                        "y" => 0.11 + ((i * 7) % 13) as f32 * 0.055,
                        "ex_lo" => 0.03,
                        "ex_hi" => 0.06,
                        "ey_lo" => 0.045,
                        "ey_hi" => 0.025,
                        "index" => 0.0,
                        _ => unreachable!(),
                    })
                    .collect();
                (
                    id,
                    crate::Column {
                        data,
                        min: 0.0,
                        max: 1.0,
                    },
                )
            })
            .collect();
    let mut failures = Vec::new();
    for samples in [1, 4] {
        for (mode, style) in styles().into_iter().enumerate() {
            for errorbars in [false, true] {
                if mode == 2 && errorbars {
                    continue;
                }
                let (device, queue) =
                    data_render::shared_device().expect("styled stream GPU required");
                let mut r = Renderer::try_new_with_sample_count(
                    RendererDevice::new(device, queue),
                    wgpu::TextureFormat::Rgba8Unorm,
                    8192,
                    samples,
                )
                .unwrap();
                r.configure_streaming_runtime(limits(2)).unwrap();
                r.register_streamed_columns(
                    fixture
                        .iter()
                        .map(|(id, _)| {
                            let mut column = source(id, 1);
                            column.len = N as u64;
                            column
                        })
                        .collect(),
                )
                .unwrap();
                for (id, column) in &fixture {
                    r.add_column(&format!("r{id}"), column).unwrap();
                }
                let config = test_config(style);
                let view = r
                    .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
                    .unwrap();
                let declarations: Vec<_> = ["front", "back"]
                    .into_iter()
                    .enumerate()
                    .map(|(i, id)| {
                        let color = if i == 0 {
                            Color::new(0.9, 0.2, 0.1, 0.65)
                        } else {
                            Color::new(0.1, 0.4, 0.9, 0.55)
                        };
                        let mut series = declaration(id, "x", "y");
                        series.render_type = if mode == 2 {
                            DataRenderType::ScatterLine {
                                scatter: scatter(color),
                                line: DataLineStyleConfig {
                                    line_color: color,
                                    line_width: 2.5,
                                    line_style: LineStylePreset::Solid,
                                },
                            }
                        } else if errorbars {
                            DataRenderType::ScatterErrorbarXY {
                                scatter: scatter(color),
                                err_x: ErrorRef::Asymmetric {
                                    lower: "ex_lo".into(),
                                    upper: "ex_hi".into(),
                                },
                                err_y: ErrorRef::Asymmetric {
                                    lower: "ey_lo".into(),
                                    upper: "ey_hi".into(),
                                },
                                err_style: error_style(color),
                            }
                        } else {
                            DataRenderType::Scatter {
                                scatter: scatter(color),
                            }
                        };
                        series
                    })
                    .collect();
                let mut resident = declarations.clone();
                for series in &mut resident {
                    remap_resident(series, false);
                }
                let mut unmapped = declarations.clone();
                for series in &mut unmapped {
                    remap_resident(series, true);
                }
                let resident_id = r.register_chart(config.clone(), resident).unwrap();
                let unmapped_id = r.register_chart(config.clone(), unmapped).unwrap();
                let stream_id = r.register_chart(config, declarations).unwrap();
                let expected = draw_registered(&mut r, resident_id, &view, samples);
                let no_maps = draw_registered(&mut r, unmapped_id, &view, samples);
                if let Some(failure) = mismatch(
                    &expected,
                    &no_maps,
                    &format!(
                        "resident style maps ignored: style={mode} errorbars={errorbars} samples={samples}"
                    ),
                ) {
                    failures.push(failure);
                }
                for chunk in [1, 2, 7] {
                    let range_sources: Vec<_> = fixture
                        .iter()
                        .map(|(_, column)| RangeOnly {
                            values: &column.data,
                            calls: Cell::new(0),
                            largest: Cell::new(0),
                            last_start: Cell::new(0),
                        })
                        .collect();
                    let bindings: Vec<_> = fixture
                        .iter()
                        .zip(&range_sources)
                        .map(|((id, _), source)| crate::StreamSourceBinding {
                            id,
                            revision: 1,
                            source: crate::StreamColumnSource::Scalar(source),
                        })
                        .collect();
                    r.begin_streaming_chart(
                        stream_id,
                        &view,
                        crate::StreamingChartOptions {
                            size: (320, 240),
                            clear_color: Color::WHITE,
                            max_primitives_per_chunk: chunk,
                        },
                    )
                    .unwrap();
                    let mut submitted = 0;
                    for _ in 0..256 {
                        match r.stream_chart_step(stream_id, &view, &bindings).unwrap() {
                            crate::StreamingProgress::Submitted {
                                submitted_primitives,
                                ..
                            } => {
                                submitted = submitted_primitives;
                            }
                            crate::StreamingProgress::Backpressure { .. } => {
                                wait_stream_slots(&mut r, 0)
                            }
                            crate::StreamingProgress::AllSubmitted { total_primitives } => {
                                assert_eq!(submitted, total_primitives);
                                break;
                            }
                        }
                    }
                    assert!(matches!(
                        r.stream_chart_step(stream_id, &view, &bindings).unwrap(),
                        crate::StreamingProgress::AllSubmitted { .. }
                    ));
                    wait_stream_slots(&mut r, 0);
                    assert!(submitted > 0);
                    assert!(range_sources[0].last_start.get() >= 14);
                    assert!(
                        range_sources.iter().all(|source| source.largest.get()
                            <= chunk as usize + usize::from(mode == 2))
                    );
                    let actual = draw_registered(&mut r, stream_id, &view, samples);
                    if let Some(failure) = mismatch(
                        &expected,
                        &actual,
                        &format!(
                            "stream style={mode} errorbars={errorbars} samples={samples} chunk={chunk}"
                        ),
                    ) {
                        failures.push(failure);
                    }
                    let stable = draw_registered(&mut r, stream_id, &view, samples);
                    assert!(
                        mismatch(&actual, &stable, "completed stream output changed").is_none()
                    );
                    r.cancel_streaming_chart(stream_id).unwrap();
                    wait_stream_slots(&mut r, 0);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn styled_stream_suppressed_histogram_and_fields_are_exact_noops() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut failures = Vec::new();
    for samples in [1, 4] {
        for (mode, style) in styles().into_iter().enumerate() {
            let (device, queue) = data_render::shared_device().expect("styled stream GPU required");
            let mut r = Renderer::try_new_with_sample_count(
                RendererDevice::new(device, queue),
                wgpu::TextureFormat::Rgba8Unorm,
                4096,
                samples,
            )
            .unwrap();
            r.configure_streaming_runtime(limits(2)).unwrap();
            r.register_streamed_columns(
                [("x", 2), ("y", 2), ("z", 1)]
                    .map(|(id, len)| {
                        let mut column = source(id, 1);
                        column.len = len;
                        column
                    })
                    .to_vec(),
            )
            .unwrap();
            for (id, data) in [
                ("rx", vec![0.1, 0.9]),
                ("ry", vec![0.2, 0.8]),
                ("rz", vec![0.6]),
            ] {
                r.add_column(
                    id,
                    &crate::Column {
                        data,
                        min: 0.0,
                        max: 1.0,
                    },
                )
                .unwrap();
            }
            let mut config = test_config(style);
            config.colorbar = Some(crate::default::default_colorbar_options());
            let view = r
                .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
                .unwrap();
            let empty = r.register_chart(config.clone(), Vec::new()).unwrap();
            let blank = draw_registered(&mut r, empty, &view, samples);
            for kind in 0..4 {
                let matrix = MatrixRef {
                    columns: vec!["z".into()],
                    orientation: MatrixOrientation::ColumnsAreX,
                    grid_layout: GridLayout::Edges,
                };
                let fill = FieldFillConfig {
                    mode: FillMode::Continuous,
                    shading: Shading::Flat,
                    opacity: 0.7,
                };
                let contour = ContourConfig {
                    levels: vec![0.5],
                    line: DataLineStyleConfig {
                        line_color: Color::BLACK,
                        line_width: 2.0,
                        line_style: LineStylePreset::Solid,
                    },
                    per_level_color: None,
                    labels: None,
                };
                let mut stream = declaration("suppressed", "x", "y");
                stream.render_type = match kind {
                    0 => DataRenderType::Histogram {
                        bar: DataBarStyleConfig {
                            fill_color: Color::BLACK,
                            border_color: Color::BLACK,
                            border_width: 1.0,
                            baseline: 0.0,
                            gap_px: 1.0,
                            width_ratio: 1.0,
                            orientation: crate::data_config::BarOrientation::Vertical,
                            bar_style_overrides: None,
                        },
                    },
                    1 => DataRenderType::Heatmap { matrix, fill },
                    2 => DataRenderType::Contour { matrix, contour },
                    3 => DataRenderType::HeatmapContour {
                        matrix,
                        fill,
                        contour,
                    },
                    _ => unreachable!(),
                };
                let mut resident = stream.clone();
                resident.x_column = "rx".into();
                resident.y_column = "ry".into();
                match &mut resident.render_type {
                    DataRenderType::Heatmap { matrix, .. }
                    | DataRenderType::Contour { matrix, .. }
                    | DataRenderType::HeatmapContour { matrix, .. } => {
                        matrix.columns = vec!["rz".into()]
                    }
                    DataRenderType::Scatter { .. }
                    | DataRenderType::Line { .. }
                    | DataRenderType::ScatterLine { .. }
                    | DataRenderType::ScatterErrorbarX { .. }
                    | DataRenderType::ScatterErrorbarY { .. }
                    | DataRenderType::ScatterErrorbarXY { .. }
                    | DataRenderType::LineScatterErrorbarX { .. }
                    | DataRenderType::LineScatterErrorbarY { .. }
                    | DataRenderType::LineScatterErrorbarXY { .. }
                    | DataRenderType::Histogram { .. } => {}
                }
                let resident_id = r.register_chart(config.clone(), vec![resident]).unwrap();
                let stream_id = r.register_chart(config.clone(), vec![stream]).unwrap();
                let expected = draw_registered(&mut r, resident_id, &view, samples);
                if let Some(failure) = mismatch(
                    &blank,
                    &expected,
                    &format!("resident suppressed style={mode} kind={kind} samples={samples}"),
                ) {
                    failures.push(failure);
                }
                for chunk in [1, 2, 7] {
                    r.begin_streaming_chart(
                        stream_id,
                        &view,
                        crate::StreamingChartOptions {
                            size: (320, 240),
                            clear_color: Color::WHITE,
                            max_primitives_per_chunk: chunk,
                        },
                    )
                    .unwrap();
                    assert!(matches!(
                        r.stream_chart_step(stream_id, &view, &[]).unwrap(),
                        crate::StreamingProgress::AllSubmitted {
                            total_primitives: 0
                        }
                    ));
                    let actual = draw_registered(&mut r, stream_id, &view, samples);
                    if let Some(failure) = mismatch(
                        &expected,
                        &actual,
                        &format!(
                            "suppressed stream style={mode} kind={kind} samples={samples} chunk={chunk}"
                        ),
                    ) {
                        failures.push(failure);
                    }
                    r.cancel_streaming_chart(stream_id).unwrap();
                }
                r.remove_chart(resident_id).unwrap();
                r.remove_chart(stream_id).unwrap();
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn streamed_line_capability_matches_connected_arc_and_heatmap_paths() {
    let (mut r, _, _) = renderer(2);
    for style in [DrawStyle::Precise, styles()[0], styles()[1], styles()[2]] {
        for dash in [LineStylePreset::Solid, LineStylePreset::Dash] {
            let mut config = r.chart_states.values().next().unwrap().config.clone();
            config.draw_style = style;
            let view = r
                .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
                .unwrap();
            let mut series = declaration("arc-gated", "x", "a");
            series.render_type = DataRenderType::ScatterLine {
                scatter: {
                    let mut point = scatter(Color::BLACK);
                    point.point_style_index_column = None;
                    point.point_style_table = None;
                    point.point_style_overrides = None;
                    point
                },
                line: DataLineStyleConfig {
                    line_color: Color::BLACK,
                    line_width: 2.0,
                    line_style: dash,
                },
            };
            let id = r.register_chart(config, vec![series]).unwrap();
            let result = r.begin_streaming_chart(
                id,
                &view,
                crate::StreamingChartOptions {
                    size: (320, 240),
                    clear_color: Color::WHITE,
                    max_primitives_per_chunk: 2,
                },
            );
            let gated = matches!(style, DrawStyle::Milkyway(_));
            if gated {
                assert!(
                    matches!(result, Err(FiggyError::InvalidSeriesConfig { .. })),
                    "unconnected arc path style={style:?} dash={dash:?}"
                );
                assert!(r.active_stream_job(id).is_none());
            } else {
                result.unwrap_or_else(|error| {
                    panic!("connected arc path {style:?} {dash:?}: {error}")
                });
                r.cancel_streaming_chart(id).unwrap();
            }
            r.remove_chart(id).unwrap();
        }
    }
    let mut series = declaration("precise-field", "x", "a");
    series.render_type = DataRenderType::Heatmap {
        matrix: MatrixRef {
            columns: vec!["a".into()],
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Edges,
        },
        fill: FieldFillConfig {
            mode: FillMode::Continuous,
            shading: Shading::Flat,
            opacity: 1.0,
        },
    };
    assert!(
        PrepareContext::validate_stream_series(&crate::default::default_config(), &series).is_ok()
    );
}

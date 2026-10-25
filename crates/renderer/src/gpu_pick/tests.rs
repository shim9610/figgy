#[cfg(feature = "cpu-twin")]
use std::collections::HashMap;

use super::engine::{GPU_PICK_GATE_WORD_POINTS, same_storage};
use super::types::{STYLE_MASK_RADIUS, STYLE_MASK_SHAPE};
use super::*;
use crate::InitPhase;
use crate::color::Color;
use crate::config::{AxisScale, Config};
use crate::data::{Column, split_f64_to_f32_pair};
use crate::data_config::{
    DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, ScatterShape, SeriesConfig,
};
#[cfg(feature = "cpu-twin")]
use crate::data_config::{DataScatterPointStyleConfig, DataScatterPointStyleOverride};
use crate::layout::{ChartArea, Rect};
use crate::line::LineStylePreset;
#[cfg(feature = "cpu-twin")]
use crate::pick::cpu_twin::{PointColumnLookup, PointPickOptions, pick_nearest_point};

/// The largest `visual_shape_radius` factor (the triangles').
const MAX_SHAPE_SCALE: f32 = 1.555_120_3;
/// `STROKE_HALF_PX`: an outline marker's stroke reaches this far either
/// side of its contour.
const STROKE_HALF: f32 = crate::data_render::MARKER_STROKE_HALF_PX;

#[cfg(feature = "cpu-twin")]
struct CpuColumns(HashMap<ColumnId, Vec<f32>>);

#[cfg(feature = "cpu-twin")]
impl CpuColumns {
    fn new(entries: &[(&str, &[f32])]) -> Self {
        Self(
            entries
                .iter()
                .map(|(id, values)| ((*id).to_owned(), values.to_vec()))
                .collect(),
        )
    }
}

#[cfg(feature = "cpu-twin")]
impl PointColumnLookup for CpuColumns {
    fn get_f32_column(&self, id: &ColumnId) -> Option<&[f32]> {
        self.0.get(id).map(Vec::as_slice)
    }
}

fn f32_column(values: Vec<f32>) -> Column<f32> {
    let min = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    Column {
        data: values,
        min,
        max,
    }
}

fn f64_column(values: Vec<f64>) -> Column<f64> {
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column {
        data: values,
        min,
        max,
    }
}

fn parity_config(width: u32, height: u32, x: (f64, f64), y: (f64, f64)) -> Config {
    let mut config = crate::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width,
        height,
    });
    config.bottom_x.min = x.0;
    config.bottom_x.max = x.1;
    config.left_y.min = y.0;
    config.left_y.max = y.1;
    config.chart_title.top_margin = 0.0;
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.out_margin = 0.0;
        axis.major_tick_length = 0.0;
    }
    config
}

fn query_from_config(
    config: &Config,
    canvas_position_px: [f32; 2],
    max_distance_px: f32,
) -> GpuPickQuery {
    let chart = config.chart_area.0;
    let data_area_px = config.data_area().ok().map(|area| {
        let rect = area.0;
        [
            rect.x as f32,
            rect.y as f32,
            rect.width as f32,
            rect.height as f32,
        ]
    });
    GpuPickQuery {
        transform: crate::data_render::scatter_transform_from_config(config),
        chart_rect_px: [
            chart.x as f32,
            chart.y as f32,
            chart.width as f32,
            chart.height as f32,
        ],
        data_area_px,
        canvas_position_px,
        max_distance_px,
        draw_style: GpuPickDrawStyle::PRECISE,
    }
}

fn resolve_pick(
    engine: &GpuPickEngine,
    pool: &ColumnPool,
    query: GpuPickQuery,
) -> Option<PickedPoint> {
    pollster::block_on(engine.pick(pool, query).unwrap().resolve()).unwrap()
}

fn resolve_pick_scaled(
    engine: &GpuPickEngine,
    pool: &ColumnPool,
    query: GpuPickQuery,
    display_scale: f32,
) -> Option<PickedPoint> {
    pollster::block_on(
        engine
            .pick_with_display_scale(pool, query, display_scale)
            .unwrap()
            .resolve(),
    )
    .unwrap()
}

#[cfg(feature = "cpu-twin")]
fn cpu_pick(
    config: &Config,
    series: &[SeriesConfig],
    columns: &CpuColumns,
    canvas_position_px: [f32; 2],
    max_distance_px: f32,
) -> Option<PickedPoint> {
    pick_nearest_point(
        config,
        series,
        columns,
        canvas_position_px[0],
        canvas_position_px[1],
        PointPickOptions { max_distance_px },
    )
}

#[cfg(feature = "cpu-twin")]
fn assert_pick_parity(cpu: Option<PickedPoint>, gpu: Option<PickedPoint>) {
    match (cpu, gpu) {
        (None, None) => {}
        (Some(cpu), Some(gpu)) => {
            assert_eq!(gpu.source_id, cpu.source_id);
            assert_eq!(gpu.series_id, cpu.series_id);
            assert_eq!(gpu.point_index, cpu.point_index);
            assert!(
                (gpu.distance_px - cpu.distance_px).abs() <= 0.001,
                "GPU distance {} differs from CPU distance {}",
                gpu.distance_px,
                cpu.distance_px
            );
        }
        (cpu, gpu) => panic!("CPU/GPU picker mismatch: CPU={cpu:?}, GPU={gpu:?}"),
    }
}

#[cfg(feature = "cpu-twin")]
fn scatter_series(
    series_id: &str,
    source_id: Option<&str>,
    x_column: &str,
    y_column: &str,
    scatter: DataScatterStyleConfig,
) -> SeriesConfig {
    SeriesConfig {
        series_id: series_id.to_owned(),
        source_id: source_id.map(str::to_owned),
        label: None,
        x_column: x_column.to_owned(),
        y_column: y_column.to_owned(),
        render_type: DataRenderType::Scatter { scatter },
    }
}

#[cfg(feature = "cpu-twin")]
fn line_series(
    series_id: &str,
    source_id: Option<&str>,
    x_column: &str,
    y_column: &str,
    line_width: f32,
) -> SeriesConfig {
    SeriesConfig {
        series_id: series_id.to_owned(),
        source_id: source_id.map(str::to_owned),
        label: None,
        x_column: x_column.to_owned(),
        y_column: y_column.to_owned(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }
}

fn scatter_descriptor(
    series_id: &str,
    x_column: &str,
    y_column: &str,
) -> GpuPickSeriesDescriptor<'static> {
    GpuPickSeriesDescriptor {
        source_id: Some(format!("source-{series_id}")),
        series_id: series_id.to_owned(),
        x_column: x_column.to_owned(),
        y_column: y_column.to_owned(),
        scatter: Some(GpuPickScatter {
            base_radius_px: 2.0,
            base_shape_id: 5,
            style_map: None,
            size_map: None,
        }),
        line_width_px: None,
        line_closed: false,
        line_interpolation: crate::data_config::LineInterpolation::Linear,
        line_style: crate::line::LineStylePreset::Solid,
        vector: None,
        errorbar: None,
        area: None,
    }
}

fn test_query(canvas_position_px: [f32; 2]) -> GpuPickQuery {
    GpuPickQuery {
        transform: ScatterTransform {
            data_min: [0.0, 0.0],
            data_max: [10.0, 10.0],
            data_min_lo: [0.0; 2],
            data_max_lo: [0.0; 2],
            scale_log: [0.0; 2],
            pixel_to_ndc: [0.02; 2],
            data_to_panel_scale: [1.0; 2],
            data_to_panel_offset: [0.0; 2],
            style_params: [[0.0; 4]; 3],
            polar_angle: [0.0; 4],
            polar_extent: [0.0; 4],
        },
        chart_rect_px: [0.0, 0.0, 100.0, 100.0],
        data_area_px: Some([0.0, 0.0, 100.0, 100.0]),
        canvas_position_px,
        max_distance_px: 0.0,
        draw_style: GpuPickDrawStyle::PRECISE,
    }
}

fn handle_snapshot(handle: ColumnHandle) -> (u32, u64, u64, usize) {
    (
        handle.generation,
        handle.offset,
        handle.byte_size,
        handle.len_values,
    )
}

#[derive(Debug, PartialEq)]
struct PickSeriesMetadataSnapshot {
    source_id: Option<String>,
    series_id: String,
    x_column: ColumnId,
    y_column: ColumnId,
    style_index_column: Option<ColumnId>,
    x_handle: (u32, u64, u64, usize),
    y_handle: (u32, u64, u64, usize),
    style_index_handle: Option<(u32, u64, u64, usize)>,
    pool_layout_generation: u64,
    x_allocation_epoch: u64,
    y_allocation_epoch: u64,
    style_index_allocation_epoch: Option<u64>,
    point_count: u32,
    gate_word_count: u32,
    dispatch_x: u32,
    dispatch_y: u32,
    direct_scan: bool,
    flags: u32,
    base_radius_bits: u32,
    base_shape_id: u32,
    line_half_width_bits: u32,
    max_extent_bits: u32,
    style_count: u32,
    override_count: u32,
    style_index_base: u32,
    style_index_len: u32,
    exact_key: PickExactKey,
}

impl PickSeriesMetadataSnapshot {
    fn capture(slot: &PickSeriesSlot) -> Self {
        let series = &slot.gpu;
        Self {
            source_id: slot.identity.source_id.clone(),
            series_id: slot.identity.series_id.clone(),
            x_column: series.x_column.clone(),
            y_column: series.y_column.clone(),
            style_index_column: series.style_index_column.clone(),
            x_handle: handle_snapshot(series.x_handle),
            y_handle: handle_snapshot(series.y_handle),
            style_index_handle: series.style_index_handle.map(handle_snapshot),
            pool_layout_generation: series.pool_layout_generation,
            x_allocation_epoch: series.x_allocation_epoch,
            y_allocation_epoch: series.y_allocation_epoch,
            style_index_allocation_epoch: series.style_index_allocation_epoch,
            point_count: series.point_count,
            gate_word_count: series.gate_word_count,
            dispatch_x: series.dispatch_x,
            dispatch_y: series.dispatch_y,
            direct_scan: series.direct_scan,
            flags: series.flags,
            base_radius_bits: series.base_radius_px.to_bits(),
            base_shape_id: series.base_shape_id,
            line_half_width_bits: series.line_half_width_px.to_bits(),
            max_extent_bits: series.max_extent_px.to_bits(),
            style_count: series.style_count,
            override_count: series.override_count,
            style_index_base: series.style_index_base,
            style_index_len: series.style_index_len,
            exact_key: series.exact_key,
        }
    }
}

/// The per-series part of the exact variant follows the built slot: the
/// module from the primitive flags, the dash walk only for a dashed line,
/// the error-bar path only for a marker series with error bars, the fill
/// path only for a line with a fill, and the series' shape only without a
/// style map. The query's bits are added per pick (`series_exact_variant`).
#[test]
fn the_stored_exact_key_is_the_slots_module_dash_and_shape() {
    use super::exact_variant::{AREA, DASH, ERRORBAR};
    let key = |shader, bits, shape| PickExactKey {
        shader,
        bits,
        shape,
    };
    let any = PickExactKey::ANY_SHAPE;
    for (flags, dash_len, errorbar, area, expected) in [
        (FLAG_SCATTER, 0, false, false, key(PickShader::Marker, 0, 7)),
        (FLAG_SCATTER, 4, false, false, key(PickShader::Marker, 0, 7)),
        (
            FLAG_SCATTER | FLAG_STYLE_MAP,
            0,
            false,
            false,
            key(PickShader::Marker, 0, any),
        ),
        (FLAG_LINE, 0, false, false, key(PickShader::Line, 0, any)),
        (FLAG_LINE, 2, false, false, key(PickShader::Line, DASH, any)),
        (
            FLAG_SCATTER | FLAG_LINE,
            2,
            false,
            false,
            key(PickShader::MarkerLine, DASH, 7),
        ),
        (
            FLAG_SCATTER | FLAG_LINE | FLAG_STYLE_MAP,
            0,
            false,
            false,
            key(PickShader::MarkerLine, 0, any),
        ),
        (
            FLAG_SCATTER | FLAG_VECTOR,
            0,
            false,
            false,
            key(PickShader::Vector, 0, 7),
        ),
        (
            FLAG_SCATTER,
            0,
            true,
            false,
            key(PickShader::Marker, ERRORBAR, 7),
        ),
        (FLAG_SCATTER, 0, false, true, key(PickShader::Marker, 0, 7)),
        (FLAG_LINE, 0, true, true, key(PickShader::Line, AREA, any)),
        (
            FLAG_SCATTER | FLAG_LINE,
            3,
            true,
            true,
            key(PickShader::MarkerLine, DASH | ERRORBAR | AREA, 7),
        ),
    ] {
        assert_eq!(
            series_exact_key(flags, dash_len, 7, errorbar, area),
            expected,
            "flags {flags:#x}, dash length {dash_len}, errorbar {errorbar}, area {area}"
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
struct PickSeriesResourceSnapshot {
    query_params: wgpu::Buffer,
    gate_masks: wgpu::Buffer,
    workgroup_candidates: wgpu::Buffer,
    query_data_bg: wgpu::BindGroup,
    query_work_bg: wgpu::BindGroup,
    reduce_bg: wgpu::BindGroup,
    result: wgpu::Buffer,
}

impl PickSeriesResourceSnapshot {
    fn capture(slot: &PickSeriesSlot) -> Self {
        let series = &slot.gpu;
        Self {
            query_params: series.query_params.clone(),
            gate_masks: series.gate_masks.clone(),
            workgroup_candidates: series.workgroup_candidates.clone(),
            query_data_bg: series.query_data_bg.clone(),
            query_work_bg: series.query_work_bg.clone(),
            reduce_bg: series.reduce_bg.clone(),
            result: series.result.clone(),
        }
    }
}

struct PickEngineSnapshot {
    identities: Arc<[PickIdentity]>,
    metadata: Vec<PickSeriesMetadataSnapshot>,
    resources: Vec<PickSeriesResourceSnapshot>,
    reduce_candidates: wgpu::Buffer,
    reduce_final_result: wgpu::Buffer,
    reduce_bind_group: wgpu::BindGroup,
}

impl PickEngineSnapshot {
    fn capture(engine: &GpuPickEngine) -> Self {
        Self {
            identities: Arc::clone(&engine.registry.identities),
            metadata: engine
                .registry
                .slots
                .iter()
                .map(PickSeriesMetadataSnapshot::capture)
                .collect(),
            resources: engine
                .registry
                .slots
                .iter()
                .map(PickSeriesResourceSnapshot::capture)
                .collect(),
            reduce_candidates: engine.registry.reduce.candidates.clone(),
            reduce_final_result: engine.registry.reduce.final_result.clone(),
            reduce_bind_group: engine.registry.reduce.bind_group.clone(),
        }
    }

    fn assert_unchanged(&self, engine: &GpuPickEngine) {
        assert!(Arc::ptr_eq(&self.identities, &engine.registry.identities));
        assert_eq!(engine.registry.slots.len(), self.metadata.len());
        assert_eq!(
            engine
                .registry
                .slots
                .iter()
                .map(PickSeriesMetadataSnapshot::capture)
                .collect::<Vec<_>>(),
            self.metadata
        );
        assert_eq!(
            engine
                .registry
                .slots
                .iter()
                .map(PickSeriesResourceSnapshot::capture)
                .collect::<Vec<_>>(),
            self.resources
        );
        assert_eq!(engine.registry.reduce.candidates, self.reduce_candidates);
        assert_eq!(
            engine.registry.reduce.final_result,
            self.reduce_final_result
        );
        assert_eq!(engine.registry.reduce.bind_group, self.reduce_bind_group);
    }
}

#[test]
fn shared_bundle_creates_pipelines_once_for_multiple_engines() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut events = Vec::new();
    let bundle = PickPipelineBundle::new_observed(
        Arc::clone(&device),
        Arc::clone(&queue),
        Arc::new(GpuLedger::new()),
        &mut |event| events.push(event),
    )
    .unwrap();
    let observed_stages = events
        .iter()
        .filter(|event| event.phase == InitPhase::Started)
        .map(|event| event.stage)
        .collect::<Vec<_>>();
    assert_eq!(
        observed_stages,
        [
            "setup",
            "pick_gate_x",
            "pick_gate_y",
            "pick_reduce_candidates",
        ]
    );
    let event_count = events.len();

    let first = GpuPickEngine::from_bundle(Arc::clone(&bundle)).unwrap();
    let second = GpuPickEngine::from_bundle(Arc::clone(&bundle)).unwrap();
    assert_eq!(events.len(), event_count);
    assert!(Arc::ptr_eq(&first.bundle, &second.bundle));
    assert_eq!(first.bundle.gate_x, second.bundle.gate_x);
    assert_eq!(first.bundle.gate_y, second.bundle.gate_y);
    assert_eq!(first.bundle.reduce, second.bundle.reduce);
}

#[test]
fn streamed_exact_matches_resident_at_seams_with_global_styles_and_ties() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let slots = [ScatterStyleSlotGpu {
        color_premul: [0.0; 4],
        meta: [2.0, 1.0, 6.0, 0.0],
    }];
    let overrides = [ScatterStyleOverrideGpu {
        point_index: 5,
        _pad: [0; 3],
        color_premul: [0.0; 4],
        meta: [8.0, 2.0, 6.0, 0.0],
    }];
    let descriptor = |order: usize, scatter: bool| GpuPickSeriesDescriptor {
        source_id: Some("stream-source".into()),
        series_id: format!("stream-{order}"),
        x_column: "x".into(),
        y_column: "y".into(),
        scatter: scatter.then_some(GpuPickScatter {
            base_radius_px: 1.0,
            base_shape_id: 5,
            style_map: Some(GpuPickScatterStyle {
                style_index_column: Some("style".into()),
                style_slots: &slots,
                style_overrides: &overrides,
                style_meta: ScatterStyleMapMeta {
                    style_count: 1,
                    override_count: 1,
                    has_index: 1,
                    _pad: 0,
                },
            }),
            size_map: None,
        }),
        line_width_px: Some(2.0),
        line_closed: false,
        line_interpolation: crate::data_config::LineInterpolation::Linear,
        line_style: crate::line::LineStylePreset::Solid,
        vector: None,
        errorbar: None,
        area: None,
    };
    for logarithmic in [false, true] {
        let origin = if logarithmic {
            0.0
        } else {
            1_000_000_000_000.0
        };
        let xs = [1.0, 2.0, 4.0, f64::NAN, 6.0, 8.0, 8.0, 9.0].map(|x| x + origin);
        let ys = [5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0];
        let style = [0.0f32, f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let mut config = parity_config(100, 100, (origin + 1.0, origin + 10.0), (0.0, 10.0));
        config.bottom_x.scale = if logarithmic {
            AxisScale::Logarithmic
        } else {
            AxisScale::Linear
        };
        config.bottom_x.inverted = logarithmic;
        config.left_y.inverted = logarithmic;
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            4096,
        )
        .unwrap();
        pool.add_hilo_column(
            "x".into(),
            &f64_column(xs.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_hilo_column(
            "y".into(),
            &f64_column(ys.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "style".into(),
            &f32_column(style.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        for scatter in [false, true] {
            let mut resident = GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
            for order in 0..2 {
                resident
                    .add_series(&pool, descriptor(order, scatter))
                    .unwrap();
            }
            for cursor in [
                [10.0, 50.0],
                [35.0, 50.0],
                [55.0, 50.0],
                [80.0, 54.0],
                [95.0, 50.0],
            ] {
                let query = query_from_config(&config, cursor, 4.0);
                let expected = resolve_pick(&resident, &pool, query);
                for chunk_size in [1usize, 2, 3, 5] {
                    let identities = (0..2)
                        .map(|order| (Some("stream-source".into()), format!("stream-{order}")))
                        .collect();
                    let mut exact = PickExactPipelines::default();
                    let mut stream = resident
                        .bundle
                        .begin_stream(query, 1.0, identities)
                        .unwrap();
                    let mut charges = vec![stream.charge()];
                    // Reverse both series and chunk submission order: ties
                    // are source identities, never scheduling order.
                    for order in (0..2).rev() {
                        let starts = (0..xs.len()).step_by(chunk_size).collect::<Vec<_>>();
                        for start in starts.into_iter().rev() {
                            let owned = chunk_size.min(xs.len() - start);
                            let end = (start + owned + 1).min(xs.len());
                            let len = end - start;
                            let mut pairs = Vec::new();
                            for value in &xs[start..end] {
                                let (hi, lo) = split_f64_to_f32_pair(*value);
                                pairs.push([hi, lo]);
                            }
                            for value in &ys[start..end] {
                                let (hi, lo) = split_f64_to_f32_pair(*value);
                                pairs.push([hi, lo]);
                            }
                            for value in &style[start..end] {
                                pairs.push([*value, 0.0]);
                            }
                            let buffer = charged_buffer_init(
                                &ChargeTally::new(),
                                &device,
                                &wgpu::util::BufferInitDescriptor {
                                    label: Some("stream pick test bounded source"),
                                    contents: bytemuck::cast_slice(&pairs),
                                    usage: wgpu::BufferUsages::STORAGE,
                                },
                            );
                            let handle = |column| ColumnHandle {
                                generation: 0,
                                offset: (column * len * 8) as u64,
                                byte_size: (len * 8) as u64,
                                len_values: len,
                            };
                            let mut encoder = device.create_command_encoder(&Default::default());
                            if let Some(charge) = stream
                                .encode_chunk(
                                    &mut exact,
                                    &mut encoder,
                                    &buffer,
                                    descriptor(order, scatter),
                                    GpuStreamPickColumns {
                                        x: handle(0),
                                        y: handle(1),
                                        style_index: scatter.then(|| handle(2)),
                                        size_value: None,
                                        vector: [None; 3],
                                    },
                                    order as u32,
                                    start as u32,
                                    owned as u32,
                                    owned.min(xs.len() - start - 1) as u32,
                                )
                                .unwrap()
                            {
                                charges.push(charge);
                            }
                            queue.submit([encoder.finish()]);
                        }
                    }
                    let actual = pollster::block_on(stream.finish().unwrap().resolve()).unwrap();
                    // Bit parity needs the same compiled code: every
                    // chunk scanned with its resident series' variant.
                    // Drivers that compile two variants differently
                    // (Metal, DX12) otherwise round differently.
                    let resident_keys: Vec<_> = resident.exact_variants(&query).collect();
                    assert!(!exact.pipelines.is_empty());
                    for (key, _) in &exact.pipelines {
                        assert!(
                            resident_keys.contains(key),
                            "{key:?} not in {resident_keys:?}"
                        );
                    }
                    assert_eq!(
                        expected, actual,
                        "stream parity logarithmic={logarithmic}, scatter={scatter}, cursor={cursor:?}, chunk_size={chunk_size}"
                    );
                    if let (Some(expected), Some(actual)) = (&expected, &actual) {
                        assert_eq!(expected.distance_px.to_bits(), actual.distance_px.to_bits());
                    }
                }
            }
        }
    }
}

#[test]
fn streamed_gates_keep_boundary_segment_and_exclude_unowned_halo() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let count = GPU_PICK_DIRECT_SCAN_POINTS as usize + 2;
    let mut pairs = vec![[f32::NAN, 0.0]; count * 2];
    pairs[count - 2] = [1.0, 0.0];
    pairs[count - 1] = [9.0, 0.0];
    pairs[2 * count - 2] = [5.0, 0.0];
    pairs[2 * count - 1] = [5.0, 0.0];
    let buffer = charged_buffer_init(
        &ChargeTally::new(),
        &device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("stream gated pick source"),
            contents: bytemuck::cast_slice(&pairs),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let columns = GpuStreamPickColumns {
        x: ColumnHandle {
            generation: 0,
            offset: 0,
            byte_size: count as u64 * 8,
            len_values: count,
        },
        y: ColumnHandle {
            generation: 0,
            offset: count as u64 * 8,
            byte_size: count as u64 * 8,
            len_values: count,
        },
        style_index: None,
        size_value: None,
        vector: [None; 3],
    };
    let bundle = PickPipelineBundle::new_observed(
        Arc::clone(&device),
        Arc::clone(&queue),
        Arc::new(GpuLedger::new()),
        &mut |_| {},
    )
    .unwrap();
    // Exact centerline hits must survive reciprocal rounding; near misses
    // and points beyond either endpoint must not become infinite-line hits.
    // A zero-width line is drawn one pixel wide, so a quarter pixel off
    // the centreline is on the stroke and three quarters is not.
    // The right halo is available to the last segment, never to scatter.
    for (line, cursor, expected_local_index) in [
        (false, [90.0, 50.0], None),
        (false, [10.0, 50.0], Some(count - 2)),
        (true, [50.0, 50.0], Some(count - 2)),
        (true, [30.0, 50.0], Some(count - 2)),
        (true, [70.0, 50.0], Some(count - 1)),
        (true, [50.0, 50.25], Some(count - 2)),
        (true, [50.0, 50.75], None),
        (true, [5.0, 50.0], None),
        (true, [95.0, 50.0], None),
    ] {
        let descriptor = GpuPickSeriesDescriptor {
            source_id: None,
            series_id: "gated-stream".into(),
            x_column: "x".into(),
            y_column: "y".into(),
            scatter: (!line).then_some(GpuPickScatter {
                base_radius_px: 2.0,
                base_shape_id: 5,
                style_map: None,
                size_map: None,
            }),
            line_width_px: line.then_some(0.0),
            line_closed: false,
            line_interpolation: crate::data_config::LineInterpolation::Linear,
            line_style: crate::line::LineStylePreset::Solid,
            vector: None,
            errorbar: None,
            area: None,
        };
        let query = test_query(cursor);
        let mut exact = PickExactPipelines::default();
        let mut stream = bundle
            .begin_stream(query, 1.0, vec![(None, "gated-stream".into())])
            .unwrap();
        assert_eq!(
            stream.charge().charged_bytes(),
            GpuStreamPick::PERSISTENT_BYTES
        );
        let bytes = stream.chunk_bytes(&descriptor, count).unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        let scratch = stream
            .encode_chunk(
                &mut exact,
                &mut encoder,
                &buffer,
                descriptor,
                columns,
                0,
                1000,
                (count - 1) as u32,
                (count - 1) as u32,
            )
            .unwrap()
            .unwrap();
        assert_eq!(scratch.charged_bytes(), bytes);
        queue.submit([encoder.finish()]);
        let answer = pollster::block_on(stream.finish().unwrap().resolve()).unwrap();
        if let Some(expected_local_index) = expected_local_index {
            let answer = answer.unwrap_or_else(|| {
                panic!("owned primitive at cursor was missed: line={line}, cursor={cursor:?}")
            });
            assert_eq!(answer.point_index, 1000 + expected_local_index);
            assert_eq!(answer.distance_px, 0.0);
        } else {
            assert_eq!(
                answer, None,
                "unexpected hit: line={line}, cursor={cursor:?}"
            );
        }
    }
}

#[test]
fn prepared_registry_drop_preserves_exact_state() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "transition-drop-x".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "transition-drop-y".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            scatter_descriptor("drop-old", "transition-drop-x", "transition-drop-y"),
        )
        .unwrap();
    let snapshot = PickEngineSnapshot::capture(&engine);

    let transition = engine
        .prepare_registry_transition(
            &pool,
            [GpuPickRegistrySlot::Build(scatter_descriptor(
                "drop-next",
                "transition-drop-x",
                "transition-drop-y",
            ))],
        )
        .unwrap();
    drop(transition);

    snapshot.assert_unchanged(&engine);
    assert_eq!(
        resolve_pick(&engine, &pool, test_query([50.0, 50.0]))
            .unwrap()
            .series_id,
        "drop-old"
    );
}

#[test]
fn source_identity_transition_reuses_every_gpu_resource() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "identity-x".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "identity-y".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            scatter_descriptor("identity-series", "identity-x", "identity-y"),
        )
        .unwrap();
    let resources = PickSeriesResourceSnapshot::capture(&engine.registry.slots[0]);
    let reduce = engine.registry.reduce.clone();

    engine
        .prepare_registry_transition(
            &pool,
            [GpuPickRegistrySlot::Reuse {
                current_index: 0,
                source_id: Some("identity-source-next".into()),
                series_id: "identity-series".into(),
            }],
        )
        .unwrap()
        .commit();

    assert_eq!(
        PickSeriesResourceSnapshot::capture(&engine.registry.slots[0]),
        resources
    );
    assert_eq!(engine.registry.reduce.candidates, reduce.candidates);
    assert_eq!(engine.registry.reduce.final_result, reduce.final_result);
    assert_eq!(engine.registry.reduce.bind_group, reduce.bind_group);
    let picked = resolve_pick(&engine, &pool, test_query([50.0, 50.0])).unwrap();
    assert_eq!(picked.source_id.as_deref(), Some("identity-source-next"));
    assert_eq!(picked.series_id, "identity-series");
}

#[test]
fn foreign_pool_with_matching_numeric_stamps_is_rejected() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let make_pool = || {
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "foreign-x".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "foreign-y".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool
    };
    let first_pool = make_pool();
    let second_pool = make_pool();
    assert_eq!(
        first_pool.layout_generation(),
        second_pool.layout_generation()
    );
    assert_eq!(
        first_pool.allocation_epoch("foreign-x"),
        second_pool.allocation_epoch("foreign-x")
    );
    assert_eq!(
        handle_snapshot(first_pool.handle_for("foreign-x").unwrap()),
        handle_snapshot(second_pool.handle_for("foreign-x").unwrap())
    );

    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &first_pool,
            scatter_descriptor("foreign", "foreign-x", "foreign-y"),
        )
        .unwrap();
    assert!(matches!(
        engine.pick(&second_pool, test_query([50.0, 50.0])),
        Err(GpuPickError::ForeignColumnPool)
    ));
    let transition = engine.prepare_registry_transition(
        &second_pool,
        [GpuPickRegistrySlot::Reuse {
            current_index: 0,
            source_id: None,
            series_id: "foreign".into(),
        }],
    );
    assert!(matches!(transition, Err(GpuPickError::ForeignColumnPool)));
}

#[test]
fn reduce_resources_grow_only_when_live_count_exceeds_capacity() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "grow-x".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "grow-y".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    for id in ["grow-a", "grow-b"] {
        engine
            .add_series(&pool, scatter_descriptor(id, "grow-x", "grow-y"))
            .unwrap();
    }
    assert_eq!(engine.registry.reduce.capacity, 2);
    let at_two = engine.registry.reduce.candidates.clone();

    engine
        .replace_series_at(0, &pool, scatter_descriptor("grow-a2", "grow-x", "grow-y"))
        .unwrap();
    assert_eq!(engine.registry.reduce.candidates, at_two);
    engine
        .prepare_registry_transition(
            &pool,
            [
                GpuPickRegistrySlot::Reuse {
                    current_index: 1,
                    source_id: None,
                    series_id: "grow-b".into(),
                },
                GpuPickRegistrySlot::Reuse {
                    current_index: 0,
                    source_id: None,
                    series_id: "grow-a2".into(),
                },
            ],
        )
        .unwrap()
        .commit();
    assert_eq!(engine.registry.reduce.candidates, at_two);
    engine.remove_series_at(1).unwrap();
    assert_eq!(engine.registry.reduce.capacity, 2);
    assert_eq!(engine.registry.reduce.candidates, at_two);
    engine
        .add_series(&pool, scatter_descriptor("grow-c", "grow-x", "grow-y"))
        .unwrap();
    assert_eq!(engine.registry.reduce.candidates, at_two);
    engine
        .add_series(&pool, scatter_descriptor("grow-d", "grow-x", "grow-y"))
        .unwrap();
    assert_eq!(engine.registry.reduce.capacity, 3);
    assert_ne!(engine.registry.reduce.candidates, at_two);
}

#[cfg(feature = "cpu-twin")]
#[test]
fn exact_gpu_scatter_style_mapping_matches_cpu_exhaustive_picker() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let config = parity_config(100, 100, (0.0, 10.0), (0.0, 10.0));
    let xs = [2.0, 5.0, 8.0];
    let ys = [5.0, 5.0, 5.0];
    let style_indices = [1.0, 0.0, 99.0];
    let cpu_columns = CpuColumns::new(&[
        ("parity-sx", &xs),
        ("parity-sy", &ys),
        ("parity-si", &style_indices),
    ]);

    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    for (id, values) in [
        ("parity-sx", xs.as_slice()),
        ("parity-sy", ys.as_slice()),
        ("parity-si", style_indices.as_slice()),
    ] {
        pool.add_column(
            id.into(),
            &f32_column(values.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    }

    let scatter = DataScatterStyleConfig {
        point_color: Color::BLACK,
        point_shape: ScatterShape::CircleFilled,
        point_size: 1.0,
        point_style_table: Some(vec![
            DataScatterPointStyleConfig {
                point_color: None,
                point_shape: Some(ScatterShape::SquareFilled),
                point_size: Some(2.0),
            },
            DataScatterPointStyleConfig {
                point_color: None,
                point_shape: Some(ScatterShape::TriangleFilled),
                point_size: Some(3.0),
            },
        ]),
        point_style_index_column: Some("parity-si".into()),
        point_style_overrides: Some(vec![DataScatterPointStyleOverride {
            index: 1,
            style: DataScatterPointStyleConfig {
                point_color: None,
                point_shape: Some(ScatterShape::DiamondFilled),
                point_size: Some(5.0),
            },
        }]),
        point_color_map: None,
        point_size_map: None,
    };
    let cpu_series = [scatter_series(
        "mapped",
        Some("mapped-source"),
        "parity-sx",
        "parity-sy",
        scatter,
    )];
    let slots = [
        ScatterStyleSlotGpu {
            color_premul: [0.0; 4],
            meta: [
                2.0,
                crate::data_render::shape_id(&ScatterShape::SquareFilled) as f32,
                (STYLE_MASK_RADIUS | STYLE_MASK_SHAPE) as f32,
                0.0,
            ],
        },
        ScatterStyleSlotGpu {
            color_premul: [0.0; 4],
            meta: [
                3.0,
                crate::data_render::shape_id(&ScatterShape::TriangleFilled) as f32,
                (STYLE_MASK_RADIUS | STYLE_MASK_SHAPE) as f32,
                0.0,
            ],
        },
    ];
    let overrides = [ScatterStyleOverrideGpu {
        point_index: 1,
        _pad: [0; 3],
        color_premul: [0.0; 4],
        meta: [
            5.0,
            crate::data_render::shape_id(&ScatterShape::DiamondFilled) as f32,
            (STYLE_MASK_RADIUS | STYLE_MASK_SHAPE) as f32,
            0.0,
        ],
    }];
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: Some("mapped-source".into()),
                series_id: "mapped".into(),
                x_column: "parity-sx".into(),
                y_column: "parity-sy".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 1.0,
                    base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                    style_map: Some(GpuPickScatterStyle {
                        style_index_column: Some("parity-si".into()),
                        style_slots: &slots,
                        style_overrides: &overrides,
                        style_meta: ScatterStyleMapMeta {
                            style_count: slots.len() as u32,
                            override_count: overrides.len() as u32,
                            has_index: 1,
                            _pad: 0,
                        },
                    }),
                    size_map: None,
                }),
                line_width_px: None,
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();

    // Point 0 is a filled triangle of visual radius 3 * 1.555 with its
    // apex up; 3 px above its centre is inside it, while the disc the
    // old picker used would also have taken 4.5 px to the right, where
    // the triangle draws nothing. Point 1 is a filled diamond of visual
    // radius 5 * 1.253 = 6.27, so 6 px right of its centre is inside.
    for (cursor, expected_index) in [([20.0, 47.0], 0), ([56.0, 50.0], 1)] {
        let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
        assert_pick_parity(cpu.clone(), gpu);
        assert_eq!(cpu.unwrap().point_index, expected_index);
    }

    // Beside the triangle, inside its circumscribed disc, nothing is
    // drawn and nothing is picked.
    let cursor = [24.5, 50.0];
    let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
    assert!(cpu.is_none(), "{cpu:?}");
    let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
    assert_pick_parity(cpu, gpu);

    // An out-of-range style-table index falls back to the one-pixel base
    // circle; 1.25 px outside its center must therefore miss exactly.
    let cursor = [81.25, 50.0];
    let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
    let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
    assert_pick_parity(cpu, gpu);
}

/// A `point_size_map` resolves each point's pick radius from its value on
/// both pickers: the domain ends map to `min_px` / `max_px`, and a
/// non-finite value hides the point from both.
#[cfg(feature = "cpu-twin")]
#[test]
fn size_map_radius_parity_between_cpu_and_gpu_pickers() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let config = parity_config(200, 100, (0.0, 20.0), (0.0, 10.0));
    let xs = [2.0, 10.0, 18.0];
    let ys = [5.0, 5.0, 5.0];
    let sizes = [0.0, 10.0, f32::NAN];
    let cpu_columns = CpuColumns::new(&[("size-x", &xs), ("size-y", &ys), ("size-s", &sizes)]);

    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    for (id, values) in [
        ("size-x", xs.as_slice()),
        ("size-y", ys.as_slice()),
        ("size-s", sizes.as_slice()),
    ] {
        pool.add_column(
            id.into(),
            &f32_column(values.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    }

    let size_map = crate::data_config::PointSizeMap {
        column: "size-s".into(),
        domain_min: 0.0,
        domain_max: 10.0,
        min_px: 1.0,
        max_px: 5.0,
        scale: crate::data_config::SizeScale::Radius,
    };
    let scatter = DataScatterStyleConfig {
        point_color: Color::BLACK,
        point_shape: ScatterShape::CircleFilled,
        point_size: 3.0,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: Some(size_map.clone()),
    };
    let cpu_series = [scatter_series("sized", None, "size-x", "size-y", scatter)];
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: None,
                series_id: "sized".into(),
                x_column: "size-x".into(),
                y_column: "size-y".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 3.0,
                    base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                    style_map: None,
                    size_map: Some(GpuPickSizeMap {
                        column: size_map.column.clone(),
                        domain_min: size_map.domain_min,
                        domain_max: size_map.domain_max,
                        min_px: size_map.min_px,
                        max_px: size_map.max_px,
                        area: false,
                    }),
                }),
                line_width_px: None,
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();

    // Point 0 at 20 px with a 1 px radius, point 1 at 100 px with 5 px.
    // Every hit sits strictly inside the 1 px pick distance: a cursor on
    // the boundary itself depends on backend rounding, which the exact
    // pass does not pad.
    for (cursor, expected) in [
        ([21.5, 50.0], Some((0, 0.5))),
        ([24.5, 50.0], None),
        ([105.5, 50.0], Some((1, 0.5))),
        ([107.5, 50.0], None),
        // Point 2 is hidden by its NaN size: nothing to hit at its center.
        ([180.0, 50.0], None),
    ] {
        let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 1.0);
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 1.0));
        assert_pick_parity(cpu.clone(), gpu);
        match expected {
            Some((index, distance)) => {
                let cpu = cpu.expect("a hit");
                assert_eq!(cpu.point_index, index);
                assert!((cpu.distance_px - distance).abs() < 1e-3);
            }
            None => assert!(cpu.is_none(), "unexpected hit at {cursor:?}: {cpu:?}"),
        }
    }
}

#[test]
fn display_scale_applies_once_to_pick_geometry_but_not_pick_distance() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let config = parity_config(200, 100, (0.0, 20.0), (0.0, 10.0));
    let base_x = [2.0];
    let base_y = [5.0];
    let mapped_x = [10.0];
    let mapped_y = [5.0];
    let mapped_style = [0.0];
    let line_x = [15.0, 17.0];
    let line_y = [2.0, 2.0];

    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    for (id, values) in [
        ("scale-base-x", base_x.as_slice()),
        ("scale-base-y", base_y.as_slice()),
        ("scale-mapped-x", mapped_x.as_slice()),
        ("scale-mapped-y", mapped_y.as_slice()),
        ("scale-style", mapped_style.as_slice()),
        ("scale-line-x", line_x.as_slice()),
        ("scale-line-y", line_y.as_slice()),
    ] {
        pool.add_column(
            id.into(),
            &f32_column(values.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    }

    let mapped_slots = [ScatterStyleSlotGpu {
        color_premul: [0.0; 4],
        meta: [6.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
    }];
    let mapped_overrides = [ScatterStyleOverrideGpu {
        point_index: 0,
        _pad: [0; 3],
        color_premul: [0.0; 4],
        meta: [
            0.0,
            crate::data_render::shape_id(&ScatterShape::Triangle) as f32,
            STYLE_MASK_SHAPE as f32,
            0.0,
        ],
    }];
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: None,
                series_id: "scaled-base".into(),
                x_column: "scale-base-x".into(),
                y_column: "scale-base-y".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 4.0,
                    base_shape_id: crate::data_render::shape_id(&ScatterShape::Circle),
                    style_map: None,
                    size_map: None,
                }),
                line_width_px: None,
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: None,
                series_id: "scaled-mapped".into(),
                x_column: "scale-mapped-x".into(),
                y_column: "scale-mapped-y".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 1.0,
                    base_shape_id: crate::data_render::shape_id(&ScatterShape::Circle),
                    style_map: Some(GpuPickScatterStyle {
                        style_index_column: Some("scale-style".into()),
                        style_slots: &mapped_slots,
                        style_overrides: &mapped_overrides,
                        style_meta: ScatterStyleMapMeta {
                            style_count: 1,
                            override_count: 1,
                            has_index: 1,
                            _pad: 0,
                        },
                    }),
                    size_map: None,
                }),
                line_width_px: None,
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: None,
                series_id: "scaled-line".into(),
                x_column: "scale-line-x".into(),
                y_column: "scale-line-y".into(),
                scatter: None,
                line_width_px: Some(6.0),
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();

    for display_scale in [0.5, 1.0, 2.0] {
        // The base marker is an outline circle: a stroke of half-width
        // `STROKE_HALF` (physical pixels, unscaled) centred on the scaled
        // radius, hollow inside.
        let base_radius = 4.0 * display_scale;
        let on_stroke_inside = query_from_config(
            &config,
            [20.0 + base_radius - STROKE_HALF + 0.25, 50.0],
            0.0,
        );
        let on_stroke_outside = query_from_config(
            &config,
            [20.0 + base_radius + STROKE_HALF - 0.25, 50.0],
            0.0,
        );
        let base_outside = query_from_config(
            &config,
            [20.0 + base_radius + STROKE_HALF + 0.25, 50.0],
            0.0,
        );
        for hit in [on_stroke_inside, on_stroke_outside] {
            assert_eq!(
                resolve_pick_scaled(&engine, &pool, hit, display_scale)
                    .unwrap()
                    .series_id,
                "scaled-base"
            );
        }
        assert!(resolve_pick_scaled(&engine, &pool, base_outside, display_scale).is_none());
        if base_radius > STROKE_HALF + 0.5 {
            let hollow = query_from_config(&config, [20.0, 50.0], 0.0);
            assert!(
                resolve_pick_scaled(&engine, &pool, hollow, display_scale).is_none(),
                "the hollow centre of an outline circle draws nothing"
            );
        }

        // The mapped marker is an outline triangle of reference radius 6:
        // its apex sits straight up (canvas y decreasing) at the visual
        // radius.
        let mapped_radius = 6.0 * MAX_SHAPE_SCALE * display_scale;
        let mapped_inside = query_from_config(&config, [100.0, 50.0 - mapped_radius + 0.25], 0.0);
        let mapped_outside = query_from_config(
            &config,
            [100.0, 50.0 - mapped_radius - STROKE_HALF - 0.25],
            0.0,
        );
        assert_eq!(
            resolve_pick_scaled(&engine, &pool, mapped_inside, display_scale)
                .unwrap()
                .series_id,
            "scaled-mapped"
        );
        assert!(resolve_pick_scaled(&engine, &pool, mapped_outside, display_scale).is_none());

        let line_half_width = 3.0 * display_scale;
        let line_inside = query_from_config(&config, [160.0, 80.0 + line_half_width - 0.25], 0.0);
        let line_outside = query_from_config(&config, [160.0, 80.0 + line_half_width + 0.25], 0.0);
        assert_eq!(
            resolve_pick_scaled(&engine, &pool, line_inside, display_scale)
                .unwrap()
                .series_id,
            "scaled-line"
        );
        assert!(resolve_pick_scaled(&engine, &pool, line_outside, display_scale).is_none());

        let physical_gap = 2.5;
        let edge = base_radius + STROKE_HALF;
        let max_distance_miss = query_from_config(&config, [20.0 + edge + physical_gap, 50.0], 2.0);
        let max_distance_hit = query_from_config(&config, [20.0 + edge + physical_gap, 50.0], 3.0);
        assert!(resolve_pick_scaled(&engine, &pool, max_distance_miss, display_scale).is_none());
        let picked = resolve_pick_scaled(&engine, &pool, max_distance_hit, display_scale).unwrap();
        assert_eq!(picked.series_id, "scaled-base");
        assert!((picked.distance_px - physical_gap).abs() <= 0.001);
    }
}

#[test]
fn invalid_display_scales_fail_closed_without_gpu_submission() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "invalid-scale-x".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "invalid-scale-y".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            scatter_descriptor("invalid-scale", "invalid-scale-x", "invalid-scale-y"),
        )
        .unwrap();

    for display_scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(
            resolve_pick_scaled(&engine, &pool, test_query([50.0, 50.0]), display_scale,).is_none()
        );
    }
}

#[cfg(feature = "cpu-twin")]
#[test]
fn exact_gpu_line_boundary_endpoint_matches_cpu_exhaustive_picker() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let config = parity_config(650, 100, (0.0, 65.0), (0.0, 10.0));
    let xs = (0..66).map(|value| value as f32).collect::<Vec<_>>();
    let ys = vec![5.0; xs.len()];
    let cpu_columns =
        CpuColumns::new(&[("parity-lx", xs.as_slice()), ("parity-ly", ys.as_slice())]);
    let cpu_series = [line_series(
        "boundary-line",
        Some("line-source"),
        "parity-lx",
        "parity-ly",
        0.0,
    )];

    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "parity-lx".into(),
        &f32_column(xs),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "parity-ly".into(),
        &f32_column(ys),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: Some("line-source".into()),
                series_id: "boundary-line".into(),
                x_column: "parity-lx".into(),
                y_column: "parity-ly".into(),
                scatter: None,
                line_width_px: Some(0.0),
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();

    // Segment start 63 and endpoint 64 straddle two 32-index gate words.
    // The start word must still retain and exact-test that segment.
    for (cursor, expected_index) in [
        ([634.998, 50.0], 63),
        ([635.0, 50.0], 63),
        ([635.002, 50.0], 64),
        ([637.5, 50.0], 64),
    ] {
        let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.01);
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.01));
        assert_pick_parity(cpu.clone(), gpu);
        assert_eq!(cpu.unwrap().point_index, expected_index);
    }
}

#[cfg(feature = "cpu-twin")]
#[test]
fn exact_gpu_line_gate_keeps_a_segment_whose_endpoints_are_both_outside() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let config = parity_config(100, 100, (0.0, 10.0), (0.0, 10.0));
    let xs = [0.0, 10.0];
    let ys = [0.0, 10.0];
    let cpu_columns = CpuColumns::new(&[
        ("gate-line-x", xs.as_slice()),
        ("gate-line-y", ys.as_slice()),
    ]);
    let cpu_series = [line_series(
        "gate-line",
        None,
        "gate-line-x",
        "gate-line-y",
        0.0,
    )];

    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "gate-line-x".into(),
        &f32_column(xs.to_vec()),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "gate-line-y".into(),
        &f32_column(ys.to_vec()),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: None,
                series_id: "gate-line".into(),
                x_column: "gate-line-x".into(),
                y_column: "gate-line-y".into(),
                scatter: None,
                line_width_px: Some(0.0),
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();

    let cursor = [50.0, 50.0];
    let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.01);
    let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.01));
    assert_pick_parity(cpu, gpu);
}

#[test]
fn two_dimensional_gate_dispatch_returns_the_original_index() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let point_count = GPU_PICK_DIRECT_SCAN_POINTS as usize + 1;
    let xs = (0..point_count)
        .map(|index| ((index * 2_053) % point_count) as f32)
        .collect::<Vec<_>>();
    let ys = (0..point_count)
        .map(|index| ((index * 12_337) % point_count) as f32)
        .collect::<Vec<_>>();
    let target_index = point_count / 2;
    let axis_max = (point_count - 1) as f32;
    let cursor = [
        xs[target_index] / axis_max * 100.0,
        (1.0 - ys[target_index] / axis_max) * 100.0,
    ];
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        2 * 1024 * 1024,
    )
    .unwrap();
    pool.add_column(
        "gate-2d-x".into(),
        &f32_column(xs),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "gate-2d-y".into(),
        &f32_column(ys),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: None,
                series_id: "gate-2d".into(),
                x_column: "gate-2d-x".into(),
                y_column: "gate-2d-y".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 0.0001,
                    base_shape_id: 5,
                    style_map: None,
                    size_map: None,
                }),
                line_width_px: None,
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();

    let groups = engine.registry.slots[0]
        .gpu
        .gate_word_count
        .div_ceil(GPU_PICK_WORKGROUP_SIZE);
    assert_eq!(groups, 33);
    assert!(!engine.registry.slots[0].gpu.direct_scan);
    engine.registry.slots[0].gpu.dispatch_x = 1;
    engine.registry.slots[0].gpu.dispatch_y = groups;
    let picked = resolve_pick(
        &engine,
        &pool,
        GpuPickQuery {
            transform: ScatterTransform {
                data_min: [0.0, 0.0],
                data_max: [axis_max, axis_max],
                data_min_lo: [0.0; 2],
                data_max_lo: [0.0; 2],
                scale_log: [0.0; 2],
                pixel_to_ndc: [0.02; 2],
                data_to_panel_scale: [1.0; 2],
                data_to_panel_offset: [0.0; 2],
                style_params: [[0.0; 4]; 3],
                polar_angle: [0.0; 4],
                polar_extent: [0.0; 4],
            },
            chart_rect_px: [0.0, 0.0, 100.0, 100.0],
            data_area_px: Some([0.0, 0.0, 100.0, 100.0]),
            canvas_position_px: cursor,
            max_distance_px: 0.0,
            draw_style: GpuPickDrawStyle::PRECISE,
        },
    )
    .unwrap();
    assert_eq!(picked.point_index, target_index);
    assert_eq!(picked.distance_px, 0.0);
}

#[test]
fn overlapping_tickets_resolve_after_registry_engine_and_pool_drop() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let point_count = GPU_PICK_DIRECT_SCAN_POINTS as usize + 1;
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        2 * 1024 * 1024,
    )
    .unwrap();
    pool.add_column(
        "overlap-x".into(),
        &f32_column((0..point_count).map(|index| index as f32).collect()),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "overlap-y".into(),
        &f32_column(vec![5.0; point_count]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();

    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: Some("overlap-source".into()),
                series_id: "overlap-series".into(),
                x_column: "overlap-x".into(),
                y_column: "overlap-y".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 0.0001,
                    base_shape_id: 5,
                    style_map: None,
                    size_map: None,
                }),
                line_width_px: None,
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();
    assert!(!engine.registry.slots[0].gpu.direct_scan);

    let query = |cursor_x| GpuPickQuery {
        transform: ScatterTransform {
            data_min: [0.0, 0.0],
            data_max: [(point_count - 1) as f32, 10.0],
            data_min_lo: [0.0; 2],
            data_max_lo: [0.0; 2],
            scale_log: [0.0; 2],
            pixel_to_ndc: [0.02; 2],
            data_to_panel_scale: [1.0; 2],
            data_to_panel_offset: [0.0; 2],
            style_params: [[0.0; 4]; 3],
            polar_angle: [0.0; 4],
            polar_extent: [0.0; 4],
        },
        chart_rect_px: [0.0, 0.0, 100.0, 100.0],
        data_area_px: Some([0.0, 0.0, 100.0, 100.0]),
        canvas_position_px: [cursor_x, 50.0],
        max_distance_px: 0.0,
        draw_style: GpuPickDrawStyle::PRECISE,
    };
    let left = engine.pick(&pool, query(25.0)).unwrap();
    let right = engine.pick(&pool, query(75.0)).unwrap();
    engine.clear_series();
    drop(engine);
    drop(pool);

    let right = pollster::block_on(right.resolve()).unwrap().unwrap();
    let left = pollster::block_on(left.resolve()).unwrap().unwrap();
    assert_eq!(right.source_id.as_deref(), Some("overlap-source"));
    assert_eq!(right.series_id, "overlap-series");
    assert_eq!(right.point_index, (point_count - 1) * 3 / 4);
    assert_eq!(right.distance_px, 0.0);
    assert_eq!(left.source_id.as_deref(), Some("overlap-source"));
    assert_eq!(left.series_id, "overlap-series");
    assert_eq!(left.point_index, (point_count - 1) / 4);
    assert_eq!(left.distance_px, 0.0);
}

#[cfg(feature = "cpu-twin")]
#[test]
fn exact_gpu_log_inverted_and_nonfinite_semantics_match_cpu_picker() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut config = parity_config(120, 100, (1.0, 1_000.0), (0.0, 10.0));
    config.bottom_x.scale = AxisScale::Logarithmic;
    config.left_y.inverted = true;
    let xs = [f32::NAN, f32::INFINITY, 0.0, -1.0, 10.0, 100.0];
    let ys = [5.0, 5.0, 5.0, 5.0, 2.0, 8.0];
    let cpu_columns = CpuColumns::new(&[("parity-log-x", &xs), ("parity-log-y", &ys)]);
    let scatter = DataScatterStyleConfig {
        point_color: Color::BLACK,
        point_shape: ScatterShape::CircleFilled,
        point_size: 2.0,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    };
    let cpu_series = [scatter_series(
        "log-inverted",
        None,
        "parity-log-x",
        "parity-log-y",
        scatter,
    )];

    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "parity-log-x".into(),
        &f32_column(xs.to_vec()),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "parity-log-y".into(),
        &f32_column(ys.to_vec()),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: None,
                series_id: "log-inverted".into(),
                x_column: "parity-log-x".into(),
                y_column: "parity-log-y".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 2.0,
                    base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                    style_map: None,
                    size_map: None,
                }),
                line_width_px: None,
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();

    for (cursor, expected_index) in [([40.0, 20.0], 4), ([80.0, 80.0], 5)] {
        let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
        assert_pick_parity(cpu.clone(), gpu);
        assert_eq!(cpu.unwrap().point_index, expected_index);
    }

    // Invalid samples must not become pickable at the old 1e-30 clamp,
    // even when that artificial coordinate lies inside the visible range.
    let mut tiny = config.clone();
    tiny.bottom_x.min = 1e-31;
    tiny.bottom_x.max = 1e-29;
    let cursor = [60.0, 50.0];
    assert!(cpu_pick(&tiny, &cpu_series, &cpu_columns, cursor, 2.0).is_none());
    assert!(resolve_pick(&engine, &pool, query_from_config(&tiny, cursor, 2.0)).is_none());

    for (cursor, max_distance) in [
        ([f32::NAN, 20.0], 1.0),
        ([40.0, f32::INFINITY], 1.0),
        ([40.0, 20.0], f32::NAN),
        ([40.0, 20.0], -1.0),
        ([-1.0, 20.0], 1.0),
    ] {
        let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, max_distance);
        let gpu = resolve_pick(
            &engine,
            &pool,
            query_from_config(&config, cursor, max_distance),
        );
        assert_pick_parity(cpu, gpu);
    }
}

/// A stepped line is picked along its axis-aligned pieces on both
/// pickers: hits on a run, on a riser and near the corners resolve to the
/// same source index at the same distance, on linear and logarithmic
/// axes, and the straight chord between two samples is not pickable.
#[cfg(feature = "cpu-twin")]
#[test]
fn exact_gpu_step_lines_match_cpu_picker_on_runs_risers_and_corners() {
    use crate::data_config::LineInterpolation;

    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    // Three samples: a rising step, then a falling one with a long riser.
    let xs = [1.0f32, 3.0, 7.0];
    let ys = [1.0f32, 5.0, 2.0];
    let cpu_columns = CpuColumns::new(&[
        ("parity-step-x", xs.as_slice()),
        ("parity-step-y", ys.as_slice()),
    ]);
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "parity-step-x".into(),
        &f32_column(xs.to_vec()),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "parity-step-y".into(),
        &f32_column(ys.to_vec()),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();

    for logarithmic in [false, true] {
        let mut config = parity_config(400, 400, (1.0, 8.0), (1.0, 8.0));
        if logarithmic {
            config.bottom_x.scale = AxisScale::Logarithmic;
            config.left_y.scale = AxisScale::Logarithmic;
        }
        // Cursor positions in canvas pixels, from data coordinates through
        // the chart's own data area and scale.
        let to_px = |x: f32, y: f32| -> [f32; 2] {
            let area = config.data_area().unwrap().0;
            let t = |v: f32, min: f64, max: f64| {
                if logarithmic {
                    (f64::from(v).log10() - min.log10()) / (max.log10() - min.log10())
                } else {
                    (f64::from(v) - min) / (max - min)
                }
            };
            let tx = t(x, config.bottom_x.min, config.bottom_x.max);
            let ty = t(y, config.left_y.min, config.left_y.max);
            [
                area.x as f32 + tx as f32 * area.width as f32,
                (area.y + area.height) as f32 - ty as f32 * area.height as f32,
            ]
        };
        for mode in [
            LineInterpolation::Linear,
            LineInterpolation::StepPre,
            LineInterpolation::StepPost,
            LineInterpolation::StepMid,
        ] {
            let mut cpu_series = line_series(
                "step",
                Some("step-source"),
                "parity-step-x",
                "parity-step-y",
                3.0,
            );
            let DataRenderType::Line { line } = &mut cpu_series.render_type else {
                unreachable!()
            };
            line.interpolation = mode;
            let cpu_series = [cpu_series];
            let mut engine = GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
            engine
                .add_series(
                    &pool,
                    GpuPickSeriesDescriptor {
                        source_id: Some("step-source".into()),
                        series_id: "step".into(),
                        x_column: "parity-step-x".into(),
                        y_column: "parity-step-y".into(),
                        scatter: None,
                        line_width_px: Some(3.0),
                        line_closed: false,
                        line_interpolation: mode,
                        line_style: crate::line::LineStylePreset::Solid,
                        vector: None,
                        errorbar: None,
                        area: None,
                    },
                )
                .unwrap();
            // Data-space probes: on the first run / riser, on the long
            // falling riser of the second segment, just past the step-mid
            // midpoint, and on the chord between samples 0 and 1 (which
            // only the linear line touches).
            let probes = [
                (2.0f32, 1.0f32),
                (1.0, 3.0),
                (3.0, 3.0),
                (2.0, 5.0),
                (2.0, 3.0),
                (5.0, 4.5),
                (5.0, 2.5),
                (7.0, 3.5),
                (3.0, 4.0),
                (5.2, 5.0),
                (4.8, 2.0),
                (6.9, 4.9),
            ];
            let mut hits = 0;
            for (x, y) in probes {
                for max_distance in [0.75, 6.0, 40.0] {
                    let cursor = to_px(x, y);
                    let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, max_distance);
                    let gpu = resolve_pick(
                        &engine,
                        &pool,
                        query_from_config(&config, cursor, max_distance),
                    );
                    hits += usize::from(cpu.is_some());
                    assert_pick_parity(cpu, gpu);
                }
            }
            assert!(
                hits >= 3,
                "{mode:?} log={logarithmic}: too few hits ({hits}) to be meaningful"
            );
            // Mode-specific resolution on a linear axis, checked on the
            // CPU side (the GPU side agreed above).
            if !logarithmic {
                let pick = |x: f32, y: f32| {
                    cpu_pick(&config, &cpu_series, &cpu_columns, to_px(x, y), 0.75)
                        .map(|p| p.point_index)
                };
                match mode {
                    LineInterpolation::Linear => {
                        assert_eq!(
                            pick(2.0, 3.0),
                            Some(0),
                            "linear: the exact midpoint ties to the lower index"
                        );
                        assert_eq!(pick(2.5, 4.0), Some(1), "linear: on the chord, nearer B");
                        assert_eq!(pick(2.0, 1.0), None, "linear: no run at y_a");
                    }
                    LineInterpolation::StepPost => {
                        assert_eq!(pick(2.0, 1.0), Some(0), "post: on the run at y_a");
                        assert_eq!(pick(3.0, 4.0), Some(1), "post: high on the riser at x_b");
                        assert_eq!(pick(2.0, 3.0), None, "post: the chord is not drawn");
                    }
                    LineInterpolation::StepPre => {
                        assert_eq!(pick(1.0, 3.0), Some(0), "pre: low on the riser at x_a");
                        assert_eq!(pick(2.0, 5.0), Some(1), "pre: on the run at y_b");
                        assert_eq!(pick(2.0, 3.0), None, "pre: the chord is not drawn");
                    }
                    LineInterpolation::StepMid => {
                        assert_eq!(
                            pick(2.0, 3.0),
                            Some(0),
                            "mid: the riser's midpoint ties to the lower index"
                        );
                        assert_eq!(
                            pick(4.8, 5.0),
                            Some(1),
                            "mid: the run at y_a before the midpoint"
                        );
                        assert_eq!(
                            pick(5.2, 2.0),
                            Some(2),
                            "mid: the run at y_b after the midpoint"
                        );
                        assert_eq!(
                            pick(5.2, 5.0),
                            None,
                            "mid: nothing at y_a after the midpoint"
                        );
                        assert_eq!(
                            pick(4.8, 2.0),
                            None,
                            "mid: nothing at y_b before the midpoint"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(feature = "cpu-twin")]
#[test]
fn exact_gpu_equal_distance_ties_match_cpu_series_and_primitive_order() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let config = parity_config(100, 100, (0.0, 10.0), (0.0, 10.0));
    let first_xs = [4.0, 6.0];
    let first_ys = [5.0, 5.0];
    let later_xs = [5.6];
    let later_ys = [5.0];
    let cpu_columns = CpuColumns::new(&[
        ("parity-tie-x0", &first_xs),
        ("parity-tie-y0", &first_ys),
        ("parity-tie-x1", &later_xs),
        ("parity-tie-y1", &later_ys),
    ]);
    let scatter = DataScatterStyleConfig {
        point_color: Color::BLACK,
        point_shape: ScatterShape::CircleFilled,
        point_size: 20.0,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    };
    let line = DataLineStyleConfig {
        line_style: LineStylePreset::Solid,
        line_color: Color::BLACK,
        line_width: 0.0,
        fill: None,
        fill_color: None,
        interpolation: crate::data_config::LineInterpolation::Linear,
    };
    let first_series = SeriesConfig {
        series_id: "first".into(),
        source_id: Some("first-source".into()),
        label: None,
        x_column: "parity-tie-x0".into(),
        y_column: "parity-tie-y0".into(),
        render_type: DataRenderType::ScatterLine { scatter, line },
    };
    let later_series = scatter_series(
        "later",
        Some("later-source"),
        "parity-tie-x1",
        "parity-tie-y1",
        DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::CircleFilled,
            point_size: 1.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        },
    );

    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    for (id, values) in [
        ("parity-tie-x0", first_xs.as_slice()),
        ("parity-tie-y0", first_ys.as_slice()),
        ("parity-tie-x1", later_xs.as_slice()),
        ("parity-tie-y1", later_ys.as_slice()),
    ] {
        pool.add_column(
            id.into(),
            &f32_column(values.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    }
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: Some("first-source".into()),
                series_id: "first".into(),
                x_column: "parity-tie-x0".into(),
                y_column: "parity-tie-y0".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 20.0,
                    base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                    style_map: None,
                    size_map: None,
                }),
                line_width_px: Some(0.0),
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();

    // Both scatter points and the line segment have distance zero at this
    // cursor. The topmost paint wins the tie: markers paint over their
    // series' line and instance 1 paints over instance 0, so both pickers
    // answer marker 1 (the old rule answered the covered marker 0).
    let cursor = [56.0, 50.0];
    let cpu = cpu_pick(
        &config,
        std::slice::from_ref(&first_series),
        &cpu_columns,
        cursor,
        0.0,
    );
    let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
    assert_pick_parity(cpu.clone(), gpu);
    assert_eq!(cpu.unwrap().point_index, 1);

    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: Some("later-source".into()),
                series_id: "later".into(),
                x_column: "parity-tie-x1".into(),
                y_column: "parity-tie-y1".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 1.0,
                    base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                    style_map: None,
                    size_map: None,
                }),
                line_width_px: None,
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();
    let series = [first_series, later_series];
    let cpu = cpu_pick(&config, &series, &cpu_columns, cursor, 0.0);
    let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
    assert_pick_parity(cpu.clone(), gpu);
    assert_eq!(cpu.unwrap().series_id, "later");
}

#[test]
fn exact_gpu_f64_residual_selects_point_index() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let epoch = 1_700_000_000_000.0_f64;
    let xs = [epoch + 0.25, epoch + 0.75];
    let ys = [0.5_f64, 0.5];
    let (x0_hi, x0_lo) = split_f64_to_f32_pair(xs[0]);
    let (x1_hi, x1_lo) = split_f64_to_f32_pair(xs[1]);
    assert_eq!(x0_hi.to_bits(), x1_hi.to_bits());
    assert_ne!(x0_lo.to_bits(), x1_lo.to_bits());

    let config = parity_config(100, 100, (epoch, epoch + 1.0), (0.0, 1.0));
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_hilo_column(
        "parity-f64-x".into(),
        &f64_column(xs.to_vec()),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_hilo_column(
        "parity-f64-y".into(),
        &f64_column(ys.to_vec()),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: Some("f64-source".into()),
                series_id: "f64-residual".into(),
                x_column: "parity-f64-x".into(),
                y_column: "parity-f64-y".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 1.0,
                    base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                    style_map: None,
                    size_map: None,
                }),
                line_width_px: None,
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();

    for (cursor, expected_index) in [([25.0, 50.0], 0), ([75.0, 50.0], 1)] {
        let picked = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0)).unwrap();
        assert_eq!(picked.point_index, expected_index);
    }
}

#[test]
fn registry_transition_failure_at_each_build_position_preserves_state_and_queries() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "pick-batch-x".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "pick-batch-y".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();

    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    for series_id in ["batch-old-0", "batch-old-1", "batch-old-2"] {
        engine
            .add_series(
                &pool,
                scatter_descriptor(series_id, "pick-batch-x", "pick-batch-y"),
            )
            .unwrap();
    }
    let snapshot = PickEngineSnapshot::capture(&engine);
    let baseline = resolve_pick(&engine, &pool, test_query([50.0, 50.0])).unwrap();
    assert_eq!(baseline.series_id, "batch-old-2");

    for failure_at in 0..3 {
        let final_slots = (0..3)
            .map(|gpu_index| {
                GpuPickRegistrySlot::Build(scatter_descriptor(
                    &format!("batch-next-{failure_at}-{gpu_index}"),
                    if gpu_index == failure_at {
                        "pick-batch-missing"
                    } else {
                        "pick-batch-x"
                    },
                    "pick-batch-y",
                ))
            })
            .collect::<Vec<_>>();
        let error = match engine.prepare_registry_transition(&pool, final_slots) {
            Ok(_) => panic!("transition unexpectedly prepared with failure at {failure_at}"),
            Err(error) => error,
        };
        assert!(matches!(error, GpuPickError::MissingColumn(ref id) if id == "pick-batch-missing"));
        snapshot.assert_unchanged(&engine);
        assert_eq!(
            resolve_pick(&engine, &pool, test_query([50.0, 50.0])),
            Some(baseline.clone())
        );
    }
}

#[test]
fn registry_transition_rejects_foreign_pool_before_installing_replacement() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut old_pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    for (id, value) in [
        ("pick-rf-ax", 2.0),
        ("pick-rf-ay", 5.0),
        ("pick-rf-bx", 8.0),
        ("pick-rf-by", 5.0),
    ] {
        old_pool
            .add_column(
                id.into(),
                &f32_column(vec![value]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
    }
    let mut provisional_pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    for (id, value) in [
        ("pick-rf-ax", 3.0),
        ("pick-rf-ay", 5.0),
        ("pick-rf-bx", 8.0),
    ] {
        provisional_pool
            .add_column(
                id.into(),
                &f32_column(vec![value]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
    }

    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(
            &old_pool,
            scatter_descriptor("rebind-old-a", "pick-rf-ax", "pick-rf-ay"),
        )
        .unwrap();
    engine
        .add_series(
            &old_pool,
            scatter_descriptor("rebind-old-b", "pick-rf-bx", "pick-rf-by"),
        )
        .unwrap();
    let snapshot = PickEngineSnapshot::capture(&engine);
    let baseline = resolve_pick(&engine, &old_pool, test_query([80.0, 50.0]));

    let error = match engine.prepare_registry_transition(
        &provisional_pool,
        [GpuPickRegistrySlot::Build(scatter_descriptor(
            "rebind-next-a",
            "pick-rf-ax",
            "pick-rf-ay",
        ))],
    ) {
        Ok(_) => panic!("transition unexpectedly accepted a foreign pool"),
        Err(error) => error,
    };
    assert!(matches!(error, GpuPickError::ForeignColumnPool));
    snapshot.assert_unchanged(&engine);
    assert_eq!(
        resolve_pick(&engine, &old_pool, test_query([80.0, 50.0])),
        baseline
    );
}

#[test]
fn registry_transition_replaces_affected_slots_and_rebinds_survivors() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut old_pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    old_pool
        .add_column(
            "pick-success-layout-prefix".into(),
            &f32_column(vec![0.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    for index in 0..5 {
        old_pool
            .add_column(
                format!("pick-success-x{index}"),
                &f32_column(vec![index as f32 + 1.0]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        old_pool
            .add_column(
                format!("pick-success-y{index}"),
                &f32_column(vec![5.0]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
    }

    let mut engine = GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
    for index in 0..5 {
        engine
            .add_series(
                &old_pool,
                scatter_descriptor(
                    &format!("success-old-{index}"),
                    &format!("pick-success-x{index}"),
                    &format!("pick-success-y{index}"),
                ),
            )
            .unwrap();
    }
    let before_metadata = engine
        .registry
        .slots
        .iter()
        .map(PickSeriesMetadataSnapshot::capture)
        .collect::<Vec<_>>();
    let before_resources = engine
        .registry
        .slots
        .iter()
        .map(PickSeriesResourceSnapshot::capture)
        .collect::<Vec<_>>();
    let before_identities = Arc::clone(&engine.registry.identities);
    let before_reduce = engine.registry.reduce.candidates.clone();
    assert!(
        old_pool
            .remove_column("pick-success-layout-prefix")
            .unwrap()
    );
    assert!(
        old_pool
            .defragment(crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap()
    );
    old_pool
        .upsert_column(
            "pick-success-x0".into(),
            &f32_column(vec![1.5]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    old_pool
        .upsert_column(
            "pick-success-x2".into(),
            &f32_column(vec![3.5]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();

    let final_slots = (0usize..5)
        .map(|gpu_index| {
            if [0usize, 2, 4].contains(&gpu_index) {
                GpuPickRegistrySlot::Build(scatter_descriptor(
                    &format!("success-next-{gpu_index}"),
                    &format!("pick-success-x{gpu_index}"),
                    &format!("pick-success-y{gpu_index}"),
                ))
            } else {
                let identity = &engine.registry.slots[gpu_index].identity;
                GpuPickRegistrySlot::Reuse {
                    current_index: gpu_index,
                    source_id: identity.source_id.clone(),
                    series_id: identity.series_id.clone(),
                }
            }
        })
        .collect::<Vec<_>>();
    engine
        .prepare_registry_transition(&old_pool, final_slots)
        .unwrap()
        .commit();

    assert_eq!(engine.registry.slots.len(), 5);
    assert!(!Arc::ptr_eq(
        &before_identities,
        &engine.registry.identities
    ));
    assert_eq!(engine.registry.reduce.candidates, before_reduce);
    assert_eq!(
        engine
            .registry
            .slots
            .iter()
            .map(|slot| slot.identity.series_id.as_str())
            .collect::<Vec<_>>(),
        [
            "success-next-0",
            "success-old-1",
            "success-next-2",
            "success-old-3",
            "success-next-4",
        ]
    );

    for index in [0usize, 2, 4] {
        assert_eq!(
            engine.registry.slots[index].gpu.pool_layout_generation,
            old_pool.layout_generation()
        );
        assert_eq!(
            engine.registry.slots[index].gpu.x_allocation_epoch,
            old_pool
                .allocation_epoch(&format!("pick-success-x{index}"))
                .unwrap()
        );
        if index != 4 {
            assert_ne!(
                engine.registry.slots[index].gpu.x_allocation_epoch,
                before_metadata[index].x_allocation_epoch
            );
        }
        assert_ne!(
            engine.registry.slots[index].gpu.gate_masks,
            before_resources[index].gate_masks
        );
        assert_ne!(
            engine.registry.slots[index].gpu.workgroup_candidates,
            before_resources[index].workgroup_candidates
        );
        assert_ne!(
            engine.registry.slots[index].gpu.query_work_bg,
            before_resources[index].query_work_bg
        );
        assert_ne!(
            engine.registry.slots[index].gpu.reduce_bg,
            before_resources[index].reduce_bg
        );
        assert_ne!(
            engine.registry.slots[index].gpu.result,
            before_resources[index].result
        );
    }
    for index in [1usize, 3] {
        let slot = &engine.registry.slots[index];
        let series = &slot.gpu;
        assert_eq!(series.gate_masks, before_resources[index].gate_masks);
        assert_eq!(
            series.workgroup_candidates,
            before_resources[index].workgroup_candidates
        );
        assert_eq!(series.query_params, before_resources[index].query_params);
        assert_ne!(series.query_data_bg, before_resources[index].query_data_bg);
        assert_eq!(series.query_work_bg, before_resources[index].query_work_bg);
        assert_eq!(series.reduce_bg, before_resources[index].reduce_bg);
        assert_eq!(series.result, before_resources[index].result);
        assert_eq!(series.pool_layout_generation, old_pool.layout_generation());
        assert_eq!(
            series.x_allocation_epoch,
            before_metadata[index].x_allocation_epoch
        );
        assert_eq!(
            series.y_allocation_epoch,
            before_metadata[index].y_allocation_epoch
        );

        let mut rebound_metadata = PickSeriesMetadataSnapshot::capture(slot);
        rebound_metadata.x_handle = before_metadata[index].x_handle;
        rebound_metadata.y_handle = before_metadata[index].y_handle;
        rebound_metadata.style_index_handle = before_metadata[index].style_index_handle;
        rebound_metadata.pool_layout_generation = before_metadata[index].pool_layout_generation;
        rebound_metadata.style_index_base = before_metadata[index].style_index_base;
        assert_eq!(rebound_metadata, before_metadata[index]);
        assert!(same_storage(
            series.x_handle,
            old_pool
                .handle_for(&series.x_column)
                .expect("rebound x handle")
        ));
        assert!(same_storage(
            series.y_handle,
            old_pool
                .handle_for(&series.y_column)
                .expect("rebound y handle")
        ));
    }

    let picked = resolve_pick(&engine, &old_pool, test_query([50.0, 50.0])).unwrap();
    assert_eq!(picked.series_id, "success-next-4");
    assert_eq!(picked.point_index, 0);
}

#[test]
fn insertion_preserves_position_allows_tail_and_is_failure_atomic() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "pick-ix".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "pick-iy".into(),
        &f32_column(vec![5.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();

    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(&pool, scatter_descriptor("first", "pick-ix", "pick-iy"))
        .unwrap();
    engine
        .add_series(&pool, scatter_descriptor("last", "pick-ix", "pick-iy"))
        .unwrap();
    let inserted = engine
        .insert_series_at(1, &pool, scatter_descriptor("middle", "pick-ix", "pick-iy"))
        .unwrap();
    assert_eq!(inserted.index(), 1);

    let tied = pollster::block_on(
        engine
            .pick(&pool, test_query([50.0, 50.0]))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(tied.series_id, "last");
    engine.remove_series_at(2).unwrap();
    let middle = pollster::block_on(
        engine
            .pick(&pool, test_query([50.0, 50.0]))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(middle.series_id, "middle");

    assert!(matches!(
        engine.insert_series_at(
            0,
            &pool,
            scatter_descriptor("missing", "pick-missing", "pick-iy"),
        ),
        Err(GpuPickError::MissingColumn(_))
    ));
    assert!(matches!(
        engine.insert_series_at(
            3,
            &pool,
            scatter_descriptor("invalid", "pick-ix", "pick-iy"),
        ),
        Err(GpuPickError::InvalidSeriesIndex { index: 3, len: 2 })
    ));
    let unchanged = pollster::block_on(
        engine
            .pick(&pool, test_query([50.0, 50.0]))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(unchanged.series_id, "middle");

    let appended = engine
        .insert_series_at(2, &pool, scatter_descriptor("tail", "pick-ix", "pick-iy"))
        .unwrap();
    assert_eq!(appended.index(), 2);
    let tail = pollster::block_on(
        engine
            .pick(&pool, test_query([50.0, 50.0]))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(tail.series_id, "tail");
}

#[test]
fn registry_mutation_preserves_order_and_clears_stale_reduction_slots() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    for (id, values) in [
        ("pick-ax", vec![2.0]),
        ("pick-ay", vec![5.0]),
        ("pick-bx", vec![8.0]),
        ("pick-by", vec![5.0]),
        ("pick-cx", vec![8.0]),
        ("pick-cy", vec![5.0]),
    ] {
        pool.add_column(
            id.into(),
            &f32_column(values),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    }

    let mut engine = GpuPickEngine::new(device, queue).unwrap();
    engine
        .add_series(&pool, scatter_descriptor("a", "pick-ax", "pick-ay"))
        .unwrap();
    engine
        .add_series(&pool, scatter_descriptor("b", "pick-bx", "pick-by"))
        .unwrap();

    let replacement = engine
        .replace_series_at(0, &pool, scatter_descriptor("c", "pick-cx", "pick-cy"))
        .unwrap();
    assert_eq!(replacement.index(), 0);
    let tied = pollster::block_on(
        engine
            .pick(&pool, test_query([80.0, 50.0]))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(tied.series_id, "b");

    engine.remove_series_at(1).unwrap();
    let no_stale = pollster::block_on(
        engine
            .pick(&pool, test_query([20.0, 50.0]))
            .unwrap()
            .resolve(),
    )
    .unwrap();
    assert!(no_stale.is_none());

    let remaining = pollster::block_on(
        engine
            .pick(&pool, test_query([80.0, 50.0]))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(remaining.series_id, "c");
    assert_eq!(remaining.source_id.as_deref(), Some("source-c"));

    engine.clear_series();
    assert!(
        pollster::block_on(
            engine
                .pick(&pool, test_query([80.0, 50.0]))
                .unwrap()
                .resolve()
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn relocation_rebind_keeps_gate_storage_and_refreshes_all_column_bases() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        64 * 1024,
    )
    .unwrap();
    pool.add_column(
        "pick-hole".into(),
        &f32_column(vec![0.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    for (id, values) in [
        ("pick-rx", vec![4.0]),
        ("pick-ry", vec![6.0]),
        ("pick-rstyle", vec![1.0]),
    ] {
        pool.add_column(
            id.into(),
            &f32_column(values),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    }
    let slots = [
        ScatterStyleSlotGpu {
            color_premul: [0.0; 4],
            meta: [0.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
        },
        ScatterStyleSlotGpu {
            color_premul: [0.0; 4],
            meta: [3.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
        },
    ];
    let mut engine = GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
    engine
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: None,
                series_id: "relocated".into(),
                x_column: "pick-rx".into(),
                y_column: "pick-ry".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 0.0,
                    base_shape_id: 5,
                    style_map: Some(GpuPickScatterStyle {
                        style_index_column: Some("pick-rstyle".into()),
                        style_slots: &slots,
                        style_overrides: &[],
                        style_meta: ScatterStyleMapMeta {
                            style_count: 2,
                            override_count: 0,
                            has_index: 1,
                            _pad: 0,
                        },
                    }),
                    size_map: None,
                }),
                line_width_px: None,
                line_closed: false,
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
                errorbar: None,
                area: None,
            },
        )
        .unwrap();

    let before = pollster::block_on(
        engine
            .pick(&pool, test_query([40.0, 40.0]))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(before.series_id, "relocated");
    let x_epoch = pool.allocation_epoch("pick-rx").unwrap();
    let y_epoch = pool.allocation_epoch("pick-ry").unwrap();
    let style_epoch = pool.allocation_epoch("pick-rstyle").unwrap();

    assert!(pool.remove_column("pick-hole").unwrap());
    assert!(
        pool.defragment(crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap()
    );
    assert!(matches!(
        engine.pick(&pool, test_query([40.0, 40.0])),
        Err(GpuPickError::StaleColumn { .. })
    ));
    engine.rebind_columns(&pool).unwrap();

    let after = pollster::block_on(
        engine
            .pick(&pool, test_query([40.0, 40.0]))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(after.series_id, "relocated");
    assert_eq!(after.point_index, before.point_index);
    assert_eq!(pool.allocation_epoch("pick-rx"), Some(x_epoch));
    assert_eq!(pool.allocation_epoch("pick-ry"), Some(y_epoch));
    assert_eq!(pool.allocation_epoch("pick-rstyle"), Some(style_epoch));

    let gate_masks = engine.registry.slots[0].gpu.gate_masks.clone();
    let workgroup_candidates = engine.registry.slots[0].gpu.workgroup_candidates.clone();
    let layout_generation = pool.layout_generation();
    let old_style_handle = pool.handle_for("pick-rstyle").unwrap();
    assert!(pool.remove_column("pick-rstyle").unwrap());
    let new_style_handle = pool
        .add_column(
            "pick-rstyle".into(),
            &f32_column(vec![1.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(new_style_handle.offset, old_style_handle.offset);
    assert_eq!(new_style_handle.byte_size, old_style_handle.byte_size);
    assert_eq!(new_style_handle.len_values, old_style_handle.len_values);
    assert_ne!(pool.allocation_epoch("pick-rstyle"), Some(style_epoch));
    assert_eq!(pool.layout_generation(), layout_generation);
    assert_eq!(engine.registry.slots[0].gpu.gate_masks, gate_masks);
    assert_eq!(
        engine.registry.slots[0].gpu.workgroup_candidates,
        workgroup_candidates
    );
    assert!(matches!(
        engine.pick(&pool, test_query([40.0, 40.0])),
        Err(GpuPickError::StaleColumn { column_id, .. })
            if column_id == "pick-rstyle"
    ));
}

#[test]
fn gate_dispatch_covers_u32_point_space_with_two_dimensions() {
    let max_groups = 65_535;
    for n in [
        1,
        31,
        32,
        33,
        2_048,
        2_049,
        1_000_000,
        100_000_000,
        u32::MAX,
    ] {
        let (words, groups, x, y) = gate_dispatch_layout(n, max_groups).unwrap();
        assert_eq!(words, n.div_ceil(GPU_PICK_GATE_WORD_POINTS));
        assert_eq!(groups, words.div_ceil(GPU_PICK_WORKGROUP_SIZE));
        assert!(x <= max_groups);
        assert!(y <= max_groups);
        assert!(u64::from(x) * u64::from(y) >= u64::from(groups));
    }
    let (words, groups, _, _) = gate_dispatch_layout(100_000_000, max_groups).unwrap();
    assert_eq!(u64::from(words) * GATE_MASK_BYTES, 25_000_000);
    assert_eq!(u64::from(groups) * CANDIDATE_BYTES, 1_562_528);
    assert_eq!(
        GPU_PICK_DIRECT_SCAN_POINTS,
        GPU_PICK_GATE_WORD_POINTS * GPU_PICK_WORKGROUP_SIZE * 32
    );
}

#[test]
fn originating_gate_word_owns_every_boundary_segment_once() {
    let n = 130u32;
    let mut starts = Vec::new();
    for first in (0..n).step_by(GPU_PICK_GATE_WORD_POINTS as usize) {
        let owned = GPU_PICK_GATE_WORD_POINTS.min(n - first);
        let segment_count = owned.min(n - 1 - first);
        starts.extend(first..first + segment_count);
    }
    assert_eq!(starts, (0..n - 1).collect::<Vec<_>>());
    assert!(starts.contains(&63));
    assert!(starts.contains(&64));
    assert!(starts.contains(&127));
}

#[test]
fn split_linear_projection_preserves_independent_hi_lo_subtraction() {
    // Deliberately anti-correlated hi/lo lanes: recombining before
    // subtracting the large epoch would lose the residual.
    let values = [
        (1_700_000_000_000.0_f64 as f32, -65_535.75_f32),
        (1_700_000_000_000.0_f64 as f32, -65_535.0_f32),
        (1_700_000_131_072.0_f64 as f32, 0.125_f32),
        (1_700_000_131_072.0_f64 as f32, 0.875_f32),
    ];
    let min_hi = values.iter().map(|v| v.0).fold(f32::INFINITY, f32::min);
    let max_hi = values.iter().map(|v| v.0).fold(f32::NEG_INFINITY, f32::max);
    let min_lo = values.iter().map(|v| v.1).fold(f32::INFINITY, f32::min);
    let max_lo = values.iter().map(|v| v.1).fold(f32::NEG_INFINITY, f32::max);
    let axis_min_hi = min_hi;
    let axis_min_lo = min_lo - 1.0;
    let lower = (min_hi - axis_min_hi) + (min_lo - axis_min_lo);
    let upper = (max_hi - axis_min_hi) + (max_lo - axis_min_lo);
    for (hi, lo) in values {
        let numerator = (hi - axis_min_hi) + (lo - axis_min_lo);
        assert!(numerator >= lower && numerator <= upper);
    }
    // A negative (inverted) range reverses but does not invalidate the
    // interval after endpoint sorting.
    let inverted = [lower / -10.0, upper / -10.0];
    let inverted_lo = inverted[0].min(inverted[1]);
    let inverted_hi = inverted[0].max(inverted[1]);
    for (hi, lo) in values {
        let t = ((hi - axis_min_hi) + (lo - axis_min_lo)) / -10.0;
        assert!(t >= inverted_lo && t <= inverted_hi);
    }
}

#[test]
fn style_cross_product_extent_is_conservative() {
    let scatter = GpuPickScatter {
        base_radius_px: 2.0,
        base_shape_id: 5,
        style_map: Some(GpuPickScatterStyle {
            style_index_column: None,
            style_slots: &[ScatterStyleSlotGpu {
                color_premul: [0.0; 4],
                meta: [10.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
            }],
            style_overrides: &[ScatterStyleOverrideGpu {
                point_index: 7,
                _pad: [0; 3],
                color_premul: [0.0; 4],
                meta: [0.0, 2.0, STYLE_MASK_SHAPE as f32, 0.0],
            }],
            style_meta: ScatterStyleMapMeta {
                style_count: 1,
                override_count: 1,
                has_index: 0,
                _pad: 0,
            },
        }),
        size_map: None,
    };
    let extent = style_max_extent("s", &scatter).unwrap();
    assert_eq!(extent, 10.0 * MAX_SHAPE_SCALE);
}

#[test]
fn unrelated_remove_keeps_gate_storage_while_reused_source_epoch_stales_it() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut pool = ColumnPool::new(
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        4 * 256,
    )
    .unwrap();
    let values = f32_column(vec![5.0]);

    let original_x = pool
        .add_column(
            "epoch-pick-x".into(),
            &values,
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    let original_x_epoch = pool.allocation_epoch("epoch-pick-x").unwrap();
    pool.add_column(
        "epoch-pick-y".into(),
        &values,
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "epoch-pick-unrelated".into(),
        &f32_column(vec![0.0]),
        crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();

    let mut engine = GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
    engine
        .add_series(
            &pool,
            scatter_descriptor("epoch-pick-series", "epoch-pick-x", "epoch-pick-y"),
        )
        .unwrap();
    let gate_masks = engine.registry.slots[0].gpu.gate_masks.clone();
    let workgroup_candidates = engine.registry.slots[0].gpu.workgroup_candidates.clone();
    let layout_generation = pool.layout_generation();
    let baseline = resolve_pick(&engine, &pool, test_query([50.0, 50.0]));
    assert!(baseline.is_some());

    assert!(pool.remove_column("epoch-pick-unrelated").unwrap());
    assert_eq!(pool.layout_generation(), layout_generation);
    assert_eq!(engine.registry.slots[0].gpu.gate_masks, gate_masks);
    assert_eq!(
        engine.registry.slots[0].gpu.workgroup_candidates,
        workgroup_candidates
    );
    assert!(GpuPickEngine::validate_series_columns(&pool, &engine.registry.slots[0]).is_ok());
    assert_eq!(
        resolve_pick(&engine, &pool, test_query([50.0, 50.0])),
        baseline
    );

    assert!(pool.remove_column("epoch-pick-x").unwrap());
    let replacement_x = pool
        .add_column(
            "epoch-pick-x".into(),
            &values,
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(replacement_x.offset, original_x.offset);
    assert_eq!(replacement_x.len_values, original_x.len_values);
    assert_ne!(
        pool.allocation_epoch("epoch-pick-x"),
        Some(original_x_epoch)
    );
    assert_eq!(pool.layout_generation(), layout_generation);
    assert_eq!(engine.registry.slots[0].gpu.gate_masks, gate_masks);
    assert_eq!(
        engine.registry.slots[0].gpu.workgroup_candidates,
        workgroup_candidates
    );
    assert!(matches!(
        GpuPickEngine::validate_series_columns(&pool, &engine.registry.slots[0]),
        Err(GpuPickError::StaleColumn { column_id, .. })
            if column_id == "epoch-pick-x"
    ));
}

/// Every assembled pick module starts with the core and ends with its entry
/// fragment, and every fragment file under `src/gpu_pick/` is in some
/// module. (`shader_assembly`'s `every_module_is_its_fragments_concatenated`
/// holds each source to its fragment list.)
#[test]
fn assembled_pick_shaders_start_with_the_core_and_use_every_fragment() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/gpu_pick");
    let mut used = std::collections::HashSet::new();
    for shader in std::iter::once(PickShader::Gate).chain(PickShader::EXACT) {
        used.extend(shader.fragments().iter().copied());
        assert_eq!(shader.fragments()[0], "gpu_pick/core.wgsl", "{shader:?}");
        let last = *shader.fragments().last().unwrap();
        if shader == PickShader::Gate {
            assert_eq!(last, "gpu_pick/gate_entries.wgsl");
        } else {
            assert_eq!(last, "gpu_pick/exact_entry.wgsl", "{shader:?}");
        }
    }
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        // The pick modules' Rust files share this directory with the fragments.
        if !name.ends_with(".wgsl") {
            continue;
        }
        assert!(
            used.contains(format!("gpu_pick/{name}").as_str()),
            "{name} is in no pick module"
        );
    }
}

/// Each exact pick module splices the same shared `common/` fragments as the
/// draw modules whose primitives it measures, so the pick projects, validates
/// and shapes a point with the draw's code: Marker the scatter's and the
/// error bars', Line the line's (its fill included), MarkerLine all three,
/// Vector the point vector's. Only `transform.wgsl`
/// (its uniform takes the binding the pick's pool holds) and `style.wgsl` (the
/// pick reads its own style rows) are the draw's alone.
#[test]
fn pick_modules_splice_their_draw_modules_common_fragments() {
    use std::collections::BTreeSet;
    const DRAW_ONLY: [&str; 2] = [
        "data_render/common/transform.wgsl",
        "data_render/common/style.wgsl",
    ];
    let common = |fragments: &[&'static str]| -> BTreeSet<&'static str> {
        fragments
            .iter()
            .copied()
            .filter(|f| f.starts_with("data_render/common/") && !DRAW_ONLY.contains(f))
            .collect()
    };
    let draw = |name: &str| {
        let (_, assembled) = crate::shader_assembly::MODULES
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("no module {name}"));
        common(assembled.fragments)
    };
    for (shader, draws) in [
        (PickShader::Marker, &["scatter", "errorbar"][..]),
        (PickShader::Line, &["line"][..]),
        (PickShader::MarkerLine, &["scatter", "line", "errorbar"][..]),
        (PickShader::Vector, &["vector_points"][..]),
    ] {
        let expected: BTreeSet<_> = draws.iter().flat_map(|name| draw(name)).collect();
        assert!(!expected.is_empty(), "{shader:?}");
        assert_eq!(
            common(shader.fragments()),
            expected,
            "{shader:?} vs {draws:?}"
        );
    }
}

/// `validate_device_limits` checks the exact layout: seven storage buffers
/// in three groups. Every binding `pick_exact_candidates` reaches in any
/// exact module (at the default variant, which carries every path) is an
/// entry of that layout of the same kind, so the check covers the pass.
#[test]
fn device_limit_check_covers_the_exact_pass_bindings() {
    use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};
    assert_eq!(exact_layout_storage_buffers(), 7);
    assert_eq!(EXACT_LAYOUT.len(), 3);
    for shader in PickShader::EXACT {
        let source = shader.source();
        let module = wgpu::naga::front::wgsl::parse_str(source).unwrap();
        let info = Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .unwrap();
        let (index, _) = module
            .entry_points
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.name == "pick_exact_candidates")
            .unwrap_or_else(|| panic!("{shader:?}: no exact entry"));
        let uses = info.get_entry_point(index);
        for (handle, global) in module.global_variables.iter() {
            let Some(binding) = global.binding.as_ref() else {
                continue;
            };
            if uses[handle].is_empty() {
                continue;
            }
            let entry = EXACT_LAYOUT
                .get(binding.group as usize)
                .and_then(|group| group.iter().find(|entry| entry.binding == binding.binding))
                .unwrap_or_else(|| panic!("{shader:?}: {binding:?} is not in the exact layout"));
            let storage = matches!(global.space, wgpu::naga::AddressSpace::Storage { .. });
            assert_eq!(
                matches!(
                    entry.ty,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { .. },
                        ..
                    }
                ),
                storage,
                "{shader:?}: {binding:?}"
            );
        }
    }
}

/// Compile cost of the point picker per module, entry point and exact
/// variant: the wall time of `create_shader_module` and of
/// `create_compute_pipeline(layout: None)` per entry, three rounds, each
/// on a fresh device. `FIGGY_PICK_TIMING_SOURCE` times one other
/// single-module `gpu_pick.wgsl` (an older revision) instead. Run with
/// `--ignored --nocapture`.
#[test]
#[ignore = "timing report"]
fn pick_shader_compile_timings() {
    use super::exact_variant as v;
    use std::time::Instant;
    let override_source = std::env::var("FIGGY_PICK_TIMING_SOURCE")
        .ok()
        .map(|path| std::fs::read_to_string(path).expect("timing source"));
    // (module label, source, [(entry, variant label, PICK_VARIANT)]).
    type Entries = Vec<(&'static str, &'static str, Option<u32>)>;
    let mut modules: Vec<(&str, String, Entries)> = Vec::new();
    let gate_entries: Entries = vec![
        ("pick_gate_x", "-", None),
        ("pick_gate_y", "-", None),
        ("pick_reduce_candidates", "-", None),
    ];
    if let Some(source) = override_source {
        let mut entries = gate_entries;
        entries.push(("pick_exact_candidates", "default", None));
        modules.push(("single module", source, entries));
    } else {
        let exact = |variants: &[(&'static str, u32)]| -> Entries {
            variants
                .iter()
                .map(|(label, bits)| ("pick_exact_candidates", *label, Some(*bits)))
                .collect()
        };
        modules.push(("gate", PickShader::Gate.source().to_string(), gate_entries));
        modules.push((
            "marker",
            PickShader::Marker.source().to_string(),
            exact(&[
                ("precise", 0),
                ("precise circle", 0x1000 | 5),
                ("precise star", 0x1000 | 25),
                ("polar", v::POLAR),
                ("styled", v::STYLED_MARKER),
                ("all", v::ALL),
            ]),
        ));
        modules.push((
            "marker+line",
            PickShader::MarkerLine.source().to_string(),
            exact(&[
                ("precise", 0),
                ("precise circle", 0x1000 | 5),
                ("dashed", v::DASH),
                ("styled", v::STYLED_MARKER | v::STYLED_LINE),
                ("all (streamed)", v::ALL),
            ]),
        ));
        modules.push((
            "line",
            PickShader::Line.source().to_string(),
            exact(&[
                ("precise", 0),
                ("dashed", v::DASH),
                ("styled", v::STYLED_LINE),
                ("all", v::ALL),
            ]),
        ));
        modules.push((
            "vector",
            PickShader::Vector.source().to_string(),
            exact(&[("precise", 0)]),
        ));
    }
    for round in 0..3 {
        let (device, _queue) = crate::test_gpu::device();
        for (name, source, entries) in &modules {
            let started = Instant::now();
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(source.clone().into()),
            });
            println!(
                "PICK_TIMING round {round} [{name}] create_shader_module {:.1} ms",
                started.elapsed().as_secs_f64() * 1e3
            );
            for (entry, label, variant) in entries {
                // 0x1000 | id: the precise variant with `PICK_SHAPE = id`.
                let constants = variant.map(|bits| {
                    if bits & 0x1000 != 0 {
                        vec![(v::CONSTANT, 0.0), ("PICK_SHAPE", f64::from(bits & 0xfff))]
                    } else {
                        vec![(v::CONSTANT, f64::from(bits))]
                    }
                });
                let started = Instant::now();
                let _pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: None,
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: constants.as_deref().unwrap_or(&[]),
                        zero_initialize_workgroup_memory: true,
                    },
                    cache: None,
                });
                println!(
                    "PICK_TIMING round {round} [{name}] {entry} [{label}] {:.1} ms",
                    started.elapsed().as_secs_f64() * 1e3
                );
            }
        }
    }
}

use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use renderer::data::COLUMN_VALUE_BYTES;
use renderer::data_render::column_pool::ALIGN;
use renderer::data_render::{create_instance, request_adapter, request_device};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, Color, Column, ColumnHandle, ColumnPairWriter, ColumnSource, ColumnUploadStats,
    DataLineStyleConfig, DataRenderType, FiggyError, Renderer, RendererDevice, SeriesConfig,
};
use wgpu::{BufferDescriptor, BufferUsages};

const DEMO_IDS: [&str; 4] = ["demo_x", "demo_sin", "demo_t", "demo_rc"];

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

fn demo_columns() -> [Column<f64>; 4] {
    [
        col_f64(vec![0.0, 1.0, 2.0]),
        col_f64(vec![20.0, 50.0, 80.0]),
        col_f64(vec![0.0, 2.5, 5.0]),
        col_f64(vec![0.0, 4.0, 5.0]),
    ]
}

fn line_series(id: &str, x: &str, y: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.to_string(),
        source_id: None,
        label: None,
        x_column: x.to_string(),
        y_column: y.to_string(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: color,
                line_width: 2.0,
            },
        },
    }
}

fn final_series() -> Vec<SeriesConfig> {
    vec![
        line_series("sine", "demo_x", "demo_sin", Color::new(0.1, 0.2, 0.8, 1.0)),
        line_series("rc", "demo_t", "demo_rc", Color::new(0.8, 0.2, 0.1, 1.0)),
    ]
}

fn final_state(
    pool: &renderer::ColumnPool,
    mut config: renderer::Config,
) -> renderer::Result<(renderer::Config, Vec<SeriesConfig>)> {
    let mut chart = Chart::new(config);
    chart.auto_fit_x(pool, "demo_x", 0.02)?;
    chart.auto_fit_y(pool, "demo_sin", 0.10)?;
    config = chart.config().clone();
    config.chart_title.text.segments = renderer::text::rich_segments_from_text("figgy");
    config.bottom_x.title_option.text.segments = renderer::text::rich_segments_from_text("x");
    config.left_y.title_option.text.segments = renderer::text::rich_segments_from_text("y");
    Ok((config, final_series()))
}

fn shared_device() -> Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
    static DEVICE: OnceLock<Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let instance = create_instance();
            let adapter = request_adapter(&instance).ok()?;
            let (device, queue) = request_device(&adapter).ok()?;
            Some((Arc::new(device), Arc::new(queue)))
        })
        .as_ref()
        .map(|(device, queue)| (Arc::clone(device), Arc::clone(queue)))
}

fn try_renderer(capacity: u64) -> Option<Renderer> {
    let (device, queue) = shared_device()?;
    Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        capacity,
    )
    .ok()
}

fn handle_tuple(handle: ColumnHandle) -> (u32, u64, u64, usize) {
    (
        handle.generation,
        handle.offset,
        handle.byte_size,
        handle.len_values,
    )
}

fn buffer_identity(buffer: &wgpu::Buffer) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    buffer.hash(&mut hasher);
    hasher.finish()
}

fn read_column(
    renderer: &Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    id: &str,
) -> Vec<f64> {
    let handle = renderer.handle_for(id).unwrap();
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("load-demo atomicity readback"),
        size: handle.byte_size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("load-demo atomicity readback"),
    });
    encoder.copy_buffer_to_buffer(
        renderer.pool().buffer(),
        handle.offset,
        &readback,
        0,
        handle.byte_size,
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    encoder.map_buffer_on_submit(
        &readback,
        wgpu::MapMode::Read,
        0..handle.byte_size,
        move |result| {
            let _ = sender.send(result);
        },
    );
    let submission = queue.submit(std::iter::once(encoder.finish()));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let mapped = readback
        .slice(..handle.byte_size)
        .get_mapped_range()
        .unwrap();
    let values = mapped[..handle.len_values * COLUMN_VALUE_BYTES]
        .chunks_exact(COLUMN_VALUE_BYTES)
        .map(|bytes| {
            let hi = f32::from_le_bytes(bytes[..4].try_into().unwrap()) as f64;
            let lo = f32::from_le_bytes(bytes[4..8].try_into().unwrap()) as f64;
            hi + lo
        })
        .collect();
    drop(mapped);
    readback.unmap();
    values
}

#[test]
fn dropping_load_demo_preserves_public_pool_chart_and_gpu_bytes() {
    let Some(mut renderer) = try_renderer(12 * ALIGN) else {
        eprintln!("no GPU adapter; skipping load-demo guard-drop assertions");
        return;
    };
    let (device, queue) = shared_device().unwrap();
    let old = col_f64(vec![1.0, 2.0, 3.0]);
    renderer.add_column("survivor", &old).unwrap();
    for id in DEMO_IDS {
        renderer.add_column(id, &old).unwrap();
    }
    let initial_config = renderer::default::default_config();
    let initial_series = vec![line_series("old", "demo_x", "demo_sin", Color::BLACK)];
    let chart = renderer
        .register_chart(initial_config.clone(), initial_series.clone())
        .unwrap();
    let generation = renderer.pool().generation();
    let used = renderer.pool().used_bytes();
    let free = renderer.pool().free_bytes();
    let buffer = buffer_identity(renderer.pool().buffer());
    let handles = ["survivor", "demo_x", "demo_sin", "demo_t", "demo_rc"]
        .map(|id| handle_tuple(renderer.handle_for(id).unwrap()));
    let bytes = ["survivor", "demo_x", "demo_sin", "demo_t", "demo_rc"]
        .map(|id| read_column(&renderer, &device, &queue, id));
    let visual = renderer.visual_revision();
    let stamp = renderer.chart_render_stamp(chart).unwrap();
    let columns = demo_columns();

    {
        let pending = renderer
            .begin_load_demo(
                chart,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
                |pool| final_state(pool, initial_config.clone()),
            )
            .unwrap();
        drop(pending);
    }

    assert_eq!(renderer.pool().generation(), generation);
    assert_eq!(renderer.pool().used_bytes(), used);
    assert_eq!(renderer.pool().free_bytes(), free);
    assert_eq!(buffer_identity(renderer.pool().buffer()), buffer);
    assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
    assert_eq!(renderer.chart_series(chart).unwrap(), initial_series);
    assert_eq!(renderer.visual_revision(), visual);
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), stamp);
    for (index, id) in ["survivor", "demo_x", "demo_sin", "demo_t", "demo_rc"]
        .iter()
        .enumerate()
    {
        assert_eq!(
            handle_tuple(renderer.handle_for(id).unwrap()),
            handles[index]
        );
        assert_eq!(read_column(&renderer, &device, &queue, id), bytes[index]);
    }
}

#[test]
fn all_new_demo_columns_publish_in_one_complete_state() {
    let Some(mut renderer) = try_renderer(8 * ALIGN) else {
        eprintln!("no GPU adapter; skipping all-new load-demo assertions");
        return;
    };
    let chart = renderer
        .register_chart(renderer::default::default_config(), Vec::new())
        .unwrap();
    let generation = renderer.pool().generation();
    let columns = demo_columns();

    renderer
        .begin_load_demo(
            chart,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
            |pool| final_state(pool, renderer::default::default_config()),
        )
        .unwrap()
        .commit();

    assert_eq!(renderer.pool().generation(), generation + 1);
    assert_eq!(renderer.chart_series(chart).unwrap(), final_series());
    assert!(DEMO_IDS.iter().all(|id| renderer.pool().slot(id).is_some()));
    assert!(!renderer.has_pending_maintenance());
}

#[test]
fn commit_from_fragmented_existing_backup_is_complete_and_repeatable() {
    let Some(mut renderer) = try_renderer(12 * ALIGN) else {
        eprintln!("no GPU adapter; skipping load-demo commit assertions");
        return;
    };
    let (device, queue) = shared_device().unwrap();
    let old = col_f64(vec![1.0, 2.0]);
    renderer.add_column("survivor", &old).unwrap();
    renderer.add_column("hole", &old).unwrap();
    renderer.add_column("demo_x", &old).unwrap();
    renderer.add_column("demo_sin", &old).unwrap();
    assert!(renderer.remove_column("hole").unwrap());
    assert!(renderer.defragment().unwrap());
    let chart = renderer
        .register_chart(renderer::default::default_config(), Vec::new())
        .unwrap();
    let generation = renderer.pool().generation();
    let survivor = read_column(&renderer, &device, &queue, "survivor");
    let columns = demo_columns();

    renderer
        .begin_load_demo(
            chart,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
            |pool| final_state(pool, renderer::default::default_config()),
        )
        .unwrap()
        .commit();
    assert_eq!(renderer.pool().generation(), generation + 1);
    assert_eq!(renderer.chart_series(chart).unwrap(), final_series());
    let config = renderer.chart_config(chart).unwrap().clone();
    assert_eq!((config.bottom_x.min, config.bottom_x.max), (-0.04, 2.04));
    assert_eq!((config.left_y.min, config.left_y.max), (14.0, 86.0));
    assert_eq!(
        read_column(&renderer, &device, &queue, "survivor"),
        survivor
    );
    for (index, id) in DEMO_IDS.iter().enumerate() {
        assert_eq!(
            read_column(&renderer, &device, &queue, id),
            columns[index].data
        );
    }
    assert!(!renderer.has_pending_maintenance());

    let first_stamp = renderer.chart_render_stamp(chart).unwrap();
    let first_config = renderer.chart_config(chart).unwrap().clone();
    let first_series = renderer.chart_series(chart).unwrap().to_vec();
    renderer
        .begin_load_demo(
            chart,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
            |pool| final_state(pool, first_config.clone()),
        )
        .unwrap()
        .commit();
    assert_eq!(renderer.chart_config(chart).unwrap(), &first_config);
    assert_eq!(renderer.chart_series(chart).unwrap(), first_series);
    assert_ne!(renderer.chart_render_stamp(chart).unwrap(), first_stamp);
}

struct PanickingSource;

impl ColumnSource for PanickingSource {
    fn len(&self) -> usize {
        2
    }

    fn min(&self) -> f64 {
        1.0
    }

    fn max(&self) -> f64 {
        2.0
    }

    fn write_f32_le_into(&self, dst: &mut [u8]) {
        dst.fill(0);
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut writer: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        writer.write_pair(0, 1.0, 0.0);
        panic!("injected demo staging failure");
    }
}

#[test]
fn staging_failure_at_each_demo_column_leaves_external_state_unchanged() {
    let good = col_f64(vec![1.0, 2.0]);
    let panicking = PanickingSource;
    for failed_index in 0..4 {
        let Some(mut renderer) = try_renderer(8 * ALIGN) else {
            eprintln!("no GPU adapter; skipping load-demo staging assertions");
            return;
        };
        let chart = renderer
            .register_chart(renderer::default::default_config(), Vec::new())
            .unwrap();
        let generation = renderer.pool().generation();
        let visual = renderer.visual_revision();
        let stamp = renderer.chart_render_stamp(chart).unwrap();
        let sources: [&dyn ColumnSource; 4] = std::array::from_fn(|index| {
            let source: &dyn ColumnSource = if index == failed_index {
                &panicking
            } else {
                &good
            };
            source
        });

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = renderer.begin_load_demo(
                chart,
                sources[0],
                sources[1],
                sources[2],
                sources[3],
                |_| Ok((renderer::default::default_config(), Vec::new())),
            );
        }));
        assert!(result.is_err());
        assert_eq!(renderer.pool().generation(), generation);
        assert_eq!(renderer.pool().used_bytes(), 0);
        assert_eq!(renderer.visual_revision(), visual);
        assert_eq!(renderer.chart_render_stamp(chart).unwrap(), stamp);
        assert!(DEMO_IDS.iter().all(|id| renderer.pool().slot(id).is_none()));
    }
}

#[test]
fn invalid_final_chart_state_aborts_after_candidate_preparation() {
    let Some(mut renderer) = try_renderer(8 * ALIGN) else {
        eprintln!("no GPU adapter; skipping load-demo validation assertions");
        return;
    };
    let chart = renderer
        .register_chart(renderer::default::default_config(), Vec::new())
        .unwrap();
    let generation = renderer.pool().generation();
    let visual = renderer.visual_revision();
    let stamp = renderer.chart_render_stamp(chart).unwrap();
    let columns = demo_columns();

    assert!(matches!(
        renderer.begin_load_demo(
            chart,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
            |_| {
                let mut invalid = renderer::default::default_config();
                invalid.chart_area.0.width = 0;
                Ok((invalid, final_series()))
            },
        ),
        Err(FiggyError::InvalidChartArea { width: 0, .. })
    ));
    assert_eq!(renderer.pool().generation(), generation);
    assert_eq!(renderer.pool().used_bytes(), 0);
    assert_eq!(renderer.visual_revision(), visual);
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), stamp);
}

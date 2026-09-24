#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, OnceLock};

use renderer::default::default_config;
use renderer::{
    Column, DataRenderType, DataScatterStyleConfig, FiggyError, LogicalColumn, Renderer,
    RendererDevice, SeriesConfig, StreamBounds, StreamColumn, StreamEncoding, StreamReplay,
    StreamStatistics,
};

fn renderer() -> Renderer {
    static GPU: OnceLock<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> = OnceLock::new();
    let (device, queue) = GPU.get_or_init(|| {
        let instance = renderer::data_render::create_instance();
        let adapter =
            renderer::data_render::request_adapter(&instance).expect("GPU adapter required");
        let (device, queue) =
            renderer::data_render::request_device(&adapter).expect("GPU device required");
        (Arc::new(device), Arc::new(queue))
    });
    Renderer::try_new(
        RendererDevice::new(Arc::clone(device), Arc::clone(queue)),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .expect("renderer creation")
}

#[test]
fn statistics_authority_starts_in_renderer_and_empty_sources_are_complete() {
    let mut renderer = renderer();
    let mut injected = source("injected", 4);
    injected.statistics = StreamStatistics::Known(Some(StreamBounds {
        min: 1.0,
        max: 4.0,
        min_positive: Some(1.0),
    }));
    assert!(matches!(
        renderer.register_streamed_columns(vec![injected]),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    renderer
        .register_streamed_columns(vec![source("empty", 0)])
        .unwrap();
    assert_eq!(
        renderer.logical_column("empty").unwrap().statistics(),
        StreamStatistics::Known(None)
    );
}

fn source(id: &str, len: u64) -> StreamColumn {
    StreamColumn {
        id: id.into(),
        len,
        encoding: StreamEncoding::ScalarF32,
        replay: StreamReplay::RandomAccess,
        revision: 1,
        statistics: StreamStatistics::Unknown,
    }
}

fn series(id: &str, x: &str, y: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: renderer::Color::new(0.1, 0.2, 0.8, 1.0),
                point_shape: renderer::ScatterShape::CircleFilled,
                point_size: 3.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
            },
        },
    }
}

fn registered(renderer: &Renderer, id: &str) -> StreamColumn {
    match renderer.logical_column(id).expect("registered source") {
        LogicalColumn::Streamed(source) => source.clone(),
        LogicalColumn::Resident(_) => panic!("unexpected resident allocation"),
    }
}

#[test]
fn arbitrary_columns_register_without_allocating_payload_or_uploading() {
    let mut renderer = renderer();
    let before = renderer.gpu_memory_usage();
    let mut columns: Vec<_> = (0..17)
        .map(|i| source(&format!("column-{i}"), u32::MAX as u64))
        .collect();
    columns[3].encoding = StreamEncoding::HiLoF32;
    columns[3].replay = StreamReplay::Sequential;
    renderer.register_streamed_columns(columns.clone()).unwrap();
    assert_eq!(renderer.gpu_memory_usage(), before);
    for column in &columns {
        assert_eq!(registered(&renderer, &column.id), *column);
        assert!(renderer.pool().slot(&column.id).is_none());
    }
    let chart = renderer
        .register_chart(default_config(), vec![series("s", "column-0", "column-3")])
        .unwrap();
    assert_eq!(
        renderer.series_draw_info(chart, "s").unwrap().drawn_count,
        u32::MAX as u64
    );
}

#[test]
fn batches_reject_invalid_late_members_without_partial_publication() {
    let mut renderer = renderer();
    renderer
        .register_streamed_columns(vec![source("x", 8), source("y", 8)])
        .unwrap();
    let chart = renderer
        .register_chart(default_config(), vec![series("s", "x", "y")])
        .unwrap();
    let stamp = renderer.chart_render_stamp(chart).unwrap();
    let memory = renderer.gpu_memory_usage();
    let mut invalid = source("overflow", u32::MAX as u64 + 1);
    for bad in [invalid.clone(), {
        invalid.len = 8;
        invalid.replay = StreamReplay::Unavailable;
        invalid
    }] {
        assert!(
            renderer
                .register_streamed_columns(vec![source("new", 8), bad])
                .is_err()
        );
        assert!(renderer.logical_column("new").is_none());
        assert!(renderer.logical_column("overflow").is_none());
    }
    assert!(
        renderer
            .register_streamed_columns(vec![source("duplicate", 8), source("duplicate", 9)])
            .is_err()
    );
    assert!(renderer.logical_column("duplicate").is_none());
    let mut newer_x = registered(&renderer, "x");
    newer_x.revision = 2;
    newer_x.len = 99;
    assert!(
        renderer
            .replace_streamed_columns(vec![newer_x, source("y", 100)])
            .is_err()
    );
    assert_eq!(registered(&renderer, "x"), source("x", 8));
    assert_eq!(registered(&renderer, "y"), source("y", 8));
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), stamp);
    assert_eq!(renderer.gpu_memory_usage(), memory);
}

#[test]
fn replacement_and_removal_touch_only_dependent_chart() {
    let mut renderer = renderer();
    renderer
        .register_streamed_columns(vec![source("x", 8), source("a", 8), source("b", 8)])
        .unwrap();
    let a = renderer
        .register_chart(default_config(), vec![series("s", "x", "a")])
        .unwrap();
    let b = renderer
        .register_chart(default_config(), vec![series("s", "x", "b")])
        .unwrap();
    let stamp_a = renderer.chart_render_stamp(a).unwrap();
    let stamp_b = renderer.chart_render_stamp(b).unwrap();
    let mut newer = registered(&renderer, "a");
    newer.revision = 2;
    newer.len = 4;
    renderer.replace_streamed_columns(vec![newer]).unwrap();
    assert_ne!(renderer.chart_render_stamp(a).unwrap(), stamp_a);
    assert_eq!(renderer.chart_render_stamp(b).unwrap(), stamp_b);
    assert_eq!(renderer.series_draw_info(a, "s").unwrap().drawn_count, 4);
    assert!(renderer.remove_column("a").unwrap());
    assert!(renderer.logical_column("a").is_none());
    assert!(renderer.chart_series(a).unwrap().is_empty());
    assert_eq!(renderer.chart_series(b).unwrap().len(), 1);
    assert_eq!(renderer.chart_render_stamp(b).unwrap(), stamp_b);
    assert!(!renderer.remove_column("a").unwrap());
}

#[test]
fn resident_and_streamed_ids_cannot_shadow_each_other() {
    let mut renderer = renderer();
    let values = Column {
        data: vec![1.0f32, 2.0],
        min: 1.0,
        max: 2.0,
    };
    renderer.add_column("resident", &values).unwrap();
    renderer
        .register_streamed_columns(vec![source("stream", 200)])
        .unwrap();
    let before = renderer.gpu_memory_usage();
    assert!(
        renderer
            .register_streamed_columns(vec![source("new", 4), source("resident", 4)])
            .is_err()
    );
    assert!(renderer.logical_column("new").is_none());
    assert!(matches!(
        renderer.add_column("stream", &values),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    assert!(matches!(
        renderer.logical_column("resident"),
        Some(LogicalColumn::Resident(_))
    ));
    assert_eq!(registered(&renderer, "stream"), source("stream", 200));
    assert_eq!(renderer.gpu_memory_usage(), before);
}

#[test]
fn nonresident_queries_do_not_report_unknown_or_empty_during_resident_transaction() {
    let mut renderer = renderer();
    renderer
        .register_streamed_columns(vec![source("x", 8), source("y", 8)])
        .unwrap();
    let declaration = series("s", "x", "y");
    let chart = renderer
        .register_chart(default_config(), vec![declaration.clone()])
        .unwrap();
    let ids = renderer::GpuSeriesExtentColumnIds {
        x: "x",
        y: "y",
        x_lower: None,
        x_upper: None,
        y_lower: None,
        y_upper: None,
    };
    assert!(matches!(
        renderer.handle_for("x"),
        Err(FiggyError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        renderer.web_derived_snapshot(chart),
        Err(FiggyError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        renderer.begin_errorbar_extent("x", "y", "y"),
        Err(renderer::GpuErrorbarError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        renderer.begin_series_extent(renderer::GpuSeriesExtentMode::Points, ids),
        Err(renderer::GpuErrorbarError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        renderer.begin_series_fit_extent(&declaration),
        Err(renderer::GpuErrorbarError::ColumnNotResident { .. })
    ));
    let values = Column {
        data: vec![1.0f32, 2.0],
        min: 1.0,
        max: 2.0,
    };
    let transaction = renderer.begin_upsert_column("resident", &values).unwrap();
    assert!(matches!(
        transaction.web_derived_snapshot(chart),
        Err(FiggyError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        transaction.begin_errorbar_extent("x", "y", "y"),
        Err(renderer::GpuErrorbarError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        transaction.begin_series_extent(renderer::GpuSeriesExtentMode::Points, ids),
        Err(renderer::GpuErrorbarError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        transaction.begin_series_fit_extent(&declaration),
        Err(renderer::GpuErrorbarError::ColumnNotResident { .. })
    ));
    drop(transaction);
    assert!(renderer.logical_column("resident").is_none());
    assert_eq!(renderer.chart_series(chart).unwrap(), &[declaration]);
}

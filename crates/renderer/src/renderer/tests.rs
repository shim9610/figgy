use super::*;

/// Wait until every retired GPU charge of `renderer` has completed. A
/// completion callback can run on another test's polling thread on the
/// shared device, so one `poll(Wait)` does not guarantee it has run yet.
fn settle_gpu_retirement(renderer: &mut Renderer) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while renderer.gpu_memory_usage().retired_bytes() != 0 {
        renderer.service_gpu_completions().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "retirement callback timeout"
        );
        std::thread::yield_now();
    }
}
use crate::data::Column;
use crate::data_config::{DataErrorBarPointStyleOverride, DataScatterPointStyleOverride};

#[test]
fn every_line_preset_packs_without_gpu_dash_truncation() {
    for preset in [
        LineStylePreset::Solid,
        LineStylePreset::Dash,
        LineStylePreset::Dot,
        LineStylePreset::DashDot,
        LineStylePreset::DashDotDot,
        LineStylePreset::ShortDash,
        LineStylePreset::ShortDot,
        LineStylePreset::ShortDashDot,
        LineStylePreset::LongDash,
        LineStylePreset::LongDashDot,
        LineStylePreset::LongDashDotDot,
    ] {
        let mut style = PrimitiveStyle::from_color(Color::BLACK);
        style.pack_dash_pattern(preset.pattern(), 1.5);
        let packed: Vec<f32> = style
            .dash
            .iter()
            .flatten()
            .copied()
            .take(style.dash_len as usize)
            .collect();
        let expected: Vec<f32> = preset.pattern().iter().map(|value| value * 1.5).collect();
        assert_eq!(
            style.dash_len as usize,
            preset.pattern().len(),
            "{preset:?}"
        );
        assert_eq!(packed, expected, "{preset:?}");
    }
}

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

fn demo_test_columns() -> [Column<f64>; 4] {
    [
        col_f64(vec![0.0, 1.0, 2.0]),
        col_f64(vec![20.0, 50.0, 80.0]),
        col_f64(vec![0.0, 2.5, 5.0]),
        col_f64(vec![0.0, 4.0, 5.0]),
    ]
}

fn revision_snapshot(renderer: &Renderer, id: ChartId) -> [RenderRevision; 7] {
    let revisions = renderer.chart_states[&id].revisions;
    [
        revisions.desired,
        revisions.config,
        revisions.series,
        revisions.data,
        revisions.view,
        revisions.selection,
        revisions.raster,
    ]
}

fn test_scatter_style() -> DataScatterStyleConfig {
    DataScatterStyleConfig {
        point_color: Color::new(0.0, 0.0, 0.0, 1.0),
        point_shape: crate::data_config::ScatterShape::CircleFilled,
        point_size: 5.0,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    }
}

fn test_errorbar_style() -> DataErrorBarStyleConfig {
    DataErrorBarStyleConfig {
        error_bar_color: Color::new(1.0, 0.0, 0.0, 1.0),
        error_bar_width: 1.0,
        error_bar_cap_size: 4.0,
        cap_width: 1.0,
        error_bar_style_table: None,
        error_bar_style_index_column: None,
        error_bar_style_overrides: None,
    }
}

fn state_test_renderer_with_capacity(pool_capacity: u64) -> Option<Renderer> {
    let (device, queue) = crate::data_render::shared_device()?;
    Some(
        Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Rgba8Unorm,
            pool_capacity,
        )
        .expect("required GPU test: renderer initialization failed"),
    )
}

fn state_test_renderer() -> Option<Renderer> {
    state_test_renderer_with_capacity(1024 * 1024)
}

fn state_test_stream(id: &str, revision: u64) -> crate::StreamColumn {
    crate::StreamColumn {
        id: id.into(),
        len: 100,
        encoding: crate::StreamEncoding::HiLoF32,
        replay: crate::StreamReplay::Sequential,
        revision,
        statistics: crate::StreamStatistics::Unknown,
    }
}

struct FailingPromotionSource {
    len: usize,
}

impl ColumnSource for FailingPromotionSource {
    fn len(&self) -> usize {
        self.len
    }

    fn min(&self) -> f64 {
        0.0
    }

    fn max(&self) -> f64 {
        1.0
    }

    fn write_f32_le_into(&self, _dst: &mut [u8]) {
        unreachable!("promotion uses the range capability")
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        _dst: crate::data::ColumnPairWriter<'_>,
    ) -> crate::data::ColumnUploadStats {
        unreachable!("promotion uses the range capability")
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        _start: u64,
        _dst: crate::data::ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, crate::data::ColumnRangeWriteError> {
        Err(crate::data::ColumnRangeWriteError::SourceFailed)
    }
}

struct MutableLenPromotionSource {
    calls: std::cell::Cell<usize>,
}

impl ColumnSource for MutableLenPromotionSource {
    fn len(&self) -> usize {
        let call = self.calls.get();
        self.calls.set(call + 1);
        if call == 0 { 1 } else { usize::MAX }
    }

    fn min(&self) -> f64 {
        1.0
    }

    fn max(&self) -> f64 {
        1.0
    }

    fn write_f32_le_into(&self, _dst: &mut [u8]) {}

    fn write_f32_pair_le_into_with_stats(
        &self,
        _dst: crate::data::ColumnPairWriter<'_>,
    ) -> crate::data::ColumnUploadStats {
        crate::data::ColumnUploadStats {
            min_positive: Some(1.0),
        }
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        _start: u64,
        mut dst: crate::data::ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, crate::data::ColumnRangeWriteError> {
        assert_eq!(dst.len(), 1, "promotion must stage the frozen length");
        dst.write_pair(0, 1.0, 0.0);
        Ok(Some(crate::StreamBounds {
            min: 1.0,
            max: 1.0,
            min_positive: Some(1.0),
        }))
    }
}

struct PanickingPromotionSource;

impl HiLoColumnSource for PanickingPromotionSource {
    fn len(&self) -> usize {
        1
    }

    fn min(&self) -> f64 {
        1.0
    }

    fn max(&self) -> f64 {
        1.0
    }

    fn write_f32_pair_le_into(&self, _dst: &mut [u8]) {}

    fn write_f32_pair_le_into_with_stats(
        &self,
        _dst: crate::data::ColumnPairWriter<'_>,
    ) -> crate::data::ColumnUploadStats {
        crate::data::ColumnUploadStats {
            min_positive: Some(1.0),
        }
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        _start: u64,
        _dst: crate::data::ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, crate::data::ColumnRangeWriteError> {
        panic!("injected promotion writer panic")
    }
}

struct InvalidBoundsPromotionSource;

impl ColumnSource for InvalidBoundsPromotionSource {
    fn len(&self) -> usize {
        1
    }

    fn min(&self) -> f64 {
        1.0
    }

    fn max(&self) -> f64 {
        1.0
    }

    fn write_f32_le_into(&self, _dst: &mut [u8]) {}

    fn write_f32_pair_le_into_with_stats(
        &self,
        _dst: crate::data::ColumnPairWriter<'_>,
    ) -> crate::data::ColumnUploadStats {
        crate::data::ColumnUploadStats {
            min_positive: Some(1.0),
        }
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        _start: u64,
        mut dst: crate::data::ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, crate::data::ColumnRangeWriteError> {
        dst.write_pair(0, 1.0, 0.0);
        Ok(Some(crate::StreamBounds {
            min: 2.0,
            max: 1.0,
            min_positive: None,
        }))
    }
}

#[test]
fn streamed_promotion_freezes_len_and_contains_source_failures() {
    fn setup(id: &str, encoding: crate::StreamEncoding) -> Option<(Renderer, ChartId)> {
        let mut renderer = state_test_renderer_with_capacity(4 * 1024)?;
        let _ = renderer.set_memory_budget(Some(u64::MAX));
        renderer
            .register_streamed_columns(vec![crate::StreamColumn {
                id: id.into(),
                len: 1,
                encoding,
                replay: crate::StreamReplay::RandomAccess,
                revision: 1,
                statistics: crate::StreamStatistics::Unknown,
            }])
            .unwrap();
        let chart = renderer
            .register_chart(
                crate::default::default_config(),
                vec![state_test_line("temporary", id, id)],
            )
            .unwrap();
        Some((renderer, chart))
    }

    let Some((mut renderer, chart)) = setup("mutable-len", crate::StreamEncoding::ScalarF32) else {
        return;
    };
    let source = MutableLenPromotionSource {
        calls: std::cell::Cell::new(0),
    };
    let before = revision_snapshot(&renderer, chart);
    let pending_before = renderer.pending_defrag;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        renderer.promote_streamed_columns_to_resident(
            &[crate::StreamSourceBinding {
                id: "mutable-len",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&source),
            }],
            0,
            &[],
            0,
            u64::MAX,
        )
    }));
    let report = result
        .expect("mutable len must not panic")
        .expect("the first validated length must drive the complete promotion");
    assert!(report.is_admissible());
    assert_eq!(
        source.calls.get(),
        1,
        "source length must be read exactly once"
    );
    let slot = renderer.pool.slot("mutable-len").unwrap();
    assert_eq!(slot.len_values, 1);
    assert_eq!(
        (slot.min, slot.max, slot.min_positive),
        (1.0, 1.0, Some(1.0))
    );
    assert!(!renderer.streaming_sources.contains_key("mutable-len"));
    assert_eq!(renderer.stream_revision_lineage["mutable-len"], 1);
    assert_ne!(revision_snapshot(&renderer, chart), before);
    assert_eq!(renderer.pending_defrag, pending_before);

    let Some((mut renderer, chart)) = setup("panicking-writer", crate::StreamEncoding::HiLoF32)
    else {
        return;
    };
    let before = revision_snapshot(&renderer, chart);
    let used_before = renderer.pool.used_bytes();
    let layout_before = renderer.pool.layout_generation();
    let pending_before = renderer.pending_defrag;
    let picker_before = renderer.picker.active.is_some();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        renderer.promote_streamed_columns_to_resident(
            &[crate::StreamSourceBinding {
                id: "panicking-writer",
                revision: 1,
                source: crate::StreamColumnSource::HiLo(&PanickingPromotionSource),
            }],
            0,
            &[],
            0,
            u64::MAX,
        )
    }));
    assert!(matches!(
        result.expect("writer panic must be contained"),
        Err(FiggyError::Pool(
            data_render::AllocError::AllocationFailed {
                resource: "streamed promotion source",
                ..
            }
        ))
    ));
    assert!(renderer.pool.slot("panicking-writer").is_none());
    assert!(renderer.streaming_sources.contains_key("panicking-writer"));
    assert_eq!(renderer.stream_revision_lineage["panicking-writer"], 1);
    assert_eq!(renderer.pool.used_bytes(), used_before);
    assert_eq!(renderer.pool.layout_generation(), layout_before);
    assert_eq!(revision_snapshot(&renderer, chart), before);
    assert_eq!(renderer.pending_defrag, pending_before);
    assert_eq!(renderer.picker.active.is_some(), picker_before);

    let Some((mut renderer, chart)) = setup("invalid-bounds", crate::StreamEncoding::ScalarF32)
    else {
        return;
    };
    let before = revision_snapshot(&renderer, chart);
    let used_before = renderer.pool.used_bytes();
    let layout_before = renderer.pool.layout_generation();
    let pending_before = renderer.pending_defrag;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        renderer.promote_streamed_columns_to_resident(
            &[crate::StreamSourceBinding {
                id: "invalid-bounds",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&InvalidBoundsPromotionSource),
            }],
            0,
            &[],
            0,
            u64::MAX,
        )
    }));
    assert!(matches!(
        result.expect("invalid statistics must not panic"),
        Err(FiggyError::Pool(
            data_render::AllocError::AllocationFailed {
                resource: "streamed promotion source",
                ..
            }
        ))
    ));
    assert!(renderer.pool.slot("invalid-bounds").is_none());
    assert!(renderer.streaming_sources.contains_key("invalid-bounds"));
    assert_eq!(renderer.stream_revision_lineage["invalid-bounds"], 1);
    assert_eq!(renderer.pool.used_bytes(), used_before);
    assert_eq!(renderer.pool.layout_generation(), layout_before);
    assert_eq!(revision_snapshot(&renderer, chart), before);
    assert_eq!(renderer.pending_defrag, pending_before);
}

#[test]
fn stream_statistics_cache_coalesces_coverage_and_encoded_bounds() {
    let mut cache = StreamStatisticsCache::default();
    cache.insert_measured(
        4..8,
        Some(crate::StreamBounds {
            min: -99.0,
            max: 99.0,
            min_positive: Some(99.0),
        }),
    );
    assert_eq!(cache.covered, vec![4..8]);
    assert_eq!(cache.uncovered(0..12).unwrap(), vec![0..4, 8..12]);
    assert_eq!(cache.bounds.unwrap().max, 99.0);
    cache.insert_measured(
        0..4,
        Some(crate::StreamBounds {
            min: -2.0,
            max: 7.0,
            min_positive: Some(0.5),
        }),
    );
    assert!(cache.covers(5..7));
    assert!(!cache.covers(0..12));
    assert_eq!(cache.covered, vec![0..8]);
    cache.insert_measured(
        8..12,
        Some(crate::StreamBounds {
            min: 1.0,
            max: 9.0,
            min_positive: Some(1.0),
        }),
    );
    assert_eq!(cache.covered, vec![0..12]);
    assert_eq!(
        cache.bounds,
        Some(crate::StreamBounds {
            min: -99.0,
            max: 99.0,
            min_positive: Some(0.5),
        })
    );

    let fragmented = StreamStatisticsCache {
        covered: (0..MAX_STREAM_STATISTICS_RANGES as u64)
            .map(|index| index * 2..index * 2 + 1)
            .collect(),
        bounds: None,
    };
    assert_eq!(
        fragmented.coverage_len_after_insert(
            &(MAX_STREAM_STATISTICS_RANGES as u64 * 2..MAX_STREAM_STATISTICS_RANGES as u64 * 2 + 1)
        ),
        MAX_STREAM_STATISTICS_RANGES + 1
    );
    assert_eq!(
        fragmented.coverage_len_after_insert(&(1..MAX_STREAM_STATISTICS_RANGES as u64 * 2)),
        1,
        "a bridge is allowed to collapse even a full fragmented cache"
    );
}

/// Streamed sources are never resident: every resident-only query answers
/// `ColumnNotResident` instead of an unknown or empty result, on the renderer
/// and inside a resident upsert transaction, and the transaction leaves the
/// chart's series and desired revision alone. Statistics commits then follow
/// the source revision and counter contract.
#[test]
fn nonresident_queries_and_streamed_statistics_preserve_revision_contract() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .register_streamed_columns(vec![state_test_stream("s", 1)])
        .unwrap();
    let series = state_test_line("streamed", "s", "s");
    let chart = renderer
        .register_chart(crate::default::default_config(), vec![series.clone()])
        .unwrap();
    let columns = crate::GpuSeriesExtentColumnIds {
        x: "s",
        y: "s",
        x_lower: None,
        x_upper: None,
        y_lower: None,
        y_upper: None,
    };
    assert!(matches!(
        renderer.handle_for("s"),
        Err(FiggyError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        renderer.web_derived_snapshot(chart),
        Err(FiggyError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        renderer.begin_errorbar_extent("s", "s", "s"),
        Err(crate::GpuErrorbarError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        renderer.begin_series_extent(crate::GpuSeriesExtentMode::Points, columns),
        Err(crate::GpuErrorbarError::ColumnNotResident { .. })
    ));
    assert!(matches!(
        renderer.begin_series_fit_extent(&series),
        Err(crate::GpuErrorbarError::ColumnNotResident { .. })
    ));
    let old = renderer.chart_states[&chart].revisions.desired;
    {
        let guard = renderer
            .begin_upsert_column("resident", &col_f64(vec![1.0, 2.0]))
            .unwrap();
        assert!(matches!(
            guard.web_derived_snapshot(chart),
            Err(FiggyError::ColumnNotResident { .. })
        ));
        assert!(matches!(
            guard.begin_series_fit_extent(&series),
            Err(crate::GpuErrorbarError::ColumnNotResident { .. })
        ));
        assert!(matches!(
            guard.begin_errorbar_extent("s", "s", "s"),
            Err(crate::GpuErrorbarError::ColumnNotResident { .. })
        ));
        for mode in [
            crate::GpuSeriesExtentMode::Line,
            crate::GpuSeriesExtentMode::Points,
        ] {
            assert!(matches!(
                guard.begin_series_extent(mode, columns),
                Err(crate::GpuErrorbarError::ColumnNotResident { .. })
            ));
        }
    }
    assert!(renderer.logical_column("resident").is_none());
    assert_eq!(renderer.chart_series(chart).unwrap(), &[series.clone()]);
    assert_eq!(renderer.chart_states[&chart].revisions.desired, old);
    let bounds = Some(crate::StreamBounds {
        min: -1.0,
        max: 2.0,
        min_positive: Some(0.5),
    });
    assert!(matches!(
        renderer.commit_streamed_statistics("s", 2, bounds),
        Err(FiggyError::StaleStateToken { .. })
    ));
    assert_eq!(
        renderer.streaming_sources["s"].statistics,
        crate::StreamStatistics::Unknown
    );
    let fit_epoch = renderer.next_stream_source_fit_epoch;
    renderer.next_stream_source_fit_epoch = u64::MAX;
    assert!(matches!(
        renderer.commit_streamed_statistics("s", 1, bounds),
        Err(FiggyError::CounterExhausted { .. })
    ));
    assert_eq!(
        renderer.streaming_sources["s"].statistics,
        crate::StreamStatistics::Unknown
    );
    renderer.next_stream_source_fit_epoch = fit_epoch;
    renderer.commit_streamed_statistics("s", 1, bounds).unwrap();
    assert_eq!(
        renderer.streaming_sources["s"].statistics,
        crate::StreamStatistics::Known(bounds)
    );
    let committed = renderer.chart_render_stamp(chart).unwrap();
    renderer.commit_streamed_statistics("s", 1, bounds).unwrap();
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), committed);
    assert!(renderer.commit_streamed_statistics("s", 1, None).is_err());
}

#[test]
fn streamed_fit_token_uses_live_source_revision_and_registration_epoch() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .register_streamed_columns(vec![state_test_stream("s", 1)])
        .unwrap();
    let series = state_test_line("streamed", "s", "s");
    let chart = renderer
        .register_chart(crate::default::default_config(), vec![series.clone()])
        .unwrap();
    assert_eq!(
        renderer.logical_column("s").unwrap().statistics(),
        crate::StreamStatistics::Unknown
    );
    let before_statistics = renderer.begin_fit_commit(chart).unwrap();
    let bounds = crate::StreamBounds {
        min: 1.0,
        max: 3.0,
        min_positive: Some(1.0),
    };
    renderer
        .commit_streamed_statistics("s", 1, Some(bounds))
        .unwrap();
    assert_eq!(
        renderer.logical_column("s").unwrap().statistics(),
        crate::StreamStatistics::Known(Some(bounds))
    );
    let extent = crate::FitExtent {
        min: bounds.min,
        max: bounds.max,
        min_positive: bounds.min_positive,
    };
    assert!(matches!(
        renderer.commit_auto_fit_all_if_current(&before_statistics, &extent, &extent, 0.0),
        Err(FiggyError::StaleStateToken { .. })
    ));
    let known = renderer.begin_fit_commit(chart).unwrap();
    renderer
        .commit_auto_fit_all_if_current(&known, &extent, &extent, 0.0)
        .unwrap();
    assert_eq!(renderer.chart_config(chart).unwrap().bottom_x.min, 1.0);
    assert_eq!(renderer.chart_config(chart).unwrap().bottom_x.max, 3.0);
    let current = renderer.begin_fit_commit(chart).unwrap();
    renderer
        .replace_streamed_columns(vec![state_test_stream("s", 2)])
        .unwrap();
    assert!(matches!(
        renderer.commit_auto_fit_all_if_current(&current, &extent, &extent, 0.0),
        Err(FiggyError::StaleStateToken { .. })
    ));

    let before_reuse = renderer.begin_fit_commit(chart).unwrap();
    let config = renderer.chart_config(chart).unwrap().clone();
    assert!(renderer.remove_column("s").unwrap());
    renderer
        .register_streamed_columns(vec![state_test_stream("s", 2)])
        .unwrap();
    renderer
        .set_chart_state(chart, config, vec![series])
        .unwrap();
    assert!(matches!(
        renderer.commit_auto_fit_all_if_current(&before_reuse, &extent, &extent, 0.0),
        Err(FiggyError::StaleStateToken { .. })
    ));
}

/// Replacing or removing a streamed source touches only the chart that
/// depends on it, and a revision counter failure leaves source, series and
/// charts as they were.
#[test]
fn streamed_source_replacement_and_removal_touch_only_the_dependent_chart() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .register_streamed_columns(vec![state_test_stream("s", 1), state_test_stream("t", 1)])
        .unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("s", "s", "s")],
        )
        .unwrap();
    let other = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("t", "t", "t")],
        )
        .unwrap();
    let other_stamp = renderer.chart_render_stamp(other).unwrap();
    let old = renderer.chart_states[&chart].revisions.desired;
    renderer
        .chart_states
        .get_mut(&chart)
        .unwrap()
        .revisions
        .desired
        .sequence = u64::MAX;
    assert!(matches!(
        renderer.replace_streamed_columns(vec![state_test_stream("s", 2)]),
        Err(FiggyError::CounterExhausted { .. })
    ));
    assert_eq!(renderer.streaming_sources["s"].revision, 1);
    assert!(renderer.remove_column("s").is_err());
    assert!(renderer.logical_column("s").is_some());
    assert_eq!(renderer.chart_states[&chart].series.len(), 1);
    renderer
        .chart_states
        .get_mut(&chart)
        .unwrap()
        .revisions
        .desired = old;
    let stamp = renderer.chart_render_stamp(chart).unwrap();
    assert_eq!(
        renderer.series_draw_info(chart, "s").unwrap().drawn_count,
        99
    );
    renderer
        .replace_streamed_columns(vec![crate::StreamColumn {
            len: 4,
            ..state_test_stream("s", 2)
        }])
        .unwrap();
    assert_ne!(renderer.chart_states[&chart].revisions.desired, old);
    assert_ne!(renderer.chart_render_stamp(chart).unwrap(), stamp);
    // The line now draws the replaced source's 4 samples: 3 segments.
    assert_eq!(
        renderer.series_draw_info(chart, "s").unwrap().drawn_count,
        3
    );
    assert_eq!(renderer.chart_render_stamp(other).unwrap(), other_stamp);
    let fit_token = renderer.begin_fit_commit(chart).unwrap();
    assert!(renderer.remove_column("s").unwrap());
    assert!(renderer.chart_states[&chart].series.is_empty());
    assert!(renderer.logical_column("s").is_none());
    assert_eq!(renderer.chart_series(other).unwrap().len(), 1);
    assert!(!renderer.remove_column("s").unwrap());
    assert!(matches!(
        renderer.commit_auto_fit_all_if_current(
            &fit_token,
            &crate::FitExtent::EMPTY,
            &crate::FitExtent::EMPTY,
            0.0,
        ),
        Err(FiggyError::StaleStateToken { .. })
    ));
    assert_eq!(renderer.chart_render_stamp(other).unwrap(), other_stamp);
}

#[test]
fn streamed_series_set_detaches_only_affected_picker_after_success() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![1.0, 2.0]);
    renderer
        .register_streamed_columns(vec![state_test_stream("s", 1)])
        .unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("resident", "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let unrelated = renderer
        .register_chart(crate::default::default_config(), vec![])
        .unwrap();
    renderer
        .set_chart_series(unrelated, vec![state_test_line("streamed", "s", "s")])
        .unwrap();
    assert_eq!(active_picker(&renderer).chart_id, chart);
    let old = renderer.chart_states[&chart].revisions.desired;
    renderer
        .chart_states
        .get_mut(&chart)
        .unwrap()
        .revisions
        .desired
        .sequence = u64::MAX;
    assert!(
        renderer
            .set_chart_series(chart, vec![state_test_line("streamed", "s", "s")])
            .is_err()
    );
    assert_eq!(active_picker(&renderer).chart_id, chart);
    renderer
        .chart_states
        .get_mut(&chart)
        .unwrap()
        .revisions
        .desired = old;
    renderer
        .set_chart_series(chart, vec![state_test_line("streamed", "s", "s")])
        .unwrap();
    assert!(renderer.picker.active.is_none());
    assert!(matches!(
        renderer.prepare_gpu_picking_for_chart(chart),
        Err(crate::GpuPickError::ColumnNotResident(_))
    ));
    renderer
        .set_chart_series(chart, vec![state_test_line("resident", "x", "y")])
        .unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    assert_eq!(active_picker(&renderer).chart_id, chart);
}

fn state_test_line(id: &str, x: &str, y: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: id.to_string(),
        source_id: None,
        label: None,
        x_column: x.to_string(),
        y_column: y.to_string(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::new(0.1, 0.2, 0.3, 1.0),
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }
}

fn state_test_scatter_line(id: &str, source: Option<&str>, x: &str, y: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: id.to_string(),
        source_id: source.map(str::to_string),
        label: None,
        x_column: x.to_string(),
        y_column: y.to_string(),
        z_column: None,
        render_type: DataRenderType::ScatterLine {
            scatter: test_scatter_style(),
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::new(0.1, 0.2, 0.3, 1.0),
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }
}

fn state_test_scatter(id: &str, source: Option<&str>, x: &str, y: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: id.to_string(),
        source_id: source.map(str::to_string),
        label: None,
        x_column: x.to_string(),
        y_column: y.to_string(),
        z_column: None,
        render_type: DataRenderType::Scatter {
            scatter: test_scatter_style(),
        },
    }
}

fn add_state_test_xy(renderer: &mut Renderer, x: &str, y: &str, values: Vec<f64>) {
    renderer.add_column(x, &col_f64(values.clone())).unwrap();
    renderer.add_column(y, &col_f64(values)).unwrap();
}

fn active_picker(renderer: &Renderer) -> &ActivePickRegistry {
    renderer
        .picker
        .active
        .as_ref()
        .expect("test prepared an active picker registry")
}

fn basic_errorbar_chart() -> Chart {
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    config.legend.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 4.0);
    chart.set_y_range(0.0, 5.0);
    chart
}

fn caps_with_limits(storage_binding: u64, buffer_size: u64) -> RendererDeviceCaps {
    RendererDeviceCaps {
        features: wgpu::Features::empty(),
        max_texture_dimension_2d: 8192,
        max_buffer_size: buffer_size,
        max_storage_buffer_binding_size: storage_binding,
        max_compute_workgroups_per_dimension: 65_535,
        max_compute_invocations_per_workgroup: 256,
    }
}

/// The pool must be clamped to whatever the arc-scan / star-pass storage
/// binding can address, so an oversized request degrades gracefully
/// instead of panicking on the first dashed line. No GPU needed — this
/// pins the clamp arithmetic `try_new` relies on.
#[test]
fn pool_capacity_clamps_to_storage_and_buffer_limits() {
    let mb = 1024 * 1024;
    let caps = caps_with_limits(128 * mb, 1024 * mb);
    // Comfortably under both limits — untouched.
    assert_eq!(effective_pool_capacity(64 * mb, caps), 64 * mb);
    // Exactly at the storage-binding limit — untouched.
    assert_eq!(effective_pool_capacity(128 * mb, caps), 128 * mb);
    // Over the storage-binding limit (wgpu default) — clamped to it,
    // even though max_buffer_size is far higher. This is the 256 MB pool
    // that used to panic on the first dashed/sketch/milkyway line.
    assert_eq!(effective_pool_capacity(256 * mb, caps), 128 * mb);
    // When max_buffer_size is the tighter of the two, it wins.
    let caps_small_buffer = caps_with_limits(u64::from(u32::MAX), 32 * mb);
    assert_eq!(
        effective_pool_capacity(256 * mb, caps_small_buffer),
        32 * mb
    );
    // A device with a generous binding limit keeps a large pool intact.
    let caps_big = caps_with_limits(u64::from(u32::MAX), 4 * 1024 * mb);
    assert_eq!(effective_pool_capacity(512 * mb, caps_big), 512 * mb);
}

#[test]
/// Enabling picking creates the gate and reduction pipelines and the
/// typed data pipelines once; the exact point pipelines wait for the first
/// pick that needs each (`PickExactPipelines`).
fn renderer_gpu_picker_activation_creates_six_pipelines_once() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let mut events = Vec::new();
    renderer
        .enable_gpu_picking_observed(&mut |event| events.push(event))
        .unwrap();
    renderer
        .enable_gpu_picking_observed(&mut |event| events.push(event))
        .unwrap();

    let pipeline_starts = events
        .iter()
        .filter(|event| {
            event.scope == "renderer.gpu_pick"
                && event.phase == crate::init::InitPhase::Started
                && matches!(
                    event.stage,
                    "pick_gate_x"
                        | "pick_gate_y"
                        | "pick_exact_candidates"
                        | "pick_reduce_candidates"
                )
        })
        .count();
    let setup_starts = events
        .iter()
        .filter(|event| {
            event.scope == "renderer.gpu_pick"
                && event.phase == crate::init::InitPhase::Started
                && event.stage == "setup"
        })
        .count();
    let data_pipeline_starts = events
        .iter()
        .filter(|event| {
            event.scope == "renderer.gpu_data_pick"
                && event.phase == crate::init::InitPhase::Started
                && matches!(
                    event.stage,
                    "pick_histogram_bin" | "pick_field_data" | "pick_vector_cell"
                )
        })
        .count();
    let data_setup_starts = events
        .iter()
        .filter(|event| {
            event.scope == "renderer.gpu_data_pick"
                && event.phase == crate::init::InitPhase::Started
                && event.stage == "setup"
        })
        .count();
    assert_eq!(pipeline_starts, 3);
    assert_eq!(setup_starts, 1);
    assert_eq!(data_pipeline_starts, 3);
    assert_eq!(data_setup_starts, 1);
    assert!(renderer.picker.active.is_none());
}

fn typed_pick_config(x: (f64, f64), y: (f64, f64)) -> Config {
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 480,
        height: 360,
    });
    config.legend.visible = false;
    for axis in [&mut config.bottom_x, &mut config.top_x] {
        axis.min = x.0;
        axis.max = x.1;
    }
    for axis in [&mut config.left_y, &mut config.right_y] {
        axis.min = y.0;
        axis.max = y.1;
    }
    config
}

fn typed_pick_position(config: &Config, x: f64, y: f64) -> [f32; 2] {
    let area = config.data_area().unwrap().0;
    let tx = ((x - config.bottom_x.min) / (config.bottom_x.max - config.bottom_x.min)) as f32;
    let ty = ((y - config.left_y.min) / (config.left_y.max - config.left_y.min)) as f32;
    [
        area.x as f32 + tx * area.width as f32,
        area.y as f32 + (1.0 - ty) * area.height as f32,
    ]
}

fn typed_pick_request(config: &Config, position: [f32; 2]) -> GpuPickRequest {
    GpuPickRequest {
        canvas_position_px: position,
        display_panel_px: config.chart_area.0,
        display_scale: 1.0,
        max_distance_px: 0.0,
    }
}

#[test]
fn typed_gpu_pick_returns_histogram_bin_without_cpu_bar_bounds() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("edges", &col_f64(vec![0.0, 1.0, 2.0]))
        .unwrap();
    renderer
        .add_column("counts", &col_f64(vec![5.0, 8.0]))
        .unwrap();
    let config = typed_pick_config((0.0, 2.0), (0.0, 10.0));
    let position = typed_pick_position(&config, 1.5, 4.0);
    let series = SeriesConfig {
        series_id: "hist".into(),
        source_id: Some("source".into()),
        label: None,
        x_column: "edges".into(),
        y_column: "counts".into(),
        z_column: None,
        render_type: DataRenderType::Histogram {
            bar: crate::data_config::DataBarStyleConfig {
                fill_color: Color::BLACK,
                border_color: Color::BLACK,
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 2.0,
                width_ratio: 1.0,
                orientation: crate::data_config::BarOrientation::Vertical,
                bar_style_overrides: None,
            },
        },
    };
    let chart = renderer
        .register_chart(config.clone(), vec![series])
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    let picked = pollster::block_on(
        renderer
            .pick_chart_data(chart, typed_pick_request(&config, position))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .expect("histogram bin hit");
    assert_eq!(
        picked,
        crate::pick::PickedData::HistogramBin {
            source_id: Some("source".into()),
            series_id: "hist".into(),
            bin_index: 1,
            distance_px: 0.0,
            half: None,
        }
    );
}

#[test]
fn typed_gpu_pick_uses_the_overridden_histogram_width() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("edges", &col_f64(vec![0.0, 1.0, 2.0]))
        .unwrap();
    renderer
        .add_column("counts", &col_f64(vec![5.0, 8.0]))
        .unwrap();
    let config = typed_pick_config((0.0, 2.0), (0.0, 10.0));
    let series = SeriesConfig {
        series_id: "hist".into(),
        source_id: None,
        label: None,
        x_column: "edges".into(),
        y_column: "counts".into(),
        z_column: None,
        render_type: DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: Color::BLACK,
                border_color: Color::BLACK,
                border_width: 0.0,
                baseline: 0.0,
                gap_px: 0.0,
                width_ratio: 1.0,
                orientation: crate::data_config::BarOrientation::Vertical,
                bar_style_overrides: Some(vec![crate::data_config::DataBarStyleOverride {
                    index: 1,
                    style: DataBarBinStyleConfig {
                        width_ratio: Some(0.2),
                        ..Default::default()
                    },
                }]),
            },
        },
    };
    let chart = renderer
        .register_chart(config.clone(), vec![series])
        .unwrap();
    renderer.enable_gpu_picking().unwrap();

    // Raw bin 1 spans x=1..2, but its overridden bar spans x=1.4..1.6.
    let outside = pollster::block_on(
        renderer
            .pick_chart_data(
                chart,
                typed_pick_request(&config, typed_pick_position(&config, 1.1, 4.0)),
            )
            .unwrap()
            .resolve(),
    )
    .unwrap();
    assert_eq!(
        outside, None,
        "raw bin area outside the narrow bar was picked"
    );

    let inside = pollster::block_on(
        renderer
            .pick_chart_data(
                chart,
                typed_pick_request(&config, typed_pick_position(&config, 1.5, 4.0)),
            )
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .expect("narrow overridden bar hit");
    assert!(matches!(
        inside,
        crate::pick::PickedData::HistogramBin { bin_index: 1, .. }
    ));
}

#[test]
fn typed_gpu_pick_returns_canonical_heatmap_cell_indices() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    for (id, values) in [
        ("x", vec![0.0, 1.0, 2.0]),
        ("y", vec![0.0, 1.0, 2.0]),
        ("z0", vec![1.0, 2.0]),
        ("z1", vec![3.0, 4.0]),
    ] {
        renderer.add_column(id, &col_f64(values)).unwrap();
    }
    let mut config = typed_pick_config((0.0, 2.0), (0.0, 2.0));
    config.colorbar = Some(crate::default::default_colorbar_options());
    let position = typed_pick_position(&config, 1.5, 0.5);
    let series = SeriesConfig {
        series_id: "heat".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Heatmap {
            matrix: crate::data_config::MatrixRef {
                columns: vec!["z0".into(), "z1".into()],
                orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                grid_layout: crate::data_config::GridLayout::Edges,
            },
            fill: crate::data_config::FieldFillConfig {
                mode: crate::data_config::FillMode::Continuous,
                shading: crate::data_config::Shading::Flat,
                opacity: 1.0,
            },
            surface: None,
        },
    };
    let chart = renderer
        .register_chart(config.clone(), vec![series])
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    let picked = pollster::block_on(
        renderer
            .pick_chart_data(chart, typed_pick_request(&config, position))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .expect("heatmap cell hit");
    assert_eq!(
        picked,
        crate::pick::PickedData::MatrixCell {
            source_id: None,
            series_id: "heat".into(),
            x_index: 1,
            y_index: 0,
            distance_px: 0.0,
        }
    );
}

#[test]
fn typed_gpu_pick_prefers_heatmap_contour_level_over_containing_cell() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    for (id, values) in [
        ("x", vec![0.0, 1.0]),
        ("y", vec![0.0, 1.0]),
        ("z0", vec![0.0, 0.0]),
        ("z1", vec![1.0, 1.0]),
    ] {
        renderer.add_column(id, &col_f64(values)).unwrap();
    }
    let mut config = typed_pick_config((-0.5, 1.5), (-0.5, 1.5));
    config.colorbar = Some(crate::default::default_colorbar_options());
    let position = typed_pick_position(&config, 0.5, 0.5);
    let series = SeriesConfig {
        series_id: "field".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::HeatmapContour {
            matrix: crate::data_config::MatrixRef {
                columns: vec!["z0".into(), "z1".into()],
                orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                grid_layout: crate::data_config::GridLayout::Centers,
            },
            fill: crate::data_config::FieldFillConfig {
                mode: crate::data_config::FillMode::Continuous,
                shading: crate::data_config::Shading::Interpolated,
                opacity: 1.0,
            },
            contour: crate::data_config::ContourConfig {
                levels: vec![0.5],
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width: 2.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
                per_level_color: None,
                labels: None,
            },
            surface: None,
        },
    };
    let chart = renderer
        .register_chart(config.clone(), vec![series])
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    let picked = pollster::block_on(
        renderer
            .pick_chart_data(chart, typed_pick_request(&config, position))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .expect("contour hit");
    assert_eq!(
        picked,
        crate::pick::PickedData::ContourLevel {
            source_id: None,
            series_id: "field".into(),
            level_index: 0,
            x_index: 0,
            y_index: 0,
            distance_px: 0.0,
        }
    );
}

#[test]
fn renderer_gpu_picker_activation_failure_is_sticky_without_retrying_creation() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .picker
        .fail_activation_once(crate::gpu_pick::GpuPickError::DeviceLimit {
            resource: "injected picker limit",
            requested: 17,
            limit: 9,
        });
    let mut events = Vec::new();
    for _ in 0..2 {
        let error = renderer
            .enable_gpu_picking_observed(&mut |event| events.push(event))
            .unwrap_err();
        assert!(matches!(
            error,
            crate::gpu_pick::GpuPickError::DeviceLimit {
                resource: "injected picker limit",
                requested: 17,
                limit: 9,
            }
        ));
    }
    assert!(events.is_empty());
    assert!(matches!(
        &renderer.picker.pipeline_state,
        PickerPipelineState::Failed(crate::gpu_pick::GpuPickError::DeviceLimit {
            resource: "injected picker limit",
            requested: 17,
            limit: 9,
        })
    ));
}

#[test]
fn active_picker_ignores_unrelated_mutations_and_reuses_source_only_changes() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let config = crate::default::default_config();
    let active_chart = renderer
        .register_chart(
            config.clone(),
            vec![state_test_scatter_line("series", Some("old"), "x", "y")],
        )
        .unwrap();
    let inactive_chart = renderer
        .register_chart(
            config.clone(),
            vec![state_test_scatter_line("other", None, "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer
        .prepare_gpu_picking_for_chart(active_chart)
        .unwrap();

    let generation = active_picker(&renderer).engine.registry_generation();
    let query_buffer = active_picker(&renderer)
        .engine
        .query_params_buffer(0)
        .unwrap();

    let mut inactive_series = renderer.chart_series(inactive_chart).unwrap()[0].clone();
    let DataRenderType::ScatterLine { line, .. } = &mut inactive_series.render_type else {
        unreachable!();
    };
    line.line_width = 8.0;
    renderer
        .set_chart_series(inactive_chart, vec![inactive_series])
        .unwrap();
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        generation
    );

    let mut decoration_only = config.clone();
    decoration_only.chart_title.text.segments = crate::text::rich_segments_from_text("new title");
    decoration_only.bottom_x.min = -10.0;
    decoration_only.bottom_x.max = 10.0;
    renderer
        .set_chart_config(active_chart, decoration_only)
        .unwrap();
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        generation
    );
    assert_eq!(
        active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap(),
        query_buffer
    );

    let mut identity_only = renderer.chart_series(active_chart).unwrap()[0].clone();
    identity_only.source_id = Some("new".into());
    renderer
        .set_chart_series(active_chart, vec![identity_only])
        .unwrap();
    assert!(active_picker(&renderer).engine.registry_generation() > generation);
    assert_eq!(
        active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap(),
        query_buffer
    );
    assert_eq!(
        active_picker(&renderer).descriptors[0].source_id.as_deref(),
        Some("new")
    );
}

#[test]
fn precise_mode_without_point_mapping_reuses_stylized_picker_resources() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let config = crate::default::default_config();
    let chart = renderer
        .register_chart(config, vec![state_test_scatter("scatter", None, "x", "y")])
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let generation = active_picker(&renderer).engine.registry_generation();
    let query_buffer = active_picker(&renderer)
        .engine
        .query_params_buffer(0)
        .unwrap();
    // No style table, index column or override: the slot has no style map.
    assert!(
        active_picker(&renderer).descriptors[0]
            .resource
            .scatter
            .as_ref()
            .unwrap()
            .style_meta
            .is_none()
    );

    let mut sketch = renderer.chart_config(chart).unwrap().clone();
    sketch.draw_style = DrawStyle::Sketch(Default::default());
    renderer.set_chart_config(chart, sketch).unwrap();

    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        generation
    );
    assert_eq!(
        active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap(),
        query_buffer
    );
    // The sketch plan picks the same markers and nothing else.
    let resource = &active_picker(&renderer).descriptors[0].resource;
    assert!(resource.scatter.is_some());
    assert_eq!(resource.line_width_px, None);
    assert!(resource.vector.is_none());
}

/// The pick never reads colour, so a colour-only edit leaves the pick
/// descriptor equal and reuses the slot: the series colour, a style table
/// colour and an override colour. A size edit in the same rows rebuilds it.
#[test]
fn colour_only_edits_reuse_the_pick_slot() {
    use crate::data_config::DataScatterPointStyleConfig;
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    renderer
        .add_column("style", &col_f64(vec![0.0, 1.0]))
        .unwrap();
    let styled = |series_color: Color, slot_color: Color, override_color: Color, size: f32| {
        let mut scatter = test_scatter_style();
        scatter.point_color = series_color;
        scatter.point_style_table = Some(vec![
            DataScatterPointStyleConfig {
                point_color: Some(slot_color),
                point_shape: None,
                point_size: Some(7.0),
            },
            DataScatterPointStyleConfig {
                point_color: Some(slot_color),
                point_shape: Some(crate::data_config::ScatterShape::Square),
                point_size: None,
            },
        ]);
        scatter.point_style_index_column = Some("style".into());
        scatter.point_style_overrides = Some(vec![DataScatterPointStyleOverride {
            index: 1,
            style: DataScatterPointStyleConfig {
                point_color: Some(override_color),
                point_shape: None,
                point_size: Some(size),
            },
        }]);
        SeriesConfig {
            series_id: "styled".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::Scatter { scatter },
        }
    };
    let red = Color::new(1.0, 0.0, 0.0, 1.0);
    let green = Color::new(0.0, 1.0, 0.0, 1.0);
    let blue = Color::new(0.0, 0.0, 1.0, 0.5);
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![styled(red, green, blue, 9.0)],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let generation = active_picker(&renderer).engine.registry_generation();
    let query_buffer = active_picker(&renderer)
        .engine
        .query_params_buffer(0)
        .unwrap();
    let descriptors = active_picker(&renderer).descriptors.clone();
    assert!(
        descriptors[0]
            .resource
            .scatter
            .as_ref()
            .unwrap()
            .style_meta
            .is_some(),
        "the case runs the precise style map"
    );

    for (what, series) in [
        ("series colour", styled(blue, green, blue, 9.0)),
        ("style table colour", styled(blue, red, blue, 9.0)),
        ("override colour", styled(blue, red, green, 9.0)),
    ] {
        renderer.set_chart_series(chart, vec![series]).unwrap();
        let active = active_picker(&renderer);
        assert_eq!(active.engine.registry_generation(), generation, "{what}");
        assert_eq!(
            active.engine.query_params_buffer(0).unwrap(),
            query_buffer,
            "{what}"
        );
        assert_eq!(active.descriptors, descriptors, "{what}");
    }

    renderer
        .set_chart_series(chart, vec![styled(blue, red, green, 11.0)])
        .unwrap();
    let active = active_picker(&renderer);
    assert!(active.engine.registry_generation() > generation);
    assert_ne!(active.engine.query_params_buffer(0).unwrap(), query_buffer);
    assert_ne!(active.descriptors, descriptors);
}

#[test]
fn constellation_picker_support_matches_rendered_scatter_line_only() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let mut config = crate::default::default_config();
    config.draw_style = DrawStyle::Constellation(Default::default());
    let chart = renderer
        .register_chart(
            config,
            vec![
                state_test_line("line", "x", "y"),
                state_test_scatter("scatter", None, "x", "y"),
                state_test_scatter_line("scatter-line", None, "x", "y"),
            ],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();

    let active = active_picker(&renderer);
    assert_eq!(active.engine.registered_series_count(), 1);
    assert_eq!(active.descriptors.len(), 1);
    assert_eq!(active.descriptors[0].series_id, "scatter-line");

    let mut precise = renderer.chart_config(chart).unwrap().clone();
    precise.draw_style = DrawStyle::Precise;
    renderer.set_chart_config(chart, precise).unwrap();
    assert_eq!(active_picker(&renderer).engine.registered_series_count(), 3);
}

#[test]
fn picker_switches_charts_without_recreating_equal_series_resources() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let config = crate::default::default_config();
    let first = renderer
        .register_chart(
            config.clone(),
            vec![state_test_scatter("same", Some("first"), "x", "y")],
        )
        .unwrap();
    let second = renderer
        .register_chart(
            config,
            vec![state_test_scatter("same", Some("second"), "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(first).unwrap();
    let first_buffer = active_picker(&renderer)
        .engine
        .query_params_buffer(0)
        .unwrap();

    renderer.prepare_gpu_picking_for_chart(second).unwrap();
    assert_eq!(active_picker(&renderer).chart_id, second);
    assert_eq!(
        active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap(),
        first_buffer
    );
    assert_eq!(
        active_picker(&renderer).descriptors[0].source_id.as_deref(),
        Some("second")
    );

    renderer.prepare_gpu_picking_for_chart(first).unwrap();
    assert_eq!(active_picker(&renderer).chart_id, first);
    assert_eq!(
        active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap(),
        first_buffer
    );
}

#[test]
fn picker_reorder_reuses_gpu_slots_and_publishes_new_identity_order() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let initial = vec![
        state_test_scatter("a", Some("source-a0"), "x", "y"),
        state_test_scatter("b", Some("source-b0"), "x", "y"),
    ];
    let chart = renderer
        .register_chart(crate::default::default_config(), initial)
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let a_buffer = active_picker(&renderer)
        .engine
        .query_params_buffer(0)
        .unwrap();
    let b_buffer = active_picker(&renderer)
        .engine
        .query_params_buffer(1)
        .unwrap();

    renderer
        .set_chart_series(
            chart,
            vec![
                state_test_scatter("b", Some("source-b1"), "x", "y"),
                state_test_scatter("a", Some("source-a1"), "x", "y"),
            ],
        )
        .unwrap();

    let active = active_picker(&renderer);
    assert_eq!(active.engine.query_params_buffer(0).unwrap(), b_buffer);
    assert_eq!(active.engine.query_params_buffer(1).unwrap(), a_buffer);
    assert_eq!(
        active.engine.identity_snapshot(),
        vec![
            (Some("source-b1".into()), "b".into()),
            (Some("source-a1".into()), "a".into()),
        ]
    );
    assert_eq!(
        active
            .descriptors
            .iter()
            .map(|descriptor| descriptor.series_id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "a"]
    );
}

#[test]
fn composite_picker_prepare_failure_preserves_all_state_and_retry_succeeds() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let initial_config = crate::default::default_config();
    let initial_series = vec![state_test_line("line", "x", "y")];
    let chart = renderer
        .register_chart(initial_config.clone(), initial_series.clone())
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let generation = active_picker(&renderer).engine.registry_generation();
    let descriptors = active_picker(&renderer).descriptors.clone();
    let render_stamp = renderer.chart_render_stamp(chart).unwrap();
    let visual_revision = renderer.visual_revision();
    active_picker(&renderer).engine.fail_next_registry_prepare();

    let mut changed_series = initial_series.clone();
    let DataRenderType::Line { line } = &mut changed_series[0].render_type else {
        unreachable!();
    };
    line.line_width = 9.0;
    let mut changed_config = initial_config.clone();
    changed_config.chart_title.text.segments =
        crate::text::rich_segments_from_text("prepared but not published");
    assert!(matches!(
        renderer.set_chart_state(chart, changed_config.clone(), changed_series.clone()),
        Err(FiggyError::GpuPick(
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "injected registry prepare failure"
            }
        ))
    ));
    assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
    assert_eq!(
        renderer.chart_series(chart).unwrap(),
        initial_series.as_slice()
    );
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
    assert_eq!(renderer.visual_revision(), visual_revision);
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        generation
    );
    assert_eq!(active_picker(&renderer).descriptors, descriptors);

    renderer
        .set_chart_state(chart, changed_config.clone(), changed_series.clone())
        .unwrap();
    assert_eq!(renderer.chart_config(chart).unwrap(), &changed_config);
    assert_eq!(
        renderer.chart_series(chart).unwrap(),
        changed_series.as_slice()
    );
    assert_ne!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
    assert_ne!(renderer.visual_revision(), visual_revision);
    assert!(active_picker(&renderer).engine.registry_generation() > generation);
    assert_ne!(active_picker(&renderer).descriptors, descriptors);
}

#[test]
fn picker_ticket_resolves_after_chart_registry_pool_and_renderer_drop() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![0.0])).unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    config.bottom_x.min = -1.0;
    config.bottom_x.max = 1.0;
    config.left_y.min = -1.0;
    config.left_y.max = 1.0;
    let chart = renderer
        .register_chart(
            config.clone(),
            vec![state_test_scatter("point", Some("source"), "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    let (display, panel, scale) = display_config_for_surface(&config, (320, 240));
    let area = display.data_area().unwrap().0;
    let ticket = renderer
        .pick_chart(
            chart,
            GpuPickRequest {
                canvas_position_px: [
                    area.x as f32 + area.width as f32 * 0.5,
                    area.y as f32 + area.height as f32 * 0.5,
                ],
                display_panel_px: panel,
                display_scale: scale,
                max_distance_px: 20.0,
            },
        )
        .unwrap();
    renderer.remove_chart(chart).unwrap();
    drop(renderer);

    let picked = pollster::block_on(ticket.resolve()).unwrap().unwrap();
    assert_eq!(picked.source_id.as_deref(), Some("source"));
    assert_eq!(picked.series_id, "point");
    assert_eq!(picked.point_index, 0);
}

#[test]
fn renderer_owned_chart_ids_are_ordered_and_never_reused() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let first = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    let second = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    assert_eq!(renderer.chart_order(), &[first, second]);

    renderer.remove_chart(first).unwrap();
    let third = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    assert_ne!(first, third);
    assert_eq!(renderer.chart_order(), &[second, third]);
    assert!(matches!(
        renderer.chart_config(first),
        Err(FiggyError::UnknownChart { .. })
    ));

    let Some(mut other_renderer) = state_test_renderer() else {
        return;
    };
    let other_first = other_renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    assert_ne!(first, other_first);
    assert_ne!(renderer.visual_revision(), other_renderer.visual_revision());
    assert_ne!(renderer.visual_stamp(), other_renderer.visual_stamp());
    assert_eq!(
        renderer.visual_stamp().font_generation,
        crate::text_render::font_generation()
    );
    assert!(matches!(
        other_renderer.chart_config(first),
        Err(FiggyError::UnknownChart { .. })
    ));
}

#[test]
fn renderer_chart_id_exhaustion_is_failure_atomic() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.next_chart_id = u64::MAX;
    let before_visual = renderer.visual_revision();

    assert!(matches!(
        renderer.register_chart(crate::default::default_config(), Vec::new()),
        Err(FiggyError::CounterExhausted {
            counter: "chart id"
        })
    ));
    assert!(renderer.chart_order().is_empty());
    assert!(renderer.chart_states.is_empty());
    assert_eq!(renderer.visual_revision(), before_visual);
}

fn state_test_heatmap(id: &str, x: &str, y: &str, z: &[&str]) -> SeriesConfig {
    SeriesConfig {
        series_id: id.to_string(),
        source_id: None,
        label: None,
        x_column: x.to_string(),
        y_column: y.to_string(),
        z_column: None,
        render_type: DataRenderType::Heatmap {
            matrix: crate::data_config::MatrixRef {
                columns: z.iter().map(|c| c.to_string()).collect(),
                orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                grid_layout: crate::data_config::GridLayout::Centers,
            },
            fill: crate::data_config::FieldFillConfig {
                mode: crate::data_config::FillMode::Continuous,
                shading: crate::data_config::Shading::Flat,
                opacity: 1.0,
            },
            surface: None,
        },
    }
}

fn state_test_contour(id: &str, x: &str, y: &str, z: &[&str], levels: usize) -> SeriesConfig {
    let mut series = state_test_heatmap(id, x, y, z);
    let DataRenderType::Heatmap { matrix, .. } = series.render_type else {
        unreachable!();
    };
    series.render_type = DataRenderType::Contour {
        matrix,
        contour: state_test_contour_config(levels),
    };
    series
}

fn state_test_heatmap_contour(
    id: &str,
    x: &str,
    y: &str,
    z: &[&str],
    levels: usize,
) -> SeriesConfig {
    let mut series = state_test_heatmap(id, x, y, z);
    let DataRenderType::Heatmap {
        matrix,
        fill,
        surface: _,
    } = series.render_type
    else {
        unreachable!();
    };
    series.render_type = DataRenderType::HeatmapContour {
        matrix,
        fill,
        contour: state_test_contour_config(levels),
        surface: None,
    };
    series
}

fn state_test_contour_config(levels: usize) -> crate::data_config::ContourConfig {
    crate::data_config::ContourConfig {
        levels: (0..levels).map(|level| level as f64).collect(),
        line: DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 1.0,
            fill: None,
            fill_color: None,
            interpolation: crate::data_config::LineInterpolation::Linear,
        },
        per_level_color: None,
        labels: None,
    }
}

fn state_test_labelled_contour(
    spacing_px: f32,
    anchors: Vec<crate::data_config::ContourLabelAnchor>,
) -> SeriesConfig {
    let mut series = state_test_contour("label-cache", "clx", "cly", &["clz0", "clz1", "clz2"], 1);
    let DataRenderType::Contour { contour, .. } = &mut series.render_type else {
        unreachable!();
    };
    contour.levels = vec![2.0];
    contour.line.line_color = Color::new(0.0, 0.2, 0.8, 1.0);
    contour.labels = Some(crate::data_config::ContourLabelConfig {
        visible: true,
        font_size: 18.0,
        color: Color::BLACK,
        format: crate::format::LabelFormat::Decimal,
        significant_digits: 3,
        spacing_px,
        anchors,
        bg_color: Some(Color::new(1.0, 0.0, 1.0, 1.0)),
        bg_padding_px: 2.0,
    });
    series
}

fn state_test_contour_labels_mut(
    series: &mut SeriesConfig,
) -> &mut crate::data_config::ContourLabelConfig {
    let (DataRenderType::Contour { contour, .. } | DataRenderType::HeatmapContour { contour, .. }) =
        &mut series.render_type
    else {
        panic!("expected contour series")
    };
    contour.labels.as_mut().expect("labelled contour")
}

fn assert_contour_spacing_error(result: Result<impl Sized>, product: bool) {
    let Err(FiggyError::InvalidSeriesConfig { reason, .. }) = result else {
        panic!("invalid contour spacing was accepted or returned the wrong error")
    };
    let expected = if product {
        "automatic contour label spacing_px * scale must be finite and greater than zero"
    } else {
        crate::data_config::ContourLabelConfig::INVALID_SPACING_REASON
    };
    assert_eq!(reason, expected);
}

fn add_state_test_contour_grid(renderer: &mut Renderer) {
    renderer
        .add_column("clx", &col_f64(vec![0.0, 0.5, 1.0]))
        .unwrap();
    renderer
        .add_column("cly", &col_f64(vec![0.0, 0.5, 1.0]))
        .unwrap();
    renderer
        .add_column("clz0", &col_f64(vec![0.0, 1.0, 2.0]))
        .unwrap();
    renderer
        .add_column("clz1", &col_f64(vec![1.0, 2.0, 3.0]))
        .unwrap();
    renderer
        .add_column("clz2", &col_f64(vec![2.0, 3.0, 4.0]))
        .unwrap();
}

fn state_test_contour_chart(rect: Rect, x_range: (f64, f64)) -> Chart {
    let mut config = panel_config(rect.width, rect.height);
    config.chart_area = crate::layout::ChartArea(rect);
    config.colorbar = Some(crate::default::default_colorbar_options());
    config.legend.visible = false;
    config.bottom_x.major_spacing = 0.25;
    config.left_y.major_spacing = 0.25;
    let mut chart = Chart::new(config);
    chart.set_x_range(x_range.0, x_range.1);
    chart.set_y_range(0.0, 1.0);
    chart.config_mut().bottom_x.major_spacing = 0.25;
    chart.config_mut().left_y.major_spacing = 0.25;
    chart
}

fn state_test_contour_variant(
    heatmap: bool,
    id: &str,
    x: &str,
    y: &str,
    z: &[&str],
    levels: usize,
) -> SeriesConfig {
    if heatmap {
        state_test_heatmap_contour(id, x, y, z, levels)
    } else {
        state_test_contour(id, x, y, z, levels)
    }
}

fn assert_contour_limit_error(result: Result<impl Sized>) {
    let Err(FiggyError::InvalidSeriesConfig { reason, .. }) = result else {
        panic!("oversized contour was accepted or returned the wrong error")
    };
    assert_eq!(
        reason,
        "contour level count 1025 exceeds the supported maximum 1024"
    );
}

#[test]
fn renderer_owned_series_boundaries_reject_spacing_without_state_changes() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_contour_grid(&mut renderer);
    let mut config = crate::default::default_config();
    config.colorbar = Some(crate::default::default_colorbar_options());
    let chart = renderer.register_chart(config.clone(), Vec::new()).unwrap();

    let invalid = |spacing_px: f32, visible: bool, explicit: bool| {
        let anchors = explicit
            .then_some(crate::data_config::ContourLabelAnchor {
                level_index: 0,
                x: 0.5,
                y: 0.5,
                tx: 1.0,
                ty: 0.0,
            })
            .into_iter()
            .collect();
        let mut series = state_test_labelled_contour(spacing_px, anchors);
        state_test_contour_labels_mut(&mut series).visible = visible;
        series
    };

    for series in [invalid(0.0, false, false), invalid(-1.0, true, true)] {
        let before_series = renderer.chart_series(chart).unwrap().to_vec();
        let before_visual = renderer.visual_revision();
        assert_contour_spacing_error(
            renderer.set_chart_series(chart, vec![series.clone()]),
            false,
        );
        assert_eq!(renderer.chart_series(chart).unwrap(), before_series);
        assert_eq!(renderer.visual_revision(), before_visual);

        assert_contour_spacing_error(
            renderer.set_chart_state(chart, config.clone(), vec![series.clone()]),
            false,
        );
        assert_eq!(renderer.chart_series(chart).unwrap(), before_series);
        assert_eq!(renderer.visual_revision(), before_visual);

        let before_order = renderer.chart_order().to_vec();
        assert_contour_spacing_error(renderer.register_chart(config.clone(), vec![series]), false);
        assert_eq!(renderer.chart_order(), before_order);
        assert_eq!(renderer.visual_revision(), before_visual);
    }
}

#[test]
fn demo_load_rejects_invalid_spacing_without_publishing_candidate_state() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let chart = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    let column = col_f64(vec![0.0, 1.0, 2.0]);
    let before_visual = renderer.visual_revision();
    let before_generation = renderer.pool().generation();
    let result = renderer.begin_load_demo(chart, &column, &column, &column, &column, |_| {
        let mut config = crate::default::default_config();
        config.colorbar = Some(crate::default::default_colorbar_options());
        let mut series = state_test_contour(
            "demo-label",
            DEMO_COLUMN_IDS[0],
            DEMO_COLUMN_IDS[1],
            &[DEMO_COLUMN_IDS[2]],
            1,
        );
        let DataRenderType::Contour { contour, .. } = &mut series.render_type else {
            unreachable!()
        };
        contour.labels = Some(crate::data_config::ContourLabelConfig {
            visible: false,
            font_size: 12.0,
            color: Color::BLACK,
            format: crate::format::LabelFormat::Decimal,
            significant_digits: 3,
            spacing_px: f32::NAN,
            anchors: Vec::new(),
            bg_color: None,
            bg_padding_px: 0.0,
        });
        Ok((config, vec![series]))
    });
    assert_contour_spacing_error(result, false);
    assert!(renderer.chart_series(chart).unwrap().is_empty());
    assert_eq!(renderer.visual_revision(), before_visual);
    assert_eq!(renderer.pool().generation(), before_generation);
}

#[test]
fn prepare_preflights_hidden_invalid_spacing_before_gpu_mutation() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_contour_grid(&mut renderer);
    let chart = state_test_contour_chart(
        Rect {
            x: 0,
            y: 0,
            width: 240,
            height: 180,
        },
        (0.0, 1.0),
    );
    let mut config = state_test_labelled_contour(0.0, Vec::new());
    state_test_contour_labels_mut(&mut config).visible = false;
    let style = renderer.create_style_for_series(&config).unwrap();
    let series = [Series {
        config: &config,
        style: &style,
    }];
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];
    let before = renderer
        .gpu_memory_usage()
        .creations_of(GpuResourceKind::ContourScratch);
    assert_contour_spacing_error(renderer.prepare(&items), false);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .creations_of(GpuResourceKind::ContourScratch),
        before
    );
    assert!(renderer.contour_label_pipelines.is_none());
}

#[test]
fn export_checks_clamped_automatic_spacing_product_only_for_automatic_labels() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_contour_grid(&mut renderer);
    let chart = state_test_contour_chart(
        Rect {
            x: 0,
            y: 0,
            width: 240,
            height: 180,
        },
        (0.0, 1.0),
    );
    let huge = f32::MAX;
    // `huge * 2.0` overflows to infinity: the automatic labels' spacing
    // product is checked, the explicit and hidden ones never multiply.
    let scale = 2.0;
    let automatic = [state_test_labelled_contour(huge, Vec::new())];
    let before = renderer
        .gpu_memory_usage()
        .creations_of(GpuResourceKind::ContourScratch);
    assert_contour_spacing_error(renderer.export_panel_rgba(&chart, &automatic, scale), true);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .creations_of(GpuResourceKind::ContourScratch),
        before
    );

    let anchor = crate::data_config::ContourLabelAnchor {
        level_index: 0,
        x: 0.5,
        y: 0.5,
        tx: 1.0,
        ty: 0.0,
    };
    let explicit = [state_test_labelled_contour(huge, vec![anchor])];
    renderer
        .export_panel_rgba(&chart, &explicit, scale)
        .expect("explicit placement must not multiply spacing by export scale");

    let mut hidden = state_test_labelled_contour(huge, Vec::new());
    state_test_contour_labels_mut(&mut hidden).visible = false;
    renderer
        .export_panel_rgba(&chart, &[hidden], scale)
        .expect("hidden labels must not multiply spacing by export scale");
    // An infinite scale is no longer clamped: it cannot fit the device.
    assert!(matches!(
        renderer.export_panel_rgba(&chart, &explicit, f32::INFINITY),
        Err(FiggyError::GpuResourceLimit { .. })
    ));
}

#[test]
fn contour_label_font_generation_drift_forces_an_atlas_cache_miss() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_contour_grid(&mut renderer);
    let chart = state_test_contour_chart(
        Rect {
            x: 0,
            y: 0,
            width: 240,
            height: 180,
        },
        (0.0, 1.0),
    );
    let config = state_test_labelled_contour(90.0, Vec::new());
    let style = renderer.create_style_for_series(&config).unwrap();
    let series = [Series {
        config: &config,
        style: &style,
    }];
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];

    let first = renderer.prepare(&items).unwrap();
    let before = renderer.contour_cache["label-cache"]
        .contour()
        .unwrap()
        .label
        .as_ref()
        .unwrap()
        .atlas_identity();
    renderer
        .contour_cache
        .get_mut("label-cache")
        .unwrap()
        .contour_mut()
        .unwrap()
        .signature
        .label
        .as_mut()
        .unwrap()
        .font_generation = crate::text_render::font_generation().wrapping_add(1);

    let second = renderer.prepare(&items).unwrap();
    let entry = renderer.contour_cache["label-cache"].contour().unwrap();
    let after = entry.label.as_ref().unwrap().atlas_identity();
    assert_ne!(before, after, "font generation drift reused a stale atlas");
    assert_eq!(
        entry.signature.label.as_ref().unwrap().font_generation,
        crate::text_render::font_generation()
    );
    drop((first, second));
}

#[test]
fn register_and_set_cover_both_contour_variants_at_the_level_boundary() {
    for heatmap in [false, true] {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("z0", &col_f64(vec![0.0, 1.0])).unwrap();
        let mut config = crate::default::default_config();
        config.colorbar = Some(crate::default::default_colorbar_options());

        let chart = renderer
            .register_chart(
                config.clone(),
                vec![state_test_contour_variant(
                    heatmap,
                    "accepted",
                    "x",
                    "y",
                    &["z0"],
                    crate::data_config::MAX_CONTOUR_LEVELS,
                )],
            )
            .unwrap();
        renderer
            .set_chart_series(
                chart,
                vec![state_test_contour_variant(
                    heatmap,
                    "set-accepted",
                    "x",
                    "y",
                    &["z0"],
                    crate::data_config::MAX_CONTOUR_LEVELS,
                )],
            )
            .unwrap();
        let before_revision = renderer.visual_revision();
        assert_contour_limit_error(renderer.set_chart_series(
            chart,
            vec![state_test_contour_variant(
                heatmap,
                "rejected",
                "x",
                "y",
                &["z0"],
                crate::data_config::MAX_CONTOUR_LEVELS + 1,
            )],
        ));
        assert_eq!(
            declared_level_count(&renderer.chart_series(chart).unwrap()[0].render_type),
            crate::data_config::MAX_CONTOUR_LEVELS
        );
        assert_eq!(renderer.visual_revision(), before_revision);

        let chart_count = renderer.chart_order().len();
        let before_revision = renderer.visual_revision();
        assert_contour_limit_error(renderer.register_chart(
            config,
            vec![state_test_contour_variant(
                heatmap,
                "register-rejected",
                "x",
                "y",
                &["z0"],
                crate::data_config::MAX_CONTOUR_LEVELS + 1,
            )],
        ));
        assert_eq!(renderer.chart_order().len(), chart_count);
        assert_eq!(renderer.visual_revision(), before_revision);
    }
}

#[test]
fn contour_level_validation_is_identical_for_contour_and_heatmap_contour() {
    let allowed = [
        state_test_contour(
            "contour",
            "x",
            "y",
            &["z0"],
            crate::data_config::MAX_CONTOUR_LEVELS,
        ),
        state_test_heatmap_contour(
            "heatmap-contour",
            "x",
            "y",
            &["z0"],
            crate::data_config::MAX_CONTOUR_LEVELS,
        ),
    ];
    for series in &allowed {
        validate_contour_level_count(&series.series_id, &series.render_type).unwrap();
    }

    let rejected = [
        state_test_contour(
            "contour",
            "x",
            "y",
            &["z0"],
            crate::data_config::MAX_CONTOUR_LEVELS + 1,
        ),
        state_test_heatmap_contour(
            "heatmap-contour",
            "x",
            "y",
            &["z0"],
            crate::data_config::MAX_CONTOUR_LEVELS + 1,
        ),
    ];
    let reasons: Vec<String> = rejected
        .iter()
        .map(|series| {
            let FiggyError::InvalidSeriesConfig { reason, .. } =
                validate_contour_level_count(&series.series_id, &series.render_type).unwrap_err()
            else {
                panic!("wrong contour limit error")
            };
            reason
        })
        .collect();
    assert_eq!(reasons[0], reasons[1]);
}

#[test]
fn direct_prepare_covers_both_contour_variants_at_the_level_boundary() {
    for heatmap in [false, true] {
        let Some((mut renderer, chart, _)) = field_prepare_fixture() else {
            return;
        };
        let view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let accepted = state_test_contour_variant(
            heatmap,
            "field",
            "fx",
            "fy",
            &["fz0", "fz1"],
            crate::data_config::MAX_CONTOUR_LEVELS,
        );
        let style = renderer.create_style_for_series(&accepted).unwrap();
        let series = [Series {
            config: &accepted,
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        drop(renderer.prepare(&items).unwrap());
        let creations = renderer
            .gpu_memory_usage()
            .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable);
        let field_cache_len = renderer.field_cache.len();
        let contour_cache_len = renderer.contour_cache.len();

        let rejected = state_test_contour_variant(
            heatmap,
            "rejected",
            "fx",
            "fy",
            &["fz0", "fz1"],
            crate::data_config::MAX_CONTOUR_LEVELS + 1,
        );
        let style = renderer.create_style_for_series(&rejected).unwrap();
        let series = [Series {
            config: &rejected,
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        assert_contour_limit_error(renderer.prepare(&items));
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            creations
        );
        assert_eq!(renderer.field_cache.len(), field_cache_len);
        assert_eq!(renderer.contour_cache.len(), contour_cache_len);
    }
}

#[test]
fn direct_export_covers_both_contour_variants_at_the_level_boundary() {
    for heatmap in [false, true] {
        let Some((mut renderer, chart, _)) = field_prepare_fixture() else {
            return;
        };
        let accepted = [state_test_contour_variant(
            heatmap,
            "field",
            "fx",
            "fy",
            &["fz0", "fz1"],
            crate::data_config::MAX_CONTOUR_LEVELS,
        )];
        let image = renderer.export_panel_rgba(&chart, &accepted, 1.0).unwrap();
        assert_eq!(image.width, chart.config().chart_area.0.width);
        assert_eq!(image.height, chart.config().chart_area.0.height);

        let creations = renderer
            .gpu_memory_usage()
            .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable);
        let rejected = [state_test_contour_variant(
            heatmap,
            "rejected",
            "fx",
            "fy",
            &["fz0", "fz1"],
            crate::data_config::MAX_CONTOUR_LEVELS + 1,
        )];
        assert_contour_limit_error(renderer.export_panel_rgba(&chart, &rejected, 1.0));
        assert_eq!(
            renderer
                .gpu_memory_usage()
                .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            creations
        );
    }
}

#[test]
fn demo_load_covers_both_contour_variants_at_the_level_boundary() {
    for heatmap in [false, true] {
        let Some(mut renderer) = state_test_renderer() else {
            return;
        };
        let chart = renderer
            .register_chart(crate::default::default_config(), Vec::new())
            .unwrap();
        let column = col_f64(vec![0.0, 1.0]);
        let mut config = crate::default::default_config();
        config.colorbar = Some(crate::default::default_colorbar_options());
        renderer
            .begin_load_demo(chart, &column, &column, &column, &column, |_| {
                Ok((
                    config,
                    vec![state_test_contour_variant(
                        heatmap,
                        "accepted",
                        DEMO_COLUMN_IDS[0],
                        DEMO_COLUMN_IDS[1],
                        &[DEMO_COLUMN_IDS[2]],
                        crate::data_config::MAX_CONTOUR_LEVELS,
                    )],
                ))
            })
            .unwrap()
            .commit();
        assert_eq!(
            declared_level_count(&renderer.chart_series(chart).unwrap()[0].render_type),
            crate::data_config::MAX_CONTOUR_LEVELS
        );

        let before_revision = renderer.visual_revision();
        let before_series = renderer.chart_series(chart).unwrap().to_vec();
        let before_generation = renderer.pool().generation();
        let before_layout_generation = renderer.pool().layout_generation();
        let before_handles = DEMO_COLUMN_IDS.map(|id| renderer.pool().handle_for(id).unwrap());
        let before_epochs = DEMO_COLUMN_IDS.map(|id| renderer.pool().allocation_epoch(id).unwrap());
        let mut rejected_config = crate::default::default_config();
        rejected_config.colorbar = Some(crate::default::default_colorbar_options());
        let result = renderer.begin_load_demo(chart, &column, &column, &column, &column, |_| {
            Ok((
                rejected_config,
                vec![state_test_contour_variant(
                    heatmap,
                    "rejected",
                    DEMO_COLUMN_IDS[0],
                    DEMO_COLUMN_IDS[1],
                    &[DEMO_COLUMN_IDS[2]],
                    crate::data_config::MAX_CONTOUR_LEVELS + 1,
                )],
            ))
        });
        assert_contour_limit_error(result);
        assert_eq!(renderer.chart_series(chart).unwrap(), before_series);
        assert_eq!(renderer.visual_revision(), before_revision);
        assert_eq!(renderer.pool().generation(), before_generation);
        assert_eq!(
            renderer.pool().layout_generation(),
            before_layout_generation
        );
        for (id, before) in DEMO_COLUMN_IDS.into_iter().zip(before_handles) {
            let after = renderer.pool().handle_for(id).unwrap();
            assert_eq!(after.generation, before.generation);
            assert_eq!(after.offset, before.offset);
            assert_eq!(after.byte_size, before.byte_size);
            assert_eq!(after.len_values, before.len_values);
        }
        assert_eq!(
            DEMO_COLUMN_IDS.map(|id| renderer.pool().allocation_epoch(id).unwrap()),
            before_epochs
        );
    }
}

// A field series without `Config.colorbar` is not a chart missing a
// decoration — it is a chart with no z range and no colormap, so there is no
// value to draw. Rejected, and rejected atomically.
#[test]
fn a_field_series_without_a_colorbar_is_rejected() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let chart = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("z0", &col_f64(vec![1.0, 2.0])).unwrap();
    let before = renderer.visual_revision();

    let field = vec![state_test_heatmap("field", "x", "y", &["z0"])];
    assert!(matches!(
        renderer.set_chart_series(chart, field.clone()),
        Err(FiggyError::InvalidSeriesConfig { .. })
    ));
    assert!(renderer.chart_series(chart).unwrap().is_empty());
    assert_eq!(renderer.visual_revision(), before);

    // With a colourbar it is accepted — including a hidden one, which is how
    // a field is drawn without the bar beside it.
    let mut config = crate::default::default_config();
    let mut bar = crate::default::default_colorbar_options();
    bar.visible = false;
    config.colorbar = Some(bar);
    renderer.set_chart_state(chart, config, field).unwrap();
    assert_eq!(renderer.chart_series(chart).unwrap().len(), 1);
}

// The grid's constituent columns are referenced columns: they must exist,
// and the reference must be reachable by everything that walks a series'
// columns (validation, the remove-column cascade, derived identity).
#[test]
fn a_matrix_column_must_be_registered_and_is_a_referenced_column() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let mut config = crate::default::default_config();
    config.colorbar = Some(crate::default::default_colorbar_options());
    let chart = renderer.register_chart(config, Vec::new()).unwrap();
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("z0", &col_f64(vec![1.0, 2.0])).unwrap();

    assert!(matches!(
        renderer.set_chart_series(
            chart,
            vec![state_test_heatmap("field", "x", "y", &["z0", "z_missing"])]
        ),
        Err(FiggyError::UnknownColumn { .. })
    ));

    let series = state_test_heatmap("field", "x", "y", &["z0"]);
    renderer
        .set_chart_series(chart, vec![series.clone()])
        .unwrap();
    assert!(series_references_column(&series, "z0"));
    assert!(!series_references_column(&series, "z_other"));

    // A declaration that does not line up with the data is not an error —
    // so a grid declaring more columns than exist in the pool is only
    // rejected for the missing *registration*, never for the count.
    let mut visited = Vec::new();
    visit_series_columns(&series, &mut |column| visited.push(column.to_string()));
    assert!(visited.iter().any(|column| column == "z0"));
}

// The paired reducer's domain is index-aligned pairs, which the field and bar
// types are not. Fields use the distinct lattice mode; histograms stay on
// metadata. Their stamps remain in the fit token in either case.
#[test]
fn field_and_bar_types_are_not_submitted_to_the_paired_extent_reducer() {
    use crate::gpu_errorbar::GpuSeriesExtentMode;

    let matrix = crate::data_config::MatrixRef {
        columns: vec!["z0".into()],
        orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
        grid_layout: crate::data_config::GridLayout::Centers,
    };
    let fill = crate::data_config::FieldFillConfig {
        mode: crate::data_config::FillMode::Continuous,
        shading: crate::data_config::Shading::Flat,
        opacity: 1.0,
    };
    let contour = crate::data_config::ContourConfig {
        levels: vec![1.0],
        line: DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 1.0,
            fill: None,
            fill_color: None,
            interpolation: crate::data_config::LineInterpolation::Linear,
        },
        per_level_color: None,
        labels: None,
    };
    let unpaired = [
        DataRenderType::Histogram {
            bar: crate::data_config::DataBarStyleConfig {
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
        DataRenderType::Heatmap {
            matrix: matrix.clone(),
            fill: fill.clone(),
            surface: None,
        },
        DataRenderType::Contour {
            matrix: matrix.clone(),
            contour: contour.clone(),
        },
        DataRenderType::HeatmapContour {
            matrix,
            fill,
            contour,
            surface: None,
        },
    ];
    for render_type in &unpaired {
        assert_eq!(
            GpuSeriesExtentMode::from_render_type(render_type),
            None,
            "{render_type:?} must not go through the paired reducer"
        );
        // And they draw neither of the paired primitives, so nothing
        // downstream compiles a pipeline or a layer for them yet.
        assert!(!has_line(render_type));
        assert!(!has_scatter(render_type));
        assert!(!has_errorbar(render_type));
        assert!(!constellation_supports(render_type));
    }
    // The paired types still are.
    assert_eq!(
        GpuSeriesExtentMode::from_render_type(&DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        }),
        Some(GpuSeriesExtentMode::Line)
    );
}

#[test]
fn draw_style_supports_matches_the_primitives_each_style_draws() {
    let line = DataLineStyleConfig {
        line_style: LineStylePreset::Solid,
        line_color: Color::BLACK,
        line_width: 1.0,
        fill: None,
        fill_color: None,
        interpolation: crate::data_config::LineInterpolation::Linear,
    };
    let scatter = crate::data_config::DataScatterStyleConfig {
        point_color: Color::BLACK,
        point_shape: crate::data_config::ScatterShape::Circle,
        point_size: 4.0,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    };
    let matrix = crate::data_config::MatrixRef {
        columns: vec!["z0".into()],
        orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
        grid_layout: crate::data_config::GridLayout::Centers,
    };
    let fill = crate::data_config::FieldFillConfig {
        mode: crate::data_config::FillMode::Continuous,
        shading: crate::data_config::Shading::Flat,
        opacity: 1.0,
    };
    let contour = crate::data_config::ContourConfig {
        levels: vec![1.0],
        line: line.clone(),
        per_level_color: None,
        labels: None,
    };
    let line_rt = DataRenderType::Line { line: line.clone() };
    let scatter_rt = DataRenderType::Scatter {
        scatter: scatter.clone(),
    };
    let marker_line_rt = DataRenderType::ScatterLine {
        scatter: scatter.clone(),
        line: line.clone(),
    };
    let precise_only = [
        DataRenderType::Histogram {
            bar: crate::data_config::DataBarStyleConfig {
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
        DataRenderType::Heatmap {
            matrix: matrix.clone(),
            fill: fill.clone(),
            surface: None,
        },
        DataRenderType::Contour {
            matrix: matrix.clone(),
            contour: contour.clone(),
        },
        DataRenderType::HeatmapContour {
            matrix,
            fill,
            contour,
            surface: None,
        },
    ];
    let err = crate::data_config::DataErrorBarStyleConfig {
        error_bar_color: Color::BLACK,
        error_bar_width: 1.0,
        error_bar_cap_size: 4.0,
        cap_width: 1.0,
        error_bar_style_table: None,
        error_bar_style_index_column: None,
        error_bar_style_overrides: None,
    };
    let sigma = || crate::data_config::ErrorRef::Symmetric { column: "e".into() };
    let errorbar_types = [
        DataRenderType::ScatterErrorbarX {
            scatter: scatter.clone(),
            err_x: sigma(),
            err_style: err.clone(),
        },
        DataRenderType::ScatterErrorbarY {
            scatter: scatter.clone(),
            err_y: sigma(),
            err_style: err.clone(),
        },
        DataRenderType::ScatterErrorbarXY {
            scatter: scatter.clone(),
            err_x: sigma(),
            err_y: sigma(),
            err_style: err.clone(),
        },
        DataRenderType::LineScatterErrorbarX {
            scatter: scatter.clone(),
            line: line.clone(),
            err_x: sigma(),
            err_style: err.clone(),
        },
        DataRenderType::LineScatterErrorbarY {
            scatter: scatter.clone(),
            line: line.clone(),
            err_y: sigma(),
            err_style: err.clone(),
        },
        DataRenderType::LineScatterErrorbarXY {
            scatter: scatter.clone(),
            line: line.clone(),
            err_x: sigma(),
            err_y: sigma(),
            err_style: err,
        },
    ];
    let sketch = DrawStyle::Sketch(Default::default());
    let milkyway = DrawStyle::Milkyway(Default::default());
    let constellation = DrawStyle::Constellation(Default::default());
    // Errorbars follow the series' scatter in Sketch and Milkyway; the
    // constellation style drops errorbar series entirely.
    for rt in &errorbar_types {
        assert!(draw_style_supports(&DrawStyle::Precise, rt), "{rt:?}");
        assert!(draw_style_supports(&sketch, rt), "sketch {rt:?}");
        assert!(draw_style_supports(&milkyway, rt), "milkyway {rt:?}");
        assert!(
            !draw_style_supports(&constellation, rt),
            "constellation {rt:?}"
        );
    }

    for rt in [&line_rt, &scatter_rt, &marker_line_rt]
        .into_iter()
        .chain(&precise_only)
    {
        assert!(draw_style_supports(&DrawStyle::Precise, rt), "{rt:?}");
    }
    for rt in [&line_rt, &scatter_rt, &marker_line_rt] {
        assert!(draw_style_supports(&sketch, rt), "sketch {rt:?}");
        assert!(draw_style_supports(&milkyway, rt), "milkyway {rt:?}");
    }
    for rt in &precise_only {
        assert!(!draw_style_supports(&sketch, rt), "sketch {rt:?}");
        assert!(!draw_style_supports(&milkyway, rt), "milkyway {rt:?}");
        assert!(
            !draw_style_supports(&constellation, rt),
            "constellation {rt:?}"
        );
    }
    assert!(draw_style_supports(&constellation, &marker_line_rt));
    assert!(!draw_style_supports(&constellation, &line_rt));
    assert!(!draw_style_supports(&constellation, &scatter_rt));
}

#[test]
fn renderer_field_fit_resolves_the_same_sample_endpoints_as_contour_draw() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("fit-gx", &col_f64(vec![-3.1, -2.9, 2.9, 3.1]))
        .unwrap();
    renderer
        .add_column("fit-gy", &col_f64(vec![-2.5, -2.3, 2.3, 2.5]))
        .unwrap();
    for (id, values) in [
        ("fit-z0", vec![0.0, 1.0, 2.0]),
        ("fit-z1", vec![1.0, 2.0, 3.0]),
        ("fit-z2", vec![2.0, 3.0, 4.0]),
    ] {
        renderer.add_column(id, &col_f64(values)).unwrap();
    }
    let mut series = state_test_contour(
        "field-fit",
        "fit-gx",
        "fit-gy",
        &["fit-z0", "fit-z1", "fit-z2"],
        1,
    );
    let DataRenderType::Contour { matrix, .. } = &mut series.render_type else {
        unreachable!()
    };
    matrix.grid_layout = crate::data_config::GridLayout::Edges;

    pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();
    let extent = pollster::block_on(
        renderer
            .begin_series_fit_extent(&series)
            .unwrap()
            .expect("contour uses a GPU field-fit ticket")
            .resolve(),
    )
    .unwrap()
    .expect("the 3x3 contour lattice has an extent");
    assert!((extent.x.min + 3.0).abs() < 1.0e-6);
    assert!((extent.x.max - 3.0).abs() < 1.0e-6);
    assert!((extent.y.min + 2.4).abs() < 1.0e-6);
    assert!((extent.y.max - 2.4).abs() < 1.0e-6);
}

#[test]
fn renderer_owned_series_validation_is_failure_atomic() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let chart = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    let before_revision = renderer.visual_revision();

    let missing = vec![state_test_line("missing", "x", "y")];
    assert!(matches!(
        renderer.set_chart_series(chart, missing),
        Err(FiggyError::UnknownColumn { .. })
    ));
    assert!(renderer.chart_series(chart).unwrap().is_empty());
    assert_eq!(renderer.visual_revision(), before_revision);

    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let duplicate = vec![
        state_test_line("same", "x", "y"),
        state_test_line("same", "x", "y"),
    ];
    assert!(matches!(
        renderer.set_chart_series(chart, duplicate),
        Err(FiggyError::InvalidSeriesConfig { .. })
    ));
    assert!(renderer.chart_series(chart).unwrap().is_empty());
    assert_eq!(renderer.visual_revision(), before_revision);
}

#[test]
fn renderer_owned_combined_state_validation_is_failure_atomic() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let initial_config = crate::default::default_config();
    let initial_series = vec![state_test_line("line", "x", "y")];
    let chart = renderer
        .register_chart(initial_config.clone(), initial_series.clone())
        .unwrap();
    let initial_stamp = renderer.chart_render_stamp(chart).unwrap();

    let mut changed_config = initial_config.clone();
    changed_config.chart_title.text.segments =
        crate::text::rich_segments_from_text("must not publish");
    let invalid_series = vec![state_test_line("missing", "x", "missing")];
    assert!(matches!(
        renderer.set_chart_state(chart, changed_config, invalid_series),
        Err(FiggyError::UnknownColumn { .. })
    ));
    assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
    assert_eq!(renderer.chart_series(chart).unwrap(), initial_series);
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), initial_stamp);

    let mut invalid_config = initial_config.clone();
    invalid_config.bottom_x.max = invalid_config.bottom_x.min;
    assert!(matches!(
        renderer.set_chart_state(chart, invalid_config, Vec::new()),
        Err(FiggyError::InvalidConfig { .. })
    ));
    assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
    assert_eq!(renderer.chart_series(chart).unwrap(), initial_series);
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), initial_stamp);
}

#[test]
fn renderer_owned_config_and_view_validation_are_failure_atomic() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let mut invalid = crate::default::default_config();
    invalid.bottom_x.max = invalid.bottom_x.min;
    let visual_before = renderer.visual_revision();
    assert!(matches!(
        renderer.register_chart(invalid, Vec::new()),
        Err(FiggyError::InvalidConfig {
            field: "bottom_x",
            ..
        })
    ));
    assert!(renderer.chart_order().is_empty());
    assert_eq!(renderer.visual_revision(), visual_before);

    let mut invalid_layout = crate::default::default_config();
    invalid_layout.left_y.out_margin = invalid_layout.chart_area.0.width as f32;
    assert!(matches!(
        renderer.register_chart(invalid_layout, Vec::new()),
        Err(FiggyError::InvalidConfig {
            field: "layout",
            ..
        })
    ));
    assert!(renderer.chart_order().is_empty());
    assert_eq!(renderer.visual_revision(), visual_before);

    let chart = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    let config_before = renderer.chart_config(chart).unwrap().clone();
    let view_before = renderer.chart_view_state(chart).unwrap();
    let visual_before = renderer.visual_revision();
    let mut invalid_view = view_before.clone();
    invalid_view.left_y.min = f64::NAN;
    assert!(matches!(
        renderer.set_chart_view_state(chart, invalid_view),
        Err(FiggyError::InvalidConfig {
            field: "left_y",
            ..
        })
    ));
    assert_eq!(renderer.chart_config(chart).unwrap(), &config_before);
    assert_eq!(renderer.chart_view_state(chart).unwrap(), view_before);
    assert_eq!(renderer.visual_revision(), visual_before);

    let mut invalid_layout_view = view_before.clone();
    invalid_layout_view.chart_area.width = 1;
    invalid_layout_view.chart_area.height = 1;
    assert!(matches!(
        renderer.set_chart_view_state(chart, invalid_layout_view),
        Err(FiggyError::InvalidConfig {
            field: "layout",
            ..
        })
    ));
    assert_eq!(renderer.chart_config(chart).unwrap(), &config_before);
    assert_eq!(renderer.chart_view_state(chart).unwrap(), view_before);
    assert_eq!(renderer.visual_revision(), visual_before);
}

#[test]
fn set_chart_config_rejects_removing_required_colorbar_without_publishing() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    for id in ["x", "y", "z"] {
        renderer.add_column(id, &col_f64(vec![1.0, 2.0])).unwrap();
    }
    let mut config = crate::default::default_config();
    config.colorbar = Some(crate::default::default_colorbar_options());
    let series = SeriesConfig {
        series_id: "heat".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Heatmap {
            matrix: crate::data_config::MatrixRef {
                columns: vec!["z".into()],
                orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                grid_layout: crate::data_config::GridLayout::Centers,
            },
            fill: crate::data_config::FieldFillConfig {
                mode: crate::data_config::FillMode::Continuous,
                shading: crate::data_config::Shading::Flat,
                opacity: 1.0,
            },
            surface: None,
        },
    };
    let chart = renderer
        .register_chart(config.clone(), vec![series])
        .unwrap();
    let stamp = renderer.chart_render_stamp(chart).unwrap();
    let visual = renderer.visual_revision();
    let mut invalid = config.clone();
    invalid.colorbar = None;
    assert!(matches!(
        renderer.set_chart_config(chart, invalid),
        Err(FiggyError::InvalidSeriesConfig { .. })
    ));
    assert_eq!(renderer.chart_config(chart).unwrap(), &config);
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), stamp);
    assert_eq!(renderer.visual_revision(), visual);
}

#[test]
fn style_map_and_uniforms_are_admitted_and_charged_together() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let mut series = state_test_scatter("mapped", None, "x", "y");
    let DataRenderType::Scatter { scatter } = &mut series.render_type else {
        unreachable!();
    };
    scatter.point_style_table = Some(vec![DataScatterPointStyleConfig {
        point_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
        ..Default::default()
    }]);
    let bytes = renderer.style_allocation_bytes(&series).unwrap();
    assert_eq!(
        bytes,
        4 * std::mem::size_of::<PrimitiveStyle>() as u64
            + std::mem::size_of::<data_render::ScatterStyleSlotGpu>() as u64
            + std::mem::size_of::<data_render::ScatterStyleOverrideGpu>() as u64
            + std::mem::size_of::<data_render::ScatterStyleMapMeta>() as u64
    );
    let before = renderer.gpu_memory_usage();
    let _ = renderer.set_memory_budget(Some(before.total_bytes() + bytes - 1));
    assert!(matches!(
        renderer.create_style_for_series(&series),
        Err(FiggyError::GpuResourceLimit {
            resource: "chart style budget",
            ..
        })
    ));
    let rejected = renderer.gpu_memory_usage();
    assert_eq!(rejected.total_bytes(), before.total_bytes());
    assert_eq!(
        rejected.creations_of(GpuResourceKind::Uniform),
        before.creations_of(GpuResourceKind::Uniform)
    );

    let _ = renderer.set_memory_budget(Some(before.total_bytes() + bytes));
    let style = renderer.create_style_for_series(&series).unwrap();
    assert!(style.scatter_map.is_some());
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::Uniform),
        before.live_bytes_of(GpuResourceKind::Uniform) + bytes
    );
    drop(style);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .retired_bytes_of(GpuResourceKind::Uniform),
        before.retired_bytes_of(GpuResourceKind::Uniform) + bytes
    );
}

#[test]
fn prepared_frame_keeps_style_charge_after_style_owner_is_dropped() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let cfg = state_test_scatter("point", None, "x", "y");
    let chart = Chart::new(crate::default::default_config());
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let style_bytes = renderer.style_allocation_bytes(&cfg).unwrap();
    let style = renderer.create_style_for_series(&cfg).unwrap();
    let frame = {
        let series = [Series {
            config: &cfg,
            style: &style,
        }];
        renderer
            .prepare(&[ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &series,
            }])
            .unwrap()
    };
    let live_with_frame = renderer
        .gpu_memory_usage()
        .live_bytes_of(GpuResourceKind::Uniform);
    drop(style);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::Uniform),
        live_with_frame
    );
    drop(frame);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::Uniform),
        live_with_frame - style_bytes
    );
}

#[test]
fn renderer_owned_explicit_setters_always_issue_checked_revisions() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let config = crate::default::default_config();
    let chart = renderer.register_chart(config.clone(), Vec::new()).unwrap();

    let after_register = renderer.visual_revision();
    renderer.set_chart_config(chart, config.clone()).unwrap();
    let after_config = renderer.visual_revision();
    assert_ne!(after_register, after_config);
    let raster_before_series = renderer.chart_states[&chart].revisions.raster;
    renderer.set_chart_series(chart, Vec::new()).unwrap();
    let after_series = renderer.visual_revision();
    assert_ne!(after_config, after_series);
    assert_ne!(
        renderer.chart_states[&chart].revisions.raster,
        raster_before_series
    );
    renderer.set_chart_selection(chart, None).unwrap();
    assert_ne!(after_series, renderer.visual_revision());

    let before_config = renderer.chart_config(chart).unwrap().clone();
    let before_visual = renderer.visual_revision();
    renderer.visual_revision = RenderRevision {
        renderer_identity: renderer.renderer_identity,
        sequence: u64::MAX,
    };
    assert!(matches!(
        renderer.set_chart_config(chart, config),
        Err(FiggyError::CounterExhausted { .. })
    ));
    assert_eq!(renderer.chart_config(chart).unwrap(), &before_config);
    renderer.visual_revision = before_visual;
}

#[test]
fn chart_render_stamp_classifies_renderer_owned_dirty_state() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("unused", &col_f64(vec![9.0, 9.0]))
        .unwrap();
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();

    let initial = renderer.chart_render_stamp(chart).unwrap();
    assert!(initial.needs_draw_since(None));
    assert!(initial.needs_raster_since(None));

    renderer.remove_column("unused").unwrap();
    assert!(renderer.defragment().unwrap());
    let after_maintenance = renderer.chart_render_stamp(chart).unwrap();
    assert_eq!(after_maintenance, initial);

    // Replacing the config with an identical one is not an edit.
    let same_config = renderer.chart_config(chart).unwrap().clone();
    renderer.set_chart_config(chart, same_config).unwrap();
    let unchanged = renderer.chart_render_stamp(chart).unwrap();
    assert_eq!(unchanged, initial);

    // A chrome edit draws and re-rasterizes the axis layers.
    let mut chrome_config = renderer.chart_config(chart).unwrap().clone();
    chrome_config.chart_title.visible = !chrome_config.chart_title.visible;
    renderer.set_chart_config(chart, chrome_config).unwrap();
    let decoration = renderer.chart_render_stamp(chart).unwrap();
    assert!(decoration.needs_draw_since(Some(&initial)));
    assert!(decoration.needs_raster_since(Some(&initial)));

    // Layer-only and selection-only edits draw without touching the raster.
    let mut layer_config = renderer.chart_config(chart).unwrap().clone();
    layer_config.active_layer = crate::LayerRef::Annotations;
    layer_config.annotations.visible = false;
    renderer.set_chart_config(chart, layer_config).unwrap();
    let layer = renderer.chart_render_stamp(chart).unwrap();
    assert!(layer.needs_draw_since(Some(&decoration)));
    assert!(!layer.needs_raster_since(Some(&decoration)));
    let mut picked_config = renderer.chart_config(chart).unwrap().clone();
    picked_config.picked_points = Some(crate::config::PickedPointsConfig::default());
    renderer.set_chart_config(chart, picked_config).unwrap();
    let picked = renderer.chart_render_stamp(chart).unwrap();
    assert!(picked.needs_draw_since(Some(&layer)));
    assert!(!picked.needs_raster_since(Some(&layer)));

    let mut view_config = renderer.chart_config(chart).unwrap().clone();
    view_config.bottom_x.min -= 1.0;
    renderer.set_chart_config(chart, view_config).unwrap();
    let view = renderer.chart_render_stamp(chart).unwrap();
    assert!(view.needs_draw_since(Some(&picked)));
    assert!(view.needs_raster_since(Some(&picked)));

    renderer
        .upsert_column("x", &col_f64(vec![10.0, 11.0]))
        .unwrap();
    let data = renderer.chart_render_stamp(chart).unwrap();
    assert!(data.needs_draw_since(Some(&view)));
    assert!(!data.needs_raster_since(Some(&view)));
}

#[test]
fn renderer_font_generation_sync_is_atomic_and_idempotent() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let chart = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    let generation = crate::text_render::font_generation();
    renderer.observed_font_generation = if generation == 0 {
        u64::MAX
    } else {
        generation - 1
    };
    let before = renderer.visual_revision();
    let stamp = renderer.chart_render_stamp(chart).unwrap();
    assert!(renderer.sync_external_invalidations().unwrap());
    assert_ne!(renderer.visual_revision(), before);
    // Text is rasterized with the fonts: the axis chrome and the annotation
    // layer both have to be redone, and the stamp has to say so.
    let after = renderer.chart_render_stamp(chart).unwrap();
    assert!(after.needs_raster_since(Some(&stamp)));
    assert!(after.needs_annotation_raster_since(Some(&stamp)));
    assert!(!renderer.sync_external_invalidations().unwrap());
    assert!(renderer.chart_config(chart).is_ok());
}

#[test]
fn column_removal_with_chart_config_moves_the_annotation_revision_with_its_items() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let initial_config = crate::default::default_config();
    let chart = renderer
        .register_chart(
            initial_config.clone(),
            vec![state_test_scatter("scatter", None, "x", "y")],
        )
        .unwrap();

    // An override that leaves the items alone keeps the annotation
    // revision, so a host does not redo a raster that did not change.
    let mut same_items = initial_config.clone();
    same_items.chart_title.text.segments = crate::text::rich_segments_from_text("renamed");
    let before = renderer.chart_render_stamp(chart).unwrap();
    assert!(
        renderer
            .remove_column_with_chart_config("x", chart, same_items.clone())
            .unwrap()
    );
    let after = renderer.chart_render_stamp(chart).unwrap();
    assert!(after.needs_raster_since(Some(&before)));
    assert!(!after.needs_annotation_raster_since(Some(&before)));

    // An override that edits the items moves it.
    renderer.add_column("z", &col_f64(vec![0.0, 1.0])).unwrap();
    let mut edited_items = same_items.clone();
    edited_items
        .annotations
        .items
        .push(crate::annotation::Annotation::new(
            crate::annotation::AnnotationKind::Span {
                axis: crate::annotation::AnnotationAxis::X,
                from: 0.2,
                to: 0.4,
                fill: Color::new(1.0, 0.0, 0.0, 0.5),
            },
        ));
    let before = after;
    assert!(
        renderer
            .remove_column_with_chart_config("z", chart, edited_items.clone())
            .unwrap()
    );
    let after = renderer.chart_render_stamp(chart).unwrap();
    assert!(after.needs_annotation_raster_since(Some(&before)));
    assert_eq!(renderer.chart_config(chart).unwrap(), &edited_items);
}

#[test]
fn renderer_column_upsert_invalidates_only_referencing_charts() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    renderer.add_column("z", &col_f64(vec![2.0, 3.0])).unwrap();
    let affected = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("affected", "x", "y")],
        )
        .unwrap();
    let unrelated = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("unrelated", "z", "y")],
        )
        .unwrap();
    let affected_before = renderer.chart_states[&affected].revisions.desired;
    let unrelated_before = renderer.chart_states[&unrelated].revisions.desired;
    let visual_before = renderer.visual_revision();

    renderer
        .upsert_column("x", &col_f64(vec![10.0, 11.0]))
        .unwrap();

    assert_ne!(
        renderer.chart_states[&affected].revisions.desired,
        affected_before
    );
    assert_eq!(
        renderer.chart_states[&unrelated].revisions.desired,
        unrelated_before
    );
    assert_ne!(renderer.visual_revision(), visual_before);
}

#[test]
fn load_demo_guard_drop_preserves_renderer_authority_and_retry_commits() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let old = col_f64(vec![1.0, 2.0, 3.0]);
    for id in DEMO_COLUMN_IDS {
        renderer.add_column(id, &old).unwrap();
    }
    let initial_config = crate::default::default_config();
    let initial_series = vec![state_test_scatter(
        "old-demo",
        Some("old"),
        "demo_x",
        "demo_sin",
    )];
    let chart = renderer
        .register_chart(initial_config.clone(), initial_series.clone())
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();

    let revisions = revision_snapshot(&renderer, chart);
    let visual = renderer.visual_revision();
    let generation = renderer.pool().generation();
    let layout_generation = renderer.pool().layout_generation();
    let epochs = DEMO_COLUMN_IDS.map(|id| renderer.pool().allocation_epoch(id).unwrap());
    let handles = DEMO_COLUMN_IDS.map(|id| renderer.pool().handle_for(id).unwrap());
    let picker_generation = active_picker(&renderer).engine.registry_generation();
    let picker_descriptors = active_picker(&renderer).descriptors.clone();
    let columns = demo_test_columns();
    let mut final_config = initial_config.clone();
    final_config.chart_title.text.segments = crate::text::rich_segments_from_text("atomic demo");
    let final_series = vec![state_test_scatter(
        "new-demo",
        Some("new"),
        "demo_t",
        "demo_rc",
    )];
    {
        let pending = renderer
            .begin_load_demo(
                chart,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
                |_| Ok((final_config.clone(), final_series.clone())),
            )
            .unwrap();
        drop(pending);
    }

    assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
    assert_eq!(renderer.chart_series(chart).unwrap(), initial_series);
    assert_eq!(revision_snapshot(&renderer, chart), revisions);
    assert_eq!(renderer.visual_revision(), visual);
    assert_eq!(renderer.pool().generation(), generation);
    assert_eq!(renderer.pool().layout_generation(), layout_generation);
    assert_eq!(
        DEMO_COLUMN_IDS.map(|id| renderer.pool().allocation_epoch(id).unwrap()),
        epochs
    );
    for (id, handle) in DEMO_COLUMN_IDS.into_iter().zip(handles) {
        let current = renderer.pool().handle_for(id).unwrap();
        assert_eq!(current.generation, handle.generation);
        assert_eq!(current.offset, handle.offset);
        assert_eq!(current.byte_size, handle.byte_size);
        assert_eq!(current.len_values, handle.len_values);
    }
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        picker_generation
    );
    assert_eq!(active_picker(&renderer).descriptors, picker_descriptors);

    renderer
        .begin_load_demo(
            chart,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
            |_| Ok((final_config.clone(), final_series.clone())),
        )
        .unwrap()
        .commit();
    assert_eq!(renderer.chart_config(chart).unwrap(), &final_config);
    assert_eq!(renderer.chart_series(chart).unwrap(), final_series);
    assert_ne!(revision_snapshot(&renderer, chart), revisions);
    assert_ne!(renderer.visual_revision(), visual);
    assert_eq!(renderer.pool().generation(), generation + 1);
    assert_eq!(renderer.pool().layout_generation(), layout_generation + 1);
    assert!(
        DEMO_COLUMN_IDS
            .into_iter()
            .zip(epochs)
            .all(|(id, epoch)| renderer.pool().allocation_epoch(id) != Some(epoch))
    );
    assert!(active_picker(&renderer).engine.registry_generation() > picker_generation);
    assert!(!renderer.has_pending_maintenance());
}

#[test]
fn load_demo_rebuilds_changed_slots_for_a_different_active_chart() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let old = col_f64(vec![1.0, 2.0, 3.0]);
    for id in DEMO_COLUMN_IDS {
        renderer.add_column(id, &old).unwrap();
    }
    add_state_test_xy(&mut renderer, "stable-x", "stable-y", vec![1.0, 2.0, 3.0]);
    let target = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    let other_series = vec![
        state_test_scatter("changed", None, "demo_x", "demo_sin"),
        state_test_scatter("stable", None, "stable-x", "stable-y"),
    ];
    let other = renderer
        .register_chart(crate::default::default_config(), other_series.clone())
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(other).unwrap();
    let changed_buffer = active_picker(&renderer)
        .engine
        .query_params_buffer(0)
        .unwrap();
    let stable_buffer = active_picker(&renderer)
        .engine
        .query_params_buffer(1)
        .unwrap();
    let other_revisions = revision_snapshot(&renderer, other);
    let columns = demo_test_columns();
    let target_series = vec![state_test_line("demo", "demo_t", "demo_rc")];

    renderer
        .begin_load_demo(
            target,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
            |_| Ok((crate::default::default_config(), target_series.clone())),
        )
        .unwrap()
        .commit();

    assert_eq!(active_picker(&renderer).chart_id, other);
    assert_ne!(
        active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap(),
        changed_buffer
    );
    assert_eq!(
        active_picker(&renderer)
            .engine
            .query_params_buffer(1)
            .unwrap(),
        stable_buffer
    );
    assert_eq!(renderer.chart_series(other).unwrap(), other_series);
    assert_ne!(revision_snapshot(&renderer, other), other_revisions);
    assert_eq!(renderer.chart_series(target).unwrap(), target_series);
}

#[test]
fn load_demo_picker_failure_preserves_pool_chart_and_picker_then_retries() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let old = col_f64(vec![1.0, 2.0]);
    for id in DEMO_COLUMN_IDS {
        renderer.add_column(id, &old).unwrap();
    }
    let initial_series = vec![state_test_scatter("demo", None, "demo_x", "demo_sin")];
    let chart = renderer
        .register_chart(crate::default::default_config(), initial_series.clone())
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let generation = renderer.pool().generation();
    let layout_generation = renderer.pool().layout_generation();
    let revisions = revision_snapshot(&renderer, chart);
    let visual = renderer.visual_revision();
    let picker_generation = active_picker(&renderer).engine.registry_generation();
    active_picker(&renderer).engine.fail_next_registry_prepare();
    let columns = demo_test_columns();

    assert!(matches!(
        renderer.begin_load_demo(
            chart,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
            |_| Ok((crate::default::default_config(), initial_series.clone())),
        ),
        Err(FiggyError::GpuPick(
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "injected registry prepare failure"
            }
        ))
    ));
    assert_eq!(renderer.pool().generation(), generation);
    assert_eq!(renderer.pool().layout_generation(), layout_generation);
    assert_eq!(revision_snapshot(&renderer, chart), revisions);
    assert_eq!(renderer.visual_revision(), visual);
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        picker_generation
    );

    renderer
        .begin_load_demo(
            chart,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
            |_| Ok((crate::default::default_config(), initial_series.clone())),
        )
        .unwrap()
        .commit();
    assert_eq!(renderer.pool().generation(), generation + 1);
}

#[test]
fn load_demo_revision_failure_happens_before_publication() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let chart = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    let original = renderer.chart_states[&chart].revisions.desired;
    renderer
        .chart_states
        .get_mut(&chart)
        .unwrap()
        .revisions
        .desired = RenderRevision {
        renderer_identity: original.renderer_identity,
        sequence: u64::MAX,
    };
    let generation = renderer.pool().generation();
    let layout_generation = renderer.pool().layout_generation();
    let visual = renderer.visual_revision();
    let columns = demo_test_columns();

    assert!(matches!(
        renderer.begin_load_demo(
            chart,
            &columns[0],
            &columns[1],
            &columns[2],
            &columns[3],
            |_| Ok((crate::default::default_config(), Vec::new())),
        ),
        Err(FiggyError::CounterExhausted {
            counter: "chart desired revision"
        })
    ));
    assert_eq!(renderer.pool().generation(), generation);
    assert_eq!(renderer.pool().layout_generation(), layout_generation);
    assert_eq!(renderer.visual_revision(), visual);
    assert_eq!(
        renderer.chart_states[&chart].revisions.desired.sequence,
        u64::MAX
    );
}

#[test]
fn active_picker_upsert_failure_rolls_back_every_authority_and_retry_succeeds() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_scatter("scatter", None, "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();

    let pool_generation = renderer.pool().generation();
    let layout_generation = renderer.pool().layout_generation();
    let picker_generation = active_picker(&renderer).engine.registry_generation();
    let picker_buffer = active_picker(&renderer)
        .engine
        .query_params_buffer(0)
        .unwrap();
    let render_stamp = renderer.chart_render_stamp(chart).unwrap();
    let visual_revision = renderer.visual_revision();
    let old_min = renderer.pool().slot("x").unwrap().min;
    active_picker(&renderer).engine.fail_next_registry_prepare();

    assert!(matches!(
        renderer.upsert_column("x", &col_f64(vec![10.0, 11.0])),
        Err(FiggyError::GpuPick(
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "injected registry prepare failure"
            }
        ))
    ));
    assert_eq!(renderer.pool().slot("x").unwrap().min, old_min);
    assert_eq!(renderer.pool().generation(), pool_generation);
    assert_eq!(renderer.pool().layout_generation(), layout_generation);
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
    assert_eq!(renderer.visual_revision(), visual_revision);
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        picker_generation
    );
    assert_eq!(
        active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap(),
        picker_buffer
    );
    assert!(!renderer.has_pending_maintenance());

    renderer
        .upsert_column("x", &col_f64(vec![10.0, 11.0]))
        .unwrap();
    assert_eq!(renderer.pool().slot("x").unwrap().min, 10.0);
    assert_ne!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
    assert!(active_picker(&renderer).engine.registry_generation() > picker_generation);
    assert_ne!(
        active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap(),
        picker_buffer
    );
}

#[test]
fn active_picker_removal_failure_rolls_back_pool_chart_and_registry() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_scatter("scatter", None, "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();

    let pool_generation = renderer.pool().generation();
    let picker_generation = active_picker(&renderer).engine.registry_generation();
    let render_stamp = renderer.chart_render_stamp(chart).unwrap();
    active_picker(&renderer).engine.fail_next_registry_prepare();

    assert!(matches!(
        renderer.remove_column("x"),
        Err(FiggyError::GpuPick(
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "injected registry prepare failure"
            }
        ))
    ));
    assert!(renderer.pool().slot("x").is_some());
    assert_eq!(renderer.pool().generation(), pool_generation);
    assert_eq!(renderer.chart_series(chart).unwrap().len(), 1);
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        picker_generation
    );
    assert_eq!(active_picker(&renderer).engine.registered_series_count(), 1);
    assert!(!renderer.has_pending_maintenance());

    assert!(renderer.remove_column("x").unwrap());
    assert!(renderer.pool().slot("x").is_none());
    assert!(renderer.chart_series(chart).unwrap().is_empty());
    assert_eq!(active_picker(&renderer).engine.registered_series_count(), 0);
    assert!(renderer.has_pending_maintenance());
}

#[test]
fn column_removal_with_chart_config_picker_failure_preserves_all_authority() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let initial_config = crate::default::default_config();
    let initial_series = vec![state_test_scatter("scatter", None, "x", "y")];
    let chart = renderer
        .register_chart(initial_config.clone(), initial_series.clone())
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();

    let mut next_config = initial_config.clone();
    next_config.chart_title.text.segments =
        crate::text::rich_segments_from_text("atomic legend replacement");
    let pool_generation = renderer.pool().generation();
    let render_stamp = renderer.chart_render_stamp(chart).unwrap();
    let visual_revision = renderer.visual_revision();
    let picker_generation = active_picker(&renderer).engine.registry_generation();
    let picker_descriptors = active_picker(&renderer).descriptors.clone();
    active_picker(&renderer).engine.fail_next_registry_prepare();

    assert!(matches!(
        renderer.remove_column_with_chart_config("x", chart, next_config.clone()),
        Err(FiggyError::GpuPick(
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "injected registry prepare failure"
            }
        ))
    ));
    assert!(renderer.pool().slot("x").is_some());
    assert_eq!(renderer.pool().generation(), pool_generation);
    assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
    assert_eq!(
        renderer.chart_series(chart).unwrap(),
        initial_series.as_slice()
    );
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
    assert_eq!(renderer.visual_revision(), visual_revision);
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        picker_generation
    );
    assert_eq!(active_picker(&renderer).descriptors, picker_descriptors);
    assert!(!renderer.has_pending_maintenance());

    assert!(
        renderer
            .remove_column_with_chart_config("x", chart, next_config.clone())
            .unwrap()
    );
    assert!(renderer.pool().slot("x").is_none());
    assert_eq!(renderer.chart_config(chart).unwrap(), &next_config);
    assert!(renderer.chart_series(chart).unwrap().is_empty());
    assert_eq!(active_picker(&renderer).engine.registered_series_count(), 0);
    assert!(renderer.has_pending_maintenance());
}

#[test]
fn column_removal_with_chart_config_counter_failure_preserves_all_authority() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let initial_config = crate::default::default_config();
    let initial_series = vec![state_test_scatter("scatter", None, "x", "y")];
    let chart = renderer
        .register_chart(initial_config.clone(), initial_series.clone())
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let renderer_identity = renderer.renderer_identity;
    renderer
        .chart_states
        .get_mut(&chart)
        .unwrap()
        .revisions
        .config = RenderRevision {
        renderer_identity,
        sequence: u64::MAX,
    };

    let mut next_config = initial_config.clone();
    next_config.chart_title.text.segments =
        crate::text::rich_segments_from_text("must not publish");
    let pool_generation = renderer.pool().generation();
    let visual_revision = renderer.visual_revision();
    let picker_generation = active_picker(&renderer).engine.registry_generation();
    let picker_descriptors = active_picker(&renderer).descriptors.clone();
    let pending_maintenance = renderer.has_pending_maintenance();

    assert!(matches!(
        renderer.remove_column_with_chart_config("x", chart, next_config),
        Err(FiggyError::CounterExhausted {
            counter: "chart config revision"
        })
    ));
    assert!(renderer.pool().slot("x").is_some());
    assert_eq!(renderer.pool().generation(), pool_generation);
    assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
    assert_eq!(
        renderer.chart_series(chart).unwrap(),
        initial_series.as_slice()
    );
    assert_eq!(renderer.visual_revision(), visual_revision);
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        picker_generation
    );
    assert_eq!(active_picker(&renderer).descriptors, picker_descriptors);
    assert_eq!(renderer.has_pending_maintenance(), pending_maintenance);
}

#[test]
fn column_removal_with_invalid_chart_config_preserves_all_authority() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let initial_config = crate::default::default_config();
    let initial_series = vec![state_test_scatter("scatter", None, "x", "y")];
    let chart = renderer
        .register_chart(initial_config.clone(), initial_series.clone())
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();

    let mut invalid_config = initial_config.clone();
    invalid_config.bottom_x.max = invalid_config.bottom_x.min;
    let pool_generation = renderer.pool().generation();
    let render_stamp = renderer.chart_render_stamp(chart).unwrap();
    let visual_revision = renderer.visual_revision();
    let picker_generation = active_picker(&renderer).engine.registry_generation();
    let picker_descriptors = active_picker(&renderer).descriptors.clone();

    assert!(matches!(
        renderer.remove_column_with_chart_config("x", chart, invalid_config),
        Err(FiggyError::InvalidConfig {
            field: "bottom_x",
            ..
        })
    ));
    assert!(renderer.pool().slot("x").is_some());
    assert_eq!(renderer.pool().generation(), pool_generation);
    assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
    assert_eq!(
        renderer.chart_series(chart).unwrap(),
        initial_series.as_slice()
    );
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
    assert_eq!(renderer.visual_revision(), visual_revision);
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        picker_generation
    );
    assert_eq!(active_picker(&renderer).descriptors, picker_descriptors);
    assert!(!renderer.has_pending_maintenance());
}

#[test]
fn missing_column_does_not_publish_chart_config_override() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let initial_config = crate::default::default_config();
    let chart = renderer
        .register_chart(initial_config.clone(), Vec::new())
        .unwrap();
    let render_stamp = renderer.chart_render_stamp(chart).unwrap();
    let visual_revision = renderer.visual_revision();
    let mut next_config = initial_config.clone();
    next_config.chart_title.text.segments =
        crate::text::rich_segments_from_text("missing column must not publish");

    assert!(
        !renderer
            .remove_column_with_chart_config("missing", chart, next_config)
            .unwrap()
    );
    assert_eq!(renderer.chart_config(chart).unwrap(), &initial_config);
    assert_eq!(renderer.chart_render_stamp(chart).unwrap(), render_stamp);
    assert_eq!(renderer.visual_revision(), visual_revision);
    assert!(!renderer.has_pending_maintenance());
}

#[test]
fn removing_errorbar_column_removes_active_scatter_picker_slot() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    renderer
        .add_column("y_err", &col_f64(vec![0.1, 0.2]))
        .unwrap();
    let series = SeriesConfig {
        series_id: "scatter-errors".into(),
        source_id: Some("source".into()),
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::ScatterErrorbarY {
            scatter: test_scatter_style(),
            err_y: ErrorRef::Symmetric {
                column: "y_err".into(),
            },
            err_style: test_errorbar_style(),
        },
    };
    let chart = renderer
        .register_chart(crate::default::default_config(), vec![series])
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    assert_eq!(active_picker(&renderer).engine.registered_series_count(), 1);

    assert!(renderer.remove_column("y_err").unwrap());
    assert!(renderer.chart_series(chart).unwrap().is_empty());
    assert_eq!(active_picker(&renderer).engine.registered_series_count(), 0);
}

#[test]
fn active_picker_defrag_failure_rolls_back_and_noop_keeps_registry_generation() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("hole", &col_f64(vec![9.0, 9.0]))
        .unwrap();
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_scatter("scatter", None, "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    assert!(renderer.remove_column("hole").unwrap());

    let old_x = renderer.pool().handle_for("x").unwrap();
    let pool_generation = renderer.pool().generation();
    let layout_generation = renderer.pool().layout_generation();
    let picker_generation = active_picker(&renderer).engine.registry_generation();
    active_picker(&renderer).engine.fail_next_registry_prepare();

    assert!(matches!(
        renderer.defragment(),
        Err(FiggyError::GpuPick(
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "injected registry prepare failure"
            }
        ))
    ));
    let rolled_back_x = renderer.pool().handle_for("x").unwrap();
    assert_eq!(rolled_back_x.generation, old_x.generation);
    assert_eq!(rolled_back_x.offset, old_x.offset);
    assert_eq!(rolled_back_x.byte_size, old_x.byte_size);
    assert_eq!(rolled_back_x.len_values, old_x.len_values);
    assert_eq!(renderer.pool().generation(), pool_generation);
    assert_eq!(renderer.pool().layout_generation(), layout_generation);
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        picker_generation
    );
    assert!(renderer.has_pending_maintenance());

    assert!(renderer.defragment().unwrap());
    assert_ne!(
        renderer.pool().handle_for("x").unwrap().offset,
        old_x.offset
    );
    let rebound_generation = active_picker(&renderer).engine.registry_generation();
    assert!(rebound_generation > picker_generation);
    assert!(!renderer.has_pending_maintenance());

    assert!(!renderer.defragment().unwrap());
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        rebound_generation
    );
}

#[test]
fn on_alloc_failure_compaction_rebinds_active_picker_transactionally() {
    let Some(mut renderer) = state_test_renderer_with_capacity(1024) else {
        return;
    };
    let block = || col_f64(vec![1.0; 32]);
    renderer.add_column("x", &block()).unwrap();
    renderer.add_column("hole-a", &block()).unwrap();
    renderer.add_column("y", &block()).unwrap();
    renderer.add_column("hole-b", &block()).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_scatter("scatter", None, "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let picker_generation = active_picker(&renderer).engine.registry_generation();
    let picker_buffer = active_picker(&renderer)
        .engine
        .query_params_buffer(0)
        .unwrap();
    let old_y_offset = renderer.pool().handle_for("y").unwrap().offset;

    assert!(renderer.remove_column("hole-a").unwrap());
    assert!(renderer.remove_column("hole-b").unwrap());
    renderer.set_defrag_policy(DefragPolicy::OnAllocFailure);
    renderer.add_column("new", &col_f64(vec![2.0; 64])).unwrap();

    assert_ne!(
        renderer.pool().handle_for("y").unwrap().offset,
        old_y_offset
    );
    assert!(active_picker(&renderer).engine.registry_generation() > picker_generation);
    assert_eq!(
        active_picker(&renderer)
            .engine
            .query_params_buffer(0)
            .unwrap(),
        picker_buffer
    );
    assert!(!renderer.has_pending_maintenance());
}

#[test]
fn dropped_renderer_column_upsert_rolls_back_pool_and_chart_state() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let visual_before = renderer.visual_revision();
    let desired_before = renderer.chart_states[&chart].revisions.desired;
    let min_before = renderer.pool().slot("x").unwrap().min;

    {
        let guard = renderer
            .begin_upsert_column("x", &col_f64(vec![20.0, 21.0]))
            .unwrap();
        assert_eq!(guard.pool().slot("x").unwrap().min, 20.0);
    }

    assert_eq!(renderer.pool().slot("x").unwrap().min, min_before);
    assert_eq!(renderer.visual_revision(), visual_before);
    assert_eq!(
        renderer.chart_states[&chart].revisions.desired,
        desired_before
    );
}

#[test]
fn provisional_upsert_snapshot_matches_commit_and_disappears_on_drop() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let before = renderer.web_derived_snapshot(chart).unwrap();

    {
        let guard = renderer
            .begin_upsert_column("x", &col_f64(vec![10.0, 11.0]))
            .unwrap();
        let prospective = guard.web_derived_snapshot(chart).unwrap();
        assert_ne!(prospective.stamp(), before.stamp());
        assert_ne!(prospective.fit_token(), before.fit_token());
    }
    assert_eq!(
        renderer.web_derived_snapshot(chart).unwrap().stamp(),
        before.stamp()
    );
    assert_eq!(
        renderer.web_derived_snapshot(chart).unwrap().fit_token(),
        before.fit_token()
    );

    let (prospective_stamp, prospective_fit_token) = {
        let guard = renderer
            .begin_upsert_column("x", &col_f64(vec![20.0, 21.0]))
            .unwrap();
        let snapshot = guard.web_derived_snapshot(chart).unwrap();
        let stamp = snapshot.stamp().clone();
        let fit_token = snapshot.fit_token().clone();
        guard.commit();
        (stamp, fit_token)
    };
    assert_eq!(
        renderer.web_derived_snapshot(chart).unwrap().stamp(),
        &prospective_stamp
    );
    assert_eq!(
        renderer.web_derived_snapshot(chart).unwrap().fit_token(),
        &prospective_fit_token
    );
}

#[test]
fn renderer_column_remove_cascades_across_registered_charts() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    for (id, values) in [
        ("x", vec![0.0, 1.0]),
        ("y", vec![1.0, 2.0]),
        ("z", vec![2.0, 3.0]),
    ] {
        renderer.add_column(id, &col_f64(values)).unwrap();
    }
    let first = renderer
        .register_chart(
            crate::default::default_config(),
            vec![
                state_test_line("removed", "x", "y"),
                state_test_line("kept", "z", "y"),
            ],
        )
        .unwrap();
    let second = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("also-removed", "x", "y")],
        )
        .unwrap();
    let first_raster = renderer.chart_states[&first].revisions.raster;
    let second_raster = renderer.chart_states[&second].revisions.raster;
    let second_config = renderer.chart_config(second).unwrap().clone();
    let mut next_first_config = renderer.chart_config(first).unwrap().clone();
    next_first_config.chart_title.text.segments =
        crate::text::rich_segments_from_text("first chart replacement");

    assert!(
        renderer
            .remove_column_with_chart_config("x", first, next_first_config.clone())
            .unwrap()
    );
    assert!(renderer.pool().slot("x").is_none());
    assert_eq!(renderer.chart_config(first).unwrap(), &next_first_config);
    assert_eq!(renderer.chart_config(second).unwrap(), &second_config);
    assert_eq!(renderer.chart_series(first).unwrap().len(), 1);
    assert_eq!(renderer.chart_series(first).unwrap()[0].series_id, "kept");
    assert!(renderer.chart_series(second).unwrap().is_empty());
    assert_ne!(renderer.chart_states[&first].revisions.raster, first_raster);
    assert_ne!(
        renderer.chart_states[&second].revisions.raster,
        second_raster
    );
    assert!(renderer.has_pending_maintenance());
}

#[test]
fn renderer_column_revision_exhaustion_preflights_before_pool_mutation() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let renderer_identity = renderer.renderer_identity;
    renderer
        .chart_states
        .get_mut(&chart)
        .unwrap()
        .revisions
        .desired = RenderRevision {
        renderer_identity,
        sequence: u64::MAX,
    };
    let min_before = renderer.pool().slot("x").unwrap().min;

    let error = renderer
        .begin_upsert_column("x", &col_f64(vec![30.0, 31.0]))
        .err()
        .expect("revision exhaustion must reject before the pool changes");
    assert!(matches!(error, FiggyError::CounterExhausted { .. }));
    assert_eq!(renderer.pool().slot("x").unwrap().min, min_before);
}

#[test]
fn web_derived_stamp_tracks_layout_while_fit_token_survives_defrag_only() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("hole", &col_f64(vec![9.0, 9.0]))
        .unwrap();
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let before = renderer.web_derived_snapshot(chart).unwrap();
    let token = renderer.begin_fit_commit(chart).unwrap();

    assert!(renderer.remove_column("hole").unwrap());
    assert!(renderer.defragment().unwrap());
    let after = renderer.web_derived_snapshot(chart).unwrap();
    assert_ne!(before.stamp(), after.stamp());

    let x_extent = crate::chart::FitExtent {
        min: 10.0,
        max: 20.0,
        min_positive: Some(10.0),
    };
    let y_extent = crate::chart::FitExtent {
        min: -5.0,
        max: 5.0,
        min_positive: Some(5.0),
    };
    renderer
        .commit_auto_fit_all_if_current(&token, &x_extent, &y_extent, 0.0)
        .unwrap();
    let config = renderer.chart_config(chart).unwrap();
    assert_eq!((config.bottom_x.min, config.bottom_x.max), (10.0, 20.0));
    assert_eq!((config.left_y.min, config.left_y.max), (-5.0, 5.0));
}

#[test]
fn web_derived_stamp_tracks_picker_source_identity() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let before = renderer.web_derived_snapshot(chart).unwrap();
    let mut series = renderer.chart_series(chart).unwrap()[0].clone();
    series.source_id = Some("replacement-source".to_string());

    renderer.set_chart_series(chart, vec![series]).unwrap();

    let after = renderer.web_derived_snapshot(chart).unwrap();
    assert_ne!(before.stamp(), after.stamp());
}

#[test]
fn resident_fit_token_survives_source_identity_only_edit() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let token = renderer.begin_fit_commit(chart).unwrap();
    let mut series = renderer.chart_series(chart).unwrap()[0].clone();
    series.source_id = Some("replacement-source".to_string());
    renderer.set_chart_series(chart, vec![series]).unwrap();
    let extent = crate::FitExtent {
        min: 10.0,
        max: 20.0,
        min_positive: Some(10.0),
    };
    renderer
        .commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0)
        .unwrap();
    assert_eq!(
        renderer.chart_series(chart).unwrap()[0]
            .source_id
            .as_deref(),
        Some("replacement-source")
    );
}

#[test]
fn stale_fit_token_rejects_without_partially_updating_axes() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let token = renderer.begin_fit_commit(chart).unwrap();
    let mut changed = renderer.chart_config(chart).unwrap().clone();
    changed.bottom_x.min = -25.0;
    changed.bottom_x.max = 25.0;
    changed.left_y.min = -50.0;
    changed.left_y.max = 50.0;
    renderer.set_chart_config(chart, changed.clone()).unwrap();

    let extent = crate::chart::FitExtent {
        min: 100.0,
        max: 200.0,
        min_positive: Some(100.0),
    };
    assert!(matches!(
        renderer.commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0),
        Err(FiggyError::StaleStateToken { .. })
    ));
    assert_eq!(renderer.chart_config(chart).unwrap(), &changed);
}

#[test]
fn fit_token_tracks_referenced_column_content() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let token = renderer.begin_fit_commit(chart).unwrap();
    renderer
        .upsert_column("x", &col_f64(vec![10.0, 11.0]))
        .unwrap();

    let extent = crate::chart::FitExtent {
        min: 10.0,
        max: 11.0,
        min_positive: Some(10.0),
    };
    assert!(matches!(
        renderer.commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0),
        Err(FiggyError::StaleStateToken { .. })
    ));
}

#[test]
fn fit_token_tracks_normalized_series_extent_mode() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("series", "x", "y")],
        )
        .unwrap();
    let line_token = renderer.begin_fit_commit(chart).unwrap();
    let mut scatter = state_test_line("series", "x", "y");
    scatter.render_type = DataRenderType::Scatter {
        scatter: test_scatter_style(),
    };
    renderer.set_chart_series(chart, vec![scatter]).unwrap();

    let extent = crate::chart::FitExtent {
        min: 10.0,
        max: 20.0,
        min_positive: Some(10.0),
    };
    assert!(matches!(
        renderer.commit_auto_fit_all_if_current(&line_token, &extent, &extent, 0.0),
        Err(FiggyError::StaleStateToken { .. })
    ));
}

#[test]
fn fit_token_ignores_title_only_config_change_and_preserves_it_on_commit() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let token = renderer.begin_fit_commit(chart).unwrap();
    let mut config = renderer.chart_config(chart).unwrap().clone();
    config.chart_title.visible = true;
    config.chart_title.text.segments = crate::text::rich_segments_from_text("newer title");
    renderer.set_chart_config(chart, config).unwrap();

    let extent = crate::chart::FitExtent {
        min: 10.0,
        max: 20.0,
        min_positive: Some(10.0),
    };
    renderer
        .commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0)
        .unwrap();
    let committed = renderer.chart_config(chart).unwrap();
    assert_eq!(
        committed
            .chart_title
            .text
            .segments
            .iter()
            .map(|segment| segment.text)
            .collect::<String>(),
        "newer title"
    );
    assert_eq!(
        (committed.bottom_x.min, committed.bottom_x.max),
        (10.0, 20.0)
    );
}

#[test]
fn foreign_or_removed_fit_token_is_stale_not_an_unknown_chart_alias() {
    let Some(mut first) = state_test_renderer() else {
        return;
    };
    let Some(mut second) = state_test_renderer() else {
        return;
    };
    for renderer in [&mut first, &mut second] {
        renderer.add_column("x", &col_f64(vec![0.0, 1.0])).unwrap();
        renderer.add_column("y", &col_f64(vec![1.0, 2.0])).unwrap();
    }
    let first_chart = first
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let second_chart = second
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("line", "x", "y")],
        )
        .unwrap();
    let token = first.begin_fit_commit(first_chart).unwrap();
    let extent = crate::chart::FitExtent {
        min: 10.0,
        max: 20.0,
        min_positive: Some(10.0),
    };
    assert!(matches!(
        second.commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0),
        Err(FiggyError::StaleStateToken { .. })
    ));

    first.remove_chart(first_chart).unwrap();
    assert!(matches!(
        first.commit_auto_fit_all_if_current(&token, &extent, &extent, 0.0),
        Err(FiggyError::StaleStateToken { .. })
    ));
    assert!(second.chart_config(second_chart).is_ok());
}

fn typecheck_windowed_load_demo_forwarding<'surface>(
    renderer: &mut WindowedRenderer<'surface>,
    chart_id: ChartId,
    source: &dyn ColumnSource,
) -> Result<()> {
    let guard = renderer.begin_load_demo(chart_id, source, source, source, source, |_| {
        Ok((crate::default::default_config(), Vec::new()))
    })?;
    drop(guard);
    Ok(())
}

#[test]
fn windowed_load_demo_forwarding_compile_contract() {
    // Runtime construction needs a live surface. Compiling this helper
    // verifies the forwarding call and guard lifetime without fabricating one.
    fn assert_compiles<T>(_value: T) {}
    assert_compiles(typecheck_windowed_load_demo_forwarding);
}

#[test]
fn renderer_begins_errorbar_extent_from_current_column_ids() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut renderer = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    renderer
        .add_column("fit-value", &col_f64(vec![0.0, 10.0]))
        .unwrap();
    renderer
        .add_column("fit-lower", &col_f64(vec![1.0, 5.0]))
        .unwrap();
    renderer
        .add_column("fit-upper", &col_f64(vec![1.0, 5.0]))
        .unwrap();
    pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();

    match renderer.begin_errorbar_extent("fit-value", "missing", "fit-upper") {
        Err(crate::gpu_errorbar::GpuErrorbarError::UnknownColumn { role, id }) => {
            assert_eq!(role, "lower error");
            assert_eq!(id, "missing");
        }
        _ => panic!("missing lower error column must be reported before submission"),
    }

    let ticket = renderer
        .begin_errorbar_extent("fit-value", "fit-lower", "fit-upper")
        .unwrap();
    let extent = pollster::block_on(ticket.resolve()).unwrap().unwrap();
    assert_eq!(extent.min, -1.0);
    assert_eq!(extent.max, 15.0);
    assert_eq!(extent.min_positive, Some(1.0));
}

#[test]
fn renderer_series_extent_uses_role_checked_provisional_mappings() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut renderer = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    renderer
        .add_column("series-fit-x", &col_f64(vec![1.0, 2.0]))
        .unwrap();
    renderer
        .add_column("series-fit-y", &col_f64(vec![10.0, 20.0]))
        .unwrap();
    pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();
    let ids = crate::gpu_errorbar::GpuSeriesExtentColumnIds {
        x: "series-fit-x",
        y: "series-fit-y",
        x_lower: None,
        x_upper: None,
        y_lower: None,
        y_upper: None,
    };

    match renderer.begin_series_extent(
        crate::gpu_errorbar::GpuSeriesExtentMode::Points,
        crate::gpu_errorbar::GpuSeriesExtentColumnIds {
            x: "missing",
            ..ids
        },
    ) {
        Err(crate::gpu_errorbar::GpuErrorbarError::UnknownColumn { role, id }) => {
            assert_eq!(role, "x");
            assert_eq!(id, "missing");
        }
        _ => panic!("missing x column must be reported before submission"),
    }
    match renderer.begin_series_extent(crate::gpu_errorbar::GpuSeriesExtentMode::PointsX, ids) {
        Err(crate::gpu_errorbar::GpuErrorbarError::MissingSeriesColumn { role, .. }) => {
            assert_eq!(role, "x lower error");
        }
        _ => panic!("active x errors require role-labelled lower and upper columns"),
    }

    let rollback_ticket = {
        let upsert = renderer
            .begin_upsert_hilo_column("series-fit-x", &col_f64(vec![100.0, 200.0]))
            .unwrap();
        upsert
            .begin_series_extent(crate::gpu_errorbar::GpuSeriesExtentMode::Points, ids)
            .unwrap()
    };
    let provisional = pollster::block_on(rollback_ticket.resolve())
        .unwrap()
        .unwrap();
    assert_eq!((provisional.x.min, provisional.x.max), (100.0, 200.0));
    let rolled_back = pollster::block_on(
        renderer
            .begin_series_extent(crate::gpu_errorbar::GpuSeriesExtentMode::Points, ids)
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!((rolled_back.x.min, rolled_back.x.max), (1.0, 2.0));

    let committed_ticket = {
        let upsert = renderer
            .begin_upsert_hilo_column("series-fit-x", &col_f64(vec![300.0, 400.0]))
            .unwrap();
        let ticket = upsert
            .begin_series_extent(crate::gpu_errorbar::GpuSeriesExtentMode::Points, ids)
            .unwrap();
        upsert.commit();
        ticket
    };
    let committed = pollster::block_on(committed_ticket.resolve())
        .unwrap()
        .unwrap();
    assert_eq!((committed.x.min, committed.x.max), (300.0, 400.0));
}

fn empty_chart_items<'a>(
    renderer: &Renderer,
    chart: &'a Chart,
    view: &'a ChartView,
) -> [ChartDrawItem<'a>; 1] {
    let _ = renderer;
    [ChartDrawItem {
        view,
        chart_config: chart.config(),
        series: &[],
    }]
}

#[test]
fn empty_prepare_compiles_only_axis_and_leaves_extent_engine_idle() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let chart = Chart::new(config);
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let items = empty_chart_items(&renderer, &chart, &view);
    renderer.prepare(&items).unwrap();

    assert!(renderer.line_pipeline().is_none());
    assert!(renderer.scatter_pipeline().is_none());
    assert!(renderer.errorbar_pipeline().is_none());
    assert!(renderer.bar_pipeline().is_none());
    assert!(renderer.pipelines.scatter_mapped.is_none());
    assert!(renderer.pipelines.errorbar_mapped.is_none());
    assert!(renderer.pipelines.pick_ring_mapped.is_none());
    assert!(!renderer.errorbar_extent_engine_ready());
    pollster::block_on(renderer.wait_submitted_work());
}

#[test]
fn full_prewarm_materializes_every_deferred_pipeline() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let mut events = Vec::new();
    pollster::block_on(renderer.prewarm_all_observed(&mut |event| events.push(event))).unwrap();

    assert!(renderer.pipelines.line.is_some());
    assert!(renderer.pipelines.scatter.is_some());
    assert!(renderer.pipelines.scatter_mapped.is_some());
    assert!(renderer.pipelines.pick_ring.is_some());
    assert!(renderer.pipelines.pick_ring_mapped.is_some());
    assert!(renderer.pipelines.errorbar.is_some());
    assert!(renderer.pipelines.errorbar_mapped.is_some());
    assert!(renderer.pipelines.bar.is_some());
    assert!(renderer.pipelines.bar_selection.is_some());
    assert!(renderer.pipelines.field.is_some());
    assert!(renderer.pipelines.field_selection.is_some());
    assert!(renderer.pipelines.contour.is_some());
    assert!(renderer.pipelines.contour_label.is_some());
    assert!(renderer.pipelines.styled.contains_key(&StyleKey::Sketch));
    assert!(renderer.pipelines.styled.contains_key(&StyleKey::Milkyway));
    assert!(
        renderer
            .pipelines
            .styled
            .contains_key(&StyleKey::Constellation)
    );
    assert!(renderer.contour_label_pipelines.is_some());
    assert!(renderer.arc_pipelines.is_some());
    assert!(renderer.errorbar_extent_engine_ready());
    assert!(events.iter().any(|event| {
        event.scope == "renderer.prewarm"
            && event.stage == "point picking compute"
            && event.phase == crate::init::InitPhase::Finished
    }));
}

// ── Histogram (W5) ─────────────────────────────────────────────────────

fn state_test_histogram(
    id: &str,
    x: &str,
    y: &str,
    orientation: crate::data_config::BarOrientation,
) -> SeriesConfig {
    SeriesConfig {
        series_id: id.to_string(),
        source_id: None,
        label: None,
        x_column: x.to_string(),
        y_column: y.to_string(),
        z_column: None,
        render_type: DataRenderType::Histogram {
            bar: crate::data_config::DataBarStyleConfig {
                fill_color: Color::new(0.2, 0.5, 0.8, 1.0),
                border_color: Color::BLACK,
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 1.0,
                width_ratio: 1.0,
                orientation,
                bar_style_overrides: None,
            },
        },
    }
}

/// A histogram compiles the bar pipeline and nothing else. Eager creation
/// would break the other isolation tests in this module, and would put a
/// pipeline on every chart that never draws a bar.
#[test]
fn histogram_prepare_compiles_only_the_bar_pipeline() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    // 4 bins: 5 edges, 4 counts.
    renderer
        .add_column("edges", &col_f64(vec![0.0, 1.0, 2.0, 3.0, 4.0]))
        .unwrap();
    renderer
        .add_column("counts", &col_f64(vec![3.0, 7.0, 5.0, 2.0]))
        .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let chart = Chart::new(config);
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let series_cfg = state_test_histogram(
        "hist",
        "edges",
        "counts",
        crate::data_config::BarOrientation::Vertical,
    );
    let style = renderer.create_style_for_series(&series_cfg).unwrap();
    let series = [Series {
        config: &series_cfg,
        style: &style,
    }];
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];
    renderer.prepare(&items).unwrap();

    assert!(renderer.bar_pipeline().is_some());
    assert!(renderer.line_pipeline().is_none());
    assert!(renderer.scatter_pipeline().is_none());
    assert!(renderer.errorbar_pipeline().is_none());
    assert!(renderer.pipelines.scatter_mapped.is_none());
    assert!(renderer.pipelines.errorbar_mapped.is_none());
    assert!(!renderer.errorbar_extent_engine_ready());
}

/// A bar is not a line: the line draw call issues `min(x, y) - 1`
/// instances, which for a correctly shaped `(n + 1, n)` histogram drops the
/// last bar. The bar count is `min(edges - 1, counts)`.
#[test]
fn bar_instance_count_keeps_the_last_bin() {
    use crate::data_render::bar_instance_count;

    // Well-formed: 11 edges bound 10 bins.
    assert_eq!(bar_instance_count(11, 10), 10);
    // What the line rule would have given for the same columns: one bar
    // short, the last bin silently gone.
    let line_rule = |a: usize, b: usize| a.min(b).saturating_sub(1);
    assert_eq!(line_rule(11, 10), 9);

    // Mismatches truncate to the smallest common extent, never error.
    assert_eq!(bar_instance_count(11, 9), 9);
    assert_eq!(bar_instance_count(5, 10), 4);
    assert_eq!(bar_instance_count(1, 10), 0);
    assert_eq!(bar_instance_count(0, 10), 0);
    assert_eq!(bar_instance_count(11, 0), 0);
}

/// The smallest-common-extent reporting window. A mismatch is drawn small and *reported* — the
/// one place a host can learn its declaration outran its data.
#[test]
fn series_draw_info_reports_the_truncation_it_drew() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("edges", &col_f64(vec![0.0, 1.0, 2.0, 3.0, 4.0]))
        .unwrap();
    renderer
        .add_column("counts4", &col_f64(vec![3.0, 7.0, 5.0, 2.0]))
        .unwrap();
    renderer
        .add_column("counts2", &col_f64(vec![3.0, 7.0]))
        .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let chart = renderer.register_chart(config, Vec::new()).unwrap();

    // 5 edges + 4 counts: exactly 4 bars, nothing left over.
    renderer
        .set_chart_series(
            chart,
            vec![state_test_histogram(
                "hist",
                "edges",
                "counts4",
                crate::data_config::BarOrientation::Vertical,
            )],
        )
        .unwrap();
    assert_eq!(
        renderer.series_draw_info(chart, "hist").unwrap(),
        SeriesDrawInfo {
            drawn_count: 4,
            cols: None,
            rows: None,
            truncated: false,
        }
    );

    // 5 edges + 2 counts: two bars, and the surplus edges are reported.
    renderer
        .set_chart_series(
            chart,
            vec![state_test_histogram(
                "hist",
                "edges",
                "counts2",
                crate::data_config::BarOrientation::Vertical,
            )],
        )
        .unwrap();
    let info = renderer.series_draw_info(chart, "hist").unwrap();
    assert_eq!(info.drawn_count, 2);
    assert!(info.truncated);

    // Orientation alone decides the roles. Same two columns in the same
    // slots, `Horizontal` instead of `Vertical`: now `y` (2 values) is the
    // edge column, bounding a single bin, against 5 values in `x`.
    renderer
        .set_chart_series(
            chart,
            vec![state_test_histogram(
                "hist",
                "edges",
                "counts2",
                crate::data_config::BarOrientation::Horizontal,
            )],
        )
        .unwrap();
    let info = renderer.series_draw_info(chart, "hist").unwrap();
    assert_eq!(
        info.drawn_count, 1,
        "horizontal reads y as edges: 2 edges bound 1 bin"
    );
    assert!(info.truncated);

    // The same window explains the existing primitives' `min` truncation.
    renderer
        .add_column("lx", &col_f64(vec![0.0, 1.0, 2.0]))
        .unwrap();
    renderer.add_column("ly", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer
        .set_chart_series(chart, vec![state_test_line("line", "lx", "ly")])
        .unwrap();
    let info = renderer.series_draw_info(chart, "line").unwrap();
    assert_eq!(info.drawn_count, 1, "two usable points make one segment");
    assert!(info.truncated);

    // And an unknown series is an error, not a zero.
    assert!(renderer.series_draw_info(chart, "nope").is_err());
}

#[test]
fn line_prepare_compiles_only_the_line_pipeline() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("lx", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("ly", &col_f64(vec![0.0, 1.0])).unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let chart = Chart::new(config);
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let series_cfg = state_test_line("line", "lx", "ly");
    let style = renderer.create_style_for_series(&series_cfg).unwrap();
    let series = [Series {
        config: &series_cfg,
        style: &style,
    }];
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];
    renderer.prepare(&items).unwrap();
    assert!(renderer.line_pipeline().is_some());
    assert!(renderer.scatter_pipeline().is_none());
    assert!(renderer.errorbar_pipeline().is_none());
    assert!(renderer.bar_pipeline().is_none());
    assert!(renderer.pipelines.scatter_mapped.is_none());
    assert!(renderer.pipelines.errorbar_mapped.is_none());
    assert!(renderer.pipelines.pick_ring_mapped.is_none());
    assert!(!renderer.errorbar_extent_engine_ready());
}

#[test]
fn scatter_prepare_compiles_only_the_scatter_pipeline() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("sx", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("sy", &col_f64(vec![0.0, 1.0])).unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let chart = Chart::new(config);
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let series_cfg = SeriesConfig {
        series_id: "scatter".into(),
        source_id: None,
        label: None,
        x_column: "sx".into(),
        y_column: "sy".into(),
        z_column: None,
        render_type: DataRenderType::Scatter {
            scatter: test_scatter_style(),
        },
    };
    let style = renderer.create_style_for_series(&series_cfg).unwrap();
    let series = [Series {
        config: &series_cfg,
        style: &style,
    }];
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];
    renderer.prepare(&items).unwrap();
    assert!(renderer.line_pipeline().is_none());
    assert!(renderer.scatter_pipeline().is_some());
    assert!(renderer.errorbar_pipeline().is_none());
    assert!(
        renderer.pipelines.scatter_mapped.is_none(),
        "unmapped scatter must not compile the mapped pipeline"
    );
    assert!(renderer.pipelines.errorbar_mapped.is_none());
    assert!(renderer.pipelines.pick_ring_mapped.is_none());
}

#[test]
fn errorbar_prepare_compiles_scatter_and_errorbar_pipelines() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("ex", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("ey", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("ee", &col_f64(vec![0.1, 0.1])).unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let chart = Chart::new(config);
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let series_cfg = SeriesConfig {
        series_id: "errorbar".into(),
        source_id: None,
        label: None,
        x_column: "ex".into(),
        y_column: "ey".into(),
        z_column: None,
        render_type: DataRenderType::ScatterErrorbarY {
            scatter: test_scatter_style(),
            err_y: ErrorRef::Symmetric {
                column: "ee".into(),
            },
            err_style: test_errorbar_style(),
        },
    };
    let style = renderer.create_style_for_series(&series_cfg).unwrap();
    let series = [Series {
        config: &series_cfg,
        style: &style,
    }];
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];
    renderer.prepare(&items).unwrap();
    assert!(renderer.line_pipeline().is_none());
    assert!(renderer.scatter_pipeline().is_some());
    assert!(renderer.errorbar_pipeline().is_some());
    assert!(
        renderer.pipelines.scatter_mapped.is_none(),
        "errorbar without a style map must not compile mapped pipelines"
    );
    assert!(renderer.pipelines.errorbar_mapped.is_none());
    assert!(renderer.pipelines.pick_ring_mapped.is_none());
    assert!(!renderer.errorbar_extent_engine_ready());
}

#[test]
fn failed_extent_prewarm_leaves_engine_unpublished() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    // Native warmup is a no-op and the public API intentionally has no
    // failure injection, so exercise the private post-warm publication
    // boundary with the same error returned by a failed wasm warmup.
    let error = crate::gpu_errorbar::GpuErrorbarError::AsyncCompileFailed(
        "injected prewarm failure".into(),
    );
    assert_eq!(
        renderer.finish_errorbar_extent_engine_preparation(Err(error.clone())),
        Err(error)
    );
    assert!(!renderer.errorbar_extent_engine_ready());
    assert!(matches!(
        renderer.begin_series_extent(
            crate::gpu_errorbar::GpuSeriesExtentMode::Points,
            crate::gpu_errorbar::GpuSeriesExtentColumnIds {
                x: "missing-x",
                y: "missing-y",
                x_lower: None,
                x_upper: None,
                y_lower: None,
                y_upper: None,
            },
        ),
        Err(crate::gpu_errorbar::GpuErrorbarError::EngineNotReady)
    ));
}

#[test]
fn point_constellation_scatterline_exports() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    r.add_column("pc_x", &col_f64(vec![0.0, 0.25, 0.5, 0.75, 1.0]))
        .unwrap();
    r.add_column("pc_y", &col_f64(vec![0.2, 0.75, 0.35, 0.9, 0.5]))
        .unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    config.legend.visible = false;
    config.draw_style =
        crate::config::DrawStyle::Constellation(crate::config::ConstellationOptions::default());
    let mut chart = Chart::new(config);
    chart.set_x_range(-0.05, 1.05);
    chart.set_y_range(0.0, 1.0);

    let series = [SeriesConfig {
        series_id: "pc".into(),
        source_id: None,
        label: None,
        x_column: "pc_x".into(),
        y_column: "pc_y".into(),
        z_column: None,
        render_type: DataRenderType::ScatterLine {
            scatter: DataScatterStyleConfig {
                point_color: Color::new(1.0, 1.0, 1.0, 1.0),
                point_shape: crate::data_config::ScatterShape::CircleFilled,
                point_size: 5.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            },
            line: DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::new(0.4, 0.7, 1.0, 1.0),
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let lit = img.rgba.chunks_exact(4).filter(|p| p[3] > 0).count();
    assert!(
        lit > 100,
        "point constellation export produced too little ink: {lit}"
    );
}

#[test]
fn all_scatter_shapes_export_visible_markers() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();

    let shapes = [
        crate::data_config::ScatterShape::Circle,
        crate::data_config::ScatterShape::Square,
        crate::data_config::ScatterShape::Triangle,
        crate::data_config::ScatterShape::Diamond,
        crate::data_config::ScatterShape::Cross,
        crate::data_config::ScatterShape::CircleFilled,
        crate::data_config::ScatterShape::SquareFilled,
        crate::data_config::ScatterShape::TriangleFilled,
        crate::data_config::ScatterShape::DiamondFilled,
        crate::data_config::ScatterShape::TriangleDown,
        crate::data_config::ScatterShape::TriangleLeft,
        crate::data_config::ScatterShape::TriangleRight,
        crate::data_config::ScatterShape::Plus,
        crate::data_config::ScatterShape::Pentagon,
        crate::data_config::ScatterShape::Hexagon,
        crate::data_config::ScatterShape::Octagon,
        crate::data_config::ScatterShape::Star,
        crate::data_config::ScatterShape::TriangleDownFilled,
        crate::data_config::ScatterShape::TriangleLeftFilled,
        crate::data_config::ScatterShape::TriangleRightFilled,
        crate::data_config::ScatterShape::PlusFilled,
        crate::data_config::ScatterShape::CrossFilled,
        crate::data_config::ScatterShape::PentagonFilled,
        crate::data_config::ScatterShape::HexagonFilled,
        crate::data_config::ScatterShape::OctagonFilled,
        crate::data_config::ScatterShape::StarFilled,
    ];

    let mut series = Vec::new();
    for (i, shape) in shapes.iter().enumerate() {
        let x = (i % 7) as f64;
        let y = (i / 7) as f64;
        let x_id = format!("shape_x_{i}");
        let y_id = format!("shape_y_{i}");
        r.add_column(&x_id, &col_f64(vec![x])).unwrap();
        r.add_column(&y_id, &col_f64(vec![y])).unwrap();
        series.push(SeriesConfig {
            series_id: format!("shape_{i}"),
            source_id: None,
            label: None,
            x_column: x_id,
            y_column: y_id,
            z_column: None,
            render_type: DataRenderType::Scatter {
                scatter: DataScatterStyleConfig {
                    point_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    point_shape: shape.clone(),
                    point_size: 8.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
            },
        });
    }

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 700,
        height: 400,
    });
    config.legend.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(-0.5, 6.5);
    chart.set_y_range(-0.5, 3.8);

    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let red_ink = img
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0] > 150 && p[1] < 80 && p[2] < 80 && p[3] > 100)
        .count();
    assert!(
        red_ink > 600,
        "scatter shape export produced too little red ink: {red_ink}"
    );
}

#[test]
fn xy_errorbar_does_not_require_zero_column() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    r.add_column("x", &col_f64(vec![1.0, 2.0, 3.0])).unwrap();
    r.add_column("y", &col_f64(vec![1.0, 3.0, 2.0])).unwrap();
    r.add_column("ex", &col_f64(vec![0.1, 0.2, 0.1])).unwrap();
    r.add_column("ey", &col_f64(vec![0.3, 0.1, 0.2])).unwrap();

    let chart = basic_errorbar_chart();
    let series = [SeriesConfig {
        series_id: "xy".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::ScatterErrorbarXY {
            scatter: test_scatter_style(),
            err_x: ErrorRef::Symmetric {
                column: "ex".into(),
            },
            err_y: ErrorRef::Symmetric {
                column: "ey".into(),
            },
            err_style: test_errorbar_style(),
        },
    }];

    r.export_panel_rgba(&chart, &series, 1.0).unwrap();
}

/// A zero-size marker draws a dot and picks (`pick_draw_agreement`), so a
/// picked one takes the ring like any other marker.
#[test]
fn picked_overlay_rings_a_zero_size_scatter_errorbar_anchor() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    r.add_column("x", &col_f64(vec![2.0])).unwrap();
    r.add_column("y", &col_f64(vec![2.5])).unwrap();
    r.add_column("ey", &col_f64(vec![1.0])).unwrap();

    let mut chart = basic_errorbar_chart();
    chart.config_mut().picked_points = Some(crate::config::PickedPointsConfig {
        visible: true,
        refs: vec![crate::config::PickedPointRef {
            source_id: None,
            series_id: "err".into(),
            point_index: 0,
        }],
        ring_color: Color::new(0.0, 1.0, 0.0, 1.0),
        ring_width_px: 3.0,
        radius_extra_px: 8.0,
    });
    let mut scatter = test_scatter_style();
    scatter.point_size = 0.0;
    let series = [SeriesConfig {
        series_id: "err".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::ScatterErrorbarY {
            scatter,
            err_y: ErrorRef::Symmetric {
                column: "ey".into(),
            },
            err_style: test_errorbar_style(),
        },
    }];

    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let green_ring_ink = img
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0] < 80 && p[1] > 160 && p[2] < 80 && p[3] > 80)
        .count();

    assert!(
        green_ring_ink > 20,
        "a picked zero-size marker over an errorbar takes the ring: {green_ring_ink}"
    );
}

#[test]
fn errorbar_style_index_colors_points_independently() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    r.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
    r.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
    r.add_column("ey", &col_f64(vec![1.0, 1.0])).unwrap();
    r.add_column("err_style", &col_f64(vec![0.0, 1.0])).unwrap();

    let chart = basic_errorbar_chart();
    let mut scatter = test_scatter_style();
    scatter.point_size = 0.0;
    let series = [SeriesConfig {
        series_id: "mapped-errorbar".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::ScatterErrorbarY {
            scatter,
            err_y: ErrorRef::Symmetric {
                column: "ey".into(),
            },
            err_style: DataErrorBarStyleConfig {
                error_bar_color: Color::BLACK,
                error_bar_width: 2.0,
                error_bar_cap_size: 7.0,
                cap_width: 2.0,
                error_bar_style_table: Some(vec![
                    DataErrorBarPointStyleConfig {
                        error_bar_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
                        ..Default::default()
                    },
                    DataErrorBarPointStyleConfig {
                        error_bar_color: Some(Color::new(0.0, 0.0, 1.0, 1.0)),
                        ..Default::default()
                    },
                ]),
                error_bar_style_index_column: Some("err_style".into()),
                error_bar_style_overrides: None,
            },
        },
    }];

    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let green = img
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0] < 80 && p[1] > 150 && p[2] < 80 && p[3] > 100)
        .count();
    let blue = img
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0] < 80 && p[1] < 80 && p[2] > 150 && p[3] > 100)
        .count();

    assert!(green > 20, "mapped errorbar style produced no green ink");
    assert!(blue > 20, "mapped errorbar style produced no blue ink");
}

fn count_rgba_ink(rgba: &[u8], pred: impl Fn([u8; 4]) -> bool) -> usize {
    rgba.chunks_exact(4)
        .filter(|p| pred([p[0], p[1], p[2], p[3]]))
        .count()
}

fn is_saturated_red(p: [u8; 4]) -> bool {
    p[0] > 150 && p[1] < 80 && p[2] < 80 && p[3] > 100
}

fn is_saturated_green(p: [u8; 4]) -> bool {
    p[0] < 80 && p[1] > 150 && p[2] < 80 && p[3] > 100
}

fn is_saturated_blue(p: [u8; 4]) -> bool {
    p[0] < 80 && p[1] < 80 && p[2] > 150 && p[3] > 100
}

struct RecordedPreparedRgba {
    command_buffer: wgpu::CommandBuffer,
    readback: wgpu::Buffer,
    unpadded: u32,
    padded: u32,
    height: u32,
}

fn record_prepared_rgba(
    renderer: &Renderer,
    prepared: &PreparedFrame,
    w: u32,
    h: u32,
) -> RecordedPreparedRgba {
    let device = renderer.device.as_ref();
    let target_desc = wgpu::TextureDescriptor {
        label: Some("mapped style regression target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    };
    let tex = device.create_texture(&target_desc);
    let tv = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let unpadded = w * 4;
    let padded = unpadded.div_ceil(256) * 256;
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &tv,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        renderer
            .paint_prepared(&mut pass, (w, h), prepared)
            .expect("paint_prepared after mapped-style prepare");
    }
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        target_desc.size,
    );
    RecordedPreparedRgba {
        command_buffer: enc.finish(),
        readback,
        unpadded,
        padded,
        height: h,
    }
}

fn submit_recorded_rgba(renderer: &Renderer, recorded: RecordedPreparedRgba) -> Vec<u8> {
    renderer
        .queue
        .submit(std::iter::once(recorded.command_buffer));
    let slice = recorded.readback.slice(..);
    let (mapped_tx, mapped_rx) = std::sync::mpsc::sync_channel(1);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = mapped_tx.send(result);
    });
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("offscreen GPU work completes before readback");
    // On the shared test device the callback can run on another test's
    // polling thread, after this poll returns.
    mapped_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("offscreen mapping callback runs after GPU completion")
        .expect("offscreen readback mapping succeeds");
    let mapped = slice
        .get_mapped_range()
        .expect("offscreen readback is mapped");
    if recorded.padded == recorded.unpadded {
        mapped.to_vec()
    } else {
        let mut out = Vec::with_capacity((recorded.unpadded * recorded.height) as usize);
        for row in 0..recorded.height {
            let start = (row * recorded.padded) as usize;
            out.extend_from_slice(&mapped[start..start + recorded.unpadded as usize]);
        }
        out
    }
}

fn paint_prepared_rgba(renderer: &Renderer, prepared: &PreparedFrame, w: u32, h: u32) -> Vec<u8> {
    submit_recorded_rgba(renderer, record_prepared_rgba(renderer, prepared, w, h))
}

struct MappedStyleRender {
    styles: Vec<ChartStyle>,
    screen: Vec<u8>,
    export: Vec<u8>,
    scatter_mapped: bool,
    errorbar_mapped: bool,
    pick_ring_mapped: bool,
}

/// On-screen path = `prepare` + `paint_prepared` (same as `WindowedRenderer::draw`).
/// PNG/export path = `export_panel_rgba` (own pipeline cache, same ensure).
fn render_mapped_style_case(
    renderer: &mut Renderer,
    chart: &Chart,
    series_cfgs: &[SeriesConfig],
) -> MappedStyleRender {
    let styles: Vec<ChartStyle> = series_cfgs
        .iter()
        .map(|cfg| renderer.create_style_for_series(cfg).unwrap())
        .collect();
    let w = chart.config().chart_area.0.width;
    let h = chart.config().chart_area.0.height;
    let (screen, scatter_mapped, errorbar_mapped, pick_ring_mapped) = {
        let view = renderer
            .create_chart_view(chart, chart.config().chart_area.0)
            .unwrap();
        let series: Vec<Series<'_>> = series_cfgs
            .iter()
            .zip(styles.iter())
            .map(|(cfg, style)| Series { config: cfg, style })
            .collect();
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        let prepared = renderer.prepare(&items).unwrap();
        let pipeline_state = (
            renderer.pipelines.scatter_mapped.is_some(),
            renderer.pipelines.errorbar_mapped.is_some(),
            renderer.pipelines.pick_ring_mapped.is_some(),
        );
        (
            paint_prepared_rgba(renderer, &prepared, w, h),
            pipeline_state.0,
            pipeline_state.1,
            pipeline_state.2,
        )
    };
    let export = renderer
        .export_panel_rgba(chart, series_cfgs, 1.0)
        .expect("export must not panic when a style map exists without an index column");
    MappedStyleRender {
        styles,
        screen,
        export: export.rgba,
        scatter_mapped,
        errorbar_mapped,
        pick_ring_mapped,
    }
}

fn assert_ink_on_screen_and_export(
    rendered: &MappedStyleRender,
    label: &str,
    pred: fn([u8; 4]) -> bool,
    min: usize,
) {
    let screen = count_rgba_ink(&rendered.screen, pred);
    let export = count_rgba_ink(&rendered.export, pred);
    assert!(
        screen >= min,
        "{label} missing on-screen ({screen} px, need {min})"
    );
    assert!(
        export >= min,
        "{label} missing on PNG/export ({export} px, need {min})"
    );
}

fn override_only_scatter_series(id: &str, x: &str, y: &str) -> SeriesConfig {
    let mut scatter = test_scatter_style();
    scatter.point_color = Color::new(0.0, 0.0, 0.0, 1.0);
    scatter.point_size = 6.0;
    scatter.point_style_table = None;
    scatter.point_style_index_column = None;
    scatter.point_style_overrides = Some(vec![DataScatterPointStyleOverride {
        index: 0,
        style: DataScatterPointStyleConfig {
            point_color: Some(Color::new(1.0, 0.0, 0.0, 1.0)),
            point_size: Some(16.0),
            ..Default::default()
        },
    }]);
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Scatter { scatter },
    }
}

/// `point_style_overrides` without `point_style_index_column` still builds
/// a style map (`has_index = false`). Prepare must compile the mapped
/// scatter pipeline that draw/export both require.
#[test]
fn override_only_scatter_paints_on_screen_and_export() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();

    let chart = basic_errorbar_chart();
    let series = [override_only_scatter_series("ov-scatter", "x", "y")];
    let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

    let map = rendered.styles[0]
        .scatter_map
        .as_ref()
        .expect("overrides-only scatter must build a style map");
    assert!(
        !map.has_index,
        "overrides-only scatter must not set has_index"
    );
    assert!(
        rendered.styles[0].errorbar_map.is_none(),
        "scatter-only series must not build an errorbar map"
    );
    assert!(
        rendered.scatter_mapped,
        "prepare must compile mapped scatter when scatter_map is Some"
    );
    assert!(!rendered.errorbar_mapped);
    assert!(!rendered.pick_ring_mapped);

    assert_ink_on_screen_and_export(&rendered, "override-only red scatter", is_saturated_red, 20);
}

/// Same override-only map plus a selected-point ring. Draw selects the
/// mapped pick-ring pipeline whenever `scatter_map` is Some.
#[test]
fn override_only_scatter_picked_ring_paints_on_screen_and_export() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();

    let mut chart = basic_errorbar_chart();
    chart.config_mut().picked_points = Some(crate::config::PickedPointsConfig {
        visible: true,
        refs: vec![crate::config::PickedPointRef {
            source_id: None,
            series_id: "ov-scatter".into(),
            point_index: 0,
        }],
        ring_color: Color::new(0.0, 1.0, 0.0, 1.0),
        ring_width_px: 3.0,
        radius_extra_px: 8.0,
    });
    let series = [override_only_scatter_series("ov-scatter", "x", "y")];
    let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

    assert!(
        rendered.styles[0]
            .scatter_map
            .as_ref()
            .is_some_and(|map| !map.has_index)
    );
    assert!(
        rendered.scatter_mapped,
        "picked override-only scatter still needs mapped scatter"
    );
    assert!(
        rendered.pick_ring_mapped,
        "picked override-only scatter must compile mapped pick ring"
    );

    assert_ink_on_screen_and_export(&rendered, "override-only red scatter", is_saturated_red, 20);
    assert_ink_on_screen_and_export(&rendered, "mapped pick ring", is_saturated_green, 10);
}

// ── Continuous point value maps (#24) ───────────────────────────────────

fn is_saturated_purple(p: [u8; 4]) -> bool {
    p[0] > 90 && p[1] < 60 && p[2] > 90 && p[3] > 100
}

fn two_stop_colorbar(scale: AxisScale, min: f64, max: f64) -> crate::config::ColorBarOptions {
    let mut bar = crate::default::default_colorbar_options();
    // Hidden, so the plot keeps the basic chart's geometry; the options
    // still own the z range and the ramp.
    bar.visible = false;
    bar.colormap = crate::colormap::ColorMap::Custom {
        stops: vec![
            Color::new(1.0, 0.0, 0.0, 1.0),
            Color::new(0.0, 0.0, 1.0, 1.0),
        ],
    };
    bar.nan_color = Color::new(0.0, 1.0, 0.0, 1.0);
    bar.axis.scale = scale;
    bar.axis.min = min;
    bar.axis.max = max;
    bar
}

fn value_mapped_scatter_series(
    id: &str,
    color_map: Option<&str>,
    size_map: Option<(&str, SizeScale)>,
) -> SeriesConfig {
    let mut scatter = test_scatter_style();
    scatter.point_color = Color::new(1.0, 0.0, 0.0, 1.0);
    scatter.point_size = 8.0;
    scatter.point_color_map = color_map.map(|column| crate::data_config::PointColorMap {
        column: column.into(),
    });
    scatter.point_size_map = size_map.map(|(column, scale)| crate::data_config::PointSizeMap {
        column: column.into(),
        domain_min: 0.0,
        domain_max: 10.0,
        min_px: 2.0,
        max_px: 12.0,
        scale,
    });
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Scatter { scatter },
    }
}

/// Colour comes from the colourbar ramp at the point's value: the ends
/// take the end stops, the middle the interpolated colour, and a
/// non-finite value the colourbar's `nan_color`. On screen and on export.
#[test]
fn color_mapped_scatter_paints_the_colorbar_ramp_and_nan_color() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("x", &col_f64(vec![0.5, 1.5, 2.5, 3.5]))
        .unwrap();
    renderer
        .add_column("y", &col_f64(vec![2.5, 2.5, 2.5, 2.5]))
        .unwrap();
    renderer
        .add_column("c", &col_f64(vec![0.0, 10.0, f64::NAN, 5.0]))
        .unwrap();

    let mut chart = basic_errorbar_chart();
    chart.config_mut().colorbar = Some(two_stop_colorbar(AxisScale::Linear, 0.0, 10.0));
    let series = [value_mapped_scatter_series("cmap", Some("c"), None)];
    let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

    assert!(
        rendered.styles[0].scatter_map.is_some(),
        "a colour map must select the mapped scatter path"
    );
    assert!(rendered.scatter_mapped);
    assert_ink_on_screen_and_export(&rendered, "z = min (red stop)", is_saturated_red, 20);
    assert_ink_on_screen_and_export(&rendered, "z = max (blue stop)", is_saturated_blue, 20);
    assert_ink_on_screen_and_export(&rendered, "z = NaN (nan_color)", is_saturated_green, 20);
    assert_ink_on_screen_and_export(&rendered, "z = mid (ramp)", is_saturated_purple, 20);
}

/// A logarithmic colourbar cannot place a non-positive value: it draws in
/// `nan_color`, as a heatmap cell would, while 10 on a 1..100 bar is the
/// ramp's middle.
#[test]
fn log_colorbar_maps_nonpositive_values_to_nan_color() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("x", &col_f64(vec![0.7, 2.0, 3.3]))
        .unwrap();
    renderer
        .add_column("y", &col_f64(vec![2.5, 2.5, 2.5]))
        .unwrap();
    renderer
        .add_column("c", &col_f64(vec![0.0, -3.0, 10.0]))
        .unwrap();

    let mut chart = basic_errorbar_chart();
    chart.config_mut().colorbar = Some(two_stop_colorbar(AxisScale::Logarithmic, 1.0, 100.0));
    let series = [value_mapped_scatter_series("cmap-log", Some("c"), None)];
    let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

    assert_ink_on_screen_and_export(&rendered, "non-positive z", is_saturated_green, 40);
    assert_ink_on_screen_and_export(&rendered, "log mid", is_saturated_purple, 20);
    assert_eq!(count_rgba_ink(&rendered.screen, is_saturated_red), 0);
    assert_eq!(count_rgba_ink(&rendered.screen, is_saturated_blue), 0);
}

/// One point at `value` on its own renderer, so each case uploads fresh
/// columns under the same ids.
fn single_point_ink(value: f64, scale: SizeScale) -> Option<usize> {
    let mut renderer = state_test_renderer()?;
    renderer.add_column("x", &col_f64(vec![2.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5])).unwrap();
    renderer.add_column("s", &col_f64(vec![value])).unwrap();
    let chart = basic_errorbar_chart();
    let series = [value_mapped_scatter_series(
        "smap",
        None,
        Some(("s", scale)),
    )];
    let rendered = render_mapped_style_case(&mut renderer, &chart, &series);
    let screen = count_rgba_ink(&rendered.screen, is_saturated_red);
    let export = count_rgba_ink(&rendered.export, is_saturated_red);
    // Export multisamples, so the exact counts differ; visibility must not.
    assert_eq!(
        screen == 0,
        export == 0,
        "screen ({screen} px) and export ({export} px) disagree for value {value}"
    );
    Some(screen)
}

/// The radius follows the size map: the domain's ends map to `min_px` and
/// `max_px`, area scaling grows faster than radius scaling in the middle,
/// a value past the domain clamps, and a non-finite value hides the point.
#[test]
fn size_mapped_scatter_scales_markers_by_value_and_hides_non_finite_values() {
    let Some(smallest) = single_point_ink(0.0, SizeScale::Radius) else {
        return;
    };
    let largest = single_point_ink(10.0, SizeScale::Radius).unwrap();
    let clamped = single_point_ink(1.0e9, SizeScale::Radius).unwrap();
    let mid_radius = single_point_ink(5.0, SizeScale::Radius).unwrap();
    let mid_area = single_point_ink(5.0, SizeScale::Area).unwrap();
    let hidden = single_point_ink(f64::NAN, SizeScale::Radius).unwrap();

    // 2 px vs 12 px radii: a 36x area ratio before antialiasing.
    assert!(smallest > 0, "min_px marker missing ({smallest} px)");
    assert!(
        largest > smallest * 8,
        "max_px marker ({largest} px) is not much larger than min_px ({smallest} px)"
    );
    assert_eq!(
        clamped, largest,
        "a value past the domain must clamp to max_px"
    );
    // Radius scaling: 7 px. Area scaling: sqrt((4 + 144) / 2) ≈ 8.6 px.
    assert!(
        mid_area > mid_radius * 13 / 10,
        "area scaling ({mid_area} px) must outgrow radius scaling ({mid_radius} px)"
    );
    assert_eq!(hidden, 0, "a non-finite size value must hide the point");
}

/// Export scales the mapped pixel range with the DPI multiplier like
/// `point_size`: a 2x export draws about four times the area.
#[test]
fn size_mapped_scatter_scales_with_the_export_multiplier() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![2.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5])).unwrap();
    renderer.add_column("s", &col_f64(vec![10.0])).unwrap();
    let chart = basic_errorbar_chart();
    let series = [value_mapped_scatter_series(
        "smap",
        None,
        Some(("s", SizeScale::Radius)),
    )];
    let one = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let two = renderer.export_panel_rgba(&chart, &series, 2.0).unwrap();
    let one_px = count_rgba_ink(&one.rgba, is_saturated_red);
    let two_px = count_rgba_ink(&two.rgba, is_saturated_red);
    assert!(one_px > 0);
    assert!(
        two_px > one_px * 3 && two_px < one_px * 5,
        "2x export drew {two_px} px against {one_px} px at 1x"
    );
    let scratch = renderer
        .value_map_cache
        .get("smap")
        .expect("value map scratch");
    assert_eq!(scratch.signature.params.size_px, [4.0, 24.0]);
}

/// Per channel, a sparse override beats the value map: an override colour
/// paints over the ramp, and an override size makes a hidden point
/// visible again.
#[test]
fn overrides_take_precedence_over_value_maps() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
    renderer
        .add_column("c", &col_f64(vec![10.0, 10.0]))
        .unwrap();
    renderer
        .add_column("s", &col_f64(vec![f64::NAN, f64::NAN]))
        .unwrap();

    let mut chart = basic_errorbar_chart();
    chart.config_mut().colorbar = Some(two_stop_colorbar(AxisScale::Linear, 0.0, 10.0));
    let mut series = value_mapped_scatter_series("ov", Some("c"), Some(("s", SizeScale::Radius)));
    let DataRenderType::Scatter { scatter } = &mut series.render_type else {
        unreachable!()
    };
    scatter.point_style_overrides = Some(vec![DataScatterPointStyleOverride {
        index: 0,
        style: DataScatterPointStyleConfig {
            point_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
            point_shape: None,
            point_size: Some(10.0),
        },
    }]);
    let rendered = render_mapped_style_case(&mut renderer, &chart, &[series]);

    // Point 0: overridden colour and size. Point 1: hidden by its size.
    assert_ink_on_screen_and_export(&rendered, "override colour", is_saturated_green, 40);
    assert_eq!(
        count_rgba_ink(&rendered.screen, is_saturated_blue),
        0,
        "the ramp colour must not paint under an override"
    );
}

/// The picked ring follows the mapped radius and is absent on a point the
/// size map hides.
#[test]
fn picked_ring_follows_the_size_map() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
    renderer
        .add_column("s", &col_f64(vec![10.0, f64::NAN]))
        .unwrap();
    let series = [value_mapped_scatter_series(
        "ring",
        None,
        Some(("s", SizeScale::Radius)),
    )];
    let picked = |point_index: usize| crate::config::PickedPointsConfig {
        visible: true,
        refs: vec![crate::config::PickedPointRef {
            source_id: None,
            series_id: "ring".into(),
            point_index,
        }],
        ring_color: Color::new(0.0, 1.0, 0.0, 1.0),
        ring_width_px: 3.0,
        radius_extra_px: 4.0,
    };

    let mut chart = basic_errorbar_chart();
    chart.config_mut().picked_points = Some(picked(0));
    let visible = render_mapped_style_case(&mut renderer, &chart, &series);
    assert!(visible.pick_ring_mapped);
    assert_ink_on_screen_and_export(&visible, "ring on the mapped point", is_saturated_green, 40);

    chart.config_mut().picked_points = Some(picked(1));
    let hidden = render_mapped_style_case(&mut renderer, &chart, &series);
    assert_eq!(
        count_rgba_ink(&hidden.screen, is_saturated_green),
        0,
        "a hidden point draws no ring"
    );
}

/// Registration rejects a colour map without a colourbar, an invalid size
/// domain and an unregistered value column.
#[test]
fn value_maps_are_validated_at_registration() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
    renderer.add_column("c", &col_f64(vec![1.0, 2.0])).unwrap();
    let config = basic_errorbar_chart().config().clone();

    let no_colorbar = vec![value_mapped_scatter_series("v", Some("c"), None)];
    assert!(matches!(
        renderer.register_chart(config.clone(), no_colorbar),
        Err(FiggyError::InvalidSeriesConfig { reason, .. }) if reason.contains("colorbar")
    ));

    let mut bad_domain = value_mapped_scatter_series("v", None, Some(("c", SizeScale::Area)));
    let DataRenderType::Scatter { scatter } = &mut bad_domain.render_type else {
        unreachable!()
    };
    scatter.point_size_map.as_mut().unwrap().domain_max = 0.0;
    assert!(matches!(
        renderer.register_chart(config.clone(), vec![bad_domain]),
        Err(FiggyError::InvalidSeriesConfig { reason, .. })
            if reason == crate::data_config::PointSizeMap::INVALID_DOMAIN_REASON
    ));

    let missing = vec![value_mapped_scatter_series(
        "v",
        None,
        Some(("nope", SizeScale::Area)),
    )];
    assert!(matches!(
        renderer.register_chart(config, missing),
        Err(FiggyError::UnknownColumn { id }) if id == "nope"
    ));
}

/// A value column shorter than the series is a prepare error, never a
/// read past the column.
#[test]
fn value_map_columns_must_cover_every_point() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("x", &col_f64(vec![1.0, 2.0, 3.0]))
        .unwrap();
    renderer
        .add_column("y", &col_f64(vec![2.5, 2.5, 2.5]))
        .unwrap();
    renderer.add_column("s", &col_f64(vec![1.0])).unwrap();
    let chart = basic_errorbar_chart();
    let series = [value_mapped_scatter_series(
        "short",
        None,
        Some(("s", SizeScale::Area)),
    )];
    let style = renderer.create_style_for_series(&series[0]).unwrap();
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let drawn = [Series {
        config: &series[0],
        style: &style,
    }];
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &drawn,
    }];
    assert!(matches!(
        renderer.prepare(&items),
        Err(FiggyError::InvalidSeriesConfig { reason, .. }) if reason.contains("size map")
    ));
}

/// Streaming is not extended to the value maps: a streamed series with one
/// is rejected with an explicit reason, like a streamed line fill.
#[test]
fn streamed_series_reject_value_maps() {
    let config = basic_errorbar_chart().config().clone();
    let colour = value_mapped_scatter_series("stream", Some("c"), None);
    let size = value_mapped_scatter_series("stream", None, Some(("s", SizeScale::Area)));
    for series in [colour, size] {
        assert!(matches!(
            PrepareContext::validate_stream_series(&config, &series),
            Err(FiggyError::InvalidSeriesConfig { reason, .. })
                if reason.contains("streamed sources are not supported")
        ));
    }
    let plain = test_scatter_style();
    let plain = SeriesConfig {
        series_id: "plain".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Scatter { scatter: plain },
    };
    assert!(PrepareContext::validate_stream_series(&config, &plain).is_ok());
}

/// The group-3 state is rebuilt only when its inputs change: two prepares
/// of the same chart share one entry, and editing the colourbar's range
/// (SSOT config) replaces it.
#[test]
fn value_map_scratch_is_reused_until_its_inputs_change() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
    renderer.add_column("c", &col_f64(vec![1.0, 2.0])).unwrap();
    let mut chart = basic_errorbar_chart();
    chart.config_mut().colorbar = Some(two_stop_colorbar(AxisScale::Linear, 0.0, 10.0));
    let series = [value_mapped_scatter_series("reuse", Some("c"), None)];

    let first = render_mapped_style_case(&mut renderer, &chart, &series);
    assert_eq!(renderer.value_map_cache.len(), 1);
    let before = renderer.value_map_cache["reuse"].signature.params;
    let second = render_mapped_style_case(&mut renderer, &chart, &series);
    assert_eq!(renderer.value_map_cache.len(), 1);
    assert_eq!(renderer.value_map_cache["reuse"].signature.params, before);
    assert_eq!(
        first.screen, second.screen,
        "an unchanged chart repaints identically"
    );

    chart.config_mut().colorbar.as_mut().unwrap().axis.max = 2.0;
    let third = render_mapped_style_case(&mut renderer, &chart, &series);
    let after = renderer.value_map_cache["reuse"].signature.params;
    assert_ne!(
        after.z_max, before.z_max,
        "a colourbar edit rebuilds the params"
    );
    assert_ne!(
        third.screen, first.screen,
        "the narrower range changes the mapped colours"
    );
}

// ── Vector fields (#25) ─────────────────────────────────────────────────

fn vector_points_series(id: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Vector {
            components: crate::data_config::VectorComponents::Points {
                u: "u".into(),
                v: "v".into(),
            },
            style: crate::data_config::VectorStyle {
                color: Color::new(1.0, 0.0, 0.0, 1.0),
                width_px: 4.0,
                scale: crate::data_config::VectorScale::Pixels { px_per_unit: 1.0 },
                pivot: crate::data_config::VectorPivot::Tail,
                head_length_px: 8.0,
                head_width_px: 8.0,
                min_length_px: 0.0,
                color_map: None,
            },
        },
    }
}

/// Streaming is not extended to vector series: a streamed one is rejected
/// with an explicit reason, like a streamed value map.
#[test]
fn streamed_series_reject_vectors() {
    let config = basic_errorbar_chart().config().clone();
    assert!(matches!(
        PrepareContext::validate_stream_series(&config, &vector_points_series("stream")),
        Err(FiggyError::InvalidSeriesConfig { reason, .. })
            if reason.contains("streamed sources are not supported")
    ));
}

/// Streaming is not extended to category bars: a streamed category bar
/// series is rejected with an explicit reason, on a chart without a
/// categorical axis (whose own rejection would otherwise come first) too.
#[test]
fn streamed_series_reject_category_bars() {
    let config = basic_errorbar_chart().config().clone();
    let series = SeriesConfig {
        series_id: "stream".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::CategoryBars {
            color: Color::BLACK,
            style: Default::default(),
            labels: Default::default(),
            label_format: Default::default(),
            overrides: None,
        },
    };
    assert!(matches!(
        PrepareContext::validate_stream_series(&config, &series),
        Err(FiggyError::InvalidSeriesConfig { reason, .. })
            if reason.contains("category bar") && reason.contains("streamed sources are not supported")
    ));
}

/// Streaming is not extended to box plots: a streamed box plot series is
/// rejected with an explicit reason (CLAUDE.md rule 8), and the draw-phase
/// backstop says the same.
#[test]
fn streamed_series_reject_box_plots() {
    let config = basic_errorbar_chart().config().clone();
    let series = SeriesConfig {
        series_id: "stream".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "record".into(),
        z_column: None,
        render_type: DataRenderType::BoxPlot {
            color: Color::BLACK,
            style: Default::default(),
            labels: Default::default(),
            outliers: None,
            overrides: None,
            violin: None,
            points: None,
        },
    };
    assert!(matches!(
        PrepareContext::validate_stream_series(&config, &series),
        Err(FiggyError::InvalidSeriesConfig { reason, .. })
            if reason.contains("box plot") && reason.contains("streamed sources are not supported")
    ));
}

/// Streaming is not extended to range bars: a streamed range bar series is
/// rejected with an explicit reason (CLAUDE.md rule 8) before a cursor
/// exists.
#[test]
fn streamed_series_reject_range_bars() {
    let config = basic_errorbar_chart().config().clone();
    let series = SeriesConfig {
        series_id: "stream".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::RangeBars {
            color: Color::BLACK,
            style: Default::default(),
            start: "start".into(),
            whiskers: None,
            color_by: Default::default(),
            progress: None,
            connector: None,
            slot_width: None,
            labels: Default::default(),
        },
    };
    assert!(matches!(
        PrepareContext::validate_stream_series(&config, &series),
        Err(FiggyError::InvalidSeriesConfig { reason, .. })
            if reason.contains("range bar") && reason.contains("streamed sources are not supported")
    ));
}

/// Streaming is not extended to ternary series (CLAUDE.md rule 8): a
/// streamed one is rejected with an explicit reason.
/// A hexagonal grid reads its lattice from resident coordinate columns
/// whose uniform steps the pool measured; a streamed one is rejected (rule
/// 8, #134).
#[test]
fn streamed_series_reject_hexagonal_grids() {
    let config = basic_errorbar_chart().config().clone();
    let series = SeriesConfig {
        series_id: "stream".into(),
        source_id: None,
        z_column: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Heatmap {
            matrix: crate::data_config::MatrixRef {
                columns: vec!["z0".into(), "z1".into()],
                orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                grid_layout: crate::data_config::GridLayout::Hexagonal,
            },
            fill: crate::data_config::FieldFillConfig {
                mode: crate::data_config::FillMode::Continuous,
                shading: crate::data_config::Shading::Flat,
                opacity: 1.0,
            },
            surface: None,
        },
    };
    assert!(matches!(
        PrepareContext::validate_stream_series(&config, &series),
        Err(FiggyError::InvalidSeriesConfig { reason, .. })
            if reason.contains("hexagonal") && reason.contains("streamed sources are not supported")
    ));
}

#[test]
fn streamed_series_reject_ternary_series() {
    let config = basic_errorbar_chart().config().clone();
    let series = SeriesConfig {
        series_id: "stream".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Ternary {
            c_column: "c".into(),
            scatter: Some(crate::data_config::DataScatterStyleConfig {
                point_color: Color::BLACK,
                point_shape: crate::ScatterShape::CircleFilled,
                point_size: 4.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            }),
            line: None,
            region: None,
        },
    };
    assert!(matches!(
        PrepareContext::validate_stream_series(&config, &series),
        Err(FiggyError::InvalidSeriesConfig { reason, .. })
            if reason.contains("ternary") && reason.contains("streamed sources are not supported")
    ));
}

/// Streaming is not extended to a box plot's violin or raw points (CLAUDE.md
/// rule 8): a streamed one is rejected with an explicit reason naming them.
#[test]
fn streamed_series_reject_violins_and_raw_points() {
    use crate::data_config::{BoxPoints, Violin, ViolinBody, ViolinHalf};
    let config = basic_errorbar_chart().config().clone();
    let series = SeriesConfig {
        series_id: "stream".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: String::new(),
        z_column: None,
        render_type: DataRenderType::BoxPlot {
            color: Color::BLACK,
            style: Default::default(),
            labels: Default::default(),
            outliers: None,
            overrides: None,
            violin: Some(Violin::new(ViolinBody::Symmetric(ViolinHalf {
                curve: "curve".into(),
                density: "density".into(),
                density_scale: 1.0,
                style: Default::default(),
            }))),
            points: Some(BoxPoints {
                positions: "x".into(),
                values: "y".into(),
                style: Default::default(),
            }),
        },
    };
    assert!(matches!(
        PrepareContext::validate_stream_series(&config, &series),
        Err(FiggyError::InvalidSeriesConfig { reason, .. })
            if reason.contains("violin") && reason.contains("raw point")
                && reason.contains("streamed sources are not supported")
    ));
}

/// Streaming is not extended to radar series: a streamed radar series is
/// rejected with an explicit reason (CLAUDE.md rule 8), on its polar chart
/// and before the polar rejection.
#[test]
fn streamed_series_reject_radar() {
    let mut config = basic_errorbar_chart().config().clone();
    config.polar = Some(crate::polar::PolarOptions::default());
    config.bottom_x.categories = Some(vec![
        crate::config::Category::new("a", "A"),
        crate::config::Category::new("b", "B"),
        crate::config::Category::new("c", "C"),
    ]);
    let series = SeriesConfig {
        series_id: "stream".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Radar {
            color: Color::BLACK,
            style: Default::default(),
            labels: Default::default(),
        },
    };
    assert!(matches!(
        PrepareContext::validate_stream_series(&config, &series),
        Err(FiggyError::InvalidSeriesConfig { reason, .. })
            if reason.contains("radar") && reason.contains("streamed sources are not supported")
    ));
}

fn streamline_series(id: &str, color: Option<&str>) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Streamlines {
            line: crate::data_config::DataLineStyleConfig::default(),
            arrows: Some(crate::data_config::StreamlineArrows::default()),
            color_map: color.map(|column| crate::data_config::PointColorMap {
                column: column.into(),
            }),
        },
    }
}

/// Streaming is not extended to streamlines (CLAUDE.md rule 8): a streamed
/// streamline series is rejected with an explicit reason, with or without
/// arrowheads and a colour map.
#[test]
fn streamed_sources_reject_streamlines() {
    let config = basic_errorbar_chart().config().clone();
    for mut series in [
        streamline_series("stream", None),
        streamline_series("stream", Some("c")),
    ] {
        for arrows in [true, false] {
            if let DataRenderType::Streamlines { arrows: a, .. } = &mut series.render_type
                && !arrows
            {
                *a = None;
            }
            assert!(matches!(
                PrepareContext::validate_stream_series(&config, &series),
                Err(FiggyError::InvalidSeriesConfig { reason, .. })
                    if reason.contains("streamline")
                        && reason.contains("streamed sources are not supported")
            ));
        }
    }
}

/// A polar chart rejects streamlines; a rectangular chart takes them, and
/// rejects a step interpolation, a fill, a head longer than its spacing and
/// a colour map without a colourbar, each atomically.
#[test]
fn polar_rejects_streamlines() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("x", &col_f64(vec![0.0, 1.0, f64::NAN, 0.0, 1.0]))
        .unwrap();
    renderer
        .add_column("y", &col_f64(vec![0.0, 1.0, f64::NAN, 1.0, 0.0]))
        .unwrap();
    renderer
        .add_column("c", &col_f64(vec![1.0, 2.0, f64::NAN, 3.0, 4.0]))
        .unwrap();
    let mut polar = crate::default::default_config();
    polar.polar = Some(crate::polar::PolarOptions::default());
    let chart = renderer.register_chart(polar, Vec::new()).unwrap();
    let before = renderer.visual_revision();
    assert!(matches!(
        renderer.set_chart_series(chart, vec![streamline_series("flow", None)]),
        Err(FiggyError::InvalidSeriesConfig { reason, .. }) if reason.contains("polar")
    ));
    assert!(renderer.chart_series(chart).unwrap().is_empty());
    assert_eq!(renderer.visual_revision(), before);

    let chart = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    renderer
        .set_chart_series(chart, vec![streamline_series("flow", None)])
        .unwrap();
    let rejected = |edit: fn(&mut SeriesConfig)| {
        let mut series = streamline_series("flow", None);
        edit(&mut series);
        series
    };
    for series in [
        rejected(|s| {
            if let DataRenderType::Streamlines { line, .. } = &mut s.render_type {
                line.interpolation = LineInterpolation::StepPost;
            }
        }),
        rejected(|s| {
            if let DataRenderType::Streamlines { line, .. } = &mut s.render_type {
                line.fill = Some(LineFill::ToBaseline(0.0));
            }
        }),
        rejected(|s| {
            if let DataRenderType::Streamlines {
                arrows: Some(arrows),
                ..
            } = &mut s.render_type
            {
                arrows.spacing_px = arrows.head_length_px * 0.5;
            }
        }),
        streamline_series("flow", Some("c")),
        streamline_series("flow", Some("missing")),
    ] {
        assert!(
            renderer
                .set_chart_series(chart, vec![series.clone()])
                .is_err(),
            "{series:?}"
        );
        assert_eq!(
            renderer.chart_series(chart).unwrap(),
            [streamline_series("flow", None)]
        );
    }
    // A colour map with a colourbar is accepted.
    let mut config = crate::default::default_config();
    config.colorbar = Some(crate::default::default_colorbar_options());
    renderer
        .set_chart_state(chart, config, vec![streamline_series("flow", Some("c"))])
        .unwrap();
}

/// Streaming is not extended to polar charts: a streamed series on one
/// is rejected with an explicit reason.
#[test]
fn streamed_series_reject_polar_charts() {
    let mut config = basic_errorbar_chart().config().clone();
    config.polar = Some(crate::polar::PolarOptions::default());
    let series = SeriesConfig {
        series_id: "stream".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: Color::BLACK,
                point_shape: crate::data_config::ScatterShape::CircleFilled,
                point_size: 4.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            },
        },
    };
    assert!(matches!(
        PrepareContext::validate_stream_series(&config, &series),
        Err(FiggyError::InvalidSeriesConfig { reason, .. })
            if reason.contains("polar") && reason.contains("streamed sources are not supported")
    ));
    config.polar = None;
    assert!(PrepareContext::validate_stream_series(&config, &series).is_ok());
}

/// Streaming is not extended to categorical axes: a streamed series on a
/// chart with one, the overlays' included, is rejected with an explicit
/// reason.
#[test]
fn streamed_series_reject_categorical_axes() {
    let base = basic_errorbar_chart().config().clone();
    let series = SeriesConfig {
        series_id: "stream".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    assert!(PrepareContext::validate_stream_series(&base, &series).is_ok());
    let categories = Some(vec![crate::config::Category::new("a", "A")]);
    let edits: [fn(&mut Config, Option<Vec<crate::config::Category>>); 5] = [
        |c, k| c.top_x.categories = k,
        |c, k| c.bottom_x.categories = k,
        |c, k| c.left_y.categories = k,
        |c, k| c.right_y.categories = k,
        |c, k| {
            let mut layer = crate::OverlayLayer::default();
            layer.y_axis.categories = k;
            c.overlays.push(layer);
        },
    ];
    for (index, edit) in edits.into_iter().enumerate() {
        let mut config = base.clone();
        edit(&mut config, categories.clone());
        assert!(
            matches!(
                PrepareContext::validate_stream_series(&config, &series),
                Err(FiggyError::InvalidSeriesConfig { reason, .. })
                    if reason.contains("categorical")
                        && reason.contains("streamed sources are not supported")
            ),
            "axis {index}"
        );
    }
}

/// The vector scratch (params, grid table, bind groups) is rebuilt only
/// when its inputs change: two prepares of the same chart share one
/// entry, a style edit replaces its params, and replacing a component
/// column replaces its source epochs.
#[test]
fn vector_scratch_is_reused_until_its_inputs_change() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
    renderer
        .add_column("u", &col_f64(vec![30.0, 30.0]))
        .unwrap();
    renderer.add_column("v", &col_f64(vec![0.0, 0.0])).unwrap();
    let chart = basic_errorbar_chart();
    let mut series = [vector_points_series("reuse")];

    let first = render_mapped_style_case(&mut renderer, &chart, &series);
    assert!(
        count_rgba_ink(&first.screen, is_saturated_red) > 50,
        "the arrows draw on screen"
    );
    assert!(
        count_rgba_ink(&first.export, is_saturated_red) > 50,
        "the arrows draw on export"
    );
    assert_eq!(renderer.vector_cache.len(), 1);
    let before = renderer.vector_cache["reuse"].signature.params;
    let epochs = renderer.vector_cache["reuse"].signature.epochs.clone();
    let second = render_mapped_style_case(&mut renderer, &chart, &series);
    assert_eq!(renderer.vector_cache.len(), 1);
    assert_eq!(renderer.vector_cache["reuse"].signature.params, before);
    assert_eq!(
        first.screen, second.screen,
        "an unchanged chart repaints identically"
    );

    let DataRenderType::Vector { style, .. } = &mut series[0].render_type else {
        unreachable!()
    };
    style.width_px = 12.0;
    let third = render_mapped_style_case(&mut renderer, &chart, &series);
    let after = renderer.vector_cache["reuse"].signature.params;
    assert_ne!(
        after.width_px, before.width_px,
        "a style edit rebuilds the params"
    );
    assert!(
        count_rgba_ink(&third.screen, is_saturated_red)
            > count_rgba_ink(&first.screen, is_saturated_red),
        "the wider shaft draws more"
    );

    renderer.remove_column("u").unwrap();
    renderer
        .add_column("u", &col_f64(vec![-30.0, -30.0]))
        .unwrap();
    let fourth = render_mapped_style_case(&mut renderer, &chart, &series);
    assert_ne!(
        renderer.vector_cache["reuse"].signature.epochs, epochs,
        "a replaced component column rebuilds the sources"
    );
    assert_ne!(fourth.screen, third.screen, "the arrows now point left");
}

/// `error_bar_style_overrides` without an index column is the errorbar
/// counterpart of a single-point edit.
#[test]
fn override_only_errorbar_paints_on_screen_and_export() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();
    renderer.add_column("ey", &col_f64(vec![1.0, 1.0])).unwrap();

    let chart = basic_errorbar_chart();
    let mut scatter = test_scatter_style();
    scatter.point_size = 0.0;
    let series = [SeriesConfig {
        series_id: "ov-errorbar".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::ScatterErrorbarY {
            scatter,
            err_y: ErrorRef::Symmetric {
                column: "ey".into(),
            },
            err_style: DataErrorBarStyleConfig {
                error_bar_color: Color::BLACK,
                error_bar_width: 2.0,
                error_bar_cap_size: 7.0,
                cap_width: 2.0,
                error_bar_style_table: None,
                error_bar_style_index_column: None,
                error_bar_style_overrides: Some(vec![
                    DataErrorBarPointStyleOverride {
                        index: 0,
                        style: DataErrorBarPointStyleConfig {
                            error_bar_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
                            ..Default::default()
                        },
                    },
                    DataErrorBarPointStyleOverride {
                        index: 1,
                        style: DataErrorBarPointStyleConfig {
                            error_bar_color: Some(Color::new(0.0, 0.0, 1.0, 1.0)),
                            ..Default::default()
                        },
                    },
                ]),
            },
        },
    }];
    let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

    let map = rendered.styles[0]
        .errorbar_map
        .as_ref()
        .expect("overrides-only errorbar must build a style map");
    assert!(!map.has_index);
    assert!(
        rendered.errorbar_mapped,
        "prepare must compile mapped errorbar when errorbar_map is Some"
    );
    assert!(
        !rendered.scatter_mapped,
        "hidden unmapped scatter must not compile mapped scatter"
    );

    assert_ink_on_screen_and_export(
        &rendered,
        "override-only green errorbar",
        is_saturated_green,
        20,
    );
    assert_ink_on_screen_and_export(
        &rendered,
        "override-only blue errorbar",
        is_saturated_blue,
        20,
    );
}

/// Existing table + index-column mapped path, scatter and errorbar, both
/// on-screen paint and PNG/export.
#[test]
fn table_and_index_mapped_scatter_and_errorbar_paints_on_screen_and_export() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("sx", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("sy", &col_f64(vec![3.5, 3.5])).unwrap();
    renderer
        .add_column("s_style", &col_f64(vec![0.0, 1.0]))
        .unwrap();
    renderer.add_column("ex", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("ey", &col_f64(vec![1.2, 1.2])).unwrap();
    renderer.add_column("ee", &col_f64(vec![0.7, 0.7])).unwrap();
    renderer
        .add_column("e_style", &col_f64(vec![0.0, 1.0]))
        .unwrap();

    let chart = basic_errorbar_chart();
    let mut hidden_scatter = test_scatter_style();
    hidden_scatter.point_size = 0.0;
    let series = [
        SeriesConfig {
            series_id: "mapped-scatter".into(),
            source_id: None,
            label: None,
            x_column: "sx".into(),
            y_column: "sy".into(),
            z_column: None,
            render_type: DataRenderType::Scatter {
                scatter: DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: crate::data_config::ScatterShape::CircleFilled,
                    point_size: 10.0,
                    point_style_table: Some(vec![
                        DataScatterPointStyleConfig {
                            point_color: Some(Color::new(1.0, 0.0, 0.0, 1.0)),
                            ..Default::default()
                        },
                        DataScatterPointStyleConfig {
                            point_color: Some(Color::new(0.0, 0.0, 1.0, 1.0)),
                            ..Default::default()
                        },
                    ]),
                    point_style_index_column: Some("s_style".into()),
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
            },
        },
        SeriesConfig {
            series_id: "mapped-errorbar".into(),
            source_id: None,
            label: None,
            x_column: "ex".into(),
            y_column: "ey".into(),
            z_column: None,
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: hidden_scatter,
                err_y: ErrorRef::Symmetric {
                    column: "ee".into(),
                },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: Color::BLACK,
                    error_bar_width: 2.0,
                    error_bar_cap_size: 7.0,
                    cap_width: 2.0,
                    error_bar_style_table: Some(vec![
                        DataErrorBarPointStyleConfig {
                            error_bar_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
                            ..Default::default()
                        },
                        DataErrorBarPointStyleConfig {
                            error_bar_color: Some(Color::new(0.0, 0.0, 1.0, 1.0)),
                            ..Default::default()
                        },
                    ]),
                    error_bar_style_index_column: Some("e_style".into()),
                    error_bar_style_overrides: None,
                },
            },
        },
    ];
    let rendered = render_mapped_style_case(&mut renderer, &chart, &series);

    let scatter_map = rendered.styles[0]
        .scatter_map
        .as_ref()
        .expect("table+index scatter must build a style map");
    assert!(scatter_map.has_index);
    let errorbar_map = rendered.styles[1]
        .errorbar_map
        .as_ref()
        .expect("table+index errorbar must build a style map");
    assert!(errorbar_map.has_index);
    assert!(rendered.scatter_mapped);
    assert!(rendered.errorbar_mapped);

    assert_ink_on_screen_and_export(&rendered, "table+index red scatter", is_saturated_red, 20);
    assert_ink_on_screen_and_export(
        &rendered,
        "table+index green errorbar",
        is_saturated_green,
        20,
    );
    assert_ink_on_screen_and_export(
        &rendered,
        "table+index blue mapped marks",
        is_saturated_blue,
        20,
    );
}

/// A style table with no index column and no overrides still produces a
/// style map (`has_index = false`). Draw uses the mapped pipeline, so
/// prepare must compile it even though the table cannot be indexed.
#[test]
fn style_table_without_index_still_ensures_mapped_scatter_pipeline() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.add_column("x", &col_f64(vec![1.0, 3.0])).unwrap();
    renderer.add_column("y", &col_f64(vec![2.5, 2.5])).unwrap();

    let chart = basic_errorbar_chart();
    let series = [SeriesConfig {
        series_id: "table-only".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: Color::new(1.0, 0.0, 0.0, 1.0),
                point_shape: crate::data_config::ScatterShape::CircleFilled,
                point_size: 10.0,
                point_style_table: Some(vec![DataScatterPointStyleConfig {
                    point_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
                    ..Default::default()
                }]),
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            },
        },
    }];
    let rendered = render_mapped_style_case(&mut renderer, &chart, &series);
    let map = rendered.styles[0]
        .scatter_map
        .as_ref()
        .expect("a non-empty style table still builds a map");
    assert!(!map.has_index);
    assert!(
        rendered.scatter_mapped,
        "table-only map must compile mapped scatter"
    );
    // Table rows are ignored without an index column; base red must paint.
    assert_ink_on_screen_and_export(
        &rendered,
        "table-only base red scatter",
        is_saturated_red,
        20,
    );
}

#[test]
fn single_direction_errorbar_needs_no_internal_filler_column() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    r.add_column("x", &col_f64(vec![1.0, 2.0, 3.0])).unwrap();
    r.add_column("y", &col_f64(vec![1.0, 3.0, 2.0])).unwrap();
    r.add_column("ey", &col_f64(vec![0.3, 0.1, 0.2])).unwrap();

    let chart = basic_errorbar_chart();
    let series_configs = [SeriesConfig {
        series_id: "y".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::ScatterErrorbarY {
            scatter: test_scatter_style(),
            err_y: ErrorRef::Symmetric {
                column: "ey".into(),
            },
            err_style: test_errorbar_style(),
        },
    }];

    {
        let style = r.create_style_for_series(&series_configs[0]).unwrap();
        let drawable = [Series {
            config: &series_configs[0],
            style: &style,
        }];
        let view = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &drawable,
        }];
        r.prepare(&items)
            .expect("single-direction errorbar prepare");
    }
    assert!(r.pool().slot("__zero").is_none());

    let mut export_renderer = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    export_renderer
        .add_column("x", &col_f64(vec![1.0, 2.0, 3.0]))
        .unwrap();
    export_renderer
        .add_column("y", &col_f64(vec![1.0, 3.0, 2.0]))
        .unwrap();
    export_renderer
        .add_column("ey", &col_f64(vec![0.3, 0.1, 0.2]))
        .unwrap();
    let image = export_renderer
        .export_panel_rgba(&chart, &series_configs, 1.0)
        .expect("single-direction errorbar export");
    assert!(export_renderer.pool().slot("__zero").is_none());
    assert!(image.rgba.chunks_exact(4).any(|pixel| pixel[3] > 0));
}

#[test]
fn differently_sized_single_direction_panels_keep_both_prepared_frames_valid() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .add_column("short_x", &col_f64(vec![1.0, 2.0]))
        .unwrap();
    renderer
        .add_column("short_y", &col_f64(vec![2.0, 3.0]))
        .unwrap();
    renderer
        .add_column("short_ey", &col_f64(vec![0.2, 0.3]))
        .unwrap();
    renderer
        .add_column("long_x", &col_f64((0..9).map(f64::from).collect()))
        .unwrap();
    renderer
        .add_column(
            "long_y",
            &col_f64((0..9).map(|value| f64::from(value) * 0.5).collect()),
        )
        .unwrap();
    renderer
        .add_column("long_ex", &col_f64(vec![0.25; 9]))
        .unwrap();

    let chart_a = basic_errorbar_chart();
    let chart_b = basic_errorbar_chart();
    let config_a = SeriesConfig {
        series_id: "short-y-error".into(),
        source_id: None,
        label: None,
        x_column: "short_x".into(),
        y_column: "short_y".into(),
        z_column: None,
        render_type: DataRenderType::ScatterErrorbarY {
            scatter: test_scatter_style(),
            err_y: ErrorRef::Symmetric {
                column: "short_ey".into(),
            },
            err_style: test_errorbar_style(),
        },
    };
    let config_b = SeriesConfig {
        series_id: "long-x-error".into(),
        source_id: None,
        label: None,
        x_column: "long_x".into(),
        y_column: "long_y".into(),
        z_column: None,
        render_type: DataRenderType::ScatterErrorbarX {
            scatter: test_scatter_style(),
            err_x: ErrorRef::Symmetric {
                column: "long_ex".into(),
            },
            err_style: test_errorbar_style(),
        },
    };
    let style_a = renderer.create_style_for_series(&config_a).unwrap();
    let style_b = renderer.create_style_for_series(&config_b).unwrap();
    assert_eq!(
        errorbar_direction_flags(&config_a.render_type),
        ERRORBAR_HAS_Y
    );
    assert_eq!(
        errorbar_direction_flags(&config_b.render_type),
        ERRORBAR_HAS_X
    );
    let series_a = [Series {
        config: &config_a,
        style: &style_a,
    }];
    let series_b = [Series {
        config: &config_b,
        style: &style_b,
    }];
    let view_a = renderer
        .create_chart_view(&chart_a, chart_a.config().chart_area.0)
        .unwrap();
    let view_b = renderer
        .create_chart_view(&chart_b, chart_b.config().chart_area.0)
        .unwrap();
    let prepared_a = renderer
        .prepare(&[ChartDrawItem {
            view: &view_a,
            chart_config: chart_a.config(),
            series: &series_a,
        }])
        .unwrap();
    let prepared_b = renderer
        .prepare(&[ChartDrawItem {
            view: &view_b,
            chart_config: chart_b.config(),
            series: &series_b,
        }])
        .unwrap();

    renderer.validate_prepared(&prepared_a).unwrap();
    renderer.validate_prepared(&prepared_b).unwrap();
    assert!(renderer.pool().slot("__zero").is_none());
}

/// Two line series on distinct column pairs must BOTH render — this
/// mirrors the web demo (sine on x/y + rc on t/v) and guards the
/// second-series draw path end to end via headless export.
#[test]
fn two_line_series_both_render() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();

    let n = 512;
    let period = 628.0_f64 / 100.0;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 * period / n as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
    let ts: Vec<f64> = (0..n).map(|i| i as f64 * 5.0 / n as f64).collect();
    let vs: Vec<f64> = ts.iter().map(|t| 1.0 - (-t).exp()).collect();
    r.add_column("x", &col_f64(xs)).unwrap();
    r.add_column("y", &col_f64(ys)).unwrap();
    r.add_column("t", &col_f64(ts)).unwrap();
    r.add_column("v", &col_f64(vs)).unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 640,
        height: 480,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 6.5);
    chart.set_y_range(-1.2, 1.2);

    let line_cfg = |id: &str, x: &str, y: &str, color: Color| SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: color,
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    let series = [
        line_cfg("sine", "x", "y", Color::BLACK),
        line_cfg("rc", "t", "v", Color::new(1.0, 0.0, 0.0, 1.0)),
    ];

    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let (mut red, mut black) = (0usize, 0usize);
    for px in img.rgba.chunks_exact(4) {
        if px[3] > 16 {
            if px[0] > 120 && px[1] < 90 && px[2] < 90 {
                red += 1;
            }
            if px[0] < 90 && px[1] < 90 && px[2] < 90 {
                black += 1;
            }
        }
    }
    assert!(black > 300, "sine (black) curve missing: {black} px");
    assert!(red > 300, "rc (red) curve missing: {red} px");
}

/// Milkyway arc-star density must be a property of the polyline's ARC, not of how
/// densely the data samples it — the field-reported failure mode of the
/// old per-segment quad budget, where a sparse polyline saturated at 24
/// stars per segment and then ignored the density knob entirely. The
/// same curve sampled at 8 vs 480 points must produce comparable star
/// ink (the indirect pass budgets by total arc; the 8-point chord arc is
/// only a few % shorter).
#[test]
fn milkyway_arc_star_density_is_sampling_invariant() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();

    let curve = |n: usize| -> (Vec<f64>, Vec<f64>) {
        let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
        let ys: Vec<f64> = xs.iter().map(|x| 50.0 + 30.0 * (x * 4.2).sin()).collect();
        (xs, ys)
    };
    let (sx, sy) = curve(8);
    let (dx, dy) = curve(480);
    r.add_column("inv_sx", &col_f64(sx)).unwrap();
    r.add_column("inv_sy", &col_f64(sy)).unwrap();
    r.add_column("inv_dx", &col_f64(dx)).unwrap();
    r.add_column("inv_dy", &col_f64(dy)).unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 640,
        height: 400,
    });
    // Stars only: ribbon/backdrop/glow off so the ink count measures the
    // star field alone. Density 60 needs ~540 stars over this arc — the
    // old per-segment budget would cap the 8-point series at 7·24 = 168.
    config.draw_style = crate::config::DrawStyle::Milkyway(crate::config::MilkywayOptions {
        star_density: 60.0,
        ribbon_intensity: 0.0,
        nebula: 0.0,
        dust: 0.0,
        glow: 0.0,
        ..Default::default()
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(-0.05, 1.05);
    chart.set_y_range(0.0, 100.0);

    let line_cfg = |id: &str, x: &str, y: &str| SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    let mut star_ink = |id: &str, x: &str, y: &str| -> usize {
        let series = [line_cfg(id, x, y)];
        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        img.rgba
            .chunks_exact(4)
            .filter(|p| p[3] > 0 && (p[0] > 40 || p[1] > 40 || p[2] > 40))
            .count()
    };
    // Same series id on purpose: identical salt isolates sampling as the
    // only variable.
    let sparse = star_ink("inv", "inv_sx", "inv_sy");
    let dense = star_ink("inv", "inv_dx", "inv_dy");

    assert!(sparse > 1_000, "sparse star field missing ({sparse} px)");
    assert!(dense > 1_000, "dense star field missing ({dense} px)");
    let ratio = sparse as f64 / dense as f64;
    assert!(
        (0.65..=1.55).contains(&ratio),
        "star ink must not depend on sampling density: sparse {sparse} px \
         vs dense {dense} px (ratio {ratio:.2})"
    );
}

/// A star pass has no slot cap (#107 R7): a series whose arc needs more
/// than the old 2 000 000 candidate slots still draws stars on its tail.
/// The series zig-zags across the left 40% of the plot for about 1.4 M px of
/// arc (2.8 M slots at the default 0.5 px pitch), then runs straight across
/// the right 40%. Every star on that run sits past slot 2 000 000.
#[test]
fn milkyway_stars_draw_past_two_million_arc_slots() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();

    let zig = 4_000usize;
    let tail = 64usize;
    let xs: Vec<f64> = (0..zig)
        .map(|i| 0.4 * i as f64 / (zig - 1) as f64)
        .chain((0..tail).map(|i| 0.6 + 0.4 * i as f64 / (tail - 1) as f64))
        .collect();
    let ys: Vec<f64> = (0..zig)
        .map(|i| if i % 2 == 0 { 2.0 } else { 98.0 })
        .chain((0..tail).map(|_| 50.0))
        .collect();
    r.add_column("cap_x", &col_f64(xs.clone())).unwrap();
    r.add_column("cap_y", &col_f64(ys.clone())).unwrap();
    // The tail alone: the star ink the long series must still reach there.
    r.add_column("tail_x", &col_f64(xs[zig..].to_vec()))
        .unwrap();
    r.add_column("tail_y", &col_f64(ys[zig..].to_vec()))
        .unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 640,
        height: 400,
    });
    config.draw_style = crate::config::DrawStyle::Milkyway(crate::config::MilkywayOptions {
        star_density: 60.0,
        ribbon_intensity: 0.0,
        nebula: 0.0,
        dust: 0.0,
        glow: 0.0,
        ..Default::default()
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 100.0);

    let line_cfg = |x: &str, y: &str| SeriesConfig {
        series_id: "cap".into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    // Ink in the right fifth of the image, where only the tail run reaches.
    let mut right_ink = |series: &[SeriesConfig]| -> usize {
        let img = r.export_panel_rgba(&chart, series, 1.0).unwrap();
        let width = img.width as usize;
        img.rgba
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, p)| {
                i % width > width * 4 / 5 && p[3] > 0 && (p[0] > 40 || p[1] > 40 || p[2] > 40)
            })
            .count()
    };
    let empty = right_ink(&[]);
    let tail_only = right_ink(&[line_cfg("tail_x", "tail_y")]).saturating_sub(empty);
    // The cost grows with the drawn arc and is the host's to budget
    // (rule 10): measured here, never capped.
    let started = std::time::Instant::now();
    let long = right_ink(&[line_cfg("cap_x", "cap_y")]).saturating_sub(empty);
    eprintln!(
        "{} points, about 2.8 M star slots: export {:?}",
        zig + tail,
        started.elapsed()
    );

    assert!(
        tail_only > 100,
        "the tail alone must draw stars ({tail_only} px)"
    );
    assert!(
        long * 2 > tail_only,
        "stars past slot 2 000 000 must draw: {long} px on the tail of the long \
         series vs {tail_only} px for the tail alone"
    );
}

/// The constellation backdrop cache must rebake only when its key
/// (panel size, nebula, dust, seed) changes: range-only refreshes hit
/// the cache, and a view already holding the baked generation skips the
/// texture re-upload (`grid_space_gen` stamp).
#[test]
fn space_background_rebakes_only_on_key_change() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();

    let rect = Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 200,
    };
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(rect);
    config.draw_style =
        crate::config::DrawStyle::Milkyway(crate::config::MilkywayOptions::default());
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);

    let mut view = r.create_chart_view(&chart, rect).unwrap();
    assert_eq!(view.grid_space_gen, None, "fresh view starts unstamped");

    r.refresh_axis(&mut view, &chart, rect).unwrap();
    assert_eq!(r.space_bg.as_ref().map(|s| s.bake_gen), Some(1));
    assert_eq!(view.grid_space_gen, Some(1));

    // Range-only change (pan/zoom): cache hit — no rebake, no restamp.
    chart.set_x_range(0.0, 2.0);
    r.refresh_axis(&mut view, &chart, rect).unwrap();
    assert_eq!(
        r.space_bg.as_ref().map(|s| s.bake_gen),
        Some(1),
        "range-only refresh must not rebake the backdrop"
    );
    assert_eq!(view.grid_space_gen, Some(1));

    // Backdrop parameter change: rebake + re-upload.
    if let crate::config::DrawStyle::Milkyway(c) = &mut chart.config_mut().draw_style {
        c.seed = 7;
    }
    r.refresh_axis(&mut view, &chart, rect).unwrap();
    assert_eq!(
        r.space_bg.as_ref().map(|s| s.bake_gen),
        Some(2),
        "seed change must rebake the backdrop"
    );
    assert_eq!(view.grid_space_gen, Some(2));

    // Leaving the style un-stamps the view; the slot itself is retained.
    chart.config_mut().draw_style = crate::config::DrawStyle::Precise;
    r.refresh_axis(&mut view, &chart, rect).unwrap();
    assert_eq!(view.grid_space_gen, None);
    assert_eq!(r.space_bg.as_ref().map(|s| s.bake_gen), Some(2));
}

#[test]
fn milkyway_export_zero_dust_zero_nebula_has_plain_background() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 96,
        height: 64,
    });
    config.chart_title.visible = false;
    config.legend.visible = false;
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.label_style.visible = false;
        axis.title_option.visible = false;
        axis.tick = crate::config::TickVisibility::None;
        axis.line_visible = false;
    }
    config.draw_style = crate::config::DrawStyle::Milkyway(crate::config::MilkywayOptions {
        nebula: 0.0,
        dust: 0.0,
        glow: 0.0,
        ..Default::default()
    });
    let chart = Chart::new(config);

    for scale in [1.0, 2.0] {
        let img = r.export_panel_rgba(&chart, &[], scale).unwrap();
        let off_base = img
            .rgba
            .chunks_exact(4)
            .filter(|p| {
                (p[0] as i16 - 11).abs() > 1
                    || (p[1] as i16 - 15).abs() > 1
                    || (p[2] as i16 - 23).abs() > 1
                    || p[3] != 255
            })
            .count();
        assert_eq!(
            off_base, 0,
            "dust=0 and nebula=0 must export a plain space background at {scale}x"
        );
    }
}

/// The GPU arc-length scan must equal a sequential CPU reference for
/// every dispatch shape: single block (n ≤ 256), one sums level, two
/// sums levels (n > 65 536), and the sequential multi-chunk path (forced
/// via a narrowed chunk size) that makes n unlimited — both an
/// exact-multiple boundary and a ragged tail. This is the correctness
/// proof that lets the pool keep NO CPU copy of column data.
#[test]
fn gpu_arc_prefix_matches_cpu_reference() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 900,
        height: 600,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(-1.2, 1.2);

    for (case, n, chunk) in [
        ("tiny", 3usize, None),
        ("one-level", 1000, None),
        ("two-level", 70_000, None),
        // Forced multi-chunk shapes: an exact-multiple boundary and a
        // ragged tail both exercise the sequential carry chain.
        ("chunked-even", 2_000, Some(1_000)),
        ("chunked-ragged", 2_500, Some(1_000)),
    ] {
        r.arc_chunk_override = chunk;
        let xs: Vec<f64> = (0..n).map(|i| i as f64 / n as f64).collect();
        let ys: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin()).collect();
        let xid = format!("ax_{case}");
        let yid = format!("ay_{case}");
        r.add_column(&xid, &col_f64(xs.clone())).unwrap();
        r.add_column(&yid, &col_f64(ys.clone())).unwrap();

        let t = data_render::scatter_transform_from_config(chart.config());
        let (arc_buf, len_bytes) = r
            .ensure_arc_prefix(
                &format!("s_{case}"),
                &xid,
                &yid,
                &t,
                None,
                LineInterpolation::Linear,
            )
            .expect("arc prefix")
            .prefix;

        // Read the GPU result back.
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: len_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&arc_buf, 0, &readback, 0, len_bytes);
        queue.submit(std::iter::once(enc.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        // Bounded so a stalled submission fails the test instead of
        // spinning forever (see `data_render::shared_device`).
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
        // Arc pairs (SHADER_COMMON.md §5.6): fixed-point (low, high) words.
        let gpu: Vec<f32> = bytemuck::cast_slice::<u8, [u32; 2]>(
            &slice
                .get_mapped_range()
                .expect("arc prefix readback is mapped"),
        )
        .iter()
        .map(|pair| arc_pair_px(*pair) as f32)
        .collect();

        // Sequential CPU reference mirroring the shader math.
        let px = |x: f64, y: f64| -> (f32, f32) {
            let axis_ndc = |value: f64, axis: usize| {
                let (hi, lo) = crate::data::split_f64_to_f32_pair(value);
                let numerator = (hi - t.data_min[axis]) + (lo - t.data_min_lo[axis]);
                let range = (t.data_max[axis] - t.data_min[axis])
                    + (t.data_max_lo[axis] - t.data_min_lo[axis]);
                let data_t = numerator / range;
                let panel_t = t.data_to_panel_offset[axis] + data_t * t.data_to_panel_scale[axis];
                panel_t * 2.0 - 1.0
            };
            let nx = axis_ndc(x, 0);
            let ny = axis_ndc(y, 1);
            (nx / t.pixel_to_ndc[0], ny / t.pixel_to_ndc[1])
        };
        let mut acc = 0.0f32;
        let mut reference = vec![0.0f32];
        for i in 1..n {
            let a = px(xs[i - 1], ys[i - 1]);
            let b = px(xs[i], ys[i]);
            acc += ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            reference.push(acc);
        }

        assert_eq!(gpu.len(), n, "{case}: length");
        let total = reference.last().copied().unwrap_or(0.0).max(1.0);
        for (i, (g, c)) in gpu.iter().zip(reference.iter()).enumerate() {
            assert!(
                (g - c).abs() <= total * 2e-4 + 0.05,
                "{case}: arc[{i}] gpu={g} cpu={c} (total {total})"
            );
        }
    }
}

/// One arc pair of the prefix (SHADER_COMMON.md §5.6 arc pairs): a 64-bit
/// fixed-point arc of 1/65536 px as (low, high) words, in pixels.
fn arc_pair_px(pair: [u32; 2]) -> f64 {
    (f64::from(pair[1]) * 4_294_967_296.0 + f64::from(pair[0])) / 65_536.0
}

/// The arc prefix is a fixed-point pair per point (#107, SHADER_COMMON.md
/// §5.6 arc pairs), so a line's cumulative arc stays exact far past 2^24 px. A
/// zig-zag of 100 000 points across a 600 px tall chart runs about 3.9e7 px
/// of arc. Every step of the GPU prefix must be its segment's length to the
/// few ulps WGSL allows `sqrt`, at every point; an f32 running sum of the
/// same lengths is off by pixels there.
#[test]
fn gpu_arc_prefix_stays_exact_past_two_to_the_twenty_four_pixels() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 900,
        height: 600,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(-1.2, 1.2);

    let n = 100_000usize;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 / n as f64).collect();
    let ys: Vec<f64> = (0..n)
        .map(|i| if i % 2 == 0 { -1.0 } else { 1.0 })
        .collect();
    r.add_column("exact_x", &col_f64(xs.clone())).unwrap();
    r.add_column("exact_y", &col_f64(ys.clone())).unwrap();
    let t = data_render::scatter_transform_from_config(chart.config());
    let (arc_buf, len_bytes) = r
        .ensure_arc_prefix(
            "exact",
            "exact_x",
            "exact_y",
            &t,
            None,
            LineInterpolation::Linear,
        )
        .expect("arc prefix")
        .prefix;
    assert_eq!(len_bytes, n as u64 * data_render::line_arc::ARC_PAIR_BYTES);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: len_bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_buffer_to_buffer(&arc_buf, 0, &readback, 0, len_bytes);
    queue.submit(std::iter::once(enc.finish()));
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(60)),
    });
    let gpu: Vec<f64> = bytemuck::cast_slice::<u8, [u32; 2]>(
        &slice
            .get_mapped_range()
            .expect("arc prefix readback is mapped"),
    )
    .iter()
    .map(|pair| arc_pair_px(*pair))
    .collect();

    // The shader's f32 pixel positions and segment lengths, summed in f64
    // and, for contrast, in f32.
    let px = |x: f64, y: f64| -> (f32, f32) {
        let axis_ndc = |value: f64, axis: usize| {
            let (hi, lo) = crate::data::split_f64_to_f32_pair(value);
            let numerator = (hi - t.data_min[axis]) + (lo - t.data_min_lo[axis]);
            let range =
                (t.data_max[axis] - t.data_min[axis]) + (t.data_max_lo[axis] - t.data_min_lo[axis]);
            let data_t = numerator / range;
            let panel_t = t.data_to_panel_offset[axis] + data_t * t.data_to_panel_scale[axis];
            panel_t * 2.0 - 1.0
        };
        (
            axis_ndc(x, 0) / t.pixel_to_ndc[0],
            axis_ndc(y, 1) / t.pixel_to_ndc[1],
        )
    };
    // WGSL lets `sqrt` err by a few ulps (it inherits `inverseSqrt`'s
    // accuracy), so a segment's f32 length is the adapter's: lavapipe
    // matches the CPU here, WARP is one ulp long on every segment and Metal
    // two (#63). Each prefix step must be that segment's length to within
    // `SQRT_ULPS` ulps and one arc unit; an f32 running sum would be off by
    // whole pixels at this arc (`single_worst`), so a step that is not the
    // exact difference of two prefix pairs still shows.
    const SQRT_ULPS: f64 = 4.0;
    let ulp = |d: f32| f64::from(f32::from_bits(d.to_bits() + 1) - d);
    let mut exact = 0.0f64;
    let mut single = 0.0f32;
    let mut worst = 0.0f64;
    let mut allowed = 0.0f64;
    let mut single_worst = 0.0f64;
    assert_eq!(gpu[0], 0.0);
    for i in 1..n {
        let a = px(xs[i - 1], ys[i - 1]);
        let b = px(xs[i], ys[i]);
        let d = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let bound = SQRT_ULPS * ulp(d) + 1.0 / 65_536.0;
        let step = gpu[i] - gpu[i - 1];
        assert!(
            (step - f64::from(d)).abs() <= bound,
            "arc step {i} is {step} px for a {d} px segment"
        );
        exact += f64::from(d);
        allowed += bound;
        single += d;
        worst = worst.max((gpu[i] - exact).abs());
        single_worst = single_worst.max((f64::from(single) - exact).abs());
    }
    assert!(
        exact > 2.0 * f64::from(1u32 << 24),
        "the arc runs past 2^24 px ({exact})"
    );
    assert!(
        single_worst > 1.0,
        "an f32 running sum must lose pixels here ({single_worst} px)"
    );
    assert!(
        worst <= allowed,
        "arc pairs must stay exact: worst {worst} px over {exact} px of arc, {allowed} px allowed"
    );
}

/// A stepped line's arc prefix sums its axis-aligned pieces in pixel
/// space — the same corners the vertex shader draws and the picker walks
/// — so the dash phase advances along the stepped path. Each mode is a
/// distinct cache key: the prefix of one mode is never reused for
/// another, and the linear prefix stays what it was.
#[test]
fn gpu_arc_prefix_follows_the_stepped_path_per_interpolation_mode() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 900,
        height: 600,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(-1.2, 1.2);
    let t = data_render::scatter_transform_from_config(chart.config());

    // Uneven x spacing and a NaN gap: the gap's segments contribute 0 to
    // every mode, and the gap stays exactly where the line breaks.
    let n = 300usize;
    let xs: Vec<f64> = (0..n).map(|i| (i as f64 / n as f64).powf(1.3)).collect();
    let ys: Vec<f64> = (0..n)
        .map(|i| {
            if i == 150 {
                f64::NAN
            } else {
                (i as f64 * 0.37).sin()
            }
        })
        .collect();
    r.add_column("step_ax", &col_f64(xs.clone())).unwrap();
    r.add_column("step_ay", &col_f64(ys.clone())).unwrap();

    let read = |device: &wgpu::Device,
                queue: &wgpu::Queue,
                arc_buf: &wgpu::Buffer,
                len_bytes: u64|
     -> Vec<f32> {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: len_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(arc_buf, 0, &readback, 0, len_bytes);
        queue.submit(std::iter::once(enc.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
        bytemuck::cast_slice(&slice.get_mapped_range().expect("arc readback mapped")).to_vec()
    };
    // Pixel position of a (hi, lo) pair point, mirroring the shader.
    let px = |x: (f32, f32), y: (f32, f32)| -> (f32, f32) {
        let axis_ndc = |(hi, lo): (f32, f32), axis: usize| {
            let numerator = (hi - t.data_min[axis]) + (lo - t.data_min_lo[axis]);
            let range =
                (t.data_max[axis] - t.data_min[axis]) + (t.data_max_lo[axis] - t.data_min_lo[axis]);
            let data_t = numerator / range;
            let panel_t = t.data_to_panel_offset[axis] + data_t * t.data_to_panel_scale[axis];
            panel_t * 2.0 - 1.0
        };
        (
            axis_ndc(x, 0) / t.pixel_to_ndc[0],
            axis_ndc(y, 1) / t.pixel_to_ndc[1],
        )
    };
    let pair = crate::data::split_f64_to_f32_pair;
    let mut prefixes = Vec::new();
    for mode in [
        LineInterpolation::Linear,
        LineInterpolation::StepPre,
        LineInterpolation::StepPost,
        LineInterpolation::StepMid,
    ] {
        let (arc_buf, len_bytes) = r
            .ensure_arc_prefix("stepped", "step_ax", "step_ay", &t, None, mode)
            .expect("arc prefix")
            .prefix;
        // Arc pairs (SHADER_COMMON.md §5.6): fixed-point (low, high) words.
        let gpu: Vec<f32> = read(&device, &queue, &arc_buf, len_bytes)
            .chunks_exact(2)
            .map(|pair| arc_pair_px([pair[0].to_bits(), pair[1].to_bits()]) as f32)
            .collect();
        assert_eq!(gpu.len(), n, "{mode:?}: length");

        // Sequential CPU reference over the stepped pieces.
        let mut acc = 0.0f32;
        let mut reference = vec![0.0f32];
        for i in 1..n {
            if !ys[i - 1].is_finite() || !ys[i].is_finite() {
                reference.push(acc);
                continue;
            }
            let (xa, ya, xb, yb) = (pair(xs[i - 1]), pair(ys[i - 1]), pair(xs[i]), pair(ys[i]));
            let xm = ((xa.0 + xb.0) * 0.5, (xa.1 + xb.1) * 0.5);
            let pieces: Vec<((f32, f32), (f32, f32), (f32, f32), (f32, f32))> = match mode {
                LineInterpolation::Linear => vec![(xa, ya, xb, yb)],
                LineInterpolation::StepPre => vec![(xa, ya, xa, yb), (xa, yb, xb, yb)],
                LineInterpolation::StepPost => vec![(xa, ya, xb, ya), (xb, ya, xb, yb)],
                LineInterpolation::StepMid => {
                    vec![(xa, ya, xm, ya), (xm, ya, xm, yb), (xm, yb, xb, yb)]
                }
            };
            for (x0, y0, x1, y1) in pieces {
                let a = px(x0, y0);
                let b = px(x1, y1);
                acc += ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            }
            reference.push(acc);
        }
        let total = reference.last().copied().unwrap_or(0.0).max(1.0);
        for (i, (g, c)) in gpu.iter().zip(reference.iter()).enumerate() {
            assert!(
                (g - c).abs() <= total * 2e-4 + 0.05,
                "{mode:?}: arc[{i}] gpu={g} cpu={c} (total {total})"
            );
        }
        prefixes.push((mode, gpu));
    }
    // A stepped path is longer than the chord, and every mode has its own
    // cached result for the same columns and transform.
    let linear_total = prefixes[0].1[n - 1];
    for (mode, gpu) in &prefixes[1..] {
        assert!(
            gpu[n - 1] > linear_total * 1.05,
            "{mode:?}: stepped total {} should exceed the linear {linear_total}",
            gpu[n - 1]
        );
    }
    assert_eq!(
        r.arc_cache["stepped"].len(),
        4,
        "one cached scan per interpolation mode"
    );
}

#[test]
fn arc_prefix_cache_never_exceeds_limit_on_new_series_churn() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    r.add_column("arc_cache_x", &col_f64(vec![0.0, 0.5, 1.0]))
        .unwrap();
    r.add_column("arc_cache_y", &col_f64(vec![0.0, 1.0, 0.25]))
        .unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);

    let t = data_render::scatter_transform_from_config(chart.config());
    for i in 0..=ARC_PREFIX_CACHE_LIMIT {
        let series_id = format!("arc-cache-series-{i}");
        assert!(
            r.ensure_arc_prefix(
                &series_id,
                "arc_cache_x",
                "arc_cache_y",
                &t,
                None,
                LineInterpolation::Linear,
            )
            .is_some()
        );
        assert!(
            r.arc_cache.len() <= ARC_PREFIX_CACHE_LIMIT,
            "arc cache exceeded limit after inserting {series_id}: {}",
            r.arc_cache.len()
        );
    }
}

/// Arc results are immutable by their exact compute input. Equal inputs
/// share one result without another traversal; a geometry/data generation
/// change gets a different buffer even after every prepared token drops.
#[test]
fn arc_results_reuse_only_exact_compute_keys() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    r.add_column("cache_x", &col_f64(vec![0.0, 0.5, 1.0]))
        .unwrap();
    r.add_column("cache_y", &col_f64(vec![0.0, 1.0, 0.25]))
        .unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    config.legend.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);

    let series_cfg = SeriesConfig {
        series_id: "arc-cache".into(),
        source_id: None,
        label: None,
        x_column: "cache_x".into(),
        y_column: "cache_y".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Dash,
                line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    let style = r.create_style_for_series(&series_cfg).unwrap();
    let series = [Series {
        config: &series_cfg,
        style: &style,
    }];
    let view_a = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let view_b = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let items_a = [ChartDrawItem {
        view: &view_a,
        chart_config: chart.config(),
        series: &series,
    }];
    let items_b = [ChartDrawItem {
        view: &view_b,
        chart_config: chart.config(),
        series: &series,
    }];

    let arc_of = |p: &PreparedFrame| -> Arc<wgpu::Buffer> {
        Arc::clone(
            &p.items[0].series[0]
                .line
                .as_ref()
                .expect("dashed series has a line layer")
                .arc
                .as_ref()
                .expect("dashed series has an arc prefix")
                .0,
        )
    };

    let p1 = r.prepare(&items_a).unwrap();
    let a1 = arc_of(&p1);
    let creations = r.gpu_memory_usage().creations_of(GpuResourceKind::ArcScan);
    let p2 = r.prepare(&items_b).unwrap();
    let a2 = arc_of(&p2);
    assert!(
        Arc::ptr_eq(&a1, &a2),
        "an identical compute key should reuse its immutable arc result"
    );
    assert_eq!(
        r.gpu_memory_usage().creations_of(GpuResourceKind::ArcScan),
        creations,
        "an identical key must not retraverse or allocate"
    );
    drop((p1, a1, p2, a2));

    let base_transform = data_render::scatter_transform_from_config(chart.config());
    let mut changed_transform = base_transform;
    changed_transform.pixel_to_ndc[0] *= 0.5;
    let changed = r
        .ensure_arc_prefix(
            "arc-cache",
            "cache_x",
            "cache_y",
            &changed_transform,
            None,
            LineInterpolation::Linear,
        )
        .expect("changed geometry arc result");
    let changed_arc = Arc::clone(&changed.prefix.0);
    assert!(
        !Arc::ptr_eq(&arc_of(&r.prepare(&items_a).unwrap()), &changed_arc),
        "a changed geometry transform must not overwrite the old result"
    );
    drop(changed);

    let changed_again = r
        .ensure_arc_prefix(
            "arc-cache",
            "cache_x",
            "cache_y",
            &changed_transform,
            None,
            LineInterpolation::Linear,
        )
        .expect("cached changed geometry arc result");
    assert!(
        Arc::ptr_eq(&changed_arc, &changed_again.prefix.0),
        "an exact changed key should reuse its immutable result"
    );

    let mut style_only = changed_transform;
    style_only.style_params[0][0] = 123.0;
    let style_only_result = r
        .ensure_arc_prefix(
            "arc-cache",
            "cache_x",
            "cache_y",
            &style_only,
            None,
            LineInterpolation::Linear,
        )
        .expect("style-only arc result");
    assert!(
        Arc::ptr_eq(&changed_arc, &style_only_result.prefix.0),
        "style-only changes must not retraverse geometry columns"
    );

    r.upsert_column("cache_y", &col_f64(vec![0.0, 0.8, 0.25]))
        .unwrap();
    let changed_data = r
        .ensure_arc_prefix(
            "arc-cache",
            "cache_x",
            "cache_y",
            &base_transform,
            None,
            LineInterpolation::Linear,
        )
        .expect("changed data generation arc result");
    assert!(
        !Arc::ptr_eq(&changed_arc, &changed_data.prefix.0),
        "a new source generation must not reuse an old prefix"
    );
    assert_eq!(
        r.arc_cache.get("arc-cache").map(Vec::len),
        Some(3),
        "the cache should hold base, changed-transform, and changed-data results"
    );
}

#[test]
fn contour_projection_matches_fixed_four_step_gpu_oracle() {
    for case in [
        "linear",
        "pan",
        "inverted",
        "log",
        "large-offset",
        "edges",
        "transposed",
        "nan",
        "infinity",
        "flat",
    ] {
        let (x, y, x_range) = match case {
            "pan" => (vec![0.0, 0.5, 1.0], vec![0.0, 0.5, 1.0], (-0.2, 1.2)),
            "inverted" => (vec![0.0, 0.5, 1.0], vec![0.0, 0.5, 1.0], (1.0, 0.0)),
            "log" => (vec![1.0, 10.0, 100.0], vec![0.0, 0.5, 1.0], (1.0, 100.0)),
            "large-offset" => (
                vec![1.0e9, 1.0e9 + 0.5, 1.0e9 + 1.0],
                vec![0.0, 0.5, 1.0],
                (1.0e9, 1.0e9 + 1.0),
            ),
            "edges" => (
                vec![0.0, 0.3, 0.7, 1.0],
                vec![0.0, 0.3, 0.7, 1.0],
                (0.0, 1.0),
            ),
            _ => (vec![0.0, 0.5, 1.0], vec![0.0, 0.5, 1.0], (0.0, 1.0)),
        };
        // Non-affine fields require repeated Newton corrections. Invalid
        // samples and zero gradients must retain their rejection behavior.
        let mut z = [
            vec![0.0, 0.0, 0.0],
            vec![0.0, 1.0, 2.0],
            vec![0.0, 2.0, 4.0],
        ];
        match case {
            "transposed" => z[1][2] = 3.0,
            "nan" => z[1][1] = f64::NAN,
            "infinity" => z[1][1] = f64::INFINITY,
            "flat" => z = [vec![2.0; 3], vec![2.0; 3], vec![2.0; 3]],
            _ => {}
        }
        let mut images = Vec::new();
        for legacy in [false, true] {
            let mut renderer = state_test_renderer().expect("required GPU renderer");
            // Scalar upload intentionally rounds to f32. This fixture
            // must exercise the explicit hi/lo contract to retain 0.5
            // increments at an offset of 1e9.
            if case == "large-offset" {
                renderer
                    .add_hilo_column("clx", &col_f64(x.clone()))
                    .unwrap();
            } else {
                renderer.add_column("clx", &col_f64(x.clone())).unwrap();
            }
            renderer.add_column("cly", &col_f64(y.clone())).unwrap();
            for (id, values) in ["clz0", "clz1", "clz2"].into_iter().zip(z.iter()) {
                renderer.add_column(id, &col_f64(values.clone())).unwrap();
            }
            if legacy {
                renderer.contour_label_pipelines = Some(
                    crate::gpu_contour::ContourLabelPipelines::legacy_projection_for_test(
                        &renderer.device,
                        &renderer.field_bgl,
                    ),
                );
            }
            let mut chart = state_test_contour_chart(
                Rect {
                    x: 0,
                    y: 0,
                    width: 320,
                    height: 240,
                },
                x_range,
            );
            if case == "log" {
                chart.config_mut().bottom_x.scale = AxisScale::Logarithmic;
                chart.config_mut().bottom_x.major_spacing = 1.0;
            }
            let mut config = state_test_labelled_contour(45.0, Vec::new());
            let DataRenderType::Contour { matrix, .. } = &mut config.render_type else {
                unreachable!()
            };
            if case == "edges" {
                matrix.grid_layout = crate::data_config::GridLayout::Edges;
            }
            if case == "transposed" {
                matrix.orientation = crate::data_config::MatrixOrientation::ColumnsAreY;
            }
            let style = renderer.create_style_for_series(&config).unwrap();
            let series = [Series {
                config: &config,
                style: &style,
            }];
            let view = renderer
                .create_chart_view(&chart, chart.config().chart_area.0)
                .unwrap();
            let items = [ChartDrawItem {
                view: &view,
                chart_config: chart.config(),
                series: &series,
            }];
            let prepared = renderer.prepare(&items).unwrap();
            let pixels = paint_prepared_rgba(&renderer, &prepared, 320, 240);
            let has_labels = pixels
                .chunks_exact(4)
                .any(|p| p[0] > 200 && p[1] < 100 && p[2] > 200);
            if !matches!(case, "nan" | "infinity" | "flat") {
                assert!(
                    has_labels,
                    "oracle fixture must produce automatic labels: {case}, legacy={legacy}"
                );
            } else if case == "flat" {
                assert!(
                    !has_labels,
                    "a zero gradient must not produce an automatic label"
                );
            }
            images.push(pixels);
        }
        // The old fused loop and four dispatches round intermediate f32
        // values at different compiler boundaries. On Metal this can move
        // antialiasing across one 8-bit quantization threshold (45 pixels
        // in the edge fixture), without changing the projection algorithm.
        // Bound every channel, including alpha; a displaced/missing contour
        // or label still fails. Do not dump two 300 KB vectors on failure.
        assert_eq!(images[0].len(), images[1].len());
        let max_error = images[0]
            .iter()
            .zip(&images[1])
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            max_error <= 1,
            "four Newton corrections changed the image: {case}, max channel error {max_error}"
        );
    }
}

/// Paint one contour-only series with automatic labels on a 320×240 panel
/// and return the RGBA frame. Labels carry an opaque magenta background, a
/// colour nothing else in this chart uses.
fn render_labelled_contour_for_test(
    columns: &[(&str, Vec<f64>)],
    x: &str,
    y: &str,
    z: &[&str],
    levels: Vec<f64>,
    spacing_px: f32,
) -> Vec<u8> {
    let mut renderer = state_test_renderer().expect("required GPU renderer");
    for (id, values) in columns {
        renderer.add_column(*id, &col_f64(values.clone())).unwrap();
    }
    let chart = state_test_contour_chart(
        Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        },
        (0.0, 1.0),
    );
    let mut config = state_test_labelled_contour(spacing_px, Vec::new());
    let DataRenderType::Contour { matrix, contour } = &mut config.render_type else {
        unreachable!()
    };
    matrix.columns = z.iter().map(|id| (*id).into()).collect();
    contour.levels = levels;
    config.x_column = x.into();
    config.y_column = y.into();
    let style = renderer.create_style_for_series(&config).unwrap();
    let series = [Series {
        config: &config,
        style: &style,
    }];
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];
    let prepared = renderer.prepare(&items).unwrap();
    paint_prepared_rgba(&renderer, &prepared, 320, 240)
}

fn label_background_pixels(rgba: &[u8]) -> usize {
    rgba.chunks_exact(4)
        .filter(|p| p[0] > 200 && p[1] < 100 && p[2] > 200)
        .count()
}

// Issue #19. An automatic anchor is only valid where its level is actually
// crossed inside the cell it lands in, which is the same test the stroke
// uses. A strictly positive field with a shallow minimum just below the
// projection tolerance used to accept a level-0 anchor at that minimum,
// where no isoline exists.
#[test]
fn automatic_contour_labels_require_a_level_crossing_in_their_cell() {
    let grid: Vec<f64> = (0..24).map(|i| i as f64 / 23.0).collect();
    let field = |x: f64, y: f64| 4.0e-4 + 0.05 * ((x - 0.3).powi(2) + (y - 0.4).powi(2));
    let ids: Vec<String> = (0..grid.len()).map(|i| format!("nfz{i}")).collect();
    let mut columns = vec![("nfx", grid.clone()), ("nfy", grid.clone())];
    for (i, id) in ids.iter().enumerate() {
        columns.push((
            id.as_str(),
            grid.iter().map(|y| field(grid[i], *y)).collect(),
        ));
    }
    let z: Vec<&str> = ids.iter().map(String::as_str).collect();
    // A level the field does cross still gets labels in this setup, so the
    // assertion below cannot pass merely because nothing is drawn.
    let crossed = render_labelled_contour_for_test(&columns, "nfx", "nfy", &z, vec![0.01], 40.0);
    assert!(
        label_background_pixels(&crossed) > 0,
        "a crossed level must still be labelled"
    );
    let uncrossed = render_labelled_contour_for_test(&columns, "nfx", "nfy", &z, vec![0.0], 40.0);
    assert_eq!(
        label_background_pixels(&uncrossed),
        0,
        "a level the field never crosses must not be labelled"
    );
}

// Issue #20. A rotated label quad used to end exactly at the edge of its
// baked background box, so the box edge was the rasterized quad edge and
// stair-stepped. The quad now reaches into the transparent gutter and the
// texture filter antialiases the box edge like the glyphs inside it.
#[test]
fn rotated_contour_label_background_edges_are_antialiased() {
    let columns = [
        ("clx", vec![0.0, 0.5, 1.0]),
        ("cly", vec![0.0, 0.5, 1.0]),
        ("clz0", vec![0.0, 1.0, 2.0]),
        ("clz1", vec![1.0, 2.0, 3.0]),
        ("clz2", vec![2.0, 3.0, 4.0]),
    ];
    // z = i + j: the level-2 isoline is the panel's diagonal, so the one
    // label is rotated away from both axes.
    let rgba = render_labelled_contour_for_test(
        &columns,
        "clx",
        "cly",
        &["clz0", "clz1", "clz2"],
        vec![2.0],
        400.0,
    );
    // The panel clears to transparent black, so the frame is premultiplied
    // magenta over nothing: an opaque box pixel is (255, 0, 255, 255) and a
    // partially covered edge pixel is (a, 0, a, a). Glyphs are drawn over the
    // opaque box and stay opaque, so they are never counted as edge.
    let solid = rgba
        .chunks_exact(4)
        .filter(|p| p[0] >= 250 && p[1] <= 2 && p[2] >= 250 && p[3] == 255)
        .count();
    let edge = rgba
        .chunks_exact(4)
        .filter(|p| {
            let premultiplied_magenta =
                p[1] <= 2 && p[0].abs_diff(p[2]) <= 2 && p[0].abs_diff(p[3]) <= 2;
            premultiplied_magenta && (8..=247).contains(&p[3])
        })
        .count();
    assert!(solid > 100, "the label background must be drawn: {solid}");
    assert!(
        edge >= 40,
        "a rotated label background needs antialiased edges, found {edge} partial pixels"
    );
}

#[test]
fn contour_label_placements_reuse_only_exact_dispatch_keys() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_contour_grid(&mut renderer);
    let chart_a = state_test_contour_chart(
        Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        },
        (0.0, 1.0),
    );
    let chart_b = state_test_contour_chart(
        Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        },
        (-0.5, 1.5),
    );
    let automatic_a = state_test_labelled_contour(70.0, Vec::new());
    let automatic_b = state_test_labelled_contour(260.0, Vec::new());
    let style_a = renderer.create_style_for_series(&automatic_a).unwrap();
    let style_b = renderer.create_style_for_series(&automatic_b).unwrap();
    let series_a = [Series {
        config: &automatic_a,
        style: &style_a,
    }];
    let series_b = [Series {
        config: &automatic_b,
        style: &style_b,
    }];
    let view_a = renderer
        .create_chart_view(&chart_a, chart_a.config().chart_area.0)
        .unwrap();
    let view_b = renderer
        .create_chart_view(&chart_b, chart_b.config().chart_area.0)
        .unwrap();
    let items_a = [ChartDrawItem {
        view: &view_a,
        chart_config: chart_a.config(),
        series: &series_a,
    }];
    let items_b = [ChartDrawItem {
        view: &view_b,
        chart_config: chart_b.config(),
        series: &series_b,
    }];
    let placement_of = |prepared: &PreparedFrame| {
        prepared.items[0].series[0]
            .contour
            .as_ref()
            .and_then(|contour| contour.label.as_ref())
            .expect("labelled contour snapshot")
            .snapshot
            .placement_identity()
    };

    let first = renderer.prepare(&items_a).unwrap();
    let first_id = placement_of(&first);
    let second = renderer.prepare(&items_b).unwrap();
    let second_id = placement_of(&second);
    assert_ne!(
        first_id, second_id,
        "different dispatch keys reused one automatic placement result"
    );
    assert!(renderer.validate_prepared(&first).is_ok());
    assert!(renderer.validate_prepared(&second).is_ok());
    assert_eq!(
        renderer.contour_cache["label-cache"]
            .contour()
            .unwrap()
            .label
            .as_ref()
            .unwrap()
            .automatic_result_count(),
        2
    );

    let creations = renderer
        .gpu_memory_usage()
        .creations_of(GpuResourceKind::ContourScratch);
    drop((first, second));
    let reused = renderer.prepare(&items_a).unwrap();
    let reused_id = placement_of(&reused);
    assert_eq!(
        reused_id, first_id,
        "the exact A input must reuse A's immutable result"
    );
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .creations_of(GpuResourceKind::ContourScratch),
        creations,
        "an exact automatic placement key should not allocate or dispatch again"
    );
    drop(reused);

    // Record A without submitting it, release the Rust token, then cross
    // the eight-result cache limit with distinct transforms. The delayed
    // command buffer must still read A's immutable anchors and indirect
    // count, while its evicted charge remains retired until submission.
    let baseline_a = {
        let prepared = renderer.prepare(&items_a).unwrap();
        paint_prepared_rgba(&renderer, &prepared, 320, 240)
    };
    let delayed_a = {
        let prepared = renderer.prepare(&items_a).unwrap();
        record_prepared_rgba(&renderer, &prepared, 320, 240)
    };
    let mut pixels_c = None;
    for key in 0..9 {
        let min = -1.0 - f64::from(key) * 0.125;
        let chart_c = state_test_contour_chart(
            Rect {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            },
            (min, 2.0 + f64::from(key) * 0.25),
        );
        let view_c = renderer
            .create_chart_view(&chart_c, chart_c.config().chart_area.0)
            .unwrap();
        let items_c = [ChartDrawItem {
            view: &view_c,
            chart_config: chart_c.config(),
            series: &series_a,
        }];
        let prepared = renderer.prepare(&items_c).unwrap();
        pixels_c = Some(paint_prepared_rgba(&renderer, &prepared, 320, 240));
    }
    assert_eq!(
        renderer.contour_cache["label-cache"]
            .contour()
            .unwrap()
            .label
            .as_ref()
            .unwrap()
            .automatic_result_count(),
        crate::gpu_contour::AUTOMATIC_PLACEMENT_CACHE_LIMIT
    );
    assert!(
        renderer
            .gpu_memory_usage()
            .retired_bytes_of(GpuResourceKind::ContourScratch)
            > 0,
        "evicted recorded contour resources must remain budgeted before submission"
    );
    let delayed_pixels_a = submit_recorded_rgba(&renderer, delayed_a);
    assert_eq!(
        delayed_pixels_a, baseline_a,
        "a later contour dispatch changed an already-recorded A command buffer"
    );
    assert_ne!(
        baseline_a,
        pixels_c.unwrap(),
        "the delayed-submit oracle needs visibly different A and C frames"
    );
    renderer.end_gpu_frame();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("retired GPU work completes");
    settle_gpu_retirement(&mut renderer);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .retired_bytes_of(GpuResourceKind::ContourScratch),
        0
    );

    let explicit_anchor = crate::data_config::ContourLabelAnchor {
        level_index: 0,
        x: 0.5,
        y: 0.5,
        tx: 1.0,
        ty: -1.0,
    };
    let explicit = state_test_labelled_contour(70.0, vec![explicit_anchor]);
    let explicit_style = renderer.create_style_for_series(&explicit).unwrap();
    let explicit_series = [Series {
        config: &explicit,
        style: &explicit_style,
    }];
    let explicit_a = [ChartDrawItem {
        view: &view_a,
        chart_config: chart_a.config(),
        series: &explicit_series,
    }];
    let explicit_b = [ChartDrawItem {
        view: &view_b,
        chart_config: chart_b.config(),
        series: &explicit_series,
    }];
    let first = renderer.prepare(&explicit_a).unwrap();
    let second = renderer.prepare(&explicit_b).unwrap();
    assert_eq!(
        placement_of(&first),
        placement_of(&second),
        "explicit placement must remain one immutable shared snapshot"
    );
    assert_eq!(
        renderer.contour_cache["label-cache"]
            .contour()
            .unwrap()
            .label
            .as_ref()
            .unwrap()
            .automatic_result_count(),
        0
    );
}

#[test]
fn automatic_contour_label_indirect_count_is_exactly_1024() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_contour_grid(&mut renderer);
    let chart = state_test_contour_chart(
        Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        },
        (0.0, 1.0),
    );
    let mut config = state_test_labelled_contour(2000.0, Vec::new());
    let DataRenderType::Contour { contour, .. } = &mut config.render_type else {
        unreachable!();
    };
    contour.levels = vec![2.0; crate::data_config::MAX_CONTOUR_LEVELS];
    let style = renderer.create_style_for_series(&config).unwrap();
    let series = [Series {
        config: &config,
        style: &style,
    }];
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];

    let prepared = renderer.prepare(&items).unwrap();
    let count = pollster::block_on(renderer.contour_label_instance_count_for_test(&prepared, 0, 0))
        .unwrap()
        .expect("automatic contour label snapshot");
    assert_eq!(count, 1024, "native indirect label count must be exact");
}

#[test]
fn contour_label_snapshot_survives_cache_replacement_until_token_drop() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_contour_grid(&mut renderer);
    let chart = state_test_contour_chart(
        Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        },
        (0.0, 1.0),
    );
    let old_config = state_test_labelled_contour(90.0, Vec::new());
    let old_style = renderer.create_style_for_series(&old_config).unwrap();
    let old_series = [Series {
        config: &old_config,
        style: &old_style,
    }];
    let old_view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let old_items = [ChartDrawItem {
        view: &old_view,
        chart_config: chart.config(),
        series: &old_series,
    }];
    let placement_of = |prepared: &PreparedFrame| {
        prepared.items[0].series[0]
            .contour
            .as_ref()
            .and_then(|contour| contour.label.as_ref())
            .expect("labelled contour snapshot")
            .snapshot
            .placement_identity()
    };

    let old_prepared = renderer.prepare(&old_items).unwrap();
    let old_atlas = renderer.contour_cache["label-cache"]
        .contour()
        .unwrap()
        .label
        .as_ref()
        .unwrap()
        .atlas_identity();
    let old_placement = placement_of(&old_prepared);
    let old_live = renderer
        .gpu_memory_usage()
        .live_bytes_of(GpuResourceKind::ContourScratch);
    assert!(old_live > 0, "old atlas and placement slot must be charged");

    let mut replacement_config = state_test_labelled_contour(90.0, Vec::new());
    state_test_contour_labels_mut(&mut replacement_config).font_size = 20.0;
    let replacement_style = renderer
        .create_style_for_series(&replacement_config)
        .unwrap();
    let replacement_series = [Series {
        config: &replacement_config,
        style: &replacement_style,
    }];
    let replacement_view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let replacement_items = [ChartDrawItem {
        view: &replacement_view,
        chart_config: chart.config(),
        series: &replacement_series,
    }];
    let replacement_prepared = renderer.prepare(&replacement_items).unwrap();
    let replacement_atlas = renderer.contour_cache["label-cache"]
        .contour()
        .unwrap()
        .label
        .as_ref()
        .unwrap()
        .atlas_identity();
    let replacement_placement = placement_of(&replacement_prepared);
    assert_ne!(old_atlas, replacement_atlas, "cache kept the old atlas");
    assert_ne!(
        old_placement, replacement_placement,
        "replacement reused the live token's placement slot"
    );
    assert!(renderer.validate_prepared(&old_prepared).is_ok());
    assert!(renderer.validate_prepared(&replacement_prepared).is_ok());

    let combined_live = renderer
        .gpu_memory_usage()
        .live_bytes_of(GpuResourceKind::ContourScratch);
    assert!(
        combined_live > old_live,
        "replacement must add a separately charged atlas and placement"
    );
    drop(old_prepared);
    let after_old_drop = renderer.gpu_memory_usage();
    let replacement_live = after_old_drop.live_bytes_of(GpuResourceKind::ContourScratch);
    assert_eq!(
        combined_live,
        old_live + replacement_live,
        "the old token's exact charge must remain live across replacement"
    );
    assert_eq!(
        after_old_drop.retired_bytes_of(GpuResourceKind::ContourScratch),
        old_live,
        "dropping the old token must retire its atlas and placement charge"
    );

    drop(replacement_prepared);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::ContourScratch),
        replacement_live,
        "the replacement cache must retain its atlas and placement charge"
    );
    renderer.contour_cache.clear();
    let retired = renderer.gpu_memory_usage();
    assert_eq!(retired.live_bytes_of(GpuResourceKind::ContourScratch), 0);
    assert_eq!(
        retired.retired_bytes_of(GpuResourceKind::ContourScratch),
        combined_live,
        "cache removal must retire the replacement charge too"
    );
    renderer.end_gpu_frame();
    // The completion callback can run on another test's polling thread
    // on the shared device, so wait for it rather than for one poll.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while renderer
        .gpu_memory_usage()
        .bytes_of(GpuResourceKind::ContourScratch)
        != 0
    {
        renderer.service_gpu_completions().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "retirement callback timeout"
        );
        std::thread::yield_now();
    }
}

#[test]
fn multi_panel_contour_labels_survive_interleaved_prepare_and_repeat_paint() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_contour_grid(&mut renderer);
    let (panel_w, panel_h) = (320, 240);
    let target_w = panel_w * 2;
    let chart_a = state_test_contour_chart(
        Rect {
            x: 0,
            y: 0,
            width: panel_w,
            height: panel_h,
        },
        (0.0, 1.0),
    );
    let chart_b = state_test_contour_chart(
        Rect {
            x: panel_w,
            y: 0,
            width: panel_w,
            height: panel_h,
        },
        (-0.5, 1.5),
    );
    let chart_c = state_test_contour_chart(
        Rect {
            x: 0,
            y: 0,
            width: panel_w,
            height: panel_h,
        },
        (0.2, 0.8),
    );
    let config_a = state_test_labelled_contour(70.0, Vec::new());
    let config_b = state_test_labelled_contour(420.0, Vec::new());
    let config_c = state_test_labelled_contour(140.0, Vec::new());
    let style_a = renderer.create_style_for_series(&config_a).unwrap();
    let style_b = renderer.create_style_for_series(&config_b).unwrap();
    let style_c = renderer.create_style_for_series(&config_c).unwrap();
    let series_a = [Series {
        config: &config_a,
        style: &style_a,
    }];
    let series_b = [Series {
        config: &config_b,
        style: &style_b,
    }];
    let series_c = [Series {
        config: &config_c,
        style: &style_c,
    }];
    let view_a = renderer
        .create_chart_view(&chart_a, chart_a.config().chart_area.0)
        .unwrap();
    let view_b = renderer
        .create_chart_view(&chart_b, chart_b.config().chart_area.0)
        .unwrap();
    let view_c = renderer
        .create_chart_view(&chart_c, chart_c.config().chart_area.0)
        .unwrap();
    let multi_items = [
        ChartDrawItem {
            view: &view_a,
            chart_config: chart_a.config(),
            series: &series_a,
        },
        ChartDrawItem {
            view: &view_b,
            chart_config: chart_b.config(),
            series: &series_b,
        },
    ];
    let interleaved_items = [ChartDrawItem {
        view: &view_c,
        chart_config: chart_c.config(),
        series: &series_c,
    }];

    let multi = renderer.prepare(&multi_items).unwrap();
    let interleaved = renderer.prepare(&interleaved_items).unwrap();
    assert!(renderer.validate_prepared(&multi).is_ok());
    let first = paint_prepared_rgba(&renderer, &multi, target_w, panel_h);
    let repeated = paint_prepared_rgba(&renderer, &multi, target_w, panel_h);
    assert_eq!(first, repeated, "repeat paint changed a prepared token");
    drop((interleaved, multi));

    let only_a = [ChartDrawItem {
        view: &view_a,
        chart_config: chart_a.config(),
        series: &series_a,
    }];
    let prepared_a = renderer.prepare(&only_a).unwrap();
    let baseline_a = paint_prepared_rgba(&renderer, &prepared_a, target_w, panel_h);
    drop(prepared_a);
    let only_b = [ChartDrawItem {
        view: &view_b,
        chart_config: chart_b.config(),
        series: &series_b,
    }];
    let prepared_b = renderer.prepare(&only_b).unwrap();
    let baseline_b = paint_prepared_rgba(&renderer, &prepared_b, target_w, panel_h);

    let crop = |rgba: &[u8], x: u32| {
        let mut out = Vec::with_capacity((panel_w * panel_h * 4) as usize);
        for row in 0..panel_h {
            let start = ((row * target_w + x) * 4) as usize;
            out.extend_from_slice(&rgba[start..start + (panel_w * 4) as usize]);
        }
        out
    };
    let multi_a = crop(&first, 0);
    let multi_b = crop(&first, panel_w);
    assert_eq!(multi_a, crop(&baseline_a, 0));
    assert_eq!(multi_b, crop(&baseline_b, panel_w));
    let magenta = |rgba: &[u8]| {
        rgba.chunks_exact(4)
            .filter(|p| p[0] > 150 && p[1] < 80 && p[2] > 150 && p[3] > 100)
            .count()
    };
    assert!(
        magenta(&multi_a) > 10,
        "left panel drew no label background"
    );
    assert!(
        magenta(&multi_b) > 10,
        "right panel drew no label background"
    );
    assert_ne!(
        multi_a, multi_b,
        "different transforms and spacing produced the same panel baseline"
    );
}

#[test]
fn stream_chunk_uses_shared_preparation_and_retains_work_charge() {
    use crate::streaming::{ColumnInput, ColumnRange, SourceEncoding};
    use crate::streaming_upload::{ChunkUploadBudget, record_chunk};
    let (device, queue) = crate::data_render::shared_device().expect("stream GPU required");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    let values = [0.1f32, 0.5, 0.9];
    for id in ["rx", "ry"] {
        r.add_column(
            id,
            &Column {
                data: values.to_vec(),
                min: 0.1,
                max: 0.9,
            },
        )
        .unwrap();
    }
    r.register_streamed_columns(
        ["sx", "sy"]
            .map(|id| crate::StreamColumn {
                id: id.into(),
                len: 3,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 1,
                statistics: crate::StreamStatistics::Unknown,
            })
            .to_vec(),
    )
    .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    let resident_series = SeriesConfig {
        series_id: "s".into(),
        source_id: None,
        label: None,
        x_column: "rx".into(),
        y_column: "ry".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::new(1.0, 0.0, 0.0, 0.5),
                line_width: 5.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    let mut stream_series = resident_series.clone();
    stream_series.x_column = "sx".into();
    stream_series.y_column = "sy".into();
    let resident_id = r
        .register_chart(chart.config().clone(), vec![resident_series])
        .unwrap();
    let streamed_id = r
        .register_chart(chart.config().clone(), vec![stream_series])
        .unwrap();
    let view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let mut frame = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id: resident_id,
            view: &view,
        }])
        .unwrap();
    let expected = paint_prepared_rgba(&r, &frame, 320, 240);
    let x = ColumnRange {
        column: 0,
        revision: 1,
        source_len: 3,
        offset: 0,
        len: 3,
        encoding: SourceEncoding::ScalarF32,
    };
    let y = ColumnRange { column: 1, ..x };
    let requested = [x, y];
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let chunk = record_chunk(
        &r.device,
        &mut encoder,
        &r.gpu_ledger,
        ChunkUploadBudget {
            max_columns: 2,
            max_input_bytes: 24,
            max_work_buffer_bytes: 48,
            max_upload_bytes: 96,
            renderer_budget_bytes: u64::MAX,
            pool_bytes: r.gpu_memory_usage().pool_bytes(),
        },
        &[
            ColumnInput {
                range: x,
                bytes: bytemuck::cast_slice(&values),
            },
            ColumnInput {
                range: y,
                bytes: bytemuck::cast_slice(&values),
            },
        ],
    )
    .unwrap();
    r.queue.submit([encoder.finish()]);
    let cfg = r.chart_series(streamed_id).unwrap()[0].clone();
    let style = r.create_style_for_series(&cfg).unwrap();
    let stream_packet = {
        let (states, preparation) = r.preparation_parts();
        let state = &states[&streamed_id];
        let series = Series {
            config: &state.series[0],
            style: &style,
        };
        assert!(
            preparation
                .build_stream_series(
                    &view,
                    &state.config,
                    &series,
                    &chunk,
                    &[
                        requested[0],
                        ColumnRange {
                            offset: 1,
                            len: 2,
                            ..requested[1]
                        },
                    ],
                    streaming_runtime::StreamDrawPhase::Line,
                )
                .is_err()
        );
        let mut dashed = state.series[0].clone();
        if let DataRenderType::Line { line } = &mut dashed.render_type {
            line.line_style = LineStylePreset::Dash;
        }
        assert!(
            preparation
                .build_stream_series(
                    &view,
                    &state.config,
                    &Series {
                        config: &dashed,
                        style: &style
                    },
                    &chunk,
                    &requested,
                    streaming_runtime::StreamDrawPhase::Line,
                )
                .is_err()
        );
        let mut sketch = state.config.clone();
        sketch.draw_style = DrawStyle::Sketch(Default::default());
        assert!(
            preparation
                .build_stream_series(
                    &view,
                    &sketch,
                    &series,
                    &chunk,
                    &requested,
                    streaming_runtime::StreamDrawPhase::Line,
                )
                .is_err()
        );
        preparation
            .build_stream_series(
                &view,
                &state.config,
                &series,
                &chunk,
                &requested,
                streaming_runtime::StreamDrawPhase::Line,
            )
            .unwrap()
    };
    frame.items[0].series = vec![stream_packet];
    frame.column_sources.clear();
    let state = &r.chart_states[&streamed_id];
    frame.registered_charts = vec![PreparedChartStamp {
        chart_id: streamed_id,
        desired: state.revisions.desired,
        data: state.revisions.data,
    }];
    // The prepared packet, not the temporary upload record, keeps work live.
    drop(chunk);
    r.end_gpu_frame();
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    // The shared device may dispatch completion callbacks on another
    // test's poll thread. Wait for retirement itself, not only queue work.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while r
        .gpu_memory_usage()
        .retired_bytes_of(crate::GpuResourceKind::StreamingUpload)
        != 0
    {
        assert!(
            std::time::Instant::now() < deadline,
            "upload retirement timed out"
        );
        r.service_gpu_completions().unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        r.gpu_memory_usage()
            .bytes_of(crate::GpuResourceKind::StreamingUpload),
        48
    );
    assert_eq!(paint_prepared_rgba(&r, &frame, 320, 240), expected);
    assert!(r.pool.slot("sx").is_none());
    let mut replacement = match r.logical_column("sx").unwrap() {
        crate::LogicalColumn::Streamed(source) => source.clone(),
        _ => unreachable!(),
    };
    replacement.revision = 2;
    r.replace_streamed_columns(vec![replacement]).unwrap();
    assert!(r.validate_prepared(&frame).is_err());
    drop(frame);
    r.end_gpu_frame();
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    // As above: the release can complete on another test's poll thread.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while r
        .gpu_memory_usage()
        .bytes_of(crate::GpuResourceKind::StreamingUpload)
        != 0
    {
        assert!(
            std::time::Instant::now() < deadline,
            "upload release timed out"
        );
        r.service_gpu_completions().unwrap();
        std::thread::yield_now();
    }
}

/// The token owns host inputs after prepare and rejects only renderer
/// resource changes that invalidate its captured GPU work.
#[test]
fn registered_prepare_preserves_chart_identity_and_ssot_styles() {
    let (device, queue) = crate::data_render::shared_device()
        .expect("registered preparation regression requires a GPU adapter");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        1024 * 1024,
    )
    .unwrap();
    for id in ["registered_x", "registered_y"] {
        r.add_column(id, &col_f64(vec![0.1, 0.5, 0.9])).unwrap();
    }
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    let series = SeriesConfig {
        series_id: "same-series-id".into(),
        source_id: None,
        label: None,
        x_column: "registered_x".into(),
        y_column: "registered_y".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                line_width: 5.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    // Equal configs, series ids and source columns cannot identify a chart.
    let a = r
        .register_chart(chart.config().clone(), vec![series.clone()])
        .unwrap();
    let b = r
        .register_chart(chart.config().clone(), vec![series.clone()])
        .unwrap();
    let view_a = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let view_b = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let view_b_only = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let both = r
        .prepare_registered(&[
            RegisteredChartDrawItem {
                chart_id: a,
                view: &view_a,
            },
            RegisteredChartDrawItem {
                chart_id: b,
                view: &view_b,
            },
        ])
        .unwrap();
    assert_eq!(both.registered_charts[0].chart_id, a);
    assert_eq!(both.registered_charts[1].chart_id, b);
    let before_a = paint_prepared_rgba(&r, &both, 320, 240);
    let b_only = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id: b,
            view: &view_b_only,
        }])
        .unwrap();
    let before_b = paint_prepared_rgba(&r, &b_only, 320, 240);
    let b_style_revision = r.chart_states[&b]
        .prepared_styles
        .as_ref()
        .unwrap()
        .series_revision;
    let b_style_buffer = r.chart_states[&b].prepared_styles.as_ref().unwrap().styles[0]
        .line_bg
        .clone();
    r.validate_prepared(&both).unwrap();

    let mut changed = series.clone();
    if let DataRenderType::Line { line } = &mut changed.render_type {
        line.line_color = Color::new(0.0, 1.0, 0.0, 1.0);
    }
    r.set_chart_series(a, vec![changed]).unwrap();
    assert!(matches!(
        r.validate_prepared(&both),
        Err(FiggyError::StalePreparedFrame { .. })
    ));
    r.validate_prepared(&b_only).unwrap();
    let next_a = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id: a,
            view: &view_a,
        }])
        .unwrap();
    let after_a = paint_prepared_rgba(&r, &next_a, 320, 240);
    assert_ne!(
        before_a, after_a,
        "registered style must be regenerated from SSOT"
    );
    let b_again = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id: b,
            view: &view_b,
        }])
        .unwrap();
    assert_eq!(before_b, paint_prepared_rgba(&r, &b_again, 320, 240));
    assert_eq!(before_b, paint_prepared_rgba(&r, &b_only, 320, 240));
    assert_eq!(
        r.chart_states[&b]
            .prepared_styles
            .as_ref()
            .unwrap()
            .series_revision,
        b_style_revision
    );
    assert_eq!(
        r.chart_states[&b].prepared_styles.as_ref().unwrap().styles[0].line_bg,
        b_style_buffer
    );

    let mut next_config = r.chart_config(a).unwrap().clone();
    next_config.bottom_x.max = 2.0;
    r.set_chart_config(a, next_config).unwrap();
    assert!(matches!(
        r.validate_prepared(&next_a),
        Err(FiggyError::StalePreparedFrame { .. })
    ));
    r.validate_prepared(&b_only).unwrap();
    let removal_token = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id: a,
            view: &view_a,
        }])
        .unwrap();
    r.remove_chart(a).unwrap();
    assert!(matches!(
        r.validate_prepared(&removal_token),
        Err(FiggyError::StalePreparedFrame { .. })
    ));
    r.validate_prepared(&b_only).unwrap();
    assert_eq!(before_b, paint_prepared_rgba(&r, &b_only, 320, 240));

    let source_token = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id: b,
            view: &view_b_only,
        }])
        .unwrap();
    r.upsert_column("registered_y", &col_f64(vec![0.9, 0.5, 0.1]))
        .unwrap();
    assert!(matches!(
        r.validate_prepared(&source_token),
        Err(FiggyError::StalePreparedFrame { .. })
    ));
}

#[test]
fn registered_prepare_preflight_failure_preserves_live_token() {
    let (device, queue) = crate::data_render::shared_device()
        .expect("registered preparation regression requires a GPU adapter");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        1024 * 1024,
    )
    .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let chart = Chart::new(config);
    let a = r.register_chart(chart.config().clone(), vec![]).unwrap();
    let removed = r.register_chart(chart.config().clone(), vec![]).unwrap();
    r.remove_chart(removed).unwrap();
    let view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let token = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id: a,
            view: &view,
        }])
        .unwrap();
    let unprepared = r.register_chart(chart.config().clone(), vec![]).unwrap();
    let stamp = r.chart_render_stamp(a).unwrap();
    let revision = view
        .content_revision
        .load(std::sync::atomic::Ordering::Acquire);
    assert!(matches!(
        r.prepare_registered(&[
            RegisteredChartDrawItem {
                chart_id: a,
                view: &view
            },
            RegisteredChartDrawItem {
                chart_id: unprepared,
                view: &view
            },
            RegisteredChartDrawItem {
                chart_id: removed,
                view: &view
            },
        ]),
        Err(FiggyError::UnknownChart { .. })
    ));
    assert!(r.chart_states[&unprepared].prepared_styles.is_none());
    assert_eq!(r.chart_render_stamp(a).unwrap(), stamp);
    assert_eq!(
        view.content_revision
            .load(std::sync::atomic::Ordering::Acquire),
        revision
    );
    r.validate_prepared(&token).unwrap();
    let mut invalid = r.chart_config(a).unwrap().clone();
    invalid.bottom_x.min = f64::NAN;
    assert!(r.set_chart_config(a, invalid).is_err());
    r.validate_prepared(&token).unwrap();
    paint_prepared_rgba(&r, &token, 320, 240);
}

#[test]
fn prepared_frame_owns_inputs_and_rejects_resource_staleness() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    r.add_column("st_x", &col_f64(vec![0.0, 0.5, 1.0])).unwrap();
    r.add_column("st_y", &col_f64(vec![0.0, 1.0, 0.25]))
        .unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    config.legend.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);

    let mk_cfg = |id: &str| SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "st_x".into(),
        y_column: "st_y".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    let (cfg_a, cfg_b) = (mk_cfg("st_a"), mk_cfg("st_b"));
    let style = r.create_style_for_series(&cfg_a).unwrap();
    let view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let series_ab = [
        Series {
            config: &cfg_a,
            style: &style,
        },
        Series {
            config: &cfg_b,
            style: &style,
        },
    ];
    let prepared = {
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series_ab,
        }];
        let p = r.prepare(&items).unwrap();
        assert!(r.validate_prepared(&p).is_ok(), "fresh token validates");
        p
    };

    // (1) Reordered series → identity stamp.
    // (2) Chart config edited between prepare and paint → transform stamp.
    assert_eq!(prepared.items[0].series.len(), 2);
    chart.set_x_range(0.0, 2.0);
    assert!(r.validate_prepared(&prepared).is_ok());
    chart.set_x_range(0.0, 1.0);

    let mut foreign = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    foreign
        .add_column("st_x", &col_f64(vec![0.0, 0.5, 1.0]))
        .unwrap();
    foreign
        .add_column("st_y", &col_f64(vec![0.0, 1.0, 0.25]))
        .unwrap();
    assert!(matches!(
        foreign.validate_prepared(&prepared),
        Err(FiggyError::StalePreparedFrame { .. })
    ));

    let mut view_reuse = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let first_view_token = {
        let items = [ChartDrawItem {
            view: &view_reuse,
            chart_config: chart.config(),
            series: &series_ab,
        }];
        r.prepare(&items).unwrap()
    };
    let second_view_token = {
        let items = [ChartDrawItem {
            view: &view_reuse,
            chart_config: chart.config(),
            series: &series_ab,
        }];
        r.prepare(&items).unwrap()
    };
    assert!(matches!(
        r.validate_prepared(&first_view_token),
        Err(FiggyError::StalePreparedFrame { .. })
    ));
    assert!(r.validate_prepared(&second_view_token).is_ok());
    r.refresh_axis(&mut view_reuse, &chart, chart.config().chart_area.0)
        .unwrap();
    assert!(matches!(
        r.validate_prepared(&second_view_token),
        Err(FiggyError::StalePreparedFrame { .. })
    ));
    view_reuse
        .content_revision
        .store(u64::MAX, std::sync::atomic::Ordering::Release);
    assert!(matches!(
        r.update_transform(&view_reuse, &chart),
        Err(FiggyError::CounterExhausted {
            counter: "chart view content revision"
        })
    ));

    // (3) Remove + re-add an arc source at the same physical layout.
    // Layout generation stays put, so the source allocation epoch must
    // invalidate the retained arc snapshot.
    let cfg_dash = SeriesConfig {
        series_id: "st_dash".into(),
        source_id: None,
        label: None,
        x_column: "st_x".into(),
        y_column: "st_y".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Dash,
                line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    let style_dash = r.create_style_for_series(&cfg_dash).unwrap();
    let series_dash = [Series {
        config: &cfg_dash,
        style: &style_dash,
    }];
    r.add_column("st_unrelated", &col_f64(vec![9.0])).unwrap();
    let prepared_dash = {
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series_dash,
        }];
        r.prepare(&items).unwrap()
    };
    assert!(r.remove_column("st_unrelated").unwrap());
    assert!(r.validate_prepared(&prepared_dash).is_ok());
    let old_y_handle = r.handle_for("st_y").unwrap();
    let old_y_epoch = r.pool().allocation_epoch("st_y").unwrap();
    assert!(r.remove_column("st_y").unwrap());
    let new_y_handle = r
        .add_column("st_y", &col_f64(vec![0.0, 1.0, 0.25]))
        .unwrap();
    assert_eq!(new_y_handle.offset, old_y_handle.offset);
    assert_eq!(new_y_handle.byte_size, old_y_handle.byte_size);
    assert_eq!(new_y_handle.len_values, old_y_handle.len_values);
    assert_ne!(r.pool().allocation_epoch("st_y"), Some(old_y_epoch));
    assert!(matches!(
        r.validate_prepared(&prepared_dash),
        Err(FiggyError::StalePreparedFrame { .. })
    ));
    assert!(matches!(
        r.validate_prepared(&prepared),
        Err(FiggyError::StalePreparedFrame { .. })
    ));
    let prepared_target = {
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series_ab,
        }];
        r.prepare(&items).unwrap()
    };

    // (4) Target-format switch rebuilds the pipeline cache → format stamp.
    assert!(
        r.ensure_target_format(wgpu::TextureFormat::Rgba8Unorm)
            .unwrap()
    );
    assert!(
        r.ensure_target_format(wgpu::TextureFormat::Bgra8Unorm)
            .unwrap()
    );
    assert!(matches!(
        r.validate_prepared(&prepared_target),
        Err(FiggyError::StalePreparedFrame { .. })
    ));
}

/// End-to-end split under the adversarial host schedule: record A, drop its
/// token without submitting, dispatch and draw a different B transform,
/// then submit A. The recorded arc/star resources must remain A's exact
/// immutable result.
#[test]
fn recorded_arc_star_frame_survives_later_compute_before_submit() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();
    r.add_column("mw_x", &col_f64(vec![0.0, 0.25, 0.5, 0.75, 1.0]))
        .unwrap();
    r.add_column("mw_y", &col_f64(vec![0.2, 0.75, 0.35, 0.9, 0.5]))
        .unwrap();

    let (w, h) = (320u32, 240u32);
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    });
    config.legend.visible = false;
    config.draw_style =
        crate::config::DrawStyle::Milkyway(crate::config::MilkywayOptions::default());
    let mut chart_a = Chart::new(config.clone());
    chart_a.set_x_range(-0.05, 1.05);
    chart_a.set_y_range(0.0, 1.0);
    let mut chart_b = Chart::new(config);
    chart_b.set_x_range(-1.0, 2.0);
    chart_b.set_y_range(-0.5, 1.5);

    let series_cfg = SeriesConfig {
        series_id: "mw".into(),
        source_id: None,
        label: None,
        x_column: "mw_x".into(),
        y_column: "mw_y".into(),
        z_column: None,
        render_type: DataRenderType::ScatterLine {
            scatter: test_scatter_style(),
            line: DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::new(0.4, 0.7, 1.0, 1.0),
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    let style = r.create_style_for_series(&series_cfg).unwrap();
    let series = [Series {
        config: &series_cfg,
        style: &style,
    }];
    let view_a = r
        .create_chart_view(&chart_a, chart_a.config().chart_area.0)
        .unwrap();
    let view_b = r
        .create_chart_view(&chart_b, chart_b.config().chart_area.0)
        .unwrap();
    let items_a = [ChartDrawItem {
        view: &view_a,
        chart_config: chart_a.config(),
        series: &series,
    }];
    let items_b = [ChartDrawItem {
        view: &view_b,
        chart_config: chart_b.config(),
        series: &series,
    }];

    let baseline_a = {
        let prepared = r.prepare(&items_a).unwrap();
        assert!(
            prepared.items[0].series[0].line_extra.is_some(),
            "milkyway line series must carry a star-pass snapshot"
        );
        paint_prepared_rgba(&r, &prepared, w, h)
    };
    let delayed_a = {
        let prepared = r.prepare(&items_a).unwrap();
        record_prepared_rgba(&r, &prepared, w, h)
    };
    let pixels_b = {
        let prepared = r.prepare(&items_b).unwrap();
        paint_prepared_rgba(&r, &prepared, w, h)
    };
    for key in 0..8 {
        let mut chart = Chart::new(chart_a.config().clone());
        chart.set_x_range(-2.0 - f64::from(key) * 0.25, 2.5 + f64::from(key) * 0.5);
        chart.set_y_range(-1.0 - f64::from(key) * 0.125, 1.75 + f64::from(key) * 0.25);
        let view = r
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        let prepared = r.prepare(&items).unwrap();
        let _ = paint_prepared_rgba(&r, &prepared, w, h);
    }
    assert_eq!(
        r.arc_cache.get("mw").map(Vec::len),
        Some(ARC_RESULT_CACHE_LIMIT_PER_SERIES)
    );
    assert!(
        r.gpu_memory_usage()
            .retired_bytes_of(GpuResourceKind::ArcScan)
            > 0,
        "evicted recorded arc resources must remain budgeted before submission"
    );
    let delayed_pixels_a = submit_recorded_rgba(&r, delayed_a);
    assert_eq!(
        delayed_pixels_a, baseline_a,
        "a later arc/star dispatch changed an already-recorded A command buffer"
    );
    assert_ne!(
        baseline_a, pixels_b,
        "the delayed-submit oracle needs visibly different A and B frames"
    );
    let lit = delayed_pixels_a
        .chunks_exact(4)
        .filter(|pixel| pixel[3] > 0)
        .count();
    assert!(
        lit > 100,
        "delayed milkyway frame produced too little ink: {lit}"
    );
    r.end_gpu_frame();
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("retired GPU work completes");
    settle_gpu_retirement(&mut r);
    assert_eq!(
        r.gpu_memory_usage()
            .retired_bytes_of(GpuResourceKind::ArcScan),
        0
    );
}

/// Log-axis auto-fit with zeros in the data must clamp the lower bound
/// to the smallest POSITIVE uploaded value instead of
/// feeding log10 a non-positive min.
#[test]
fn log_auto_fit_clamps_to_smallest_positive_uploaded_value() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    // Scalar stats follow the recorded (hi, lo) pair, not the source f64
    // (since #107 a `Column<f64>` uploads its split, not an f32 cast).
    let vals = vec![0.0, 0.04, 1.0, 50.0, 1000.0];
    r.add_column("v", &col_f64(vals)).unwrap();

    let mut config = crate::default::default_config();
    config.left_y.scale = crate::config::AxisScale::Logarithmic;
    let mut chart = Chart::new(config);
    chart.auto_fit_y_union(r.pool(), &["v"], 0.0).unwrap();

    let min = chart.config().left_y.min;
    let (hi, lo) = crate::data::split_f64_to_f32_pair(0.04);
    let uploaded_min = f64::from(hi) + f64::from(lo);
    // The log range round trip may move the last ulps; the f32 cast this
    // replaced sits about 1e-9 away, far outside that.
    assert!(
        (min - uploaded_min).abs() <= uploaded_min * 1e-14,
        "{min} vs the uploaded {uploaded_min}"
    );
    assert_ne!(uploaded_min, 0.04_f64);
    assert!((uploaded_min - 0.04).abs() < (f64::from(0.04_f64 as f32) - 0.04).abs());
    assert!(chart.config().left_y.max >= 1000.0);
}

/// `auto_fit_*` must leave a UNIFORM margin fraction on all four sides —
/// measured in pixels, from the user's point of view. Data is a triangle
/// wave touching its exact min/max so the ink bbox equals the data bbox.
#[test]
fn auto_fit_margins_are_uniform_on_all_sides() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();

    let n = 400;
    let xs: Vec<f64> = (0..n).map(|i| i as f64).collect();
    // Triangle wave between exactly 10 and 90.
    let ys: Vec<f64> = (0..n)
        .map(|i| {
            let t = (i % 80) as f64 / 80.0;
            let tri = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
            10.0 + tri * 80.0
        })
        .collect();
    r.add_column("x", &col_f64(xs)).unwrap();
    r.add_column("y", &col_f64(ys)).unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    });
    config.legend.visible = false;
    // Bare data area: no grid/tick ink inside to pollute the bbox.
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    let mut chart = Chart::new(config);
    let pad = 0.05;
    chart.auto_fit_x_union(r.pool(), &["x"], pad).unwrap();
    chart.auto_fit_y_union(r.pool(), &["y"], pad).unwrap();

    let series = [SeriesConfig {
        series_id: "s".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }];

    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let (w, h) = (img.width as usize, img.height as usize);
    let da = chart.config().data_area().unwrap().0;

    let (mut min_x, mut max_x, mut min_y, mut max_y) = (usize::MAX, 0usize, usize::MAX, 0usize);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let p = &img.rgba[i..i + 4];
            if p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90 {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    assert!(min_x < max_x, "no data ink found");

    // Expected margin fraction of the data_area extent on every side:
    // pad / (1 + 2·pad) of the full padded span.
    let expect = pad / (1.0 + 2.0 * pad);
    let close = |frac: f64, side: &str| {
        assert!(
            (frac - expect).abs() < 0.015,
            "{side} margin {frac:.3}, expected {expect:.3}"
        );
    };
    let daw = da.width as f64;
    let dah = da.height as f64;
    close((min_x as f64 - da.x as f64) / daw, "left");
    close((da.x as f64 + daw - 1.0 - max_x as f64) / daw, "right");
    close((min_y as f64 - da.y as f64) / dah, "top");
    close((da.y as f64 + dah - 1.0 - max_y as f64) / dah, "bottom");
}

/// Hide the four axis lines for a data-layer pixel test.
///
/// The decoration layer is composited **over** the data by design, so an axis
/// line sitting on the data area's boundary legitimately covers the first or
/// last column of whatever the data layer drew there. A continuity assertion
/// that scans every column between the first and last inked one would then
/// report that covered column as a break in the *curve*, which is not what it
/// is measuring.
///
/// This is not a tolerance: the axis chrome is simply not part of the question
/// "does the stroke ever break". Whether it lands opaque on that column or
/// leaks a little colour through its antialiasing is a rasterizer detail — on
/// llvmpipe it lands opaque, which is what made these two tests look
/// environment-dependent.
fn hide_axis_lines(config: &mut crate::Config) {
    config.bottom_x.line_visible = false;
    config.top_x.line_visible = false;
    config.left_y.line_visible = false;
    config.right_y.line_visible = false;
}

/// Offscreen endpoints still contribute their connecting segment on the
/// first draw, including after a very deep zoom or a small axis change.
#[test]
fn precise_zoom_keeps_connections_to_distant_endpoints() {
    let (device, queue) = crate::data_render::shared_device().expect("GPU required");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 800,
        height: 400,
    });
    config.legend.visible = false;
    hide_axis_lines(&mut config);
    let series = [SeriesConfig {
        series_id: "crossing".into(),
        source_id: None,
        label: None,
        x_column: "zx".into(),
        y_column: "zy".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_width: 2.0,
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }];
    for vertical in [false, true] {
        for reverse in [false, true] {
            for span in [2.0, 1e4, 1e6, 1e9] {
                let mut along = vec![-span, 0.5, span];
                let mut across = vec![0.1, 0.5, 0.9];
                if reverse {
                    along.reverse();
                    across.reverse();
                }
                let (x, y) = if vertical {
                    (across, along)
                } else {
                    (along, across)
                };
                let _ = r.remove_column("zx");
                let _ = r.remove_column("zy");
                r.add_column("zx", &col_f64(x)).unwrap();
                r.add_column("zy", &col_f64(y)).unwrap();
                for shift in [0.0, 0.001, 0.0] {
                    let mut chart = Chart::new(config.clone());
                    chart.set_x_range(shift, 1.0 + shift);
                    chart.set_y_range(shift, 1.0 + shift);
                    let image = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
                    let area = chart.config().data_area().unwrap().0;
                    let red = |x: u32, y: u32| {
                        let p = &image.rgba[((y * image.width + x) * 4) as usize..][..4];
                        p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
                    };
                    for fraction in [0.05, 0.25, 0.5, 0.75, 0.95] {
                        let present = if vertical {
                            let y = area.y + (area.height as f32 * fraction) as u32;
                            (area.x..area.x + area.width).any(|x| red(x, y))
                        } else {
                            let x = area.x + (area.width as f32 * fraction) as u32;
                            (area.y..area.y + area.height).any(|y| red(x, y))
                        };
                        assert!(
                            present,
                            "missing connection: vertical={vertical} reverse={reverse} span={span} shift={shift} fraction={fraction}"
                        );
                    }
                }
            }
        }
    }
}

/// A solid line stays CONTINUOUS no matter how dense the data is. With
/// sub-pixel segments (here ~0.04 px each) naive per-segment quads
/// degenerate into disconnected slivers — every x column the curve
/// crosses must contain ink.
///
/// Axis lines off ([`hide_axis_lines`]): the y-axis line covers the data
/// area's first column, and that is the decoration layer doing its job rather
/// than the stroke breaking.
#[test]
fn dense_line_has_no_gaps() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();

    let n = 50_000;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    // Decay curve like the report, plus measurement-style jitter: the
    // sub-pixel zig-zag flips the segment direction at every point,
    // which is what real noisy data does to the quad extrusion.
    let ys: Vec<f64> = xs
        .iter()
        .enumerate()
        .map(|(i, x)| (-6.0 * x).exp() + if i % 2 == 0 { 0.0008 } else { -0.0008 })
        .collect();
    r.add_column("dx", &col_f64(xs)).unwrap();
    r.add_column("dy", &col_f64(ys)).unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 800,
        height: 400,
    });
    config.legend.visible = false;
    hide_axis_lines(&mut config);
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(-0.05, 1.05);

    let series = [SeriesConfig {
        series_id: "dense".into(),
        source_id: None,
        label: None,
        x_column: "dx".into(),
        y_column: "dy".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                line_width: 1.5,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }];

    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let (w, h) = (img.width as usize, img.height as usize);
    let col_has_red = |x: usize| {
        (0..h).any(|y| {
            let i = (y * w + x) * 4;
            let p = &img.rgba[i..i + 4];
            p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
        })
    };
    let first = (0..w)
        .find(|&x| col_has_red(x))
        .expect("curve drew nothing");
    let last = (0..w).rev().find(|&x| col_has_red(x)).unwrap();
    let gaps: Vec<usize> = (first..=last).filter(|&x| !col_has_red(x)).collect();
    assert!(
        gaps.is_empty(),
        "dense solid line has {} gap columns between x={first} and x={last}: {:?}…",
        gaps.len(),
        &gaps[..gaps.len().min(20)]
    );

    // Continuity alone is not enough — a stippled line can still touch
    // every column. A 1.5 px stroke must average ≥ 1 ink pixel per
    // column along a near-horizontal curve.
    let red_per_col = |x: usize| {
        (0..h)
            .filter(|&y| {
                let i = (y * w + x) * 4;
                let p = &img.rgba[i..i + 4];
                p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
            })
            .count()
    };
    let total: usize = (first..=last).map(red_per_col).sum();
    let density = total as f64 / (last - first + 1) as f64;
    assert!(
        density >= 1.0,
        "dense solid line is stippled: {density:.2} ink px per column (expected ≥ 1.0)"
    );
}

/// Precise solid lines are rendered into a single-sample export target, so
/// smooth edges must come from the line shader itself. A shallow 1 px line
/// should therefore contain both strong stroke coverage and partial alpha
/// fringe pixels.
#[test]
fn solid_line_exports_analytic_aa_fringe() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();

    r.add_column("aa_x", &col_f64(vec![0.0, 10.0])).unwrap();
    r.add_column("aa_y", &col_f64(vec![4.8, 5.2])).unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 220,
        height: 90,
    });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(0.0, 10.0);

    let series = [SeriesConfig {
        series_id: "aa".into(),
        source_id: None,
        label: None,
        x_column: "aa_x".into(),
        y_column: "aa_y".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }];

    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let red_alphas: Vec<u8> = img
        .rgba
        .chunks_exact(4)
        .filter_map(|p| (p[0] > 200 && p[1] < 40 && p[2] < 40 && p[3] > 0).then_some(p[3]))
        .collect();

    assert!(!red_alphas.is_empty(), "line drew no red pixels");
    let max_alpha = red_alphas.iter().copied().max().unwrap_or(0);
    assert!(
        max_alpha >= 180,
        "line has no strong stroke coverage, max alpha {max_alpha}: {red_alphas:?}"
    );
    assert!(
        red_alphas.iter().any(|&a| (16..=239).contains(&a)),
        "line has no partial-alpha AA fringe pixels: {red_alphas:?}"
    );
}

/// The square-cap fix must NOT change line semantics: NaN values still
/// break the line (the caps may narrow a gap by ~line_width, never close
/// a real one), and nothing assumes monotonic x — a path that doubles
/// back stays continuous.
///
/// Axis lines off ([`hide_axis_lines`]) for the same reason as
/// `dense_line_has_no_gaps`: the y-axis line covers the data area's first
/// column, which would read as one pass of the doubled-back path missing.
#[test]
fn caps_preserve_nan_breaks_and_nonmonotonic_paths() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 800,
        height: 400,
    });
    config.legend.visible = false;
    hide_axis_lines(&mut config);
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(-0.1, 1.1);

    let line_cfg = |id: &str, x: &str, y: &str| SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                line_width: 1.5,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    };
    let red_cols = |img: &crate::RasterImage| -> Vec<usize> {
        let (w, h) = (img.width as usize, img.height as usize);
        (0..w)
            .filter(|&x| {
                (0..h).any(|y| {
                    let i = (y * w + x) * 4;
                    let p = &img.rgba[i..i + 4];
                    p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
                })
            })
            .collect()
    };

    // 1) Dense noisy line with a NaN band: x ∈ [0.40, 0.45] → ~36 px of
    //    data area. The break must survive (gap may shrink by ~1.5 px of
    //    caps, so probe the gap's middle).
    let n = 20_000;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    let ys: Vec<f64> = xs
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            if (0.40..0.45).contains(&x) {
                f64::NAN
            } else {
                0.5 + 0.3 * (x * 9.0).sin() + if i % 2 == 0 { 0.0008 } else { -0.0008 }
            }
        })
        .collect();
    r.add_column("gx", &col_f64(xs)).unwrap();
    r.add_column("gy", &col_f64(ys)).unwrap();

    let series = [line_cfg("gap", "gx", "gy")];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let cols = red_cols(&img);
    let da = chart.config().data_area().unwrap().0;
    let to_px = |v: f64| da.x as f64 + v * da.width as f64;
    let (gap_a, gap_b) = (to_px(0.40), to_px(0.45));
    let mid_a = (gap_a + 4.0) as usize;
    let mid_b = (gap_b - 4.0) as usize;
    let leaked: Vec<&usize> = cols.iter().filter(|&&x| x >= mid_a && x <= mid_b).collect();
    assert!(
        leaked.is_empty(),
        "NaN break was bridged: ink at columns {leaked:?} inside the gap ({mid_a}..{mid_b})"
    );
    // Both sides of the gap still drew.
    assert!(
        cols.iter().any(|&x| (x as f64) < gap_a - 2.0),
        "left of gap missing"
    );
    assert!(
        cols.iter().any(|&x| (x as f64) > gap_b + 2.0),
        "right of gap missing"
    );

    // 2) Non-monotonic path: x sweeps 0→1 then back 1→0 at a higher y.
    //    Every column the path covers must have ink in BOTH passes.
    let m = 30_000;
    let xs2: Vec<f64> = (0..m)
        .map(|i| {
            let t = i as f64 / (m - 1) as f64;
            if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }
        })
        .collect();
    let ys2: Vec<f64> = (0..m)
        .map(|i| {
            let t = i as f64 / (m - 1) as f64;
            let base = if t < 0.5 { 0.25 } else { 0.75 };
            base + if i % 2 == 0 { 0.0008 } else { -0.0008 }
        })
        .collect();
    r.add_column("mx", &col_f64(xs2)).unwrap();
    r.add_column("my", &col_f64(ys2)).unwrap();

    let series2 = [line_cfg("loop", "mx", "my")];
    let img2 = r.export_panel_rgba(&chart, &series2, 1.0).unwrap();
    let (w2, h2) = (img2.width as usize, img2.height as usize);
    let half = h2 / 2; // y=0.25 band renders below the midline, 0.75 above
    let band_has = |x: usize, top: bool| {
        let range = if top { 0..half } else { half..h2 };
        range.into_iter().any(|y| {
            let i = (y * w2 + x) * 4;
            let p = &img2.rgba[i..i + 4];
            p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
        })
    };
    let cols2 = red_cols(&img2);
    let (first2, last2) = (*cols2.first().unwrap(), *cols2.last().unwrap());
    let gaps2: Vec<usize> = (first2..=last2)
        .filter(|&x| !(band_has(x, true) && band_has(x, false)))
        .collect();
    assert!(
        gaps2.is_empty(),
        "non-monotonic path broke: {} columns missing one of the two passes: {:?}…",
        gaps2.len(),
        &gaps2[..gaps2.len().min(20)]
    );
}

/// Dash regularity on a CURVED line: the dash phase must advance with
/// arc length, so on/off run lengths along the curve stay close to the
/// pattern (Dash = [8, 4]). A phase that jumps at segment joints
/// (per-segment restart, or screen-position projection) shatters the
/// pattern into irregular fragments and fails the run statistics.
#[test]
fn dashed_curve_runs_match_pattern() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();

    // Worst case for joint phase errors: a SPARSE sine (large direction
    // change per segment) on a WIDE panel (fragments far from the origin
    // amplify any screen-position-derived phase).
    let n = 64;
    let period = 628.0_f64 / 100.0;
    let ts: Vec<f64> = (0..n).map(|i| i as f64 * period / (n - 1) as f64).collect();
    let vs: Vec<f64> = ts.iter().map(|t| t.sin()).collect();
    r.add_column("t", &col_f64(ts)).unwrap();
    r.add_column("v", &col_f64(vs)).unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 1000,
        height: 600,
    });
    // Hide chrome that could add red-ish AA pixels; keep it minimal.
    config.legend.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, period);
    chart.set_y_range(-1.2, 1.2);

    let series = [SeriesConfig {
        series_id: "rc".into(),
        source_id: None,
        label: None,
        x_column: "t".into(),
        y_column: "v".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Dash, // [8, 4]
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }];

    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let (w, h) = (img.width as usize, img.height as usize);
    let is_red = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        let p = &img.rgba[i..i + 4];
        // Dash run statistics should follow the solid core of the stroke,
        // not the intentionally-added analytic AA fringe.
        p[3] > 96 && p[0] > 120 && p[1] < 90 && p[2] < 90
    };

    // Trace the x-monotonic curve column by column: ink centroid y per
    // column (None = gap), arc step ds = √(1 + dy²) per 1-px x step.
    // On/off run lengths in ARC pixels must then match the [8, 4]
    // pattern regardless of local slope.
    let col_y: Vec<Option<f64>> = (0..w)
        .map(|x| {
            let ys: Vec<usize> = (0..h).filter(|&y| is_red(x, y)).collect();
            if ys.is_empty() {
                None
            } else {
                Some(ys.iter().sum::<usize>() as f64 / ys.len() as f64)
            }
        })
        .collect();
    let x0 = col_y
        .iter()
        .position(|c| c.is_some())
        .expect("dashed curve drew nothing");
    let x1 = col_y.iter().rposition(|c| c.is_some()).unwrap();

    let mut runs_on: Vec<f64> = Vec::new();
    let mut runs_off: Vec<f64> = Vec::new();
    let mut cur_on = true;
    let mut run = 0.0f64;
    let mut last_y = col_y[x0].unwrap();
    for value in &col_y[x0..=x1] {
        let (on, ds) = match *value {
            Some(y) => {
                let ds = (1.0 + (y - last_y).powi(2)).sqrt();
                last_y = y;
                (true, ds)
            }
            // In a gap the curve continues invisibly — approximate the
            // arc step with the local slope we last saw.
            None => (false, 1.0),
        };
        if on == cur_on {
            run += ds;
        } else {
            if cur_on {
                runs_on.push(run)
            } else {
                runs_off.push(run)
            }
            cur_on = on;
            run = ds;
        }
    }
    if cur_on {
        runs_on.push(run)
    } else {
        runs_off.push(run)
    }

    let median = |v: &mut Vec<f64>| -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if v.is_empty() { 0.0 } else { v[v.len() / 2] }
    };
    let needles = runs_on.iter().filter(|&&l| l <= 3.0).count();
    let n_on = runs_on.len();
    let med_on = median(&mut runs_on);
    let med_off = median(&mut runs_off);

    // Dash pattern is [8 on, 4 off] in arc px (line caps blur ±~2 px).
    assert!(
        (5.0..=12.0).contains(&med_on),
        "median on-run {med_on:.1} arc-px, expected ≈8 (runs: {runs_on:?})"
    );
    assert!(
        (2.0..=7.0).contains(&med_off),
        "median off-run {med_off:.1} arc-px, expected ≈4 (runs: {runs_off:?})"
    );
    assert!(
        needles * 5 <= n_on.max(1),
        "{needles}/{n_on} on-runs are needle fragments (≤3 arc-px): {runs_on:?}"
    );
}

/// The dash phase stays exact far along the arc (#107, SHADER_COMMON.md §5.6
/// arc pairs). A line first runs about 1.3e8 px off-screen, so every visible
/// segment starts past 2^26 px of arc, where an f32 arc lands on multiples
/// of 8 px. The visible run is horizontal, and every dash and gap along it
/// must keep the [8 on, 4 off] pattern.
#[test]
fn dashes_keep_their_pattern_past_two_to_the_twenty_six_pixels_of_arc() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();
    let tail = 65usize;
    let xs: Vec<f64> = [-200_000.0, -0.5]
        .into_iter()
        .chain((0..tail).map(|i| i as f64 / (tail - 1) as f64))
        .collect();
    let ys = vec![50.0; xs.len()];
    r.add_column("far_x", &col_f64(xs)).unwrap();
    r.add_column("far_y", &col_f64(ys)).unwrap();

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 800,
        height: 200,
    });
    config.legend.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 100.0);
    let series = [SeriesConfig {
        series_id: "far".into(),
        source_id: None,
        label: None,
        x_column: "far_x".into(),
        y_column: "far_y".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Dash, // [8, 4]
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let (w, h) = (img.width as usize, img.height as usize);
    let red = |x: usize, y: usize| {
        let p = &img.rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
        p[3] > 96 && p[0] > 120 && p[1] < 90 && p[2] < 90
    };
    // The row through the stroke's core.
    let row = (0..h)
        .max_by_key(|&y| (0..w).filter(|&x| red(x, y)).count())
        .unwrap();
    let on: Vec<bool> = (0..w).map(|x| red(x, row)).collect();
    let first = on
        .iter()
        .position(|&v| v)
        .expect("the dashed run drew nothing");
    let last = on.iter().rposition(|&v| v).unwrap();
    // Run lengths between the first and last ink, the outer ones dropped (the
    // plot edge and the joint at the left edge may cut them).
    let mut runs: Vec<(bool, usize)> = Vec::new();
    for &v in &on[first..=last] {
        match runs.last_mut() {
            Some((state, len)) if *state == v => *len += 1,
            _ => runs.push((v, 1)),
        }
    }
    let inner = &runs[4..runs.len().saturating_sub(2)];
    assert!(
        inner.len() > 80,
        "the run must cross many dashes ({} runs)",
        inner.len()
    );
    for &(state, len) in inner {
        let expected = if state { 6..=10 } else { 2..=6 };
        assert!(
            expected.contains(&len),
            "{} run of {len} px past 2^26 px of arc, expected {expected:?} (runs: {runs:?})",
            if state { "dash" } else { "gap" }
        );
    }
}

#[test]
fn renderer_init_and_add_column() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };

    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .unwrap();

    let c = col_f64((0..100).map(|i| i as f64).collect());
    let h = r.add_column("x", &c).unwrap();
    assert_eq!(h.len_values, 100);

    let h2 = r.handle_for("x").unwrap();
    assert!(r.is_valid_handle(&h2));

    // Unknown id → UnknownColumn.
    let res = r.handle_for("nope");
    assert!(matches!(res, Err(FiggyError::UnknownColumn { .. })));
}

// GPU memory accounting. These live in-crate so they can compare the
// report against the resources the renderer actually holds — a self-report
// is only worth what an independent recount proves.

fn panel_config(width: u32, height: u32) -> Config {
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width,
        height,
    });
    config
}

/// Independent recount of the pool row: the buffers the pool holds, read
/// back from wgpu rather than from any counter.
fn pool_bytes_held(renderer: &Renderer) -> u64 {
    renderer.pool.buffer().size()
        + renderer
            .pool
            .backup_buffer_for_test()
            .map_or(0, |backup| backup.size())
}

#[test]
fn pool_row_equals_the_buffers_the_pool_actually_holds() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let usage = renderer.gpu_memory_usage();
    assert_eq!(
        usage.pool_bytes(),
        pool_bytes_held(&renderer),
        "pool row disagrees with the buffers held\n{}",
        usage.report()
    );
    assert_eq!(usage.pool_bytes(), renderer.pool.capacity());

    // A defragmentation materializes the ping-pong backup, so the pool's
    // footprint becomes two buffers and the row must follow.
    let column = col_f64((0..64).map(|i| i as f64).collect());
    renderer.add_column("x", &column).unwrap();
    renderer.add_column("y", &column).unwrap();
    renderer.remove_column("x").unwrap();
    renderer.defragment().unwrap();

    let usage = renderer.gpu_memory_usage();
    assert_eq!(
        usage.live_bytes_of(GpuResourceKind::ColumnPool),
        pool_bytes_held(&renderer),
        "pool row missed the ping-pong backup\n{}",
        usage.report()
    );
    assert_eq!(
        usage.pool_bytes(),
        renderer.pool.capacity() * 2 + usage.retired_bytes_of(GpuResourceKind::ColumnPool)
    );
}

#[test]
fn data_upload_moves_only_the_pool_row() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let before = renderer.gpu_memory_usage();
    let column = col_f64((0..4096).map(|i| i as f64).collect());
    renderer.add_column("x", &column).unwrap();
    renderer.end_gpu_frame();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("retired GPU work completes");
    settle_gpu_retirement(&mut renderer);
    let after = renderer.gpu_memory_usage();

    assert_eq!(
        before.external_bytes(),
        after.external_bytes(),
        "uploading data must not change out-of-pool bytes\nbefore:\n{}after:\n{}",
        before.report(),
        after.report()
    );
    assert_eq!(
        before.pool_bytes(),
        after.pool_bytes(),
        "the slab was already allocated; an upload fills it, it does not grow it"
    );
}

#[test]
fn upload_peak_and_current_usage_include_submitted_staging() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let values = 4096usize;
    let column = col_f64((0..values).map(|i| i as f64).collect());
    renderer.pool.reset_peak_bytes();
    renderer.add_column("x", &column).unwrap();

    let staged = (values as u64) * crate::data::COLUMN_VALUE_BYTES as u64;
    let usage = renderer.gpu_memory_usage();
    assert_eq!(
        renderer.pool.peak_bytes(),
        usage.live_bytes_of(GpuResourceKind::ColumnPool) + staged,
        "the peak must include submitted staging"
    );
    assert_eq!(
        usage.retired_bytes_of(GpuResourceKind::ColumnPool),
        staged,
        "submitted staging remains budgeted until queue completion"
    );
}

#[test]
fn panel_textures_are_charged_at_exactly_two_rgba_layers() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let (w, h) = (320u32, 240u32);
    let chart = Chart::new(panel_config(w, h));
    let before = renderer.gpu_memory_usage();
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let after = renderer.gpu_memory_usage();

    let charged = after.bytes_of(GpuResourceKind::PanelTexture)
        - before.bytes_of(GpuResourceKind::PanelTexture);
    let held = crate::gpu_memory::texture_bytes(&view.grid_texture)
        + crate::gpu_memory::texture_bytes(&view.decoration_texture);
    assert_eq!(charged, held, "charge disagrees with the textures held");
    assert_eq!(
        charged,
        u64::from(w) * u64::from(h) * 4 * 2,
        "grid + decoration, RGBA8, one sample\n{}",
        after.report()
    );

    // Dropping the panel credits the bytes back — as retired first,
    // because wgpu releases them when the queue drains, not at the drop.
    drop(view);
    let retired = renderer.gpu_memory_usage();
    assert_eq!(
        retired.retired_bytes_of(GpuResourceKind::PanelTexture),
        held
    );
    assert_eq!(
        retired.bytes_of(GpuResourceKind::PanelTexture),
        before.bytes_of(GpuResourceKind::PanelTexture) + held,
        "released bytes stay in the total until the submission boundary"
    );
    renderer.end_gpu_frame();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("retired GPU work completes");
    settle_gpu_retirement(&mut renderer);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .bytes_of(GpuResourceKind::PanelTexture),
        before.bytes_of(GpuResourceKind::PanelTexture),
        "the submission boundary clears retirement"
    );
}

#[test]
fn idle_nonblocking_service_completes_retirement_without_another_submission() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let chart = Chart::new(panel_config(320, 240));
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    drop(view);
    assert!(renderer.gpu_memory_usage().retired_bytes() > 0);
    renderer.end_gpu_frame();

    // No further submission or boundary call: an idle host services only
    // nonblocking callbacks until the already submitted work completes.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        renderer
            .service_gpu_completions()
            .expect("device remains available");
        if renderer.gpu_memory_usage().retired_bytes() == 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "idle retirement did not complete"
        );
        std::thread::yield_now();
    }
    renderer
        .service_gpu_completions()
        .expect("empty poll is idempotent");
    assert_eq!(renderer.gpu_memory_usage().retired_bytes(), 0);
}

#[test]
fn pool_cleanup_releases_capacity_after_delete_and_keeps_survivor() {
    let Some(mut renderer) = state_test_renderer_with_capacity(1024 * 1024) else {
        return;
    };
    renderer
        .add_column("keep", &col_f64(vec![1.0, 2.0, 3.0]))
        .unwrap();
    renderer
        .add_column("drop", &col_f64(vec![4.0, 5.0, 6.0]))
        .unwrap();
    renderer.remove_column("drop").unwrap();
    let old_handle = renderer.handle_for("keep").unwrap();
    let before = renderer.gpu_memory_usage().total_bytes();

    pollster::block_on(renderer.release_unused_pool_memory(16 * 1024)).unwrap();

    assert_eq!(renderer.pool().capacity(), 16 * 1024);
    assert_eq!(renderer.pool().backup_bytes(), 0);
    assert_eq!(renderer.pool().slot("keep").unwrap().len_values, 3);
    assert!(!renderer.is_valid_handle(&old_handle));
    assert!(renderer.gpu_memory_usage().total_bytes() < before);
    assert_eq!(renderer.pool().retired_bytes(), 0);
}

#[test]
fn pool_cleanup_rebinds_active_picker_to_shrunk_primary() {
    let Some(mut renderer) = state_test_renderer_with_capacity(1024 * 1024) else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_scatter("scatter", None, "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let picker_generation = active_picker(&renderer).engine.registry_generation();

    pollster::block_on(renderer.release_unused_pool_memory(16 * 1024)).unwrap();

    assert_eq!(renderer.pool().capacity(), 16 * 1024);
    assert!(active_picker(&renderer).engine.registry_generation() > picker_generation);
    assert!(!renderer.has_pending_maintenance());
}

#[test]
fn pool_cleanup_picker_failure_preserves_original_pool() {
    let Some(mut renderer) = state_test_renderer_with_capacity(1024 * 1024) else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_scatter("scatter", None, "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let handle = renderer.handle_for("x").unwrap();
    active_picker(&renderer).engine.fail_next_registry_prepare();

    assert!(pollster::block_on(renderer.release_unused_pool_memory(16 * 1024)).is_err());

    assert_eq!(renderer.pool().capacity(), 1024 * 1024);
    assert!(renderer.is_valid_handle(&handle));
    assert_eq!(renderer.pool().slot("x").unwrap().len_values, 2);
    assert_eq!(renderer.pool().retired_bytes(), 0);
}

#[test]
fn prepared_view_keeps_panel_charges_after_chart_view_drop() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let chart = Chart::new(panel_config(320, 240));
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let panel_bytes = renderer
        .gpu_memory_usage()
        .live_bytes_of(GpuResourceKind::PanelTexture);
    let series: [Series<'_>; 0] = [];
    let prepared = {
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        renderer.prepare(&items).unwrap()
    };
    drop(view);

    let held = renderer.gpu_memory_usage();
    assert_eq!(
        held.live_bytes_of(GpuResourceKind::PanelTexture),
        panel_bytes,
        "the prepared bind groups and their accounting lifetime must agree"
    );
    assert_eq!(held.retired_bytes_of(GpuResourceKind::PanelTexture), 0);

    drop(prepared);
    let retired = renderer.gpu_memory_usage();
    assert_eq!(retired.live_bytes_of(GpuResourceKind::PanelTexture), 0);
    assert_eq!(
        retired.retired_bytes_of(GpuResourceKind::PanelTexture),
        panel_bytes
    );
    renderer.end_gpu_frame();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("retired GPU work completes");
    settle_gpu_retirement(&mut renderer);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .bytes_of(GpuResourceKind::PanelTexture),
        0
    );
}

#[test]
fn panel_resize_charges_the_replacement_before_releasing_the_old_one() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let mut chart = Chart::new(panel_config(320, 240));
    let mut view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let small = renderer
        .gpu_memory_usage()
        .bytes_of(GpuResourceKind::PanelTexture);
    assert_eq!(small, 320 * 240 * 4 * 2);

    let big_rect = Rect {
        x: 0,
        y: 0,
        width: 640,
        height: 480,
    };
    chart.config_mut().chart_area = crate::layout::ChartArea(big_rect);
    renderer.refresh_axis(&mut view, &chart, big_rect).unwrap();

    let during = renderer.gpu_memory_usage();
    let big = 640 * 480 * 4 * 2;
    assert_eq!(
        during.live_bytes_of(GpuResourceKind::PanelTexture),
        big,
        "the live rows are the new textures\n{}",
        during.report()
    );
    assert_eq!(
        during.bytes_of(GpuResourceKind::PanelTexture),
        big + small,
        "old and new coexist until the queue drains"
    );

    renderer.end_gpu_frame();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("retired GPU work completes");
    settle_gpu_retirement(&mut renderer);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .bytes_of(GpuResourceKind::PanelTexture),
        big
    );
}

/// The batch is the matrix upload path, so what it has to deliver is: every
/// declared column registered (or `validate_renderer_series` rejects the
/// grid), and one staging buffer for the lot instead of one per column.
#[test]
fn a_renderer_batch_registers_every_grid_column_at_one_buffer_each_batch() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let x = col_f64(vec![0.0, 1.0, 2.0]);
    let y = col_f64(vec![0.0, 1.0]);
    let z0 = col_f64(vec![1.0, 2.0]);
    let z1 = col_f64(vec![-3.0, 4.0]);

    let before = renderer.pool.buffer_creations();
    renderer
        .add_columns(&[
            ("x", &x as &dyn ColumnSource),
            ("y", &y as &dyn ColumnSource),
            ("z0", &z0 as &dyn ColumnSource),
            ("z1", &z1 as &dyn ColumnSource),
        ])
        .expect("batch upload");
    assert_eq!(
        renderer.pool.buffer_creations() - before,
        1,
        "four columns, one staging buffer"
    );

    for (id, len) in [("x", 3usize), ("y", 2), ("z0", 2), ("z1", 2)] {
        let slot = renderer.pool.slot(id).expect("batch column is registered");
        assert_eq!(slot.len_values, len, "{id} length");
    }
    assert_eq!(renderer.pool.slot("z1").unwrap().min, -3.0);
    assert_eq!(renderer.pool.slot("z1").unwrap().max, 4.0);

    // The point of registration: the grid declaration now validates.
    let mut config = crate::default::default_config();
    config.colorbar = Some(crate::default::default_colorbar_options());
    renderer
        .register_chart(
            config,
            vec![state_test_heatmap("field", "x", "y", &["z0", "z1"])],
        )
        .expect("a grid over batched columns is drawable");
}

/// An empty batch is a no-op, and a repeated id is refused with nothing
/// uploaded — the same two edges the single-column path has.
#[test]
fn a_renderer_batch_rejects_repeated_ids_and_accepts_an_empty_one() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let a = col_f64(vec![1.0, 2.0]);
    renderer.add_columns(&[]).expect("empty batch is a no-op");
    assert_eq!(renderer.pool.used_bytes(), 0);

    assert!(matches!(
        renderer.add_columns(&[
            ("dup", &a as &dyn ColumnSource),
            ("dup", &a as &dyn ColumnSource),
        ]),
        Err(FiggyError::Pool(data_render::AllocError::DuplicateId(_)))
    ));
    assert!(renderer.pool.slot("dup").is_none());
    assert_eq!(renderer.pool.used_bytes(), 0, "nothing was uploaded");

    renderer
        .add_columns(&[("live", &a as &dyn ColumnSource)])
        .expect("first insert");
    let after_first = renderer.pool.used_bytes();
    assert!(matches!(
        renderer.add_columns(&[("live", &a as &dyn ColumnSource)]),
        Err(FiggyError::Pool(data_render::AllocError::DuplicateId(_)))
    ));
    assert_eq!(
        renderer.pool.used_bytes(),
        after_first,
        "the second insert of a live id added nothing"
    );
}

/// The pool grows by default (#107 R8): a column far past the capacity the
/// renderer was created with registers without the host opting in, and the
/// values past the initial capacity draw.
#[test]
fn the_default_pool_grows_to_hold_and_draw_a_column_past_its_capacity() {
    let Some(mut r) = state_test_renderer_with_capacity(64 * 1024) else {
        return;
    };
    assert_eq!(
        r.pool_growth_policy(),
        data_render::GrowthPolicy::OnAllocFailure
    );
    // 100 000 values are 800 kB of pairs per column, twelve times the pool.
    let n = 100_000usize;
    r.add_column(
        "grow_x",
        &col_f64((0..n).map(|i| i as f64 / (n - 1) as f64).collect()),
    )
    .expect("the default pool grows to fit");
    r.add_column("grow_y", &col_f64(vec![50.0; n]))
        .expect("the default pool grows to fit");
    assert!(r.pool.capacity() >= 2 * 8 * n as u64, "the pool grew");

    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 200,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 100.0);
    let series = [SeriesConfig {
        series_id: "grow".into(),
        source_id: None,
        label: None,
        x_column: "grow_x".into(),
        y_column: "grow_y".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
        },
    }];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let width = img.width as usize;
    // Red ink in the right fifth: values from index 80 000 on, far past the
    // 8 192 the initial pool held.
    let red_right = img
        .rgba
        .chunks_exact(4)
        .enumerate()
        .filter(|(i, p)| i % width > width * 4 / 5 && p[0] > 200 && p[1] < 80 && p[2] < 80)
        .count();
    assert!(
        red_right > 20,
        "the tail of the grown column draws ({red_right} px)"
    );
}

/// A batch that has to grow the pool relocates every column, so the active
/// picker is rebound in the same call and the maintenance bit is cleared —
/// the same contract the single-column upsert has.
#[test]
fn a_renderer_batch_that_grows_the_pool_rebinds_the_active_picker() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    renderer.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_scatter("scatter", None, "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();

    let picker_generation = active_picker(&renderer).engine.registry_generation();
    let old_x = renderer.pool.handle_for("x").unwrap();
    let capacity_before = renderer.pool.capacity();
    let layout_before = renderer.pool.layout_generation();

    let big: Vec<f64> = (0..512).map(|i| i as f64).collect();
    let b0 = col_f64(big.clone());
    let b1 = col_f64(big);
    renderer
        .add_columns(&[
            ("b0", &b0 as &dyn ColumnSource),
            ("b1", &b1 as &dyn ColumnSource),
        ])
        .expect("the batch grew the pool to fit");

    assert!(
        renderer.pool.capacity() > capacity_before,
        "the batch was supposed to grow the pool"
    );
    assert!(renderer.pool.layout_generation() > layout_before);
    assert_ne!(
        renderer.pool.handle_for("x").unwrap().generation,
        old_x.generation,
        "growth republished every handle"
    );
    assert!(
        active_picker(&renderer).engine.registry_generation() > picker_generation,
        "the picker must be rebound against the new layout"
    );
    assert!(!renderer.has_pending_maintenance());
    for id in ["b0", "b1"] {
        assert_eq!(renderer.pool.slot(id).unwrap().len_values, 512, "{id}");
    }
}

/// The renderer half of the batch is fallible too. When the picker rebuild
/// fails, the guard drops: no column is published and the region goes back.
/// The relayout that made room is *not* undone — it is not part of the batch
/// — and the picker reports `StaleColumn` until it is re-prepared, which is
/// the same recovery a defragmentation asks for.
#[test]
fn a_renderer_batch_rolled_back_by_a_picker_failure_publishes_no_column() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    renderer.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_scatter("scatter", None, "x", "y")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();

    let picker_generation = active_picker(&renderer).engine.registry_generation();
    let used_before = renderer.pool.used_bytes();
    active_picker(&renderer).engine.fail_next_registry_prepare();

    let big: Vec<f64> = (0..512).map(|i| i as f64).collect();
    let b0 = col_f64(big.clone());
    let b1 = col_f64(big);
    assert!(matches!(
        renderer.add_columns(&[
            ("b0", &b0 as &dyn ColumnSource),
            ("b1", &b1 as &dyn ColumnSource),
        ]),
        Err(FiggyError::GpuPick(
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "injected registry prepare failure"
            }
        ))
    ));

    assert!(renderer.pool.slot("b0").is_none(), "b0 was published");
    assert!(renderer.pool.slot("b1").is_none(), "b1 was published");
    assert_eq!(
        renderer.pool.used_bytes(),
        used_before,
        "the batch region went back to the free list"
    );
    assert_eq!(
        renderer.pool.free_bytes(),
        renderer.pool.capacity() - used_before,
        "and it coalesced back into the rest of the free space"
    );
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        picker_generation,
        "the failed prepare must not publish a registry"
    );

    // Retrying now succeeds: the region is allocatable again.
    renderer
        .add_columns(&[
            ("b0", &b0 as &dyn ColumnSource),
            ("b1", &b1 as &dyn ColumnSource),
        ])
        .expect("the retry lands where the abandoned batch was");
    assert_eq!(renderer.pool.slot("b1").unwrap().len_values, 512);
}

/// The field's group-2 buffers are the grid `(base, len)` table, the levels,
/// the stops and `FieldParams`. At matrix scale the table alone is tens of
/// kilobytes, so a steady frame must rebuild **none** of it.
#[test]
fn a_steady_field_frame_rebuilds_no_group_two_buffers() {
    let Some((mut renderer, chart, series_cfg)) = field_prepare_fixture() else {
        return;
    };
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let style = renderer.create_style_for_series(&series_cfg).unwrap();
    let series = [Series {
        config: &series_cfg,
        style: &style,
    }];
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];

    let first = renderer.prepare(&items).unwrap();
    renderer.validate_prepared(&first).unwrap();
    let after_first = renderer
        .gpu_memory_usage()
        .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable);
    assert!(
        after_first > 0,
        "the first prepare must build the field tables"
    );
    let live = renderer
        .gpu_memory_usage()
        .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable);
    assert!(live > 0, "and charge them");
    assert_eq!(renderer.field_cache.len(), 1);

    let second = renderer.prepare(&items).unwrap();
    renderer.validate_prepared(&second).unwrap();
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable),
        after_first,
        "nothing changed, so nothing may be rebuilt"
    );
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable),
        live,
        "and the charge must not double"
    );
}

/// Replacing a grid column both stales the live token and rebuilds the
/// scratch on the next prepare. The token check is the point: the captured
/// `rows` came from that column's length, so drawing on with it would paint a
/// grid the data no longer has.
#[test]
fn a_replaced_grid_column_stales_the_token_and_rebuilds_the_scratch() {
    let Some((mut renderer, chart, series_cfg)) = field_prepare_fixture() else {
        return;
    };
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let style = renderer.create_style_for_series(&series_cfg).unwrap();
    let series = [Series {
        config: &series_cfg,
        style: &style,
    }];
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];

    let token = renderer.prepare(&items).unwrap();
    renderer.validate_prepared(&token).unwrap();
    let creations_before = renderer
        .gpu_memory_usage()
        .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable);

    // Same id, same length, new bytes: the layout does not move, so only the
    // per-column allocation epoch marks the change.
    renderer
        .upsert_column("fz0", &col_f64(vec![7.0, 8.0]))
        .unwrap();
    let stale = renderer
        .validate_prepared(&token)
        .expect_err("a replaced grid column must stale the token");
    match &stale {
        FiggyError::StalePreparedFrame { reason } => {
            assert!(
                reason.contains("grid column"),
                "expected the grid column to be named: {reason}"
            );
        }
        other => panic!("expected StalePreparedFrame, got {other:?}"),
    }

    let next = renderer.prepare(&items).unwrap();
    renderer.validate_prepared(&next).unwrap();
    assert!(
        renderer
            .gpu_memory_usage()
            .creations_of(crate::gpu_memory::GpuResourceKind::FieldTable)
            > creations_before,
        "the scratch must be rebuilt against the new epoch"
    );
}

/// A chart, a colourbar and a two-column grid, ready to prepare.
fn field_prepare_fixture() -> Option<(Renderer, Chart, SeriesConfig)> {
    let mut renderer = state_test_renderer()?;
    renderer.add_column("fx", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("fy", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer
        .add_column("fz0", &col_f64(vec![0.0, 1.0]))
        .unwrap();
    renderer
        .add_column("fz1", &col_f64(vec![2.0, 3.0]))
        .unwrap();
    let mut config = panel_config(320, 240);
    config.colorbar = Some(crate::default::default_colorbar_options());
    Some((
        renderer,
        Chart::new(config),
        state_test_heatmap("field", "fx", "fy", &["fz0", "fz1"]),
    ))
}

#[test]
fn a_growth_makes_a_live_prepared_frame_stale_and_the_next_one_valid() {
    // Small pool so one oversized upload has to relayout.
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    renderer.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
    renderer.add_column("lx", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.add_column("ly", &col_f64(vec![0.0, 1.0])).unwrap();
    let chart = Chart::new(panel_config(320, 240));
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let series_cfg = state_test_line("line", "lx", "ly");
    let style = renderer.create_style_for_series(&series_cfg).unwrap();
    let series = [Series {
        config: &series_cfg,
        style: &style,
    }];
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];
    let prepared = renderer.prepare(&items).unwrap();
    renderer
        .validate_prepared(&prepared)
        .expect("a fresh token is valid");
    let capacity_before = renderer.pool.capacity();

    // Force a growth: this relayouts the slab, which moves every column and
    // advances the pool's layout generation.
    let big: Vec<f64> = (0..2048).map(|i| i as f64).collect();
    renderer.add_column("big", &col_f64(big)).unwrap();
    assert!(
        renderer.pool.capacity() > capacity_before,
        "the upload was supposed to grow the pool"
    );

    // The token prepared against the old layout must be refused rather than
    // drawn: its vertex offsets point into a buffer that no longer exists.
    let stale = renderer
        .validate_prepared(&prepared)
        .expect_err("a token prepared before a growth must be stale");
    assert!(
        matches!(stale, FiggyError::StalePreparedFrame { .. }),
        "expected StalePreparedFrame, got {stale:?}"
    );

    // Recovery is a fresh prepare, exactly as it is after a defragmentation.
    let reprepared = renderer.prepare(&items).unwrap();
    renderer
        .validate_prepared(&reprepared)
        .expect("the next frame prepares normally after a growth");
}

#[test]
fn the_report_tracks_the_pool_across_a_growth() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    renderer.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
    renderer.add_column("lx", &col_f64(vec![0.0, 1.0])).unwrap();
    renderer.end_gpu_frame();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("retired GPU work completes");
    settle_gpu_retirement(&mut renderer);

    let before = renderer.gpu_memory_usage();
    assert_eq!(before.pool_bytes(), pool_bytes_held(&renderer));
    let capacity_before = renderer.pool.capacity();

    let big: Vec<f64> = (0..2048).map(|i| i as f64).collect();
    renderer.add_column("big", &col_f64(big)).unwrap();
    let capacity_after = renderer.pool.capacity();
    assert!(capacity_after > capacity_before, "growth expected");

    // Immediately after the growth the old slab is retired but not yet
    // counted as gone, so the row is new + old.
    let during = renderer.gpu_memory_usage();
    assert_eq!(
        during.live_bytes_of(GpuResourceKind::ColumnPool),
        pool_bytes_held(&renderer),
        "live pool bytes must equal the buffers held\n{}",
        during.report()
    );
    assert_eq!(
        during.retired_bytes_of(GpuResourceKind::ColumnPool),
        capacity_before + 2048 * crate::data::COLUMN_VALUE_BYTES as u64,
        "the old slab and submitted staging stay retired until completion\n{}",
        during.report()
    );
    assert!(
        during.peak_bytes() >= capacity_before + capacity_after,
        "the peak must remember that both slabs existed across the copy: \
         {} < {} + {}\n{}",
        during.peak_bytes(),
        capacity_before,
        capacity_after,
        during.report()
    );

    renderer.end_gpu_frame();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("retired GPU work completes");
    settle_gpu_retirement(&mut renderer);
    let after = renderer.gpu_memory_usage();
    assert_eq!(after.retired_bytes(), 0);
    assert_eq!(after.pool_bytes(), pool_bytes_held(&renderer));
    assert_eq!(after.pool_bytes(), capacity_after);
}

#[test]
fn an_undeclared_ceiling_carries_no_out_of_pool_total() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let chart = Chart::new(panel_config(320, 240));
    let _view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    renderer.end_gpu_frame();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("retired GPU work completes");
    settle_gpu_retirement(&mut renderer);

    // No ceiling: the context carries no budget at all, so there is no
    // out-of-pool total to compute and none to misread. `GpuBudget` fusing
    // the two is what makes that a type-level fact rather than a comment.
    let ctx = pool_alloc_ctx(
        renderer.device.as_ref(),
        renderer.queue.as_ref(),
        renderer.memory_budget,
        renderer.gpu_ledger.as_ref(),
    );
    assert!(
        ctx.budget.is_none(),
        "with no ceiling declared the pool must be handed no budget"
    );

    // With a ceiling, the total rides along and matches what the report
    // says is held outside the pool right now.
    let external = renderer.gpu_memory_usage().external_bytes();
    assert!(external > 0, "the panel textures should be charged by now");
    assert_eq!(renderer.set_memory_budget(Some(u64::MAX)), None);
    let ctx = pool_alloc_ctx(
        renderer.device.as_ref(),
        renderer.queue.as_ref(),
        renderer.memory_budget,
        renderer.gpu_ledger.as_ref(),
    );
    let budget = ctx.budget.expect("a declared ceiling reaches the pool");
    assert_eq!(budget.ceiling_bytes, u64::MAX);
    assert_eq!(
        budget.external_bytes, external,
        "the out-of-pool total must be the one read at this call"
    );
}

#[test]
fn lowering_the_ceiling_below_the_current_total_reports_it_at_the_call() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let chart = Chart::new(panel_config(320, 240));
    let _view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    renderer.end_gpu_frame();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("retired GPU work completes");
    settle_gpu_retirement(&mut renderer);
    let total = renderer.gpu_memory_usage().total_bytes();

    // A ceiling with room says nothing.
    assert_eq!(renderer.set_memory_budget(Some(total + 1)), None);

    // A ceiling already crossed is still applied — a host lowering it on
    // purpose, about to release something, gets what it asked for — but it
    // hears about the conflict here instead of at some later growth.
    assert_eq!(
        renderer.set_memory_budget(Some(total - 1)),
        Some(total),
        "the setter must report the total that already exceeds the ceiling"
    );
    assert_eq!(
        renderer.memory_budget(),
        Some(total - 1),
        "and it must still have been applied"
    );

    assert_eq!(renderer.set_memory_budget(None), None);
    assert_eq!(renderer.memory_budget(), None);
}

#[test]
fn resident_admission_counts_staging_derived_headroom_and_requires_a_budget() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let lengths = [1, 33]; // 256 + 512 bytes after per-column alignment.
    let derived = [128, 192];
    let request = ResidentAdmissionRequest {
        column_value_counts: &lengths,
        already_resident_working_set_bytes: 256,
        derived_buffer_sizes: &derived,
        transition_headroom_bytes: 128,
        working_set_limit_bytes: 1344,
    };
    let current = renderer.gpu_memory_usage().total_bytes();
    let unbounded = renderer.resident_admission(request);
    assert_eq!(unbounded.status, ResidentAdmissionStatus::MemoryBudgetUnset);
    assert_eq!(unbounded.resident_column_bytes, 768);
    assert_eq!(unbounded.derived_resident_bytes, 320);
    assert_eq!(unbounded.upload_staging_bytes, 768);
    assert_eq!(unbounded.resident_working_set_bytes, 1344);
    assert_eq!(unbounded.pool_transition_bytes, 0);
    assert_eq!(unbounded.transition_peak_bytes, current + 768 + 320 + 128);

    assert_eq!(
        renderer.set_memory_budget(Some(unbounded.transition_peak_bytes)),
        None
    );
    let accepted = renderer.resident_admission(request);
    assert!(accepted.is_admissible(), "{accepted:?}");

    assert_eq!(
        renderer.set_memory_budget(Some(accepted.transition_peak_bytes - 1)),
        None
    );
    assert_eq!(
        renderer.resident_admission(request).status,
        ResidentAdmissionStatus::MemoryBudgetExceeded
    );
    let capped = renderer.resident_admission(ResidentAdmissionRequest {
        working_set_limit_bytes: 1343,
        ..request
    });
    assert_eq!(
        capped.status,
        ResidentAdmissionStatus::WorkingSetLimitExceeded
    );
}

#[test]
fn streamed_promotion_commits_mixed_encodings_and_only_invalidates_dependents() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let _ = renderer.set_memory_budget(Some(u64::MAX));
    let scalar = col_f64(vec![-2.0, 0.5, 4.0]);
    let hilo = col_f64(vec![
        1_700_000_000_000.125,
        1_700_000_000_000.25,
        1_700_000_000_000.5,
    ]);
    renderer
        .register_streamed_columns(vec![
            crate::StreamColumn {
                id: "promote-x".into(),
                len: 3,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 7,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "promote-y".into(),
                len: 3,
                encoding: crate::StreamEncoding::HiLoF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 9,
                statistics: crate::StreamStatistics::Unknown,
            },
        ])
        .unwrap();
    let series = state_test_line("promoted", "promote-x", "promote-y");
    let dependent = renderer
        .register_chart(crate::default::default_config(), vec![series.clone()])
        .unwrap();
    let unrelated = renderer
        .register_chart(crate::default::default_config(), Vec::new())
        .unwrap();
    let dependent_before = revision_snapshot(&renderer, dependent);
    let unrelated_before = revision_snapshot(&renderer, unrelated);
    let fit_before = renderer.begin_fit_commit(dependent).unwrap();
    let sources = [
        crate::StreamSourceBinding {
            id: "promote-x",
            revision: 7,
            source: crate::data::StreamColumnSource::Scalar(&scalar),
        },
        crate::StreamSourceBinding {
            id: "promote-y",
            revision: 9,
            source: crate::data::StreamColumnSource::HiLo(&hilo),
        },
    ];

    let report = renderer
        .promote_streamed_columns_to_resident(&sources, 0, &[], 0, u64::MAX)
        .unwrap();
    assert!(report.is_admissible(), "{report:?}");
    assert!(matches!(
        renderer.logical_column("promote-x"),
        Some(crate::LogicalColumn::Resident(_))
    ));
    assert!(matches!(
        renderer.logical_column("promote-y"),
        Some(crate::LogicalColumn::Resident(_))
    ));
    assert!(!renderer.streaming_sources.contains_key("promote-x"));
    assert!(!renderer.streaming_sources.contains_key("promote-y"));
    assert_eq!(renderer.stream_revision_lineage["promote-x"], 7);
    assert_eq!(renderer.stream_revision_lineage["promote-y"], 9);
    assert_eq!(renderer.chart_states[&dependent].series, vec![series]);
    let dependent_after = revision_snapshot(&renderer, dependent);
    assert_ne!(dependent_after[0], dependent_before[0]);
    assert_ne!(dependent_after[3], dependent_before[3]);
    assert_eq!(dependent_after[1], dependent_before[1]);
    assert_eq!(dependent_after[2], dependent_before[2]);
    assert_eq!(revision_snapshot(&renderer, unrelated), unrelated_before);
    assert_eq!(renderer.pool.slot("promote-x").unwrap().min, -2.0);
    assert_eq!(
        renderer.pool.slot("promote-y").unwrap().min_positive,
        Some(1_700_000_000_000.125)
    );
    assert!(matches!(
        renderer.commit_auto_fit_all_if_current(
            &fit_before,
            &crate::chart::FitExtent::EMPTY,
            &crate::chart::FitExtent::EMPTY,
            0.0,
        ),
        Err(FiggyError::StaleStateToken { .. })
    ));
}

#[test]
fn streamed_promotion_validation_admission_and_source_failure_leave_stream_authority() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let source = FailingPromotionSource { len: 3 };
    renderer
        .register_streamed_columns(vec![crate::StreamColumn {
            id: "stay-streamed".into(),
            len: 3,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 4,
            statistics: crate::StreamStatistics::Unknown,
        }])
        .unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line(
                "streamed",
                "stay-streamed",
                "stay-streamed",
            )],
        )
        .unwrap();
    let revisions = revision_snapshot(&renderer, chart);
    let used = renderer.pool.used_bytes();
    let layout = renderer.pool.layout_generation();
    let stale = [crate::StreamSourceBinding {
        id: "stay-streamed",
        revision: 3,
        source: crate::data::StreamColumnSource::Scalar(&source),
    }];
    assert!(matches!(
        renderer.resident_admission_for_streamed_columns(&stale, 0, &[], 0, u64::MAX),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    let short = FailingPromotionSource { len: 2 };
    let wrong_len = [crate::StreamSourceBinding {
        id: "stay-streamed",
        revision: 4,
        source: crate::data::StreamColumnSource::Scalar(&short),
    }];
    assert!(matches!(
        renderer.resident_admission_for_streamed_columns(&wrong_len, 0, &[], 0, u64::MAX),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    let wrong_encoding_values = col_f64(vec![1.0, 2.0, 3.0]);
    let wrong_encoding = [crate::StreamSourceBinding {
        id: "stay-streamed",
        revision: 4,
        source: crate::data::StreamColumnSource::HiLo(&wrong_encoding_values),
    }];
    assert!(matches!(
        renderer.resident_admission_for_streamed_columns(&wrong_encoding, 0, &[], 0, u64::MAX,),
        Err(FiggyError::InvalidStreamSource { .. })
    ));

    let current = renderer.gpu_memory_usage().checked_total_bytes().unwrap();
    let _ = renderer.set_memory_budget(Some(current));
    let exact = [crate::StreamSourceBinding {
        revision: 4,
        ..stale[0]
    }];
    let duplicate = [exact[0], exact[0]];
    assert!(matches!(
        renderer.resident_admission_for_streamed_columns(&duplicate, 0, &[], 0, u64::MAX),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    let rejected = renderer
        .promote_streamed_columns_to_resident(&exact, 0, &[], 0, u64::MAX)
        .unwrap();
    assert_eq!(
        rejected.status,
        ResidentAdmissionStatus::MemoryBudgetExceeded
    );
    let _ = renderer.set_memory_budget(Some(u64::MAX));
    assert!(matches!(
        renderer.promote_streamed_columns_to_resident(&exact, 0, &[], 0, u64::MAX),
        Err(FiggyError::Pool(
            data_render::AllocError::AllocationFailed {
                resource: "streamed promotion source",
                ..
            }
        ))
    ));
    assert!(matches!(
        renderer.logical_column("stay-streamed"),
        Some(crate::LogicalColumn::Streamed(_))
    ));
    assert!(renderer.pool.slot("stay-streamed").is_none());
    assert_eq!(renderer.stream_revision_lineage["stay-streamed"], 4);
    assert_eq!(renderer.pool.used_bytes(), used);
    assert_eq!(renderer.pool.layout_generation(), layout);
    assert_eq!(revision_snapshot(&renderer, chart), revisions);
}

#[test]
fn resident_demotion_commits_mixed_metadata_and_preserves_chart_authority() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let demote_x = col_f64(vec![1.0, 2.0, 3.0]);
    let demote_y = col_f64(vec![
        1_700_000_000_000.0,
        1_700_000_000_000.25,
        1_700_000_000_000.5,
    ]);
    renderer.add_column("demote-x", &demote_x).unwrap();
    renderer.add_hilo_column("demote-y", &demote_y).unwrap();
    renderer
        .add_column("resident-z", &col_f64(vec![4.0, 5.0, 6.0]))
        .unwrap();
    let config = crate::default::default_config();
    let series = state_test_line("demoted", "demote-x", "demote-y");
    let dependent = renderer
        .register_chart(config.clone(), vec![series.clone()])
        .unwrap();
    let unrelated = renderer
        .register_chart(
            config.clone(),
            vec![state_test_line("resident", "resident-z", "resident-z")],
        )
        .unwrap();
    let dependent_before = revision_snapshot(&renderer, dependent);
    let unrelated_before = revision_snapshot(&renderer, unrelated);
    let fit_before = renderer.begin_fit_commit(dependent).unwrap();
    let chart = Chart::new(config.clone());
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let prepared = renderer
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id: dependent,
            view: &view,
        }])
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(dependent).unwrap();
    assert_eq!(active_picker(&renderer).chart_id, dependent);

    let sources = [
        crate::StreamSourceBinding {
            id: "demote-x",
            revision: 31,
            source: crate::StreamColumnSource::Scalar(&demote_x),
        },
        crate::StreamSourceBinding {
            id: "demote-y",
            revision: 47,
            source: crate::StreamColumnSource::HiLo(&demote_y),
        },
    ];
    renderer
        .demote_resident_columns_to_streamed(
            &["demote-x"],
            vec![
                crate::StreamColumn {
                    id: "demote-x".into(),
                    len: 3,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    revision: 31,
                    statistics: crate::StreamStatistics::Unknown,
                },
                crate::StreamColumn {
                    id: "demote-y".into(),
                    len: 3,
                    encoding: crate::StreamEncoding::HiLoF32,
                    replay: crate::StreamReplay::Sequential,
                    revision: 47,
                    statistics: crate::StreamStatistics::Unknown,
                },
            ],
            &sources,
        )
        .unwrap();

    assert!(matches!(
        renderer.logical_column("demote-x"),
        Some(crate::LogicalColumn::Streamed(source))
            if source.revision == 31
                && source.encoding == crate::StreamEncoding::ScalarF32
    ));
    assert!(matches!(
        renderer.logical_column("demote-y"),
        Some(crate::LogicalColumn::Streamed(source))
            if source.revision == 47
                && source.encoding == crate::StreamEncoding::HiLoF32
    ));
    assert!(renderer.pool.slot("demote-x").is_none());
    assert!(renderer.pool.slot("demote-y").is_none());
    assert!(renderer.pool.slot("resident-z").is_some());
    assert_eq!(renderer.stream_revision_lineage["demote-x"], 31);
    assert_eq!(renderer.stream_revision_lineage["demote-y"], 47);
    assert_eq!(renderer.chart_config(dependent).unwrap(), &config);
    assert_eq!(renderer.chart_series(dependent).unwrap(), &[series]);
    let dependent_after = revision_snapshot(&renderer, dependent);
    assert_ne!(dependent_after[0], dependent_before[0]);
    assert_ne!(dependent_after[3], dependent_before[3]);
    assert_eq!(dependent_after[1], dependent_before[1]);
    assert_eq!(dependent_after[2], dependent_before[2]);
    assert_eq!(revision_snapshot(&renderer, unrelated), unrelated_before);
    assert!(renderer.picker.active.is_none());
    assert!(matches!(
        renderer.validate_prepared(&prepared),
        Err(FiggyError::StalePreparedFrame { .. })
    ));
    assert!(matches!(
        renderer.commit_auto_fit_all_if_current(
            &fit_before,
            &crate::FitExtent::EMPTY,
            &crate::FitExtent::EMPTY,
            0.0,
        ),
        Err(FiggyError::StaleStateToken { .. })
    ));
}

#[test]
fn resident_demotion_rejects_the_whole_batch_before_any_authority_changes() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let resident_a = col_f64(vec![1.0, 2.0]);
    let resident_b = col_f64(vec![1.0, 2.0]);
    renderer.add_column("resident-a", &resident_a).unwrap();
    renderer.add_column("resident-b", &resident_b).unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("resident", "resident-a", "resident-b")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let revisions = revision_snapshot(&renderer, chart);
    let generation = renderer.pool.generation();
    let used = renderer.pool.used_bytes();
    let stream_count = renderer.streaming_sources.len();
    let fit_epoch = renderer.next_stream_source_fit_epoch;
    let lineage = renderer.stream_revision_lineage.clone();

    let metadata = |id: &str, len: u64| crate::StreamColumn {
        id: id.into(),
        len,
        encoding: crate::StreamEncoding::ScalarF32,
        replay: crate::StreamReplay::RandomAccess,
        revision: 5,
        statistics: crate::StreamStatistics::Unknown,
    };
    let sources = [
        crate::StreamSourceBinding {
            id: "resident-a",
            revision: 5,
            source: crate::StreamColumnSource::Scalar(&resident_a),
        },
        crate::StreamSourceBinding {
            id: "resident-b",
            revision: 5,
            source: crate::StreamColumnSource::Scalar(&resident_b),
        },
    ];
    assert!(matches!(
        renderer.demote_resident_columns_to_streamed(
            &["resident-a"],
            vec![metadata("resident-a", 2), metadata("resident-a", 2)],
            &sources,
        ),
        Err(FiggyError::Pool(data_render::AllocError::DuplicateId(_)))
    ));
    let short = col_f64(vec![1.0]);
    let wrong_companion = [
        sources[0],
        crate::StreamSourceBinding {
            id: "resident-b",
            revision: 5,
            source: crate::StreamColumnSource::Scalar(&short),
        },
    ];
    assert!(matches!(
        renderer.demote_resident_columns_to_streamed(
            &["resident-a"],
            vec![metadata("resident-a", 2), metadata("resident-b", 1)],
            &wrong_companion,
        ),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    let mut known = metadata("resident-a", 2);
    known.statistics = crate::StreamStatistics::Known(Some(crate::StreamBounds {
        min: 1.0,
        max: 2.0,
        min_positive: Some(1.0),
    }));
    assert!(matches!(
        renderer.demote_resident_columns_to_streamed(
            &["resident-a"],
            vec![known, metadata("resident-b", 2)],
            &sources,
        ),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    let missing_source = [crate::StreamSourceBinding {
        id: "missing",
        revision: 5,
        source: crate::StreamColumnSource::Scalar(&resident_a),
    }];
    assert!(matches!(
        renderer.demote_resident_columns_to_streamed(
            &["missing"],
            vec![metadata("missing", 2)],
            &missing_source,
        ),
        Err(FiggyError::UnknownColumn { .. })
    ));

    let old_desired = renderer.chart_states[&chart].revisions.desired;
    renderer
        .chart_states
        .get_mut(&chart)
        .unwrap()
        .revisions
        .desired = RenderRevision {
        renderer_identity: renderer.renderer_identity,
        sequence: u64::MAX,
    };
    assert!(matches!(
        renderer.demote_resident_columns_to_streamed(
            &["resident-a"],
            vec![metadata("resident-a", 2), metadata("resident-b", 2)],
            &sources,
        ),
        Err(FiggyError::CounterExhausted { .. })
    ));
    renderer
        .chart_states
        .get_mut(&chart)
        .unwrap()
        .revisions
        .desired = old_desired;

    assert_eq!(renderer.pool.generation(), generation);
    assert_eq!(renderer.pool.used_bytes(), used);
    assert!(renderer.pool.slot("resident-a").is_some());
    assert!(renderer.pool.slot("resident-b").is_some());
    assert_eq!(renderer.streaming_sources.len(), stream_count);
    assert_eq!(renderer.stream_revision_lineage, lineage);
    assert_eq!(renderer.next_stream_source_fit_epoch, fit_epoch);
    assert_eq!(revision_snapshot(&renderer, chart), revisions);
    assert_eq!(active_picker(&renderer).chart_id, chart);
}

#[test]
fn resident_demotion_closes_over_shared_charts_and_accepts_new_root_length() {
    let Some(mut renderer) = state_test_renderer_with_capacity(8 * 1024) else {
        return;
    };
    let old_a = col_f64(vec![1.0, 2.0]);
    let new_a = col_f64(vec![1.0, 2.0, 3.0]);
    let b = col_f64(vec![4.0, 5.0]);
    let c = col_f64(vec![6.0, 7.0]);
    let d = col_f64(vec![8.0, 9.0]);
    for (id, source) in [
        ("closure-a", &old_a),
        ("closure-b", &b),
        ("closure-c", &c),
        ("closure-d", &d),
    ] {
        renderer.add_column(id, source).unwrap();
    }
    let first = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("first", "closure-a", "closure-b")],
        )
        .unwrap();
    let shared = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("shared", "closure-b", "closure-c")],
        )
        .unwrap();
    let unrelated = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("unrelated", "closure-d", "closure-d")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(shared).unwrap();

    let metadata = |id: &str, len: u64| crate::StreamColumn {
        id: id.into(),
        len,
        encoding: crate::StreamEncoding::ScalarF32,
        replay: crate::StreamReplay::RandomAccess,
        revision: 1,
        statistics: crate::StreamStatistics::Unknown,
    };
    let sources = [
        crate::StreamSourceBinding {
            id: "closure-a",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&new_a),
        },
        crate::StreamSourceBinding {
            id: "closure-b",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&b),
        },
        crate::StreamSourceBinding {
            id: "closure-c",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&c),
        },
    ];
    let first_before = revision_snapshot(&renderer, first);
    let shared_before = revision_snapshot(&renderer, shared);
    let unrelated_before = revision_snapshot(&renderer, unrelated);
    let generation = renderer.pool.generation();
    let used = renderer.pool.used_bytes();
    assert!(matches!(
        renderer.demote_resident_columns_to_streamed(
            &["closure-a"],
            vec![metadata("closure-a", 3), metadata("closure-b", 2)],
            &sources[..2],
        ),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    assert_eq!(renderer.pool.generation(), generation);
    assert_eq!(renderer.pool.used_bytes(), used);
    assert!(renderer.streaming_sources.is_empty());
    assert!(renderer.stream_revision_lineage.is_empty());
    assert_eq!(revision_snapshot(&renderer, first), first_before);
    assert_eq!(revision_snapshot(&renderer, shared), shared_before);
    assert_eq!(active_picker(&renderer).chart_id, shared);

    renderer
        .demote_resident_columns_to_streamed(
            &["closure-a"],
            vec![
                metadata("closure-a", 3),
                metadata("closure-b", 2),
                metadata("closure-c", 2),
            ],
            &sources,
        )
        .unwrap();
    for id in ["closure-a", "closure-b", "closure-c"] {
        assert!(renderer.pool.slot(id).is_none());
        assert!(renderer.streaming_sources.contains_key(id));
    }
    assert_eq!(renderer.streaming_sources["closure-a"].len, 3);
    assert!(renderer.pool.slot("closure-d").is_some());
    for (before, after) in [
        (first_before, revision_snapshot(&renderer, first)),
        (shared_before, revision_snapshot(&renderer, shared)),
    ] {
        assert_eq!(after[0].sequence, before[0].sequence + 1);
        assert_eq!(after[3].sequence, before[3].sequence + 1);
        assert_eq!(after[1], before[1]);
        assert_eq!(after[2], before[2]);
        assert_eq!(after[4], before[4]);
        assert_eq!(after[5], before[5]);
        assert_eq!(after[6], before[6]);
    }
    assert_eq!(revision_snapshot(&renderer, unrelated), unrelated_before);
    assert!(renderer.picker.active.is_none());
    assert!(renderer.pending_defrag);

    let first_demoted = revision_snapshot(&renderer, first);
    let shared_demoted = revision_snapshot(&renderer, shared);
    let _ = renderer.set_memory_budget(Some(u64::MAX));
    assert!(matches!(
        renderer.promote_streamed_columns_to_resident(&sources[..1], 0, &[], 0, u64::MAX,),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    for id in ["closure-a", "closure-b", "closure-c"] {
        assert!(renderer.pool.slot(id).is_none());
        assert!(renderer.streaming_sources.contains_key(id));
    }
    let report = renderer
        .promote_streamed_columns_to_resident(&sources, 0, &[], 0, u64::MAX)
        .unwrap();
    assert!(report.is_admissible(), "{report:?}");
    for id in ["closure-a", "closure-b", "closure-c"] {
        assert!(renderer.pool.slot(id).is_some());
        assert!(!renderer.streaming_sources.contains_key(id));
        assert_eq!(renderer.stream_revision_lineage[id], 1);
    }
    for (before, after) in [
        (first_demoted, revision_snapshot(&renderer, first)),
        (shared_demoted, revision_snapshot(&renderer, shared)),
    ] {
        assert_eq!(after[0].sequence, before[0].sequence + 1);
        assert_eq!(after[3].sequence, before[3].sequence + 1);
        assert_eq!(after[1], before[1]);
        assert_eq!(after[2], before[2]);
        assert_eq!(after[4], before[4]);
        assert_eq!(after[5], before[5]);
        assert_eq!(after[6], before[6]);
    }
    assert_eq!(revision_snapshot(&renderer, unrelated), unrelated_before);
}

#[test]
fn stream_revision_lineage_survives_promotion_and_true_remove_clears_it() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let values = col_f64(vec![1.0, 2.0]);
    let metadata = |revision| crate::StreamColumn {
        id: "lineage".into(),
        len: 2,
        encoding: crate::StreamEncoding::ScalarF32,
        replay: crate::StreamReplay::RandomAccess,
        revision,
        statistics: crate::StreamStatistics::Unknown,
    };
    renderer
        .register_streamed_columns(vec![metadata(5)])
        .unwrap();
    assert_eq!(renderer.stream_revision_lineage["lineage"], 5);
    let binding = |revision| crate::StreamSourceBinding {
        id: "lineage",
        revision,
        source: crate::StreamColumnSource::Scalar(&values),
    };
    let _ = renderer.set_memory_budget(Some(u64::MAX));
    renderer
        .promote_streamed_columns_to_resident(&[binding(5)], 0, &[], 0, u64::MAX)
        .unwrap();
    assert_eq!(renderer.stream_revision_lineage["lineage"], 5);
    assert!(matches!(
        renderer.demote_resident_columns_to_streamed(
            &["lineage"],
            vec![metadata(5)],
            &[binding(5)],
        ),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    assert!(renderer.pool.slot("lineage").is_some());
    assert!(!renderer.streaming_sources.contains_key("lineage"));
    assert_eq!(renderer.stream_revision_lineage["lineage"], 5);

    renderer
        .demote_resident_columns_to_streamed(&["lineage"], vec![metadata(6)], &[binding(6)])
        .unwrap();
    renderer
        .replace_streamed_columns(vec![metadata(7)])
        .unwrap();
    assert_eq!(renderer.stream_revision_lineage["lineage"], 7);
    assert!(renderer.remove_column("lineage").unwrap());
    assert!(!renderer.stream_revision_lineage.contains_key("lineage"));

    renderer.add_column("native-zero", &values).unwrap();
    let zero = crate::StreamColumn {
        id: "native-zero".into(),
        revision: 0,
        ..metadata(1)
    };
    let zero_binding = crate::StreamSourceBinding {
        id: "native-zero",
        revision: 0,
        source: crate::StreamColumnSource::Scalar(&values),
    };
    assert!(matches!(
        renderer
            .demote_resident_columns_to_streamed(&["native-zero"], vec![zero], &[zero_binding],),
        Err(FiggyError::InvalidStreamSource { .. })
    ));
    assert!(renderer.pool.slot("native-zero").is_some());
}

#[test]
fn resident_demotion_normalizes_a_mixed_chart_and_preserves_streamed_companion() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let resident = col_f64(vec![1.0, 2.0]);
    let streamed = col_f64(vec![3.0, 4.0]);
    renderer.add_column("mixed-resident", &resident).unwrap();
    renderer
        .register_streamed_columns(vec![crate::StreamColumn {
            id: "mixed-streamed".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 1,
            statistics: crate::StreamStatistics::Unknown,
        }])
        .unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("mixed", "mixed-resident", "mixed-streamed")],
        )
        .unwrap();
    let revisions = revision_snapshot(&renderer, chart);
    let streamed_fit_epoch = renderer.streaming_sources["mixed-streamed"].fit_epoch;
    let resident_metadata = crate::StreamColumn {
        id: "mixed-resident".into(),
        len: 2,
        encoding: crate::StreamEncoding::ScalarF32,
        replay: crate::StreamReplay::RandomAccess,
        revision: 1,
        statistics: crate::StreamStatistics::Unknown,
    };
    let streamed_metadata = crate::StreamColumn {
        id: "mixed-streamed".into(),
        ..resident_metadata.clone()
    };
    let bindings = [
        crate::StreamSourceBinding {
            id: "mixed-resident",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&resident),
        },
        crate::StreamSourceBinding {
            id: "mixed-streamed",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&streamed),
        },
    ];
    renderer
        .demote_resident_columns_to_streamed(
            &["mixed-resident"],
            vec![resident_metadata, streamed_metadata],
            &bindings,
        )
        .unwrap();
    assert!(renderer.pool.slot("mixed-resident").is_none());
    assert_eq!(renderer.streaming_sources["mixed-resident"].revision, 1);
    assert!(renderer.streaming_sources.contains_key("mixed-streamed"));
    assert_eq!(
        renderer.streaming_sources["mixed-streamed"].fit_epoch,
        streamed_fit_epoch
    );
    assert_eq!(renderer.stream_revision_lineage["mixed-resident"], 1);
    assert_eq!(renderer.stream_revision_lineage["mixed-streamed"], 1);
    let after = revision_snapshot(&renderer, chart);
    assert_eq!(after[0].sequence, revisions[0].sequence + 1);
    assert_eq!(after[3].sequence, revisions[3].sequence + 1);
    for index in [1, 2, 4, 5, 6] {
        assert_eq!(after[index], revisions[index]);
    }
}

#[test]
fn mixed_normalization_closes_three_shared_charts_and_preserves_caches_and_picker() {
    let Some(mut renderer) = state_test_renderer_with_capacity(8 * 1024) else {
        return;
    };
    let a = col_f64(vec![1.0, 2.0]);
    let b = col_f64(vec![3.0, 4.0]);
    let c = col_f64(vec![5.0, 6.0]);
    let d = col_f64(vec![7.0, 8.0]);
    let picker_values = col_f64(vec![9.0, 10.0]);
    renderer.add_column("graph-a", &a).unwrap();
    renderer.add_column("graph-c", &c).unwrap();
    renderer.add_column("graph-picker", &picker_values).unwrap();
    renderer
        .register_streamed_columns(vec![
            crate::StreamColumn {
                id: "graph-b".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 10,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "graph-d".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::Sequential,
                revision: 20,
                statistics: crate::StreamStatistics::Unknown,
            },
        ])
        .unwrap();
    let chart_ab = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("ab", "graph-a", "graph-b")],
        )
        .unwrap();
    let chart_bc = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("bc", "graph-b", "graph-c")],
        )
        .unwrap();
    let chart_cd = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("cd", "graph-c", "graph-d")],
        )
        .unwrap();
    let picker_chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("picker", "graph-picker", "graph-picker")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer
        .prepare_gpu_picking_for_chart(picker_chart)
        .unwrap();
    renderer
        .streaming_sources
        .get_mut("graph-b")
        .unwrap()
        .statistics_cache
        .covered
        .push(0..1);
    renderer
        .streaming_sources
        .get_mut("graph-d")
        .unwrap()
        .statistics_cache
        .covered
        .push(1..2);

    let chart_snapshots = [chart_ab, chart_bc, chart_cd].map(|chart| {
        (
            chart,
            renderer.chart_config(chart).unwrap().clone(),
            renderer.chart_series(chart).unwrap().to_vec(),
            revision_snapshot(&renderer, chart),
        )
    });
    let picker_revisions = revision_snapshot(&renderer, picker_chart);
    let b_fit = renderer.streaming_sources["graph-b"].fit_epoch;
    let d_fit = renderer.streaming_sources["graph-d"].fit_epoch;
    let metadata = vec![
        crate::StreamColumn {
            id: "graph-a".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 1,
            statistics: crate::StreamStatistics::Unknown,
        },
        crate::StreamColumn {
            id: "graph-b".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 10,
            statistics: crate::StreamStatistics::Unknown,
        },
        crate::StreamColumn {
            id: "graph-c".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 1,
            statistics: crate::StreamStatistics::Unknown,
        },
        crate::StreamColumn {
            id: "graph-d".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::Sequential,
            revision: 20,
            statistics: crate::StreamStatistics::Unknown,
        },
    ];
    let bindings = [
        crate::StreamSourceBinding {
            id: "graph-a",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&a),
        },
        crate::StreamSourceBinding {
            id: "graph-b",
            revision: 10,
            source: crate::StreamColumnSource::Scalar(&b),
        },
        crate::StreamSourceBinding {
            id: "graph-c",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&c),
        },
        crate::StreamSourceBinding {
            id: "graph-d",
            revision: 20,
            source: crate::StreamColumnSource::Scalar(&d),
        },
    ];
    renderer
        .demote_resident_columns_to_streamed(&["graph-a"], metadata, &bindings)
        .unwrap();

    for id in ["graph-a", "graph-b", "graph-c", "graph-d"] {
        assert!(renderer.pool.slot(id).is_none(), "{id} stayed resident");
        assert!(
            renderer.streaming_sources.contains_key(id),
            "{id} did not become streamed"
        );
    }
    assert_eq!(renderer.streaming_sources["graph-b"].fit_epoch, b_fit);
    assert_eq!(renderer.streaming_sources["graph-d"].fit_epoch, d_fit);
    assert_eq!(
        renderer.streaming_sources["graph-b"]
            .statistics_cache
            .covered,
        vec![0..1]
    );
    assert_eq!(
        renderer.streaming_sources["graph-d"]
            .statistics_cache
            .covered,
        vec![1..2]
    );
    for (chart, config, series, before) in chart_snapshots {
        assert_eq!(renderer.chart_config(chart).unwrap(), &config);
        assert_eq!(renderer.chart_series(chart).unwrap(), series);
        let after = revision_snapshot(&renderer, chart);
        assert_eq!(after[0].sequence, before[0].sequence + 1);
        assert_eq!(after[3].sequence, before[3].sequence + 1);
        for index in [1, 2, 4, 5, 6] {
            assert_eq!(after[index], before[index]);
        }
    }
    assert_eq!(revision_snapshot(&renderer, picker_chart), picker_revisions);
    assert_eq!(active_picker(&renderer).chart_id, picker_chart);
    assert!(renderer.pending_defrag);
}

#[test]
fn mixed_normalize_promote_and_length_change_demote_preserve_lineage_monotonicity() {
    let Some(mut renderer) = state_test_renderer_with_capacity(8 * 1024) else {
        return;
    };
    let x_old = col_f64(vec![1.0, 2.0]);
    let x_new = col_f64(vec![1.0, 2.0, 3.0]);
    let y = col_f64(vec![4.0, 5.0]);
    renderer.add_column("round-x", &x_old).unwrap();
    renderer
        .register_streamed_columns(vec![crate::StreamColumn {
            id: "round-y".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 5,
            statistics: crate::StreamStatistics::Unknown,
        }])
        .unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("round", "round-x", "round-y")],
        )
        .unwrap();
    let initial_revisions = revision_snapshot(&renderer, chart);
    let initial_metadata = vec![
        crate::StreamColumn {
            id: "round-x".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 1,
            statistics: crate::StreamStatistics::Unknown,
        },
        crate::StreamColumn {
            id: "round-y".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 5,
            statistics: crate::StreamStatistics::Unknown,
        },
    ];
    let initial_bindings = [
        crate::StreamSourceBinding {
            id: "round-x",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&x_old),
        },
        crate::StreamSourceBinding {
            id: "round-y",
            revision: 5,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];
    renderer
        .demote_resident_columns_to_streamed(&["round-x"], initial_metadata, &initial_bindings)
        .unwrap();
    assert_eq!(renderer.stream_revision_lineage["round-x"], 1);
    assert_eq!(renderer.stream_revision_lineage["round-y"], 5);

    let _ = renderer.set_memory_budget(Some(u64::MAX));
    let report = renderer
        .promote_streamed_columns_to_resident(&initial_bindings, 0, &[], 0, u64::MAX)
        .unwrap();
    assert!(report.is_admissible(), "{report:?}");
    assert!(renderer.pool.slot("round-x").is_some());
    assert!(renderer.pool.slot("round-y").is_some());
    assert_eq!(renderer.stream_revision_lineage["round-x"], 1);
    assert_eq!(renderer.stream_revision_lineage["round-y"], 5);

    let final_metadata = vec![
        crate::StreamColumn {
            id: "round-x".into(),
            len: 3,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::Sequential,
            revision: 2,
            statistics: crate::StreamStatistics::Unknown,
        },
        crate::StreamColumn {
            id: "round-y".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 6,
            statistics: crate::StreamStatistics::Unknown,
        },
    ];
    let final_bindings = [
        crate::StreamSourceBinding {
            id: "round-x",
            revision: 2,
            source: crate::StreamColumnSource::Scalar(&x_new),
        },
        crate::StreamSourceBinding {
            id: "round-y",
            revision: 6,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];
    renderer
        .demote_resident_columns_to_streamed(&["round-x"], final_metadata, &final_bindings)
        .unwrap();
    assert_eq!(renderer.streaming_sources["round-x"].len, 3);
    assert_eq!(renderer.streaming_sources["round-x"].revision, 2);
    assert_eq!(renderer.streaming_sources["round-y"].revision, 6);
    assert_eq!(renderer.stream_revision_lineage["round-x"], 2);
    assert_eq!(renderer.stream_revision_lineage["round-y"], 6);
    let final_revisions = revision_snapshot(&renderer, chart);
    assert_eq!(
        final_revisions[0].sequence,
        initial_revisions[0].sequence + 3
    );
    assert_eq!(
        final_revisions[3].sequence,
        initial_revisions[3].sequence + 3
    );
    for index in [1, 2, 4, 5, 6] {
        assert_eq!(final_revisions[index], initial_revisions[index]);
    }
}

#[test]
fn resident_demotion_replaces_a_changed_streamed_root_within_the_mixed_commit() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let resident = col_f64(vec![1.0, 2.0]);
    let new_stream = col_f64(vec![3.0, 4.0, 5.0]);
    renderer.add_column("replace-resident", &resident).unwrap();
    renderer
        .register_streamed_columns(vec![crate::StreamColumn {
            id: "replace-stream".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 5,
            statistics: crate::StreamStatistics::Unknown,
        }])
        .unwrap();
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line(
                "replace",
                "replace-resident",
                "replace-stream",
            )],
        )
        .unwrap();
    let before = revision_snapshot(&renderer, chart);
    let old_fit_epoch = renderer.streaming_sources["replace-stream"].fit_epoch;
    let metadata = vec![
        crate::StreamColumn {
            id: "replace-resident".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 1,
            statistics: crate::StreamStatistics::Unknown,
        },
        crate::StreamColumn {
            id: "replace-stream".into(),
            len: 3,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::Sequential,
            revision: 6,
            statistics: crate::StreamStatistics::Unknown,
        },
    ];
    let bindings = [
        crate::StreamSourceBinding {
            id: "replace-resident",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&resident),
        },
        crate::StreamSourceBinding {
            id: "replace-stream",
            revision: 6,
            source: crate::StreamColumnSource::Scalar(&new_stream),
        },
    ];
    renderer
        .demote_resident_columns_to_streamed(&["replace-stream"], metadata, &bindings)
        .unwrap();

    assert!(renderer.pool.slot("replace-resident").is_none());
    assert_eq!(renderer.streaming_sources["replace-resident"].revision, 1);
    let replaced = &renderer.streaming_sources["replace-stream"];
    assert_eq!(replaced.revision, 6);
    assert_eq!(replaced.len, 3);
    assert_eq!(replaced.replay, crate::StreamReplay::Sequential);
    assert!(replaced.fit_epoch > old_fit_epoch);
    assert_eq!(renderer.stream_revision_lineage["replace-stream"], 6);
    let after = revision_snapshot(&renderer, chart);
    assert_eq!(after[0].sequence, before[0].sequence + 1);
    assert_eq!(after[3].sequence, before[3].sequence + 1);
    for index in [1, 2, 4, 5, 6] {
        assert_eq!(after[index], before[index]);
    }
}

#[test]
fn all_streamed_same_revision_normalization_is_a_true_noop() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let x = col_f64(vec![1.0, 2.0]);
    let y = col_f64(vec![3.0, 4.0]);
    let resident = col_f64(vec![5.0, 6.0]);
    renderer
        .register_streamed_columns(vec![
            crate::StreamColumn {
                id: "noop-x".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 3,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "noop-y".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::Sequential,
                revision: 4,
                statistics: crate::StreamStatistics::Unknown,
            },
        ])
        .unwrap();
    let streamed_chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("noop", "noop-x", "noop-y")],
        )
        .unwrap();
    renderer.add_column("noop-resident", &resident).unwrap();
    let picker_chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("picker", "noop-resident", "noop-resident")],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer
        .prepare_gpu_picking_for_chart(picker_chart)
        .unwrap();
    renderer
        .streaming_sources
        .get_mut("noop-x")
        .unwrap()
        .statistics_cache
        .covered
        .push(0..1);
    renderer.pending_defrag = true;

    let revisions = revision_snapshot(&renderer, streamed_chart);
    let picker_revisions = revision_snapshot(&renderer, picker_chart);
    let visual = renderer.visual_revision;
    let generation = renderer.pool.generation();
    let used = renderer.pool.used_bytes();
    let next_fit_epoch = renderer.next_stream_source_fit_epoch;
    let x_fit_epoch = renderer.streaming_sources["noop-x"].fit_epoch;
    let y_fit_epoch = renderer.streaming_sources["noop-y"].fit_epoch;
    let lineage = renderer.stream_revision_lineage.clone();
    let metadata = || {
        vec![
            crate::StreamColumn {
                id: "noop-x".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 3,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "noop-y".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::Sequential,
                revision: 4,
                statistics: crate::StreamStatistics::Unknown,
            },
        ]
    };
    let bindings = [
        crate::StreamSourceBinding {
            id: "noop-x",
            revision: 3,
            source: crate::StreamColumnSource::Scalar(&x),
        },
        crate::StreamSourceBinding {
            id: "noop-y",
            revision: 4,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];
    for _ in 0..4 {
        renderer
            .demote_resident_columns_to_streamed(&["noop-x"], metadata(), &bindings)
            .unwrap();
    }

    assert_eq!(renderer.pool.generation(), generation);
    assert_eq!(renderer.pool.used_bytes(), used);
    assert_eq!(renderer.next_stream_source_fit_epoch, next_fit_epoch);
    assert_eq!(renderer.streaming_sources["noop-x"].fit_epoch, x_fit_epoch);
    assert_eq!(renderer.streaming_sources["noop-y"].fit_epoch, y_fit_epoch);
    assert_eq!(
        renderer.streaming_sources["noop-x"]
            .statistics_cache
            .covered,
        vec![0..1]
    );
    assert_eq!(renderer.stream_revision_lineage, lineage);
    assert_eq!(revision_snapshot(&renderer, streamed_chart), revisions);
    assert_eq!(revision_snapshot(&renderer, picker_chart), picker_revisions);
    assert_eq!(renderer.visual_revision, visual);
    assert_eq!(active_picker(&renderer).chart_id, picker_chart);
    assert!(renderer.pending_defrag);
}

#[test]
fn live_normalization_supersedes_the_active_auto_snapshot_immediately() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let new_x = col_f64(vec![10.0, 20.0]);
    let y = col_f64(vec![3.0, 4.0]);
    renderer
        .register_streamed_columns(vec![
            crate::StreamColumn {
                id: "auto-live-x".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 1,
                statistics: crate::StreamStatistics::Unknown,
            },
            crate::StreamColumn {
                id: "auto-live-y".into(),
                len: 2,
                encoding: crate::StreamEncoding::ScalarF32,
                replay: crate::StreamReplay::RandomAccess,
                revision: 1,
                statistics: crate::StreamStatistics::Unknown,
            },
        ])
        .unwrap();
    let config = crate::default::default_config();
    let chart = renderer
        .register_chart(
            config.clone(),
            vec![state_test_line("auto-live", "auto-live-x", "auto-live-y")],
        )
        .unwrap();
    renderer
        .configure_streaming(crate::StreamingLimits {
            max_active_charts: 1,
            max_in_flight_chunks: 2,
            max_columns_per_chunk: 2,
            max_chunk_input_bytes: 4096,
            max_in_flight_gpu_bytes: 8192,
        })
        .unwrap();
    let view = renderer
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 1,
    };
    let crate::AutoStreamingRequest::Started {
        revision: active_revision,
        ..
    } = renderer
        .request_auto_streaming_chart(chart, &view, options)
        .unwrap()
    else {
        panic!("automatic stream must start")
    };
    let job = renderer.active_stream_job(chart).unwrap();
    assert_eq!(
        renderer.auto_stream_snapshot(job).unwrap().sources["auto-live-x"].revision,
        1
    );

    let metadata = vec![
        crate::StreamColumn {
            id: "auto-live-x".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 2,
            statistics: crate::StreamStatistics::Unknown,
        },
        crate::StreamColumn {
            id: "auto-live-y".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 1,
            statistics: crate::StreamStatistics::Unknown,
        },
    ];
    let live_bindings = [
        crate::StreamSourceBinding {
            id: "auto-live-x",
            revision: 2,
            source: crate::StreamColumnSource::Scalar(&new_x),
        },
        crate::StreamSourceBinding {
            id: "auto-live-y",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];
    renderer
        .demote_resident_columns_to_streamed(&["auto-live-x"], metadata, &live_bindings)
        .unwrap();
    let latest = renderer.chart_states[&chart].revisions.desired;
    let crate::AutoStreamingRequest::Started {
        revision: replacement_revision,
        ..
    } = renderer
        .request_auto_streaming_chart(chart, &view, options)
        .unwrap()
    else {
        panic!("live source normalization must immediately start the latest execution")
    };
    assert_eq!(replacement_revision, latest);
    let replacement_job = renderer.active_stream_job(chart).unwrap();
    assert_ne!(replacement_job, job);
    let snapshot = renderer.auto_stream_snapshot(replacement_job).unwrap();
    assert_eq!(snapshot.sources["auto-live-x"].revision, 2);
    assert_eq!(snapshot.sources["auto-live-y"].revision, 1);
    assert!(renderer.request_chart_stream_draw(job).is_err());
    let streaming_runtime::StreamDrawRequestStatus::Ready(ticket) =
        renderer.request_chart_stream_draw(replacement_job).unwrap()
    else {
        panic!("replacement automatic stream must issue the latest source request")
    };
    let requested = renderer.stream_request_columns(ticket).unwrap();
    assert!(
        requested
            .iter()
            .any(|request| { request.column == "auto-live-x" && request.range.revision == 2 })
    );
    assert!(
        requested
            .iter()
            .any(|request| { request.column == "auto-live-y" && request.range.revision == 1 })
    );
    assert!(
        requested
            .iter()
            .all(|request| { request.column != "auto-live-x" || request.range.revision != 1 })
    );
    assert_ne!(active_revision, latest);
}

#[test]
fn mixed_normalization_validation_failures_preserve_every_authority() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let resident = col_f64(vec![1.0, 2.0]);
    let streamed = col_f64(vec![3.0, 4.0]);
    let unrelated = col_f64(vec![5.0, 6.0]);
    let short = col_f64(vec![3.0]);
    renderer.add_column("fail-resident", &resident).unwrap();
    renderer.add_column("fail-unrelated", &unrelated).unwrap();
    renderer
        .register_streamed_columns(vec![crate::StreamColumn {
            id: "fail-stream".into(),
            len: 2,
            encoding: crate::StreamEncoding::ScalarF32,
            replay: crate::StreamReplay::RandomAccess,
            revision: 5,
            statistics: crate::StreamStatistics::Unknown,
        }])
        .unwrap();
    let affected = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line("affected", "fail-resident", "fail-stream")],
        )
        .unwrap();
    let picker_chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_line(
                "unrelated",
                "fail-unrelated",
                "fail-unrelated",
            )],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer
        .prepare_gpu_picking_for_chart(picker_chart)
        .unwrap();
    {
        let cache = &mut renderer
            .streaming_sources
            .get_mut("fail-stream")
            .unwrap()
            .statistics_cache;
        cache.covered.push(0..1);
        cache.bounds = Some(crate::StreamBounds {
            min: 3.0,
            max: 3.0,
            min_positive: Some(3.0),
        });
    }

    let generation = renderer.pool.generation();
    let used = renderer.pool.used_bytes();
    let resident_slot = renderer.pool.slot("fail-resident").unwrap().clone();
    let stream_column = renderer.streaming_sources["fail-stream"].column.clone();
    let stream_fit_epoch = renderer.streaming_sources["fail-stream"].fit_epoch;
    let stream_covered = renderer.streaming_sources["fail-stream"]
        .statistics_cache
        .covered
        .clone();
    let stream_bounds = renderer.streaming_sources["fail-stream"]
        .statistics_cache
        .bounds;
    let lineage = renderer.stream_revision_lineage.clone();
    let affected_revisions = revision_snapshot(&renderer, affected);
    let picker_revisions = revision_snapshot(&renderer, picker_chart);
    let visual = renderer.visual_revision;
    let fit_epoch = renderer.next_stream_source_fit_epoch;
    let pending_defrag = renderer.pending_defrag;
    let assert_unchanged = |renderer: &Renderer| {
        assert_eq!(renderer.pool.generation(), generation);
        assert_eq!(renderer.pool.used_bytes(), used);
        let slot = renderer.pool.slot("fail-resident").unwrap();
        assert_eq!(slot.offset, resident_slot.offset);
        assert_eq!(slot.len_values, resident_slot.len_values);
        let registered = &renderer.streaming_sources["fail-stream"];
        assert_eq!(registered.column, stream_column);
        assert_eq!(registered.fit_epoch, stream_fit_epoch);
        assert_eq!(registered.statistics_cache.covered, stream_covered);
        assert_eq!(registered.statistics_cache.bounds, stream_bounds);
        assert_eq!(renderer.stream_revision_lineage, lineage);
        assert_eq!(revision_snapshot(renderer, affected), affected_revisions);
        assert_eq!(revision_snapshot(renderer, picker_chart), picker_revisions);
        assert_eq!(renderer.visual_revision, visual);
        assert_eq!(renderer.next_stream_source_fit_epoch, fit_epoch);
        assert_eq!(renderer.pending_defrag, pending_defrag);
        assert_eq!(active_picker(renderer).chart_id, picker_chart);
    };
    let metadata = |id: &str, len: u64, revision: u64| crate::StreamColumn {
        id: id.into(),
        len,
        encoding: crate::StreamEncoding::ScalarF32,
        replay: crate::StreamReplay::RandomAccess,
        revision,
        statistics: crate::StreamStatistics::Unknown,
    };
    let resident_binding = crate::StreamSourceBinding {
        id: "fail-resident",
        revision: 1,
        source: crate::StreamColumnSource::Scalar(&resident),
    };
    let stream_binding = crate::StreamSourceBinding {
        id: "fail-stream",
        revision: 5,
        source: crate::StreamColumnSource::Scalar(&streamed),
    };

    assert!(
        renderer
            .demote_resident_columns_to_streamed(
                &["fail-resident"],
                vec![metadata("fail-resident", 2, 1)],
                &[resident_binding],
            )
            .is_err()
    );
    assert_unchanged(&renderer);

    assert!(
        renderer
            .demote_resident_columns_to_streamed(
                &["fail-resident"],
                vec![
                    metadata("fail-resident", 2, 1),
                    metadata("fail-stream", 2, 5),
                    metadata("fail-stream", 2, 5),
                ],
                &[resident_binding, stream_binding],
            )
            .is_err()
    );
    assert_unchanged(&renderer);

    let extra_binding = crate::StreamSourceBinding {
        id: "fail-extra",
        revision: 1,
        source: crate::StreamColumnSource::Scalar(&resident),
    };
    assert!(
        renderer
            .demote_resident_columns_to_streamed(
                &["fail-resident"],
                vec![
                    metadata("fail-resident", 2, 1),
                    metadata("fail-stream", 2, 5),
                    metadata("fail-extra", 2, 1),
                ],
                &[resident_binding, stream_binding, extra_binding],
            )
            .is_err()
    );
    assert_unchanged(&renderer);

    let stale_binding = crate::StreamSourceBinding {
        id: "fail-stream",
        revision: 4,
        source: crate::StreamColumnSource::Scalar(&streamed),
    };
    assert!(
        renderer
            .demote_resident_columns_to_streamed(
                &["fail-stream"],
                vec![
                    metadata("fail-resident", 2, 1),
                    metadata("fail-stream", 2, 4),
                ],
                &[resident_binding, stale_binding],
            )
            .is_err()
    );
    assert_unchanged(&renderer);

    let short_binding = crate::StreamSourceBinding {
        id: "fail-stream",
        revision: 5,
        source: crate::StreamColumnSource::Scalar(&short),
    };
    assert!(
        renderer
            .demote_resident_columns_to_streamed(
                &["fail-resident"],
                vec![
                    metadata("fail-resident", 2, 1),
                    metadata("fail-stream", 1, 5),
                ],
                &[resident_binding, short_binding],
            )
            .is_err()
    );
    assert_unchanged(&renderer);

    let wrong_encoding = crate::StreamSourceBinding {
        id: "fail-stream",
        revision: 5,
        source: crate::StreamColumnSource::HiLo(&streamed),
    };
    assert!(
        renderer
            .demote_resident_columns_to_streamed(
                &["fail-resident"],
                vec![
                    metadata("fail-resident", 2, 1),
                    metadata("fail-stream", 2, 5),
                ],
                &[resident_binding, wrong_encoding],
            )
            .is_err()
    );
    assert_unchanged(&renderer);

    let newer_companion = crate::StreamSourceBinding {
        id: "fail-stream",
        revision: 6,
        source: crate::StreamColumnSource::Scalar(&streamed),
    };
    assert!(
        renderer
            .demote_resident_columns_to_streamed(
                &["fail-resident"],
                vec![
                    metadata("fail-resident", 2, 1),
                    metadata("fail-stream", 2, 6),
                ],
                &[resident_binding, newer_companion],
            )
            .is_err()
    );
    assert_unchanged(&renderer);

    renderer
        .demote_resident_columns_to_streamed(
            &["fail-resident"],
            vec![
                metadata("fail-resident", 2, 1),
                metadata("fail-stream", 2, 5),
            ],
            &[resident_binding, stream_binding],
        )
        .unwrap();
    assert!(renderer.pool.slot("fail-resident").is_none());
    assert_eq!(renderer.streaming_sources["fail-resident"].revision, 1);
    assert_eq!(
        renderer.streaming_sources["fail-stream"].column,
        stream_column
    );
    assert_eq!(
        renderer.streaming_sources["fail-stream"]
            .statistics_cache
            .covered,
        stream_covered
    );
    assert_eq!(
        renderer.streaming_sources["fail-stream"]
            .statistics_cache
            .bounds,
        stream_bounds
    );
    let after = revision_snapshot(&renderer, affected);
    assert_eq!(after[0].sequence, affected_revisions[0].sequence + 1);
    assert_eq!(after[3].sequence, affected_revisions[3].sequence + 1);
    for index in [1, 2, 4, 5, 6] {
        assert_eq!(after[index], affected_revisions[index]);
    }
    assert_eq!(revision_snapshot(&renderer, picker_chart), picker_revisions);
    assert_eq!(active_picker(&renderer).chart_id, picker_chart);
    assert!(renderer.pending_defrag);
}

#[test]
fn resident_admission_counts_submitted_column_staging_until_completion() {
    let Some(mut renderer) = state_test_renderer_with_capacity(4 * 1024) else {
        return;
    };
    let before = renderer.gpu_memory_usage().checked_total_bytes().unwrap();
    renderer.add_column("staged", &col_f64(vec![1.0])).unwrap();
    let during = renderer.gpu_memory_usage();
    assert_eq!(
        during.retired_bytes_of(GpuResourceKind::ColumnPool),
        data_render::column_pool::ALIGN
    );
    let current = during.checked_total_bytes().unwrap();
    assert_eq!(current, before + data_render::column_pool::ALIGN);
    assert_eq!(renderer.set_memory_budget(Some(current)), None);
    let admitted = renderer.resident_admission(ResidentAdmissionRequest {
        column_value_counts: &[],
        already_resident_working_set_bytes: 0,
        derived_buffer_sizes: &[],
        transition_headroom_bytes: 0,
        working_set_limit_bytes: 0,
    });
    assert!(admitted.is_admissible(), "{admitted:?}");
    assert_eq!(admitted.current_gpu_bytes, current);

    renderer.end_gpu_frame();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("submitted staging completes");
    assert_eq!(
        renderer.gpu_memory_usage().checked_total_bytes(),
        Some(before)
    );
}

#[test]
fn resident_admission_mirrors_fixed_and_growing_pool_policies_without_mutation() {
    let Some(mut renderer) = state_test_renderer_with_capacity(data_render::column_pool::ALIGN)
    else {
        return;
    };
    let lengths = [64]; // 512 resident bytes, twice the initial pool.
    let request = ResidentAdmissionRequest {
        column_value_counts: &lengths,
        already_resident_working_set_bytes: 0,
        derived_buffer_sizes: &[],
        transition_headroom_bytes: 0,
        working_set_limit_bytes: 512,
    };
    let before_capacity = renderer.pool.capacity();
    assert_eq!(renderer.set_memory_budget(Some(u64::MAX)), None);
    renderer.set_pool_growth_policy(data_render::GrowthPolicy::Fixed);
    let fixed = renderer.resident_admission(request);
    assert_eq!(
        fixed.status,
        ResidentAdmissionStatus::PoolCapacityUnavailable
    );
    assert_eq!(renderer.pool.capacity(), before_capacity);
    assert_eq!(renderer.pool.used_bytes(), 0);

    renderer.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
    let growing = renderer.resident_admission(request);
    assert!(growing.is_admissible(), "{growing:?}");
    assert_eq!(growing.pool_capacity_before, 256);
    assert_eq!(growing.pool_capacity_after, 512);
    assert_eq!(growing.pool_transition_bytes, 512);
    assert_eq!(growing.upload_staging_bytes, 512);
    assert_eq!(renderer.pool.capacity(), before_capacity);
    assert_eq!(renderer.pool.used_bytes(), 0);
}

#[test]
fn resident_admission_fails_closed_on_index_and_byte_overflow() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    assert_eq!(renderer.set_memory_budget(Some(u64::MAX)), None);
    let over_index = [u64::from(u32::MAX) + 1];
    let rejected = renderer.resident_admission(ResidentAdmissionRequest {
        column_value_counts: &over_index,
        already_resident_working_set_bytes: 0,
        derived_buffer_sizes: &[],
        transition_headroom_bytes: 0,
        working_set_limit_bytes: u64::MAX,
    });
    assert_eq!(
        rejected.status,
        ResidentAdmissionStatus::ColumnLengthLimitExceeded {
            column: 0,
            length: u64::from(u32::MAX) + 1,
        }
    );

    let one = [1];
    let derived = [1];
    let overflow = renderer.resident_admission(ResidentAdmissionRequest {
        column_value_counts: &one,
        already_resident_working_set_bytes: u64::MAX,
        derived_buffer_sizes: &derived,
        transition_headroom_bytes: 0,
        working_set_limit_bytes: u64::MAX,
    });
    assert_eq!(overflow.status, ResidentAdmissionStatus::ArithmeticOverflow);

    let oversized = [renderer
        .caps
        .max_buffer_size
        .min(renderer.caps.max_storage_buffer_binding_size)
        + 1];
    let rejected = renderer.resident_admission(ResidentAdmissionRequest {
        column_value_counts: &[],
        already_resident_working_set_bytes: 0,
        derived_buffer_sizes: &oversized,
        transition_headroom_bytes: 0,
        working_set_limit_bytes: u64::MAX,
    });
    assert_eq!(
        rejected.status,
        ResidentAdmissionStatus::DerivedBufferLimitExceeded {
            buffer: 0,
            bytes: oversized[0],
        }
    );
}

#[test]
fn registering_a_pick_series_charges_pick_scratch_and_releases_it() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![state_test_scatter("scatter", None, "x", "y")],
        )
        .unwrap();

    let before = renderer.gpu_memory_usage();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    let after = renderer.gpu_memory_usage();
    assert!(
        after.live_bytes_of(GpuResourceKind::PickScratch)
            > before.live_bytes_of(GpuResourceKind::PickScratch),
        "pick registration allocates gate masks, candidates, style rows and \
         a scalar result — none of it was charged\n{}",
        after.report()
    );

    // Dropping the renderer must credit every charged byte back — the
    // ledger outlives it, so a leak shows up as a non-zero live row.
    let ledger = Arc::clone(renderer.gpu_ledger());
    drop(renderer);
    let released = ledger.snapshot();
    for kind in GpuResourceKind::ALL {
        assert_eq!(
            released.live_bytes_of(kind),
            0,
            "{} bytes still charged after the renderer was dropped\n{}",
            kind.label(),
            released.report()
        );
    }
    assert!(
        released.creations_of(GpuResourceKind::PickScratch) > 0,
        "the pick rows were never populated, so the check above proves nothing"
    );
}

#[test]
fn out_of_pool_bytes_ride_into_the_pool_and_a_budget_refuses_growth() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer.pool.growth_policy = data_render::GrowthPolicy::OnAllocFailure;
    let chart = Chart::new(panel_config(320, 240));
    let _view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();

    let usage = renderer.gpu_memory_usage();
    let external = usage.external_bytes();
    assert!(
        external >= 320 * 240 * 4 * 2,
        "the panel textures must be visible as out-of-pool bytes\n{}",
        usage.report()
    );

    // A ceiling that the settled state fits under but a growth cannot:
    // the pool would need old + new on top of `external`.
    assert_eq!(
        renderer.set_memory_budget(Some(external + usage.pool_bytes() + 1)),
        None,
        "this ceiling is above the settled total, so nothing is exceeded yet"
    );
    let capacity_before = renderer.pool.capacity();
    let oversized = col_f64((0..(capacity_before as usize)).map(|i| i as f64).collect());
    let error = renderer
        .add_column("too-big", &oversized)
        .expect_err("an upload larger than the pool must not fit under the budget");
    assert!(
        matches!(
            error,
            FiggyError::Pool(data_render::AllocError::BudgetExceeded { .. })
        ),
        "growth refused under budget must name the actual cause: {error:?}"
    );
    assert_eq!(
        renderer.pool.capacity(),
        capacity_before,
        "a refused growth leaves the pool exactly as it was"
    );
    assert!(renderer.pool.slot("too-big").is_none());

    // Lifting the ceiling lets the same upload through, which proves the
    // refusal came from the budget and not from the data.
    assert_eq!(
        renderer.set_memory_budget(None),
        None,
        "clearing the ceiling cannot conflict with anything"
    );
    renderer
        .add_column("too-big", &oversized)
        .expect("the same upload fits once the ceiling is lifted");
    assert!(renderer.pool.capacity() > capacity_before);
}

#[cfg(feature = "serde")]
#[path = "arc_mutation_tests.rs"]
mod arc_mutation;

/// Error bars and fills join a series' pick key by what they paint (#37
/// G10): a colour edit that keeps the alpha above zero reuses the slot; a
/// zero alpha, a stem width or a fill baseline edit rebuilds it with the new
/// geometry; and the slot references the error and second-series columns,
/// so removing one rebuilds the registry without the series.
#[test]
fn errorbar_and_fill_pick_keys_follow_what_they_paint() {
    use crate::data_config::{LineFill, LineInterpolation};
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    renderer
        .add_column("y_err", &col_f64(vec![0.1, 0.2]))
        .unwrap();
    renderer.add_column("y2", &col_f64(vec![0.5, 0.2])).unwrap();
    let bars = |color: Color, width: f32| {
        let mut err_style = test_errorbar_style();
        err_style.error_bar_color = color;
        err_style.error_bar_width = width;
        SeriesConfig {
            series_id: "bars".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: test_scatter_style(),
                err_y: ErrorRef::Symmetric {
                    column: "y_err".into(),
                },
                err_style,
            },
        }
    };
    let filled = |fill: LineFill, color: Color| SeriesConfig {
        series_id: "filled".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::new(0.0, 0.0, 0.0, 1.0),
                line_width: 1.0,
                fill: Some(fill),
                fill_color: Some(color),
                interpolation: LineInterpolation::Linear,
            },
        },
    };
    let lower = SeriesConfig {
        series_id: "lower".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y2".into(),
        z_column: None,
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::new(0.0, 0.0, 0.0, 1.0),
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: LineInterpolation::Linear,
            },
        },
    };
    let red = Color::new(1.0, 0.0, 0.0, 1.0);
    let blue = Color::new(0.0, 0.0, 1.0, 0.5);
    let clear = Color::new(0.0, 0.0, 1.0, 0.0);
    let to_lower = || LineFill::ToSeries("lower".into());
    let chart = renderer
        .register_chart(
            crate::default::default_config(),
            vec![bars(red, 1.0), filled(to_lower(), red), lower.clone()],
        )
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer.prepare_gpu_picking_for_chart(chart).unwrap();
    {
        let resource = &active_picker(&renderer).descriptors[0].resource;
        let errorbar = resource.errorbar.as_ref().expect("the bars are picked");
        assert!(errorbar.paints);
        assert!(resource.references_column("y_err"));
        let resource = &active_picker(&renderer).descriptors[1].resource;
        let area = resource.area.as_ref().expect("the fill is picked");
        assert_eq!(area.to_series.as_deref(), Some("y2"));
        assert!(resource.references_column("y2"));
    }
    let generation = active_picker(&renderer).engine.registry_generation();
    renderer
        .set_chart_series(
            chart,
            vec![bars(blue, 1.0), filled(to_lower(), blue), lower.clone()],
        )
        .unwrap();
    assert_eq!(
        active_picker(&renderer).engine.registry_generation(),
        generation,
        "a colour edit that still paints reuses the slots"
    );

    for (what, series) in [
        (
            "a transparent bar",
            vec![bars(clear, 1.0), filled(to_lower(), blue), lower.clone()],
        ),
        (
            "a stem width",
            vec![bars(blue, 3.0), filled(to_lower(), blue), lower.clone()],
        ),
        (
            "a transparent fill",
            vec![bars(blue, 3.0), filled(to_lower(), clear), lower.clone()],
        ),
        (
            "a fill to a baseline",
            vec![
                bars(blue, 3.0),
                filled(LineFill::ToBaseline(0.25), blue),
                lower.clone(),
            ],
        ),
    ] {
        let before = active_picker(&renderer).engine.registry_generation();
        renderer.set_chart_series(chart, series).unwrap();
        assert!(
            active_picker(&renderer).engine.registry_generation() > before,
            "{what} rebuilds the pick slot"
        );
    }
    let resource = &active_picker(&renderer).descriptors[1].resource;
    let area = resource.area.as_ref().unwrap();
    assert_eq!(
        (area.to_series.as_deref(), area.baseline, area.paints),
        (None, 0.25, true)
    );

    assert!(renderer.remove_column("y_err").unwrap());
    assert!(
        !active_picker(&renderer)
            .descriptors
            .iter()
            .any(|descriptor| descriptor.series_id == "bars")
    );
}

fn composite_test_record(rows: usize) -> Vec<(&'static str, Column<f64>)> {
    ["median", "q1", "q3", "whisker_low", "whisker_high"]
        .iter()
        .map(|name| {
            let data: Vec<f64> = (0..rows).map(|row| row as f64).collect();
            let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            (
                *name,
                Column {
                    data,
                    min: 0.0,
                    max,
                },
            )
        })
        .collect()
}

/// Streaming is not extended to records and ragged columns (rule 8): an
/// upsert onto a streamed id and a stream registered onto a record or ragged
/// id both fail with `InvalidSeriesConfig` naming the limitation.
#[test]
fn records_and_ragged_columns_reject_streaming_both_ways() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    renderer
        .register_streamed_columns(vec![state_test_stream("s", 1)])
        .unwrap();
    let columns = composite_test_record(3);
    let fields: Vec<(&str, &dyn ColumnSource)> = columns
        .iter()
        .map(|(name, column)| (*name, column as &dyn ColumnSource))
        .collect();
    let record = RecordSource {
        schema: data_render::BOX_PLOT_RECORD,
        fields: &fields,
    };
    let values = Column {
        data: vec![1.0, 2.0],
        min: 1.0,
        max: 2.0,
    };
    let offsets = Column {
        data: vec![0.0, 2.0],
        min: 0.0,
        max: 2.0,
    };
    let ragged = RaggedSource {
        values: &values,
        offsets: &offsets,
    };
    let streamed = |result: Result<String>| match result {
        Err(FiggyError::InvalidSeriesConfig { reason, .. }) => {
            assert!(
                reason.contains("streaming is not extended to records and ragged columns"),
                "{reason}"
            );
        }
        other => panic!("expected InvalidSeriesConfig, got {other:?}"),
    };
    streamed(renderer.upsert_record("s", &record));
    streamed(renderer.upsert_ragged("s", &ragged));
    assert!(renderer.pool().record("s").is_none());
    assert!(renderer.pool().ragged("s").is_none());

    renderer.upsert_record("box", &record).unwrap();
    renderer.upsert_ragged("outliers", &ragged).unwrap();
    for id in ["box", "outliers"] {
        match renderer.register_streamed_columns(vec![state_test_stream(id, 1)]) {
            Err(FiggyError::InvalidSeriesConfig { reason, .. }) => assert!(
                reason.contains("streaming is not extended to records and ragged columns"),
                "{reason}"
            ),
            other => panic!("expected InvalidSeriesConfig for {id}, got {other:?}"),
        }
        assert!(renderer.logical_column(id).is_none());
    }
}

/// Records, ragged columns and columns share one id space, and the member
/// columns belong to their record: the host cannot insert, replace, remove
/// or reference them as columns.
#[test]
fn member_columns_are_owned_by_their_record_or_ragged_column() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    let columns = composite_test_record(3);
    let fields: Vec<(&str, &dyn ColumnSource)> = columns
        .iter()
        .map(|(name, column)| (*name, column as &dyn ColumnSource))
        .collect();
    let record = RecordSource {
        schema: data_render::BOX_PLOT_RECORD,
        fields: &fields,
    };
    let column = Column {
        data: vec![1.0, 2.0, 3.0],
        min: 1.0,
        max: 3.0,
    };
    renderer.upsert_record("box", &record).unwrap();
    let member = renderer.pool().record("box").unwrap().slot().fields[0]
        .clone()
        .unwrap();

    assert!(matches!(
        renderer.upsert_column(member.clone(), &column),
        Err(FiggyError::ReservedColumnId { .. })
    ));
    assert!(matches!(
        renderer.remove_column(&member),
        Err(FiggyError::ReservedColumnId { .. })
    ));
    assert!(matches!(
        renderer.add_columns(&[(member.as_str(), &column as &dyn ColumnSource)]),
        Err(FiggyError::ReservedColumnId { .. })
    ));
    assert!(matches!(
        renderer.upsert_column("box", &column),
        Err(FiggyError::Pool(data_render::AllocError::DuplicateId(_)))
    ));
    renderer.add_column("x", &column).unwrap();
    assert!(matches!(
        renderer.register_chart(
            crate::default::default_config(),
            vec![state_test_line("member", "x", &member)],
        ),
        Err(FiggyError::UnknownColumn { .. })
    ));
    assert!(matches!(
        renderer.upsert_record("x", &record),
        Err(FiggyError::InvalidSeriesConfig { reason, .. }) if reason.contains("the id is a column")
    ));
    assert_eq!(
        renderer.pool().record("box").unwrap().slot().fields[0].as_deref(),
        Some(member.as_str())
    );

    // Replacing and removing go through the record.
    let revision = renderer.visual_revision();
    renderer.upsert_record("box", &record).unwrap();
    assert!(
        renderer.pool().slot(&member).is_none(),
        "the old member was freed"
    );
    assert!(
        renderer.has_pending_maintenance(),
        "the freed members leave a hole"
    );
    assert!(renderer.remove_record("box").unwrap());
    assert!(!renderer.remove_record("box").unwrap());
    assert!(!renderer.remove_ragged("box").unwrap());
    assert!(renderer.pool().record("box").is_none());
    assert_eq!(
        renderer.visual_revision(),
        revision,
        "no chart reads a record yet"
    );
}

// A series label is a rich text like any title: a math label registers, and
// a malformed math tree is refused with the series named.
#[test]
fn series_labels_with_math_validate_at_registration() {
    let Some(mut renderer) = state_test_renderer() else {
        return;
    };
    add_state_test_xy(&mut renderer, "x", "y", vec![0.0, 1.0]);
    let mut series = state_test_scatter("scatter", None, "x", "y");
    series.label =
        Some(crate::text::RichText::from_tex(r"$\sigma_x^2$", Color::BLACK, 12.0, "").unwrap());
    renderer
        .register_chart(crate::default::default_config(), vec![series.clone()])
        .unwrap();
    let label = series.label.as_mut().unwrap();
    label.segments[2].math_parent = Some(crate::text::MathSlot { id: 4, slot: 0 });
    let result = renderer.register_chart(crate::default::default_config(), vec![series]);
    let Err(FiggyError::InvalidSeriesConfig { series_id, reason }) = result else {
        panic!("a malformed label was accepted: {result:?}");
    };
    assert_eq!(series_id, "scatter");
    assert!(reason.starts_with("label: segment 2"), "{reason}");
}

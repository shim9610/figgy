//! Test-first contracts for original-source GPU cache reuse and invalidation.
use super::*;

// These paths need original ordering/adjacency, not packed visible rows. Cache
// reuse must still rerun their GPU preparation against the current SSOT.
#[test]
fn bounded_source_cache_updates_styled_and_dashed_first_frame_without_source_reads() {
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut failures = Vec::new();
    for kind in [
        "dash",
        "sketch",
        "constellation",
        "milkyway points",
        "histogram",
    ] {
        for edit in ["size", "grid", "range", "inverted", "title"] {
            let mut c = baseline();
            let mut s = declaration("s", "x", "a");
            match kind {
                "dash" => {
                    if let DataRenderType::Line { line } = &mut s.render_type {
                        line.line_style = LineStylePreset::Dash;
                    }
                }
                "histogram" => {
                    s.render_type = DataRenderType::Histogram {
                        bar: DataBarStyleConfig {
                            fill_color: Color::new(1.0, 0.0, 0.0, 1.0),
                            border_color: Color::BLACK,
                            border_width: 1.0,
                            baseline: 0.0,
                            gap_px: 0.0,
                            width_ratio: 1.0,
                            orientation: crate::data_config::BarOrientation::Vertical,
                            bar_style_overrides: None,
                        },
                    }
                }
                "sketch" => c.draw_style = DrawStyle::Sketch(Default::default()),
                "constellation" => {
                    c.draw_style = DrawStyle::Constellation(Default::default());
                    let DataRenderType::Line { line } = s.render_type else {
                        unreachable!()
                    };
                    s.render_type = DataRenderType::ScatterLine {
                        line,
                        scatter: DataScatterStyleConfig {
                            point_color: Color::BLACK,
                            point_shape: ScatterShape::CircleFilled,
                            point_size: 8.0,
                            point_style_index_column: None,
                            point_style_table: None,
                            point_style_overrides: None,
                        },
                    };
                }
                "milkyway points" => {
                    c.draw_style = DrawStyle::Milkyway(Default::default());
                    s.render_type = DataRenderType::Scatter {
                        scatter: DataScatterStyleConfig {
                            point_color: Color::new(1.0, 0.0, 0.0, 1.0),
                            point_shape: ScatterShape::CircleFilled,
                            point_size: 8.0,
                            point_style_index_column: None,
                            point_style_table: None,
                            point_style_overrides: None,
                        },
                    };
                }
                _ => unreachable!(),
            }
            let (mut r, id) = fixture(&c, false, true);
            r.set_chart_series(id, vec![s.clone()]).unwrap();
            let (before, reads) = frame(&mut r, id, false);
            if reads == 0 {
                failures.push(format!("{kind}/{edit}: initial source upload not counted"));
            }
            let allocations = r
                .gpu_memory_usage()
                .creations_of(GpuResourceKind::ViewResident);
            let mut next = c;
            match edit {
                "size" => next.chart_area.0.width = 280,
                "grid" => next.grid.major_x_color = Color::new(0.0, 0.8, 0.1, 1.0),
                "range" => next.bottom_x.max = 0.7,
                "inverted" => next.left_y.inverted = true,
                "title" => {
                    next.chart_title.text = RichText::plain("New title", Color::BLACK, 12.0, "")
                }
                _ => unreachable!(),
            }
            r.set_chart_config(id, next.clone()).unwrap();
            let (actual, reads) = frame(&mut r, id, false);
            let (mut fresh, fresh_id) = fixture(&next, false, false);
            fresh.set_chart_series(fresh_id, vec![s]).unwrap();
            let (expected, _) = frame(&mut fresh, fresh_id, false);
            let label = format!("{kind}/{edit}");
            if actual != expected {
                failures.push(format!("{label}: stale first frame"));
            }
            if actual == before && !(kind == "milkyway points" && edit == "grid") {
                failures.push(format!("{label}: edit invisible"));
            }
            if reads != 0 {
                failures.push(format!("{label}: {reads} unnecessary source requests"));
            }
            if r.gpu_memory_usage()
                .creations_of(GpuResourceKind::ViewResident)
                != allocations
            {
                failures.push(format!("{label}: immutable cache reallocated"));
            }
            let job = r.active_stream_job(id);
            let (settled, reads) = frame(&mut r, id, false);
            assert_eq!(settled, actual, "{label}: required a repair frame");
            assert_eq!(reads, 0, "{label}: settled frame reread source");
            assert_eq!(
                r.active_stream_job(id),
                job,
                "{label}: settled frame restarted"
            );
        }
    }
    drop(font);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn bounded_source_cache_rejects_new_data_aba_and_series_changes() {
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut failures = Vec::new();
    for change in [
        "revision",
        "remove/reregister",
        "series style",
        "chart replacement",
    ] {
        let mut c = baseline();
        c.draw_style = DrawStyle::Sketch(Default::default());
        let (mut r, mut id) = fixture(&c, false, true);
        let (before, _) = frame(&mut r, id, false);
        let (x, mut y) = values();
        let mut revision = 1;
        let mut s = declaration("s", "x", "a");
        match change {
            "revision" => {
                revision = 2;
                r.replace_streamed_columns(vec![source("x", 2), source("a", 2)])
                    .unwrap();
                for v in &mut y.data {
                    *v = 1.0 - *v;
                }
            }
            "remove/reregister" => {
                r.remove_column("a").unwrap();
                r.register_streamed_columns(vec![source("a", 1)]).unwrap();
                r.set_chart_series(id, vec![s.clone()]).unwrap();
                for v in &mut y.data {
                    *v = 1.0 - *v;
                }
            }
            "series style" => {
                let DataRenderType::Line { line } = &mut s.render_type else {
                    unreachable!()
                };
                line.line_color = Color::new(0.0, 0.7, 0.0, 1.0);
                line.line_width = 4.0;
                r.set_chart_series(id, vec![s.clone()]).unwrap();
            }
            "chart replacement" => {
                r.remove_chart(id).unwrap();
                id = r.register_chart(c.clone(), vec![s.clone()]).unwrap();
            }
            _ => unreachable!(),
        }
        let (actual, reads) = frame_values(&mut r, id, false, &x, &y, revision);
        let (mut fresh, fid) = fixture(&c, false, false);
        fresh.set_chart_series(fid, vec![s]).unwrap();
        if revision != 1 {
            fresh
                .replace_streamed_columns(vec![source("x", revision), source("a", revision)])
                .unwrap();
        }
        let (expected, _) = frame_values(&mut fresh, fid, false, &x, &y, revision);
        if reads == 0 {
            failures.push(format!("{change}: invalid cache was reused"));
        }
        if actual != expected {
            failures.push(format!("{change}: first frame has stale pixels"));
        }
        if change != "chart replacement" && actual == before {
            failures.push(format!("{change}: data change invisible"));
        }
    }
    drop(font);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn bounded_source_cache_obeys_budget_and_releases_on_cancel() {
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut c = baseline();
    c.draw_style = DrawStyle::Sketch(Default::default());
    for cap in [0, 127, 128, 1024] {
        let (mut r, id) = fixture(&c, false, true);
        let _ = r.set_auto_resident_working_set_limit(Some(cap));
        let (mut fresh, fid) = fixture(&c, false, false);
        let expected = frame(&mut fresh, fid, false).0;
        for _ in 0..3 {
            assert_eq!(frame(&mut r, id, false).0, expected);
            let usage = r.gpu_memory_usage();
            assert_eq!(
                usage.live_bytes_of(GpuResourceKind::ViewResident),
                if cap >= 128 { 128 } else { 0 }
            );
            assert!(usage.total_bytes() <= 128 * 1024 * 1024);
            r.cancel_streaming_chart(id).unwrap();
            r.end_gpu_frame();
            r.wait_idle();
            r.service_stream_requests();
            r.end_gpu_frame();
            r.wait_idle();
            assert_eq!(
                r.gpu_memory_usage()
                    .live_bytes_of(GpuResourceKind::ViewResident),
                0
            );
            assert_eq!(
                r.gpu_memory_usage()
                    .retired_bytes_of(GpuResourceKind::ViewResident),
                0
            );
        }
    }
    drop(font);
}

#[test]
fn bounded_source_cache_disabled_policy_releases_completed_cache() {
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut c = baseline();
    c.draw_style = DrawStyle::Sketch(Default::default());
    let (mut r, id) = fixture(&c, false, true);
    frame(&mut r, id, false);
    assert_eq!(
        r.gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::ViewResident),
        128
    );
    let _ = r.set_auto_resident_working_set_limit(Some(0));
    frame(&mut r, id, false);
    assert_eq!(
        r.gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::ViewResident),
        0
    );
    c.chart_area.0.width -= 20;
    r.set_chart_config(id, c).unwrap();
    assert!(frame(&mut r, id, false).1 > 0);
    drop(font);
}

#[test]
fn bounded_source_cache_never_publishes_discarded_gpu_copies() {
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut c = baseline();
    c.draw_style = DrawStyle::Sketch(Default::default());
    let (mut r, id) = fixture(&c, false, true);
    let view = r
        .create_chart_view(&Chart::new(c.clone()), c.chart_area.0)
        .unwrap();
    r.request_auto_streaming_chart(id, &view, options())
        .unwrap();
    let first = match r.auto_stream_chart_request_ranges(id).unwrap() {
        crate::AutoStreamingRangeRequest::Ready { ranges, .. } => ranges,
        _ => panic!("source request required"),
    };
    let (x, y) = values();
    let bindings = [
        crate::StreamSourceBinding {
            id: "x",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&x),
        },
        crate::StreamSourceBinding {
            id: "a",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];
    r.reject_next_stream_completion_reserve_for_test();
    assert!(r.auto_stream_chart_step(id, &bindings).is_err());
    // accept/upload recorded the cache copy, but queue submission failed. A
    // cache hit here would bind zero/uninitialized GPU data instead of retrying.
    let retry = match r.auto_stream_chart_request_ranges(id).unwrap() {
        crate::AutoStreamingRangeRequest::Ready { ranges, .. } => ranges,
        _ => panic!("discarded copy must NOT satisfy a source request"),
    };
    assert_eq!(first, retry);
    let actual = frame(&mut r, id, false).0;
    let (mut fresh, fid) = fixture(&c, false, false);
    assert_eq!(actual, frame(&mut fresh, fid, false).0);
    drop(font);
}

#[test]
fn bounded_source_cache_resizes_physical_target_and_dpr_preserving_arc_boundaries() {
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let rows = 519;
    let x = crate::Column {
        data: (0..rows).map(|i| i as f32 / (rows - 1) as f32).collect(),
        min: 0.0,
        max: 1.0,
    };
    let y = crate::Column {
        data: (0..rows)
            .map(|i| {
                if i == 255 {
                    f32::NAN
                } else {
                    0.5 + (i as f32 * 0.031).sin() * 0.37
                }
            })
            .collect(),
        min: 0.0,
        max: 1.0,
    };
    let bindings = [
        crate::StreamSourceBinding {
            id: "x",
            revision: 2,
            source: crate::StreamColumnSource::Scalar(&x),
        },
        crate::StreamSourceBinding {
            id: "a",
            revision: 2,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];
    let prepare_sources = |r: &mut Renderer| {
        r.replace_streamed_columns(vec![
            crate::StreamColumn {
                len: rows as u64,
                ..source("x", 2)
            },
            crate::StreamColumn {
                len: rows as u64,
                ..source("a", 2)
            },
        ])
        .unwrap();
    };
    let render = |r: &mut Renderer, id, document: &Config, size| {
        let (config, panel, scale) = display_config_for_surface(document, size);
        let view = r
            .create_chart_view(&Chart::new(config.clone()), panel)
            .unwrap();
        r.request_auto_streaming_chart_with_display_scale(
            id,
            &view,
            config,
            scale,
            crate::StreamingChartOptions {
                size,
                clear_color: Color::WHITE,
                max_primitives_per_chunk: 17,
            },
        )
        .unwrap();
        let mut reads = 0;
        let mut done = false;
        for _ in 0..4096 {
            match r.auto_stream_chart_request_ranges(id).unwrap() {
                crate::AutoStreamingRangeRequest::Ready { .. } => {
                    reads += 1;
                    r.auto_stream_chart_step(id, &bindings).unwrap();
                }
                crate::AutoStreamingRangeRequest::Backpressure { .. } => wait_stream_slots(r, 0),
                crate::AutoStreamingRangeRequest::AllSubmitted { .. }
                | crate::AutoStreamingRangeRequest::Complete { .. } => {
                    done = true;
                    break;
                }
            }
        }
        assert!(done, "cached replay failed to finish");
        wait_stream_slots(r, 0);
        r.prepare_registered(&[RegisteredChartDrawItem {
            chart_id: id,
            view: &view,
        }])
        .unwrap();
        assert!(matches!(
            r.auto_stream_chart_request_ranges(id).unwrap(),
            crate::AutoStreamingRangeRequest::Complete { .. }
        ));
        let job = r.active_stream_job(id).unwrap();
        let target = wgpu::Texture::clone(r.chart_stream_display(job).unwrap());
        (read_draw_target(r, &target), reads)
    };
    for samples in [1, 4] {
        let mut c = baseline();
        c.draw_style = DrawStyle::Sketch(Default::default());
        let (mut r, id) = fixture_samples(&c, false, true, samples);
        prepare_sources(&mut r);
        assert!(render(&mut r, id, &c, (320, 240)).1 > 0);
        let allocations = r
            .gpu_memory_usage()
            .creations_of(GpuResourceKind::ViewResident);
        for size in [(640, 480), (260, 500), (900, 300), (320, 240)] {
            let (actual, reads) = render(&mut r, id, &c, size);
            assert_eq!(reads, 0, "MSAA {samples}, target {size:?}: source reread");
            assert_eq!(
                r.gpu_memory_usage()
                    .creations_of(GpuResourceKind::ViewResident),
                allocations
            );
            let (mut fresh, fid) = fixture_samples(&c, false, false, samples);
            prepare_sources(&mut fresh);
            let expected = render(&mut fresh, fid, &c, size).0;
            assert_eq!(
                actual, expected,
                "MSAA {samples}, target {size:?}: arc/carry or clipping mismatch"
            );
            assert!(
                actual
                    .chunks_exact(4)
                    .any(|p| p[0] > 150 && p[1] < 80 && p[2] < 80),
                "missing curve"
            );
        }
    }
    drop(font);
}

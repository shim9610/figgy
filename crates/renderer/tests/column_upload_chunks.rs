//! A resident column uploaded in chunks (#311): `begin_column_upload` holds
//! one mapped staging buffer, `ColumnUpload::write_range` writes the chunks
//! into it in index order, and `commit_column_upload` installs the column as
//! one upsert. Overlapping, gapped, oversized and mistyped chunks are
//! refused without changing the upload; an incomplete, stale or foreign
//! upload is refused at commit without changing the renderer; a budget that
//! cannot hold the staging buffer refuses the upload with a pool error.
//! `inspect_streaming_candidate` checks a candidate with the rules
//! `inspect_streaming_capability` applies to a registered chart.
//!
//! `begin_replace_chart_sources` (#311) uploads plain columns, records and
//! ragged columns, removes ids and replaces a chart in one transaction:
//! the commit installs everything, a failure in a late upload or a budget
//! below the peak rolls everything back, removed ids stay until the commit,
//! and the batch has no count limit.
//!
//! `commit_column_uploads` (#334) installs several chunked uploads and the
//! chart's config and series in that transaction: a stale revision on any
//! column, a budget below the peak or a state that does not hold refuses
//! all of it, a dropped transaction changes nothing, and the number of
//! staged columns is not limited.
#![cfg(feature = "helpers")]

use renderer::{
    AllocError, BOX_PLOT_RECORD, BatchSource, Chart, ChartId, Color, ColumnSource, Config,
    FiggyError, OverlayLayer, RaggedSource, RecordSource, Renderer, SeriesConfig,
    StreamColumnSource, StreamEncoding,
};

fn wave(n: usize) -> (Vec<f64>, Vec<f64>) {
    let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let y: Vec<f64> = x.iter().map(|x| (x * 0.01).sin() * 3.0 + 1.0).collect();
    (x, y)
}

/// Write `values` at `start` as one hi/lo chunk.
fn put(upload: &mut renderer::ColumnUpload, start: u64, values: &[f64]) -> renderer::Result<()> {
    upload.write_range(start, StreamColumnSource::HiLo(&values))
}

/// Write `values` in chunks of `chunk` values, in order.
fn write_all(upload: &mut renderer::ColumnUpload, values: &[f64], chunk: usize) {
    for (index, part) in values.chunks(chunk).enumerate() {
        put(upload, (index * chunk) as u64, part).expect("chunk in order");
    }
}

#[test]
fn a_chunked_upload_installs_the_column_a_single_upload_would() {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    // Past any size a fixed staging chunk would hold.
    let n = 300_000;
    let (x, y) = wave(n);
    renderer.upsert_column("x", &x).expect("x");

    let mut upload = renderer
        .begin_column_upload("y", n, StreamEncoding::HiLoF32, None)
        .expect("begin");
    assert_eq!(upload.len(), n);
    assert!(
        renderer.logical_column("y").is_none(),
        "nothing before commit"
    );
    write_all(&mut upload, &y, 65_536);
    assert_eq!(upload.written(), n);
    renderer.commit_column_upload(upload).expect("commit");

    renderer.upsert_column("y_whole", &y).expect("whole");
    let pool = renderer.pool();
    let staged = pool.slot("y").expect("staged slot");
    let whole = pool.slot("y_whole").expect("whole slot");
    assert_eq!(staged.len_values, n);
    assert_eq!(staged.min, whole.min);
    assert_eq!(staged.max, whole.max);
    assert_eq!(staged.min_positive, whole.min_positive);

    // A chart draws every value of it.
    let series = vec![
        SeriesConfig::line("wave", "x", "y")
            .color(Color::from_rgb8(30, 60, 200))
            .width(1.0),
    ];
    let mut chart = Chart::sized(320, 200);
    chart.set_x_range(0.0, n as f64);
    chart.set_y_range(-3.0, 5.0);
    let id = renderer
        .register_chart(chart.config().clone(), series)
        .expect("register");
    assert!(!renderer.series_draw_info(id, "wave").unwrap().truncated);

    // Replacing it takes the revision it holds; the replacement renews it.
    let revision = renderer.column_revision("y");
    assert!(revision.is_some());
    let mut again = renderer
        .begin_column_upload("y", 3, StreamEncoding::HiLoF32, revision)
        .expect("replace");
    put(&mut again, 0, &[1.0, 2.0, 3.0]).unwrap();
    renderer
        .commit_column_upload(again)
        .expect("commit replace");
    assert_ne!(renderer.column_revision("y"), revision);
    assert_eq!(renderer.pool().slot("y").unwrap().len_values, 3);
}

#[test]
fn refused_chunks_change_nothing() {
    let renderer = Renderer::headless_blocking().expect("headless renderer");
    let mut upload = renderer
        .begin_column_upload("v", 10, StreamEncoding::HiLoF32, None)
        .expect("begin");
    put(&mut upload, 0, &[0.0; 4]).unwrap();
    let refused = |result: renderer::Result<()>| {
        assert!(
            matches!(result, Err(FiggyError::InvalidColumnUpload { ref id, .. }) if id == "v"),
            "{result:?}"
        );
    };
    // Overlap, gap, past the end, empty, and the other encoding.
    refused(put(&mut upload, 2, &[1.0; 2]));
    refused(put(&mut upload, 5, &[1.0; 2]));
    refused(put(&mut upload, 4, &[1.0; 7]));
    refused(put(&mut upload, 4, &[]));
    let scalar_chunk: &[f32] = &[1.0, 2.0];
    refused(upload.write_range(4, StreamColumnSource::Scalar(&scalar_chunk)));
    assert_eq!(upload.written(), 4);
    // The next chunk in order still goes in.
    put(&mut upload, 4, &[1.0; 6]).unwrap();
    assert_eq!(upload.written(), 10);

    let scalar = renderer
        .begin_column_upload("s", 2, StreamEncoding::ScalarF32, None)
        .expect("begin scalar");
    assert_eq!(scalar.encoding(), StreamEncoding::ScalarF32);
    // An empty upload is refused at once.
    assert!(matches!(
        renderer.begin_column_upload("e", 0, StreamEncoding::HiLoF32, None),
        Err(FiggyError::Pool(AllocError::EmptySource))
    ));
}

#[test]
fn incomplete_stale_and_foreign_uploads_are_refused_at_commit() {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let mut other = Renderer::headless_blocking().expect("second renderer");

    // Incomplete: nothing is registered.
    let mut upload = renderer
        .begin_column_upload("v", 4, StreamEncoding::HiLoF32, None)
        .unwrap();
    put(&mut upload, 0, &[1.0, 2.0]).unwrap();
    assert!(matches!(
        renderer.commit_column_upload(upload),
        Err(FiggyError::InvalidColumnUpload { .. })
    ));
    assert!(renderer.logical_column("v").is_none());

    // Stale at begin: the id is registered but the upload expects a new id,
    // or expects a revision it does not hold.
    renderer.upsert_column("v", &vec![1.0f64, 2.0]).unwrap();
    let revision = renderer.column_revision("v");
    assert!(matches!(
        renderer.begin_column_upload("v", 2, StreamEncoding::HiLoF32, None),
        Err(FiggyError::StaleStateToken { .. })
    ));
    assert!(matches!(
        renderer.begin_column_upload("w", 2, StreamEncoding::HiLoF32, Some(1)),
        Err(FiggyError::StaleStateToken { .. })
    ));

    // Stale at commit: the column changed while the upload was open.
    let mut upload = renderer
        .begin_column_upload("v", 2, StreamEncoding::HiLoF32, revision)
        .unwrap();
    put(&mut upload, 0, &[7.0, 8.0]).unwrap();
    renderer.upsert_column("v", &vec![3.0f64, 4.0]).unwrap();
    let current = renderer.column_revision("v");
    assert!(matches!(
        renderer.commit_column_upload(upload),
        Err(FiggyError::StaleStateToken { .. })
    ));
    assert_eq!(renderer.column_revision("v"), current);
    assert_eq!(renderer.pool().slot("v").unwrap().max, 4.0);

    // Foreign: another renderer refuses it.
    let mut upload = renderer
        .begin_column_upload("n", 1, StreamEncoding::HiLoF32, None)
        .unwrap();
    put(&mut upload, 0, &[1.0]).unwrap();
    assert!(matches!(
        other.commit_column_upload(upload),
        Err(FiggyError::StaleStateToken { .. })
    ));
    assert!(other.logical_column("n").is_none());
}

#[test]
fn a_budget_that_cannot_hold_the_staging_buffer_refuses_the_upload() {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let current = renderer.gpu_memory_usage().total_bytes();
    assert!(renderer.set_memory_budget(Some(current)).is_none());
    assert!(matches!(
        renderer.begin_column_upload("big", 1 << 20, StreamEncoding::HiLoF32, None),
        Err(FiggyError::Pool(AllocError::BudgetExceeded { .. }))
    ));
    assert!(renderer.logical_column("big").is_none());
    assert_eq!(renderer.gpu_memory_usage().total_bytes(), current);
}

#[test]
fn a_candidate_streaming_check_applies_the_registered_chart_rules() {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let (x, y) = wave(16);
    renderer.upsert_column("x", &x).unwrap();
    renderer.upsert_column("y", &y).unwrap();
    let series = vec![SeriesConfig::line("wave", "x", "y")];
    let config = Chart::sized(320, 200).config().clone();
    let id = renderer
        .register_chart(config.clone(), series.clone())
        .unwrap();
    let registered = renderer.chart_config(id).unwrap().clone();
    assert_eq!(
        Renderer::inspect_streaming_candidate(&config, &series).is_ok(),
        renderer.inspect_streaming_capability(id).is_ok()
    );

    // A layered candidate is refused with the rule the chart check names,
    // and nothing registered changes.
    let mut layered = config.clone();
    layered.overlays.push(OverlayLayer::default());
    let refused = Renderer::inspect_streaming_candidate(&layered, &series);
    assert!(
        matches!(
            refused,
            Err(FiggyError::InvalidConfig {
                field: "overlays",
                ..
            })
        ),
        "{refused:?}"
    );
    assert_eq!(renderer.chart_config(id).unwrap(), &registered);
}

// ---- one transaction for columns, records, ragged columns and removals ----

/// The five required box plot fields of `rows` rows.
fn box_fields(rows: usize, base: f64) -> [Vec<f64>; 5] {
    std::array::from_fn(|field| {
        (0..rows)
            .map(|row| base + field as f64 * 10.0 + row as f64)
            .collect()
    })
}

fn record_source<'a>(names: &'a [(&'a str, &'a dyn ColumnSource); 5]) -> RecordSource<'a> {
    RecordSource {
        schema: BOX_PLOT_RECORD,
        fields: names,
    }
}

fn named<'a>(fields: &'a [Vec<f64>; 5]) -> [(&'a str, &'a dyn ColumnSource); 5] {
    let names = ["median", "q1", "q3", "whisker_low", "whisker_high"];
    std::array::from_fn(|k| (names[k], &fields[k] as &dyn ColumnSource))
}

/// A renderer holding a previous "document": columns `x` and `y` a chart
/// reads, a column, a record and a ragged column to remove.
fn previous_document() -> (Renderer, ChartId, Config) {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let (x, y) = wave(64);
    renderer.upsert_column("x", &x).unwrap();
    renderer.upsert_column("y", &y).unwrap();
    renderer.upsert_column("old_col", &vec![1.0, 2.0]).unwrap();
    let fields = box_fields(3, 0.0);
    renderer
        .upsert_record("old_rec", &record_source(&named(&fields)))
        .unwrap();
    let (values, offsets) = (vec![1.0, 2.0, 3.0], vec![0.0, 1.0, 3.0]);
    renderer
        .upsert_ragged(
            "old_rag",
            &RaggedSource {
                values: &values,
                offsets: &offsets,
            },
        )
        .unwrap();
    let config = Chart::sized(320, 200).config().clone();
    let id = renderer
        .register_chart(config.clone(), vec![SeriesConfig::line("wave", "x", "y")])
        .unwrap();
    (renderer, id, config)
}

/// What a rolled-back transaction must leave as it was.
#[derive(Debug, PartialEq)]
struct PoolState {
    ids: Vec<String>,
    lens: Vec<usize>,
    generation: u32,
    capacity: u64,
    record: Option<u32>,
    ragged: Option<u32>,
    series: Vec<SeriesConfig>,
}

fn pool_state(renderer: &Renderer, chart: ChartId) -> PoolState {
    let pool = renderer.pool();
    let mut ids: Vec<String> = renderer.logical_column_ids().map(str::to_owned).collect();
    ids.sort();
    let lens = ids
        .iter()
        .map(|id| pool.slot(id).map_or(0, |slot| slot.len_values))
        .collect();
    PoolState {
        ids,
        lens,
        generation: pool.generation(),
        capacity: pool.capacity(),
        record: pool.record_stats("old_rec").map(|stats| stats.len),
        ragged: pool.ragged_stats("old_rag").map(|stats| stats.groups),
        series: renderer.chart_series(chart).unwrap().to_vec(),
    }
}

#[test]
fn a_mixed_transaction_installs_every_upload_and_removes_the_given_ids_at_commit() {
    let (mut renderer, chart, config) = previous_document();
    let n = 200_000;
    let (x, y) = wave(n);
    // Single lane: staged compact and expanded on the GPU (#317).
    let z: Vec<f32> = (0..n).map(|i| (i % 7) as f32).collect();
    let fields = box_fields(4, 100.0);
    let names = named(&fields);
    let (values, offsets) = (vec![5.0, 6.0, 7.0, 8.0], vec![0.0, 2.0, 2.0, 3.0, 4.0]);
    let uploads = [
        ("x", BatchSource::Column(&x)),
        ("y", BatchSource::Column(&y)),
        ("z", BatchSource::Column(&z)),
        ("rec", BatchSource::Record(record_source(&names))),
        (
            "rag",
            BatchSource::Ragged(RaggedSource {
                values: &values,
                offsets: &offsets,
            }),
        ),
    ];
    let series = vec![
        SeriesConfig::line("wave", "x", "y"),
        SeriesConfig::line("steps", "x", "z"),
    ];
    let guard = renderer
        .begin_replace_chart_sources(
            chart,
            &uploads,
            &["old_col", "old_rec", "old_rag"],
            |pool| {
                // The candidate holds the uploads and none of the removals.
                assert_eq!(pool.slot("x").unwrap().len_values, n);
                assert!(pool.record("rec").is_some());
                assert!(pool.slot("old_col").is_none());
                assert!(pool.record("old_rec").is_none());
                let mut fitted = Chart::new(config);
                fitted.auto_fit_x(pool, "x", 0.0)?;
                Ok((fitted.config().clone(), series.clone()))
            },
        )
        .expect("begin");
    assert_eq!(guard.pool().slot("z").unwrap().len_values, n);
    guard.commit();

    let pool = renderer.pool();
    assert_eq!(pool.slot("x").unwrap().len_values, n);
    assert_eq!(pool.slot("y").unwrap().len_values, n);
    let z_slot = pool.slot("z").unwrap();
    assert_eq!((z_slot.len_values, z_slot.min, z_slot.max), (n, 0.0, 6.0));
    let record = pool.record_stats("rec").expect("record");
    assert_eq!(record.len, 4);
    assert_eq!((record.min[0], record.max[0]), (100.0, 103.0));
    let ragged = pool.ragged_stats("rag").expect("ragged");
    assert_eq!((ragged.groups, ragged.total, ragged.max_group), (4, 4, 2));
    assert!(pool.slot("old_col").is_none());
    assert!(pool.record("old_rec").is_none());
    assert!(pool.ragged("old_rag").is_none());
    assert_eq!(renderer.chart_series(chart).unwrap(), &series[..]);
    assert_eq!(
        renderer.chart_config(chart).unwrap().bottom_x.max,
        (n - 1) as f64
    );
    assert!(!renderer.series_draw_info(chart, "steps").unwrap().truncated);
}

#[test]
fn a_failure_in_a_late_upload_rolls_back_every_upload_and_keeps_the_removals() {
    let (mut renderer, chart, config) = previous_document();
    let before = pool_state(&renderer, chart);
    let total = renderer.gpu_memory_usage().live_bytes();
    let (x, y) = wave(1_000);
    let fields = box_fields(2, 0.0);
    let names = named(&fields);
    // Offsets that decrease: refused while they are written, after every
    // earlier upload was staged.
    let (values, offsets) = (vec![1.0, 2.0, 3.0], vec![0.0, 3.0, 1.0, 3.0]);
    let uploads = [
        ("x", BatchSource::Column(&x)),
        ("y", BatchSource::Column(&y)),
        ("new", BatchSource::Column(&y)),
        ("rec", BatchSource::Record(record_source(&names))),
        (
            "rag",
            BatchSource::Ragged(RaggedSource {
                values: &values,
                offsets: &offsets,
            }),
        ),
    ];
    let mut prepared = false;
    let result = renderer.begin_replace_chart_sources(
        chart,
        &uploads,
        &["old_col", "old_rec", "old_rag"],
        |_| {
            prepared = true;
            Ok((config.clone(), vec![SeriesConfig::line("wave", "x", "new")]))
        },
    );
    assert!(
        matches!(result, Err(FiggyError::InvalidSeriesConfig { ref series_id, .. }) if series_id == "rag"),
        "{:?}",
        result.err()
    );
    assert!(!prepared, "the fit runs after every upload succeeded");
    assert_eq!(pool_state(&renderer, chart), before);
    assert!(renderer.pool().slot("new").is_none());
    assert!(renderer.pool().record("rec").is_none());
    assert_eq!(renderer.gpu_memory_usage().live_bytes(), total);

    // A failing fit also leaves everything as it was.
    let result = renderer.begin_replace_chart_sources(chart, &uploads[..3], &["old_col"], |_| {
        Err(FiggyError::Internal {
            invariant: "test fit",
        })
    });
    assert!(matches!(result, Err(FiggyError::Internal { .. })));
    assert_eq!(pool_state(&renderer, chart), before);
    assert_eq!(renderer.gpu_memory_usage().live_bytes(), total);

    // So do series the candidate cannot hold: a removed column is read.
    let result = renderer.begin_replace_chart_sources(chart, &uploads[..3], &["old_col"], |_| {
        Ok((
            config.clone(),
            vec![SeriesConfig::line("wave", "x", "old_col")],
        ))
    });
    assert!(result.is_err());
    assert_eq!(pool_state(&renderer, chart), before);
}

#[test]
fn a_budget_below_the_transaction_peak_refuses_it_before_any_buffer() {
    let (mut renderer, chart, config) = previous_document();
    let before = pool_state(&renderer, chart);
    let current = renderer.gpu_memory_usage().total_bytes();
    assert!(renderer.set_memory_budget(Some(current)).is_none());
    let (x, y) = wave(1_000);
    let fields = box_fields(2, 0.0);
    let names = named(&fields);
    let uploads = [
        ("x", BatchSource::Column(&x)),
        ("y", BatchSource::Column(&y)),
        ("rec", BatchSource::Record(record_source(&names))),
    ];
    let result = renderer.begin_replace_chart_sources(chart, &uploads, &["old_col"], |_| {
        Ok((config.clone(), vec![SeriesConfig::line("wave", "x", "y")]))
    });
    assert!(
        matches!(
            result,
            Err(FiggyError::Pool(AllocError::BudgetExceeded { .. }))
        ),
        "{:?}",
        result.err()
    );
    assert_eq!(pool_state(&renderer, chart), before);
    assert_eq!(renderer.gpu_memory_usage().total_bytes(), current);
}

#[test]
fn the_transaction_refuses_streamed_ids_and_ids_twice() {
    let (mut renderer, chart, config) = previous_document();
    renderer
        .register_streamed_columns(vec![renderer::StreamColumn {
            id: "streamed".into(),
            len: 4,
            encoding: StreamEncoding::HiLoF32,
            replay: renderer::StreamReplay::Sequential,
            revision: 1,
            statistics: renderer::StreamStatistics::Unknown,
        }])
        .unwrap();
    let before = pool_state(&renderer, chart);
    let values = vec![1.0, 2.0, 3.0, 4.0];
    let state = || Ok((config.clone(), vec![SeriesConfig::line("wave", "x", "y")]));
    for (uploads, removals) in [
        (&[("streamed", BatchSource::Column(&values))][..], &[][..]),
        (&[][..], &["streamed"][..]),
        (
            &[
                ("a", BatchSource::Column(&values)),
                ("a", BatchSource::Column(&values)),
            ][..],
            &[][..],
        ),
        (
            &[("old_col", BatchSource::Column(&values))][..],
            &["old_col"][..],
        ),
        (&[][..], &["nothing"][..]),
    ] {
        assert!(
            renderer
                .begin_replace_chart_sources(chart, uploads, removals, |_| state())
                .is_err()
        );
        assert_eq!(pool_state(&renderer, chart), before);
    }
}

#[test]
fn a_transaction_carries_any_number_of_uploads() {
    let (mut renderer, chart, config) = previous_document();
    // Past any small fixed count of columns, records or ragged columns.
    let count = 300;
    let ids: Vec<String> = (0..count).map(|k| format!("c{k}")).collect();
    let records: Vec<String> = (0..count).map(|k| format!("r{k}")).collect();
    let columns: Vec<Vec<f64>> = (0..count).map(|k| vec![k as f64; 3]).collect();
    let fields = box_fields(3, 0.0);
    let names = named(&fields);
    let mut uploads: Vec<(&str, BatchSource)> = ids
        .iter()
        .zip(&columns)
        .map(|(id, column)| (id.as_str(), BatchSource::Column(column)))
        .collect();
    uploads.extend(
        records
            .iter()
            .map(|id| (id.as_str(), BatchSource::Record(record_source(&names)))),
    );
    let series: Vec<SeriesConfig> = ids
        .iter()
        .map(|id| SeriesConfig::line(format!("s{id}"), "c0", id.as_str()))
        .collect();
    renderer
        .begin_replace_chart_sources(chart, &uploads, &["old_col"], |_| {
            Ok((config.clone(), series.clone()))
        })
        .expect("begin")
        .commit();
    let pool = renderer.pool();
    for (k, id) in ids.iter().enumerate() {
        assert_eq!(pool.slot(id).unwrap().max, k as f64);
    }
    for id in &records {
        assert_eq!(pool.record_stats(id).unwrap().len, 3);
    }
    assert_eq!(renderer.chart_series(chart).unwrap().len(), count);
    let last = format!("sc{}", count - 1);
    assert!(!renderer.series_draw_info(chart, &last).unwrap().truncated);
}

// ---- several chunked uploads and the chart state in one commit (#334) ----

/// A chunked upload of `values` replacing the column at `revision`.
fn staged(
    renderer: &Renderer,
    id: &str,
    values: &[f64],
    revision: Option<u64>,
) -> renderer::ColumnUpload {
    let mut upload = renderer
        .begin_column_upload(id, values.len(), StreamEncoding::HiLoF32, revision)
        .expect("begin upload");
    write_all(&mut upload, values, 7);
    upload
}

/// What a refused transaction must leave as it was, beyond the pool.
#[derive(Debug, PartialEq)]
struct ChartState {
    pool: PoolState,
    revisions: [Option<u64>; 2],
    config: Config,
    maxima: [f64; 2],
}

fn chart_state(renderer: &Renderer, chart: ChartId) -> ChartState {
    let pool = renderer.pool();
    ChartState {
        pool: pool_state(renderer, chart),
        revisions: [renderer.column_revision("x"), renderer.column_revision("y")],
        config: renderer.chart_config(chart).unwrap().clone(),
        maxima: [pool.slot("x").unwrap().max, pool.slot("y").unwrap().max],
    }
}

#[test]
fn staged_columns_and_the_chart_state_commit_together() {
    let (mut renderer, chart, config) = previous_document();
    let before = [renderer.column_revision("x"), renderer.column_revision("y")];
    let n = 100;
    let x: Vec<f64> = (0..n).map(|i| i as f64 * 2.0).collect();
    let y: Vec<f64> = (0..n).map(|i| 50.0 - i as f64).collect();
    let uploads = vec![
        staged(&renderer, "x", &x, before[0]),
        staged(&renderer, "y", &y, before[1]),
    ];
    let mut next = Chart::new(config);
    next.set_x_range(0.0, 200.0);
    next.set_y_range(-60.0, 60.0);
    let series = vec![SeriesConfig::line("next", "x", "y")];
    renderer
        .commit_column_uploads(
            chart,
            uploads,
            Some((next.config().clone(), series.clone())),
        )
        .expect("commit");

    let after = [renderer.column_revision("x"), renderer.column_revision("y")];
    assert!(after[0] != before[0] && after[1] != before[1], "{after:?}");
    let pool = renderer.pool();
    assert_eq!(pool.slot("x").unwrap().len_values, n);
    assert_eq!(pool.slot("x").unwrap().max, 198.0);
    assert_eq!(pool.slot("y").unwrap().min, -49.0);
    assert_eq!(renderer.chart_series(chart).unwrap(), series.as_slice());
    assert_eq!(renderer.chart_config(chart).unwrap().bottom_x.max, 200.0);
    assert!(!renderer.series_draw_info(chart, "next").unwrap().truncated);

    // Without a state the chart keeps its own: only the columns move.
    let kept = renderer.chart_config(chart).unwrap().clone();
    let again = vec![staged(&renderer, "y", &[1.0, 2.0, 3.0], after[1])];
    renderer
        .commit_column_uploads(chart, again, None)
        .expect("commit without a state");
    assert_ne!(renderer.column_revision("y"), after[1]);
    assert_eq!(renderer.column_revision("x"), after[0]);
    assert_eq!(renderer.chart_config(chart).unwrap(), &kept);
    assert_eq!(renderer.chart_series(chart).unwrap(), series.as_slice());
    assert_eq!(renderer.pool().slot("y").unwrap().len_values, 3);
}

#[test]
fn a_stale_revision_on_one_column_refuses_every_column() {
    let (mut renderer, chart, config) = previous_document();
    let x_revision = renderer.column_revision("x");
    let y_revision = renderer.column_revision("y");
    let uploads = vec![
        staged(&renderer, "x", &[9.0, 10.0], x_revision),
        staged(&renderer, "y", &[11.0, 12.0], y_revision),
    ];
    // y changes while the transaction is open.
    renderer.upsert_column("y", &vec![5.0f64, 6.0]).unwrap();
    let before = chart_state(&renderer, chart);
    let result = renderer.commit_column_uploads(
        chart,
        uploads,
        Some((config, vec![SeriesConfig::line("next", "x", "y")])),
    );
    assert!(
        matches!(result, Err(FiggyError::StaleStateToken { ref reason }) if reason.contains("y")),
        "{result:?}"
    );
    assert_eq!(chart_state(&renderer, chart), before);
    assert_eq!(renderer.column_revision("x"), x_revision);
}

#[test]
fn a_budget_below_the_peak_refuses_the_staged_columns_at_commit() {
    let (mut renderer, chart, config) = previous_document();
    // Live bytes: a released buffer is retired until the queue is done.
    let idle = renderer.gpu_memory_usage().live_bytes();
    let (x, y) = wave(1_000);
    let uploads = vec![
        staged(&renderer, "x", &x, renderer.column_revision("x")),
        staged(&renderer, "y", &y, renderer.column_revision("y")),
    ];
    let held = renderer.gpu_memory_usage().total_bytes();
    assert!(
        renderer.gpu_memory_usage().live_bytes() > idle,
        "the staging is charged while open"
    );
    // The staging fits; the candidate slab on top of it does not.
    assert!(renderer.set_memory_budget(Some(held)).is_none());
    let before = chart_state(&renderer, chart);
    let result = renderer.commit_column_uploads(
        chart,
        uploads,
        Some((config, vec![SeriesConfig::line("wave", "x", "y")])),
    );
    assert!(
        matches!(
            result,
            Err(FiggyError::Pool(AllocError::BudgetExceeded { .. }))
        ),
        "{result:?}"
    );
    assert_eq!(chart_state(&renderer, chart), before);
    // The refused uploads were consumed and their staging released.
    assert_eq!(renderer.gpu_memory_usage().live_bytes(), idle);
}

#[test]
fn a_dropped_transaction_changes_nothing_and_leaves_its_uploads_usable() {
    let (mut renderer, chart, config) = previous_document();
    // Live bytes: a released buffer is retired until the queue is done.
    let idle = renderer.gpu_memory_usage().live_bytes();
    let before = chart_state(&renderer, chart);
    let revision = renderer.column_revision("y");

    // Cancelled before the commit: the uploads are dropped.
    let uploads = vec![
        staged(&renderer, "x", &[1.0, 2.0], renderer.column_revision("x")),
        staged(&renderer, "y", &[3.0, 4.0], revision),
    ];
    drop(uploads);
    assert_eq!(chart_state(&renderer, chart), before);
    assert_eq!(renderer.gpu_memory_usage().live_bytes(), idle);

    // A batch that holds an upload, dropped before its commit, rolls back;
    // the upload, already copied from, still installs on its own.
    let upload = staged(&renderer, "y", &[3.0, 4.0, 5.0], revision);
    {
        let guard = renderer
            .begin_replace_chart_sources(chart, &[("y", BatchSource::Staged(&upload))], &[], |_| {
                Ok((config.clone(), vec![SeriesConfig::line("wave", "x", "y")]))
            })
            .expect("begin");
        assert_eq!(guard.pool().slot("y").unwrap().len_values, 3);
    }
    assert_eq!(chart_state(&renderer, chart), before);
    renderer
        .commit_column_upload(upload)
        .expect("the upload installs after the rollback");
    assert_eq!(renderer.pool().slot("y").unwrap().max, 5.0);

    // An upload under another id than its own is refused.
    let upload = staged(&renderer, "x", &[1.0], renderer.column_revision("x"));
    let state = chart_state(&renderer, chart);
    assert!(matches!(
        renderer.begin_replace_chart_sources(
            chart,
            &[("y", BatchSource::Staged(&upload))],
            &[],
            |_| Ok((config.clone(), vec![SeriesConfig::line("wave", "x", "y")])),
        ),
        Err(FiggyError::InvalidColumnUpload { .. })
    ));
    assert_eq!(chart_state(&renderer, chart), state);
}

#[test]
fn a_state_that_does_not_hold_against_the_staged_columns_changes_nothing() {
    let (mut renderer, chart, config) = previous_document();
    let before = chart_state(&renderer, chart);

    // A series that reads a column nothing registers.
    let uploads = vec![staged(
        &renderer,
        "x",
        &[1.0, 2.0],
        renderer.column_revision("x"),
    )];
    let result = renderer.commit_column_uploads(
        chart,
        uploads,
        Some((
            config.clone(),
            vec![SeriesConfig::line("next", "x", "nope")],
        )),
    );
    assert!(
        matches!(result, Err(FiggyError::UnknownColumn { ref id }) if id == "nope"),
        "{result:?}"
    );
    assert_eq!(chart_state(&renderer, chart), before);

    // An incomplete upload refuses the transaction, naming its column.
    let mut short = renderer
        .begin_column_upload(
            "x",
            4,
            StreamEncoding::HiLoF32,
            renderer.column_revision("x"),
        )
        .unwrap();
    put(&mut short, 0, &[1.0, 2.0]).unwrap();
    let result = renderer.commit_column_uploads(chart, vec![short], None);
    assert!(
        matches!(result, Err(FiggyError::InvalidColumnUpload { ref id, .. }) if id == "x"),
        "{result:?}"
    );
    assert_eq!(chart_state(&renderer, chart), before);

    // The same id twice is refused.
    let twice = vec![
        staged(&renderer, "x", &[1.0], renderer.column_revision("x")),
        staged(&renderer, "x", &[2.0], renderer.column_revision("x")),
    ];
    assert!(renderer.commit_column_uploads(chart, twice, None).is_err());
    assert_eq!(chart_state(&renderer, chart), before);
}

#[test]
fn without_a_state_the_kept_series_must_hold_against_the_new_columns() {
    use renderer::data_config::{
        FieldFillConfig, FieldSeries, FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
    };
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    renderer
        .upsert_column("hx", &vec![1.0f64, 2.0, 3.0])
        .unwrap();
    renderer.upsert_column("hy", &vec![1.0f64, 2.0]).unwrap();
    let mut ids = Vec::new();
    for k in 0..3 {
        let id = format!("z{k}");
        renderer
            .upsert_column(id.as_str(), &vec![k as f64, 1.0])
            .unwrap();
        ids.push(id);
    }
    let mut config = renderer::default::default_config();
    config.colorbar = Some(renderer::default::default_colorbar_options());
    let hex = SeriesConfig {
        series_id: "hex".into(),
        source_id: None,
        z_column: None,
        label: None,
        x_column: "hx".into(),
        y_column: "hy".into(),
        render_type: renderer::DataRenderType::Field(FieldSeries {
            matrix: MatrixRef {
                columns: ids,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Hexagonal,
            },
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            }),
            contour: None,
            surface: None,
        }),
    };
    let chart = renderer.register_chart(config, vec![hex]).unwrap();
    let revision = renderer.column_revision("hx");
    let series = renderer.chart_series(chart).unwrap().to_vec();

    // A hexagonal lattice needs uniform coordinates: the kept series
    // refuses the new hx, and nothing changes.
    let uploads = vec![staged(&renderer, "hx", &[1.0, 2.0, 4.0], revision)];
    let result = renderer.commit_column_uploads(chart, uploads, None);
    assert!(
        matches!(result, Err(FiggyError::InvalidSeriesConfig { ref series_id, .. }) if series_id == "hex"),
        "{result:?}"
    );
    assert_eq!(renderer.column_revision("hx"), revision);
    assert_eq!(renderer.pool().slot("hx").unwrap().max, 3.0);
    assert_eq!(renderer.chart_series(chart).unwrap(), series.as_slice());

    // A uniform one holds.
    let uploads = vec![staged(&renderer, "hx", &[2.0, 4.0, 6.0], revision)];
    renderer
        .commit_column_uploads(chart, uploads, None)
        .expect("a uniform lattice");
    assert_eq!(renderer.pool().slot("hx").unwrap().max, 6.0);
}

#[test]
fn a_column_transaction_stages_any_number_of_columns() {
    let (mut renderer, chart, config) = previous_document();
    // Past any small fixed count of staged columns.
    let count = 300;
    let ids: Vec<String> = (0..count).map(|k| format!("t{k}")).collect();
    let uploads: Vec<renderer::ColumnUpload> = ids
        .iter()
        .enumerate()
        .map(|(k, id)| staged(&renderer, id, &[k as f64, k as f64 + 0.5, 1.0], None))
        .collect();
    let series: Vec<SeriesConfig> = ids
        .iter()
        .map(|id| SeriesConfig::line(format!("s{id}"), "t0", id.as_str()))
        .collect();
    renderer
        .commit_column_uploads(chart, uploads, Some((config, series)))
        .expect("commit");
    let pool = renderer.pool();
    for (k, id) in ids.iter().enumerate() {
        assert_eq!(pool.slot(id).unwrap().max, (k as f64 + 0.5).max(1.0));
    }
    assert_eq!(renderer.chart_series(chart).unwrap().len(), count);
    let last = format!("st{}", count - 1);
    assert!(!renderer.series_draw_info(chart, &last).unwrap().truncated);
}

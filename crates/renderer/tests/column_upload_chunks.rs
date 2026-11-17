//! A resident column uploaded in chunks (#311): `begin_column_upload` holds
//! one mapped staging buffer, `ColumnUpload::write_range` writes the chunks
//! into it in index order, and `commit_column_upload` installs the column as
//! one upsert. Overlapping, gapped, oversized and mistyped chunks are
//! refused without changing the upload; an incomplete, stale or foreign
//! upload is refused at commit without changing the renderer; a budget that
//! cannot hold the staging buffer refuses the upload with a pool error.
//! `inspect_streaming_candidate` checks a candidate with the rules
//! `inspect_streaming_capability` applies to a registered chart.
#![cfg(feature = "helpers")]

use renderer::{
    AllocError, Chart, Color, FiggyError, OverlayLayer, Renderer, SeriesConfig, StreamColumnSource,
    StreamEncoding,
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

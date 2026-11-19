//! A column replaced under the same id may change its length: the pool
//! uploads the new values whole, a registered chart draws the new length, a
//! mismatch between role columns draws their common extent and reports the
//! truncation, and a refused replacement leaves the old column in place.
#![cfg(feature = "helpers")]

use renderer::{Chart, Color, ExportOptions, ExportSource, FiggyError, Renderer, SeriesConfig};

fn wave(n: usize, scale: f64) -> (Vec<f64>, Vec<f64>) {
    let x: Vec<f64> = (0..n).map(|i| i as f64 / n as f64 * 10.0).collect();
    let y: Vec<f64> = x.iter().map(|x| (x * scale).sin()).collect();
    (x, y)
}

fn upload(renderer: &mut Renderer, n: usize) {
    let (x, y) = wave(n, 1.0);
    renderer.upsert_column("x", &x).expect("x");
    renderer.upsert_column("y", &y).expect("y");
}

fn drew_something(renderer: &mut Renderer, chart: &Chart, series: &[SeriesConfig]) -> bool {
    let image = renderer
        .export_rgba_blocking(
            ExportSource::chart(chart, series),
            ExportOptions::panel(1.0).with_clear(Color::WHITE),
        )
        .expect("export");
    image
        .rgba
        .chunks_exact(4)
        .any(|px| px[2] > 120 && px[0] < 120)
}

#[test]
fn a_replaced_column_may_grow_shrink_and_mismatch() {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    upload(&mut renderer, 100);
    let series = vec![
        SeriesConfig::line("wave", "x", "y")
            .color(Color::from_rgb8(30, 60, 200))
            .width(2.0),
    ];
    let mut chart = Chart::sized(480, 320);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-1.2, 1.2);
    let id = renderer
        .register_chart(chart.config().clone(), series.clone())
        .expect("register");
    let at_100 = renderer.series_draw_info(id, "wave").unwrap();
    assert!(!at_100.truncated);
    assert!(drew_something(&mut renderer, &chart, &series));

    // Longer: every new value is drawn.
    upload(&mut renderer, 250);
    let at_250 = renderer.series_draw_info(id, "wave").unwrap();
    assert!(!at_250.truncated);
    assert_eq!(at_250.drawn_count - at_100.drawn_count, 150);
    assert!(drew_something(&mut renderer, &chart, &series));

    // y shorter than x: the common extent is drawn and the truncation reported.
    let (_, short_y) = wave(40, 1.0);
    renderer.upsert_column("y", &short_y).expect("short y");
    let mismatched = renderer.series_draw_info(id, "wave").unwrap();
    assert!(mismatched.truncated);
    assert_eq!(at_250.drawn_count - mismatched.drawn_count, 210);
    assert!(drew_something(&mut renderer, &chart, &series));

    // x shortened to match: no truncation, the same count.
    let (short_x, _) = wave(40, 1.0);
    renderer.upsert_column("x", &short_x).expect("short x");
    let at_40 = renderer.series_draw_info(id, "wave").unwrap();
    assert!(!at_40.truncated);
    assert_eq!(at_40.drawn_count, mismatched.drawn_count);

    // An empty replacement is refused and the 40 values stay.
    let empty: Vec<f64> = Vec::new();
    let refused = renderer.upsert_column("y", &empty);
    assert!(matches!(refused, Err(FiggyError::Pool(_))), "{refused:?}");
    assert_eq!(renderer.series_draw_info(id, "wave").unwrap(), at_40);
    assert!(drew_something(&mut renderer, &chart, &series));
}

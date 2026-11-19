//! The pick ID pass prototype (#381) against the exact picker on the
//! playground's gallery scenes.
//!
//! Every Cartesian scene of the playground catalogue
//! (`support/playground_scenes.rs`) is registered and picked on a grid of
//! cursor positions over its data area, once through the exact picker
//! (`Renderer::pick_chart`) and once through each geometry of the pick ID
//! pass (`Renderer::pick_chart_id_prototype`). A position agrees when both
//! answer nothing, or both answer the same series and point index.
//!
//! Measurement, not a gate (both tests are `#[ignore]`d; run them in
//! release with `--ignored`): the test prints a table and writes it to
//! `target/ci-results/pick_id_agreement.csv`, and lists each position where
//! the exact geometry, which calls the exact picker's own candidate
//! functions, disagrees with the exact picker. It does not fail on them.
//!
//! `FIGGY_PICK_ID_STEP` sets the grid step in pixels (default 6) and
//! `FIGGY_PICK_ID_REACH` the hover reach (default 4).
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/harness.rs"]
mod harness;
#[path = "support/gpu.rs"]
mod test_gpu;
use harness::headless_renderer as renderer;
#[path = "support/playground_scenes.rs"]
mod scenes;

use std::time::{Duration, Instant};

use renderer::DataRenderType;
use renderer::layout::Rect;
use renderer::monotonic::MonotonicSlices;
use renderer::renderer::{MonotonicChartId, MonotonicRange};
use renderer::{ChartId, GpuPickRequest, IdPickGeometry, IdPickHit, PickedPoint, Renderer};

use scenes::SceneKind;

const BIG_DATA_POINTS: usize = 20_000;

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[derive(Default)]
struct Tally {
    positions: usize,
    hits: usize,
    agree: usize,
    /// Both hit, different series.
    series: usize,
    /// Same series, different index.
    index: usize,
    /// One side hit, the other did not.
    presence: usize,
    time: Duration,
}

impl Tally {
    fn rate(&self) -> f64 {
        if self.positions == 0 {
            1.0
        } else {
            self.agree as f64 / self.positions as f64
        }
    }
}

fn skipped(kind: SceneKind) -> Option<&'static str> {
    match kind {
        SceneKind::Radial | SceneKind::Sunburst => Some("radial: DataLess, CPU/sector pick"),
        _ => None,
    }
}

/// A registered scene: an ordinary chart, or a monotonic chart reached
/// through its handle (#271).
#[derive(Clone, Copy)]
enum Target {
    Chart(ChartId),
    Monotonic(MonotonicChartId),
}

fn register(r: &mut Renderer, scene: &scenes::CartesianScene) -> Target {
    if scene.monotonic.is_empty() {
        return Target::Chart(
            r.register_chart(scene.config.clone(), scene.series.clone())
                .expect("register"),
        );
    }
    let id = r
        .register_monotonic_chart(scene.config.clone(), scene.series.clone())
        .expect("register monotonic");
    let mut chart = r.monotonic_chart(id).expect("monotonic chart");
    for group in &scene.monotonic {
        let view: Vec<&[f64]> = group.channels.iter().map(Vec::as_slice).collect();
        let source = MonotonicSlices::new(group.start, group.interval, &view);
        let range = scene
            .config
            .bottom_x
            .max
            .is_finite()
            .then(|| MonotonicRange::X {
                min: scene.config.bottom_x.min,
                max: scene.config.bottom_x.max,
            })
            .unwrap_or(MonotonicRange::Autoscale);
        chart
            .set_monotonic(&group.group, &source, range)
            .expect("set_monotonic");
    }
    Target::Monotonic(id)
}

fn exact_pick(
    r: &mut Renderer,
    target: Target,
    request: GpuPickRequest,
) -> Result<Option<PickedPoint>, renderer::GpuPickError> {
    match target {
        Target::Chart(id) => r.pick_chart(id, request)?.resolve_blocking(),
        Target::Monotonic(id) => r
            .monotonic_chart(id)
            .expect("monotonic chart")
            .pick(request)?
            .resolve_blocking(),
    }
}

fn id_pick(
    r: &mut Renderer,
    target: Target,
    request: GpuPickRequest,
    geometry: IdPickGeometry,
) -> Result<Option<IdPickHit>, renderer::GpuPickError> {
    match target {
        Target::Chart(id) => r
            .pick_chart_id_prototype(id, request, geometry)?
            .resolve_blocking(),
        Target::Monotonic(id) => r
            .monotonic_chart(id)
            .expect("monotonic chart")
            .pick_id_prototype(request, geometry)?
            .resolve_blocking(),
    }
}

#[test]
#[ignore = "measurement (#381): run in release with --ignored"]
fn id_pass_agrees_with_exact_picker_on_gallery_scenes() {
    let step = env_f32("FIGGY_PICK_ID_STEP", 6.0);
    let reach = env_f32("FIGGY_PICK_ID_REACH", 4.0);
    let mut rows = Vec::new();
    let mut exact_failures = Vec::new();
    println!(
        "| scene | positions | exact hits | ID exact agree | ID simple agree | simple: series / index / presence | exact ms/pick | ID ms/pick |"
    );
    println!("|---|---:|---:|---:|---:|---|---:|---:|");
    for kind in SceneKind::ALL {
        if let Some(reason) = skipped(kind) {
            println!("| {} | skipped: {reason} | | | | | | |", kind.label());
            continue;
        }
        let mut r: Renderer = renderer();
        let scene = scenes::build(&mut r, kind, BIG_DATA_POINTS)
            .unwrap_or_else(|error| panic!("{}: {error}", kind.label()));
        let target = register(&mut r, &scene);
        r.enable_gpu_picking().expect("picking");
        let document = scene.config.document_area();
        let Ok(area) = scene.config.data_area() else {
            continue;
        };
        let area: Rect = area.0;
        let request = |x: f32, y: f32| GpuPickRequest {
            canvas_position_px: [x, y],
            display_panel_px: document,
            display_scale: 1.0,
            max_distance_px: reach,
        };
        let mut exact_t = Tally::default();
        let mut id_exact = Tally::default();
        let mut id_simple = Tally::default();
        let mut y = area.y as f32 + 0.5;
        while y < (area.y + area.height) as f32 {
            let mut x = area.x as f32 + 0.5;
            while x < (area.x + area.width) as f32 {
                let started = Instant::now();
                let exact = exact_pick(&mut r, target, request(x, y));
                exact_t.time += started.elapsed();
                let Ok(exact) = exact else {
                    // The exact picker refuses this chart (a family it does
                    // not pick): nothing to compare.
                    x += step;
                    continue;
                };
                exact_t.positions += 1;
                exact_t.hits += usize::from(exact.is_some());
                for (geometry, tally) in [
                    (IdPickGeometry::Exact, &mut id_exact),
                    (IdPickGeometry::Simple, &mut id_simple),
                ] {
                    let started = Instant::now();
                    let hit = id_pick(&mut r, target, request(x, y), geometry)
                        .unwrap_or_else(|error| panic!("{}: {error}", kind.label()));
                    tally.time += started.elapsed();
                    tally.positions += 1;
                    tally.hits += usize::from(hit.is_some());
                    match (&exact, &hit) {
                        (None, None) => tally.agree += 1,
                        (Some(e), Some(h))
                            if e.series_id == h.series_id && e.point_index == h.point_index =>
                        {
                            tally.agree += 1
                        }
                        (Some(e), Some(h)) if e.series_id != h.series_id => {
                            tally.series += 1;
                            if geometry == IdPickGeometry::Exact {
                                exact_failures.push(format!(
                                    "{} ({x}, {y}): exact {}#{} at {} px, ID {}#{} covering {}",
                                    kind.label(),
                                    e.series_id,
                                    e.point_index,
                                    e.distance_px,
                                    h.series_id,
                                    h.point_index,
                                    h.covering
                                ));
                            }
                        }
                        (Some(e), Some(h)) => {
                            tally.index += 1;
                            if geometry == IdPickGeometry::Exact {
                                exact_failures.push(format!(
                                    "{} ({x}, {y}): exact {}#{} at {} px, ID #{} covering {}",
                                    kind.label(),
                                    e.series_id,
                                    e.point_index,
                                    e.distance_px,
                                    h.point_index,
                                    h.covering
                                ));
                            }
                        }
                        _ => {
                            tally.presence += 1;
                            if geometry == IdPickGeometry::Exact {
                                exact_failures.push(format!(
                                    "{} ({x}, {y}): exact {:?}, ID {:?}",
                                    kind.label(),
                                    exact.as_ref().map(|e| (
                                        &e.series_id,
                                        e.point_index,
                                        e.distance_px
                                    )),
                                    hit.as_ref()
                                        .map(|h| (&h.series_id, h.point_index, h.covering)),
                                ));
                            }
                        }
                    }
                }
                x += step;
            }
            y += step;
        }
        let per_pick = |t: &Tally| {
            if t.positions == 0 {
                0.0
            } else {
                t.time.as_secs_f64() * 1e3 / t.positions as f64
            }
        };
        let row = format!(
            "| {} | {} | {} | {:.2}% | {:.2}% | {} / {} / {} | {:.3} | {:.3} |",
            kind.label(),
            exact_t.positions,
            exact_t.hits,
            id_exact.rate() * 100.0,
            id_simple.rate() * 100.0,
            id_simple.series,
            id_simple.index,
            id_simple.presence,
            per_pick(&exact_t),
            per_pick(&id_exact),
        );
        println!("{row}");
        rows.push(format!(
            "{},{},{},{},{},{},{},{},{:.4},{:.4},{:.4}",
            kind.label().replace(',', ";"),
            exact_t.positions,
            exact_t.hits,
            id_exact.agree,
            id_simple.agree,
            id_simple.series,
            id_simple.index,
            id_simple.presence,
            per_pick(&exact_t),
            per_pick(&id_exact),
            per_pick(&id_simple),
        ));
        match target {
            Target::Chart(id) => r.remove_chart(id).ok(),
            Target::Monotonic(id) => r.monotonic_chart(id).and_then(|chart| chart.remove()).ok(),
        };
        println!(
            "  pick ID pipelines created: {}",
            r.pick_id_pipelines_created()
        );
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ci-results");
    std::fs::create_dir_all(&out).ok();
    std::fs::write(
        out.join("pick_id_agreement.csv"),
        format!(
            "scene,positions,exact_hits,id_exact_agree,id_simple_agree,simple_series,simple_index,simple_presence,exact_ms,id_exact_ms,id_simple_ms\n{}\n",
            rows.join("\n")
        ),
    )
    .ok();
    for failure in exact_failures.iter().take(40) {
        println!("exact-geometry disagreement: {failure}");
    }
    println!("exact-geometry disagreements: {}", exact_failures.len());
}

/// Single-pick latency (#381 item 4) on the large-data scene at
/// `FIGGY_PICK_ID_POINTS` points per series (comma-separated, default
/// 20000,1000000): the first pick on a fresh renderer (registry, pipeline
/// compiles and the pick), then the median of 21 picks at the plot's
/// centre, each submitted, mapped and decoded, for the exact picker and
/// both ID geometries.
#[test]
#[ignore = "measurement (#381): run in release with --ignored"]
fn id_pass_single_pick_latency() {
    let sizes: Vec<usize> = std::env::var("FIGGY_PICK_ID_POINTS")
        .unwrap_or_else(|_| "20000,1000000".into())
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    println!("| points | picker | first pick (cold) ms | median pick ms | pipelines |");
    println!("|---:|---|---:|---:|---:|");
    for points in sizes {
        for picker in ["exact", "ID exact", "ID simple"] {
            let mut r: Renderer = renderer();
            let scene = scenes::build(&mut r, SceneKind::BigData, points).expect("big data");
            let target = register(&mut r, &scene);
            let document = scene.config.document_area();
            let area: Rect = scene.config.data_area().expect("data area").0;
            let request = GpuPickRequest {
                canvas_position_px: [
                    area.x as f32 + area.width as f32 * 0.5,
                    area.y as f32 + area.height as f32 * 0.5,
                ],
                display_panel_px: document,
                display_scale: 1.0,
                max_distance_px: 4.0,
            };
            let pick = |r: &mut Renderer| match picker {
                "exact" => exact_pick(r, target, request).map(|_| ()),
                "ID exact" => id_pick(r, target, request, IdPickGeometry::Exact).map(|_| ()),
                _ => id_pick(r, target, request, IdPickGeometry::Simple).map(|_| ()),
            };
            let started = Instant::now();
            r.enable_gpu_picking().expect("picking");
            pick(&mut r).expect("first pick");
            let cold = started.elapsed().as_secs_f64() * 1e3;
            let mut times: Vec<f64> = (0..21)
                .map(|_| {
                    let started = Instant::now();
                    pick(&mut r).expect("pick");
                    started.elapsed().as_secs_f64() * 1e3
                })
                .collect();
            times.sort_by(f64::total_cmp);
            println!(
                "| {points} | {picker} | {cold:.1} | {:.3} | {} |",
                times[times.len() / 2],
                if picker == "exact" {
                    "-".to_owned()
                } else {
                    r.pick_id_pipelines_created().to_string()
                },
            );
        }
    }
}

fn nearest_pick(
    r: &mut Renderer,
    target: Target,
    request: GpuPickRequest,
) -> Result<renderer::NearestPick, renderer::GpuPickError> {
    match target {
        Target::Chart(id) => r
            .pick_chart_nearest_layers_prototype(id, request)?
            .resolve_blocking(),
        Target::Monotonic(id) => r
            .monotonic_chart(id)
            .expect("monotonic chart")
            .pick_nearest_layers_prototype(request)?
            .resolve_blocking(),
    }
}

/// The maintainer's rule (#381) against today's picker on the gallery
/// scenes. Today a click hits when the exact picker finds the cursor inside
/// a drawn shape (`max_distance_px` 0). Under the rule, the cursor hits when
/// the drawn pixel under it has data ink, and the hit is the nearest datum
/// over every series of every drawn layer at the cursor (overlays
/// included: each series' nearest, with no reach, then the nearest of
/// those), whichever series it belongs to. Data ink here is the export
/// with the scene's series against the export without them (a channel
/// difference of at least 6), which stands in for the panel's resolved
/// data pixels. Only the point picker's families count as ink (the other
/// families keep today's typed data picks), and 3D charts are skipped: this
/// kernel places data with the 2D projection only. Reported, not failed:
/// - hit/no-hit agreement with today;
/// - where both hit, how often the nearest datum's series differs from
///   today's series, and how often the same series answers another index.
#[test]
#[ignore = "measurement (#381): run in release with --ignored"]
fn nearest_per_series_with_ink_hit_vs_exact_picker_on_gallery_scenes() {
    let step = env_f32("FIGGY_PICK_ID_STEP", 6.0);
    let mut rows = Vec::new();
    let mut total = [0usize; 8];
    println!(
        "| scene | positions | today hits | ink hits | hit/no-hit agree | ink only | today only | both: other series | both: same series, other index | series answered (min) | today ms | nearest ms |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for kind in SceneKind::ALL {
        if skipped(kind).is_some() {
            continue;
        }
        let mut r: Renderer = renderer();
        let scene = scenes::build(&mut r, kind, BIG_DATA_POINTS)
            .unwrap_or_else(|error| panic!("{}: {error}", kind.label()));
        let chart = renderer::Chart::new(scene.config.clone());
        let export = |r: &mut Renderer,
                      series: &[renderer::SeriesConfig],
                      monotonic: &[scenes::MonotonicData]| {
            scenes::export_scene(r, &chart, series, monotonic, |r, source| {
                r.export_rgba_blocking(source, renderer::ExportOptions::document(1.0))
            })
            .unwrap_or_else(|error| panic!("{}: {error}", kind.label()))
        };
        if scene.config.view_3d().is_some() {
            println!(
                "| {} | skipped: 3D (this kernel places data in 2D only) | | | | | | | | | | |",
                kind.label()
            );
            continue;
        }
        // Data ink of the point picker's families only: the export without
        // them keeps every other series (fields, bars, boxes, levels...),
        // which today's typed data picks answer.
        let point_family = |s: &renderer::SeriesConfig| {
            matches!(
                s.render_type,
                DataRenderType::Xy(_)
                    | DataRenderType::Radar(_)
                    | DataRenderType::Ternary(_)
                    | DataRenderType::Streamlines(_)
                    | DataRenderType::MonotonicLine(_)
                    | DataRenderType::Vector(_)
            )
        };
        if !scene.series.iter().any(point_family) {
            continue;
        }
        let others: Vec<renderer::SeriesConfig> = scene
            .series
            .iter()
            .filter(|s| !point_family(s))
            .cloned()
            .collect();
        let with = export(&mut r, &scene.series, &scene.monotonic);
        let without = export(&mut r, &others, &[]);
        let target = register(&mut r, &scene);
        r.enable_gpu_picking().expect("picking");
        let document = scene.config.document_area();
        assert_eq!(
            (with.width, with.height),
            (document.width, document.height),
            "{}: the export covers the document",
            kind.label()
        );
        let Ok(area) = scene.config.data_area() else {
            continue;
        };
        let area: Rect = area.0;
        let ink = |x: f32, y: f32| {
            let px = (x - document.x as f32).floor() as u32;
            let py = (y - document.y as f32).floor() as u32;
            if px >= with.width || py >= with.height {
                return false;
            }
            let i = ((py * with.width + px) * 4) as usize;
            (0..4).any(|c| with.rgba[i + c].abs_diff(without.rgba[i + c]) >= 6)
        };
        let request = |x: f32, y: f32, reach: f32| GpuPickRequest {
            canvas_position_px: [x, y],
            display_panel_px: document,
            display_scale: 1.0,
            max_distance_px: reach,
        };
        // positions, today hits, ink hits, agree, ink only, today only,
        // other series, other index
        let mut t = [0usize; 8];
        let mut min_k = usize::MAX;
        let (mut today_time, mut nearest_time) = (Duration::ZERO, Duration::ZERO);
        let mut y = area.y as f32 + 0.5;
        while y < (area.y + area.height) as f32 {
            let mut x = area.x as f32 + 0.5;
            while x < (area.x + area.width) as f32 {
                let started = Instant::now();
                let today = exact_pick(&mut r, target, request(x, y, 0.0));
                today_time += started.elapsed();
                let Ok(today) = today else {
                    x += step;
                    continue;
                };
                let started = Instant::now();
                let picked = nearest_pick(&mut r, target, request(x, y, 0.0))
                    .unwrap_or_else(|error| panic!("{}: {error}", kind.label()));
                nearest_time += started.elapsed();
                min_k = min_k.min(picked.per_series.len());
                let nearest = picked.nearest;
                let hit = ink(x, y);
                t[0] += 1;
                t[1] += usize::from(today.is_some());
                t[2] += usize::from(hit);
                match (&today, hit) {
                    (None, false) => t[3] += 1,
                    (None, true) => t[4] += 1,
                    (Some(_), false) => t[5] += 1,
                    (Some(e), true) => {
                        t[3] += 1;
                        match nearest.as_ref() {
                            Some(n) if n.series_id != e.series_id => t[6] += 1,
                            Some(n) if n.point_index != e.point_index => t[7] += 1,
                            Some(_) => {}
                            None => t[6] += 1,
                        }
                    }
                }
                x += step;
            }
            y += step;
        }
        for (sum, value) in total.iter_mut().zip(t) {
            *sum += value;
        }
        let per = |time: Duration| time.as_secs_f64() * 1e3 / t[0].max(1) as f64;
        if t[1] + t[2] > 0 {
            println!(
                "| {} | {} | {} | {} | {:.2}% | {} | {} | {} | {} | {} | {:.3} | {:.3} |",
                kind.label(),
                t[0],
                t[1],
                t[2],
                t[3] as f64 * 100.0 / t[0].max(1) as f64,
                t[4],
                t[5],
                t[6],
                t[7],
                if min_k == usize::MAX { 0 } else { min_k },
                per(today_time),
                per(nearest_time),
            );
        }
        rows.push(format!(
            "{},{},{},{},{},{},{},{},{},{},{:.4},{:.4}",
            kind.label().replace(',', ";"),
            t[0],
            t[1],
            t[2],
            t[3],
            t[4],
            t[5],
            t[6],
            t[7],
            if min_k == usize::MAX { 0 } else { min_k },
            per(today_time),
            per(nearest_time)
        ));
    }
    println!(
        "TOTAL positions {} today hits {} ink hits {} agree {} ({:.2}%) ink only {} today only {} other series {} other index {}",
        total[0],
        total[1],
        total[2],
        total[3],
        total[3] as f64 * 100.0 / total[0].max(1) as f64,
        total[4],
        total[5],
        total[6],
        total[7]
    );
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ci-results");
    std::fs::create_dir_all(&out).ok();
    std::fs::write(
        out.join("pick_nearest_per_series_agreement.csv"),
        format!(
            "scene,positions,today_hits,ink_hits,agree,ink_only,today_only,other_series,other_index,min_series,today_ms,nearest_ms\n{}\n",
            rows.join("\n")
        ),
    )
    .ok();
}

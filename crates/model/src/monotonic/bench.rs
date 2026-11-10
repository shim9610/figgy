//! Measurements of the monotonic core (#271). Ignored by default; run in
//! release mode:
//!
//! ```text
//! cargo test --release -p figgy-model --lib monotonic::bench -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Every figure is the median of 5 runs. Appends and slides pass only the new
//! samples, as a host that drops old data would.

use std::time::Instant;

use super::*;

const RUNS: usize = 5;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// A random walk, generated quickly.
fn walk_f64(n: usize, seed: u64) -> Vec<f64> {
    let mut s = seed | 1;
    let mut y = 0.0f64;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            y += (s >> 33) as f64 / (1u64 << 31) as f64 - 0.5;
            y
        })
        .collect()
}

fn walk_i16(n: usize, seed: u64) -> Vec<i16> {
    let mut s = seed | 1;
    let mut y = 0i32;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            y = (y + ((s >> 60) as i32 - 8)).clamp(-32768, 32767);
            y as i16
        })
        .collect()
}

fn ms(t: f64) -> String {
    format!("{:.3}", t * 1e3)
}

fn time(f: impl FnOnce()) -> f64 {
    let t = Instant::now();
    f();
    t.elapsed().as_secs_f64()
}

fn full_seconds<T: MonotonicElement>(data: &[T], width: u32) -> f64 {
    let channels: &[&[T]] = &[data];
    let source = MonotonicSlices::new(channels);
    let group = MonotonicGroup::new(0.0, 1.0, &source);
    let mapping = MonotonicMapping::linear(0.0, data.len() as f64, width);
    let mut envelope = MonotonicEnvelope::empty();
    median(
        (0..RUNS)
            .map(|_| {
                time(|| {
                    envelope.set_monotonic(&group, &mapping).unwrap();
                })
            })
            .collect(),
    )
}

#[test]
#[ignore = "measurement; run with --release"]
fn bench_full_reduction() {
    println!("| samples | width | type | median ms | samples/s |");
    println!("|---:|---:|---|---:|---:|");
    for n in [1_000_000usize, 10_000_000, 100_000_000] {
        let data = walk_f64(n, 1);
        for width in [1000, 2000] {
            let t = full_seconds(&data, width);
            println!("| {n} | {width} | f64 | {} | {:.3e} |", ms(t), n as f64 / t);
        }
        drop(data);
        let data = walk_i16(n, 2);
        for width in [1000, 2000] {
            let t = full_seconds(&data, width);
            println!("| {n} | {width} | i16 | {} | {:.3e} |", ms(t), n as f64 / t);
        }
    }
}

/// The maintainer's scenario on one group: appends of `step` new samples
/// from 0 to `window`, then sliding appends of `step` until `total`.
/// `source(range)` lends logical samples `range` as a new source. Returns
/// the median time of each call and its dropped columns.
fn scenario<'a, S: MonotonicSource + ?Sized + 'a>(
    window: usize,
    step: usize,
    total: usize,
    empty: &S,
    source: &dyn Fn(std::ops::Range<usize>) -> &'a S,
) -> Vec<(String, f64, usize)> {
    let calls = total / step;
    let mut times = vec![Vec::new(); calls];
    let mut dropped = vec![0; calls];
    for _ in 0..RUNS {
        let mapping = MonotonicMapping::linear(0.0, window as f64, 1000);
        let mut envelope = MonotonicEnvelope::empty();
        envelope
            .set_monotonic(&MonotonicGroup::new(0.0, 1.0, empty), &mapping)
            .unwrap();
        for (k, slot) in times.iter_mut().enumerate() {
            let new = source(k * step..(k + 1) * step);
            let sliding = (k + 1) * step > window;
            let mut update = None;
            slot.push(time(|| {
                update = Some(
                    envelope
                        .append_monotonic(new, MonotonicAppend { sliding })
                        .unwrap(),
                );
            }));
            dropped[k] = update.map_or(0, |u| u.dropped);
        }
    }
    times
        .into_iter()
        .enumerate()
        .map(|(k, t)| {
            let name = if (k + 1) * step > window {
                format!("slide {}", k + 1 - window / step)
            } else {
                format!("append {}", k + 1)
            };
            (name, median(t), dropped[k])
        })
        .collect()
}

fn print_scenario(name: &str, rows: Vec<(String, f64, usize)>) {
    println!("\n{name}");
    println!("| call | median ms | dropped columns |");
    println!("|---|---:|---:|");
    for (call, t, dropped) in rows {
        println!("| {call} | {} | {dropped} |", ms(t));
    }
}

#[test]
#[ignore = "measurement; run with --release"]
fn bench_maintainer_scenario() {
    let window = 1_000_000;
    let step = 100_000;
    let total = 2_000_000;
    let channels = 8;
    let data: Vec<Vec<f64>> = (0..channels)
        .map(|c| walk_f64(total, 10 + c as u64))
        .collect();
    // One source per step, built outside the timed calls: each lends only
    // that step's new samples.
    let ones: Vec<[&[f64]; 1]> = (0..=total / step)
        .map(|k| [&data[0][k * step..((k + 1) * step).min(total)]])
        .collect();
    let one_sources: Vec<MonotonicSlices<'_, f64>> =
        ones.iter().map(|v| MonotonicSlices::new(v)).collect();
    print_scenario(
        "1 channel, f64",
        scenario(window, step, total, &one_sources[total / step], &|r| {
            &one_sources[r.start / step]
        }),
    );
    let frames: Vec<f64> = (0..total)
        .flat_map(|i| data.iter().map(move |d| d[i]))
        .collect();
    let columns: Vec<usize> = (0..channels).collect();
    let interleaved: Vec<MonotonicInterleaved<'_, f64>> = (0..=total / step)
        .map(|k| {
            let from = (k * step).min(total) * channels;
            let to = ((k + 1) * step).min(total) * channels;
            MonotonicInterleaved::new(&frames[from..to], channels, &columns).unwrap()
        })
        .collect();
    print_scenario(
        "8 channels, f64, interleaved",
        scenario(window, step, total, &interleaved[total / step], &|r| {
            &interleaved[r.start / step]
        }),
    );
    let views: Vec<Vec<&[f64]>> = (0..=total / step)
        .map(|k| {
            data.iter()
                .map(|d| &d[(k * step).min(total)..((k + 1) * step).min(total)])
                .collect()
        })
        .collect();
    let sliced: Vec<MonotonicSlices<'_, f64>> =
        views.iter().map(|v| MonotonicSlices::new(v)).collect();
    print_scenario(
        "8 channels, f64, slices",
        scenario(window, step, total, &sliced[total / step], &|r| {
            &sliced[r.start / step]
        }),
    );
}

#[test]
#[ignore = "measurement; run with --release"]
fn bench_append_at_ten_million() {
    let base = 10_000_000;
    let data = walk_f64(base + 100_000, 5);
    println!("| call at 10M samples | median ms | dropped columns | offset |");
    println!("|---|---:|---:|---:|");
    for (k, sliding, x_max) in [
        (1_000, false, 2.0e7),
        (100_000, false, 2.0e7),
        (1_000, true, 1.0e7),
        (100_000, true, 1.0e7),
    ] {
        let mapping = MonotonicMapping::linear(0.0, x_max, 1000);
        let before: &[&[f64]] = &[&data[..base]];
        let new: &[&[f64]] = &[&data[base..base + k]];
        let before = MonotonicSlices::new(before);
        let new = MonotonicSlices::new(new);
        let mut last = None;
        let times = (0..RUNS)
            .map(|_| {
                let mut envelope = MonotonicEnvelope::empty();
                envelope
                    .set_monotonic(&MonotonicGroup::new(0.0, 1.0, &before), &mapping)
                    .unwrap();
                time(|| {
                    last = Some(
                        envelope
                            .append_monotonic(&new, MonotonicAppend { sliding })
                            .unwrap(),
                    );
                })
            })
            .collect();
        let name = if sliding { "sliding append" } else { "append" };
        let (dropped, offset) = last.map_or((0, 0.0), |u| (u.dropped, u.pixel_offset));
        println!(
            "| {name} {k} | {} | {dropped} | {offset} |",
            ms(median(times))
        );
    }
}

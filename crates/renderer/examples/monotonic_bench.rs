//! Monotonic-x line cost probe (#271): times a monotonic chart through the
//! production calls, `set_monotonic` / `append_monotonic`,
//! `prepare_monotonic` and `paint_prepared` into an offscreen target, and
//! waits for the GPU after every frame. Runs on the native adapter wgpu
//! picks (Metal on macOS) and prints it first.
//!
//! ```text
//! cargo run --locked --release -p figgy-renderer --example monotonic_bench -- <mode> [options]
//!
//! once  --samples N [--reps R]                full call of N samples, then one frame, R times
//! slide --window N --step N --frames F        full call of N samples, then F sliding appends
//! check --samples N --type i16                1x export: the line covers each column's data range
//!
//! --type f64|i16   f64: a random walk (default); i16: ADC counts on a y axis printed in volts
//! --scale S        display scale of the 1280x720 chart (default 1)
//! --no-grid        grid lines off
//! --sine A --noise B --spike-every K
//!                  i16 data: a slow sine of A counts plus uniform noise of +/-B counts,
//!                  and every K samples a one-sample spike to +32767 / -32768
//! --y-pad P        i16 y range widened by P counts on each side, so that full-scale
//!                  spikes stay clear of the plot frame in `check`
//! ```
//!
//! The data is generated before the timed calls; an append lends only its
//! new samples, as a host that drops old data would. The frame times are
//! serial (prepare, record, submit, wait), not a display rate: a window adds
//! MSAA, presentation and vsync.

use std::time::Instant;

use renderer::layout::{ChartArea, Rect};
use renderer::monotonic::MonotonicElement;
use renderer::renderer::MonotonicRange;
use renderer::{
    Chart, ExportOptions, ExportSource, MonotonicAppend, MonotonicChartDrawItem, MonotonicChartId,
    MonotonicSlices, Renderer, RendererDevice, SeriesConfig,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const SIZE: (u32, u32) = (1280, 720);

struct Options {
    samples: usize,
    reps: usize,
    window: usize,
    step: usize,
    frames: usize,
    i16: bool,
    scale: f32,
    grid: bool,
    sine: f64,
    noise: i64,
    spike_every: usize,
    y_pad: f64,
}

fn parse(args: &[String]) -> Options {
    let mut o = Options {
        samples: 100_000_000,
        reps: 6,
        window: 50_000_000,
        step: 250_000,
        frames: 400,
        i16: false,
        scale: 1.0,
        grid: true,
        sine: 20_000.0,
        noise: 512,
        spike_every: 0,
        y_pad: 0.0,
    };
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| -> &str {
            args.get(i + 1)
                .map(String::as_str)
                .unwrap_or_else(|| panic!("{} needs a value", args[i]))
        };
        let number = |i: usize| -> f64 { value(i).parse().expect("a number") };
        match args[i].as_str() {
            "--no-grid" => {
                o.grid = false;
                i += 1;
                continue;
            }
            "--samples" => o.samples = number(i) as usize,
            "--reps" => o.reps = number(i) as usize,
            "--window" => o.window = number(i) as usize,
            "--step" => o.step = number(i) as usize,
            "--frames" => o.frames = number(i) as usize,
            "--type" => {
                o.i16 = match value(i) {
                    "i16" => true,
                    "f64" => false,
                    other => panic!("--type takes f64 or i16, not {other}"),
                }
            }
            "--scale" => o.scale = number(i) as f32,
            "--sine" => o.sine = number(i),
            "--noise" => o.noise = number(i) as i64,
            "--spike-every" => o.spike_every = number(i) as usize,
            "--y-pad" => o.y_pad = number(i),
            other => panic!("unknown option {other}"),
        }
        i += 2;
    }
    o
}

/// A random walk with a slow swing.
fn walk(n: usize, seed: u64) -> Vec<f64> {
    let mut s = seed | 1;
    let mut y = 0.0f64;
    (0..n)
        .map(|i| {
            s = lcg(s);
            y += (s >> 33) as f64 / (1u64 << 31) as f64 - 0.5;
            y + 50.0 * (i as f64 * 2e-6).sin()
        })
        .collect()
}

/// ADC-like counts: a slow sine plus uniform noise, and optional
/// one-sample spikes to full scale, alternating in sign.
fn counts(n: usize, seed: u64, o: &Options) -> Vec<i16> {
    let mut s = seed | 1;
    (0..n)
        .map(|i| {
            s = lcg(s);
            let noise = ((s >> 33) as i64 % (2 * o.noise + 1)) - o.noise;
            let mut c = (o.sine * (i as f64 * 2e-6).sin()) as i64 + noise;
            if o.spike_every > 0 && i % o.spike_every == o.spike_every / 2 {
                c = if (i / o.spike_every) % 2 == 0 {
                    32767
                } else {
                    -32768
                };
            }
            c.clamp(-32768, 32767) as i16
        })
        .collect()
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407)
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}

fn summary(v: &[f64]) -> String {
    if v.is_empty() {
        return "no frames".to_owned();
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let at = |p: f64| s[((s.len() - 1) as f64 * p).round() as usize];
    format!(
        "median {:.3} | p95 {:.3} | max {:.3} ms",
        at(0.5),
        at(0.95),
        s[s.len() - 1]
    )
}

struct Bench {
    r: Renderer,
    id: MonotonicChartId,
    view: renderer::ChartView,
    target: wgpu::Texture,
    surface: (u32, u32),
    volts: bool,
}

impl Bench {
    fn new(o: &Options) -> Self {
        let instance = renderer::gpu::create_instance();
        let adapter = renderer::gpu::request_adapter_blocking(&instance).expect("adapter");
        let info = adapter.get_info();
        println!("adapter: {} ({:?})", info.name, info.backend);
        let (device, queue) = renderer::gpu::request_device_blocking(&adapter).expect("device");
        let gpu = RendererDevice::new(device.clone(), queue).expect("figgy poll thread");
        let mut r = Renderer::try_new(gpu, FORMAT, 64 * 1024 * 1024).expect("renderer");
        let mut config = renderer::default::default_config();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: SIZE.0,
            height: SIZE.1,
        });
        config.grid.show_major_x = o.grid;
        config.grid.show_major_y = o.grid;
        if o.i16 {
            // The data stays in counts; the y axis prints volts (#273).
            let mut chart = Chart::new(config);
            chart.set_y_range(-32768.0 - o.y_pad, 32767.0 + o.y_pad);
            config = chart.with_y_unit(5.0 / 32768.0, 0.0).config().clone();
        }
        let series = vec![SeriesConfig::monotonic_line("m", "g", 0)];
        let id = r.register_monotonic_chart(config, series).expect("chart");
        let surface = (
            (SIZE.0 as f32 * o.scale) as u32,
            (SIZE.1 as f32 * o.scale) as u32,
        );
        let area = r.monotonic_config(id).expect("config").chart_area.0;
        r.monotonic_chart(id)
            .and_then(|mut chart| chart.resize(area, None, Some(surface), &[]))
            .expect("display");
        let (display, rect, scale) = r.monotonic_display_config(id).expect("display config");
        let view = r
            .create_chart_view_at(&Chart::new(display.into_owned()), rect, scale)
            .expect("view");
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("monotonic bench target"),
            size: wgpu::Extent3d {
                width: surface.0,
                height: surface.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        println!(
            "chart {}x{} at scale {} ({}x{} px), grid {}, {}",
            SIZE.0,
            SIZE.1,
            o.scale,
            surface.0,
            surface.1,
            if o.grid { "on" } else { "off" },
            if o.i16 {
                "i16 counts, y axis in volts"
            } else {
                "f64, autoscaled"
            }
        );
        Self {
            r,
            id,
            view,
            target,
            surface,
            volts: o.i16,
        }
    }

    /// The full call's range: a fixed x range keeps the volts y range.
    fn range(&self, len: usize) -> MonotonicRange {
        if self.volts {
            MonotonicRange::X {
                min: 0.0,
                max: (len - 1) as f64,
            }
        } else {
            MonotonicRange::Autoscale
        }
    }

    fn set<T: MonotonicElement>(&mut self, data: &[T]) -> renderer::MonotonicUpdate {
        let channels = [data];
        let range = self.range(data.len());
        self.r
            .monotonic_chart(self.id)
            .and_then(|mut chart| {
                chart.set_monotonic("g", &MonotonicSlices::new(0.0, 1.0, &channels), range)
            })
            .expect("set_monotonic")
    }

    /// `prepare_monotonic`, then the frame painted into the target,
    /// submitted and waited for: (prepare, record and submit, GPU wait) ms.
    fn frame(&mut self) -> (f64, f64, f64) {
        let t0 = Instant::now();
        let prepared = self
            .r
            .prepare_monotonic(&mut [MonotonicChartDrawItem {
                chart: self.id,
                view: &mut self.view,
            }])
            .expect("prepare");
        let prepare = ms(t0);
        let t1 = Instant::now();
        let view = self.target.create_view(&Default::default());
        let mut encoder = self.r.device().create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("monotonic bench frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.r
                .paint_prepared(&mut pass, self.surface, &prepared)
                .expect("paint");
        }
        self.r.queue().submit([encoder.finish()]);
        let record = ms(t1);
        let t2 = Instant::now();
        self.r.wait_idle().expect("wait");
        (prepare, record, ms(t2))
    }
}

fn once<T: MonotonicElement>(data: &[T], o: &Options) {
    let mut b = Bench::new(o);
    for rep in 0..o.reps {
        let t0 = Instant::now();
        let update = b.set(data);
        let set = ms(t0);
        let (prepare, record, gpu) = b.frame();
        println!(
            "rep {rep}: set_monotonic {set:.3} | prepare {prepare:.3} | record+submit {record:.3} | gpu wait {gpu:.3} | total {:.3} ms ({} samples)",
            ms(t0),
            update.len
        );
    }
}

fn slide<T: MonotonicElement>(data: &[T], o: &Options) {
    let mut b = Bench::new(o);
    let t0 = Instant::now();
    b.set(&data[..o.window]);
    let set = ms(t0);
    let (prepare, _, gpu) = b.frame();
    println!(
        "window of {}: set_monotonic {set:.3} | prepare {prepare:.3} | gpu wait {gpu:.3} ms",
        o.window
    );
    let mut times: [Vec<f64>; 5] = Default::default();
    let started = Instant::now();
    for k in 0..o.frames {
        let from = o.window + k * o.step;
        let new = [&data[from..from + o.step]];
        let t0 = Instant::now();
        b.r.monotonic_chart(b.id)
            .and_then(|mut chart| {
                chart.append_monotonic(
                    "g",
                    &MonotonicSlices::new(0.0, 1.0, &new),
                    MonotonicAppend { sliding: true },
                )
            })
            .expect("append_monotonic");
        let append = ms(t0);
        let (prepare, record, gpu) = b.frame();
        for (v, t) in times.iter_mut().zip([append, prepare, record, gpu, ms(t0)]) {
            v.push(t);
        }
    }
    let wall = started.elapsed().as_secs_f64();
    for (name, v) in [
        "append_monotonic",
        "prepare_monotonic",
        "record+submit",
        "gpu wait",
        "frame",
    ]
    .iter()
    .zip(&times)
    {
        println!("{name:<18} {}", summary(v));
    }
    println!(
        "{} frames of {} new samples in {wall:.3} s: {:.1} frames/s",
        o.frames,
        o.step,
        o.frames as f64 / wall
    );
}

/// Exports the chart at 1x and checks, per pixel column of the plot, that
/// the drawn line covers the data's min..max there within 1.5 px (the line
/// is looked for in the column and its two neighbours, away from the plot
/// frame), and that every spike is a kept point.
fn check(data: &[i16], o: &Options) {
    // The 1x export reuses the screen's envelope only at the screen's own
    // scale; at another scale it would need the source.
    assert!(
        o.scale == 1.0,
        "check exports at 1x and takes --scale 1 only, not {}",
        o.scale
    );
    let mut b = Bench::new(o);
    let update = b.set(data);
    let config = b.r.monotonic_config(b.id).expect("config").clone();
    let area = config.data_area().expect("data area").0;
    let (y_min, y_max) = (config.left_y.min, config.left_y.max);
    let image =
        b.r.export_rgba_blocking(
            ExportSource::Monotonic {
                chart: b.id,
                monotonic: &[],
            },
            ExportOptions::panel(1.0),
        )
        .expect("export");
    let (x0, x1) = update.x_range;
    let columns = area.width as usize;
    let mut low = vec![i16::MAX; columns];
    let mut high = vec![i16::MIN; columns];
    for (i, v) in data.iter().enumerate() {
        let c = ((i as f64 - x0) / (x1 - x0) * columns as f64).floor() as usize;
        let c = c.min(columns - 1);
        low[c] = low[c].min(*v);
        high[c] = high[c].max(*v);
    }
    let row_of =
        |v: i16| area.y as f64 + (y_max - f64::from(v)) / (y_max - y_min) * area.height as f64;
    let ink = |x: usize, y: usize| {
        let p = &image.rgba[(y * image.width as usize + x) * 4..][..4];
        p[3] > 60 && 255 - p[0].min(p[1]).min(p[2]) > 60
    };
    let rows = area.y as usize + 4..(area.y + area.height) as usize - 4;
    let (mut missed, mut worst) = (0, 0.0f64);
    for c in 6..columns - 6 {
        let (mut top, mut bottom) = (f64::INFINITY, f64::NEG_INFINITY);
        for x in c - 1..=c + 1 {
            for y in rows.clone() {
                if ink(area.x as usize + x, y) {
                    top = top.min(y as f64);
                    bottom = bottom.max(y as f64 + 1.0);
                }
            }
        }
        let miss = (top - row_of(high[c]))
            .max(row_of(low[c]) - bottom)
            .max(0.0);
        if miss > 1.5 {
            missed += 1;
        }
        worst = worst.max(miss);
    }
    println!(
        "columns whose data range the line misses by more than 1.5 px: {missed} of {}; worst {worst:.2} px",
        columns - 12
    );
    if o.spike_every > 0 {
        let spikes: Vec<usize> = (o.spike_every / 2..data.len())
            .step_by(o.spike_every)
            .collect();
        let kept = spikes
            .iter()
            .filter(|&&i| {
                b.r.monotonic_kept_point(b.id, "m", i).expect("kept point")
                    == Some((i as f64, f64::from(data[i])))
            })
            .count();
        println!("spikes kept as envelope points: {kept} of {}", spikes.len());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("once");
    let o = parse(args.get(1..).unwrap_or(&[]));
    let n = match mode {
        "slide" => o.window + o.step * o.frames,
        _ => o.samples,
    };
    let t = Instant::now();
    if o.i16 {
        let data = counts(n, 3, &o);
        println!("{n} i16 samples generated in {:.0} ms", ms(t));
        match mode {
            "once" => once(&data, &o),
            "slide" => slide(&data, &o),
            "check" => check(&data, &o),
            other => panic!("unknown mode {other}"),
        }
    } else {
        let data = walk(n, 1);
        println!("{n} f64 samples generated in {:.0} ms", ms(t));
        match mode {
            "once" => once(&data, &o),
            "slide" => slide(&data, &o),
            "check" => panic!("check takes --type i16"),
            other => panic!("unknown mode {other}"),
        }
    }
}

//! Which ID target layouts a multisampled panel pass can carry (#381
//! prototype, measurement).
//!
//! An always-on ID target is one more colour attachment of every panel
//! pass. The panel pass is multisampled (`multisample_state(sample_count)`,
//! 4x where the surface format resolves), and every attachment of a pass
//! shares its sample count. This test records, for the integer formats an
//! ID could use, what WebGPU guarantees (`guaranteed_format_features` with
//! no features, the browser's baseline) and what this adapter offers, tries
//! to create each as a 4x attachment on a default device, and times the
//! store traffic one more attachment of each layout adds to a cleared and
//! stored 4x pass at panel sizes (a lower bound: no draw writes it).
//!
//! It prints a table and writes `target/ci-results/pick_id_target_formats.txt`;
//! nothing here gates.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/gpu.rs"]
mod test_gpu;

use std::fmt::Write as _;
use std::time::Instant;

const FORMATS: [wgpu::TextureFormat; 7] = [
    wgpu::TextureFormat::R32Uint,
    wgpu::TextureFormat::Rg32Uint,
    wgpu::TextureFormat::Rgba32Uint,
    wgpu::TextureFormat::R16Uint,
    wgpu::TextureFormat::Rg16Uint,
    wgpu::TextureFormat::Rgba16Uint,
    wgpu::TextureFormat::Rgba8Uint,
];

fn flags(features: wgpu::TextureFormatFeatures) -> String {
    let f = features.flags;
    format!(
        "4x {} | resolve {} | blend {}",
        if f.sample_count_supported(4) {
            "yes"
        } else {
            "no"
        },
        if f.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE) {
            "yes"
        } else {
            "no"
        },
        if f.contains(wgpu::TextureFormatFeatureFlags::BLENDABLE) {
            "yes"
        } else {
            "no"
        },
    )
}

fn texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    samples: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ID layout probe"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// Milliseconds per cleared and stored 4x pass with a resolved RGBA8 colour
/// and the `extra` attachments, median of `rounds`.
fn pass_ms(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: (u32, u32),
    extra: &[wgpu::TextureFormat],
    rounds: usize,
) -> f64 {
    let color = texture(device, wgpu::TextureFormat::Rgba8Unorm, size, 4);
    let resolve = texture(device, wgpu::TextureFormat::Rgba8Unorm, size, 1);
    let extras: Vec<wgpu::Texture> = extra.iter().map(|f| texture(device, *f, size, 4)).collect();
    let color_view = color.create_view(&Default::default());
    let resolve_view = resolve.create_view(&Default::default());
    let extra_views: Vec<wgpu::TextureView> = extras
        .iter()
        .map(|t| t.create_view(&Default::default()))
        .collect();
    let mut times = Vec::new();
    for _ in 0..rounds {
        let mut attachments = vec![Some(wgpu::RenderPassColorAttachment {
            view: &color_view,
            depth_slice: None,
            resolve_target: Some(&resolve_view),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                store: wgpu::StoreOp::Store,
            },
        })];
        for view in &extra_views {
            attachments.push(Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            }));
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ID layout probe pass"),
            color_attachments: &attachments,
            ..Default::default()
        }));
        let started = Instant::now();
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        times.push(started.elapsed().as_secs_f64() * 1e3);
    }
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

#[test]
fn id_target_layouts_on_a_multisampled_panel_pass() {
    let instance = test_gpu::instance();
    let adapter = test_gpu::adapter(&instance);
    let (device, queue) = test_gpu::request_device(&adapter, &wgpu::DeviceDescriptor::default());
    let mut out = String::new();
    writeln!(out, "adapter: {:?}", adapter.get_info()).unwrap();
    writeln!(
        out,
        "| format | WebGPU baseline (no features) | this adapter | 4x attachment on a default device |"
    )
    .unwrap();
    writeln!(out, "|---|---|---|---|").unwrap();
    let mut legal = Vec::new();
    for format in FORMATS {
        let baseline = format.guaranteed_format_features(wgpu::Features::empty());
        let adapter_features = adapter.get_texture_format_features(format);
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _ = texture(&device, format, (64, 64), 4);
        let error = test_gpu::block_on(scope.pop());
        writeln!(
            out,
            "| {format:?} | {} | {} | {} |",
            flags(baseline),
            flags(adapter_features),
            match error {
                None => {
                    legal.push(format);
                    "created".to_owned()
                }
                Some(error) => format!(
                    "refused: {}",
                    error.to_string().lines().last().unwrap_or("")
                ),
            }
        )
        .unwrap();
    }
    // Store traffic of one more 4x attachment, per 64-bit layout, at 1x and
    // 2x of an 800 × 600 panel: only the layouts this device created.
    let layouts: [(&str, &[wgpu::TextureFormat]); 3] = [
        ("Rg32Uint", &[wgpu::TextureFormat::Rg32Uint]),
        (
            "R32Uint ×2",
            &[wgpu::TextureFormat::R32Uint, wgpu::TextureFormat::R32Uint],
        ),
        ("Rgba16Uint", &[wgpu::TextureFormat::Rgba16Uint]),
    ];
    writeln!(
        out,
        "\n| panel | 4x RGBA8 + resolve, cleared and stored | layout | + 64-bit ID at 4x | ID bytes at 4x | 1-sample 64-bit ID bytes |"
    )
    .unwrap();
    writeln!(out, "|---|---:|---|---:|---:|---:|").unwrap();
    for (label, size) in [
        ("800×600 (1x)", (800, 600)),
        ("1600×1200 (2x)", (1600, 1200)),
    ] {
        let px = u64::from(size.0) * u64::from(size.1);
        let base = pass_ms(&device, &queue, size, &[], 15);
        for (name, formats) in layouts {
            let timed = if formats.iter().all(|f| legal.contains(f)) {
                format!("{:.2} ms", pass_ms(&device, &queue, size, formats, 15))
            } else {
                "not a legal 4x attachment".to_owned()
            };
            writeln!(
                out,
                "| {label} | {base:.2} ms | {name} | {timed} | {} MiB | {} MiB |",
                fmt_mib(px * 4 * 8),
                fmt_mib(px * 8),
            )
            .unwrap();
        }
    }
    println!("{out}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ci-results");
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(dir.join("pick_id_target_formats.txt"), out).ok();
}

fn fmt_mib(bytes: u64) -> String {
    format!("{:.2}", bytes as f64 / (1024.0 * 1024.0))
}

/// The extra latency of the colour snap (#381): copying the resolved
/// panel's pixels at the cursor (1×1 and 3×3) to a mappable buffer and
/// reading them, on a 1600 × 1200 RGBA8 target, median of 21.
#[test]
fn resolved_pixel_copy_latency() {
    let (device, queue) = test_gpu::device();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("resolved panel"),
        size: wgpu::Extent3d {
            width: 1600,
            height: 1200,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut out = String::from("| pixels | copy + map + read, median ms |\n|---|---:|\n");
    for side in [1u32, 3] {
        let mut times = Vec::new();
        for _ in 0..21 {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pixel readback"),
                size: 256 * u64::from(side),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let started = Instant::now();
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 800,
                        y: 600,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(side),
                    },
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let slice = buffer.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let _ = slice.get_mapped_range().expect("mapped")[0];
            times.push(started.elapsed().as_secs_f64() * 1e3);
        }
        times.sort_by(f64::total_cmp);
        writeln!(out, "| {side}×{side} | {:.3} |", times[times.len() / 2]).unwrap();
    }
    println!("{out}");
}

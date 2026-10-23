//! The scatter transform: the uniform every data shader maps data to the
//! panel with, its bind group and its derivation from a Config.

use super::*;

/// Shared transform uniform for scatter / line / errorbar shaders.
///
/// 144 bytes (eight `vec2<f32>` fields, one `array<vec4<f32>, 3>` at
/// offset 64, stride 16, and two polar `vec4<f32>` at 112 and 128, WGSL
/// uniform layout). Pixel sizes (point radius, cap half-length) live in
/// [`PrimitiveStyle`]; shaders convert them to NDC via `pixel_to_ndc`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterTransform {
    pub data_min: [f32; 2],    // offset 0
    pub data_max: [f32; 2],    // offset 8
    pub data_min_lo: [f32; 2], // offset 16
    pub data_max_lo: [f32; 2], // offset 24
    /// Per-axis flag: 0.0 = linear, 1.0 = log10.
    pub scale_log: [f32; 2], // offset 32
    /// `(2 / chart_w, 2 / chart_h)` — 1 pixel in NDC. Shaders multiply pixel
    /// sizes (line width, point radius, cap half-length) by this.
    pub pixel_to_ndc: [f32; 2], // offset 40
    /// Affine from an axis-normalized SSoT coordinate into panel `t`.
    /// Layout margins belong here, never in `data_min` / `data_max`.
    pub data_to_panel_scale: [f32; 2], // offset 48
    pub data_to_panel_offset: [f32; 2], // offset 56
    /// Generic per-panel style parameter slots mirrored by WGSL `Transform`,
    /// packed by the renderer's style table (`StyleVariant::pack_params`, flat
    /// `[f32; 12]` split into three vec4 slots). All zeros in precise mode —
    /// the precise entry points never read them. Sketch:
    /// `[0] = [amplitude_px, wavelength_px, seed as f32, 0.0]`, rest 0;
    /// milkyway: `[0] = [star_density, ribbon_width_px,
    /// ribbon_intensity, seed as f32]`, `[1] = [star_scale, spread_px,
    /// faint_bias, planet_rim]`, `[2] = [structure_scale, star_brightness,
    /// 0, 0]`; constellation: `[0] = [star_opacity, line_opacity, 0.0,
    /// 0.0]`, rest 0. Seeds are stored as f32 (exact up to 2^24) and shaders
    /// recover them via `u32(...)`. Independently of style, `[2][2]` is
    /// reserved for the global point-base u32 bit pattern (resident: zero).
    /// Streamed point/errorbar entries recover it with `bitcast<u32>`, not
    /// numeric conversion, preserving global identities above 2^24.
    pub style_params: [[f32; 4]; 3], // offset 64 → 112 byte
    /// `Config.polar`, all zero in a rectangular chart: `(flag, zero angle
    /// in radians, radians per angle unit signed by the direction, inner
    /// radius fraction)`. With the flag set, `data_to_ndc` maps the x column
    /// to an angle around the data area's centre and the y axis fraction to
    /// a radius between the inner ring and `polar_extent[3]`.
    pub polar_angle: [f32; 4], // offset 112
    /// `(sector start, sector end in angle units; end <= start is the full
    /// circle, arc pieces per line segment or 0 for chords, outer radius in
    /// pixels)`.
    pub polar_extent: [f32; 4], // offset 128 → 144 byte
}

/// Admission headroom for one immutable streamed point/errorbar transform.
pub(crate) const STREAM_POINT_TRANSFORM_BYTES: u64 = std::mem::size_of::<ScatterTransform>() as u64;

/// Snapshot transform metadata with an exact global point identity. The
/// caller must admit [`STREAM_POINT_TRANSFORM_BYTES`] before allocation and
/// validate that its local draw range fits the global u32 index domain.
///
/// The bind group owns the uniform's GPU handle; its matching charge must be
/// retained by the same prepared packet. No source payload is copied, and the
/// shared view transform is never rewritten. Arc-driven line/star identities
/// have their own replay state and must not use this point-base field.
pub(crate) fn create_stream_point_transform_bind_group(
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    transform: &ScatterTransform,
    global_base: u32,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    let mut chunk_transform = *transform;
    chunk_transform.style_params[2][2] = f32::from_bits(global_base);
    let tally = crate::gpu_memory::ChargeTally::new();
    let buffer = crate::gpu_memory::charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy streamed point transform"),
            contents: bytemuck::bytes_of(&chunk_transform),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let bind_group = create_scatter_transform_bind_group(device, layout, &buffer);
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    (bind_group, charge)
}

/// Allocate the transform uniform buffer with `COPY_DST` so subsequent
/// updates can use `queue.write_buffer` instead of recreating it.
pub fn create_scatter_transform_uniform_buffer(
    device: &wgpu::Device,
    transform: &ScatterTransform,
) -> wgpu::Buffer {
    // gpu-alloc: caller
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy scatter transform uniform"),
        contents: bytemuck::bytes_of(transform),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// Overwrite the uniform buffer in place. Call on resize or autoscale change.
pub fn update_scatter_transform(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    transform: &ScatterTransform,
) {
    queue.write_buffer(buffer, 0, bytemuck::bytes_of(transform));
}

/// Bind-group layout for the transform uniform. Render entries use it in the
/// vertex/fragment stages; exact histogram/field picking uses the same bytes in
/// compute so hit geometry cannot diverge from the draw transform.
pub fn create_scatter_transform_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy scatter transform bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT.union(wgpu::ShaderStages::COMPUTE),
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        },
    )
}

/// Bind the transform uniform buffer into the layout above.
pub fn create_scatter_transform_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy scatter transform bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

/// Build a [`ScatterTransform`] from a `Config`.
///
/// The transform copies the Config SSoT axis range into `(hi, lo)` fields.
/// Data-area placement is a separate affine, so layout margins can never
/// silently replace the range the GPU draws against.
pub fn scatter_transform_from_config(config: &Config) -> ScatterTransform {
    let ca = &config.chart_area.0;
    let chart_w = ca.width.max(1) as f32;
    let chart_h = ca.height.max(1) as f32;

    use crate::config::AxisScale;
    let log_x = matches!(config.bottom_x.scale, AxisScale::Logarithmic);
    let log_y = matches!(config.left_y.scale, AxisScale::Logarithmic);

    // For log axes, guard only the axis range. Data values still follow the
    // shader's NaN/non-positive handling.
    let to_log = |v: f64| v.log10();
    let (x_min, x_max) = if log_x {
        crate::chart::guarded_log_range(config.bottom_x.min, config.bottom_x.max)
    } else {
        (config.bottom_x.min, config.bottom_x.max)
    };
    let (y_min, y_max) = if log_y {
        crate::chart::guarded_log_range(config.left_y.min, config.left_y.max)
    } else {
        (config.left_y.min, config.left_y.max)
    };

    let data_min_x = if log_x { to_log(x_min) } else { x_min };
    let data_max_x = if log_x { to_log(x_max) } else { x_max };
    let data_min_y = if log_y { to_log(y_min) } else { y_min };
    let data_max_y = if log_y { to_log(y_max) } else { y_max };

    let scale_log = [if log_x { 1.0 } else { 0.0 }, if log_y { 1.0 } else { 0.0 }];

    // 1 px in NDC: NDC spans 2 across chart_w pixels.
    let pixel_to_ndc = [2.0 / chart_w, 2.0 / chart_h];

    // Per-style shader parameters, packed by the style table's `pack_params`
    // (renderer/style.rs). Precise mode writes zeros — the precise entry points
    // never read them, so the output is unaffected. The export path's
    // `Config::scaled` already multiplied the style's pixel dims (e.g. sketch
    // amplitude/wavelength) by the DPI scale; pack functions read them as-is.
    let packed = match crate::renderer::style_variant(&config.draw_style) {
        Some(v) => (v.pack_params)(&config.draw_style),
        None => [0.0; 12],
    };
    let style_params = [
        [packed[0], packed[1], packed[2], packed[3]],
        [packed[4], packed[5], packed[6], packed[7]],
        [packed[8], packed[9], packed[10], packed[11]],
    ];
    let orient_axis = |min: f64, max: f64, inverted: bool| {
        if inverted { (max, min) } else { (min, max) }
    };
    let (data_to_panel_scale, data_to_panel_offset) = config.data_area().map_or_else(
        |_| ([1.0, 1.0], [0.0, 0.0]),
        |area| {
            let da = area.0;
            let rel_x = (da.x as i64 - ca.x as i64) as f64;
            let rel_y = (da.y as i64 - ca.y as i64) as f64;
            let chart_w64 = f64::from(chart_w);
            let chart_h64 = f64::from(chart_h);
            let sx = rel_x / chart_w64;
            let ex = (rel_x + f64::from(da.width)) / chart_w64;
            let sy = (chart_h64 - (rel_y + f64::from(da.height))) / chart_h64;
            let ey = (chart_h64 - rel_y) / chart_h64;
            ([(ex - sx) as f32, (ey - sy) as f32], [sx as f32, sy as f32])
        },
    );

    let (data_min_x, data_max_x) = orient_axis(data_min_x, data_max_x, config.bottom_x.inverted);
    let (data_min_y, data_max_y) = orient_axis(data_min_y, data_max_y, config.left_y.inverted);
    let (min_x_hi, min_x_lo) = crate::data::split_f64_to_f32_pair(data_min_x);
    let (max_x_hi, max_x_lo) = crate::data::split_f64_to_f32_pair(data_max_x);
    let (min_y_hi, min_y_lo) = crate::data::split_f64_to_f32_pair(data_min_y);
    let (max_y_hi, max_y_lo) = crate::data::split_f64_to_f32_pair(data_max_y);
    let (polar_angle, polar_extent) = transform_constants(config);
    ScatterTransform {
        data_min: [min_x_hi, min_y_hi],
        data_max: [max_x_hi, max_y_hi],
        data_min_lo: [min_x_lo, min_y_lo],
        data_max_lo: [max_x_lo, max_y_lo],
        scale_log,
        pixel_to_ndc,
        data_to_panel_scale,
        data_to_panel_offset,
        style_params,
        polar_angle,
        polar_extent,
    }
}

/// `(polar_angle, polar_extent)` of [`ScatterTransform`] for `config`: all
/// zero for a rectangular chart.
pub(crate) fn transform_constants(config: &Config) -> ([f32; 4], [f32; 4]) {
    let (Some(polar), Some(geometry)) = (&config.polar, config.polar_geometry()) else {
        return ([0.0; 4], [0.0; 4]);
    };
    let [start, end] = polar.sector.unwrap_or([0.0, 0.0]);
    (
        [
            1.0,
            polar.zero_angle_degrees.to_radians() as f32,
            (polar.direction() * polar.angle_unit.radians_per_unit()) as f32,
            polar.inner_radius_fraction,
        ],
        [
            start as f32,
            end as f32,
            crate::line_step::arc_pieces(config.polar.as_ref()) as f32,
            geometry.radius,
        ],
    )
}

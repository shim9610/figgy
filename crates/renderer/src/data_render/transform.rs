//! The scatter transform: the uniform every data shader maps data to the
//! panel with, its bind group and its derivation from a Config.

use super::*;

/// Shared transform uniform for scatter / line / errorbar shaders.
///
/// 288 bytes (eight `vec2<f32>` fields, one `array<vec4<f32>, 3>` at
/// offset 64, stride 16, two polar `vec4<f32>` at 112 and 128, two
/// ternary `vec4<f32>` at 144 and 160, the break table
/// `array<vec4<f32>, 6>` at 176 and its `vec4<f32>` info at 272, WGSL
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
    /// in radians, radians per angle-axis value signed by the direction,
    /// inner radius fraction)`. With the flag set, `data_to_ndc` maps the x column
    /// to an angle around the data area's centre and the y axis fraction to
    /// a radius between the inner ring and `polar_extent[3]`.
    pub polar_angle: [f32; 4], // offset 112
    /// `(sector start, sector end in angle-axis values; end <= start is the
    /// full circle, arc pieces per line segment or 0 for chords, outer radius
    /// in pixels)`.
    pub polar_extent: [f32; 4], // offset 128 → 144 byte
    /// `Config.ternary`, all zero otherwise: the corners L (x's 100 %) and T
    /// (y's 100 %) in NDC, `(L.x, L.y, T.x, T.y)`. With the flag in
    /// `ternary_r` set, a ternary entry places a sample at the corners
    /// weighted by its three components (`ternary_ndc`).
    pub ternary_lt: [f32; 4], // offset 144
    /// `(R.x, R.y, flag, 0)`: the corner R (c's 100 %) in NDC and 1 for a
    /// ternary chart.
    pub ternary_r: [f32; 4], // offset 160
    /// Broken axes (#200): up to [`MAX_AXIS_BREAKS`](crate::config::MAX_AXIS_BREAKS)
    /// cuts per axis, x in `[0, 3)` and y in `[3, 6)`, each `(from, to,
    /// start, end)`: the raw `t` interval of the cut values and the gap it
    /// is drawn as ([`crate::layout::BreakKnot`]), in the shader's `t`,
    /// which the `inverted` swap of `data_min` / `data_max` has already
    /// mirrored. Unused entries are zero.
    pub breaks: [[f32; 4]; 6], // offset 176
    /// `(cuts on x, cuts on y, piece scale x, piece scale y)`; zero cuts is
    /// the identity.
    pub break_info: [f32; 4], // offset 272 → 288 byte
}

/// The break table entries of an unbroken chart.
pub(crate) const NO_BREAKS: [[f32; 4]; 6] = [[0.0; 4]; 6];
/// The break info of an unbroken chart.
pub(crate) const NO_BREAK_INFO: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// `(breaks, break_info)` of [`ScatterTransform`] for the x axis `x` and
/// the y axis `y`: their [`crate::layout::BreakMap`]s in the shader's `t`,
/// mirrored on an inverted axis, where the shader's `t` runs from the
/// axis' far end.
pub(crate) fn break_constants(
    x: &crate::config::AxisOptions,
    y: &crate::config::AxisOptions,
) -> ([[f32; 4]; 6], [f32; 4]) {
    let mut breaks = NO_BREAKS;
    let mut info = NO_BREAK_INFO;
    for (axis, options) in [x, y].into_iter().enumerate() {
        // A `Panels` axis has no cut remap: each piece is drawn at its own
        // linear map ([`panels_scale`], `DataPieces`).
        if panels_scale(options).is_some() {
            continue;
        }
        let map = crate::layout::BreakMap::of(options);
        let knots = map.knots();
        info[axis] = knots.len() as f32;
        info[2 + axis] = map.scale() as f32;
        for (k, knot) in knots.iter().enumerate() {
            let entry = if options.inverted {
                // The mirror of the cuts: the last one first.
                let knot = knots[knots.len() - 1 - k];
                [
                    1.0 - knot.to,
                    1.0 - knot.from,
                    1.0 - knot.end,
                    1.0 - knot.start,
                ]
            } else {
                [knot.from, knot.to, knot.start, knot.end]
            };
            breaks[3 * axis + k] = entry.map(|v| v as f32);
        }
    }
    (breaks, info)
}

/// The piece scale of a broken axis drawn as [`AxisBreakStyle::Panels`]
/// (#200), `None` for any other axis. Such an axis' transform maps data at
/// its first piece's linear map (the shader's `t` times this scale) and each
/// piece is drawn with that map moved by the piece's shift, so every piece
/// keeps its own linear scale and a mark crossing a gap is clipped at each
/// piece's edge.
///
/// [`AxisBreakStyle::Panels`]: crate::config::AxisBreakStyle::Panels
pub(crate) fn panels_scale(axis: &crate::config::AxisOptions) -> Option<f64> {
    let style = axis.breaks.as_ref()?.style;
    let map = crate::layout::BreakMap::of(axis);
    (style == crate::config::AxisBreakStyle::Panels && map.is_broken()).then(|| map.scale())
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
) -> crate::Result<(wgpu::BindGroup, crate::gpu_memory::SharedCharge)> {
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
    )?;
    let bind_group = create_scatter_transform_bind_group(device, layout, &buffer);
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    Ok((bind_group, charge))
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
    // A categorical axis is linear over its category range.
    let log_x = matches!(config.bottom_x.scale, AxisScale::Logarithmic)
        && config.bottom_x.categories.is_none();
    let log_y =
        matches!(config.left_y.scale, AxisScale::Logarithmic) && config.left_y.categories.is_none();

    // For log axes, guard only the axis range. Data values still follow the
    // shader's NaN/non-positive handling.
    let to_log = |v: f64| v.log10();
    let (x_min, x_max) = if log_x {
        crate::chart::guarded_log_range(config.bottom_x.min, config.bottom_x.max)
    } else {
        config.bottom_x.data_range()
    };
    let (y_min, y_max) = if log_y {
        crate::chart::guarded_log_range(config.left_y.min, config.left_y.max)
    } else {
        config.left_y.data_range()
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
    // A `Panels` axis maps at its first piece's linear map: `t` times the
    // piece scale, a range shortened by that factor from its start.
    let at_first_piece = |min: f64, max: f64, axis: &crate::config::AxisOptions| {
        panels_scale(axis).map_or(max, |scale| min + (max - min) / scale)
    };
    let data_max_x = at_first_piece(data_min_x, data_max_x, &config.bottom_x);
    let data_max_y = at_first_piece(data_min_y, data_max_y, &config.left_y);
    let (min_x_hi, min_x_lo) = crate::data::split_f64_to_f32_pair(data_min_x);
    let (max_x_hi, max_x_lo) = crate::data::split_f64_to_f32_pair(data_max_x);
    let (min_y_hi, min_y_lo) = crate::data::split_f64_to_f32_pair(data_min_y);
    let (max_y_hi, max_y_lo) = crate::data::split_f64_to_f32_pair(data_max_y);
    let (polar_angle, polar_extent) = transform_constants(config);
    let (ternary_lt, ternary_r) = ternary_constants(config, *ca, chart_w, chart_h);
    let (breaks, break_info) = break_constants(&config.bottom_x, &config.left_y);
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
        ternary_lt,
        ternary_r,
        breaks,
        break_info,
    }
}

/// `(ternary_lt, ternary_r)` of [`ScatterTransform`] for `config`: the
/// triangle's corners ([`Config::ternary_geometry`]) in the NDC of the
/// panel `ca` of `chart_w × chart_h` pixels, all zero for any other chart.
pub(crate) fn ternary_constants(
    config: &Config,
    ca: crate::layout::Rect,
    chart_w: f32,
    chart_h: f32,
) -> ([f32; 4], [f32; 4]) {
    let Some(geometry) = config.ternary_geometry() else {
        return ([0.0; 4], [0.0; 4]);
    };
    let ndc = |p: [f32; 2]| {
        [
            (p[0] - ca.x as f32) / chart_w * 2.0 - 1.0,
            1.0 - (p[1] - ca.y as f32) / chart_h * 2.0,
        ]
    };
    let [l, t, r] = geometry.corners.map(ndc);
    ([l[0], l[1], t[0], t[1]], [r[0], r[1], 1.0, 0.0])
}

/// `(polar_angle, polar_extent)` of [`ScatterTransform`] for `config`: all
/// zero for a rectangular chart. The angle words are the chart's
/// [`AngleMap`](crate::polar::AngleMap): a numeric angle axis reads its
/// values in the angle unit from the zero angle inside the sector, and a
/// categorical one puts category `i` at `i / n` of the turn or the sector,
/// its window `[0, n]`.
pub(crate) fn transform_constants(config: &Config) -> ([f32; 4], [f32; 4]) {
    let (Some(polar), Some(map), Some(geometry)) = (
        config.polar(),
        config.polar_angle_map(),
        config.polar_geometry(),
    ) else {
        return ([0.0; 4], [0.0; 4]);
    };
    let [start, end] = map.window.unwrap_or([0.0, 0.0]);
    (
        [
            1.0,
            map.zero_radians as f32,
            map.radians_per_value as f32,
            polar.inner_radius_fraction,
        ],
        [
            start as f32,
            end as f32,
            crate::line_step::arc_pieces(config.polar()) as f32,
            geometry.radius,
        ],
    )
}

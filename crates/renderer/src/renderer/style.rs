//! GPU styles for series: the draw-style variants and their packed
//! parameters, the style sets and style maps, and the create_style_for_series
//! entry points.

use super::*;

/// Discriminant for cached per-style pipeline sets.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum StyleKey {
    Sketch,
    Milkyway,
    Constellation,
}

/// Everything the renderer needs to know about one stylized mode up front;
/// the per-format GPU objects live in the [`StyleSet`] the key maps to.
pub(crate) struct StyleVariant {
    pub(crate) key: StyleKey,
    pub(crate) needs_arc_prefix: bool,
    /// Packs all three vec4 slots mirrored by
    /// [`data_render::ScatterTransform::style_params`].
    pub(crate) pack_params: fn(&DrawStyle) -> [f32; 12],
}

/// sketch: `[0] = (amplitude_px, wavelength_px, seed, 0)`, rest 0.
pub(super) fn pack_sketch_params(style: &DrawStyle) -> [f32; 12] {
    match style.sketch() {
        Some(crate::config::SketchOptions {
            amplitude_px,
            wavelength_px,
            seed,
        }) => [
            *amplitude_px,
            *wavelength_px,
            *seed as f32,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        None => [0.0; 12],
    }
}

/// milkyway: `[0] = (star_density, ribbon_width_px, ribbon_intensity,
/// seed)`, `[1] = (star_scale, spread_px, faint_bias, planet_rim)`,
/// `[2] = (structure_scale, star_brightness, 0, 0)`.
/// `glow`/`nebula`/`dust` are CPU-raster parameters and never reach the GPU.
pub(super) fn pack_milkyway_params(style: &DrawStyle) -> [f32; 12] {
    match style.milkyway() {
        Some(crate::config::MilkywayOptions {
            star_density,
            ribbon_width_px,
            ribbon_intensity,
            star_scale,
            star_brightness,
            spread_px,
            structure_scale,
            faint_bias,
            glow: _,
            nebula: _,
            dust: _,
            planet_rim,
            seed,
        }) => [
            *star_density,
            *ribbon_width_px,
            *ribbon_intensity,
            *seed as f32,
            *star_scale,
            *spread_px,
            *faint_bias,
            *planet_rim,
            *structure_scale,
            *star_brightness,
            0.0,
            0.0,
        ],
        None => [0.0; 12],
    }
}

/// constellation: `[0] = (star_opacity, line_opacity, 0, 0)`,
/// `[1]`/`[2]` = 0.
/// Only ScatterLine series use this style.
pub(super) fn pack_constellation_params(style: &DrawStyle) -> [f32; 12] {
    match style.constellation() {
        Some(crate::config::ConstellationOptions {
            star_opacity,
            line_opacity,
        }) => [
            *star_opacity,
            *line_opacity,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        None => [0.0; 12],
    }
}

pub(super) static SKETCH_VARIANT: StyleVariant = StyleVariant {
    key: StyleKey::Sketch,
    needs_arc_prefix: true,
    pack_params: pack_sketch_params,
};

pub(super) static MILKYWAY_VARIANT: StyleVariant = StyleVariant {
    key: StyleKey::Milkyway,
    // Ribbon profile and every star attribute are arc-length parameterized.
    needs_arc_prefix: true,
    pack_params: pack_milkyway_params,
};

pub(super) static CONSTELLATION_VARIANT: StyleVariant = StyleVariant {
    key: StyleKey::Constellation,
    needs_arc_prefix: false,
    pack_params: pack_constellation_params,
};

/// Style lookup. None for Precise.
pub(crate) fn style_variant(style: &DrawStyle) -> Option<&'static StyleVariant> {
    match style {
        DrawStyle::Precise => None,
        DrawStyle::Sketch(_) => Some(&SKETCH_VARIANT),
        DrawStyle::Milkyway(_) => Some(&MILKYWAY_VARIANT),
        DrawStyle::Constellation(_) => Some(&CONSTELLATION_VARIANT),
    }
}

/// One style's compiled GPU objects against one target format. Styles differ
/// structurally (sketch swaps entry points; milkyway draws additive star
/// passes and carries baked textures; constellation renders scatter stars
/// plus a connecting line), so the set is an enum and the draw side matches
/// once.
pub(super) enum StyleSet {
    Sketch {
        line: wgpu::RenderPipeline,
        scatter: wgpu::RenderPipeline,
        errorbar: wgpu::RenderPipeline,
        line_verts: u32,
    },
    Milkyway(data_render::MilkywaySet),
    Constellation(data_render::PointConstellationSet),
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_style_set(
    key: StyleKey,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    shaders: &data_render::ShaderModules,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    star_data_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
) -> StyleSet {
    match key {
        StyleKey::Sketch => StyleSet::Sketch {
            line: data_render::create_line_columnar_pipeline_with_entries(
                device,
                &shaders.line,
                transform_bgl,
                style_bgl,
                surface_format,
                sample_count,
                "vs_sketch",
                "fs_main",
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                wgpu::PrimitiveTopology::TriangleStrip,
                None,
                "figgy line styled pipeline",
            ),
            scatter: data_render::create_scatter_columnar_pipeline_with_entries(
                device,
                &shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_format,
                sample_count,
                "vs_sketch",
                "fs_sketch",
                "figgy scatter styled pipeline",
            ),
            errorbar: data_render::create_errorbar_columnar_pipeline_with_entries(
                device,
                &shaders.errorbar,
                transform_bgl,
                style_bgl,
                surface_format,
                sample_count,
                "vs_sketch",
                "figgy errorbar styled pipeline",
            ),
            line_verts: data_render::LINE_SKETCH_VERTICES_PER_INSTANCE,
        },
        StyleKey::Milkyway => StyleSet::Milkyway(data_render::create_milkyway_set(
            device,
            queue,
            shaders,
            transform_bgl,
            style_bgl,
            star_data_bgl,
            surface_format,
            sample_count,
        )),
        StyleKey::Constellation => {
            StyleSet::Constellation(data_render::create_point_constellation_set(
                device,
                queue,
                shaders,
                transform_bgl,
                style_bgl,
                surface_format,
                sample_count,
            ))
        }
    }
}

pub(super) fn premul_rgba(c: Color) -> [f32; 4] {
    let a = c.a.clamp(0.0, 1.0);
    [c.r * a, c.g * a, c.b * a, a]
}

pub(super) fn scatter_style_slot_gpu(
    slot: &DataScatterPointStyleConfig,
    scale: f32,
    mask_color: u32,
    mask_radius: u32,
    mask_shape: u32,
) -> data_render::ScatterStyleSlotGpu {
    let DataScatterPointStyleConfig {
        point_color,
        point_shape,
        point_size,
    } = slot;
    let mut mask = 0u32;
    let mut color_premul = [0.0; 4];
    let mut radius_px = 0.0;
    let mut shape_id = 0.0;
    if let Some(c) = *point_color {
        mask |= mask_color;
        color_premul = premul_rgba(c);
    }
    if let Some(size) = *point_size {
        mask |= mask_radius;
        radius_px = size * scale;
    }
    if let Some(shape) = point_shape {
        mask |= mask_shape;
        shape_id = data_render::shape_id(shape) as f32;
    }
    data_render::ScatterStyleSlotGpu {
        color_premul,
        meta: [radius_px, shape_id, mask as f32, 0.0],
    }
}

pub(super) fn errorbar_style_slot_gpu(
    slot: &DataErrorBarPointStyleConfig,
    scale: f32,
    mask_color: u32,
    mask_stem_width: u32,
    mask_cap_half: u32,
    mask_cap_width: u32,
) -> data_render::ErrorBarStyleSlotGpu {
    let DataErrorBarPointStyleConfig {
        error_bar_color,
        error_bar_width,
        error_bar_cap_size,
        cap_width: cap_width_px,
    } = slot;
    let mut mask = 0u32;
    let mut color_premul = [0.0; 4];
    let mut stem_width = 0.0;
    let mut cap_half = 0.0;
    let mut cap_width = 0.0;
    if let Some(c) = *error_bar_color {
        mask |= mask_color;
        color_premul = premul_rgba(c);
    }
    if let Some(width) = *error_bar_width {
        mask |= mask_stem_width;
        stem_width = width * scale;
    }
    if let Some(size) = *error_bar_cap_size {
        mask |= mask_cap_half;
        cap_half = size * scale;
    }
    if let Some(width) = *cap_width_px {
        mask |= mask_cap_width;
        cap_width = width * scale;
    }
    data_render::ErrorBarStyleSlotGpu {
        color_premul,
        meta: [stem_width, cap_half, mask as f32, cap_width],
    }
}

pub(super) fn bar_style_slot_gpu(
    slot: &DataBarBinStyleConfig,
    scale: f32,
) -> data_render::BarStyleSlotGpu {
    const MASK_FILL: u32 = 1;
    const MASK_BORDER_COLOR: u32 = 2;
    const MASK_BORDER_WIDTH: u32 = 4;
    const MASK_GAP: u32 = 8;
    const MASK_WIDTH_RATIO: u32 = 16;

    let DataBarBinStyleConfig {
        fill_color,
        border_color,
        border_width,
        gap_px: slot_gap_px,
        width_ratio: slot_width_ratio,
    } = slot;
    let mut mask = 0u32;
    let mut fill_color_premul = [0.0; 4];
    let mut border_color_premul = [0.0; 4];
    let mut border_width_px = 0.0;
    let mut gap_px = 0.0;
    let mut width_ratio = 1.0;
    if let Some(color) = *fill_color {
        mask |= MASK_FILL;
        fill_color_premul = premul_rgba(color);
    }
    if let Some(color) = *border_color {
        mask |= MASK_BORDER_COLOR;
        border_color_premul = premul_rgba(color);
    }
    if let Some(width) = *border_width {
        mask |= MASK_BORDER_WIDTH;
        border_width_px = if width.is_finite() {
            width.max(0.0) * scale
        } else {
            0.0
        };
    }
    if let Some(gap) = *slot_gap_px {
        mask |= MASK_GAP;
        gap_px = if gap.is_finite() {
            gap.max(0.0) * scale
        } else {
            0.0
        };
    }
    if let Some(ratio) = *slot_width_ratio {
        mask |= MASK_WIDTH_RATIO;
        width_ratio = data_render::sanitize_bar_width_ratio(ratio);
    }
    data_render::BarStyleSlotGpu {
        fill_color_premul,
        border_color_premul,
        params: [border_width_px, gap_px, width_ratio, mask as f32],
        material: [0.0; 4],
        finish: [0.0; 4],
    }
}

/// The per-bar overrides of a category bar series as the bar override table
/// its draw, pick and highlight resolve by row: each override's category id
/// is looked up in `categories` (the axis' categories), its colour sets the
/// fill and its style the outline, the material, the corner radius and the
/// emphasis. An override whose category is not on the axis is skipped (the
/// gate rejects it). An empty list binds the empty table.
pub(super) fn category_bar_style_map(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    categories: &[crate::config::Category],
    bars: crate::data_config::CategoryBarsRef<'_>,
    scale: f32,
) -> data_render::BarStyleMap {
    const MASK_FILL: u32 = 1;
    const MASK_BORDER_COLOR: u32 = 2;
    const MASK_BORDER_WIDTH: u32 = 4;
    let overrides: Vec<data_render::BarStyleOverrideGpu> = bars
        .overrides
        .iter()
        .filter_map(|o| {
            let row = categories
                .iter()
                .position(|c| c.id == o.target.category_id)?;
            let mut record = data_render::BarStyleOverrideGpu {
                bin_index: u32::try_from(row).ok()?,
                ..bytemuck::Zeroable::zeroed()
            };
            let mut mask = 0;
            if let Some(color) = o.color {
                mask |= MASK_FILL;
                record.fill_color_premul = premul_rgba(color);
            }
            if let Some(style) = &o.style {
                mask |=
                    MASK_BORDER_COLOR | MASK_BORDER_WIDTH | data_render::BAR_STYLE_MASK_MATERIAL;
                record.border_color_premul = premul_rgba(style.outline_color);
                record.params[0] = if style.outline {
                    style.outline_width.max(0.0) * scale
                } else {
                    0.0
                };
                record.material = data_render::category_bar_material(style);
                record.finish = [
                    style.corner_radius.max(0.0) * scale,
                    style.emphasis_brightness,
                    0.0,
                    0.0,
                ];
            }
            record.params[3] = mask as f32;
            Some(record)
        })
        .collect();
    data_render::create_bar_style_map(device, layout, &overrides)
}

/// Bundle of style bind groups for line, scatter, and errorbar primitives.
/// Built via `Renderer::create_style_for_series*`. Multiple series in one
/// chart can share a single `ChartStyle` (`Series.style: &ChartStyle`).
///
/// The style uniform buffers live inside the bind groups — wgpu keeps bound
/// resources alive, so no named buffer fields are needed.
pub struct ChartStyle {
    pub(super) _charge: crate::gpu_memory::SharedCharge,
    pub(super) line_bg: wgpu::BindGroup,
    /// The area fill's `Style` (fill colour, baseline, fill flags), only when
    /// the series' line declares a fill.
    pub(super) area_bg: Option<wgpu::BindGroup>,
    pub(super) scatter_bg: wgpu::BindGroup,
    pub(super) errorbar_bg: wgpu::BindGroup,
    pub(super) bar_bg: wgpu::BindGroup,
    pub(super) scatter_map: Option<data_render::ScatterStyleMap>,
    pub(super) errorbar_map: Option<data_render::ErrorBarStyleMap>,
    pub(super) bar_map: Option<data_render::BarStyleMap>,
    pub(super) display_scale: f32,
    pub(super) scatter_radius_px: f32,
}

/// One drawable series — `data_config::SeriesConfig` (declarative) plus a
/// `ChartStyle` (GPU uniform buffers). The unit of input to `Renderer::paint`.
#[derive(Clone, Copy)]
pub struct Series<'a> {
    /// Column ids + render type + label. Pure declaration, no GPU state.
    pub config: &'a SeriesConfig,
    /// GPU style created by `Renderer::create_style_for_series`.
    pub style: &'a ChartStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EffectiveSeriesPrimitives {
    pub(super) field: bool,
    pub(super) contour: bool,
    pub(super) line: bool,
    pub(super) scatter: bool,
    pub(super) bar: bool,
    /// A category bar series' bars. Precise mode only, like histogram bars.
    pub(super) category_bars: bool,
    /// A vector (quiver) series' arrows. Precise mode only, like fields.
    pub(super) vector: bool,
}

pub(super) fn effective_series_primitives(
    draw_style: &DrawStyle,
    render_type: &DataRenderType,
) -> EffectiveSeriesPrimitives {
    if matches!(draw_style, DrawStyle::Constellation(_)) {
        return EffectiveSeriesPrimitives {
            field: false,
            contour: false,
            line: constellation_supports(render_type),
            scatter: constellation_supports(render_type),
            bar: false,
            category_bars: false,
            vector: false,
        };
    }
    EffectiveSeriesPrimitives {
        // A field is a precise-mode primitive, for the same reason bars are: a
        // stylized mode has no field entry point, and a precise field inside a
        // sketch panel would mix two visual languages.
        field: matches!(draw_style, DrawStyle::Precise) && has_field(render_type),
        // Contours reuse the precise line pipeline, so like the field they are a
        // precise-mode primitive: a stylized panel has no entry point that would
        // draw them in its own visual language.
        contour: matches!(draw_style, DrawStyle::Precise) && has_contour(render_type),
        line: has_line(render_type),
        scatter: has_scatter(render_type),
        // Bars are a precise-mode primitive. A stylized mode has no bar entry
        // point, and drawing the precise one inside a sketch/milkyway chart
        // would mix two visual languages in one panel — so a stylized chart
        // draws no bars at all, the same answer `constellation_supports` gives.
        bar: matches!(draw_style, DrawStyle::Precise) && has_bar(render_type),
        // Category bars are precise like histogram bars, for the same reason.
        category_bars: matches!(draw_style, DrawStyle::Precise)
            && render_type.category_bars().is_some(),
        // Arrows are a precise-mode primitive for the same reason: no stylized
        // entry draws them, and a precise arrow in a sketch panel would mix
        // two visual languages. A stylized chart draws no arrows.
        vector: matches!(draw_style, DrawStyle::Precise) && has_vector(render_type),
    }
}

/// Whether a line series with a fill draws it under `draw_style`: the fill
/// follows the line, so it is drawn exactly when the style draws the line.
/// Not part of [`EffectiveSeriesPrimitives`] because the fill is a property of
/// the line (it picks as part of the line, with no pick registry entry of its
/// own).
pub(super) fn series_has_line_fill(draw_style: &DrawStyle, render_type: &DataRenderType) -> bool {
    effective_series_primitives(draw_style, render_type).line
        && extract_line(render_type).is_some_and(|line| line.fill.is_some())
}

/// Whether `style` draws every primitive `render_type` draws in the precise
/// style.
///
/// `false` exactly when the style has no entry point for one of those
/// primitives: Sketch and Milkyway draw line, scatter and errorbar series but
/// not bars, fields, contours or vectors; Constellation draws marker lines
/// (`ScatterLine`) only. Prepare then skips that series without an error, so a
/// host that validates with this predicate can reject the combination
/// instead of drawing an empty plot. It compares the per-style primitive
/// selection prepare itself uses against the precise one, so the two cannot
/// drift apart. Errorbars have no primitive of their own here: they follow the
/// series' scatter, and Constellation drops errorbar series entirely.
///
/// `true` answers for resident drawing. Streaming has its own limits and can
/// still reject a supported combination with an error; see
/// `inspect_streaming_capability`.
pub fn draw_style_supports(style: &DrawStyle, render_type: &DataRenderType) -> bool {
    effective_series_primitives(style, render_type)
        == effective_series_primitives(&DrawStyle::Precise, render_type)
}

/// Whether the constellation style has a rendering for this render type.
///
/// Exhaustive rather than `matches!(rt, ScatterLine { .. })` for the same reason
/// as the predicates below: a new variant defaulting to `false` here would be
/// *correct* — a stylized mode that cannot draw bars should draw nothing rather
/// than something wrong — but it would be correct by accident. The compiler
/// stopping here makes it a decision.
pub(super) fn constellation_supports(rt: &DataRenderType) -> bool {
    match rt {
        DataRenderType::ScatterLine { .. } => true,
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. }
        | DataRenderType::CategoryBars { .. } => false,
    }
}

impl Renderer {
    /// Build a budget-checked `ChartStyle` from `cfg.render_type`'s
    /// line/scatter/errorbar styles. Missing components default to BLACK / 1 px.
    /// The four uniforms and any mapped-style rows are admitted and charged
    /// together before their GPU buffers are created.
    /// This is a derived snapshot: after editing the series' appearance, mappings
    /// or identity, recreate it before [`Self::prepare`]. That method does not
    /// update caller-owned styles. The registered-chart path manages its own cache.
    pub fn create_style_for_series(&mut self, cfg: &SeriesConfig) -> Result<ChartStyle> {
        self.create_style_for_series_scaled(cfg, 1.0)
    }

    /// Scaled variant of `create_style_for_series` — every pixel size (line
    /// width, dash lengths, point radius, errorbar widths/cap) is multiplied
    /// by `scale` (used by the high-DPI export path).
    pub fn create_style_for_series_scaled(
        &mut self,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<ChartStyle> {
        self.create_style_for_series_scaled_inner(cfg, scale)
    }

    pub(super) fn create_style_for_series_scaled_inner(
        &self,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<ChartStyle> {
        let style_bytes = self.style_allocation_bytes(cfg)?;
        let requested = self
            .gpu_memory_usage()
            .checked_total_bytes()
            .and_then(|bytes| bytes.checked_add(style_bytes))
            .ok_or(FiggyError::GpuResourceLimit {
                resource: "chart style budget",
                requested: u64::MAX,
                limit: self.memory_budget.unwrap_or(u64::MAX),
            })?;
        if requested > self.memory_budget.unwrap_or(u64::MAX) {
            return Err(FiggyError::GpuResourceLimit {
                resource: "chart style budget",
                requested,
                limit: self.memory_budget.unwrap_or(u64::MAX),
            });
        }
        // Decorrelates per-series hash patterns in the styled shader entries
        // (star placement, sketch wobble). Derived from the series id, so the
        // result stays deterministic for a given config; renaming a series
        // re-rolls its pattern.
        let SeriesConfig {
            series_id,
            // Identity and column bindings: not part of the GPU style.
            source_id: _,
            label: _,
            x_column: _,
            y_column: _,
            render_type,
        } = cfg;
        let series_salt = crate::sketch::fnv1a(series_id);
        let line = match extract_line(render_type) {
            Some(DataLineStyleConfig {
                line_style,
                line_color,
                line_width,
                // The fill has its own `Style` (`area` below).
                fill: _,
                fill_color: _,
                interpolation,
            }) => {
                let mut s = PrimitiveStyle::from_color_with_width(*line_color, line_width * scale);
                s.pack_dash_pattern(line_style.pattern(), scale);
                // Bits 2-3: the step interpolation mode the line entries read.
                s.primitive_flags = crate::line_step::step_flag_bits(*interpolation);
                s
            }
            None => PrimitiveStyle::from_color_with_width(Color::BLACK, 1.0),
        };
        // The fill's `Style` reinterpretation (SHADER_COMMON.md §2.2): fill
        // colour, the step mode, the fill-to-series bit and the baseline as
        // the pool's `(hi, lo)` pair in `dash[1].xy`.
        let area = extract_line(render_type)
            .and_then(|l| l.fill.as_ref().map(|fill| (l, fill)))
            .map(|(l, fill)| {
                let DataLineStyleConfig {
                    // Dashes and width belong to the stroke, not the fill.
                    line_style: _,
                    line_width: _,
                    // Read through `effective_fill_color`.
                    line_color: _,
                    fill_color: _,
                    // Matched above.
                    fill: _,
                    interpolation,
                } = l;
                let mut s = PrimitiveStyle::from_color(l.effective_fill_color());
                s.primitive_flags = crate::line_step::step_flag_bits(*interpolation);
                match fill {
                    LineFill::ToBaseline(baseline) => {
                        let (hi, lo) = crate::data::split_f64_to_f32_pair(*baseline);
                        s.dash[1][0] = hi;
                        s.dash[1][1] = lo;
                    }
                    LineFill::ToSeries(_) => {
                        s.primitive_flags |= crate::line_step::AREA_FILL_TO_SERIES;
                    }
                }
                s.series_salt = series_salt;
                s
            });
        let scatter = match extract_scatter(render_type) {
            Some(DataScatterStyleConfig {
                point_color,
                point_shape,
                point_size,
                // Per-point styles: the scatter style map below.
                point_style_table: _,
                point_style_index_column: _,
                point_style_overrides: _,
                point_color_map: _,
                point_size_map: _,
            }) => {
                let mut s = PrimitiveStyle::from_color(*point_color);
                s.point_radius_px = point_size * scale;
                s.shape_id = data_render::shape_id(point_shape);
                s
            }
            None => PrimitiveStyle::from_color(Color::BLACK),
        };
        let mut errorbar = match extract_errorbar_style(render_type) {
            Some(DataErrorBarStyleConfig {
                error_bar_color,
                error_bar_width,
                error_bar_cap_size,
                cap_width,
                // Per-point styles: the error-bar style map below.
                error_bar_style_table: _,
                error_bar_style_index_column: _,
                error_bar_style_overrides: _,
            }) => {
                let mut s = PrimitiveStyle::from_color_with_width(
                    *error_bar_color,
                    error_bar_width * scale,
                );
                s.cap_half_px = error_bar_cap_size * scale;
                s.cap_width_px = cap_width * scale;
                s
            }
            None => PrimitiveStyle::from_color(Color::BLACK),
        };
        errorbar.primitive_flags = errorbar_direction_flags(render_type);
        // Bars reinterpret the same `Style` bytes — fill, border, gap, baseline,
        // orientation. SHADER_COMMON.md's bar reinterpretation table is the SSoT.
        // A category bar series' lanes are its fill, outline and material
        // (SHADER_COMMON.md §5.13); its layout is prepared with its group.
        let bar = match (extract_bar(render_type), render_type.category_bars()) {
            (Some(b), _) => PrimitiveStyle::from_bar(b, scale),
            (None, Some(bars)) => PrimitiveStyle::from_category_bars(bars.color, bars.style, scale),
            (None, None) => PrimitiveStyle::from_color(Color::BLACK),
        };
        let (mut line, mut scatter, mut bar) = (line, scatter, bar);
        line.series_salt = series_salt;
        scatter.series_salt = series_salt;
        errorbar.series_salt = series_salt;
        bar.series_salt = series_salt;
        let scatter_map = extract_scatter(render_type)
            .and_then(|sc| self.create_scatter_style_map_for(sc, scale));
        let errorbar_map = extract_errorbar_style(render_type)
            .and_then(|eb| self.create_errorbar_style_map_for(eb, scale));
        let bar_map =
            extract_bar(render_type).and_then(|bar| self.create_bar_style_map_for(bar, scale));
        Ok(self.create_style_from_primitives(
            line,
            area,
            scatter,
            errorbar,
            bar,
            scatter_map,
            errorbar_map,
            bar_map,
            scale,
            style_bytes,
        ))
    }

    pub(super) fn style_allocation_bytes(&self, cfg: &SeriesConfig) -> Result<u64> {
        let caps = self.device.limits();
        let max_storage = caps
            .max_buffer_size
            .min(u64::from(caps.max_storage_buffer_binding_size));
        let primitive_uniforms =
            4 + u64::from(extract_line(&cfg.render_type).is_some_and(|line| line.fill.is_some()));
        let mut total = primitive_uniforms * std::mem::size_of::<PrimitiveStyle>() as u64;
        let overflow = || FiggyError::GpuResourceLimit {
            resource: "chart style bytes",
            requested: u64::MAX,
            limit: self.memory_budget.unwrap_or(u64::MAX),
        };
        let mut add_map = |slots: usize,
                           slot_bytes: u64,
                           overrides: usize,
                           override_row_bytes: u64|
         -> Result<()> {
            let slots = u64::try_from(slots).map_err(|_| overflow())?;
            let overrides = u64::try_from(overrides).map_err(|_| overflow())?;
            let style_bytes = slots.max(1).checked_mul(slot_bytes).ok_or_else(overflow)?;
            let override_bytes = overrides
                .max(1)
                .checked_mul(override_row_bytes)
                .ok_or_else(overflow)?;
            if style_bytes > max_storage || override_bytes > max_storage {
                return Err(FiggyError::GpuResourceLimit {
                    resource: "chart style map buffer",
                    requested: style_bytes.max(override_bytes),
                    limit: max_storage,
                });
            }
            total = total
                .checked_add(style_bytes)
                .and_then(|bytes| bytes.checked_add(override_bytes))
                .and_then(|bytes| bytes.checked_add(16))
                .ok_or_else(overflow)?;
            Ok(())
        };
        if let Some(sc) = extract_scatter(&cfg.render_type) {
            let slots = sc.point_style_table.as_ref().map_or(0, Vec::len);
            let overrides = sc.point_style_overrides.as_ref().map_or(0, |rows| {
                rows.iter()
                    .filter(|row| u32::try_from(row.index).is_ok())
                    .count()
            });
            if sc.point_style_index_column.is_some()
                || slots != 0
                || overrides != 0
                || sc.point_color_map.is_some()
                || sc.point_size_map.is_some()
            {
                add_map(
                    slots,
                    std::mem::size_of::<data_render::ScatterStyleSlotGpu>() as u64,
                    overrides,
                    std::mem::size_of::<data_render::ScatterStyleOverrideGpu>() as u64,
                )?;
            }
        }
        if let Some(eb) = extract_errorbar_style(&cfg.render_type) {
            let slots = eb.error_bar_style_table.as_ref().map_or(0, Vec::len);
            let overrides = eb.error_bar_style_overrides.as_ref().map_or(0, |rows| {
                rows.iter()
                    .filter(|row| u32::try_from(row.index).is_ok())
                    .count()
            });
            if eb.error_bar_style_index_column.is_some() || slots != 0 || overrides != 0 {
                add_map(
                    slots,
                    std::mem::size_of::<data_render::ErrorBarStyleSlotGpu>() as u64,
                    overrides,
                    std::mem::size_of::<data_render::ErrorBarStyleOverrideGpu>() as u64,
                )?;
            }
        }
        if let Some(bar) = extract_bar(&cfg.render_type) {
            let overrides = bar.bar_style_overrides.as_ref().map_or(0, |rows| {
                rows.iter()
                    .filter(|row| u32::try_from(row.index).is_ok())
                    .count()
            });
            if overrides != 0 {
                add_map(
                    1,
                    std::mem::size_of::<data_render::BarStyleSlotGpu>() as u64,
                    overrides,
                    std::mem::size_of::<data_render::BarStyleOverrideGpu>() as u64,
                )?;
            }
        }
        Ok(total)
    }

    /// Shared tail of `create_style_for_series*`: allocate the four style
    /// uniform buffers and bind groups from fully-built values.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_style_from_primitives(
        &self,
        line: PrimitiveStyle,
        area: Option<PrimitiveStyle>,
        scatter: PrimitiveStyle,
        errorbar: PrimitiveStyle,
        bar: PrimitiveStyle,
        scatter_map: Option<data_render::ScatterStyleMap>,
        errorbar_map: Option<data_render::ErrorBarStyleMap>,
        bar_map: Option<data_render::BarStyleMap>,
        display_scale: f32,
        style_bytes: u64,
    ) -> ChartStyle {
        let dev = &self.device;
        let scatter_radius_px = scatter.point_radius_px;
        let line_buf = data_render::create_style_uniform_buffer(dev, &line);
        let area_buf = area.map(|area| data_render::create_style_uniform_buffer(dev, &area));
        let sc_buf = data_render::create_style_uniform_buffer(dev, &scatter);
        let eb_buf = data_render::create_style_uniform_buffer(dev, &errorbar);
        let bar_buf = data_render::create_style_uniform_buffer(dev, &bar);
        let tally = crate::gpu_memory::ChargeTally::new();
        tally.add(style_bytes);
        ChartStyle {
            _charge: crate::gpu_memory::shared_charge(
                tally,
                &self.gpu_ledger,
                GpuResourceKind::Uniform,
            ),
            line_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &line_buf),
            area_bg: area_buf
                .as_ref()
                .map(|buf| data_render::create_style_bind_group(dev, &self.style_bgl, buf)),
            scatter_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &sc_buf),
            errorbar_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &eb_buf),
            bar_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &bar_buf),
            scatter_map,
            errorbar_map,
            bar_map,
            display_scale,
            scatter_radius_px,
        }
    }

    pub(super) fn create_scatter_style_map_for(
        &self,
        scatter: &DataScatterStyleConfig,
        scale: f32,
    ) -> Option<data_render::ScatterStyleMap> {
        use data_render::{STYLE_MASK_COLOR, STYLE_MASK_RADIUS, STYLE_MASK_SHAPE};

        let has_index = scatter.point_style_index_column.is_some();
        let style_slots: Vec<data_render::ScatterStyleSlotGpu> = scatter
            .point_style_table
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|slot| {
                scatter_style_slot_gpu(
                    slot,
                    scale,
                    STYLE_MASK_COLOR,
                    STYLE_MASK_RADIUS,
                    STYLE_MASK_SHAPE,
                )
            })
            .collect();
        let overrides: Vec<data_render::ScatterStyleOverrideGpu> = scatter
            .point_style_overrides
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter_map(|ov| {
                let point_index = u32::try_from(ov.index).ok()?;
                let slot = scatter_style_slot_gpu(
                    &ov.style,
                    scale,
                    STYLE_MASK_COLOR,
                    STYLE_MASK_RADIUS,
                    STYLE_MASK_SHAPE,
                );
                Some(data_render::ScatterStyleOverrideGpu {
                    point_index,
                    _pad: [0; 3],
                    color_premul: slot.color_premul,
                    meta: slot.meta,
                })
            })
            .collect();

        // A continuous value map resolves in the mapped entries too: it needs
        // the per-point path even when no table or override exists.
        let has_value_map = scatter.point_color_map.is_some() || scatter.point_size_map.is_some();
        if !has_index && style_slots.is_empty() && overrides.is_empty() && !has_value_map {
            return None;
        }
        Some(data_render::create_scatter_style_map(
            &self.device,
            &self.per_point_style_map_bgl,
            &style_slots,
            &overrides,
            data_render::ScatterStyleMapMeta {
                style_count: style_slots.len().min(u32::MAX as usize) as u32,
                override_count: overrides.len().min(u32::MAX as usize) as u32,
                has_index: u32::from(has_index),
                _pad: 0,
            },
        ))
    }

    pub(super) fn create_errorbar_style_map_for(
        &self,
        errorbar: &DataErrorBarStyleConfig,
        scale: f32,
    ) -> Option<data_render::ErrorBarStyleMap> {
        const MASK_COLOR: u32 = 1;
        const MASK_STEM_WIDTH: u32 = 2;
        const MASK_CAP_HALF: u32 = 4;
        const MASK_CAP_WIDTH: u32 = 8;

        let has_index = errorbar.error_bar_style_index_column.is_some();
        let style_slots: Vec<data_render::ErrorBarStyleSlotGpu> = errorbar
            .error_bar_style_table
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|slot| {
                errorbar_style_slot_gpu(
                    slot,
                    scale,
                    MASK_COLOR,
                    MASK_STEM_WIDTH,
                    MASK_CAP_HALF,
                    MASK_CAP_WIDTH,
                )
            })
            .collect();
        let overrides: Vec<data_render::ErrorBarStyleOverrideGpu> = errorbar
            .error_bar_style_overrides
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter_map(|ov| {
                let point_index = u32::try_from(ov.index).ok()?;
                let slot = errorbar_style_slot_gpu(
                    &ov.style,
                    scale,
                    MASK_COLOR,
                    MASK_STEM_WIDTH,
                    MASK_CAP_HALF,
                    MASK_CAP_WIDTH,
                );
                Some(data_render::ErrorBarStyleOverrideGpu {
                    point_index,
                    _pad: [0; 3],
                    color_premul: slot.color_premul,
                    meta: slot.meta,
                })
            })
            .collect();

        if !has_index && style_slots.is_empty() && overrides.is_empty() {
            return None;
        }
        Some(data_render::create_errorbar_style_map(
            &self.device,
            &self.per_point_style_map_bgl,
            &style_slots,
            &overrides,
            data_render::ErrorBarStyleMapMeta {
                style_count: style_slots.len().min(u32::MAX as usize) as u32,
                override_count: overrides.len().min(u32::MAX as usize) as u32,
                has_index: u32::from(has_index),
                _pad: 0,
            },
        ))
    }

    pub(super) fn create_bar_style_map_for(
        &self,
        bar: &DataBarStyleConfig,
        scale: f32,
    ) -> Option<data_render::BarStyleMap> {
        let overrides: Vec<data_render::BarStyleOverrideGpu> = bar
            .bar_style_overrides
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter_map(|style_override| {
                let bin_index = u32::try_from(style_override.index).ok()?;
                let slot = bar_style_slot_gpu(&style_override.style, scale);
                Some(data_render::BarStyleOverrideGpu {
                    bin_index,
                    _pad: [0; 3],
                    fill_color_premul: slot.fill_color_premul,
                    border_color_premul: slot.border_color_premul,
                    params: slot.params,
                    material: slot.material,
                    finish: slot.finish,
                })
            })
            .collect();
        if overrides.is_empty() {
            return None;
        }
        Some(data_render::create_bar_style_map(
            &self.device,
            &self.per_point_style_map_bgl,
            &overrides,
        ))
    }
}

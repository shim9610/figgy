//! Contour series: the level lookup, the labelled contour pipelines and the
//! contour layers.
//!
//! The contour label atlas: every level's label text, formatted the way a
//! tick label is, baked once per signature into one texture the GPU places.

use super::*;
use crate::axis_render::effective_major_spacing;
use crate::config::AxisScale;
use crate::decoration_render::format_tick_value;
use crate::raster::{Canvas, Paint};
use crate::text_render::{FontPolicy, draw_plain_text, measure_plain_text};

/// Contour lookup granularity. Each block fits in one shader `u32` candidate
/// mask, so the original declaration order can be restored without a second
/// per-fragment table or an O(level_count) scan.
pub(crate) const CONTOUR_LEVEL_BLOCK_SIZE: usize = 32;

pub(crate) const CONTOUR_LEVEL_BLOCK_COUNT: usize = 32;

pub(crate) const CONTOUR_LEVEL_LOOKUP_CAPACITY: usize =
    CONTOUR_LEVEL_BLOCK_SIZE * CONTOUR_LEVEL_BLOCK_COUNT;
const _: () = assert!(crate::data_config::MAX_CONTOUR_LEVELS == CONTOUR_LEVEL_LOOKUP_CAPACITY);

/// Per-32-level lookup bounds uploaded at field group-2 binding 6.
///
/// GPU twin: `field_columnar.wgsl::ContourLookupMetadata`, 8 B. Counts are
/// derived from the actual f32 search keys, after the declared f64 levels have
/// crossed the renderer upload boundary.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ContourLookupMetadataGpu {
    pub finite_count: u32,
    pub negative_infinity_count: u32,
}

const _: () = assert!(std::mem::size_of::<ContourLookupMetadataGpu>() == 8);

/// The selected-field highlight for a labelled contour: group 3 is the
/// series' label-gap group, which `fs_data_selection_labelled` reads.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_field_selection_labelled_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    selection_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    label_gap_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_optional_extra_layout(
        device,
        shader,
        transform_bgl,
        selection_bgl,
        field_bgl,
        Some(label_gap_bgl),
        target_format,
        sample_count,
        "fs_data_selection_labelled",
        "figgy selected labelled field data pipeline",
    )
}

/// Labelled contours add group 3 containing the exact selected-anchor buffers.
/// No-label contours keep the original layout and entry point, so merely adding
/// contour support does not compile or bind label machinery.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_labelled_contour_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    label_gap_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_optional_extra_layout(
        device,
        shader,
        transform_bgl,
        style_bgl,
        field_bgl,
        Some(label_gap_bgl),
        target_format,
        sample_count,
        "fs_contour_labelled",
        "figgy labelled field contour pipeline",
    )
}

/// One contour series' draw.
///
/// The **same quad as the field**, through `fs_contour`: the isolines are the
/// level set of the same bilinear interpolation, so there is no segment list, no
/// indirect draw and no per-level draw call. Per-level colour comes from the
/// group-2 `level_colors` table. Stroke coverage comes from the quadratic root
/// of the current cell's bilinear polynomial along the fragment's gradient
/// normal; it is not a claim of globally shortest Euclidean distance.
pub struct ColumnContourLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    /// group(1): the fallback colour for an unplaceable z. Unread by
    /// `fs_contour`, but the layout is shared with the fill.
    pub style_bg: wgpu::BindGroup,
    /// group(2): pool, grid table, levels, stops, params, level colours.
    pub field_bg: wgpu::BindGroup,
    /// Keeps the group-2 lump charge alive for as long as this layer can draw.
    pub charge: std::sync::Arc<crate::gpu_memory::GpuByteCharge>,
    /// False when the grid has no cell to interpolate across.
    pub drawable: bool,
    /// `None` when the series draws no labels.
    pub label: Option<ColumnContourLabel<'a>>,
}

/// The inline level labels for one contour series.
///
/// One quad per anchor, textured from the CPU-baked atlas. The serial
/// `anchor_select` pass writes the indirect instance count; the CPU neither
/// learns it nor needs to. A host override writes the same records and argument
/// quad, so there is exactly one draw path.
pub struct ColumnContourLabel<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    /// Immutable atlas plus this occurrence's explicit or exact-key automatic
    /// placement resources and their ledger charges.
    pub(crate) snapshot: crate::gpu_contour::ContourLabelSnapshot,
}

// Contour labels.
//
// The *placement* of an inline level label is a GPU fact — the anchor pass
// projects seed points onto the same implicit field the line uses — but the
// label's *content* is `levels[i]`, which is config. The CPU therefore bakes the
// strings once per signature and the GPU only places the resulting images.
//
// Baking whole strings rather than glyphs is what keeps this in the existing text
// stack: `measure_plain_text` / `draw_plain_text` / `FontPolicy` handle
// per-character fallback and the sketch face exactly as they do for a tick label.
// There is no second text stack and no GPU glyph rasterizer.
//
// The label is drawn *over* the line. matplotlib erases the stroke under an
// inline label; here `bg_color` paints behind the text instead, which needs no
// change to the contour draw at all — and unlike the chart background, the
// label's own background is something the label knows (design B.4.7).

/// One level's cell in the baked label atlas.
pub(crate) struct LabelAtlasCell {
    /// Content width in texels. The quad is drawn this wide and samples only
    /// this far into the cell, so a short string is not stretched.
    pub width: f32,
}

/// A baked contour-label atlas: one uniformly-strided grid cell per level.
pub(crate) struct BakedLabelAtlas {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Drawn label height inside each cell, excluding the transparent gutter.
    pub cell_h: f32,
    pub cell_stride_w: u32,
    pub cell_stride_h: u32,
    pub columns: u32,
    pub rows: u32,
    pub gutter: u32,
    pub cells: Vec<LabelAtlasCell>,
}

/// Why a requested contour-label atlas could not be baked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContourLabelAtlasError {
    /// The uniform cell grid cannot fit in the device's 2D texture limit.
    DeviceLimit {
        level_count: u64,
        capacity: u64,
        cell_stride_w: u32,
        cell_stride_h: u32,
        max_dimension: u32,
    },
    /// The dimensions passed the device limit, but the CPU raster allocation
    /// failed before upload.
    AllocationFailed { width: u32, height: u32 },
}

impl std::fmt::Display for ContourLabelAtlasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceLimit {
                level_count,
                capacity,
                cell_stride_w,
                cell_stride_h,
                max_dimension,
            } => write!(
                f,
                "{level_count} contour label cells of stride {cell_stride_w}x{cell_stride_h} \
                 exceed a {max_dimension}x{max_dimension} texture (capacity {capacity})"
            ),
            Self::AllocationFailed { width, height } => {
                write!(
                    f,
                    "could not allocate a {width}x{height} contour label atlas"
                )
            }
        }
    }
}

impl std::error::Error for ContourLabelAtlasError {}

pub(super) const CONTOUR_LABEL_ATLAS_GUTTER: u32 = 1;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct LabelAtlasGrid {
    pub(super) columns: u32,
    pub(super) rows: u32,
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(crate) fn contour_label_atlas_grid(
    level_count: usize,
    cell_stride_w: u32,
    cell_stride_h: u32,
    max_dimension: u32,
) -> Result<LabelAtlasGrid, ContourLabelAtlasError> {
    let levels = level_count as u64;
    let max_columns = max_dimension.checked_div(cell_stride_w).unwrap_or(0);
    let max_rows = max_dimension.checked_div(cell_stride_h).unwrap_or(0);
    let capacity = u64::from(max_columns) * u64::from(max_rows);
    if levels == 0 || levels > capacity {
        return Err(ContourLabelAtlasError::DeviceLimit {
            level_count: levels,
            capacity,
            cell_stride_w,
            cell_stride_h,
            max_dimension,
        });
    }

    // Choose the valid grid whose larger texel dimension is smallest. The
    // search is bounded by the device dimension (normally <= 16384), not by the
    // number of labels, and runs only when the atlas signature changes.
    let mut best: Option<(u32, u64, u32, u32)> = None;
    let candidate_columns = u64::from(max_columns).min(levels) as u32;
    for columns in 1..=candidate_columns {
        let rows = levels.div_ceil(u64::from(columns));
        if rows > u64::from(max_rows) {
            continue;
        }
        let rows = rows as u32;
        let width = columns * cell_stride_w;
        let height = rows * cell_stride_h;
        let score = width.max(height);
        let area = u64::from(width) * u64::from(height);
        if best
            .as_ref()
            .is_none_or(|(best_score, best_area, _, _)| (score, area) < (*best_score, *best_area))
        {
            best = Some((score, area, columns, rows));
        }
    }
    let (_, _, columns, rows) = best.expect("capacity check guarantees a valid label grid");
    Ok(LabelAtlasGrid {
        columns,
        rows,
        width: columns * cell_stride_w,
        height: rows * cell_stride_h,
    })
}

/// The text of every level's label, formatted the way a tick label is.
///
/// Separated from the bake so the signature can be compared against the strings
/// themselves: two configs that format to the same numbers need no new atlas.
pub(crate) fn contour_label_texts(
    config: &Config,
    labels: &crate::data_config::ContourLabelConfig,
    levels: &[f64],
) -> Vec<String> {
    let mut finite_levels: Vec<f64> = levels
        .iter()
        .copied()
        .filter(|level| level.is_finite())
        .collect();
    finite_levels.sort_by(f64::total_cmp);
    let level_spacing = finite_levels
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|spacing| spacing.is_finite() && *spacing > 0.0)
        .min_by(f64::total_cmp);
    let colorbar_axis = config.colorbar.as_ref().map(|bar| &bar.axis);
    let spacing = level_spacing
        .or_else(|| colorbar_axis.map(effective_major_spacing))
        .unwrap_or_else(|| {
            finite_levels
                .first()
                .copied()
                .unwrap_or(1.0)
                .abs()
                .max(f64::MIN_POSITIVE)
        });
    let range_min = finite_levels
        .first()
        .copied()
        .or_else(|| colorbar_axis.map(|axis| axis.min))
        .unwrap_or(0.0);
    let range_max = finite_levels
        .last()
        .copied()
        .or_else(|| colorbar_axis.map(|axis| axis.max))
        .unwrap_or(range_min);
    levels
        .iter()
        .map(|level| {
            format_tick_value(
                *level,
                &labels.format,
                labels.significant_digits,
                &AxisScale::Linear,
                spacing,
                range_min,
                range_max,
            )
        })
        .collect()
}

/// Everything an atlas is baked from, in physical pixels.
///
/// [`bake_contour_label_atlas`] takes nothing else, so a cache that compares
/// two of these compares everything the bake can read: a field the bake
/// needs has to be here, or the bake does not compile. Floats are held as
/// bits so two inputs that differ only in a NaN payload still compare equal
/// and an atlas is not re-baked every frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContourLabelAtlasInput {
    /// The final formatted strings: the atlas' exact content authority.
    pub texts: Vec<String>,
    /// `font_size * scale`: an export at 2x re-bakes rather than magnifying
    /// a window-resolution atlas.
    pub font_size_bits: u32,
    pub color_bits: [u32; 4],
    pub policy: FontPolicy,
    pub family: String,
    pub bg_color_bits: Option<[u32; 4]>,
    /// `bg_padding_px * scale`, clamped at zero. Padding also defines the
    /// transparent line break around a label, so it is geometric even when
    /// no background colour is painted.
    pub bg_padding_bits: u32,
}

impl ContourLabelAtlasInput {
    /// What `labels` bake to under `config` at `scale`, or `None` when there
    /// is nothing to bake: hidden labels, a non-positive font size or no
    /// strings.
    pub(crate) fn from_config(
        config: &Config,
        labels: &crate::data_config::ContourLabelConfig,
        texts: Vec<String>,
        scale: f32,
    ) -> Option<Self> {
        if !labels.visible || labels.font_size <= 0.0 || texts.is_empty() {
            return None;
        }
        let bits = |c: &Color| [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()];
        Some(Self {
            texts,
            font_size_bits: (labels.font_size * scale).to_bits(),
            color_bits: bits(&labels.color),
            policy: FontPolicy::for_style(&config.draw_style),
            // Contour labels are data annotations, so they take the chart's
            // tick-label face; `FontPolicy` still swaps it in sketch mode.
            family: config.bottom_x.label_style.label_font.clone(),
            bg_color_bits: labels.bg_color.as_ref().map(bits),
            bg_padding_bits: (labels.bg_padding_px.max(0.0) * scale).to_bits(),
        })
    }
}

fn color_from_bits([r, g, b, a]: [u32; 4]) -> Color {
    Color::new(
        f32::from_bits(r),
        f32::from_bits(g),
        f32::from_bits(b),
        f32::from_bits(a),
    )
}

/// Bake one contour series' labels into an atlas.
///
/// Returns `Ok(None)` when the strings measure to nothing. Device-limit and
/// allocation failures are explicit so the renderer can fail the frame with
/// context rather than silently drawing an unlabelled series.
pub(crate) fn bake_contour_label_atlas(
    input: &ContourLabelAtlasInput,
    max_dimension: u32,
) -> Result<Option<BakedLabelAtlas>, ContourLabelAtlasError> {
    let ContourLabelAtlasInput {
        texts,
        font_size_bits,
        color_bits,
        policy: fp,
        family,
        bg_color_bits,
        bg_padding_bits,
    } = input;
    let fp = *fp;
    let font_size = f32::from_bits(*font_size_bits);
    let pad = f32::from_bits(*bg_padding_bits);
    let color = color_from_bits(*color_bits);
    let bg_color = bg_color_bits.map(color_from_bits);

    // One measure pass first: the cell height is uniform across levels (the
    // tallest), so it cannot be known before every string has been measured.
    let measured: Vec<_> = texts
        .iter()
        .map(|text| measure_plain_text(text, family, font_size, false, false, fp))
        .collect();
    let mut cell_w = 0.0f32;
    let mut ascent = 0.0f32;
    let mut descent = 0.0f32;
    for m in &measured {
        cell_w = cell_w.max(m.width);
        ascent = ascent.max(m.ascent);
        descent = descent.max(m.descent);
    }
    if cell_w <= 0.0 {
        return Ok(None);
    }
    let cell_h = (ascent + descent + 2.0 * pad).ceil().max(1.0);
    let content_w = (cell_w + 2.0 * pad).ceil().max(1.0) as u32;
    let content_h = cell_h as u32;
    let gutter = CONTOUR_LABEL_ATLAS_GUTTER;
    let cell_stride_w = content_w.saturating_add(2 * gutter);
    let cell_stride_h = content_h.saturating_add(2 * gutter);
    let grid = contour_label_atlas_grid(texts.len(), cell_stride_w, cell_stride_h, max_dimension)?;
    let mut canvas =
        Canvas::new(grid.width, grid.height).ok_or(ContourLabelAtlasError::AllocationFailed {
            width: grid.width,
            height: grid.height,
        })?;

    let mut cells = Vec::with_capacity(texts.len());
    for (index, (text, m)) in texts.iter().zip(measured.iter()).enumerate() {
        let content = m.width + 2.0 * pad;
        cells.push(LabelAtlasCell { width: content });
        if text.is_empty() {
            continue;
        }
        let column = index as u32 % grid.columns;
        let row = index as u32 / grid.columns;
        let left = (column * cell_stride_w + gutter) as f32;
        let top = (row * cell_stride_h + gutter) as f32;
        if let Some(bg) = bg_color.as_ref() {
            canvas.draw_rect(left, top, content, cell_h, &Paint::fill(bg));
        }
        draw_plain_text(
            &mut canvas,
            text,
            (left + pad, top + pad + ascent),
            color,
            family,
            font_size,
            false,
            false,
            fp,
        );
    }

    Ok(Some(BakedLabelAtlas {
        rgba: canvas.into_rgba(),
        width: grid.width,
        height: grid.height,
        cell_h,
        cell_stride_w,
        cell_stride_h,
        columns: grid.columns,
        rows: grid.rows,
        gutter,
        cells,
    }))
}

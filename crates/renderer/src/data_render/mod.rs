//! GPU data rendering via wgpu.
//!
//! wgpu owns the window surface; the CPU raster stack (tiny-skia + swash)
//! rasterises axes / titles / labels and uploads them as a texture. A single
//! render pass draws data primitives followed by the chrome texture overlay,
//! then presents.

use crate::color::Color;
use crate::config::Config;
use crate::data_config::ScatterShape;
use crate::layout::Rect;

use wgpu::util::DeviceExt;

mod surface;
pub use surface::*;
mod textured;
pub use textured::*;
mod transform;
pub use transform::*;
mod style;
pub use style::*;
mod value_map;
pub use value_map::*;
mod selection;
pub(crate) use selection::*;
mod line;
pub use line::*;
mod stylized;
pub use stylized::*;
mod scatter;
pub use scatter::*;
mod errorbar;
pub use errorbar::*;
mod bar;
pub use bar::*;
mod field;
pub use field::*;
mod contour;
pub use contour::*;
pub mod bar_envelope;
pub mod column_pool;
pub mod line_arc;
pub(crate) mod stream_field;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod stream_point_style_tests;
pub mod vector;
pub use column_pool::{
    AllocError, ColumnHandle, ColumnId, ColumnPool, ColumnSlot, DefragPolicy, FreeRegion,
    GpuAllocCtx, GpuBudget, GrowthPolicy,
};

// Instance, adapter, surface, and device setup.

// Texture upload, samplers, bind groups, and pipelines for the chrome overlay.

// Transform uniform (data-space -> NDC), shared by scatter / line / errorbar.
// Vertex buffers carry data-space values directly; resize/zoom only updates
// this uniform, never the instance data.

// WGSL mirror size guards. Field order and size must remain byte-identical to
// every shader common block before either CPU structure changes.
const _: () = assert!(std::mem::size_of::<ScatterTransform>() == 144);

// Primitive style uniform — color and per-primitive options.
// Bind groups: group(0) = transform (shared), group(1) = style (per primitive).

// Columnar pipelines backed by ColumnPool.
//
// Every column is uploaded once into a ColumnPool buffer; charts draw by
// binding `pool.buffer().slice(handle.byte_range())` into vertex slots.
// Shaders take a single column per slot; lengths and offsets are decided by
// the caller via the slice and draw range.

// Procedural planet-atlas bakes run once per style-set creation; results are
// cached on the GPU and never recomputed per frame.

/// One series' data primitives. A panel can hold multiple of these.
pub struct SeriesLayers<'a> {
    /// A line's area fill. Not drawn by `issue_series_data`: every series'
    /// fill is issued first by `issue_series_fill`, in series order, before
    /// any series body — see `draw_chart_panel_columnar`.
    pub area: Option<ColumnAreaLayer<'a>>,
    /// The field covers the whole grid, so it goes under everything else —
    /// see `issue_series_data` for the full order.
    pub field: Option<ColumnFieldLayer<'a>>,
    /// Bars are area primitives, so they are issued before the stroked ones.
    pub bar: Option<ColumnBarLayer<'a>>,
    /// Contour lines go over the field they describe and under the markers.
    pub contour: Option<ColumnContourLayer<'a>>,
    /// Arrows go over the field and contours they annotate, under the series'
    /// own stroked marks.
    pub vector: Option<vector::ColumnVectorLayer<'a>>,
    pub errorbar: Option<ColumnErrorBarDraw<'a>>,
    pub line: Option<ColumnLineLayer<'a>>,
    /// Constellation star pass over the same polyline as `line` — drawn
    /// right after it, before `scatter`. `None` everywhere else.
    pub line_extra: Option<ColumnStarLayer<'a>>,
    pub scatter: Option<ColumnScatterLayer<'a>>,
    pub selected_bars: Vec<ColumnBarSelectionLayer<'a>>,
    pub selected_fields: Vec<ColumnFieldSelectionLayer<'a>>,
    pub picked: Vec<ColumnPickRingLayer<'a>>,
}

/// Clamp a rect into `(0..target.0, 0..target.1)`. Returns `None` if the
/// clamped width or height is zero so callers can skip the draw entirely.
pub(crate) fn clamp_rect_to_target(r: Rect, target: (u32, u32)) -> Option<Rect> {
    let (tw, th) = target;
    let x0 = r.x.min(tw);
    let y0 = r.y.min(th);
    let x1 = r.x.saturating_add(r.width).min(tw);
    let y1 = r.y.saturating_add(r.height).min(th);
    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);
    if w == 0 || h == 0 {
        None
    } else {
        Some(Rect {
            x: x0,
            y: y0,
            width: w,
            height: h,
        })
    }
}

/// Issue one series' area fill. Called for every series before any series
/// body (`draw_chart_panel_columnar`): the data pass is "every fill → every
/// body → every picked highlight", in SSOT series order with no sorting, so
/// a fill composites over earlier fills and under every line.
pub(crate) fn issue_series_fill(pass: &mut wgpu::RenderPass<'_>, series: &SeriesLayers<'_>) {
    let Some(a) = series.area.as_ref() else {
        return;
    };
    let count = a.x.len_values.min(a.y.len_values).min(a.base.len_values) as u32;
    if count < 2 {
        return;
    }
    pass.set_pipeline(a.pipeline);
    pass.set_bind_group(0, a.transform_bg, &[]);
    pass.set_bind_group(1, a.style_bg, &[]);
    let x_full = a.x.byte_range();
    let y_full = a.y.byte_range();
    let base_full = a.base.byte_range();
    let shift = crate::data::COLUMN_VALUE_BYTES as u64;
    pass.set_vertex_buffer(0, a.pool_buffer.slice(x_full.clone()));
    pass.set_vertex_buffer(1, a.pool_buffer.slice(y_full.clone()));
    pass.set_vertex_buffer(2, a.pool_buffer.slice((x_full.start + shift)..x_full.end));
    pass.set_vertex_buffer(3, a.pool_buffer.slice((y_full.start + shift)..y_full.end));
    // The second y column, one logical value apart like x/y, so instance i
    // sees base[i] and base[i + 1].
    pass.set_vertex_buffer(4, a.pool_buffer.slice(base_full.clone()));
    pass.set_vertex_buffer(
        5,
        a.pool_buffer
            .slice((base_full.start + shift)..base_full.end),
    );
    pass.draw(0..a.verts_per_instance, 0..(count - 1));
}

/// Issue draw calls for one series' data primitives. The caller must have
/// already set the viewport (panel) and scissor (data_area).
pub(crate) fn issue_series_data(pass: &mut wgpu::RenderPass<'_>, series: &SeriesLayers<'_>) {
    // The field is the backdrop: it paints every cell of the grid, so bars,
    // contour lines and markers all have to land on top of it.
    if let Some(f) = series.field.as_ref().filter(|f| f.drawable) {
        pass.set_pipeline(f.pipeline);
        pass.set_bind_group(0, f.transform_bg, &[]);
        pass.set_bind_group(1, &f.style_bg, &[]);
        pass.set_bind_group(2, &f.field_bg, &[]);
        pass.draw(0..FIELD_VERTICES, 0..1);
    }

    // Bars next: they are filled areas, and the stroked primitives below must
    // land on top of them rather than under.
    if let Some(b) = series.bar.as_ref() {
        let count = bar_instance_count(b.edges.len_values, b.values.len_values);
        if count > 0 {
            pass.set_pipeline(b.pipeline);
            pass.set_bind_group(0, b.transform_bg, &[]);
            pass.set_bind_group(1, b.style_bg, &[]);
            if let Some(map) = b.style_map_bg {
                pass.set_bind_group(2, map, &[]);
            }
            let edges = b.edges.byte_range();
            // The same column one logical value along — instance i then sees
            // `edges[i]` and `edges[i + 1]`.
            let edges_next = (edges.start + crate::data::COLUMN_VALUE_BYTES as u64)..edges.end;
            pass.set_vertex_buffer(0, b.pool_buffer.slice(edges.clone()));
            pass.set_vertex_buffer(1, b.pool_buffer.slice(edges_next));
            pass.set_vertex_buffer(2, b.pool_buffer.slice(b.values.byte_range()));
            pass.draw(0..BAR_VERTICES_PER_INSTANCE, 0..count);
            if let Some(envelope) = &b.envelope {
                envelope.draw(pass, b.transform_bg, b.style_bg);
            }
        }
    }

    // Contour lines: over the field they describe, under everything stroked for
    // the data itself. One draw of the same quad the fill uses — `fs_contour`
    // finds every level in one fragment pass, so the cost is the data area's
    // pixels rather than the grid's cells.
    if let Some(c) = series.contour.as_ref().filter(|c| c.drawable) {
        pass.set_pipeline(c.pipeline);
        pass.set_bind_group(0, c.transform_bg, &[]);
        pass.set_bind_group(1, &c.style_bg, &[]);
        pass.set_bind_group(2, &c.field_bg, &[]);
        if let Some(label) = c.label.as_ref() {
            pass.set_bind_group(3, label.snapshot.label_gap_bind_group(), &[]);
        }
        pass.draw(0..FIELD_VERTICES, 0..1);
        // Labels over the lines they name, still inside the data pass — so the
        // `data_area` scissor trims them and the decoration layer (axes, legend,
        // colourbar) composites on top, which is the order a legend box covering
        // a label needs.
        if let Some(l) = c.label.as_ref() {
            pass.set_pipeline(l.pipeline);
            pass.set_bind_group(0, c.transform_bg, &[]);
            pass.set_bind_group(1, l.snapshot.bind_group(), &[]);
            pass.set_vertex_buffer(0, l.snapshot.anchors().slice(..));
            pass.draw_indirect(l.snapshot.indirect(), 0);
        }
    }

    // Arrows: over the field and contours, under errorbars, lines and markers.
    // No vertex buffers; `vs_main` reads sample `instance_index` from group 2.
    if let Some(v) = series.vector.as_ref().filter(|v| v.count > 0) {
        pass.set_pipeline(v.pipeline);
        pass.set_bind_group(0, v.transform_bg, &[]);
        pass.set_bind_group(1, &v.style_bg, &[]);
        pass.set_bind_group(2, &v.vector_bg, &[]);
        pass.draw(0..vector::ARROW_VERTICES, 0..v.count);
    }

    if let Some(eb) = series.errorbar.as_ref() {
        let count = [
            eb.x.len_values,
            eb.y.len_values,
            eb.err_y_lo.len_values,
            eb.err_y_hi.len_values,
            eb.err_x_lo.len_values,
            eb.err_x_hi.len_values,
        ]
        .into_iter()
        .min()
        .unwrap_or(0) as u32;
        if count > 0 {
            pass.set_pipeline(eb.pipeline);
            pass.set_bind_group(0, eb.transform_bg, &[]);
            pass.set_bind_group(1, eb.style_bg, &[]);
            if let Some(map) = eb.style_map_bg {
                pass.set_bind_group(2, map, &[]);
            }
            pass.set_vertex_buffer(0, eb.pool_buffer.slice(eb.x.byte_range()));
            pass.set_vertex_buffer(1, eb.pool_buffer.slice(eb.y.byte_range()));
            pass.set_vertex_buffer(2, eb.pool_buffer.slice(eb.err_y_lo.byte_range()));
            pass.set_vertex_buffer(3, eb.pool_buffer.slice(eb.err_y_hi.byte_range()));
            pass.set_vertex_buffer(4, eb.pool_buffer.slice(eb.err_x_lo.byte_range()));
            pass.set_vertex_buffer(5, eb.pool_buffer.slice(eb.err_x_hi.byte_range()));
            if eb.style_map_bg.is_some() {
                let style_index = eb.style_index.as_ref().unwrap_or(&eb.x);
                pass.set_vertex_buffer(6, eb.pool_buffer.slice(style_index.byte_range()));
            }
            // 36 vertices = 6 quads × 2 triangles (see errorbar_columnar.wgsl).
            pass.draw(0..36, 0..count);
        }
    }

    if let Some(l) = series.line.as_ref() {
        draw_line_layer(pass, l);
    }
    if let Some(s) = series.line_extra.as_ref() {
        // Constellation star pass: indirect draw — the instance count was
        // computed on the GPU from the polyline's total arc (line_arc.wgsl
        // star_indirect), so the CPU never sees, nor caps, the star count.
        pass.set_pipeline(s.pipeline);
        pass.set_bind_group(0, s.transform_bg, &[]);
        pass.set_bind_group(1, s.style_bg, &[]);
        pass.set_bind_group(2, s.texture_bg, &[]);
        pass.set_bind_group(3, s.star_bg, &[]);
        pass.draw_indirect(s.indirect, 0);
    }

    if let Some(s) = series.scatter.as_ref() {
        let count = s.x.len_values.min(s.y.len_values) as u32;
        if count > 0 {
            pass.set_pipeline(s.pipeline);
            pass.set_bind_group(0, s.transform_bg, &[]);
            pass.set_bind_group(1, s.style_bg, &[]);
            if let Some(map) = s.style_map_bg {
                pass.set_bind_group(2, map, &[]);
                pass.set_bind_group(
                    3,
                    s.value_map_bg
                        .expect("mapped scatter draw binds its value map group"),
                    &[],
                );
            } else if let Some(tex) = s.texture_bg {
                pass.set_bind_group(2, tex, &[]);
            }
            pass.set_vertex_buffer(0, s.quad_vb.slice(..));
            pass.set_vertex_buffer(1, s.pool_buffer.slice(s.x.byte_range()));
            pass.set_vertex_buffer(2, s.pool_buffer.slice(s.y.byte_range()));
            if s.style_map_bg.is_some() {
                let style_index = s.style_index.as_ref().unwrap_or(&s.x);
                pass.set_vertex_buffer(3, s.pool_buffer.slice(style_index.byte_range()));
                let color_value = s.color_value.as_ref().unwrap_or(&s.x);
                pass.set_vertex_buffer(4, s.pool_buffer.slice(color_value.byte_range()));
                let size_value = s.size_value.as_ref().unwrap_or(&s.x);
                pass.set_vertex_buffer(5, s.pool_buffer.slice(size_value.byte_range()));
            }
            pass.draw(0..4, 0..count);
        }
    }
}

/// Draw one chart panel: grid → every series → decoration. The function
/// configures viewport and scissor itself, so callers only supply rects.
///
/// `target_size` is the pixel size of the current color attachment;
/// `panel_rect` / `data_area` are clamped to it to avoid wgpu validation
/// errors when a panel partially exits the surface.
pub(crate) fn issue_series_picked(pass: &mut wgpu::RenderPass<'_>, series: &SeriesLayers<'_>) {
    for selection in &series.selected_bars {
        pass.set_pipeline(selection.pipeline);
        pass.set_bind_group(0, selection.transform_bg, &[]);
        pass.set_bind_group(1, &selection.style_bg, &[]);
        pass.set_bind_group(2, &selection.selection_bg, &[]);
        let edges = selection.edges.byte_range();
        let edges_next = (edges.start + crate::data::COLUMN_VALUE_BYTES as u64)..edges.end;
        pass.set_vertex_buffer(0, selection.pool_buffer.slice(edges.clone()));
        pass.set_vertex_buffer(1, selection.pool_buffer.slice(edges_next));
        pass.set_vertex_buffer(
            2,
            selection.pool_buffer.slice(selection.values.byte_range()),
        );
        pass.draw(
            0..BAR_SELECTION_VERTICES,
            selection.instance..selection.instance + 1,
        );
    }

    for selection in series.selected_fields.iter().filter(|layer| layer.drawable) {
        pass.set_pipeline(selection.pipeline);
        pass.set_bind_group(0, selection.transform_bg, &[]);
        pass.set_bind_group(1, &selection.selection_bg, &[]);
        pass.set_bind_group(2, &selection.field_bg, &[]);
        // A labelled contour's highlight pipeline reads the label gaps the
        // contour draw leaves; the plain one ignores the extra group.
        if let Some(label) = series.contour.as_ref().and_then(|c| c.label.as_ref()) {
            pass.set_bind_group(3, label.snapshot.label_gap_bind_group(), &[]);
        }
        pass.draw(0..FIELD_VERTICES, 0..1);
    }

    for p in &series.picked {
        pass.set_pipeline(p.pipeline);
        pass.set_bind_group(0, p.transform_bg, &[]);
        pass.set_bind_group(1, &p.style_bg, &[]);
        if let Some(map) = p.style_map_bg {
            pass.set_bind_group(2, map, &[]);
            pass.set_bind_group(
                3,
                p.value_map_bg
                    .expect("mapped pick ring draw binds its value map group"),
                &[],
            );
        }
        pass.set_vertex_buffer(0, p.quad_vb.slice(..));
        pass.set_vertex_buffer(1, p.pool_buffer.slice(p.x.byte_range()));
        pass.set_vertex_buffer(2, p.pool_buffer.slice(p.y.byte_range()));
        if p.style_map_bg.is_some() {
            let style_index = p.style_index.as_ref().unwrap_or(&p.x);
            pass.set_vertex_buffer(3, p.pool_buffer.slice(style_index.byte_range()));
            let color_value = p.color_value.as_ref().unwrap_or(&p.x);
            pass.set_vertex_buffer(4, p.pool_buffer.slice(color_value.byte_range()));
            let size_value = p.size_value.as_ref().unwrap_or(&p.x);
            pass.set_vertex_buffer(5, p.pool_buffer.slice(size_value.byte_range()));
        }
        pass.draw(0..4, p.instance..p.instance + 1);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_chart_panel_columnar(
    pass: &mut wgpu::RenderPass<'_>,
    target_size: (u32, u32),
    panel_rect: Rect,
    data_area: Rect,
    grid: AxisLayer<'_>,
    annotation_underlay: Option<AxisLayer<'_>>,
    series_list: &[SeriesLayers<'_>],
    decoration: AxisLayer<'_>,
    annotation_overlay: Option<AxisLayer<'_>>,
) {
    draw_chart_panel_with_bodies(
        pass,
        target_size,
        panel_rect,
        data_area,
        grid,
        annotation_underlay,
        series_list,
        None,
        decoration,
        annotation_overlay,
    );
}

/// Every series' fill, then every body, each loop in SSOT series order: the
/// part of a panel a resident composite caches. The fill loop comes first by
/// construction so another series' fill can never cover this series' line.
pub(crate) fn issue_series_bodies(
    pass: &mut wgpu::RenderPass<'_>,
    series_list: &[SeriesLayers<'_>],
) {
    for s in series_list {
        issue_series_fill(pass, s);
    }
    for s in series_list {
        issue_series_data(pass, s);
    }
}

/// [`draw_chart_panel_columnar`], with the fills and bodies taken from
/// `cached_bodies` when given: a panel-sized texture of exactly what
/// [`issue_series_bodies`] draws. The picked highlights always come from
/// `series_list`, so a selection never needs the bodies drawn again.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_chart_panel_with_bodies(
    pass: &mut wgpu::RenderPass<'_>,
    target_size: (u32, u32),
    panel_rect: Rect,
    data_area: Rect,
    grid: AxisLayer<'_>,
    annotation_underlay: Option<AxisLayer<'_>>,
    series_list: &[SeriesLayers<'_>],
    cached_bodies: Option<AxisLayer<'_>>,
    decoration: AxisLayer<'_>,
    annotation_overlay: Option<AxisLayer<'_>>,
) {
    let Some(panel_clamped) = clamp_rect_to_target(panel_rect, target_size) else {
        return;
    };
    let Some(data_clamped) = clamp_rect_to_target(data_area, target_size) else {
        return;
    };

    pass.set_viewport(
        panel_clamped.x as f32,
        panel_clamped.y as f32,
        panel_clamped.width as f32,
        panel_clamped.height as f32,
        0.0,
        1.0,
    );

    // 1) Grid layer (under data).
    pass.set_scissor_rect(
        panel_clamped.x,
        panel_clamped.y,
        panel_clamped.width,
        panel_clamped.height,
    );
    pass.set_pipeline(grid.pipeline);
    pass.set_bind_group(0, grid.bind_group, &[]);
    pass.draw(0..3, 0..1);

    // 1b) Annotation underlay (spans): below the data, above the grid.
    if let Some(layer) = annotation_underlay {
        pass.set_pipeline(layer.pipeline);
        pass.set_bind_group(0, layer.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    // 2) Data primitives — scissor once to data_area, then issue every series:
    //    every fill, then every body, then every picked highlight, each loop
    //    in SSOT series order. The fill loop comes first by construction so
    //    another series' fill can never cover this series' line.
    pass.set_scissor_rect(
        data_clamped.x,
        data_clamped.y,
        data_clamped.width,
        data_clamped.height,
    );
    match cached_bodies {
        Some(layer) => {
            // The composite was clipped to the data area when it was drawn;
            // clipping its blit again would apply the data scissor twice at
            // the edge samples, which a direct draw applies once.
            pass.set_scissor_rect(
                panel_clamped.x,
                panel_clamped.y,
                panel_clamped.width,
                panel_clamped.height,
            );
            pass.set_pipeline(layer.pipeline);
            pass.set_bind_group(0, layer.bind_group, &[]);
            pass.draw(0..3, 0..1);
            pass.set_scissor_rect(
                data_clamped.x,
                data_clamped.y,
                data_clamped.width,
                data_clamped.height,
            );
        }
        None => issue_series_bodies(pass, series_list),
    }
    for s in series_list {
        issue_series_picked(pass, s);
    }

    // 3) Decoration layer (over data).
    pass.set_scissor_rect(
        panel_clamped.x,
        panel_clamped.y,
        panel_clamped.width,
        panel_clamped.height,
    );
    pass.set_pipeline(decoration.pipeline);
    pass.set_bind_group(0, decoration.bind_group, &[]);
    pass.draw(0..3, 0..1);

    // 4) Annotation overlay: topmost, above every decoration.
    if let Some(layer) = annotation_overlay {
        pass.set_pipeline(layer.pipeline);
        pass.set_bind_group(0, layer.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

// Tests.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contour_lookup_uses_finite_prefixes_numeric_indices_and_block_metadata() {
        let mut levels: Vec<f32> = (0..35).map(|index| 100.0 - index as f32).collect();
        levels[2] = 50.0;
        levels[3] = 50.0;
        levels[4] = f32::NAN;
        levels[30] = f32::NEG_INFINITY;
        levels[32] = f32::NEG_INFINITY;
        levels[33] = f32::INFINITY;
        let ramp = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]];
        let lookup = build_field_lookup_tables(&ramp, &levels).unwrap();

        assert_eq!(&lookup.stops[..ramp.len()], &ramp);
        assert_eq!(lookup.stops.len(), ramp.len() + levels.len());
        assert_eq!(
            lookup.metadata,
            vec![
                ContourLookupMetadataGpu {
                    finite_count: 30,
                    negative_infinity_count: 1,
                },
                ContourLookupMetadataGpu {
                    finite_count: 1,
                    negative_infinity_count: 1,
                },
            ]
        );

        let search = &lookup.stops[ramp.len()..];
        let first_finite = &search[..lookup.metadata[0].finite_count as usize];
        assert!(first_finite.windows(2).all(|pair| pair[0][0] <= pair[1][0]));
        assert!(search[30..32].iter().all(|record| *record == [0.0; 4]));
        assert_eq!(search[32][0], levels[34]);
        assert_eq!(search[32][1], 34.0);
        assert!(search[33..35].iter().all(|record| *record == [0.0; 4]));

        let duplicate_indices: Vec<u32> = first_finite
            .iter()
            .filter(|record| record[0] == 50.0)
            .map(|record| record[1] as u32)
            .collect();
        assert_eq!(duplicate_indices, vec![2, 3]);
        for record in first_finite {
            let original = record[1] as usize;
            assert_eq!(record[1], original as f32);
            assert_eq!(record[0], levels[original]);
        }
    }

    #[test]
    fn contour_lookup_pads_empty_metadata_and_preserves_31_32_1023_indices() {
        let empty = build_field_lookup_tables(&[[0.0; 4]], &[]).unwrap();
        assert_eq!(empty.stops, vec![[0.0; 4]]);
        assert_eq!(empty.metadata, vec![ContourLookupMetadataGpu::default()]);

        let allowed = vec![0.0; CONTOUR_LEVEL_LOOKUP_CAPACITY];
        let lookup = build_field_lookup_tables(&[[0.0; 4]], &allowed).unwrap();
        assert_eq!(lookup.stops.len(), 1 + CONTOUR_LEVEL_LOOKUP_CAPACITY);
        assert_eq!(lookup.metadata.len(), CONTOUR_LEVEL_BLOCK_COUNT);
        let search = &lookup.stops[1..];
        for index in [31usize, 32, 1023] {
            let block = index / CONTOUR_LEVEL_BLOCK_SIZE;
            let start = block * CONTOUR_LEVEL_BLOCK_SIZE;
            let end = start + lookup.metadata[block].finite_count as usize;
            let record = search[start..end]
                .iter()
                .find(|record| record[1] == index as f32)
                .unwrap_or_else(|| panic!("numeric original index {index} was not preserved"));
            assert_eq!(record[1].to_bits(), (index as f32).to_bits());
        }

        let rejected = vec![0.0; CONTOUR_LEVEL_LOOKUP_CAPACITY + 1];
        assert!(matches!(
            build_field_lookup_tables(&[[0.0; 4]], &rejected),
            Err(crate::FiggyError::StateAllocationFailed {
                resource: "contour level lookup",
                ..
            })
        ));
    }

    #[test]
    fn contour_lookup_dimensions_match_the_model_and_shader() {
        assert_eq!(CONTOUR_LEVEL_BLOCK_SIZE, 32);
        assert_eq!(CONTOUR_LEVEL_BLOCK_COUNT, 32);
        assert_eq!(
            CONTOUR_LEVEL_LOOKUP_CAPACITY,
            crate::data_config::MAX_CONTOUR_LEVELS
        );

        let shader = include_str!("field_columnar.wgsl");
        assert!(shader.contains("const CONTOUR_LEVEL_BLOCK_SIZE: u32 = 32u;"));
        assert!(shader.contains("let original = firstTrailingBit(candidates);"));
        assert!(shader.contains("let original = u32(record.y) - start;"));
        assert!(shader.contains("contour_lookup_metadata[block].finite_count"));
        assert!(shader.contains("metadata.negative_infinity_count"));
        assert!(shader.contains("let search_lo = s.z_lo - margin;"));
        assert!(shader.contains("let search_hi = s.z_hi + margin;"));
        assert!(shader.contains("contour_lower_bound(start, count, search_lo)"));
        assert!(shader.contains("contour_upper_bound(start, count, search_hi)"));
        assert!(
            !shader.contains("for (var i = 0u; i < field.level_count"),
            "contour fragments must not return to a full level scan"
        );
    }

    #[test]
    fn field_table_exact_charge_includes_zero_level_lookup_metadata_padding() {
        let Some((device, _queue)) = shared_device() else {
            return;
        };
        let ledger = std::sync::Arc::new(crate::gpu_memory::GpuLedger::new());
        let layout = create_field_data_bind_group_layout(&device);
        let pool = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("field table charge test pool"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let lookup = build_field_lookup_tables(&[[0.0; 4]], &[]).unwrap();
        let params = <FieldParamsGpu as bytemuck::Zeroable>::zeroed();
        let (_bind_group, charge) = create_field_data_bind_group(
            &device,
            &ledger,
            &layout,
            &pool,
            FieldTables {
                grid: &[GridColumnGpu { base: 0, len: 0 }],
                levels: &[0.0],
                stops: &lookup.stops,
                level_colors: &[[0.0; 4]],
                lookup_metadata: &lookup.metadata,
                params: &params,
            },
        );

        let expected = std::mem::size_of::<GridColumnGpu>()
            + std::mem::size_of::<f32>()
            + std::mem::size_of::<[f32; 4]>()
            + std::mem::size_of::<[f32; 4]>()
            + std::mem::size_of::<ContourLookupMetadataGpu>()
            + std::mem::size_of::<FieldParamsGpu>();
        assert_eq!(expected, 116);
        assert_eq!(
            ledger
                .snapshot()
                .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            expected as u64
        );
        drop(charge);
        assert_eq!(
            ledger
                .snapshot()
                .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            0
        );
    }

    /// Instance creation must not panic, even on driver-less environments
    /// (it does not talk to any GPU yet).
    #[test]
    fn instance_creation_succeeds() {
        let _instance = create_instance();
    }

    /// Print adapter info; a missing adapter is a test failure.
    /// Mostly useful locally with `cargo test -- --nocapture`.
    #[test]
    fn adapter_request_prints_info_when_available() {
        let instance = create_instance();
        match request_adapter(&instance) {
            Ok(adapter) => {
                let info = adapter.get_info();
                println!("adapter name    : {}", info.name);
                println!("adapter backend : {:?}", info.backend);
                println!("adapter type    : {:?}", info.device_type);
                println!("adapter driver  : {} / {}", info.driver, info.driver_info);
            }
            Err(e) => {
                panic!("required GPU test: adapter initialization failed: {e}");
            }
        }
    }

    /// The constellation star picks on the disc its quad inscribes
    /// (SHADER_COMMON.md §5.7): the sprite is zero from 0.98 of the half
    /// width out, so even a bilinear tap a texel further stays inside it.
    #[test]
    fn psf_sprite_is_zero_outside_the_inscribed_disc() {
        let size = 128u32;
        let psf = bake_psf_rgba(size);
        let half = (size as f32 - 1.0) * 0.5;
        for y in 0..size {
            for x in 0..size {
                let dx = (x as f32 - half) / half;
                let dy = (y as f32 - half) / half;
                if (dx * dx + dy * dy).sqrt() >= 0.98 {
                    let i = ((y * size + x) * 4) as usize;
                    assert_eq!((psf[i], psf[i + 1]), (0, 0), "texel ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn psf_sprite_fades_to_zero_at_square_edges() {
        let psf = bake_psf_rgba(128);
        let px = |x: usize, y: usize, c: usize| psf[(y * 128 + x) * 4 + c];

        for i in 0..128 {
            assert_eq!(px(i, 0, 0), 0, "top edge core leaked at x={i}");
            assert_eq!(px(i, 0, 1), 0, "top edge halo leaked at x={i}");
            assert_eq!(px(i, 127, 0), 0, "bottom edge core leaked at x={i}");
            assert_eq!(px(i, 127, 1), 0, "bottom edge halo leaked at x={i}");
            assert_eq!(px(0, i, 0), 0, "left edge core leaked at y={i}");
            assert_eq!(px(0, i, 1), 0, "left edge halo leaked at y={i}");
            assert_eq!(px(127, i, 0), 0, "right edge core leaked at y={i}");
            assert_eq!(px(127, i, 1), 0, "right edge halo leaked at y={i}");
        }
    }

    #[test]
    fn log_transform_guards_manual_range_without_raising_tiny_positive_min() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 0.0;
            axis.major_tick_length = 0.0;
        }

        config.bottom_x.scale = crate::config::AxisScale::Logarithmic;
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 1000.0;
        let t = scatter_transform_from_config(&config);
        assert_eq!(t.scale_log[0], 1.0);
        assert!((t.data_min[0] + 12.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] - 3.0).abs() < 1.0e-6, "{:?}", t.data_max);

        config.bottom_x.min = 1.0e-15;
        config.bottom_x.max = 1.0e-12;
        let t = scatter_transform_from_config(&config);
        assert!((t.data_min[0] + 15.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] + 12.0).abs() < 1.0e-6, "{:?}", t.data_max);

        config.bottom_x.min = 10.0;
        config.bottom_x.max = 1.0;
        let t = scatter_transform_from_config(&config);
        assert!((t.data_min[0] - 1.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] - 2.0).abs() < 1.0e-6, "{:?}", t.data_max);
    }

    #[test]
    fn scatter_transform_inverted_axes_swap_final_ranges() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 160,
            height: 120,
        });
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 8.0;
            axis.major_tick_length = 0.0;
        }
        config.bottom_x.scale = crate::config::AxisScale::Logarithmic;
        config.bottom_x.min = 1.0;
        config.bottom_x.max = 100.0;
        config.left_y.min = -5.0;
        config.left_y.max = 5.0;

        let normal = scatter_transform_from_config(&config);
        config.bottom_x.inverted = true;
        config.left_y.inverted = true;
        let inverted = scatter_transform_from_config(&config);

        assert_eq!(inverted.scale_log, normal.scale_log);
        assert!((inverted.data_min[0] - normal.data_max[0]).abs() < 1.0e-6);
        assert!((inverted.data_max[0] - normal.data_min[0]).abs() < 1.0e-6);
        assert!((inverted.data_min[1] - normal.data_max[1]).abs() < 1.0e-6);
        assert!((inverted.data_max[1] - normal.data_min[1]).abs() < 1.0e-6);
        for v in inverted.data_min.into_iter().chain(inverted.data_max) {
            assert!(v.is_finite(), "{inverted:?}");
        }
    }

    #[test]
    fn gpu_transform_uses_auto_fit_ssot_bounds_bit_for_bit() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 17,
            y: 29,
            width: 900,
            height: 600,
        });
        let x_extent = crate::chart::FitExtent {
            min: 1_700_000_000_000.125,
            max: 1_700_000_000_009.875,
            min_positive: Some(1_700_000_000_000.125),
        };
        let y_extent = crate::chart::FitExtent {
            min: -3.25,
            max: 8.75,
            min_positive: Some(0.125),
        };
        crate::chart::apply_auto_fit_all(&mut config, &x_extent, &y_extent, 0.05);

        let transform = scatter_transform_from_config(&config);
        let x_min = crate::data::split_f64_to_f32_pair(config.bottom_x.min);
        let x_max = crate::data::split_f64_to_f32_pair(config.bottom_x.max);
        let y_min = crate::data::split_f64_to_f32_pair(config.left_y.min);
        let y_max = crate::data::split_f64_to_f32_pair(config.left_y.max);
        assert_eq!(
            [transform.data_min[0], transform.data_min_lo[0]],
            [x_min.0, x_min.1]
        );
        assert_eq!(
            [transform.data_max[0], transform.data_max_lo[0]],
            [x_max.0, x_max.1]
        );
        assert_eq!(
            [transform.data_min[1], transform.data_min_lo[1]],
            [y_min.0, y_min.1]
        );
        assert_eq!(
            [transform.data_max[1], transform.data_max_lo[1]],
            [y_max.0, y_max.1]
        );

        // Layout changes only the panel affine. They can no longer manufacture
        // a second, margin-extended axis range behind Config's back.
        let mut different_layout = config.clone();
        different_layout.bottom_x.out_margin += 40.0;
        different_layout.left_y.out_margin += 30.0;
        let moved = scatter_transform_from_config(&different_layout);
        assert_eq!(moved.data_min, transform.data_min);
        assert_eq!(moved.data_max, transform.data_max);
        assert_eq!(moved.data_min_lo, transform.data_min_lo);
        assert_eq!(moved.data_max_lo, transform.data_max_lo);
        assert_ne!(moved.data_to_panel_offset, transform.data_to_panel_offset);
        assert_ne!(moved.data_to_panel_scale, transform.data_to_panel_scale);
    }

    /// Smoke-test the texture-upload API path (no readback). Validation
    /// would surface here if the call shape were wrong.
    #[test]
    fn rgba_texture_upload_roundtrips_api() {
        let Some((device, queue)) = shared_device() else {
            println!("no adapter — skipping texture upload test");
            return;
        };

        // 2x2 checkerboard: 4 RGBA pixels = 16 bytes.
        let rgba: [u8; 16] = [
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 255, 0, 255, // yellow
        ];
        let tex = upload_rgba_texture(
            &device,
            &queue,
            device.limits().max_texture_dimension_2d,
            2,
            2,
            &rgba,
        )
        .expect("upload texture");

        // Wait for the queued write_texture to complete; validation errors
        // surface during this poll.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });

        assert_eq!(tex.width(), 2);
        assert_eq!(tex.height(), 2);
        assert_eq!(tex.format(), wgpu::TextureFormat::Rgba8Unorm);
    }

    /// Compile the WGSL and build the fullscreen textured pipeline. Shader
    /// or layout mismatches would panic during creation.
    #[test]
    fn fullscreen_textured_pipeline_compiles_and_creates() {
        let Some((device, _queue)) = shared_device() else {
            println!("no adapter — skipping pipeline test");
            return;
        };

        let bgl = create_texture_bind_group_layout(&device);
        let pipeline = create_fullscreen_textured_pipeline(
            &device,
            &bgl,
            // Same non-sRGB format the surface picks at runtime.
            wgpu::TextureFormat::Bgra8Unorm,
        );

        let _ = pipeline;
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    #[test]
    fn streamed_point_style_map_reuses_rows_and_charges_only_meta() {
        let (device, _queue) = shared_device().expect("streamed point style map GPU adapter");
        let layout = create_scatter_style_map_bind_group_layout(&device);
        let map = create_scatter_style_map(
            &device,
            &layout,
            &[ScatterStyleSlotGpu {
                color_premul: [1.0, 0.0, 0.0, 1.0],
                meta: [3.0, 0.0, 1.0, 0.0],
            }],
            &[ScatterStyleOverrideGpu {
                point_index: 17,
                _pad: [0; 3],
                color_premul: [0.0, 1.0, 0.0, 1.0],
                meta: [3.0, 0.0, 1.0, 0.0],
            }],
            ScatterStyleMapMeta {
                style_count: 1,
                override_count: 1,
                has_index: 1,
                _pad: 0,
            },
        );
        assert_eq!(map.meta._pad, 0);
        let tally = crate::gpu_memory::ChargeTally::new();
        let streamed = map.stream_bind_group(&device, &layout, 17, &tally);
        assert_eq!(tally.bytes(), 16);
        drop(map);
        let _ = streamed;
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Compile the scatter WGSL entries added outside the common block for
    /// per-point style mapping and picked-point decoration. This catches WGSL
    /// syntax/layout drift without running a full render snapshot.
    #[test]
    fn scatter_mapped_and_pick_ring_pipelines_compile() {
        let Some((device, _queue)) = shared_device() else {
            println!("no adapter ??skipping scatter entry pipeline test");
            return;
        };
        let transform_bgl = create_scatter_transform_bind_group_layout(&device);
        let style_bgl = create_style_bind_group_layout(&device);
        let style_map_bgl = create_scatter_style_map_bind_group_layout(&device);
        let value_map_bgl = create_point_value_map_bind_group_layout(&device);
        let shaders = ShaderModules::new(&device);

        let mapped = create_scatter_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            &value_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
        );
        let picked = create_scatter_columnar_pipeline_with_entries(
            &device,
            &shaders.scatter,
            &transform_bgl,
            &style_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
            "vs_pick_ring",
            "fs_pick_ring",
            "figgy picked point ring pipeline test",
        );
        let picked_mapped = create_scatter_columnar_mapped_pipeline_with_entries(
            &device,
            &shaders.scatter,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            &value_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
            "vs_pick_ring_mapped",
            "fs_pick_ring",
            "figgy picked point mapped ring pipeline test",
        );
        let errorbar_mapped = create_errorbar_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
        );
        let bar_mapped = create_bar_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
        );

        let _ = (mapped, picked, picked_mapped, errorbar_mapped, bar_mapped);
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Wire up sampler + bind-group layout + bind group end-to-end; a
    /// slot-type mismatch would panic in `create_bind_group`.
    #[test]
    fn texture_sampler_bind_group_wires_up() {
        let Some((device, queue)) = shared_device() else {
            println!("no adapter — skipping bind group test");
            return;
        };

        let rgba: [u8; 16] = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let texture = upload_rgba_texture(
            &device,
            &queue,
            device.limits().max_texture_dimension_2d,
            2,
            2,
            &rgba,
        )
        .expect("upload texture");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = create_linear_sampler(&device);
        let layout = create_texture_bind_group_layout(&device);
        let _bind_group = create_texture_bind_group(&device, &layout, &view, &sampler);

        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Open a device + queue without a surface and print a few limits.
    /// Fails when no adapter is available.
    #[test]
    fn device_request_opens_device_and_queue() {
        let instance = create_instance();
        let adapter =
            request_adapter(&instance).expect("required GPU test: adapter initialization failed");
        match request_device(&adapter) {
            Ok((device, _queue)) => {
                let limits = device.limits();
                println!("device opened OK");
                println!(
                    "  max_texture_dim_2d     : {}",
                    limits.max_texture_dimension_2d
                );
                println!("  max_buffer_size        : {}", limits.max_buffer_size);
                println!("  max_bind_groups        : {}", limits.max_bind_groups);
            }
            Err(e) => panic!("request_device failed on available adapter: {e}"),
        }
    }
}

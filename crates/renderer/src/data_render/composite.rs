//! The data slot of a panel: the series layer set and the order its
//! primitives are issued in (every fill, then every body, then every picked
//! highlight, then every value label, in SSOT series order). Where the slot sits between the chrome
//! below and above the data is the compositor's (`renderer::composite`).

use super::*;

/// One series' data primitives. A panel can hold multiple of these.
pub struct SeriesLayers<'a> {
    /// A line's area fill. Not drawn by `issue_series_data`: every series'
    /// fill is issued first by `issue_series_fill`, in series order, before
    /// any series body — see `issue_series_bodies`.
    pub area: Option<ColumnAreaLayer<'a>>,
    /// The field covers the whole grid, so it goes under everything else —
    /// see `issue_series_data` for the full order.
    pub field: Option<ColumnFieldLayer<'a>>,
    /// A heatmap on a 3D chart, drawn as a surface into the panel
    /// composite with the depth test (`issue_series_surfaces`), never by
    /// `issue_series_data`.
    pub surface: Option<super::surface_mesh::ColumnSurfaceLayer<'a>>,
    /// A contour on a 3D chart's floor (`vs_floor`, one instance), drawn
    /// before every surface without writing depth.
    pub floor: Option<super::surface_mesh::ColumnSurfaceLayer<'a>>,
    /// The picked sample's patch of the surface, in the live highlight slot.
    pub selected_surface: Option<super::surface_mesh::ColumnSurfaceLayer<'a>>,
    /// A ternary series' region fill, drawn into the panel composite's
    /// stencil attachment before every series' fill and body
    /// (`issue_series_regions`), never by `issue_series_data`.
    pub region: Option<super::ternary_region::ColumnTernaryRegionLayer<'a>>,
    /// Bars are area primitives, so they are issued before the stroked ones.
    pub bar: Option<ColumnBarLayer<'a>>,
    /// A box plot series' violins, under its boxes.
    pub violin: Option<ColumnCategoryBarLayer<'a>>,
    /// The bars of a category bar series or the boxes of a box plot
    /// series, issued where histogram bars are.
    pub category_bars: Option<ColumnCategoryBarLayer<'a>>,
    /// A box plot series' raw points, over its boxes.
    pub box_points: Option<ColumnCategoryBarLayer<'a>>,
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
    /// A line series on a 3D chart (#132), drawn into the panel composite
    /// with the depth test after the surfaces (`issue_series_surfaces`),
    /// never by `issue_series_data`.
    pub line_3d: Option<ColumnLineLayer<'a>>,
    /// A scatter series on a 3D chart (#132), drawn as `line_3d` is, after
    /// every series' 3D line.
    pub scatter_3d: Option<ColumnScatterLayer<'a>>,
    pub selected_bars: Vec<ColumnBarSelectionLayer<'a>>,
    pub selected_fields: Vec<ColumnFieldSelectionLayer<'a>>,
    pub picked: Vec<ColumnPickRingLayer<'a>>,
    /// The value labels of a category bar, box plot or radar series and its
    /// selected label's box: issued after every series' picked highlights
    /// (`issue_series_value_labels`), so a label sits over all the data of
    /// its panel and under the decoration.
    pub value_labels: Option<ColumnValueLabelLayer<'a>>,
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
/// body (`issue_series_bodies`): the data pass is "every fill → every
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
    if a.closed {
        super::line::draw_area_closing(pass, a, count);
    }
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

    // Category bars and box plots: area primitives like histogram bars, one
    // instance per bar or box built from group 3; no vertex buffers. A box
    // plot's violins go under its boxes and its raw points over them.
    for c in [&series.violin, &series.category_bars, &series.box_points]
        .into_iter()
        .flatten()
        .filter(|c| c.count > 0)
    {
        pass.set_pipeline(c.pipeline);
        pass.set_bind_group(0, c.transform_bg, &[]);
        pass.set_bind_group(1, c.style_bg, &[]);
        pass.set_bind_group(2, &c.data.style_map, &[]);
        pass.set_bind_group(3, &c.data.bind_group, &[]);
        pass.draw(0..c.vertices, 0..c.count);
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
    // Streamline arrowheads draw indirect: one instance per arc slot, the
    // count the arc scan wrote on the GPU, group 3 the slots' arc data.
    if let Some(v) = series.vector.as_ref() {
        match v.indirect {
            Some((args, slots)) => {
                pass.set_pipeline(v.pipeline);
                pass.set_bind_group(0, v.transform_bg, &[]);
                pass.set_bind_group(1, &v.style_bg, &[]);
                pass.set_bind_group(2, &v.vector_bg, &[]);
                pass.set_bind_group(3, slots, &[]);
                pass.draw_indirect(args, 0);
            }
            None if v.count > 0 => {
                pass.set_pipeline(v.pipeline);
                pass.set_bind_group(0, v.transform_bg, &[]);
                pass.set_bind_group(1, &v.style_bg, &[]);
                pass.set_bind_group(2, &v.vector_bg, &[]);
                pass.draw(0..vector::ARROW_VERTICES, 0..v.count);
            }
            None => {}
        }
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
        draw_scatter_layer(pass, s);
    }
}

/// Issue one scatter layer: the markers of a flat, ternary or 3D series
/// (the 3D one into a pass with the depth attachment, its camera as
/// group 3).
pub(crate) fn draw_scatter_layer(pass: &mut wgpu::RenderPass<'_>, s: &ColumnScatterLayer<'_>) {
    let count =
        s.x.len_values
            .min(s.y.len_values)
            .min(s.third.map_or(usize::MAX, |c| c.len_values)) as u32;
    if count == 0 {
        return;
    }
    pass.set_pipeline(s.pipeline);
    pass.set_bind_group(0, s.transform_bg, &[]);
    pass.set_bind_group(1, s.style_bg, &[]);
    if let Some(map) = s.style_map_bg {
        pass.set_bind_group(2, map, &[]);
        // A 3D series binds its value map and camera as one group.
        if s.view3d.is_none() {
            pass.set_bind_group(
                3,
                s.value_map_bg
                    .expect("mapped scatter draw binds its value map group"),
                &[],
            );
        }
    } else if let Some(tex) = s.texture_bg {
        pass.set_bind_group(2, tex, &[]);
    }
    if let Some(view3d) = &s.view3d {
        pass.set_bind_group(3, &view3d.bind_group, &[]);
    }
    pass.set_vertex_buffer(0, s.quad_vb.slice(..));
    pass.set_vertex_buffer(1, s.pool_buffer.slice(s.x.byte_range()));
    pass.set_vertex_buffer(2, s.pool_buffer.slice(s.y.byte_range()));
    // A ternary series' third component or a 3D series' height, after the
    // slots its pipeline shares with the plain one.
    if let Some(c) = s.third {
        let slot = if s.style_map_bg.is_some() { 6 } else { 3 };
        pass.set_vertex_buffer(slot, s.pool_buffer.slice(c.byte_range()));
    }
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

/// Issue one series' picked highlights: the selected bars and fields and the
/// pick rings, after every body. The caller must have already set the
/// viewport (panel) and scissor (data_area).
pub(crate) fn issue_series_picked(pass: &mut wgpu::RenderPass<'_>, series: &SeriesLayers<'_>) {
    // The picked surface sample's patch, over whatever is in front of it
    // (#49 D4): the live slot has no depth.
    if let Some(patch) = &series.selected_surface {
        pass.set_pipeline(patch.pipeline);
        pass.set_bind_group(0, patch.transform_bg, &[]);
        pass.set_bind_group(1, &patch.style_bg, &[]);
        pass.set_bind_group(2, &patch.field_bg, &[]);
        pass.set_bind_group(3, &patch.surface_bg, &[]);
        pass.draw(
            0..super::surface_mesh::SURFACE_QUAD_VERTICES,
            0..super::surface_mesh::SURFACE_SELECTED_INSTANCES,
        );
    }
    for selection in &series.selected_bars {
        pass.set_pipeline(selection.pipeline);
        pass.set_bind_group(0, selection.transform_bg, &[]);
        pass.set_bind_group(1, &selection.style_bg, &[]);
        if let Some(map) = selection.style_map_bg {
            pass.set_bind_group(2, map, &[]);
        }
        if let Some(category) = &selection.category {
            pass.set_bind_group(2, &category.style_map, &[]);
            pass.set_bind_group(3, &category.bind_group, &[]);
            pass.draw(
                0..CATEGORY_BAR_VERTICES,
                selection.instance..selection.instance.saturating_add(selection.instances),
            );
            continue;
        }
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
            if p.view3d.is_none() {
                pass.set_bind_group(
                    3,
                    p.value_map_bg
                        .expect("mapped pick ring draw binds its value map group"),
                    &[],
                );
            }
        }
        // A 3D series' ring: the camera (with its value map when mapped).
        if let Some(view3d) = &p.view3d {
            pass.set_bind_group(3, &view3d.bind_group, &[]);
        }
        pass.set_vertex_buffer(0, p.quad_vb.slice(..));
        pass.set_vertex_buffer(1, p.pool_buffer.slice(p.x.byte_range()));
        pass.set_vertex_buffer(2, p.pool_buffer.slice(p.y.byte_range()));
        if let Some(c) = p.third {
            let slot = if p.style_map_bg.is_some() { 6 } else { 3 };
            pass.set_vertex_buffer(slot, p.pool_buffer.slice(c.byte_range()));
        }
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

/// Issue one series' value labels and its selected label's box. Called for
/// every series after every picked highlight; the caller has set the
/// viewport (panel) and scissor (data_area).
pub(crate) fn issue_series_value_labels(
    pass: &mut wgpu::RenderPass<'_>,
    series: &SeriesLayers<'_>,
) {
    if let Some(layer) = series.value_labels.as_ref() {
        issue_value_labels(pass, layer);
    }
}

/// Every series' fill, then every body, each loop in SSOT series order: the
/// part of a panel a resident composite caches. The fill loop comes first by
/// construction so another series' fill can never cover this series' line.
/// The caller has set the viewport (panel) and the scissor (data area).
/// Issue every series' surface: the 3D draw, into a pass with the panel
/// composite's depth-stencil attachment.
/// Every series' ternary region fill, in series order, into a pass with
/// the stencil attachment: under every fill and body, as the line fills
/// are under every body.
pub(crate) fn issue_series_regions(
    pass: &mut wgpu::RenderPass<'_>,
    series_list: &[SeriesLayers<'_>],
) {
    for s in series_list {
        if let Some(region) = &s.region {
            super::ternary_region::issue_ternary_region_layer(pass, region);
        }
    }
}

pub(crate) fn issue_series_surfaces(
    pass: &mut wgpu::RenderPass<'_>,
    series_list: &[SeriesLayers<'_>],
) {
    // The floor contours first, without writing depth: the surfaces blend
    // over them, so an opaque surface covers the floor under it and a
    // translucent one (`fill.opacity` below 1) shows it through.
    for s in series_list {
        if let Some(floor) = &s.floor {
            super::surface_mesh::issue_surface_layer(pass, floor);
        }
    }
    for s in series_list {
        if let Some(surface) = &s.surface {
            super::surface_mesh::issue_surface_layer(pass, surface);
        }
    }
    // The 3D lines and markers (#132), depth tested against the surfaces
    // and each other: every series' line, then every series' markers, as
    // the flat bodies order them.
    for s in series_list {
        if let Some(line) = &s.line_3d {
            draw_line_layer(pass, line);
        }
    }
    for s in series_list {
        if let Some(scatter) = &s.scatter_3d {
            draw_scatter_layer(pass, scatter);
        }
    }
}

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

//! The colourbar strip and its title.

use super::*;

/// The colourbar and its strip rect when it is drawn: present, visible and
/// not degenerate. The renderer draws the bar, its chrome and its title only
/// then, and the strip, axis, label and title elements exist only then.
pub fn visible_colorbar_rect(cfg: &Config) -> Option<(&ColorBarOptions, RectF)> {
    let bar = cfg.colorbar.as_ref()?;
    if !bar.visible {
        return None;
    }
    let da = cfg.data_area().ok()?;
    let rect = colorbar_rect(&cfg.chart_area, &da, cfg.chart_title.top_margin, bar);
    (rect.width > 0.0 && rect.height > 0.0).then_some((bar, rect))
}

/// The room [`colorbar_title_placement`] keeps between the colourbar's
/// labels and its title, and between the title and the band's outer edge:
/// half the label font ([`AUTO_LAYOUT_TITLE_GAP_EMS`]) and half the title
/// font ([`AUTO_LAYOUT_EDGE_PADDING_EMS`]), the proportional parts of what
/// [`auto_layout`] keeps (#270), so a scaled export keeps the 1x proportions.
pub(super) fn colorbar_title_gaps(axis: &AxisOptions) -> (f32, f32) {
    (
        AUTO_LAYOUT_TITLE_GAP_EMS * axis.label_style.font_size.max(0.0),
        AUTO_LAYOUT_EDGE_PADDING_EMS * axis.title_option.text.font_size.max(0.0),
    )
}

/// Place a colourbar title from the actual painted strip rectangle.
///
/// The title sits in `axis.out_margin`, immediately beyond the strip's major
/// tick reach, and is centred along the strip's own long dimension. Therefore
/// `length_frac`, `align`, `offset_{x,y}`, and resize all move the title through
/// the same rectangle that the renderer paints and the hit map selects.
///
/// `labels_reach` is how far past the strip's outer edge the tick labels
/// reach ([`colorbar_labels_reach`](crate::chrome::colorbar_labels_reach),
/// zero without labels). A title that the band's centre would put over the
/// labels sits a small gap past them instead, as far as the band reaches
/// (#259).
pub fn colorbar_title_placement(
    side: Side,
    strip: &RectF,
    axis: &AxisOptions,
    offset: (f32, f32),
    extents: TextExtents,
    labels_reach: f32,
) -> TitlePlacement {
    let tick = axis.major_tick_length.max(0.0);
    let out_margin = axis.out_margin.max(0.0);
    // The room kept between the labels and the title, and between the title
    // and the band's outer edge: half the label font and half the title font
    // ([`colorbar_title_gaps`]).
    let (gap, edge) = colorbar_title_gaps(axis);
    let clear = if labels_reach.is_finite() && labels_reach > 0.0 {
        labels_reach + gap
    } else {
        0.0
    };
    // How far past the strip's outer edge the title's inner edge sits:
    // centred in the band, or past the labels, but no closer to the band's
    // outer edge (where the chart area may end) than the edge room.
    let centred = tick + (out_margin - extents.height()) * 0.5;
    let inner = centred.max(clear.min(tick + out_margin - extents.height() - edge));
    match side {
        Side::Top | Side::Bottom => {
            let title_top = if matches!(side, Side::Top) {
                strip.y - inner - extents.height()
            } else {
                strip.y + strip.height + inner
            };
            let origin = (
                strip.x + strip.width * 0.5 - extents.width * 0.5 + offset.0,
                title_top + extents.ascent + offset.1,
            );
            TitlePlacement {
                origin,
                rotation_center: origin,
                rotation_degrees: 0.0,
            }
        }
        Side::Left | Side::Right => {
            // The rotated title's thickness across the strip is its height.
            let reach = inner + extents.height() * 0.5;
            let center = (
                if matches!(side, Side::Left) {
                    strip.x - reach
                } else {
                    strip.x + strip.width + reach
                },
                strip.y + strip.height * 0.5,
            );
            TitlePlacement {
                origin: (
                    center.0 - extents.width * 0.5 + offset.0,
                    center.1 + (extents.ascent - extents.descent) * 0.5 + offset.1,
                ),
                rotation_center: center,
                rotation_degrees: if matches!(side, Side::Left) {
                    -90.0
                } else {
                    90.0
                },
            }
        }
    }
}

/// The colourbar strip's rectangle.
///
/// `side` alone decides the orientation: `Left` / `Right` give a vertical strip
/// `thickness_px` wide, `Top` / `Bottom` a horizontal one `thickness_px` tall.
/// The long dimension is `length_frac` of the data area's matching dimension,
/// placed by `align`, so the bar lines up with the plot it describes. The
/// strip's axis runs along its outer long edge — which is why its ends carry
/// `axis.min` / `axis.max` rather than the data area's corners.
///
/// **The short dimension is measured inward from the chart-area edge, not
/// outward from the data area.** The colourbar band is the *outermost* part of
/// its side's margin: from the chart edge inward it is the strip's own label
/// margin and tick, then the strip, then `gap_px`, and only then the chart
/// axis' band next to the data area. Placing the strip immediately outside the
/// data area instead would sit it on top of that axis' tick labels whenever
/// they are visible on the same side — and axis labels are drawn from the data
/// area outward, so they cannot move out of the way.
///
/// The two agree by construction: `gap_px` is what is left between the axis band
/// and the strip precisely because `colorbar_parts` charged the same terms to
/// that side's margin.
///
/// `offset_x` / `offset_y` shift the result and nothing else: they are
/// margin-noncontributing, exactly like the legend's and the titles' offsets, so
/// dragging the bar does not reflow the data area out from under the pointer.
///
/// Callers get a rect for a hidden bar too — the visibility decision belongs to
/// the drawing code and to `colorbar_contribution`, not here.
pub fn colorbar_rect(
    chart_area: &ChartArea,
    data_area: &DataArea,
    chart_title_margin: f32,
    bar: &ColorBarOptions,
) -> RectF {
    let frac = if bar.length_frac.is_finite() {
        bar.length_frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thickness = bar.thickness_px.max(0.0);
    // The strip's distance from the chart edge: its labels, then its ticks, then
    // the strip itself.
    let outer = bar.axis.out_margin.max(0.0) + bar.axis.major_tick_length.max(0.0) + thickness;
    let (dx, dy, dw, dh) = (
        data_area.x as f32,
        data_area.y as f32,
        data_area.width as f32,
        data_area.height as f32,
    );
    let (cx, cy, cw, ch) = (
        chart_area.x as f32,
        chart_area.y as f32,
        chart_area.width as f32,
        chart_area.height as f32,
    );

    // Written as one match over the four sides so a nested wildcard never stands
    // in for a side.
    let vertical_length = dh * frac;
    let horizontal_length = dw * frac;
    let anchored = match bar.side {
        Side::Left => RectF {
            x: cx + outer - thickness,
            y: dy + align_offset(dh, vertical_length, &bar.align),
            width: thickness,
            height: vertical_length,
        },
        Side::Right => RectF {
            x: cx + cw - outer,
            y: dy + align_offset(dh, vertical_length, &bar.align),
            width: thickness,
            height: vertical_length,
        },
        // The chart title owns the very top of the chart area; the colourbar
        // band is the outermost part of what is left, exactly as
        // `top_total` sums them.
        Side::Top => RectF {
            x: dx + align_offset(dw, horizontal_length, &bar.align),
            y: cy + chart_title_margin + outer - thickness,
            width: horizontal_length,
            height: thickness,
        },
        Side::Bottom => RectF {
            x: dx + align_offset(dw, horizontal_length, &bar.align),
            y: cy + ch - outer,
            width: horizontal_length,
            height: thickness,
        },
    };
    anchored.translated(bar.offset_x, bar.offset_y)
}

/// Where a strip of `length` starts within `available`. `Start` is top / left,
/// matching the screen direction the sides are named in.
fn align_offset(available: f32, length: f32, align: &BarAlign) -> f32 {
    match align {
        BarAlign::Start => 0.0,
        BarAlign::Center => (available - length) * 0.5,
        BarAlign::End => available - length,
    }
}

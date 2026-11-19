//! The model's own placements of the decoration elements.

use super::*;

// The model's own placements: the elements it can lay out from the layout
// functions and a text measure. The renderer draws them through the same
// function.

/// Place decoration `element` of `cfg` into `sink`, in `cfg`'s chart frame,
/// and return whether it could be laid out: the one placement of the chart
/// title, the axis titles, the legend, the colourbar strip and its title and
/// the data area, which the renderer's raster draws with too. Tick labels,
/// axis strokes and the colourbar's axis and labels are placed by
/// [`crate::chrome::emit_part_of`], with the tick plan, formatting and
/// pruning the raster draws, measured by `measure`.
///
/// `cfg` is drawn as it is: a caller with a layered document passes the
/// panel the layer is drawn with (the elements of this module do).
pub fn emit_element(
    cfg: &Config,
    element: &Element,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) -> bool {
    match element {
        Element::ChartTitle => emit_chart_title(cfg, measure, sink),
        Element::AxisTitle(side) => emit_axis_title(cfg, side, measure, sink),
        Element::Legend => emit_legend(cfg, measure, sink),
        Element::ColorBar => emit_colorbar_strip(cfg, sink),
        Element::ColorBarTitle => emit_colorbar_title(cfg, measure, sink),
        Element::DataArea => emit_data_area(cfg, sink),
        Element::AxisLabel(_)
        | Element::Axis(_)
        | Element::ColorBarAxis
        | Element::ColorBarLabel
        | Element::ParallelAxisLabel(_)
        | Element::CategoryGroupLabel(_)
        | Element::CategoryLabel(_) => {
            // The hit map has no display scale: a broken axis' frames snap
            // on the grid of the pixels it is given, within a logical pixel
            // of the drawn frame and well inside the stroke grab width.
            return crate::chrome::emit_part_of(cfg, element, measure, sink, 1.0);
        }
        // Resize targets: nothing of their own is drawn.
        Element::DataAreaEdge(_) | Element::ColorBarEdge(_) | Element::SubplotFrameEdge(..) => {}
        // The frame inset by the selection padding and half a handle, so
        // its selection box and handles lie inside the frame, in the
        // subplot's own panel, wherever the subplot is (#205).
        Element::SubplotFrame(index) => {
            if let Some(subplot) = cfg.subplots.get(*index) {
                let inset = SELECTION_PADDING + crate::resize::HANDLE_SIZE / 2.0;
                sink.rect(
                    Slot::Above,
                    RectF::from_rect(&subplot.config.chart_area.0).expanded(-inset),
                    None,
                    None,
                );
            }
        }
    }
    true
}

fn emit_chart_title(cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) {
    let ct = &cfg.chart_title;
    emit_chart_title_in(
        ct,
        RectF::from_rect(&cfg.chart_area),
        ct.top_margin,
        measure,
        sink,
    );
}

/// The one placement of a chart title: `title` centred across `chart_area`
/// and on its baseline in the band `top_margin` high at its top
/// ([`chart_title_placement_in`]), moved by the title's offsets. A Cartesian chart passes its chart area and the title's own
/// band; a chart that lays out its own title band (the radial chart) passes
/// its canvas and that band. Places nothing for a hidden or empty title.
pub fn emit_chart_title_in(
    title: &ChartTitleOptions,
    chart_area: RectF,
    top_margin: f32,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let ct = title;
    if !ct.visible || ct.text.segments.is_empty() {
        return;
    }
    let measured = MeasuredTitle::of(&ct.text, measure);
    let placement =
        chart_title_placement_in(chart_area, top_margin, (ct.offset_x, ct.offset_y), measured);
    sink.text(
        Slot::Above,
        &PlacedText {
            text: TextRef::Rich(&ct.text),
            origin: placement.origin,
            extents: measured.ink,
            rotation: None,
        },
    );
}

/// The axis title on `side`. A polar chart draws no top or right axis, so
/// their titles are not placed either; a ternary chart places the titles of
/// its three edges along them, a 3D chart those of its three box axes, and
/// neither draws a top title.
fn emit_axis_title(cfg: &Config, side: &Side, measure: &dyn MeasureText, sink: &mut dyn Sink) {
    if cfg.polar().is_some() && matches!(side, Side::Top | Side::Right) {
        return;
    }
    let axis = crate::layout::axis_ref(cfg, side);
    let to = &axis.title_option;
    if !to.visible || to.text.segments.is_empty() {
        return;
    }
    let Ok(da) = cfg.data_area() else {
        return;
    };
    let m = measure.measure_rich(&to.text);
    let slanted = crate::ternary::side_component(side).and_then(|k| {
        let offset = (to.offset_x, to.offset_y);
        if cfg.ternary().is_some() {
            let reach = crate::chrome::ternary_label_reach(cfg, k, measure);
            cfg.ternary_title_placement(k, offset, m, reach)
        } else {
            let reach = crate::chrome::box3d_label_reach(cfg, k, measure);
            cfg.box_title_placement(k, offset, m, reach)
        }
    });
    if (cfg.ternary().is_some() || cfg.view_3d().is_some()) && slanted.is_none() {
        return;
    }
    let ternary = slanted;
    let placement = ternary.unwrap_or_else(|| {
        let at = |offset| {
            axis_title_placement(
                side.clone(),
                &cfg.chart_area,
                &da,
                cfg.title_band(side),
                offset,
                m,
            )
        };
        let mut placement = at((to.offset_x, to.offset_y));
        // A polar chart's titles sit beside its disc, not at the band's
        // edge (#272); the offsets move them from there.
        if let Some((dx, dy)) = cfg
            .polar()
            .and_then(|_| crate::chrome::polar_title_shift(cfg, side, &at((0.0, 0.0)), m, measure))
        {
            placement.origin = (placement.origin.0 + dx, placement.origin.1 + dy);
            placement.rotation_center = (
                placement.rotation_center.0 + dx,
                placement.rotation_center.1 + dy,
            );
        }
        placement
    });
    sink.text(
        Slot::Above,
        &PlacedText {
            text: TextRef::Rich(&to.text),
            origin: placement.origin,
            extents: m,
            rotation: (placement.rotation_degrees != 0.0)
                .then_some((placement.rotation_degrees, placement.rotation_center)),
        },
    );
}

/// The legend box in one corner of the data area: the box with its fill and
/// border, then the content, whose first baseline sits `ascent` below the
/// padded top-left corner.
fn emit_legend(cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) {
    let lg = &cfg.legend;
    if !lg.visible || lg.content.segments.is_empty() {
        return;
    }
    let Ok(da) = cfg.data_area() else {
        return;
    };
    let m = measure.measure_rich(&lg.content);
    let rect = legend_rect(&da, lg.corner, lg.padding, (lg.offset_x, lg.offset_y), m);
    sink.rect(
        Slot::Above,
        rect,
        Some(lg.bg_color),
        Some(Border {
            color: lg.border_color,
            width: LEGEND_BORDER_WIDTH,
            tag: "legend_box",
        }),
    );
    sink.text(
        Slot::Above,
        &PlacedText {
            text: TextRef::Rich(&lg.content),
            origin: (rect.x + lg.padding, rect.y + lg.padding + m.ascent),
            extents: m,
            rotation: None,
        },
    );
}

/// The colourbar strip: the ramp and its border.
fn emit_colorbar_strip(cfg: &Config, sink: &mut dyn Sink) {
    let Some((bar, rect)) = visible_colorbar_rect(cfg) else {
        return;
    };
    sink.ramp(Slot::Above, rect, bar);
    if bar.border_width > 0.0 {
        sink.rect(
            Slot::Above,
            rect,
            None,
            Some(Border {
                color: bar.border_color,
                width: bar.border_width,
                tag: "colorbar_strip",
            }),
        );
    }
}

/// The colourbar title, anchored on the painted strip so shortening,
/// aligning, dragging or resizing the bar moves the title with it.
fn emit_colorbar_title(cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) {
    let Some((bar, rect)) = visible_colorbar_rect(cfg) else {
        return;
    };
    let to = &bar.axis.title_option;
    if !to.visible || to.text.segments.is_empty() {
        return;
    }
    let m = measure.measure_rich(&to.text);
    let reach = crate::chrome::colorbar_labels_reach(&bar.axis, bar.side.clone(), &rect, measure);
    let placement = colorbar_title_placement(
        bar.side.clone(),
        &rect,
        &bar.axis,
        (to.offset_x, to.offset_y),
        m,
        reach,
    );
    sink.text(
        Slot::Above,
        &PlacedText {
            text: TextRef::Rich(&to.text),
            origin: placement.origin,
            extents: m,
            rotation: (placement.rotation_degrees != 0.0)
                .then_some((placement.rotation_degrees, placement.rotation_center)),
        },
    );
}

/// The data area: a rectangle that paints nothing of its own.
fn emit_data_area(cfg: &Config, sink: &mut dyn Sink) {
    if let Ok(da) = cfg.data_area() {
        sink.rect(Slot::Above, RectF::from_rect(&da.0), None, None);
    }
}

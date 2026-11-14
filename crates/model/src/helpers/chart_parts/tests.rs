use super::*;
use crate::annotation::{ArrowStyle, ShapeStyle};
use crate::layer::AnnotationLayer;

const RED: Color = Color::from_rgb8(200, 30, 30);

fn layer(items: Vec<Annotation>) -> AnnotationLayer {
    let mut layer = AnnotationLayer::empty();
    layer.items = items;
    layer
}

// The alpha changes and the channels stay; it is usable in a const.
#[test]
fn with_alpha_keeps_the_channels() {
    const FADED: Color = RED.with_alpha(0.25);
    assert_eq!(FADED, Color::new(RED.r, RED.g, RED.b, 0.25));
    assert_eq!(FADED.with_alpha(1.0), RED);
}

// A reference line is solid and unlabelled; the modifiers add a label and a
// dash, and the item validates in a document.
#[test]
fn reference_line_and_its_modifiers() {
    let line = Annotation::reference_line(AnnotationAxis::Y, 2.5, RED, 1.5);
    assert!(line.visible && line.id.is_empty());
    assert_eq!(
        line.kind,
        AnnotationKind::ReferenceLine {
            axis: AnnotationAxis::Y,
            value: 2.5,
            style: AnnotationLineStyle {
                color: RED,
                width: 1.5,
                style: LineStylePreset::Solid,
            },
            label: None,
        }
    );
    let label = RichText::plain("limit", RED, 16.0, "");
    let dashed = line
        .with_label(label.clone())
        .with_line_style(LineStylePreset::Dash);
    let AnnotationKind::ReferenceLine {
        style, label: set, ..
    } = &dashed.kind
    else {
        panic!("a reference line, not {:?}", dashed.kind);
    };
    assert_eq!(style.style, LineStylePreset::Dash);
    assert_eq!(set.as_ref(), Some(&label));

    let mut config = crate::defaults::config();
    config.annotations.items.push(dashed);
    assert!(config.validate().is_ok(), "{:?}", config.validate());

    let too_wide = Annotation::reference_line(AnnotationAxis::X, 0.0, RED, 100.0);
    assert_eq!(
        layer(vec![too_wide]).validate(),
        Err(crate::RuleKind::AnnotationLineWidth)
    );
}

// A span is the band between its two values in its fill.
#[test]
fn span_is_a_filled_band() {
    let fill = RED.with_alpha(0.2);
    let span = Annotation::span(AnnotationAxis::X, 1.0, 3.0, fill);
    assert_eq!(
        span.kind,
        AnnotationKind::Span {
            axis: AnnotationAxis::X,
            from: 1.0,
            to: 3.0,
            fill,
        }
    );
    assert!(span.is_underlay());
    // A label or a dash does not apply to a span.
    let unchanged = span
        .clone()
        .with_label(RichText::plain("x", RED, 12.0, ""))
        .with_line_style(LineStylePreset::Dot);
    assert_eq!(unchanged, span);
    assert!(layer(vec![span]).validate().is_ok());
}

// A text block is plain text at its anchor, top-left aligned and unboxed;
// the modifiers frame it and move its alignment point.
#[test]
fn text_and_its_modifiers() {
    let anchor = Anchor::Data { x: 1.0, y: 2.0 };
    let note = Annotation::text(anchor, "peak", RED, 18.0);
    assert_eq!(
        note.kind,
        AnnotationKind::Text {
            anchor,
            text: RichText::plain("peak", RED, 18.0, ""),
            box_style: None,
            align: NoteAlign::TopLeft,
        }
    );
    let tex = RichText::from_tex(r"\alpha^2", RED, 20.0, "").unwrap();
    let rich = Annotation::text_rich(Anchor::Ratio { x: 0.5, y: 0.5 }, tex.clone());
    assert!(matches!(&rich.kind, AnnotationKind::Text { text, .. } if *text == tex));

    let frame = crate::defaults::box_style();
    let boxed = note
        .with_box(frame.clone())
        .with_align(NoteAlign::BottomRight);
    let AnnotationKind::Text {
        box_style, align, ..
    } = &boxed.kind
    else {
        panic!("a text block, not {:?}", boxed.kind);
    };
    assert_eq!(box_style.as_ref(), Some(&frame));
    assert_eq!(*align, NoteAlign::BottomRight);
    assert!(layer(vec![boxed, rich]).validate().is_ok());

    // A box and an alignment apply to a text block only.
    let line = Annotation::reference_line(AnnotationAxis::X, 0.0, RED, 1.0);
    assert_eq!(
        line.clone()
            .with_box(crate::defaults::box_style())
            .with_align(NoteAlign::Center),
        line
    );
}

// The label and dash modifiers reach an arrow and a shape's border.
#[test]
fn modifiers_reach_arrows_and_shapes() {
    let from = Anchor::Ratio { x: 0.1, y: 0.1 };
    let to = Anchor::Ratio { x: 0.4, y: 0.4 };
    let arrow = Annotation::new(AnnotationKind::Arrow {
        from,
        to,
        style: ArrowStyle::default(),
        text: None,
    })
    .with_label(RichText::plain("here", RED, 14.0, ""))
    .with_line_style(LineStylePreset::Dot);
    let AnnotationKind::Arrow { style, text, .. } = &arrow.kind else {
        panic!("an arrow");
    };
    assert_eq!(style.line.style, LineStylePreset::Dot);
    assert!(text.is_some());

    let rect = Annotation::new(AnnotationKind::Rect {
        from,
        to,
        style: ShapeStyle::default(),
    })
    .with_line_style(LineStylePreset::Dash);
    assert_eq!(
        rect.kind.shape_style().map(|s| s.border.style),
        Some(LineStylePreset::Dash)
    );
    assert!(layer(vec![arrow, rect]).validate().is_ok());
}

// The overlay axis is the default y axis with a shown title and labels in
// the series colour; everything else stays the default.
#[test]
fn overlay_y_axis_is_titled_in_its_colour() {
    let axis = AxisOptions::overlay_y("Current (A)", RED);
    let base = crate::defaults::axis_options_y();
    assert!(axis.title_option.visible);
    let text: String = axis
        .title_option
        .text
        .segments
        .iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(text, "Current (A)");
    assert_eq!(axis.title_option.text.color, RED);
    assert_eq!(
        axis.title_option.text.font_size,
        base.title_option.text.font_size
    );
    assert_eq!(axis.label_style.color, RED);

    let mut restored = axis.clone();
    restored.title_option = base.title_option.clone();
    restored.label_style.color = base.label_style.color;
    assert_eq!(restored, base);

    let mut config = crate::defaults::config();
    let mut overlay = crate::layer::OverlayLayer::default();
    overlay.y_axis = axis;
    overlay.series = vec!["current".into()];
    config.overlays.push(overlay);
    assert!(config.validate().is_ok(), "{:?}", config.validate());
}

// `unit` stores the unit and chooses the spacing for the shown range, as
// the range helpers do (#273); the rest of the axis is untouched.
#[test]
fn unit_prints_the_axis_in_another_unit() {
    let mut counts = crate::defaults::axis_options_y();
    counts.min = 0.0;
    counts.max = 32768.0;
    counts.major_spacing = 5000.0;
    let volts = counts.clone().unit(5.0 / 32768.0, 0.0);
    assert_eq!(volts.unit, Some(AxisUnit::new(5.0 / 32768.0, 0.0)));
    // 0..5 V: one volt per major, read in shown units.
    assert_eq!(volts.major_spacing, 1.0);
    let mut restored = volts.clone();
    restored.unit = None;
    restored.major_spacing = counts.major_spacing;
    assert_eq!(restored, counts);

    // A log axis plans decades of the shown value.
    let mut log = counts.clone();
    log.scale = AxisScale::Logarithmic;
    log.min = 1.0;
    log.max = 1e12;
    let log = log.unit(1e-3, 0.0);
    assert_eq!(
        log.major_spacing,
        crate::tick::axis_major_spacing(&AxisScale::Logarithmic, 1e-3, 1e9)
    );

    // A unit the axis refuses is kept for validation to name; the spacing
    // stays as it was.
    let mut categorical = counts.clone();
    categorical.categories = Some(vec![crate::config::Category::new("a", "A")]);
    let refused = categorical.clone().unit(2.0, 0.0);
    assert_eq!(refused.major_spacing, categorical.major_spacing);
    assert_eq!(
        refused.validate_unit(),
        Err(crate::RuleKind::AxisUnitCategorical)
    );

    // An overlay axis in volts validates in a chart.
    let mut config = crate::defaults::config();
    let mut overlay = crate::layer::OverlayLayer::default();
    overlay.y_axis = AxisOptions::overlay_y("Voltage (V)", RED).unit(5.0 / 32768.0, 0.0);
    overlay.series = vec!["adc".into()];
    config.overlays.push(overlay);
    assert_eq!(config.validate_chart(), Ok(()));
}

// A subplot holds its chart and series, links nothing until asked, and
// validates as a document's subplot.
#[test]
fn subplot_new_and_links() {
    let mut cell = crate::defaults::config();
    cell.chart_area.0.x = 600;
    let subplot = Subplot::new(cell.clone(), &["a", "b"]);
    assert_eq!(subplot.config, cell);
    assert_eq!(subplot.series, ["a", "b"]);
    assert_eq!((subplot.link_x, subplot.link_y), (None, None));
    assert!(subplot.marker.is_none());

    let linked = subplot
        .with_link_x(CellRef::Parent)
        .with_link_y(CellRef::Parent);
    assert_eq!(linked.link_x, Some(CellRef::Parent));
    assert_eq!(linked.link_y, Some(CellRef::Parent));
    assert_eq!(linked.text_scale, crate::SubplotTextScale::Auto);
    let fixed = linked
        .clone()
        .with_text_scale(crate::SubplotTextScale::Fixed);
    assert_eq!(fixed.text_scale, crate::SubplotTextScale::Fixed);

    let mut config = crate::defaults::config();
    config.subplots.push(linked);
    assert!(config.validate().is_ok(), "{:?}", config.validate());
}

// The model's text wrappers read their text as a label (#272): `$…$` spans
// are TeX, the segments `RichText::from_tex` gives; a string without math
// is the plain text it was; text whose TeX does not parse is shown as
// written. The SSOT constructors stay explicit.
#[test]
fn the_text_wrappers_read_dollar_spans_as_tex() {
    let tex = r"$\hat y \pm 1.96\sigma$";
    let math = crate::text::tex::rich_segments_from_tex(tex).unwrap();
    assert!(math.iter().any(|s| s.math_node.is_some()));

    let anchor = Anchor::Ratio { x: 0.5, y: 0.5 };
    let note = Annotation::text(anchor, tex, RED, 14.0);
    assert!(matches!(&note.kind, AnnotationKind::Text { text, .. } if text.segments == math));
    let axis = AxisOptions::overlay_y(tex, RED);
    assert_eq!(axis.title_option.text.segments, math);
    let series = crate::data_config::SeriesConfig::line("fit", "x", "y").label(tex);
    assert_eq!(series.label.as_ref().map(|l| &l.segments), Some(&math));
    let radial = crate::defaults::radial_chart().with_title(tex);
    assert_eq!(
        radial.chart_title.as_ref().map(|t| &t.text.segments),
        Some(&math)
    );
    let mut config = crate::defaults::config();
    config.annotations.items.push(note);
    let mut overlay = crate::defaults::overlay_layer();
    overlay.y_axis = axis;
    overlay.series = vec!["fit".into()];
    config.overlays.push(overlay);
    assert!(config.validate().is_ok(), "{:?}", config.validate());

    // Plain text, an escaped dollar and TeX that does not parse.
    let plain = crate::text::rich_segments_from_text;
    let note = Annotation::text(anchor, "peak", RED, 14.0);
    assert!(
        matches!(&note.kind, AnnotationKind::Text { text, .. } if text.segments == plain("peak"))
    );
    let series = crate::data_config::SeriesConfig::line("a", "x", "y").label(r"cost (\$)");
    assert_eq!(series.label.map(|l| l.segments), Some(plain("cost ($)")));
    let radial = crate::defaults::radial_chart().with_title(r"$\frac{$");
    assert_eq!(
        radial.chart_title.map(|t| t.text.segments),
        Some(plain(r"$\frac{$"))
    );
}

//! Render first, edit exactly one SSOT property, then inspect the FIRST updated
//! frame and the work needed to produce it. The pixel oracle is a fresh renderer
//! with the final state, never an invalidation predicate from the implementation.
use super::*;
use crate::annotation::{Anchor, Annotation, AnnotationAxis, AnnotationKind};
use crate::config::{AxisScale, DrawStyle, TickVisibility};
use crate::gpu_memory::GpuResourceKind;
use crate::layout::Side;
use crate::text::RichText;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Work {
    Prefix,
    Packed,
    Replay,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pixels {
    /// The first updated frame must differ from the frame before the edit.
    Changed,
    /// The first updated frame must be byte-identical to the frame before.
    Unchanged,
    /// Either; the edit's visible effect depends on the host's font set.
    Any,
}
/// One single-property edit and the work it is allowed to cause. `work` is the
/// data-side classification (and the `view` revision: everything but `Prefix`
/// changes the data transform or area); `raster` is whether the CPU axis
/// chrome must be rasterized again (the `raster` revision); `annotation` is
/// whether the annotation layer's own rasters must be redone (the
/// `annotation` revision, `annotations.items` only); `differs` is whether the
/// config changed at all (the `desired` and `config` revisions).
struct Edit {
    name: &'static str,
    apply: fn(&mut Config),
    work: Work,
    differs: bool,
    raster: bool,
    annotation: bool,
    pixels: Pixels,
    /// Whether the nonresident stream pass accepts the edited chart at all.
    /// A rejected combination is a contract of its own and is not a
    /// first-frame case there.
    streams: bool,
}

fn edits() -> Vec<Edit> {
    let mut cases = Vec::new();
    macro_rules! edit {
        ($path:ident $(.$field:ident)*, $value:expr, $work:ident) => {
            cases.push(Edit { name: stringify!($path $(.$field)*),
                apply: |c| c.$path $(.$field)* = $value,
                work: Work::$work, differs: true, raster: true, annotation: false,
                pixels: Pixels::Changed, streams: true });
        };
    }
    cases.push(Edit {
        name: "identical config",
        apply: |_| {},
        work: Work::Prefix,
        differs: false,
        raster: false,
        annotation: false,
        pixels: Pixels::Unchanged,
        streams: true,
    });
    // Layer foundation: each field is an edit of its own layer only. Pixels,
    // the axis raster, data work and the data job must stay exactly as they were.
    cases.push(Edit {
        name: "active_layer",
        apply: |c| c.active_layer = crate::LayerRef::Annotations,
        work: Work::Prefix,
        differs: true,
        raster: false,
        annotation: false,
        pixels: Pixels::Unchanged,
        streams: true,
    });
    cases.push(Edit {
        name: "annotations.visible",
        apply: |c| c.annotations.visible = false,
        work: Work::Prefix,
        differs: true,
        raster: false,
        annotation: false,
        pixels: Pixels::Unchanged,
        streams: true,
    });
    // The family lookup order is registered, installed, then bundled, so
    // naming the bundled face explicitly may or may not pick a different
    // installed copy on a given host; the raster still has to be redone.
    cases.push(Edit {
        name: "bottom_x.label_style.label_font",
        apply: |c| c.bottom_x.label_style.label_font = "Liberation Sans".into(),
        work: Work::Prefix,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Any,
        streams: true,
    });
    edit!(bottom_x.label_style.significant_digits, 5, Prefix);
    edit!(
        bottom_x.label_style.format,
        crate::format::LabelFormat::Plain(crate::format::PlainNumberFormat {
            decimals: Some(1),
            trim_zeros: true,
        }),
        Prefix
    );
    macro_rules! grid {
        ($show:ident, $color:ident, $width:ident, $style:ident) => {
            edit!(grid.$show, false, Prefix);
            edit!(grid.$color, Color::new(0.0, 0.8, 0.1, 1.0), Prefix);
            edit!(grid.$width, 3.0, Prefix);
            edit!(grid.$style, LineStylePreset::Dash, Prefix);
        };
    }
    grid!(show_major_x, major_x_color, major_x_width, major_x_style);
    grid!(show_major_y, major_y_color, major_y_width, major_y_style);
    grid!(show_minor_x, minor_x_color, minor_x_width, minor_x_style);
    grid!(show_minor_y, minor_y_color, minor_y_width, minor_y_style);
    macro_rules! axis {
        ($axis:ident) => {
            edit!($axis.major_spacing, 0.3, Prefix);
            edit!($axis.minor_count, 3, Prefix);
            edit!($axis.line_visible, false, Prefix);
            edit!($axis.line_color, Color::new(0.1, 0.8, 0.2, 1.0), Prefix);
            edit!($axis.line_width, 3.0, Prefix);
            edit!($axis.line_style, LineStylePreset::Dash, Prefix);
            edit!($axis.line_offset, 4.0, Prefix);
            edit!($axis.tick, TickVisibility::None, Prefix);
            edit!($axis.minor_tick_length, 6.0, Prefix);
            edit!($axis.label_style.label_visible, false, Prefix);
            edit!(
                $axis.label_style.color,
                Color::new(0.0, 0.5, 0.9, 1.0),
                Prefix
            );
            edit!($axis.label_style.font_size, 12.0, Prefix);
            edit!($axis.label_style.label_offset_x, 4.0, Prefix);
            edit!($axis.label_style.label_offset_y, 4.0, Prefix);
            edit!($axis.label_style.visible, false, Prefix);
            edit!(
                $axis.label_style.format,
                crate::format::LabelFormat::Scientific,
                Prefix
            );
            edit!($axis.title_option.visible, false, Prefix);
            edit!(
                $axis.title_option.text,
                RichText::plain("New", Color::BLACK, 10.0, ""),
                Prefix
            );
            edit!(
                $axis.title_option.text.color,
                Color::new(0.8, 0.0, 0.8, 1.0),
                Prefix
            );
            edit!($axis.title_option.text.font_size, 12.0, Prefix);
            edit!($axis.title_option.offset_x, 4.0, Prefix);
            edit!($axis.title_option.offset_y, 4.0, Prefix);
            edit!($axis.out_margin, 42.0, Packed);
            edit!($axis.major_tick_length, 8.0, Packed);
        };
    }
    axis!(bottom_x);
    axis!(left_y);
    axis!(top_x);
    axis!(right_y);
    edit!(bottom_x.inverted, true, Replay);
    edit!(left_y.inverted, true, Replay);
    edit!(top_x.inverted, true, Prefix);
    edit!(right_y.inverted, true, Prefix);
    edit!(top_x.min, 0.2, Prefix);
    edit!(top_x.max, 2.0, Prefix);
    edit!(top_x.scale, AxisScale::Logarithmic, Prefix);
    edit!(right_y.min, 0.2, Prefix);
    edit!(right_y.max, 2.0, Prefix);
    edit!(right_y.scale, AxisScale::Logarithmic, Prefix);
    edit!(bottom_x.min, 0.2, Packed);
    edit!(bottom_x.max, 0.8, Packed);
    edit!(left_y.min, 0.2, Packed);
    edit!(left_y.max, 0.8, Packed);
    edit!(bottom_x.max, 2.0, Replay);
    edit!(left_y.max, 2.0, Replay);
    edit!(bottom_x.scale, AxisScale::Logarithmic, Replay);
    edit!(left_y.scale, AxisScale::Logarithmic, Replay);
    edit!(chart_title.visible, false, Prefix);
    edit!(
        chart_title.text,
        RichText::plain("Changed title", Color::BLACK, 12.0, ""),
        Prefix
    );
    edit!(
        chart_title.text.color,
        Color::new(0.1, 0.6, 0.9, 1.0),
        Prefix
    );
    edit!(chart_title.text.font_size, 16.0, Prefix);
    edit!(chart_title.offset_x, 8.0, Prefix);
    edit!(chart_title.offset_y, 4.0, Prefix);
    edit!(chart_title.top_margin, 30.0, Packed);
    edit!(legend.visible, false, Prefix);
    edit!(legend.offset_x, 8.0, Prefix);
    edit!(legend.offset_y, 8.0, Prefix);
    edit!(legend.padding, 14.0, Prefix);
    edit!(legend.bg_color, Color::new(0.5, 0.8, 0.5, 1.0), Prefix);
    edit!(legend.border_color, Color::new(0.9, 0.1, 0.8, 1.0), Prefix);
    edit!(
        legend.content,
        RichText::plain("Changed legend", Color::BLACK, 10.0, ""),
        Prefix
    );
    edit!(legend.content.color, Color::new(0.9, 0.1, 0.8, 1.0), Prefix);
    edit!(
        legend.corner,
        crate::legend::LegendCorner::BottomRight,
        Prefix
    );
    // ChartArea is a tuple struct, so use explicit single-field edits.
    for (name, apply) in [
        (
            "chart_area.x",
            (|c: &mut Config| c.chart_area.0.x = 8) as fn(&mut Config),
        ),
        ("chart_area.y", |c| c.chart_area.0.y = 8),
        ("chart_area.width", |c| c.chart_area.0.width = 300),
        ("chart_area.height", |c| c.chart_area.0.height = 220),
    ] {
        cases.push(Edit {
            name,
            apply,
            work: Work::Packed,
            differs: true,
            raster: true,
            annotation: false,
            pixels: Pixels::Changed,
            streams: true,
        });
    }
    edit!(draw_style, DrawStyle::Sketch(Default::default()), Replay);
    // The stream pass rejects Milkyway line series outright (arc-driven star
    // chains, `validate_stream_series`), and draws no primitive at all for a
    // Line series in the constellation style (`constellation_supports`), so
    // neither style has a stream first frame to compare: resident-only cases.
    cases.push(Edit {
        name: "draw_style (Milkyway)",
        apply: |c| c.draw_style = DrawStyle::Milkyway(Default::default()),
        work: Work::Replay,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: false,
    });
    // The polar coordinate system moves every sample and redraws the grid
    // as rings and spokes; the stream pass rejects polar charts, so there is
    // no stream first frame to compare.
    cases.push(Edit {
        name: "polar (None -> Some)",
        apply: |c| c.polar = Some(crate::polar::PolarOptions::default()),
        work: Work::Replay,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: false,
    });
    cases.push(Edit {
        name: "draw_style (Constellation)",
        apply: |c| c.draw_style = DrawStyle::Constellation(Default::default()),
        work: Work::Replay,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: false,
    });
    // Adding a colourbar reserves its band, so the data area moves.
    cases.push(Edit {
        name: "colorbar (None -> Some)",
        apply: |c| c.colorbar = Some(crate::default::default_colorbar_options()),
        work: Work::Packed,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: true,
    });
    cases
}

/// Colourbar edits run against a baseline that already has one. The band
/// (`gap_px + thickness_px + axis.out_margin + axis.major_tick_length` on its
/// side, when visible) is part of the data area, so those fields are geometry
/// edits; everything else on the bar is axis chrome.
fn colorbar_edits() -> Vec<Edit> {
    let mut cases = Vec::new();
    macro_rules! bar {
        ($($field:ident).+, $value:expr, $work:ident, $pixels:ident) => {
            cases.push(Edit {
                name: concat!("colorbar.", stringify!($($field).+)),
                apply: |c| c.colorbar.as_mut().unwrap().$($field).+ = $value,
                work: Work::$work,
                differs: true,
                raster: true,
                annotation: false,
                pixels: Pixels::$pixels,
                streams: true,
            });
        };
    }
    cases.push(Edit {
        name: "colorbar (Some -> None)",
        apply: |c| c.colorbar = None,
        work: Work::Packed,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: true,
    });
    bar!(visible, false, Packed, Changed);
    bar!(side, Side::Left, Packed, Changed);
    bar!(thickness_px, 30.0, Packed, Changed);
    bar!(gap_px, 10.0, Packed, Changed);
    bar!(axis.out_margin, 42.0, Packed, Changed);
    bar!(axis.major_tick_length, 8.0, Packed, Changed);
    bar!(length_frac, 0.5, Prefix, Changed);
    bar!(align, crate::config::BarAlign::Start, Prefix, Changed);
    bar!(offset_x, 4.0, Prefix, Changed);
    bar!(offset_y, 4.0, Prefix, Changed);
    bar!(colormap, crate::ColorMap::Magma, Prefix, Changed);
    // Only field cells wear `nan_color`; an XY chart's strip never shows it.
    bar!(nan_color, Color::new(1.0, 0.0, 1.0, 1.0), Prefix, Unchanged);
    bar!(
        border_color,
        Color::new(0.9, 0.1, 0.1, 1.0),
        Prefix,
        Changed
    );
    bar!(border_width, 3.0, Prefix, Changed);
    bar!(axis.min, 0.2, Prefix, Changed);
    bar!(axis.max, 2.0, Prefix, Changed);
    bar!(axis.scale, AxisScale::Logarithmic, Prefix, Changed);
    bar!(axis.inverted, true, Prefix, Changed);
    bar!(axis.major_spacing, 0.3, Prefix, Changed);
    bar!(axis.minor_count, 1, Prefix, Any);
    bar!(axis.tick, TickVisibility::None, Prefix, Any);
    bar!(axis.minor_tick_length, 6.0, Prefix, Any);
    bar!(axis.line_visible, true, Prefix, Changed);
    bar!(axis.line_color, Color::new(0.1, 0.8, 0.2, 1.0), Prefix, Any);
    bar!(axis.line_width, 3.0, Prefix, Any);
    bar!(axis.line_style, LineStylePreset::Dash, Prefix, Any);
    bar!(axis.line_offset, 4.0, Prefix, Any);
    bar!(axis.label_style.label_visible, false, Prefix, Changed);
    bar!(axis.label_style.visible, false, Prefix, Changed);
    bar!(
        axis.label_style.color,
        Color::new(0.0, 0.5, 0.9, 1.0),
        Prefix,
        Changed
    );
    bar!(axis.label_style.font_size, 12.0, Prefix, Changed);
    bar!(
        axis.label_style.label_font,
        "Liberation Sans".into(),
        Prefix,
        Any
    );
    bar!(axis.label_style.label_offset_x, 4.0, Prefix, Changed);
    bar!(axis.label_style.label_offset_y, 4.0, Prefix, Changed);
    bar!(
        axis.label_style.format,
        crate::format::LabelFormat::Scientific,
        Prefix,
        Changed
    );
    bar!(axis.label_style.significant_digits, 5, Prefix, Changed);
    bar!(axis.title_option.visible, true, Prefix, Changed);
    // The baseline already titles the bar "Z" (hidden), so edit to a new string.
    bar!(
        axis.title_option.text,
        RichText::plain("Zed", Color::BLACK, 10.0, ""),
        Prefix,
        Any
    );
    bar!(axis.title_option.offset_x, 4.0, Prefix, Any);
    bar!(axis.title_option.offset_y, 4.0, Prefix, Any);
    cases
}

/// Overlay edits run against a baseline whose one series is scaled by an
/// overlay layer. The overlay's Y range, scale, direction and series list are
/// geometry of its own panel; its band (`out_margin + major_tick_length`)
/// widens the shared right margin, so those are data-area edits; the rest is
/// chrome of the overlay panel. None of these documents stream: the stream
/// pass rejects overlays as a contract of its own.
fn overlay_edits() -> Vec<Edit> {
    let mut cases = Vec::new();
    macro_rules! overlay {
        ($($field:ident).+, $value:expr, $work:ident, $pixels:ident) => {
            cases.push(Edit {
                name: concat!("overlays[0].", stringify!($($field).+)),
                apply: |c| c.overlays[0].$($field).+ = $value,
                work: Work::$work,
                differs: true,
                raster: true,
                annotation: false,
                pixels: Pixels::$pixels,
                streams: false,
            });
        };
    }
    cases.push(Edit {
        name: "overlays (one -> none)",
        apply: |c| c.overlays.clear(),
        work: Work::Packed,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: false,
    });
    cases.push(Edit {
        name: "overlays (one -> two)",
        apply: |c| {
            let mut overlay = crate::OverlayLayer::default();
            overlay.y_axis.out_margin = 60.0;
            overlay.y_axis.line_offset = 40.0;
            c.overlays.push(overlay);
        },
        work: Work::Packed,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: false,
    });
    cases.push(Edit {
        name: "active_layer (overlay)",
        apply: |c| c.active_layer = crate::LayerRef::Overlay(0),
        work: Work::Prefix,
        differs: true,
        raster: false,
        annotation: false,
        pixels: Pixels::Unchanged,
        streams: false,
    });
    overlay!(series, Vec::new(), Packed, Changed);
    overlay!(y_axis.min, -2.0, Packed, Changed);
    overlay!(y_axis.max, 4.0, Packed, Changed);
    overlay!(y_axis.scale, AxisScale::Logarithmic, Packed, Changed);
    overlay!(y_axis.inverted, true, Packed, Changed);
    overlay!(y_axis.out_margin, 70.0, Packed, Changed);
    overlay!(y_axis.major_tick_length, 12.0, Packed, Changed);
    overlay!(y_axis.major_spacing, 0.25, Prefix, Changed);
    overlay!(y_axis.minor_count, 3, Prefix, Changed);
    overlay!(y_axis.tick, TickVisibility::None, Prefix, Changed);
    overlay!(y_axis.minor_tick_length, 6.0, Prefix, Changed);
    overlay!(y_axis.line_visible, false, Prefix, Changed);
    overlay!(
        y_axis.line_color,
        Color::new(0.1, 0.8, 0.2, 1.0),
        Prefix,
        Changed
    );
    overlay!(y_axis.line_width, 3.0, Prefix, Changed);
    // The overlay axis line sits on the parent's own right axis line, which
    // the SSOT keeps drawing underneath: a dash pattern shows that solid line
    // through its gaps, so the composite may come out identical.
    overlay!(y_axis.line_style, LineStylePreset::Dash, Prefix, Any);
    overlay!(y_axis.line_offset, 6.0, Prefix, Changed);
    overlay!(y_axis.label_style.label_visible, false, Prefix, Changed);
    overlay!(y_axis.label_style.visible, false, Prefix, Changed);
    overlay!(
        y_axis.label_style.color,
        Color::new(0.0, 0.5, 0.9, 1.0),
        Prefix,
        Changed
    );
    overlay!(y_axis.label_style.font_size, 13.0, Prefix, Changed);
    overlay!(
        y_axis.label_style.label_font,
        "Liberation Sans".into(),
        Prefix,
        Any
    );
    overlay!(y_axis.label_style.label_offset_x, 4.0, Prefix, Changed);
    overlay!(y_axis.label_style.label_offset_y, 4.0, Prefix, Changed);
    overlay!(
        y_axis.label_style.format,
        crate::format::LabelFormat::Scientific,
        Prefix,
        Changed
    );
    overlay!(y_axis.label_style.significant_digits, 5, Prefix, Changed);
    overlay!(y_axis.title_option.visible, false, Prefix, Changed);
    overlay!(
        y_axis.title_option.text,
        RichText::plain("Pressure", Color::BLACK, 10.0, ""),
        Prefix,
        Changed
    );
    overlay!(y_axis.title_option.offset_x, 4.0, Prefix, Changed);
    overlay!(y_axis.title_option.offset_y, 4.0, Prefix, Changed);
    cases.push(Edit {
        name: "overlays[0].legend (None -> Some)",
        apply: |c| {
            let mut legend = crate::default::default_legend();
            legend.visible = true;
            legend.content = RichText::plain("Overlay", Color::BLACK, 10.0, "");
            c.overlays[0].legend = Some(legend);
        },
        work: Work::Prefix,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: false,
    });
    cases
}

/// Subplot edits run against a baseline whose one series is drawn by a
/// subplot in the lower half of the surface, with its own axes. The
/// subplot's ranges, area, series list and links are geometry of its own
/// panel; its chrome is not. None of these documents stream: the stream
/// pass rejects subplots as a contract of its own.
fn subplot_edits() -> Vec<Edit> {
    let mut cases = Vec::new();
    macro_rules! subplot {
        ($name:expr, $apply:expr, $work:ident, $raster:expr, $pixels:ident) => {
            cases.push(Edit {
                name: $name,
                apply: $apply,
                work: Work::$work,
                differs: true,
                raster: $raster,
                annotation: false,
                pixels: Pixels::$pixels,
                streams: false,
            });
        };
    }
    subplot!(
        "subplots (one -> none)",
        |c| c.subplots.clear(),
        Packed,
        true,
        Changed
    );
    subplot!(
        "active_layer (subplot)",
        |c| c.active_layer = crate::LayerRef::Subplot(0),
        Prefix,
        false,
        Unchanged
    );
    subplot!(
        "subplots[0].series",
        |c| c.subplots[0].series.clear(),
        Packed,
        true,
        Changed
    );
    subplot!(
        "subplots[0].link_y (None -> Parent)",
        |c| c.subplots[0].link_y = Some(crate::CellRef::Parent),
        Packed,
        true,
        Changed
    );
    subplot!(
        "subplots[0].config.chart_area.height",
        |c| c.subplots[0].config.chart_area.0.height = 110,
        Packed,
        true,
        Changed
    );
    subplot!(
        "subplots[0].config.left_y.max",
        |c| c.subplots[0].config.left_y.max = 3.0,
        Packed,
        true,
        Changed
    );
    subplot!(
        "subplots[0].config.bottom_x.min",
        |c| c.subplots[0].config.bottom_x.min = -0.5,
        Packed,
        true,
        Changed
    );
    subplot!(
        "subplots[0].config.left_y.out_margin",
        |c| c.subplots[0].config.left_y.out_margin = 40.0,
        Packed,
        true,
        Changed
    );
    subplot!(
        "subplots[0].config.left_y.line_color",
        |c| c.subplots[0].config.left_y.line_color = Color::new(0.1, 0.8, 0.2, 1.0),
        Prefix,
        true,
        Changed
    );
    subplot!(
        "subplots[0].config.grid.show_major_x",
        |c| c.subplots[0].config.grid.show_major_x = false,
        Prefix,
        true,
        Changed
    );
    subplot!(
        "subplots[0].config.chart_title.text",
        |c| {
            c.subplots[0].config.chart_title.text = RichText::plain("Lower", Color::BLACK, 10.0, "")
        },
        Prefix,
        true,
        Changed
    );
    cases
}

fn baseline() -> Config {
    let mut c = crate::default::default_config();
    c.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    c.chart_title.text = RichText::plain("SSOT baseline", Color::BLACK, 12.0, "");
    c.chart_title.top_margin = 20.0;
    c.legend.visible = true;
    c.legend.content = RichText::plain("Curve", Color::BLACK, 10.0, "");
    for a in [&mut c.bottom_x, &mut c.left_y, &mut c.top_x, &mut c.right_y] {
        a.min = 0.1;
        a.max = 1.0;
        a.major_spacing = 0.2;
        a.minor_count = 1;
        a.out_margin = 30.0;
        a.title_option.visible = true;
        a.title_option.text = RichText::plain("Axis", Color::BLACK, 10.0, "");
        a.label_style.label_visible = true;
        a.label_style.font_size = 10.0;
    }
    c.grid.show_major_x = true;
    c.grid.show_major_y = true;
    c.grid.show_minor_x = true;
    c.grid.show_minor_y = true;
    c
}

/// The baseline in the upper half of the surface and a subplot in the lower
/// half drawing its one series, with its own axes, title and grid.
fn baseline_with_subplot() -> Config {
    let mut c = baseline();
    c.chart_area.0.height = 120;
    c.chart_title.top_margin = 14.0;
    for a in [&mut c.bottom_x, &mut c.left_y, &mut c.top_x, &mut c.right_y] {
        a.out_margin = 18.0;
        a.title_option.visible = false;
    }
    let mut chart = c.clone();
    chart.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 120,
        width: 320,
        height: 120,
    });
    chart.chart_title.text = RichText::plain("Below", Color::BLACK, 10.0, "");
    chart.legend.visible = false;
    chart.left_y.min = 0.0;
    chart.left_y.max = 2.0;
    chart.left_y.major_spacing = 0.5;
    c.subplots.push(crate::Subplot {
        config: chart,
        series: vec!["s".into()],
        link_x: None,
        link_y: None,
    });
    c
}

/// Annotation edits run against a baseline that already has one item of each
/// kind, so a `visible` toggle has pixels to hide. Items are the only field
/// of the layer that re-rasterizes it, and they never touch the axis raster,
/// the data transform or the data job; `visible` only skips the composite.
fn annotation_edits() -> Vec<Edit> {
    fn case(name: &'static str, apply: fn(&mut Config), annotation: bool, pixels: Pixels) -> Edit {
        Edit {
            name,
            apply,
            work: Work::Prefix,
            differs: true,
            raster: false,
            annotation,
            pixels,
            streams: true,
        }
    }
    vec![
        case(
            "annotations.visible (with items)",
            |c| c.annotations.visible = false,
            false,
            Pixels::Changed,
        ),
        case(
            "annotations.items[span].to",
            |c| match &mut c.annotations.items[0].kind {
                AnnotationKind::Span { to, .. } => *to = 1.2,
                _ => unreachable!(),
            },
            true,
            Pixels::Changed,
        ),
        case(
            "annotations.items[line].value",
            |c| match &mut c.annotations.items[1].kind {
                AnnotationKind::ReferenceLine { value, .. } => *value = 0.3,
                _ => unreachable!(),
            },
            true,
            Pixels::Changed,
        ),
        case(
            "annotations.items[text].anchor",
            |c| match &mut c.annotations.items[2].kind {
                AnnotationKind::Text { anchor, .. } => *anchor = Anchor::Ratio { x: 0.6, y: 0.6 },
                _ => unreachable!(),
            },
            true,
            Pixels::Changed,
        ),
        case(
            "annotations.items[arrow].to",
            |c| match &mut c.annotations.items[3].kind {
                AnnotationKind::Arrow { to, .. } => *to = Anchor::Data { x: 1.0, y: 0.7 },
                _ => unreachable!(),
            },
            true,
            Pixels::Changed,
        ),
        case(
            "annotations.items[line].visible",
            |c| c.annotations.items[1].visible = false,
            true,
            Pixels::Changed,
        ),
        case(
            "annotations.items (push)",
            |c| {
                c.annotations
                    .items
                    .push(Annotation::new(AnnotationKind::Span {
                        axis: AnnotationAxis::Y,
                        from: 0.25,
                        to: 0.35,
                        fill: Color::new(0.1, 0.6, 0.2, 0.4),
                    }))
            },
            true,
            Pixels::Changed,
        ),
        case(
            "annotations.items (clear)",
            |c| c.annotations.items.clear(),
            true,
            Pixels::Changed,
        ),
        // An item that cannot be placed is a warning, not a pixel: the
        // raster is redone (the items changed) and nothing is drawn for it.
        case(
            "annotations.items (push non-finite line)",
            |c| {
                c.annotations
                    .items
                    .push(Annotation::new(AnnotationKind::ReferenceLine {
                        axis: AnnotationAxis::X,
                        value: f64::NAN,
                        style: Default::default(),
                        label: None,
                    }))
            },
            true,
            Pixels::Unchanged,
        ),
    ]
}

/// The baseline plus one annotation of each kind inside the drawn ranges.
fn baseline_with_annotations() -> Config {
    let mut c = baseline();
    let text = RichText::plain("note", Color::BLACK, 11.0, "");
    c.annotations.items = vec![
        Annotation::new(AnnotationKind::Span {
            axis: AnnotationAxis::X,
            from: 0.4,
            to: 0.8,
            fill: Color::new(0.9, 0.3, 0.1, 0.35),
        }),
        Annotation::new(AnnotationKind::ReferenceLine {
            axis: AnnotationAxis::Y,
            value: 0.5,
            style: Default::default(),
            label: Some(text.clone()),
        }),
        Annotation::new(AnnotationKind::Text {
            anchor: Anchor::Ratio { x: 0.1, y: 0.1 },
            text: text.clone(),
            box_style: Some(Default::default()),
        }),
        Annotation::new(AnnotationKind::Arrow {
            from: Anchor::Data { x: 0.2, y: 0.3 },
            to: Anchor::Data { x: 1.2, y: 0.6 },
            style: Default::default(),
            text: Some(text),
        }),
    ];
    c
}

/// The baseline plus a visible colourbar whose z axis is valid in every scale.
fn baseline_with_colorbar() -> Config {
    let mut c = baseline();
    let mut bar = crate::default::default_colorbar_options();
    bar.axis.min = 0.1;
    bar.axis.max = 1.0;
    bar.axis.major_spacing = 0.2;
    bar.axis.title_option.text = RichText::plain("Z", Color::BLACK, 10.0, "");
    c.colorbar = Some(bar);
    c
}

/// The baseline with its series moved to one overlay layer: a right-hand Y
/// axis with its own positive range (so a log edit is valid), a band wider
/// than the parent's right axis, and a titled axis.
fn baseline_with_overlay() -> Config {
    let mut c = baseline();
    let mut overlay = crate::OverlayLayer::default();
    overlay.y_axis.min = 0.1;
    overlay.y_axis.max = 2.0;
    overlay.y_axis.major_spacing = 0.5;
    overlay.y_axis.minor_count = 1;
    overlay.y_axis.out_margin = 45.0;
    overlay.y_axis.label_style.font_size = 10.0;
    overlay.y_axis.title_option.visible = true;
    overlay.y_axis.title_option.text = RichText::plain("Right", Color::BLACK, 10.0, "");
    overlay.series = vec!["s".into()];
    c.overlays.push(overlay);
    c
}

fn values() -> (crate::Column<f32>, crate::Column<f32>) {
    (
        crate::Column {
            data: vec![0.05, 0.18, 0.3, 0.45, 0.6, 0.75, 0.9, 1.5],
            min: 0.05,
            max: 1.5,
        },
        crate::Column {
            data: vec![0.2, 0.7, 0.3, 0.8, 0.4, 0.7, 0.2, 0.6],
            min: 0.2,
            max: 0.8,
        },
    )
}

fn fixture(config: &Config, resident: bool, cache: bool) -> (Renderer, ChartId) {
    fixture_samples(config, resident, cache, 1)
}

fn fixture_samples(
    config: &Config,
    resident: bool,
    cache: bool,
    samples: u32,
) -> (Renderer, ChartId) {
    let (device, queue) = data_render::shared_device().unwrap();
    let mut r = Renderer::try_new_with_sample_count(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
        samples,
    )
    .unwrap();
    let (x, y) = values();
    if resident {
        r.add_column("x", &x).unwrap();
        r.add_column("a", &y).unwrap();
    } else {
        r.register_streamed_columns(vec![source("x", 1), source("a", 1)])
            .unwrap();
        r.configure_streaming_runtime(limits(2)).unwrap();
        let _ = r.set_memory_budget(Some(128 * 1024 * 1024));
        let _ = r.set_auto_resident_working_set_limit(Some(if cache { 1024 * 1024 } else { 0 }));
    }
    let id = r
        .register_chart(config.clone(), vec![declaration("s", "x", "a")])
        .unwrap();
    (r, id)
}

fn options() -> crate::StreamingChartOptions {
    crate::StreamingChartOptions {
        size: (320, 240),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 3,
    }
}

// A complete renderer frame, including its first display composition. Count
// actual source submissions, rather than status totals (which are logical rows).
fn frame(r: &mut Renderer, id: ChartId, resident: bool) -> (Vec<u8>, usize) {
    let (x, y) = values();
    frame_values(r, id, resident, &x, &y, 1)
}

fn frame_values(
    r: &mut Renderer,
    id: ChartId,
    resident: bool,
    x: &crate::Column<f32>,
    y: &crate::Column<f32>,
    revision: u64,
) -> (Vec<u8>, usize) {
    let c = r.chart_config(id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(c.clone()), c.chart_area.0)
        .unwrap();
    if resident {
        let prepared = r
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: id,
                view: &view,
            }])
            .unwrap();
        return (paint_frame_pixels(r, &prepared, 1), 0);
    }
    r.request_auto_streaming_chart(id, &view, options())
        .unwrap();
    let bindings = [
        crate::StreamSourceBinding {
            id: "x",
            revision,
            source: crate::StreamColumnSource::Scalar(x),
        },
        crate::StreamSourceBinding {
            id: "a",
            revision,
            source: crate::StreamColumnSource::Scalar(y),
        },
    ];
    let mut submissions = 0;
    for _ in 0..128 {
        let data_requested = matches!(
            r.auto_stream_chart_request_ranges(id).unwrap(),
            crate::AutoStreamingRangeRequest::Ready { .. }
        );
        submissions += usize::from(data_requested);
        match r.auto_stream_chart_step(id, &bindings).unwrap() {
            crate::AutoStreamingProgress::Submitted { .. } => {}
            crate::AutoStreamingProgress::Backpressure { .. } => wait_stream_slots(r, 0),
            crate::AutoStreamingProgress::AllSubmitted { .. }
            | crate::AutoStreamingProgress::Complete { .. } => break,
        }
    }
    // Selection is a separate asynchronous suffix; prepare exactly once after
    // its inputs arrive, without a second draw to repair stale cached pixels.
    for _ in 0..128 {
        match r.request_stream_selection_ranges(id).unwrap() {
            crate::StreamingSelectionRequest::Complete { .. } => break,
            crate::StreamingSelectionRequest::Failed { .. } => panic!("selection failed"),
            crate::StreamingSelectionRequest::Backpressure { .. } => wait_stream_slots(r, 0),
            crate::StreamingSelectionRequest::Ready { .. } => {
                r.auto_stream_chart_step(id, &bindings).unwrap();
            }
        }
    }
    assert!(matches!(
        r.request_stream_selection_ranges(id).unwrap(),
        crate::StreamingSelectionRequest::Complete { .. }
    ));
    wait_stream_slots(r, 0);
    r.prepare_registered(&[RegisteredChartDrawItem {
        chart_id: id,
        view: &view,
    }])
    .unwrap();
    assert!(matches!(
        r.auto_stream_chart_request_ranges(id).unwrap(),
        crate::AutoStreamingRangeRequest::Complete { .. }
    ));
    let job = r.active_stream_job(id).unwrap();
    let target = wgpu::Texture::clone(r.chart_stream_display(job).unwrap());
    (read_draw_target(r, &target), submissions)
}

fn matrix(resident: bool, cache: bool) {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut failures = Vec::new();
    let suites = [
        (baseline(), edits()),
        (baseline_with_colorbar(), colorbar_edits()),
        (baseline_with_overlay(), overlay_edits()),
        (baseline_with_subplot(), subplot_edits()),
        (baseline_with_annotations(), annotation_edits()),
    ];
    eprintln!(
        "SSOT: {} single-property cases, resident={resident}, cache={cache}",
        suites.iter().map(|(_, cases)| cases.len()).sum::<usize>()
    );
    for (c, edit) in suites
        .into_iter()
        .flat_map(|(c, cases)| cases.into_iter().map(move |edit| (c.clone(), edit)))
    {
        if !resident && !edit.streams {
            continue;
        }
        let (mut r, id) = fixture(&c, resident, cache);
        let (before, _) = frame(&mut r, id, resident);
        let job = r.active_stream_job(id);
        let prefix = job.map(|j| r.chart_stream_prefix_for_test(j).unwrap());
        let prefix_pixels = prefix.as_ref().map(|t| read_draw_target(&r, t));
        let memory = r.gpu_memory_usage();
        let style_buffer = resident.then(|| {
            r.chart_states[&id].prepared_styles.as_ref().unwrap().styles[0]
                .line_bg
                .clone()
        });
        let mut next = c.clone();
        (edit.apply)(&mut next);
        assert_eq!(
            next != c,
            edit.differs,
            "{} must edit one property",
            edit.name
        );
        let revisions = r.chart_states[&id].revisions;
        r.set_chart_config(id, next.clone()).unwrap();
        let updated = r.chart_states[&id].revisions;
        let (actual, reads) = frame(&mut r, id, resident);
        let (mut fresh, fresh_id) = fixture(&next, resident, false);
        let (expected, _) = frame(&mut fresh, fresh_id, resident);
        let label = format!("{} resident={resident} cache={cache}", edit.name);
        let mut check = |ok: bool, msg: &str| {
            if !ok {
                failures.push(format!("{label}: {msg}"));
            }
        };
        // The registry's revisions must follow the classification exactly:
        // no refresh that the edit does not need, none missing that it does.
        check(
            (updated.desired != revisions.desired) == edit.differs,
            "desired revision does not follow whether the config changed",
        );
        check(
            (updated.config != revisions.config) == edit.differs,
            "config revision does not follow whether the config changed",
        );
        check(
            (updated.raster != revisions.raster) == edit.raster,
            "raster revision does not follow the chrome classification",
        );
        check(
            (updated.view != revisions.view) == (edit.work != Work::Prefix),
            "view revision does not follow the geometry classification",
        );
        check(
            updated.selection == revisions.selection,
            "a non-selection edit changed the selection revision",
        );
        check(
            (updated.annotation != revisions.annotation) == edit.annotation,
            "annotation revision does not follow the annotation item classification",
        );
        check(
            actual == expected,
            "first updated frame differs from fresh final-state render",
        );
        match edit.pixels {
            Pixels::Changed => check(actual != before, "edit did not change visible pixels"),
            Pixels::Unchanged => check(actual == before, "edit changed visible pixels"),
            Pixels::Any => {}
        }
        if let Some(style_buffer) = style_buffer {
            check(
                r.chart_states[&id].prepared_styles.as_ref().unwrap().styles[0].line_bg
                    == style_buffer,
                "config edit rebuilt unchanged resident series style buffer",
            );
        }
        if resident || edit.work == Work::Prefix || (cache && edit.work == Work::Packed) {
            check(reads == 0, "unnecessary source replay");
            for kind in [
                GpuResourceKind::ColumnPool,
                GpuResourceKind::StreamingUpload,
                GpuResourceKind::ViewResident,
            ] {
                check(
                    r.gpu_memory_usage().creations_of(kind) == memory.creations_of(kind),
                    &format!("unnecessary {kind:?} allocation"),
                );
            }
        } else {
            check(reads > 0, "required source replay was skipped");
        }
        if !resident {
            let after_job = r.active_stream_job(id).unwrap();
            if edit.work == Work::Prefix {
                check(Some(after_job) == job, "decoration restarted data job");
                let after_prefix = r.chart_stream_prefix_for_test(after_job).unwrap();
                check(
                    prefix.as_ref() == Some(&after_prefix),
                    "decoration replaced data texture",
                );
                check(
                    prefix_pixels.as_ref() == Some(&read_draw_target(&r, &after_prefix)),
                    "decoration rewrote data pixels",
                );
            } else {
                check(
                    Some(after_job) != job,
                    "geometry/style edit retained old data job",
                );
            }
        }
        // Repainting the SAME already-updated state must be stable, with no
        // further source upload or job churn (including after a real replay).
        let settled_job = r.active_stream_job(id);
        let (again, reads) = frame(&mut r, id, resident);
        check(
            again == actual,
            "second frame changed after first-frame publication",
        );
        check(reads == 0, "settled frame replayed source");
        check(
            r.active_stream_job(id) == settled_job,
            "settled frame restarted job",
        );
    }
    drop(_fonts);
    assert!(
        failures.is_empty(),
        "{} SSOT failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// Every `Config` field must be classified by the matrix above or by the
// selection test below. Destructuring without `..` makes a new field a
// compile error until it is, which is also what keeps `raster_config_equal`
// and `stream_config_equal` honest: the registry's revisions are tested
// against this same catalogue.
#[test]
fn config_fields_are_classified() {
    use crate::config::{
        AxisOptions, AxisTitleOptions, ChartTitleOptions, ColorBarOptions, GridOptions, LabelStyle,
    };
    let Config {
        chart_area: _,    // chart_area.x/y/width/height: Packed
        top_x,            // axis!(top_x) + top_x range/scale/inverted: Prefix
        bottom_x: _,      // axis!(bottom_x) + range/scale/inverted: Packed/Replay
        left_y: _,        // axis!(left_y) + range/scale/inverted: Packed/Replay
        right_y: _,       // axis!(right_y) + range/scale/inverted: Prefix
        chart_title,      // chart_title.*: Prefix, top_margin: Packed
        grid,             // grid!(...): Prefix
        legend: _,        // legend.*: Prefix (Legend fields listed inline)
        picked_points: _, // ssot_selection_edits_replace_only_the_overlay
        picked_data: _,   // selection revision, same classification as picked_points
        colorbar,         // colorbar_edits(): band fields Packed, the rest Prefix
        draw_style: _,    // Sketch / Milkyway / Constellation: Replay
        polar: _,         // the coordinate system: geometry (Packed/Replay) and raster
        overlays,         // overlay_edits(): range/series/band Packed, the rest Prefix
        subplots,         // subplot_edits(): geometry/series/links Packed, chrome Prefix
        active_layer: _,  // layer only: no raster, no pixels
        annotations: _,   // annotation_edits(): annotation revision only
    } = baseline_with_colorbar();
    let AxisOptions {
        scale: _,
        min: _,
        max: _,
        major_spacing: _,
        minor_count: _,
        inverted: _,
        label_style,
        tick: _,
        title_option,
        out_margin: _,
        line_offset: _,
        line_visible: _,
        line_color: _,
        line_width: _,
        line_style: _,
        major_tick_length: _,
        minor_tick_length: _,
    } = top_x;
    let LabelStyle {
        visible: _,
        color: _,
        font_size: _,
        label_visible: _,
        label_font: _,
        label_offset_x: _,
        label_offset_y: _,
        format: _,
        significant_digits: _,
    } = label_style;
    let AxisTitleOptions {
        text: _,
        visible: _,
        offset_x: _,
        offset_y: _,
    } = title_option;
    let ChartTitleOptions {
        text: _,
        visible: _,
        offset_x: _,
        offset_y: _,
        top_margin: _,
    } = chart_title;
    let GridOptions {
        show_major_x: _,
        major_x_color: _,
        major_x_width: _,
        major_x_style: _,
        show_major_y: _,
        major_y_color: _,
        major_y_width: _,
        major_y_style: _,
        show_minor_x: _,
        minor_x_color: _,
        minor_x_width: _,
        minor_x_style: _,
        show_minor_y: _,
        minor_y_color: _,
        minor_y_width: _,
        minor_y_style: _,
    } = grid;
    let crate::legend::Legend {
        visible: _,
        content: _,
        corner: _,
        offset_x: _,
        offset_y: _,
        padding: _,
        bg_color: _,
        border_color: _,
    } = baseline().legend;
    let ColorBarOptions {
        visible: _,
        side: _,
        thickness_px: _,
        gap_px: _,
        length_frac: _,
        align: _,
        offset_x: _,
        offset_y: _,
        colormap: _,
        nan_color: _,
        border_color: _,
        border_width: _,
        axis: _,
    } = colorbar.expect("the colourbar baseline has a colourbar");
    // The annotation layer: `visible` skips the composite only (no raster,
    // no annotation raster); `items` redo the annotation rasters only
    // (`annotation_edits`, the annotation revision), never the axis raster.
    let crate::AnnotationLayer {
        visible: _,
        items: _,
    } = crate::AnnotationLayer::default();
    let crate::OverlayLayer {
        y_axis: _, // every AxisOptions field, as overlay!(y_axis.*)
        series: _, // overlays[0].series
        legend: _, // overlays[0].legend (None -> Some)
    } = overlays.into_iter().next().unwrap_or_default();
    let _ = baseline_with_overlay();
    // A subplot's chart is a `Config` classified by the same catalogue (its
    // geometry and its chrome, `subplot_edits`); its series list and links
    // decide what it draws and against which range.
    let crate::Subplot {
        config: _, // subplots[0].config.* (geometry Packed, chrome Prefix)
        series: _, // subplots[0].series
        link_x: _, // same rule as link_y
        link_y: _, // subplots[0].link_y (None -> Parent)
    } = subplots.into_iter().next().unwrap_or_default();
    let _ = baseline_with_subplot();
}

#[test]
fn ssot_edits_refresh_resident_first_frame() {
    matrix(true, false);
}

#[test]
fn histogram_orientation_round_trip_refreshes_first_frame_without_reupload() {
    use crate::data_config::{BarOrientation, DataBarStyleConfig};
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = baseline();
    config.bottom_x.min = 0.0;
    config.bottom_x.max = 1.0;
    config.left_y.min = 0.0;
    config.left_y.max = 1.0;
    config.legend.visible = false;
    let mut series = declaration("hist", "x", "a");
    series.render_type = DataRenderType::Histogram {
        bar: DataBarStyleConfig {
            fill_color: Color::new(0.1, 0.5, 0.8, 1.0),
            border_color: Color::BLACK,
            border_width: 1.0,
            baseline: 0.0,
            gap_px: 2.0,
            width_ratio: 0.75,
            orientation: BarOrientation::Vertical,
            bar_style_overrides: None,
        },
    };
    let (mut r, id) = fixture(&config, true, false);
    r.set_chart_series(id, vec![series.clone()]).unwrap();
    let (vertical, _) = frame(&mut r, id, true);
    let generation = r.pool.generation();
    let used_bytes = r.pool.used_bytes();
    let original_buffers = r.chart_states[&id].prepared_styles.as_ref().unwrap().styles[0]
        .bar_bg
        .clone();

    for horizontal in [true, false] {
        if horizontal {
            std::mem::swap(&mut series.x_column, &mut series.y_column);
        } else {
            series.x_column = "x".into();
            series.y_column = "a".into();
        }
        let DataRenderType::Histogram { bar } = &mut series.render_type else {
            unreachable!()
        };
        bar.orientation = if horizontal {
            BarOrientation::Horizontal
        } else {
            BarOrientation::Vertical
        };
        r.set_chart_state(id, config.clone(), vec![series.clone()])
            .unwrap();
        let (actual, _) = frame(&mut r, id, true);
        let (mut fresh, fresh_id) = fixture(&config, true, false);
        fresh
            .set_chart_series(fresh_id, vec![series.clone()])
            .unwrap();
        let (expected, _) = frame(&mut fresh, fresh_id, true);
        assert_eq!(
            actual, expected,
            "first frame after orientation change must match fresh rendering"
        );
        assert_eq!(actual != vertical, horizontal);
        assert_eq!(
            r.pool.generation(),
            generation,
            "orientation must reuse column allocations"
        );
        assert_eq!(r.pool.used_bytes(), used_bytes);
        let styles = r.chart_states[&id].prepared_styles.as_ref().unwrap().styles[0]
            .bar_bg
            .clone();
        if horizontal {
            assert_ne!(
                styles, original_buffers,
                "orientation needs refreshed bar state"
            );
        }
        // Reapplying the same SSOT state must not rebuild that state again.
        r.set_chart_state(id, config.clone(), vec![series.clone()])
            .unwrap();
        assert_eq!(frame(&mut r, id, true).0, expected);
        assert_eq!(
            r.chart_states[&id].prepared_styles.as_ref().unwrap().styles[0].bar_bg,
            styles
        );
    }
    // Repeating a changed declaration before its first prepare must not mark
    // the previous, now-stale styles as valid for the new declaration.
    let DataRenderType::Histogram { bar } = &mut series.render_type else {
        unreachable!()
    };
    bar.fill_color = Color::new(0.8, 0.2, 0.1, 1.0);
    r.set_chart_series(id, vec![series.clone()]).unwrap();
    r.set_chart_series(id, vec![series.clone()]).unwrap();
    let (recolored, _) = frame(&mut r, id, true);
    let (mut fresh, fresh_id) = fixture(&config, true, false);
    fresh
        .set_chart_series(fresh_id, vec![series.clone()])
        .unwrap();
    assert_eq!(recolored, frame(&mut fresh, fresh_id, true).0);
    assert_ne!(recolored, vertical);
    let styles = r.chart_states[&id].prepared_styles.as_ref().unwrap().styles[0]
        .bar_bg
        .clone();
    config.grid.show_major_x = false;
    r.set_chart_state(id, config.clone(), vec![series.clone()])
        .unwrap();
    fresh
        .set_chart_state(fresh_id, config, vec![series])
        .unwrap();
    assert_eq!(
        frame(&mut r, id, true).0,
        frame(&mut fresh, fresh_id, true).0
    );
    assert_eq!(
        r.chart_states[&id].prepared_styles.as_ref().unwrap().styles[0].bar_bg,
        styles,
        "a full-state grid edit must reuse unchanged bar styles"
    );
}
/// Line fill and step interpolation are series declarations: each edit
/// refreshes the first frame to exactly what a fresh renderer draws for the
/// final state, moves only the data revision (never the view or selection),
/// reuses the column allocations, and an identical re-declaration changes
/// nothing.
#[test]
fn line_fill_and_step_edits_refresh_first_frame_without_reupload() {
    use crate::data_config::{DataLineStyleConfig, LineFill, LineInterpolation};
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = baseline();
    config.legend.visible = false;
    let mut series = declaration("s", "x", "a");
    let (mut r, id) = fixture(&config, true, false);
    let (plain, _) = frame(&mut r, id, true);
    let generation = r.pool.generation();
    let used_bytes = r.pool.used_bytes();
    let mut previous = plain;
    let edits: [(&str, fn(&mut DataLineStyleConfig)); 5] = [
        ("fill to baseline", |line| {
            line.fill = Some(LineFill::ToBaseline(0.0));
        }),
        ("fill colour", |line| {
            line.fill_color = Some(Color::new(0.1, 0.6, 0.2, 0.5));
        }),
        ("step post with fill", |line| {
            line.interpolation = LineInterpolation::StepPost;
        }),
        ("step mid without fill", |line| {
            line.fill = None;
            line.interpolation = LineInterpolation::StepMid;
        }),
        ("back to linear", |line| {
            line.interpolation = LineInterpolation::Linear;
        }),
    ];
    for (name, apply) in edits {
        let DataRenderType::Line { line } = &mut series.render_type else {
            unreachable!()
        };
        apply(line);
        let revisions = r.chart_states[&id].revisions;
        r.set_chart_series(id, vec![series.clone()]).unwrap();
        let updated = r.chart_states[&id].revisions;
        assert_ne!(
            updated.data, revisions.data,
            "{name}: a render_type edit is a data declaration change"
        );
        assert_eq!(updated.view, revisions.view, "{name}: no view replay");
        assert_eq!(updated.selection, revisions.selection, "{name}");
        let (actual, _) = frame(&mut r, id, true);
        let (mut fresh, fresh_id) = fixture(&config, true, false);
        fresh
            .set_chart_series(fresh_id, vec![series.clone()])
            .unwrap();
        let (expected, _) = frame(&mut fresh, fresh_id, true);
        assert_eq!(
            actual, expected,
            "{name}: first frame must match a fresh renderer"
        );
        assert_ne!(actual, previous, "{name}: the edit must be visible");
        previous = actual;
        assert_eq!(r.pool.generation(), generation, "{name}: no re-upload");
        assert_eq!(r.pool.used_bytes(), used_bytes, "{name}");
        // An identical re-declaration leaves the data revision and the frame.
        let revisions = r.chart_states[&id].revisions;
        r.set_chart_series(id, vec![series.clone()]).unwrap();
        assert_eq!(r.chart_states[&id].revisions.data, revisions.data, "{name}");
        assert_eq!(frame(&mut r, id, true).0, expected, "{name}: stable");
    }
    // Back to the original declaration: the plain line, pixel for pixel.
    assert_eq!(previous, frame(&mut r, id, true).0);
    let mut fresh_plain = declaration("s", "x", "a");
    let DataRenderType::Line { line } = &mut fresh_plain.render_type else {
        unreachable!()
    };
    line.interpolation = LineInterpolation::Linear;
    r.set_chart_series(id, vec![fresh_plain]).unwrap();
    let (mut fresh, fresh_id) = fixture(&config, true, false);
    assert_eq!(
        frame(&mut r, id, true).0,
        frame(&mut fresh, fresh_id, true).0
    );
}

#[test]
fn ssot_edits_refresh_stream_first_frame_without_excess_work() {
    matrix(false, false);
}
#[test]
fn ssot_edits_reuse_packed_rows_only_when_valid() {
    matrix(false, true);
}

/// Every selection edit reaches the first frame through the selection
/// suffix alone, on the resident path and the stream path with and without
/// the packed cache. One renderer per path takes every case: each starts
/// from the baseline selection, redrawn and equal to the first baseline
/// frame, and is compared with a fresh-render oracle: a chart registered
/// with the final state on one shared oracle renderer per data path, whose
/// first frame has no invalidation history.
#[test]
fn ssot_selection_edits_replace_only_the_overlay() {
    use crate::config::{PickedPointRef, PickedPointsConfig};
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let cases: &[(&str, fn(&mut PickedPointsConfig))] = &[
        ("visible", |s| s.visible = false),
        ("ring_color", |s| {
            s.ring_color = Color::new(0.0, 0.0, 1.0, 1.0)
        }),
        ("ring_width_px", |s| s.ring_width_px = 5.0),
        ("radius_extra_px", |s| s.radius_extra_px = 8.0),
        ("point_index", |s| s.refs[0].point_index = 5),
        ("refs.clear", |s| s.refs.clear()),
    ];
    let mut base = baseline();
    base.picked_points = Some(PickedPointsConfig {
        refs: vec![PickedPointRef {
            source_id: None,
            series_id: "s".into(),
            point_index: 2,
        }],
        ..Default::default()
    });
    let edited = |name: &str, edit: fn(&mut PickedPointsConfig)| {
        let mut c = base.clone();
        edit(c.picked_points.as_mut().unwrap());
        assert_ne!(c, base, "{name} must edit the selection");
        c
    };
    let series = || {
        let mut series = declaration("s", "x", "a");
        let line = super::super::extract_line(&series.render_type)
            .unwrap()
            .clone();
        series.render_type = DataRenderType::ScatterLine {
            line,
            scatter: DataScatterStyleConfig {
                point_color: Color::BLACK,
                point_size: 4.0,
                point_shape: ScatterShape::CircleFilled,
                point_style_index_column: None,
                point_style_table: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            },
        };
        series
    };
    let setup = |c: &Config, resident: bool, cache: bool| {
        let (mut r, id) = fixture(c, resident, cache);
        r.set_chart_series(id, vec![series()]).unwrap();
        (r, id)
    };
    // The oracle frames of every final state, per data path.
    let oracle = |resident: bool| -> Vec<Vec<u8>> {
        let (mut fresh, unused) = fixture(&base, resident, false);
        fresh.remove_chart(unused).unwrap();
        cases
            .iter()
            .map(|&(name, edit)| {
                let id = fresh
                    .register_chart(edited(name, edit), vec![series()])
                    .unwrap();
                let (expected, _) = frame(&mut fresh, id, resident);
                fresh.remove_chart(id).unwrap();
                expected
            })
            .collect()
    };
    let resident_oracle = oracle(true);
    let stream_oracle = oracle(false);
    for (resident, cache) in [(true, false), (false, false), (false, true)] {
        let expected_frames = if resident {
            &resident_oracle
        } else {
            &stream_oracle
        };
        let (mut r, id) = setup(&base, resident, cache);
        let (baseline_frame, _) = frame(&mut r, id, resident);
        for (&(name, edit), expected) in cases.iter().zip(expected_frames) {
            if r.chart_config(id).unwrap() != &base {
                r.set_chart_config(id, base.clone()).unwrap();
            }
            let (before, _) = frame(&mut r, id, resident);
            assert_eq!(
                before, baseline_frame,
                "{name}: the baseline selection did not redraw, resident={resident} cache={cache}"
            );
            let job = r.active_stream_job(id);
            let prefix = job.map(|j| r.chart_stream_prefix_for_test(j).unwrap());
            let data_pixels = prefix.as_ref().map(|t| read_draw_target(&r, t));
            let c = edited(name, edit);
            let revisions = r.chart_states[&id].revisions;
            r.set_chart_config(id, c).unwrap();
            let updated = r.chart_states[&id].revisions;
            assert_ne!(updated.desired, revisions.desired, "{name}: no draw");
            assert_ne!(
                updated.selection, revisions.selection,
                "{name}: no selection"
            );
            assert_eq!(
                updated.raster, revisions.raster,
                "{name}: selection re-rasterized"
            );
            assert_eq!(
                updated.view, revisions.view,
                "{name}: selection changed the view"
            );
            let (actual, reads) = frame(&mut r, id, resident);
            assert_eq!(
                &actual, expected,
                "{name}: stale selection, resident={resident} cache={cache}"
            );
            assert_ne!(actual, before, "{name}: selection oracle unchanged");
            assert_eq!(reads, 0, "{name}: selection replayed data");
            assert_eq!(
                r.active_stream_job(id),
                job,
                "{name}: selection restarted data job"
            );
            if let Some(job) = job {
                let after = r.chart_stream_prefix_for_test(job).unwrap();
                assert_eq!(
                    prefix.as_ref(),
                    Some(&after),
                    "{name}: selection replaced data texture"
                );
                assert_eq!(
                    data_pixels.as_ref(),
                    Some(&read_draw_target(&r, &after)),
                    "{name}: selection rewrote data"
                );
            }
        }
    }
    drop(font);
}

#[test]
fn ssot_cached_fit_restores_secondary_axes_without_replaying_data() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let (mut r, id) = fixture(&baseline(), false, false);
    frame(&mut r, id, false);
    r.request_stream_auto_fit(id, 0.0).unwrap();
    frame(&mut r, id, false);
    let fitted = r.chart_config(id).unwrap().clone();
    let job = r.active_stream_job(id);
    let mut edited = fitted.clone();
    edited.top_x.min = 0.4;
    r.set_chart_config(id, edited).unwrap();
    frame(&mut r, id, false);
    r.request_stream_auto_fit(id, 0.0).unwrap();
    assert_eq!(r.chart_config(id).unwrap().top_x.min, fitted.top_x.min);
    assert_eq!(
        r.chart_config(id).unwrap().bottom_x.min,
        fitted.bottom_x.min
    );
    let (_, reads) = frame(&mut r, id, false);
    assert_eq!(reads, 0);
    assert_eq!(r.active_stream_job(id), job);
}

#[path = "source_cache_invalidation_tests.rs"]
mod source_cache_invalidation;

#[cfg(feature = "serde")]
#[path = "ssot_mutation_tests.rs"]
mod mutation;

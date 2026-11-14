//! Render first, edit exactly one SSOT property, then inspect the FIRST updated
//! frame and the work needed to produce it. The pixel oracle is a fresh renderer
//! with the final state, never an invalidation predicate from the implementation.
use super::*;
use crate::annotation::{Anchor, Annotation, AnnotationAxis, AnnotationKind};
use crate::config::{AxisScale, DrawStyle, LogTickPolicy, TickVisibility};
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
    // The plot fill (#203) is the grid raster's first part.
    edit!(
        plot_background,
        Some(crate::Color::from_rgb8(240, 244, 250)),
        Prefix
    );
    // The chart fill (#265) is the grid raster's first part too.
    edit!(
        background,
        Some(crate::Color::from_rgb8(250, 246, 236)),
        Prefix
    );
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
            // Only the axis line moves; the data area does not.
            edit!($axis.cross_at, Some(0.5), Prefix);
            // The unit (#273) changes what the axis prints and where its
            // ticks fall, not the data: the axis raster is redone.
            edit!(
                $axis.unit,
                Some(crate::config::AxisUnit::new(2.0, 1.0)),
                Prefix
            );
            // A linear axis ignores the log tick policy: the axis raster is
            // redone (its config changed) and draws the same pixels. The
            // log axes of `log_tick_edits` draw new ones.
            cases.push(Edit {
                name: concat!(stringify!($axis), ".log_ticks"),
                apply: |c| c.$axis.log_ticks = LogTickPolicy::Adaptive,
                work: Work::Prefix,
                differs: true,
                raster: true,
                annotation: false,
                pixels: Pixels::Unchanged,
                streams: true,
            });
            // The tick label clearance (#272) is a label style edit: the
            // axis raster is redone. No label of the baseline overlaps a
            // label across or a break mark, so `Keep` draws the same
            // pixels; `label_clearance_edits` drops one.
            cases.push(Edit {
                name: concat!(stringify!($axis), ".label_style.clearance"),
                apply: |c| c.$axis.label_style.clearance = crate::config::LabelClearanceMode::Keep,
                work: Work::Prefix,
                differs: true,
                raster: true,
                annotation: false,
                pixels: Pixels::Unchanged,
                streams: true,
            });
        };
    }
    axis!(bottom_x);
    axis!(left_y);
    axis!(top_x);
    axis!(right_y);
    // A categorical axis on the plain baseline: chrome on the top and right
    // axes, the data range on the bottom and left ones. The stream pass
    // rejects categorical axes, so these are resident only.
    for (name, apply, work) in [
        (
            "top_x.categories (None -> Some)",
            (|c: &mut Config| c.top_x.categories = three_categories()) as fn(&mut Config),
            Work::Prefix,
        ),
        (
            "right_y.categories (None -> Some)",
            |c| c.right_y.categories = three_categories(),
            Work::Prefix,
        ),
        (
            "bottom_x.categories (None -> Some)",
            |c| c.bottom_x.categories = three_categories(),
            Work::Replay,
        ),
        (
            "left_y.categories (None -> Some)",
            |c| c.left_y.categories = three_categories(),
            Work::Replay,
        ),
    ] {
        cases.push(Edit {
            name,
            apply,
            work,
            differs: true,
            raster: true,
            annotation: false,
            pixels: Pixels::Changed,
            streams: false,
        });
    }
    // A cut through the drawn line (#200); see `break_edits` for the rest.
    // The stream pass rejects broken axes, so these are resident only.
    for (name, apply) in [
        (
            "bottom_x.breaks (None -> Some)",
            (|c: &mut Config| c.bottom_x.breaks = Some(cut(0.4, 0.55))) as fn(&mut Config),
        ),
        ("left_y.breaks (None -> Some)", |c| {
            c.left_y.breaks = Some(cut(0.45, 0.6))
        }),
    ] {
        cases.push(Edit {
            name,
            apply,
            work: Work::Packed,
            differs: true,
            raster: true,
            annotation: false,
            pixels: Pixels::Changed,
            streams: false,
        });
    }
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
    // The math tree of the title (#45): each field of the math nodes is part
    // of the raster key. The baseline title holds `x^2_i` and `\frac{a}{b}`.
    fn math(c: &mut Config, f: fn(&mut crate::text::RichSegment)) {
        c.chart_title.text.segments.iter_mut().for_each(f);
        assert_eq!(c.chart_title.text.validate_math(), Ok(()));
    }
    cases.push(Edit {
        name: "chart_title.text math_node.kind",
        apply: |c| {
            math(c, |s| {
                if let Some(n) = &mut s.math_node
                    && matches!(n.kind, crate::text::MathKind::Frac(_))
                {
                    n.kind = crate::text::MathKind::Binom(crate::text::FracStyle::Auto);
                }
            })
        },
        work: Work::Prefix,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: true,
    });
    cases.push(Edit {
        name: "chart_title.text math_parent.slot",
        apply: |c| {
            // The superscript becomes a second subscript.
            math(c, |s| {
                if s.text == '2'
                    && let Some(p) = &mut s.math_parent
                {
                    p.slot = 2;
                }
            })
        },
        work: Work::Prefix,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: true,
    });
    cases.push(Edit {
        name: "chart_title.text math slot child text",
        apply: |c| {
            math(c, |s| {
                if s.text == 'b' && s.math_parent.is_some() {
                    s.text = 'c';
                }
            })
        },
        work: Work::Prefix,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: true,
    });
    cases.push(Edit {
        // A valid renumbering is a different document with the same layout.
        name: "chart_title.text math ids renumbered",
        apply: |c| {
            math(c, |s| {
                if let Some(n) = &mut s.math_node {
                    n.id += 10;
                }
                if let Some(p) = &mut s.math_parent {
                    p.id += 10;
                }
            })
        },
        work: Work::Prefix,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Unchanged,
        streams: true,
    });
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
        apply: |c| c.set_polar(Some(crate::polar::PolarOptions::default())),
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
    // How category bars share a slot: the baseline has no category bar
    // series, so the bodies, the chrome and the data area all stay.
    cases.push(Edit {
        name: "bar_groups (None -> Some)",
        apply: |c| c.bar_groups = Some(crate::data_config::BarGroups::default()),
        work: Work::Prefix,
        differs: true,
        raster: false,
        annotation: false,
        pixels: Pixels::Unchanged,
        streams: true,
    });
    // A line offset group moves its members into panels of their own: a
    // relayout like adding an overlay, so the view and the raster follow.
    // The one member is the stack's first, unshifted, so the frame is the
    // same picture; the stream pass rejects line offsets (rule 8).
    cases.push(Edit {
        name: "line_offsets (None -> Some)",
        apply: |c| {
            c.line_offsets = Some(crate::data_config::LineOffsets {
                series: vec!["s".into()],
                step_y: 0.5,
                step_x: 0.25,
            })
        },
        work: Work::Packed,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Unchanged,
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

/// Three categories, for the categorical axis edits.
fn three_categories() -> Option<Vec<crate::config::Category>> {
    Some(vec![
        crate::config::Category::new("a", "Alpha"),
        crate::config::Category::new("b", "Beta"),
        crate::config::Category::new("c", "Gamma"),
    ])
}

/// The baseline with categorical top and bottom axes (three categories
/// each). The stream pass rejects categorical axes, so every case here is
/// resident only.
fn baseline_with_categories() -> Config {
    let mut c = baseline();
    c.top_x.categories = three_categories();
    c.bottom_x.categories = three_categories();
    c
}

/// Every field of `AxisOptions::categories`, one edit each. A label is tick
/// text (raster only); an id is drawn nowhere, but it is part of the axis the
/// raster key compares, so the raster is redone with unchanged pixels; the
/// category count is the bottom axis' data range, so it moves the data.
fn category_edits() -> Vec<Edit> {
    let mut cases = Vec::new();
    macro_rules! category {
        ($name:literal, $apply:expr, $work:ident, $pixels:ident) => {
            cases.push(Edit {
                name: $name,
                apply: $apply,
                work: Work::$work,
                differs: true,
                raster: true,
                annotation: false,
                pixels: Pixels::$pixels,
                streams: false,
            });
        };
    }
    category!(
        "top_x.categories[1].label",
        |c| c.top_x.categories.as_mut().unwrap()[1].label = "Changed".into(),
        Prefix,
        Changed
    );
    category!(
        "top_x.categories[1].id",
        |c| c.top_x.categories.as_mut().unwrap()[1].id = "changed".into(),
        Prefix,
        Unchanged
    );
    category!(
        "top_x.categories (one more)",
        |c| c
            .top_x
            .categories
            .as_mut()
            .unwrap()
            .push(crate::config::Category::new("d", "Delta")),
        Prefix,
        Changed
    );
    category!(
        "top_x.categories (Some -> None)",
        |c| c.top_x.categories = None,
        Prefix,
        Changed
    );
    // The window (#205) is the range of a categorical axis: the bottom one
    // maps data like `bottom_x.min`, the top one draws chrome only.
    category!(
        "bottom_x.category_window",
        |c| c.bottom_x.category_window = Some([0.5, 2.5]),
        Packed,
        Changed
    );
    category!(
        "top_x.category_window",
        |c| c.top_x.category_window = Some([0.5, 2.5]),
        Prefix,
        Changed
    );
    category!(
        "bottom_x.categories[1].label",
        |c| c.bottom_x.categories.as_mut().unwrap()[1].label = "Changed".into(),
        Prefix,
        Changed
    );
    category!(
        "bottom_x.categories[1].id",
        |c| c.bottom_x.categories.as_mut().unwrap()[1].id = "changed".into(),
        Prefix,
        Unchanged
    );
    category!(
        "bottom_x.categories (one more)",
        |c| c
            .bottom_x
            .categories
            .as_mut()
            .unwrap()
            .push(crate::config::Category::new("d", "Delta")),
        Replay,
        Changed
    );
    category!(
        "bottom_x.categories (Some -> None)",
        |c| c.bottom_x.categories = None,
        Replay,
        Changed
    );
    // A rich tick label and the group headers (#203) are tick chrome.
    category!(
        "bottom_x.categories[1].rich_label (None -> Some)",
        |c| c.bottom_x.categories.as_mut().unwrap()[1].rich_label = Some(
            crate::text::RichText::plain("B₂", crate::Color::BLACK, 12.0, "")
        ),
        Prefix,
        Changed
    );
    category!(
        "bottom_x.category_groups (None -> Some)",
        |c| c.bottom_x.category_groups = Some(crate::config::CategoryGroups {
            groups: vec![crate::config::CategoryGroup {
                label: crate::text::RichText::plain("Group", crate::Color::BLACK, 12.0, ""),
                first: "a".into(),
                last: "b".into(),
                bracket: true,
            }],
            offset: [0.0; 2],
        }),
        Prefix,
        Changed
    );
    // `min` / `max` are ignored on a categorical axis: a written range is
    // configuration only.
    category!(
        "bottom_x.max (categorical)",
        |c| c.bottom_x.max = 2.0,
        Prefix,
        Unchanged
    );
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
    // The baseline bar is linear, which ignores the log tick policy.
    bar!(axis.log_ticks, LogTickPolicy::Adaptive, Prefix, Unchanged);
    // The colour ramp stays on the data; the labels print the unit (#273).
    bar!(
        axis.unit,
        Some(crate::config::AxisUnit::new(2.0, 1.0)),
        Prefix,
        Changed
    );
    cases
}

/// The colourbar baseline with logarithmic top, right and colourbar axes
/// over 0.1..1, whose decade ladder labels 0.1 and 1 only: the adaptive
/// policy draws the 1-2-5 ladder there, a chrome edit of each axis (#42).
fn baseline_with_log_axes() -> Config {
    let mut c = baseline_with_colorbar();
    let bar = c.colorbar.as_mut().unwrap();
    for axis in [&mut c.top_x, &mut c.right_y, &mut bar.axis] {
        axis.scale = AxisScale::Logarithmic;
        axis.major_spacing = 1.0;
        axis.minor_count = 8;
    }
    c
}

/// A break cutting `from..to` out of an axis, `Continuous` with the default
/// gap and mark (#200).
fn cut(from: f64, to: f64) -> crate::config::AxisBreaks {
    crate::config::AxisBreaks {
        ranges: vec![crate::config::AxisBreak::new(from, to)],
        ..Default::default()
    }
}

/// The baseline with `bottom_x` cut `Continuous` and `left_y` cut `Panels`,
/// both through the drawn line.
fn baseline_with_breaks() -> Config {
    let mut c = baseline();
    c.bottom_x.breaks = Some(cut(0.4, 0.55));
    c.left_y.breaks = Some(crate::config::AxisBreaks {
        style: crate::config::AxisBreakStyle::Panels,
        ..cut(0.45, 0.6)
    });
    c
}

/// Break edits (#200): the cut ranges, style and gap move the data's
/// pixels and the piece layout, so they are data-area edits; the mark and
/// the piece frames are chrome. None of these documents stream: the stream
/// pass rejects broken axes as a contract of its own.
fn break_edits() -> Vec<Edit> {
    use crate::config::{AxisBreak, AxisBreakStyle, BreakMark};
    let mut cases = Vec::new();
    let mut push = |name, apply, work, pixels| {
        cases.push(Edit {
            name,
            apply,
            work,
            differs: true,
            raster: true,
            annotation: false,
            pixels,
            streams: false,
        });
    };
    push(
        "bottom_x.breaks (Some -> None)",
        (|c: &mut Config| c.bottom_x.breaks = None) as fn(&mut Config),
        Work::Packed,
        Pixels::Changed,
    );
    push(
        "left_y.breaks (Some -> None)",
        |c| c.left_y.breaks = None,
        Work::Packed,
        Pixels::Changed,
    );
    push(
        "bottom_x.breaks.ranges (one -> two)",
        |c| {
            if let Some(b) = c.bottom_x.breaks.as_mut() {
                b.ranges.push(AxisBreak::new(0.7, 0.8));
            }
        },
        Work::Packed,
        Pixels::Changed,
    );
    push(
        "bottom_x.breaks.style (Continuous -> Panels)",
        |c| {
            if let Some(b) = c.bottom_x.breaks.as_mut() {
                b.style = AxisBreakStyle::Panels;
            }
        },
        Work::Packed,
        Pixels::Changed,
    );
    push(
        "left_y.breaks.style (Panels -> Continuous)",
        |c| {
            if let Some(b) = c.left_y.breaks.as_mut() {
                b.style = AxisBreakStyle::Continuous;
            }
        },
        Work::Packed,
        Pixels::Changed,
    );
    push(
        "left_y.breaks.gap_px",
        |c| {
            if let Some(b) = c.left_y.breaks.as_mut() {
                b.gap_px = 30.0;
            }
        },
        Work::Packed,
        Pixels::Changed,
    );
    push(
        "bottom_x.breaks.mark (Slash -> None)",
        |c| {
            if let Some(b) = c.bottom_x.breaks.as_mut() {
                b.mark = BreakMark::None;
            }
        },
        Work::Prefix,
        Pixels::Changed,
    );
    // The piece frames are chrome in either style.
    push(
        "bottom_x.breaks.frame_each_panel (Continuous)",
        |c| {
            if let Some(b) = c.bottom_x.breaks.as_mut() {
                b.frame_each_panel = false;
            }
        },
        Work::Prefix,
        Pixels::Changed,
    );
    push(
        "left_y.breaks.frame_each_panel (Panels)",
        |c| {
            if let Some(b) = c.left_y.breaks.as_mut() {
                b.frame_each_panel = false;
            }
        },
        Work::Prefix,
        Pixels::Changed,
    );
    cases
}

/// The baseline with a corner where the labels meet (#272): `0.00` on
/// both axes at the bottom-left corner, in fonts large enough that the
/// lowest y label overlaps the first x label, which `Auto` then drops.
fn baseline_with_crowded_corner() -> Config {
    let mut c = baseline();
    for (a, font) in [(&mut c.bottom_x, 24.0), (&mut c.left_y, 40.0)] {
        a.min = 0.0;
        a.label_style.font_size = font;
        a.label_style.format =
            crate::format::LabelFormat::Plain(crate::format::PlainNumberFormat {
                decimals: Some(2),
                trim_zeros: false,
            });
    }
    // The corner drops a label as drawn, and `Keep` draws it.
    let left = |c: &Config| {
        let mut n = 0;
        crate::select::emit_element(
            c,
            &crate::layout::Element::AxisLabel(Side::Left),
            &crate::CpuTextMeasure::for_style(&c.draw_style),
            &mut crate::sink::Parts(&mut |_| n += 1),
        );
        n
    };
    let mut keep = c.clone();
    keep.left_y.label_style.clearance = crate::config::LabelClearanceMode::Keep;
    assert_eq!(left(&c) + 1, left(&keep), "the corner y label yields");
    c
}

/// Tick label clearance edits (#272) on a corner where the labels overlap:
/// a `Keep` y axis draws the corner label `Auto` dropped; the x axis does
/// not yield there, so its `Keep` draws the same pixels. Either way the
/// axis raster is redone and the data is untouched.
fn label_clearance_edits() -> Vec<Edit> {
    use crate::config::LabelClearanceMode::Keep;
    let mut cases = Vec::new();
    for (name, apply, pixels) in [
        (
            "left_y.label_style.clearance (crowded corner)",
            (|c: &mut Config| c.left_y.label_style.clearance = Keep) as fn(&mut Config),
            Pixels::Changed,
        ),
        (
            "bottom_x.label_style.clearance (crowded corner)",
            |c| c.bottom_x.label_style.clearance = Keep,
            Pixels::Unchanged,
        ),
    ] {
        cases.push(Edit {
            name,
            apply,
            work: Work::Prefix,
            differs: true,
            raster: true,
            annotation: false,
            pixels,
            streams: true,
        });
    }
    cases
}

/// A unit on a logarithmic axis (#273): its decades are re-planned on the
/// shown value, an axis chrome edit like the tick policy.
fn log_unit_edits() -> Vec<Edit> {
    let mut cases = Vec::new();
    for (name, apply) in [
        (
            "top_x.unit (log axis)",
            (|c: &mut Config| c.top_x.unit = Some(crate::config::AxisUnit::new(1e3, 0.0)))
                as fn(&mut Config),
        ),
        ("right_y.unit (log axis)", |c| {
            c.right_y.unit = Some(crate::config::AxisUnit::new(1e3, 0.0))
        }),
        ("colorbar.axis.unit (log axis)", |c| {
            c.colorbar.as_mut().unwrap().axis.unit = Some(crate::config::AxisUnit::new(1e3, 0.0))
        }),
    ] {
        cases.push(Edit {
            name,
            apply,
            work: Work::Prefix,
            differs: true,
            raster: true,
            annotation: false,
            pixels: Pixels::Changed,
            streams: true,
        });
    }
    cases
}

fn log_tick_edits() -> Vec<Edit> {
    let mut cases = Vec::new();
    for (name, apply) in [
        (
            "top_x.log_ticks (log axis)",
            (|c: &mut Config| c.top_x.log_ticks = LogTickPolicy::Adaptive) as fn(&mut Config),
        ),
        ("right_y.log_ticks (log axis)", |c| {
            c.right_y.log_ticks = LogTickPolicy::Adaptive
        }),
        ("colorbar.axis.log_ticks (log axis)", |c| {
            c.colorbar.as_mut().unwrap().axis.log_ticks = LogTickPolicy::Adaptive
        }),
    ] {
        cases.push(Edit {
            name,
            apply,
            work: Work::Prefix,
            differs: true,
            raster: true,
            annotation: false,
            pixels: Pixels::Changed,
            // Axis chrome only: the stream pass accepts the policy.
            streams: true,
        });
    }
    cases
}

/// `c` with every axis bounded inside `(0, 1)`, so any of them can take a
/// probability scale (#209): `max` 1.0 is outside the domain.
fn in_unit_interval(mut c: Config) -> Config {
    fn bound(axis: &mut crate::config::AxisOptions) {
        axis.min = 0.1;
        axis.max = 0.9;
        axis.major_spacing = 0.2;
    }
    for axis in [&mut c.bottom_x, &mut c.left_y, &mut c.top_x, &mut c.right_y] {
        bound(axis);
    }
    for overlay in &mut c.overlays {
        bound(&mut overlay.y_axis);
    }
    for subplot in &mut c.subplots {
        let s = &mut subplot.config;
        for axis in [&mut s.bottom_x, &mut s.left_y, &mut s.top_x, &mut s.right_y] {
            bound(axis);
        }
    }
    c
}

/// Probability scale edits (#209), each scale on each of the four axes, the
/// overlay axis and a subplot axis. A scale moves every sample on its axis
/// as the logarithmic edit does, so the base axes replay, the partner axes
/// redraw chrome only, and the overlay and subplot pack. The stream pass
/// refuses a probability base axis, so these are resident only.
fn probability_edits() -> Vec<Edit> {
    use crate::config::ProbabilityScale::{Logistic, Normal, Weibull};
    let mut cases = Vec::new();
    let mut push = |name, apply, work| {
        cases.push(Edit {
            name,
            apply,
            work,
            differs: true,
            raster: true,
            annotation: false,
            pixels: Pixels::Changed,
            streams: false,
        });
    };
    macro_rules! scale {
        ($($field:ident).+, $kind:ident, $work:ident) => {
            push(
                concat!(stringify!($($field).+), ".scale (Probability ", stringify!($kind), ")"),
                (|c: &mut Config| c.$($field).+.scale = AxisScale::Probability($kind))
                    as fn(&mut Config),
                Work::$work,
            )
        };
    }
    scale!(bottom_x, Normal, Replay);
    scale!(bottom_x, Weibull, Replay);
    scale!(bottom_x, Logistic, Replay);
    scale!(left_y, Normal, Replay);
    scale!(left_y, Weibull, Replay);
    scale!(left_y, Logistic, Replay);
    scale!(top_x, Normal, Prefix);
    scale!(top_x, Weibull, Prefix);
    scale!(top_x, Logistic, Prefix);
    scale!(right_y, Normal, Prefix);
    scale!(right_y, Weibull, Prefix);
    scale!(right_y, Logistic, Prefix);
    cases
}

/// A probability edit of an overlay or subplot axis: packed, as the
/// logarithmic one is.
fn packed_probability_edit(name: &'static str, apply: fn(&mut Config)) -> Edit {
    Edit {
        name,
        apply,
        work: Work::Packed,
        differs: true,
        raster: true,
        annotation: false,
        pixels: Pixels::Changed,
        streams: false,
    }
}

fn probability_overlay_edits() -> Vec<Edit> {
    use crate::config::ProbabilityScale::{Logistic, Normal, Weibull};
    vec![
        packed_probability_edit("overlays[0].y_axis.scale (Probability Normal)", |c| {
            c.overlays[0].y_axis.scale = AxisScale::Probability(Normal)
        }),
        packed_probability_edit("overlays[0].y_axis.scale (Probability Weibull)", |c| {
            c.overlays[0].y_axis.scale = AxisScale::Probability(Weibull)
        }),
        packed_probability_edit("overlays[0].y_axis.scale (Probability Logistic)", |c| {
            c.overlays[0].y_axis.scale = AxisScale::Probability(Logistic)
        }),
    ]
}

fn probability_subplot_edits() -> Vec<Edit> {
    use crate::config::ProbabilityScale::{Logistic, Normal, Weibull};
    vec![
        packed_probability_edit(
            "subplots[0].config.left_y.scale (Probability Normal)",
            |c| c.subplots[0].config.left_y.scale = AxisScale::Probability(Normal),
        ),
        packed_probability_edit(
            "subplots[0].config.left_y.scale (Probability Weibull)",
            |c| c.subplots[0].config.left_y.scale = AxisScale::Probability(Weibull),
        ),
        packed_probability_edit(
            "subplots[0].config.left_y.scale (Probability Logistic)",
            |c| c.subplots[0].config.left_y.scale = AxisScale::Probability(Logistic),
        ),
    ]
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
    // The baseline overlay axis is linear, which ignores the policy.
    overlay!(y_axis.log_ticks, LogTickPolicy::Adaptive, Prefix, Unchanged);
    overlay!(
        y_axis.unit,
        Some(crate::config::AxisUnit::new(2.0, 1.0)),
        Prefix,
        Changed
    );
    overlay!(y_axis.breaks, Some(cut(0.6, 1.2)), Packed, Changed);
    // The links choose the axes the overlay panel scales by and draws, so
    // each is geometry of its own panel.
    overlay!(link_x, false, Packed, Changed);
    overlay!(link_y, true, Packed, Changed);
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

/// Edits of an overlay's own X axis run against [`baseline_with_overlay`]
/// with `link_x` off, so the overlay draws its X axis on the top side and
/// scales by it: its range, scale and direction are geometry of its panel,
/// its band widens the shared top margin, and the rest is chrome of the
/// overlay panel, as for its Y axis (`overlay_edits`).
fn overlay_x_edits() -> Vec<Edit> {
    let mut cases = Vec::new();
    macro_rules! overlay {
        ($($field:ident).+, $value:expr, $work:ident, $pixels:ident) => {
            cases.push(Edit {
                name: concat!("overlays[0].", stringify!($($field).+), " (own x)"),
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
    overlay!(link_x, true, Packed, Changed);
    overlay!(x_axis.min, 0.2, Packed, Changed);
    overlay!(x_axis.max, 1.4, Packed, Changed);
    overlay!(x_axis.inverted, true, Packed, Changed);
    overlay!(x_axis.out_margin, 70.0, Packed, Changed);
    overlay!(x_axis.major_tick_length, 12.0, Packed, Changed);
    overlay!(x_axis.breaks, Some(cut(0.6, 0.9)), Packed, Changed);
    overlay!(x_axis.major_spacing, 0.25, Prefix, Changed);
    overlay!(x_axis.line_width, 3.0, Prefix, Changed);
    overlay!(x_axis.label_style.font_size, 13.0, Prefix, Changed);
    overlay!(x_axis.label_style.label_offset_y, 4.0, Prefix, Changed);
    overlay!(
        x_axis.title_option.text,
        RichText::plain("Top", Color::BLACK, 10.0, ""),
        Prefix,
        Changed
    );
    overlay!(x_axis.title_option.offset_y, 4.0, Prefix, Changed);
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
    c.chart_title.text =
        RichText::from_tex(r"SSOT $x^2_i + \frac{a}{b}$", Color::BLACK, 12.0, "").unwrap();
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
        marker: None,
        text_scale: Default::default(),
    });
    c
}

/// [`baseline_with_subplot`] with the subplot's cell half as wide as the
/// parent chart, so its text is drawn below its stored sizes (#292).
fn baseline_with_small_subplot() -> Config {
    let mut c = baseline_with_subplot();
    c.subplots[0].config.chart_area.0.width = 160;
    assert!(c.subplot_text_factor(0) < 0.8);
    c
}

/// A subplot's text scale (#292) is chrome computed from the chart areas:
/// the mode re-rasterizes the subplot's text only, and a resize of the cell
/// or of the parent chart, which is geometry already, carries the new
/// factor with it.
fn subplot_text_scale_edits() -> Vec<Edit> {
    fn case(name: &'static str, apply: fn(&mut Config), work: Work) -> Edit {
        Edit {
            name,
            apply,
            work,
            differs: true,
            raster: true,
            annotation: false,
            pixels: Pixels::Changed,
            streams: false,
        }
    }
    vec![
        case(
            "subplots[0].text_scale (Auto -> Fixed)",
            |c| c.subplots[0].text_scale = crate::SubplotTextScale::Fixed,
            Work::Prefix,
        ),
        case(
            "subplots[0].config.chart_area.width (text scale follows)",
            |c| c.subplots[0].config.chart_area.0.width = 300,
            Work::Packed,
        ),
        case(
            "chart_area.height (a subplot's text scale follows)",
            |c| c.chart_area.0.height = 100,
            Work::Packed,
        ),
    ]
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
            align: Default::default(),
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

fn baseline_with_overlay_x() -> Config {
    let mut c = baseline_with_overlay();
    let overlay = &mut c.overlays[0];
    overlay.link_x = false;
    overlay.x_axis.min = 0.0;
    overlay.x_axis.max = 1.6;
    overlay.x_axis.major_spacing = 0.4;
    overlay.x_axis.out_margin = 35.0;
    overlay.x_axis.label_style.font_size = 10.0;
    overlay.x_axis.title_option.visible = true;
    overlay.x_axis.title_option.text = RichText::plain("Own x", Color::BLACK, 10.0, "");
    c
}

fn values() -> (crate::Column<f32>, crate::Column<f32>) {
    (
        crate::Column {
            data: vec![0.05, 0.18, 0.3, 0.45, 0.6, 0.75, 0.9, 1.5],
        },
        crate::Column {
            data: vec![0.2, 0.7, 0.3, 0.8, 0.4, 0.7, 0.2, 0.6],
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
        RendererDevice::new(device, queue).expect("figgy poll thread"),
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
    let mut view = r
        .create_chart_view(&Chart::new(c.clone()), c.chart_area.0)
        .unwrap();
    if resident {
        let prepared = r
            .prepare_registered(&mut [RegisteredChartDrawItem {
                chart_id: id,
                view: &mut view,
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
    r.prepare_registered(&mut [RegisteredChartDrawItem {
        chart_id: id,
        view: &mut view,
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
        (baseline_with_overlay_x(), overlay_x_edits()),
        (baseline_with_subplot(), subplot_edits()),
        (baseline_with_small_subplot(), subplot_text_scale_edits()),
        (baseline_with_annotations(), annotation_edits()),
        (baseline_with_categories(), category_edits()),
        (baseline_with_log_axes(), log_tick_edits()),
        (baseline_with_log_axes(), log_unit_edits()),
        (baseline_with_crowded_corner(), label_clearance_edits()),
        (baseline_with_breaks(), break_edits()),
        (in_unit_interval(baseline()), probability_edits()),
        (
            in_unit_interval(baseline_with_overlay()),
            probability_overlay_edits(),
        ),
        (
            in_unit_interval(baseline_with_subplot()),
            probability_subplot_edits(),
        ),
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
        chart_area: _,  // chart_area.x/y/width/height: Packed
        top_x,          // axis!(top_x) + top_x range/scale/inverted: Prefix
        bottom_x: _,    // axis!(bottom_x) + range/scale/inverted: Packed/Replay
        left_y: _,      // axis!(left_y) + range/scale/inverted: Packed/Replay
        right_y: _,     // axis!(right_y) + range/scale/inverted: Prefix
        chart_title,    // chart_title.*: Prefix, top_margin: Packed
        grid,           // grid!(...): Prefix
        legend: _,      // legend.*: Prefix (Legend fields listed inline)
        picked_data: _, // ssot_selection_edits_replace_only_the_overlay
        colorbar,       // colorbar_edits(): band fields Packed, the rest Prefix
        draw_style: _,  // Sketch / Milkyway / Constellation: Replay
        // the coordinate system: geometry (Packed/Replay) and raster; a 3D
        // camera and parallel axes are covered by tests/surface_render.rs and
        // tests/parallel_render.rs, since the baseline series are neither kind
        coordinates: _,
        overlays,           // overlay_edits(): range/series/band Packed, the rest Prefix
        subplots,           // subplot_edits(): geometry/series/links Packed, chrome Prefix
        active_layer: _,    // layer only: no raster, no pixels
        annotations: _,     // annotation_edits(): annotation revision only
        bar_groups: _,      // "bar_groups (None -> Some)" + resident display mutations
        line_offsets: _,    // "line_offsets (None -> Some)" + resident display mutations
        plot_background: _, // edit!(plot_background): Prefix, raster
        background: _,      // edit!(background): Prefix, raster
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
        categories: _, // categories (None -> Some) + category_edits(): Prefix / Replay
        category_groups: _, // category_edits(): Prefix
        category_window: _, // category_edits(): bottom Packed, top Prefix
        cross_at: _,   // axis!(...).cross_at: Prefix
        log_ticks: _,  // axis!(...).log_ticks + log_tick_edits(): Prefix
        breaks: _, // breaks (None -> Some) + break_edits(): ranges/style/gap Packed, mark/frames Prefix
        unit: _,   // axis!(...).unit + bar!/overlay! + log_unit_edits(): Prefix
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
        clearance: _, // axis!(...).label_style.clearance + label_clearance_edits(): Prefix
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
        link_x: _, // overlays[0].link_x, both ways (overlay_x_edits)
        x_axis: _, // overlay_x_edits(): x_axis.* as y_axis.* (own x)
        link_y: _, // overlays[0].link_y
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
        marker: _,
        text_scale: _, // subplot_text_scale_edits(): chrome Prefix
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
        let DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(line),
            errors: None,
            ..
        }) = &mut series.render_type
        else {
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
    let DataRenderType::Xy(crate::data_config::XySeries {
        scatter: None,
        line: Some(line),
        errors: None,
        ..
    }) = &mut fresh_plain.render_type
    else {
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

/// A stem edit (#206) is a series declaration change: the data revision
/// moves, nothing is re-uploaded, and the first frame after each edit (add
/// stems, baseline, width, dash, direction, colour, remove) equals a fresh
/// renderer's.
#[test]
fn stem_edits_refresh_first_frame_without_reupload() {
    use crate::data_config::{CategoryBarDirection, DataScatterStyleConfig, ScatterShape, Stem};
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = baseline();
    config.legend.visible = false;
    let mut series = declaration("s", "x", "a");
    series.render_type = DataRenderType::Xy(crate::data_config::XySeries {
        scatter: Some(DataScatterStyleConfig {
            point_color: Color::new(0.1, 0.3, 0.8, 1.0),
            point_shape: ScatterShape::CircleFilled,
            point_size: 5.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        }),
        line: None,
        errors: None,
        stem: None,
    });
    let (mut r, id) = fixture(&config, true, false);
    r.set_chart_series(id, vec![series.clone()]).unwrap();
    let (plain, _) = frame(&mut r, id, true);
    let generation = r.pool.generation();
    let used_bytes = r.pool.used_bytes();
    let mut previous = plain.clone();
    let edits: [(&str, fn(&mut Option<Stem>)); 7] = [
        ("add stems", |stem| {
            *stem = Some(Stem {
                baseline: 0.0,
                direction: CategoryBarDirection::Vertical,
                color: Color::new(0.8, 0.1, 0.1, 1.0),
                width: 2.0,
                line_style: crate::line::LineStylePreset::Solid,
            });
        }),
        ("baseline", |stem| stem.as_mut().unwrap().baseline = 0.4),
        ("width", |stem| stem.as_mut().unwrap().width = 4.0),
        ("dash", |stem| {
            stem.as_mut().unwrap().line_style = crate::line::LineStylePreset::Dash;
        }),
        ("horizontal", |stem| {
            stem.as_mut().unwrap().direction = CategoryBarDirection::Horizontal;
        }),
        ("colour", |stem| {
            stem.as_mut().unwrap().color = Color::new(0.1, 0.7, 0.2, 1.0);
        }),
        ("remove", |stem| *stem = None),
    ];
    for (name, apply) in edits {
        let DataRenderType::Xy(xy) = &mut series.render_type else {
            unreachable!()
        };
        apply(&mut xy.stem);
        let revisions = r.chart_states[&id].revisions;
        r.set_chart_series(id, vec![series.clone()]).unwrap();
        let updated = r.chart_states[&id].revisions;
        assert_ne!(
            updated.data, revisions.data,
            "{name}: a stem edit is a data declaration change"
        );
        assert_eq!(updated.view, revisions.view, "{name}: no view replay");
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
    }
    // Removing the stems restores the plain markers, pixel for pixel.
    assert_eq!(previous, plain);
}

/// A polar heatmap (#208) follows its chart's polar options and colourbar:
/// each edit (zero angle, direction, unit, sector, hole, ray angle, the
/// colour map and its range) gives a first frame equal to a fresh
/// renderer's, is visible, and re-uploads nothing.
#[test]
fn polar_heatmap_edits_refresh_first_frame_without_reupload() {
    use crate::data_config::{
        ContourConfig, DataLineStyleConfig, FieldFillConfig, FieldSeries, FillMode, GridLayout,
        MatrixOrientation, MatrixRef, Shading,
    };
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = baseline_with_colorbar();
    config.legend.visible = false;
    config.left_y.min = 0.0;
    config.left_y.max = 3.0;
    config.set_polar(Some(crate::polar::PolarOptions::default()));
    let mut series = declaration("field", "gx", "gy");
    series.render_type = DataRenderType::Field(FieldSeries {
        matrix: MatrixRef {
            columns: vec!["z0".into(), "z1".into(), "z2".into(), "z3".into()],
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Edges,
        },
        fill: Some(FieldFillConfig {
            mode: FillMode::Continuous,
            shading: Shading::Flat,
            opacity: 1.0,
        }),
        contour: Some(ContourConfig {
            levels: vec![0.5],
            line: DataLineStyleConfig::default(),
            per_level_color: None,
            labels: None,
        }),
        surface: None,
    });
    let add_field = |r: &mut Renderer| {
        let column = |v: Vec<f64>| crate::data::Column { data: v };
        r.add_column("gx", &column(vec![0.0, 90.0, 180.0, 270.0, 360.0]))
            .unwrap();
        r.add_column("gy", &column(vec![0.0, 1.0, 2.0, 3.0]))
            .unwrap();
        for i in 0..4 {
            let base = f64::from(i) * 0.2;
            r.add_column(
                &format!("z{i}"),
                &column(vec![0.1 + base, 0.4 + base, 0.2 + base]),
            )
            .unwrap();
        }
    };
    let fixture_with_field = |c: &Config| {
        let (mut r, id) = fixture(c, true, false);
        add_field(&mut r);
        r.set_chart_series(id, vec![series.clone()]).unwrap();
        (r, id)
    };
    let (mut r, id) = fixture_with_field(&config);
    let (mut previous, _) = frame(&mut r, id, true);
    let generation = r.pool.generation();
    let used_bytes = r.pool.used_bytes();
    let edits: [(&str, fn(&mut Config)); 9] = [
        ("zero_angle_degrees", |c| {
            c.polar_mut().unwrap().zero_angle_degrees = 90.0;
        }),
        ("clockwise", |c| c.polar_mut().unwrap().clockwise = true),
        ("angle_unit", |c| {
            c.polar_mut().unwrap().angle_unit = crate::polar::AngleUnit::Radians;
        }),
        ("angle_unit back", |c| {
            c.polar_mut().unwrap().angle_unit = crate::polar::AngleUnit::Degrees;
        }),
        ("sector", |c| {
            c.polar_mut().unwrap().sector = Some([0.0, 200.0])
        }),
        ("inner_radius_fraction", |c| {
            c.polar_mut().unwrap().inner_radius_fraction = 0.3;
        }),
        ("radial_label_angle_degrees", |c| {
            c.polar_mut().unwrap().radial_label_angle_degrees = 45.0;
        }),
        ("colormap", |c| {
            c.colorbar.as_mut().unwrap().colormap = crate::ColorMap::Custom {
                stops: vec![
                    Color::new(0.0, 0.0, 1.0, 1.0),
                    Color::new(1.0, 1.0, 0.0, 1.0),
                ],
            };
        }),
        ("colorbar range", |c| {
            c.colorbar.as_mut().unwrap().axis.max = 2.0
        }),
    ];
    for (name, apply) in edits {
        apply(&mut config);
        r.set_chart_config(id, config.clone()).unwrap();
        let (actual, _) = frame(&mut r, id, true);
        let (mut fresh, fresh_id) = fixture_with_field(&config);
        let (expected, _) = frame(&mut fresh, fresh_id, true);
        assert_eq!(
            actual, expected,
            "{name}: first frame must match a fresh renderer"
        );
        assert_ne!(actual, previous, "{name}: the edit must be visible");
        previous = actual;
        assert_eq!(r.pool.generation(), generation, "{name}: no re-upload");
        assert_eq!(r.pool.used_bytes(), used_bytes, "{name}");
    }
}

/// SSOT invalidation of a triangle mesh (#210): every fill, contour, value
/// and colourbar edit is drawn on the first frame exactly as a fresh
/// renderer draws it, is visible, and re-uploads nothing; the mesh's group 2
/// is rebuilt from its signature, the pool untouched.
#[test]
fn tri_mesh_edits_refresh_first_frame_without_reupload() {
    use crate::data_config::{
        ContourConfig, DataLineStyleConfig, FieldFillConfig, FillMode, Shading,
    };
    use crate::tri_mesh::{MeshValueLocation, MeshValues, TriMeshSeries};
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = baseline_with_colorbar();
    config.legend.visible = false;
    let mut series = declaration("mesh", "mx", "my");
    series.render_type = DataRenderType::TriMesh(TriMeshSeries {
        triangles: "tris".into(),
        values: MeshValues {
            column: "mv".into(),
            at: MeshValueLocation::Vertices,
        },
        fill: Some(FieldFillConfig {
            mode: FillMode::Continuous,
            shading: Shading::Interpolated,
            opacity: 1.0,
        }),
        contour: None,
        surface: None,
    });
    let add_mesh = |r: &mut Renderer| {
        let column = |v: Vec<f64>| crate::data::Column { data: v };
        r.add_column("mx", &column(vec![0.15, 0.95, 0.95, 0.15, 0.55]))
            .unwrap();
        r.add_column("my", &column(vec![0.15, 0.15, 0.95, 0.95, 0.55]))
            .unwrap();
        r.add_column("mv", &column(vec![0.1, 0.4, 0.9, 0.3, 0.6]))
            .unwrap();
        r.add_column("faces", &column(vec![0.2, 0.4, 0.6, 0.8]))
            .unwrap();
        r.upsert_triangles(
            "tris",
            &crate::TriangleSource {
                indices: &crate::data::Column {
                    data: vec![0u32, 1, 4, 1, 2, 4, 2, 3, 4, 3, 0, 4],
                },
            },
        )
        .unwrap();
    };
    let fixture_with_mesh = |c: &Config, s: &SeriesConfig| {
        let (mut r, id) = fixture(c, true, false);
        add_mesh(&mut r);
        r.set_chart_series(id, vec![s.clone()]).unwrap();
        (r, id)
    };
    let (mut r, id) = fixture_with_mesh(&config, &series);
    let (mut previous, _) = frame(&mut r, id, true);
    let generation = r.pool.generation();
    let used_bytes = r.pool.used_bytes();
    fn mesh(s: &mut SeriesConfig) -> &mut TriMeshSeries {
        match &mut s.render_type {
            DataRenderType::TriMesh(mesh) => mesh,
            _ => unreachable!(),
        }
    }
    let series_edits: [(&str, fn(&mut TriMeshSeries)); 6] = [
        ("opacity", |m| m.fill.as_mut().unwrap().opacity = 0.5),
        ("contours", |m| {
            m.contour = Some(ContourConfig {
                levels: vec![0.25, 0.5, 0.75],
                line: DataLineStyleConfig {
                    line_width: 2.0,
                    ..DataLineStyleConfig::default()
                },
                per_level_color: None,
                labels: None,
            })
        }),
        ("contour width", |m| {
            m.contour.as_mut().unwrap().line.line_width = 4.0
        }),
        ("bands", |m| m.fill.as_mut().unwrap().mode = FillMode::Bands),
        ("contours only", |m| m.fill = None),
        ("face values", |m| {
            m.contour = None;
            m.fill = Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            });
            m.values = MeshValues {
                column: "faces".into(),
                at: MeshValueLocation::Faces,
            };
        }),
    ];
    for (name, apply) in series_edits {
        apply(mesh(&mut series));
        r.set_chart_series(id, vec![series.clone()]).unwrap();
        let (actual, _) = frame(&mut r, id, true);
        let (mut fresh, fresh_id) = fixture_with_mesh(&config, &series);
        let (expected, _) = frame(&mut fresh, fresh_id, true);
        assert_eq!(
            actual, expected,
            "{name}: first frame must match a fresh renderer"
        );
        assert_ne!(actual, previous, "{name}: the edit must be visible");
        previous = actual;
        assert_eq!(r.pool.generation(), generation, "{name}: no re-upload");
        assert_eq!(r.pool.used_bytes(), used_bytes, "{name}");
    }
    let config_edits: [(&str, fn(&mut Config)); 3] = [
        ("colormap", |c| {
            c.colorbar.as_mut().unwrap().colormap = crate::ColorMap::Custom {
                stops: vec![
                    Color::new(0.0, 0.0, 1.0, 1.0),
                    Color::new(1.0, 1.0, 0.0, 1.0),
                ],
            };
        }),
        ("colorbar range", |c| {
            c.colorbar.as_mut().unwrap().axis.max = 2.0
        }),
        ("x range", |c| c.bottom_x.max = 2.0),
    ];
    for (name, apply) in config_edits {
        apply(&mut config);
        r.set_chart_config(id, config.clone()).unwrap();
        let (actual, _) = frame(&mut r, id, true);
        let (mut fresh, fresh_id) = fixture_with_mesh(&config, &series);
        let (expected, _) = frame(&mut fresh, fresh_id, true);
        assert_eq!(
            actual, expected,
            "{name}: first frame must match a fresh renderer"
        );
        assert_ne!(actual, previous, "{name}: the edit must be visible");
        previous = actual;
        assert_eq!(r.pool.generation(), generation, "{name}: no re-upload");
        assert_eq!(r.pool.used_bytes(), used_bytes, "{name}");
    }
}

/// SSOT invalidation of a streamline series' `arrows` and `color_map`: each
/// edit is a data declaration change whose first frame matches a fresh
/// renderer's, is visible, re-uploads nothing, and an identical
/// re-declaration keeps the frame.
#[test]
fn streamline_arrow_and_colour_map_edits_refresh_first_frame_without_reupload() {
    use crate::data_config::{DataLineStyleConfig, PointColorMap, StreamlineArrows};
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = baseline_with_colorbar();
    config.legend.visible = false;
    let mut series = declaration("s", "x", "a");
    series.render_type = DataRenderType::Streamlines {
        line: DataLineStyleConfig {
            line_width: 1.5,
            ..DataLineStyleConfig::default()
        },
        arrows: None,
        color_map: None,
    };
    let (mut r, id) = fixture(&config, true, false);
    r.set_chart_series(id, vec![series.clone()]).unwrap();
    let (mut previous, _) = frame(&mut r, id, true);
    let generation = r.pool.generation();
    let used_bytes = r.pool.used_bytes();
    type Edit = fn(&mut Option<StreamlineArrows>, &mut Option<PointColorMap>);
    let edits: [(&str, Edit); 6] = [
        ("arrows", |arrows, _| {
            *arrows = Some(StreamlineArrows {
                spacing_px: 40.0,
                head_length_px: 9.0,
                head_width_px: 8.0,
            });
        }),
        ("arrow spacing", |arrows, _| {
            if let Some(arrows) = arrows {
                arrows.spacing_px = 25.0;
            }
        }),
        ("head width", |arrows, _| {
            if let Some(arrows) = arrows {
                arrows.head_width_px = 12.0;
            }
        }),
        ("colour map", |_, color_map| {
            *color_map = Some(PointColorMap { column: "a".into() });
        }),
        ("no arrows", |arrows, _| *arrows = None),
        ("no colour map", |_, color_map| *color_map = None),
    ];
    for (name, apply) in edits {
        let DataRenderType::Streamlines {
            arrows, color_map, ..
        } = &mut series.render_type
        else {
            unreachable!()
        };
        apply(arrows, color_map);
        let revisions = r.chart_states[&id].revisions;
        r.set_chart_series(id, vec![series.clone()]).unwrap();
        let updated = r.chart_states[&id].revisions;
        assert_ne!(
            updated.data, revisions.data,
            "{name}: a render_type edit is a data declaration change"
        );
        assert_eq!(updated.view, revisions.view, "{name}: no view replay");
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
        let revisions = r.chart_states[&id].revisions;
        r.set_chart_series(id, vec![series.clone()]).unwrap();
        assert_eq!(r.chart_states[&id].revisions.data, revisions.data, "{name}");
        assert_eq!(frame(&mut r, id, true).0, expected, "{name}: stable");
    }
}

/// SSOT invalidation of a 3D point vector (#211). Each series edit (`w`,
/// `z_column`, the scale, the pivot, the widths, the head, `min_length_px`
/// and the colour map) is a data declaration change with no view replay
/// whose first frame matches a fresh renderer's, is visible, re-uploads
/// nothing and is stable when declared again. Each camera, `right_y` and
/// `picked_data` edit is a config change whose first frame matches a fresh
/// renderer's, is visible and keeps the series' scratch: its sources are
/// the same allocation, so group 2 was not rebuilt.
#[test]
fn vector_3d_edits_refresh_first_frame_without_reupload() {
    use crate::data_config::{
        VectorColorMap, VectorComponents, VectorPivot, VectorScale, VectorStyle,
    };
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = baseline_with_colorbar();
    config.legend.visible = false;
    config.set_view_3d(Some(crate::view3d::View3d::default()));
    let mut series = declaration("s", "x", "a");
    series.z_column = Some("zc".into());
    series.render_type = DataRenderType::Vector {
        components: VectorComponents::Points {
            u: "u".into(),
            v: "v".into(),
            w: Some("w".into()),
        },
        style: VectorStyle {
            color: Color::new(0.8, 0.1, 0.1, 1.0),
            width_px: 2.0,
            scale: VectorScale::Data { factor: 0.15 },
            pivot: VectorPivot::Tail,
            head_length_px: 8.0,
            head_width_px: 7.0,
            min_length_px: 0.0,
            color_map: None,
        },
    };
    let add_columns = |r: &mut Renderer| {
        let column = |v: [f32; 8]| crate::Column { data: v.to_vec() };
        for (id, data) in [
            ("zc", [0.3, 0.5, 0.7, 0.4, 0.6, 0.3, 0.5, 0.4]),
            ("zc2", [0.6, 0.3, 0.4, 0.7, 0.3, 0.5, 0.6, 0.7]),
            ("u", [1.0, -0.5, 0.2, 0.8, -1.0, 0.3, 0.5, 0.1]),
            ("v", [0.2, 1.0, -0.8, 0.1, 0.4, -0.2, 0.6, 0.3]),
            ("w", [0.5, -0.3, 0.9, -1.0, 0.2, 0.7, -0.4, 0.1]),
            ("w2", [-0.6, 0.8, -0.2, 0.4, -0.9, 0.1, 0.5, -0.3]),
        ] {
            r.add_column(id, &column(data)).unwrap();
        }
    };
    // The fixture registers a flat line series, which a 3D chart refuses:
    // register flat, declare the arrows, then turn the chart 3D.
    let fixture_with_arrows = |c: &Config, s: &SeriesConfig| {
        let mut flat = c.clone();
        flat.set_view_3d(None);
        let (mut r, id) = fixture(&flat, true, false);
        add_columns(&mut r);
        r.set_chart_series(id, vec![s.clone()]).unwrap();
        r.set_chart_config(id, c.clone()).unwrap();
        (r, id)
    };
    let (mut r, id) = fixture_with_arrows(&config, &series);
    let (mut previous, _) = frame(&mut r, id, true);
    let generation = r.pool.generation();
    let used_bytes = r.pool.used_bytes();

    type SeriesEdit = fn(&mut SeriesConfig);
    fn style(series: &mut SeriesConfig) -> &mut VectorStyle {
        match &mut series.render_type {
            DataRenderType::Vector { style, .. } => style,
            _ => unreachable!(),
        }
    }
    let series_edits: [(&str, SeriesEdit); 10] = [
        ("w", |s| {
            if let DataRenderType::Vector {
                components: VectorComponents::Points { w, .. },
                ..
            } = &mut s.render_type
            {
                *w = Some("w2".into());
            }
        }),
        ("z_column", |s| s.z_column = Some("zc2".into())),
        ("data factor", |s| {
            style(s).scale = VectorScale::Data { factor: 0.25 }
        }),
        ("pixels", |s| {
            style(s).scale = VectorScale::Pixels { px_per_unit: 40.0 }
        }),
        ("pivot", |s| style(s).pivot = VectorPivot::Middle),
        ("width", |s| style(s).width_px = 4.0),
        ("head length", |s| style(s).head_length_px = 14.0),
        ("head width", |s| style(s).head_width_px = 12.0),
        ("min length", |s| style(s).min_length_px = 30.0),
        ("colour map", |s| {
            style(s).color_map = Some(VectorColorMap::Column("zc".into()))
        }),
    ];
    for (name, apply) in series_edits {
        apply(&mut series);
        let revisions = r.chart_states[&id].revisions;
        r.set_chart_series(id, vec![series.clone()]).unwrap();
        let updated = r.chart_states[&id].revisions;
        assert_ne!(
            updated.data, revisions.data,
            "{name}: a render_type edit is a data declaration change"
        );
        assert_eq!(updated.view, revisions.view, "{name}: no view replay");
        let (actual, _) = frame(&mut r, id, true);
        let (mut fresh, fresh_id) = fixture_with_arrows(&config, &series);
        let (expected, _) = frame(&mut fresh, fresh_id, true);
        assert_eq!(
            actual, expected,
            "{name}: first frame must match a fresh renderer"
        );
        assert_ne!(actual, previous, "{name}: the edit must be visible");
        previous = actual;
        assert_eq!(r.pool.generation(), generation, "{name}: no re-upload");
        assert_eq!(r.pool.used_bytes(), used_bytes, "{name}");
        let revisions = r.chart_states[&id].revisions;
        r.set_chart_series(id, vec![series.clone()]).unwrap();
        assert_eq!(r.chart_states[&id].revisions.data, revisions.data, "{name}");
        assert_eq!(frame(&mut r, id, true).0, expected, "{name}: stable");
    }

    let config_edits: [(&str, fn(&mut Config)); 7] = [
        ("orbit_view", |c| {
            c.orbit_view(40.0, 15.0);
        }),
        ("zoom_view", |c| {
            c.zoom_view(1.4);
        }),
        ("projection", |c| {
            c.view_3d_mut().unwrap().projection = crate::view3d::Projection3d::Perspective;
        }),
        ("box_aspect", |c| {
            c.view_3d_mut().unwrap().box_aspect = Some([1.0, 0.7, 0.5]);
        }),
        ("right_y range", |c| {
            c.right_y.min = 0.0;
            c.right_y.max = 1.5;
        }),
        ("right_y scale", |c| {
            c.right_y.min = 0.1;
            c.right_y.scale = AxisScale::Logarithmic;
        }),
        ("picked_data", |c| {
            c.picked_data = Some(crate::DataSelectionsConfig {
                refs: vec![crate::PickedDataRef::Point {
                    source_id: None,
                    series_id: "s".into(),
                    point_index: 2,
                }],
                ..crate::DataSelectionsConfig::default()
            });
        }),
    ];
    for (name, apply) in config_edits {
        let sources = Arc::clone(&r.vector_cache["s"].sources);
        apply(&mut config);
        r.set_chart_config(id, config.clone()).unwrap();
        let (actual, _) = frame(&mut r, id, true);
        let (mut fresh, fresh_id) = fixture_with_arrows(&config, &series);
        let (expected, _) = frame(&mut fresh, fresh_id, true);
        assert_eq!(
            actual, expected,
            "{name}: first frame must match a fresh renderer"
        );
        assert_ne!(actual, previous, "{name}: the edit must be visible");
        previous = actual;
        assert!(
            Arc::ptr_eq(&sources, &r.vector_cache["s"].sources),
            "{name}: the vector scratch is reused"
        );
        assert_eq!(r.pool.generation(), generation, "{name}: no re-upload");
        assert_eq!(r.pool.used_bytes(), used_bytes, "{name}");
    }
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
    use crate::config::{DataSelectionsConfig, PickedDataRef};
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let cases: &[(&str, fn(&mut DataSelectionsConfig))] = &[
        ("visible", |s| s.visible = false),
        ("highlight_color", |s| {
            s.highlight_color = Color::new(0.0, 0.0, 1.0, 1.0)
        }),
        ("outline_width_px", |s| s.outline_width_px = 5.0),
        ("point_radius_extra_px", |s| s.point_radius_extra_px = 8.0),
        ("point_index", |s| {
            if let PickedDataRef::Point { point_index, .. } = &mut s.refs[0] {
                *point_index = 5;
            }
        }),
        ("refs.clear", |s| s.refs.clear()),
    ];
    let mut base = baseline();
    base.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "s".into(),
            point_index: 2,
        }],
        ..Default::default()
    });
    let edited = |name: &str, edit: fn(&mut DataSelectionsConfig)| {
        let mut c = base.clone();
        edit(c.picked_data.as_mut().unwrap());
        assert_ne!(c, base, "{name} must edit the selection");
        c
    };
    let series = || {
        let mut series = declaration("s", "x", "a");
        let line = super::super::extract_line(&series.render_type)
            .unwrap()
            .into_owned();
        series.render_type = DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(DataScatterStyleConfig {
                point_color: Color::BLACK,
                point_size: 4.0,
                point_shape: ScatterShape::CircleFilled,
                point_style_index_column: None,
                point_style_table: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            }),
            line: Some(line),
            errors: None,
            stem: None,
        });
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

/// SSOT invalidation of a Feynman diagram (#234): topology, position,
/// label, bend, style and picked-part edits of the series, and axis, area
/// and selection edits of the chart, are drawn on the first frame exactly
/// as a fresh renderer draws them and are visible; the canvas re-rasters
/// through its signature and the column pool is untouched.
#[test]
fn feynman_edits_refresh_first_frame_without_reupload() {
    use crate::feynman::{FeynmanSeries, FeynmanVertex, ParticleLine, Propagator, VertexRole};
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = baseline();
    config.legend.visible = false;
    let vertex = |id: &str, role, at: [f64; 2]| {
        let mut v = FeynmanVertex::new(id, role);
        v.at = Some(at);
        v
    };
    let mut series = declaration("d", "", "");
    series.x_column = String::new();
    series.y_column = String::new();
    series.render_type = DataRenderType::Feynman(FeynmanSeries {
        vertices: vec![
            vertex("in", VertexRole::Incoming, [0.15, 0.6]),
            vertex("a", VertexRole::Internal, [0.4, 0.55]),
            vertex("b", VertexRole::Internal, [0.7, 0.55]),
            vertex("out", VertexRole::Outgoing, [0.95, 0.6]),
        ],
        propagators: vec![
            Propagator::new("p", "in", "a", ParticleLine::Fermion),
            Propagator::new("g", "a", "b", ParticleLine::Photon),
            Propagator::new("q", "b", "out", ParticleLine::Fermion),
        ],
        ..FeynmanSeries::default()
    });
    let fixture_with = |c: &Config, s: &SeriesConfig| {
        let (mut r, id) = fixture(c, true, false);
        r.set_chart_series(id, vec![s.clone()]).unwrap();
        (r, id)
    };
    let (mut r, id) = fixture_with(&config, &series);
    let (mut previous, _) = frame(&mut r, id, true);
    let generation = r.pool.generation();
    let used_bytes = r.pool.used_bytes();
    fn diagram(s: &mut SeriesConfig) -> &mut FeynmanSeries {
        s.feynman_mut().unwrap()
    }
    let series_edits: [(&str, fn(&mut FeynmanSeries)); 7] = [
        ("a gluon joins", |d| {
            d.propagators
                .push(Propagator::new("glue", "a", "b", ParticleLine::Gluon))
        }),
        ("a vertex moves", |d| d.vertices[1].at = Some([0.35, 0.3])),
        ("a label", |d| {
            d.propagators[1].label = Some(RichText::plain("γ", Color::BLACK, 14.0, ""))
        }),
        ("a label moves", |d| {
            d.propagators[1].label_offset = [6.0, -4.0]
        }),
        ("a bend", |d| d.propagators[0].bend = Some(60.0)),
        ("the line colour", |d| {
            d.style.color = Color::from_rgb8(200, 30, 30)
        }),
        ("the line width", |d| d.style.width = 3.0),
    ];
    for (name, apply) in series_edits {
        apply(diagram(&mut series));
        r.set_chart_series(id, vec![series.clone()]).unwrap();
        let (actual, _) = frame(&mut r, id, true);
        let (mut fresh, fresh_id) = fixture_with(&config, &series);
        let (expected, _) = frame(&mut fresh, fresh_id, true);
        assert_eq!(
            actual, expected,
            "{name}: first frame must match a fresh renderer"
        );
        assert_ne!(actual, previous, "{name}: the edit must be visible");
        previous = actual;
        assert_eq!(r.pool.generation(), generation, "{name}: no re-upload");
        assert_eq!(r.pool.used_bytes(), used_bytes, "{name}");
    }
    let config_edits: [(&str, fn(&mut Config)); 5] = [
        ("x range", |c| c.bottom_x.max = 1.4),
        ("y range", |c| c.left_y.min = -0.2),
        ("a picked vertex", |c| {
            c.picked_data = Some(crate::config::DataSelectionsConfig {
                refs: vec![crate::config::PickedDataRef::FeynmanVertex {
                    source_id: None,
                    series_id: "d".into(),
                    vertex_index: 1,
                }],
                ..Default::default()
            })
        }),
        ("a picked label", |c| {
            c.picked_data = Some(crate::config::DataSelectionsConfig {
                refs: vec![crate::config::PickedDataRef::FeynmanLabel {
                    source_id: None,
                    series_id: "d".into(),
                    part: crate::feynman::FeynmanLabelPart::Propagator,
                    index: 1,
                }],
                ..Default::default()
            })
        }),
        ("a narrower plot", |c| c.left_y.out_margin = 60.0),
    ];
    for (name, apply) in config_edits {
        apply(&mut config);
        r.set_chart_config(id, config.clone()).unwrap();
        let (actual, _) = frame(&mut r, id, true);
        let (mut fresh, fresh_id) = fixture_with(&config, &series);
        let (expected, _) = frame(&mut fresh, fresh_id, true);
        assert_eq!(
            actual, expected,
            "{name}: first frame must match a fresh renderer"
        );
        assert_ne!(actual, previous, "{name}: the edit must be visible");
        previous = actual;
        assert_eq!(r.pool.generation(), generation, "{name}: no re-upload");
        assert_eq!(r.pool.used_bytes(), used_bytes, "{name}");
    }
}

/// A monotonic series (#271). Its line's colour, width (a thin one and a
/// wide one: the columns kept beyond the plot's edges are fixed) and dash
/// are style: each edit draws from the envelope the last full call built,
/// without the source and without writing its columns again, and the first
/// frame equals a fresh renderer's. The x range moves the x-to-pixel
/// mapping: the chart's document editor refuses it and changes nothing,
/// and the full call sets it. The channel names other samples of the same group's
/// data, so it needs no source; a group without data draws nothing until
/// its full call. The y range is not mapping. No frame ever fails for a
/// missing source.
#[test]
fn monotonic_edits_refresh_first_frame_or_go_through_the_full_call() {
    use crate::monotonic::{MappingChange, MonotonicSlices};
    use crate::renderer::{MonotonicChartId, MonotonicRange};
    // A monotonic chart beside the fixture's columns (#271).
    let monotonic = |config: &Config| {
        let (mut r, _) = fixture(config, true, false);
        let id = r
            .register_monotonic_chart(config.clone(), Vec::new())
            .unwrap()
            .0;
        (r, id)
    };
    let edit = |r: &mut Renderer, id: ChartId, config: Config| {
        r.monotonic_chart(MonotonicChartId(id))
            .unwrap()
            .edit_config(|editor| editor.assign(config))
            .unwrap()
    };
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = baseline();
    config.legend.visible = false;
    config.bottom_x.min = 0.0;
    config.bottom_x.max = 19_999.0;
    config.bottom_x.major_spacing = 5_000.0;
    let a: Vec<f64> = (0..20_000)
        .map(|i| 0.5 + 0.4 * (i as f64 * 1e-3).sin())
        .collect();
    let b: Vec<f64> = a.iter().map(|v| 1.1 - v).collect();
    let channels = [a.as_slice(), b.as_slice()];
    let source = MonotonicSlices::new(0.0, 1.0, &channels);
    // The full call, at the x range the chart's document has.
    let supply = |r: &mut Renderer, id: ChartId, group: &str| {
        let (min, max) = r.chart_config(id).unwrap().bottom_x.data_range();
        r.set_monotonic(id, group, &source, MonotonicRange::X { min, max })
            .unwrap();
    };
    let fresh_frame = |config: &Config, series: &SeriesConfig, group: &str| {
        let (mut fresh, fresh_id) = monotonic(config);
        fresh
            .set_chart_series_unreported(fresh_id, vec![series.clone()])
            .unwrap();
        supply(&mut fresh, fresh_id, group);
        frame(&mut fresh, fresh_id, true).0
    };
    let (mut r, id) = monotonic(&config);
    let mut series = SeriesConfig::monotonic_line("s", "g", 0);
    r.set_chart_series_unreported(id, vec![series.clone()])
        .unwrap();
    // No data yet: the series draws nothing, and the frame draws.
    assert_eq!(
        r.needs_source(id).unwrap()[0].change,
        MappingChange::NotBuilt
    );
    let empty = frame(&mut r, id, true).0;
    supply(&mut r, id, "g");
    let mut previous = frame(&mut r, id, true).0;
    assert_ne!(previous, empty, "the full call draws the series");
    let epoch = r.pool.allocation_epoch("s.monotonic.x");

    // Style: drawn from the envelope as it is.
    let styles: [(&str, fn(&mut crate::data_config::DataLineStyleConfig)); 4] = [
        ("colour", |line| {
            line.line_color = Color::new(0.8, 0.1, 0.1, 1.0)
        }),
        ("width", |line| line.line_width = 3.0),
        ("wide width", |line| line.line_width = 40.0),
        ("dash", |line| {
            line.line_style = crate::line::LineStylePreset::Dash;
        }),
    ];
    for (name, apply) in styles {
        let DataRenderType::MonotonicLine(monotonic) = &mut series.render_type else {
            unreachable!()
        };
        apply(&mut monotonic.line);
        r.set_chart_series_unreported(id, vec![series.clone()])
            .unwrap();
        assert!(r.needs_source(id).unwrap().is_empty(), "{name}");
        let actual = frame(&mut r, id, true).0;
        assert_eq!(actual, fresh_frame(&config, &series, "g"), "{name}");
        assert_ne!(actual, previous, "{name}: the edit must be visible");
        assert_eq!(
            r.pool.allocation_epoch("s.monotonic.x"),
            epoch,
            "{name}: no column written"
        );
        previous = actual;
    }

    // The channel reads the group's other samples: no source. A group
    // without data draws nothing until its full call.
    let DataRenderType::MonotonicLine(monotonic) = &mut series.render_type else {
        unreachable!()
    };
    monotonic.channel = 1;
    r.set_chart_series_unreported(id, vec![series.clone()])
        .unwrap();
    let actual = frame(&mut r, id, true).0;
    assert_eq!(actual, fresh_frame(&config, &series, "g"), "channel");
    assert_ne!(actual, previous, "channel: the edit must be visible");
    let DataRenderType::MonotonicLine(monotonic) = &mut series.render_type else {
        unreachable!()
    };
    monotonic.group = "h".into();
    r.set_chart_series_unreported(id, vec![series.clone()])
        .unwrap();
    assert_eq!(r.needs_source(id).unwrap().len(), 1, "group");
    assert_eq!(frame(&mut r, id, true).0, empty, "a group without data");
    supply(&mut r, id, "h");
    assert_eq!(
        frame(&mut r, id, true).0,
        fresh_frame(&config, &series, "h"),
        "group"
    );

    // The config: the x range is mapping, the y range is not.
    let mut edited = r.chart_config(id).unwrap().clone();
    edited.left_y.max = 1.4;
    edit(&mut r, id, edited.clone()).unwrap();
    assert!(r.needs_source(id).unwrap().is_empty(), "y range");
    let ranged = frame(&mut r, id, true).0;
    assert_eq!(ranged, fresh_frame(&edited, &series, "h"), "y range");
    edited.bottom_x.max = 9_999.0;
    assert_eq!(
        edit(&mut r, id, edited.clone()),
        Err(crate::RuleKind::MonotonicMappingEdit),
        "the editor refuses an x range"
    );
    assert_eq!(
        frame(&mut r, id, true).0,
        ranged,
        "a refused range draws as before"
    );
    r.set_monotonic(
        id,
        "h",
        &source,
        MonotonicRange::X {
            min: 0.0,
            max: 9_999.0,
        },
    )
    .unwrap();
    let x = &r.chart_config(id).unwrap().bottom_x;
    assert_eq!(
        (x.min, x.max),
        (0.0, 9_999.0),
        "the full call sets the range"
    );
    let edited = r.chart_config(id).unwrap().clone();
    assert_eq!(
        frame(&mut r, id, true).0,
        fresh_frame(&edited, &series, "h"),
        "x range"
    );
}

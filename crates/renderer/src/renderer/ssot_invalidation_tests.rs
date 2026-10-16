//! Render first, edit exactly one SSOT property, then inspect the FIRST updated
//! frame and the work needed to produce it. The pixel oracle is a fresh renderer
//! with the final state, never an invalidation predicate from the implementation.
use super::*;
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
/// chrome must be rasterized again (the `raster` revision); `differs` is
/// whether the config changed at all (the `desired` and `config` revisions).
struct Edit {
    name: &'static str,
    apply: fn(&mut Config),
    work: Work,
    differs: bool,
    raster: bool,
    pixels: Pixels,
}

fn edits() -> Vec<Edit> {
    let mut cases = Vec::new();
    macro_rules! edit {
        ($path:ident $(.$field:ident)*, $value:expr, $work:ident) => {
            cases.push(Edit { name: stringify!($path $(.$field)*),
                apply: |c| c.$path $(.$field)* = $value,
                work: Work::$work, differs: true, raster: true, pixels: Pixels::Changed });
        };
    }
    cases.push(Edit {
        name: "identical config",
        apply: |_| {},
        work: Work::Prefix,
        differs: false,
        raster: false,
        pixels: Pixels::Unchanged,
    });
    // Layer foundation: each field is an edit of its own layer only. Pixels,
    // the axis raster, data work and the data job must stay exactly as they were.
    cases.push(Edit {
        name: "active_layer",
        apply: |c| c.active_layer = crate::LayerRef::Annotations,
        work: Work::Prefix,
        differs: true,
        raster: false,
        pixels: Pixels::Unchanged,
    });
    cases.push(Edit {
        name: "annotations.visible",
        apply: |c| c.annotations.visible = false,
        work: Work::Prefix,
        differs: true,
        raster: false,
        pixels: Pixels::Unchanged,
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
        pixels: Pixels::Any,
    });
    edit!(bottom_x.label_style.significant_digits, 5, Prefix);
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
            pixels: Pixels::Changed,
        });
    }
    edit!(draw_style, DrawStyle::Sketch(Default::default()), Replay);
    edit!(draw_style, DrawStyle::Milkyway(Default::default()), Replay);
    edit!(
        draw_style,
        DrawStyle::Constellation(Default::default()),
        Replay
    );
    // Adding a colourbar reserves its band, so the data area moves.
    cases.push(Edit {
        name: "colorbar (None -> Some)",
        apply: |c| c.colorbar = Some(crate::default::default_colorbar_options()),
        work: Work::Packed,
        differs: true,
        raster: true,
        pixels: Pixels::Changed,
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
                pixels: Pixels::$pixels,
            });
        };
    }
    cases.push(Edit {
        name: "colorbar (Some -> None)",
        apply: |c| c.colorbar = None,
        work: Work::Packed,
        differs: true,
        raster: true,
        pixels: Pixels::Changed,
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
    ];
    eprintln!(
        "SSOT: {} single-property cases, resident={resident}, cache={cache}",
        suites.iter().map(|(_, cases)| cases.len()).sum::<usize>()
    );
    for (c, edit) in suites
        .into_iter()
        .flat_map(|(c, cases)| cases.into_iter().map(move |edit| (c.clone(), edit)))
    {
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
        active_layer: _,  // layer only: no raster, no pixels
        annotations: _,   // layer only: no raster, no pixels
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
    let crate::AnnotationLayer { visible: _ } = crate::AnnotationLayer::default();
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
#[test]
fn ssot_edits_refresh_stream_first_frame_without_excess_work() {
    matrix(false, false);
}
#[test]
fn ssot_edits_reuse_packed_rows_only_when_valid() {
    matrix(false, true);
}

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
    for (resident, cache) in [(true, false), (false, false), (false, true)] {
        for &(name, edit) in cases {
            let mut c = baseline();
            c.picked_points = Some(PickedPointsConfig {
                refs: vec![PickedPointRef {
                    source_id: None,
                    series_id: "s".into(),
                    point_index: 2,
                }],
                ..Default::default()
            });
            let setup = |c: &Config, cache: bool| {
                let (mut r, id) = fixture(c, resident, cache);
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
                    },
                };
                r.set_chart_series(id, vec![series]).unwrap();
                (r, id)
            };
            let (mut r, id) = setup(&c, cache);
            let (before, _) = frame(&mut r, id, resident);
            let job = r.active_stream_job(id);
            let prefix = job.map(|j| r.chart_stream_prefix_for_test(j).unwrap());
            let data_pixels = prefix.as_ref().map(|t| read_draw_target(&r, t));
            edit(c.picked_points.as_mut().unwrap());
            let revisions = r.chart_states[&id].revisions;
            r.set_chart_config(id, c.clone()).unwrap();
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
            let (mut fresh, fresh_id) = setup(&c, false);
            let (expected, _) = frame(&mut fresh, fresh_id, resident);
            assert_eq!(
                actual, expected,
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

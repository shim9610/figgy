//! Snap guides while dragging a decoration (#440): the guide is selection
//! chrome on the overlay that draws the selection box. It shows while the
//! drag is in progress, is gone after the release, and no export (PNG at 1x
//! and 2x, SVG) ever draws it.
use super::*;
use crate::HitMap;
use crate::layout::NudgeResult;
use crate::layout::{ChartArea, DragSnap, Rect, SnapGuides};
use crate::text::RichText;

const PANEL: Rect = Rect {
    x: 0,
    y: 0,
    width: 320,
    height: 240,
};

fn renderer() -> Renderer {
    let (device, queue) = data_render::shared_device().unwrap();
    Renderer::try_new_with_sample_count(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
        1,
    )
    .unwrap()
}

fn titled_config() -> Config {
    let mut c = crate::default::default_config();
    c.chart_area = ChartArea(PANEL);
    c.chart_title.visible = true;
    c.chart_title.text = RichText::plain("Snap", Color::BLACK, 14.0, "");
    c.chart_title.top_margin = 24.0;
    c.layout_state = crate::LayoutState::Fixed;
    c
}

fn frame(r: &mut Renderer, id: ChartId, view: &mut ChartView) -> Vec<u8> {
    let prepared = r
        .prepare_registered(&mut [RegisteredChartDrawItem { chart_id: id, view }])
        .unwrap();
    paint_frame_pixels(r, &prepared, 1)
}

/// Pixels of the snap guide's magenta (`SNAP_GUIDE_COLOR`): no title,
/// axis, grid or selection-box pixel is that colour.
fn guide_pixels(rgba: &[u8]) -> usize {
    rgba.chunks_exact(4)
        .filter(|px| px[3] > 0 && px[0] > 170 && px[1] < 110 && px[2] > 90 && px[2] < px[0])
        .count()
}

#[test]
fn a_drag_shows_its_guide_until_release_and_no_export_draws_it() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut r = renderer();
    let config = titled_config();
    let id = r.register_chart(config.clone(), Vec::new()).unwrap();
    let mut view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    let map = HitMap::for_active_layer(&config);
    let title = map.find("chart_title").expect("the title is registered");
    r.set_chart_selection(id, Some(title)).unwrap();

    // Drag the title off centre, then back to 3 px short of the chart
    // centre: the drag snaps onto it.
    let measure = crate::CpuTextMeasure::for_style(&config.draw_style);
    let mut moved = config.clone();
    let areas = moved.snap_areas();
    let mut off = DragSnap::off();
    let _ = map.drag_snapped(
        title, &mut moved, &measure, None, 60.0, 0.0, areas, &mut off,
    );
    let bounds = |c: &Config| {
        use crate::select::SelectionGeometry;
        crate::select::ChartTitleElement
            .bounds(c, &measure)
            .unwrap()
    };
    let dx = 160.0 - bounds(&moved).center().0 - 3.0;
    let mut snap = DragSnap::new(1.0);
    assert_eq!(
        map.drag_snapped(title, &mut moved, &measure, None, dx, 0.0, areas, &mut snap),
        Some(NudgeResult::Moved)
    );
    assert!((bounds(&moved).center().0 - 160.0).abs() < 1e-3);
    let guides = snap.guides();
    assert!(guides.vertical.is_some(), "the snap shows a vertical guide");
    r.set_chart_config(id, moved.clone()).unwrap();

    let released = frame(&mut r, id, &mut view);
    assert_eq!(
        guide_pixels(&released),
        0,
        "no guide before the drag shows one"
    );

    // While the drag is in progress the guide is drawn.
    r.set_chart_snap_guides(id, guides).unwrap();
    let dragging = frame(&mut r, id, &mut view);
    assert!(
        guide_pixels(&dragging) > 100,
        "the guide is drawn while dragging: {} px",
        guide_pixels(&dragging)
    );
    // Guides are chrome: the document is the same.
    assert_eq!(r.chart_config(id).unwrap(), &moved);

    // Exports never draw it, at 1x and 2x, PNG and SVG.
    let exports = |r: &mut Renderer| {
        let png = [1.0, 2.0].map(|scale| {
            r.export_rgba_blocking(ExportSource::Registered(id), ExportOptions::document(scale))
                .unwrap()
        });
        let svg = [1.0, 2.0].map(|scale| {
            r.export_svg_blocking(ExportSource::Registered(id), ExportOptions::document(scale))
                .unwrap()
        });
        (png, svg)
    };
    let (png_dragging, svg_dragging) = exports(&mut r);
    for image in &png_dragging {
        assert_eq!(guide_pixels(&image.rgba), 0, "a PNG export drew the guide");
    }

    // The release removes the guide; the frame is the one before.
    r.set_chart_snap_guides(id, SnapGuides::NONE).unwrap();
    assert_eq!(frame(&mut r, id, &mut view), released);
    let (png_released, svg_released) = exports(&mut r);
    for (dragging, released) in png_dragging.iter().zip(&png_released) {
        assert_eq!(
            dragging.rgba, released.rgba,
            "the export changed with a guide"
        );
    }
    assert_eq!(
        svg_dragging, svg_released,
        "the SVG export changed with a guide"
    );

    // A new selection drops guides left behind.
    r.set_chart_snap_guides(id, guides).unwrap();
    r.set_chart_selection(id, None).unwrap();
    r.set_chart_selection(id, Some(title)).unwrap();
    assert_eq!(guide_pixels(&frame(&mut r, id, &mut view)), 0);
}

#[test]
fn snap_guides_are_drawn_only_with_the_selection_box() {
    let config = titled_config();
    let measure = crate::CpuTextMeasure::for_style(&config.draw_style);
    let map = HitMap::for_active_layer(&config);
    let title = map.find("chart_title").unwrap();
    let selection = map.selection_box(title, &config, &measure).unwrap();
    assert!(selection.snap_guides().is_empty());
    let guides = SnapGuides {
        vertical: Some(crate::layout::SnapGuide {
            from: [160.0, 0.0],
            to: [160.0, 240.0],
        }),
        horizontal: None,
    };
    let raster = |boxes: &[crate::SelectionBox]| {
        crate::axis_render::try_raster_chart_layer_to_rgba_with_selection_at(
            &config,
            crate::axis_render::AxisLayerKind::Decoration,
            boxes,
            1.0,
        )
        .unwrap()
    };
    let with = selection.clone().with_snap_guides(guides);
    assert!(guide_pixels(&raster(std::slice::from_ref(&with))) > 100);
    assert_eq!(guide_pixels(&raster(std::slice::from_ref(&selection))), 0);
    assert_eq!(guide_pixels(&raster(&[])), 0);
}

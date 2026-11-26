//! Snap guides while dragging a radial decoration (#440, #465): the drag
//! goes through the common `HitMap::drag_snapped`, the guides are drawn
//! with the selection box of the interactive frame while the gesture lasts,
//! and no export (PNG at 1x and 2x, SVG) ever draws them.
use super::*;
use crate::layout::{DragSnap, NudgeResult, SnapGuides};
use crate::test_gpu::BlockOn as _;
use model::DecorationId;

const SIZE: (u32, u32) = (480, 360);

fn renderer() -> RadialRenderer {
    let (device, queue) = crate::test_gpu::device();
    RadialRenderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap()
}

fn chart() -> RadialChart {
    let mut c = RadialChart {
        slices: vec![
            RadialSlice::new("North", 35., crate::Color::from_rgb8(100, 145, 185)),
            RadialSlice::new("South", 65., crate::Color::from_rgb8(90, 170, 110)),
        ],
        labels: RadialLabels::Outside,
        ..Default::default()
    };
    c.set_title_text("Snap");
    c.format_labels();
    c
}

/// Pixels of the snap guide's magenta (`SNAP_GUIDE_COLOR`): no slice,
/// label, title or selection-box pixel of [`chart`] is that colour.
fn guide_pixels(rgba: &[u8]) -> usize {
    rgba.chunks_exact(4)
        .filter(|px| px[3] > 0 && px[0] > 170 && px[1] < 110 && px[2] > 90 && px[2] < px[0])
        .count()
}

/// The interactive frame of `chart` with `guides`, as a host draws it: the
/// frame, then the annotation overlay.
fn frame_pixels(r: &mut RadialRenderer, chart: &RadialChart, guides: SnapGuides) -> Vec<u8> {
    let frame = r.prepare_with_guides(chart, SIZE, 1.0, guides).unwrap();
    let annotations = r
        .prepare_annotations_with_guides(chart, SIZE, 1.0, guides)
        .unwrap();
    r.render_rgba_blocking(frame.size(), crate::Color::WHITE, |pass| {
        frame.draw(pass);
        annotations.draw_overlay(pass);
    })
    .unwrap()
    .rgba
}

fn title_bounds(r: &mut RadialRenderer, chart: &RadialChart) -> crate::layout::RectF {
    let frame = r.prepare(chart, SIZE, 1.0).unwrap();
    let hit = frame.decoration(&DecorationId::ChartTitle).unwrap();
    frame.decoration_bounds(chart, hit).unwrap()
}

#[test]
fn a_radial_title_drag_snaps_shows_its_guide_until_release_and_no_export_draws_it() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut r = renderer();
    let mut c = chart();
    c.decorations.selected = Some(DecorationId::ChartTitle);
    let centre = SIZE.0 as f32 / 2.0;

    // Off centre first, unsnapped; then back to 3 px short of the canvas
    // centre with a gesture: the drag snaps onto it.
    let frame = r.prepare(&c, SIZE, 1.0).unwrap();
    let title = frame.decoration(&DecorationId::ChartTitle).unwrap();
    let mut off = DragSnap::off();
    assert_eq!(
        frame.drag_decoration_snapped(&mut c, title, 60.0, 0.0, &mut off),
        Some(NudgeResult::Moved)
    );
    assert!(off.guides().is_empty(), "an unsnapped drag shows no guide");
    let dx = centre - title_bounds(&mut r, &c).center().0 - 3.0;
    let unsnapped = {
        // A drag without snapping keeps its old result (`drag_decoration`).
        let mut plain = c.clone();
        let mut also = c.clone();
        let frame = r.prepare(&c, SIZE, 1.0).unwrap();
        assert_eq!(
            frame.drag_decoration(&mut plain, title, dx, 0.0),
            Some(NudgeResult::Moved)
        );
        let mut off = DragSnap::off();
        assert_eq!(
            frame.drag_decoration_snapped(&mut also, title, dx, 0.0, &mut off),
            Some(NudgeResult::Moved)
        );
        assert_eq!(plain, also, "an off gesture moves as the plain drag");
        plain
    };
    let frame = r.prepare(&c, SIZE, 1.0).unwrap();
    let mut snap = DragSnap::new(1.0);
    assert_eq!(
        frame.drag_decoration_snapped(&mut c, title, dx, 0.0, &mut snap),
        Some(NudgeResult::Moved)
    );
    assert!(
        (title_bounds(&mut r, &c).center().0 - centre).abs() < 1e-3,
        "the title snaps to the canvas centre"
    );
    assert!(
        (title_bounds(&mut r, &unsnapped).center().0 - centre).abs() > 2.0,
        "the unsnapped drag stops short of it"
    );
    let guides = snap.guides();
    assert!(guides.vertical.is_some(), "the snap shows a vertical guide");

    // The snap is stored as the title's usual offset, and the document
    // round-trips as before: guides are not part of it.
    #[cfg(feature = "serde")]
    {
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("guide"), "{json}");
        let back: RadialChart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    let released = frame_pixels(&mut r, &c, SnapGuides::NONE);
    assert_eq!(guide_pixels(&released), 0, "no guide without a gesture");
    let dragging = frame_pixels(&mut r, &c, guides);
    assert!(
        guide_pixels(&dragging) > 100,
        "the guide is drawn while dragging: {} px",
        guide_pixels(&dragging)
    );
    // Without a decoration selection there is no selection box to draw
    // the guides with.
    let mut unselected = c.clone();
    unselected.decorations.selected = None;
    assert_eq!(guide_pixels(&frame_pixels(&mut r, &unselected, guides)), 0);

    // Exports never draw them, at 1x and 2x, PNG and SVG, whatever the
    // interactive frame last showed.
    let exports = |r: &mut RadialRenderer| {
        let png = [1.0, 2.0].map(|scale| {
            r.export_rgba(&c, SIZE, crate::ExportOptions::document(scale))
                .block_on()
                .unwrap()
        });
        let svg = [1.0, 2.0].map(|scale| {
            r.export_svg_blocking(&c, SIZE, crate::ExportOptions::document(scale))
                .unwrap()
        });
        (png, svg)
    };
    let _ = frame_pixels(&mut r, &c, guides);
    let (png_dragging, svg_dragging) = exports(&mut r);
    for image in &png_dragging {
        assert_eq!(guide_pixels(&image.rgba), 0, "a PNG export drew the guide");
    }
    // The release removes the guide; the frame is the one before.
    assert_eq!(frame_pixels(&mut r, &c, SnapGuides::NONE), released);
    let (png_released, svg_released) = exports(&mut r);
    for (dragging, released) in png_dragging.iter().zip(&png_released) {
        assert_eq!(dragging.rgba, released.rgba, "the export changed");
    }
    assert_eq!(svg_dragging, svg_released, "the SVG export changed");
}

#[test]
fn a_radial_label_snaps_to_another_decoration_and_the_modifier_turns_it_off() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut r = renderer();
    let mut c = chart();
    let label = DecorationId::RadialLabel(RadialTarget::main(0));
    c.decorations.selected = Some(label.clone());
    let frame = r.prepare(&c, SIZE, 1.0).unwrap();
    let hit = frame.decoration(&label).unwrap();
    let title = frame.decoration(&DecorationId::ChartTitle).unwrap();
    let title_top = frame.decoration_bounds(&c, title).unwrap().y;
    let bounds = frame.decoration_bounds(&c, hit).unwrap();
    // Up to 3 px below the title's top edge: the label's top snaps to it
    // (or to a nearer alignment), with a horizontal guide.
    let dy = title_top + 3.0 - bounds.y;
    let held = {
        let mut held = c.clone();
        let mut snap = DragSnap::new(1.0);
        // Alt / Option held: the drag follows the pointer exactly.
        snap.set_enabled(false);
        assert_eq!(
            frame.drag_decoration_snapped(&mut held, hit, 0.0, dy, &mut snap),
            Some(NudgeResult::Moved)
        );
        assert!(snap.guides().is_empty());
        let mut plain = c.clone();
        frame.drag_decoration(&mut plain, hit, 0.0, dy);
        assert_eq!(held, plain, "the modifier drag is the plain drag");
        held
    };
    let mut snap = DragSnap::new(1.0);
    assert_eq!(
        frame.drag_decoration_snapped(&mut c, hit, 0.0, dy, &mut snap),
        Some(NudgeResult::Moved)
    );
    assert!(snap.guides().horizontal.is_some(), "a horizontal guide");
    assert_ne!(c, held, "the snapped drag differs from the plain one");
    let moved = r.prepare(&c, SIZE, 1.0).unwrap();
    let after = moved.decoration_bounds(&c, hit).unwrap();
    // The label's top, centre or bottom sits on the guide, within the
    // reach of where the pointer put it.
    let line = snap.guides().horizontal.unwrap().from[1];
    assert!(
        [
            after.y,
            after.y + after.height / 2.0,
            after.y + after.height
        ]
        .iter()
        .any(|y| (y - line).abs() < 1e-2),
        "the label sits on the guide at y = {line}: {after:?}"
    );
    assert!((after.y - (bounds.y + dy)).abs() <= crate::layout::SNAP_THRESHOLD_PX);
    assert!(guide_pixels(&frame_pixels(&mut r, &c, snap.guides())) > 50);
    // The plot area does not snap.
    let area = frame.decoration(&DecorationId::DataArea).unwrap();
    let mut snap = DragSnap::new(1.0);
    let mut plot = chart();
    let _ = frame.drag_decoration_snapped(&mut plot, area, 1.0, 0.0, &mut snap);
    assert!(snap.guides().is_empty(), "the plot area shows no guide");
}

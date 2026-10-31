//! Selection-only pixel assertions shared by native and browser area tests.
use ::renderer::{RasterImage, SelectionBox, layout::RectF};

pub fn assert_selection_only(
    before: &RasterImage,
    selected: &RasterImage,
    selection: &SelectionBox,
    scale: f32,
    context: &str,
) {
    assert_selection_pixels(before, selected, selection, scale, None, context);
}

/// A lifted inside radial label switches from translated cached pixels to
/// fresh rasterization on selection. Its measured glyph region can change AA;
/// the border and every pixel outside that region still obey the same checks.
#[cfg(not(target_arch = "wasm32"))]
pub fn assert_selection_with_rerasterized_label(
    before: &RasterImage,
    selected: &RasterImage,
    selection: &SelectionBox,
    scale: f32,
    label: RectF,
    context: &str,
) {
    assert_selection_pixels(before, selected, selection, scale, Some(label), context);
    for image in [before, selected] {
        let white_glyphs = image
            .rgba
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, p)| {
                let x = (*i as u32 % image.width) as f32 / scale + 0.5 / scale;
                let y = (*i as u32 / image.width) as f32 / scale + 0.5 / scale;
                label.contains(x, y) && p[..3].iter().all(|c| *c > 200)
            })
            .count();
        assert!(
            white_glyphs > 20,
            "{context}: inside label glyphs disappeared"
        );
    }
}

fn assert_selection_pixels(
    before: &RasterImage,
    selected: &RasterImage,
    selection: &SelectionBox,
    scale: f32,
    rerasterized_label: Option<RectF>,
    context: &str,
) {
    assert_eq!(
        (before.width, before.height),
        (selected.width, selected.height)
    );
    let r = selection.rect;
    // Half the stroke plus one physical pixel for raster antialiasing.
    let halo = selection.stroke_width * 0.5 + 1.0 / scale;
    let bands = [
        RectF {
            x: r.x - halo,
            y: r.y - halo,
            width: r.width + 2.0 * halo,
            height: 2.0 * halo,
        },
        RectF {
            x: r.x - halo,
            y: r.y + r.height - halo,
            width: r.width + 2.0 * halo,
            height: 2.0 * halo,
        },
        RectF {
            x: r.x - halo,
            y: r.y - halo,
            width: 2.0 * halo,
            height: r.height + 2.0 * halo,
        },
        RectF {
            x: r.x + r.width - halo,
            y: r.y - halo,
            width: 2.0 * halo,
            height: r.height + 2.0 * halo,
        },
    ];
    let mut changed_blue = [0; 4];
    for (i, (old, new)) in before
        .rgba
        .chunks_exact(4)
        .zip(selected.rgba.chunks_exact(4))
        .enumerate()
    {
        if old == new {
            continue;
        }
        let x = (i as u32 % selected.width) as f32 / scale + 0.5 / scale;
        let y = (i as u32 / selected.width) as f32 / scale + 0.5 / scale;
        let on_border = bands.map(|b| b.contains(x, y));
        let on_handle = selection
            .handles
            .iter()
            .any(|h| h.expanded(halo).contains(x, y));
        assert!(
            on_border.iter().any(|v| *v)
                || on_handle
                || rerasterized_label
                    .is_some_and(|label| label.expanded(1.0 / scale).contains(x, y)),
            "{context}: selection changed unrelated pixel at ({x}, {y})"
        );
        if new[2] > 140 && i32::from(new[2]) - i32::from(new[0]) > 30 {
            for (edge, (count, on_edge)) in changed_blue.iter_mut().zip(on_border).enumerate() {
                // Corners from adjacent strokes and resize handles cannot stand
                // in for a missing edge. Inspect the middle of each border.
                let along_edge = if edge < 2 {
                    x > r.x + r.width * 0.2 && x < r.x + r.width * 0.8
                } else {
                    y > r.y + r.height * 0.2 && y < r.y + r.height * 0.8
                };
                *count += usize::from(on_edge && along_edge && !on_handle);
            }
        }
    }
    for (edge, count) in ["top", "bottom", "left", "right"]
        .into_iter()
        .zip(changed_blue)
    {
        assert!(
            count >= 4,
            "{context}: missing {edge} selection border ({count} changed blue pixels)"
        );
    }
}

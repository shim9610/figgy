//! Orthogonal text rotation must preserve the once-rasterized glyph coverage.
use renderer::{
    Color,
    raster::Canvas,
    text_render::{FontPolicy, draw_plain_text},
};

const SIZE: usize = 128;

fn text(canvas: &mut Canvas, origin: (f32, f32)) {
    draw_plain_text(
        canvas,
        "Response (%)",
        origin,
        Color::BLACK,
        "Liberation Sans",
        18.0,
        false,
        false,
        FontPolicy::Standard,
    );
}

// Independent pixel permutation: no rasterizer or image filtering in the oracle.
fn rotate_pixels(input: &[u8], clockwise: bool) -> Vec<u8> {
    let mut output = vec![0; input.len()];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) = if clockwise {
                (SIZE - 1 - y, x)
            } else {
                (y, SIZE - 1 - x)
            };
            output[(dy * SIZE + dx) * 4..(dy * SIZE + dx + 1) * 4]
                .copy_from_slice(&input[(y * SIZE + x) * 4..(y * SIZE + x + 1) * 4]);
        }
    }
    output
}

#[test]
fn left_and_right_titles_preserve_glyph_pixels_at_fractional_positions() {
    for clockwise in [false, true] {
        for phase in [0.0, 0.125, 0.25, 0.5, 0.875] {
            let mut horizontal = Canvas::new(SIZE as u32, SIZE as u32).unwrap();
            horizontal.translate(phase, 0.5);
            text(&mut horizontal, (7.0, 44.0));
            let expected = rotate_pixels(&horizontal.into_rgba(), clockwise);
            assert!(expected.chunks_exact(4).any(|p| p[3] == 255));
            let mut vertical = Canvas::new(SIZE as u32, SIZE as u32).unwrap();
            vertical.rotate_at(if clockwise { 90.0 } else { -90.0 }, 64.0, 64.0);
            vertical.translate(phase, 0.5);
            text(&mut vertical, (7.0, 44.0));
            assert!(
                vertical.into_rgba() == expected,
                "clockwise={clockwise}, phase={phase}"
            );
        }
    }
}

#[test]
fn orthogonal_text_clipping_matches_rotated_horizontal_clipping() {
    for clockwise in [false, true] {
        for origin in [(-8.25, 10.5), (70.25, 132.5)] {
            let mut horizontal = Canvas::new(SIZE as u32, SIZE as u32).unwrap();
            text(&mut horizontal, origin);
            let expected = rotate_pixels(&horizontal.into_rgba(), clockwise);
            assert!(expected.chunks_exact(4).any(|p| p[3] > 0));
            let mut vertical = Canvas::new(SIZE as u32, SIZE as u32).unwrap();
            vertical.rotate_at(if clockwise { 90.0 } else { -90.0 }, 64.0, 64.0);
            text(&mut vertical, origin);
            assert!(
                vertical.into_rgba() == expected,
                "clockwise={clockwise}, origin={origin:?}"
            );
        }
    }
}

#[test]
fn rich_titles_preserve_premultiplied_color_at_fractional_rotation_centers() {
    use renderer::{
        text::{RichText, rich_segments_from_text},
        text_render::draw_rich_text,
    };
    let mut segments = rich_segments_from_text("Area m2");
    segments.last_mut().unwrap().superscript = true;
    let rt = RichText {
        segments,
        color: Color::new(0.8, 0.2, 0.1, 0.6),
        font_size: 18.0,
        font: "Liberation Sans".into(),
    };
    for clockwise in [false, true] {
        let mut horizontal = Canvas::new(SIZE as u32, SIZE as u32).unwrap();
        // Moving the rotation center from (64,64) to (64.125,63.875)
        // translates local coordinates by these amounts for the two turns.
        let (tx, ty) = if clockwise { (-0.25, 0.0) } else { (0.0, 0.25) };
        horizontal.translate(tx, ty);
        draw_rich_text(&mut horizontal, &rt, (7.5, 44.25), FontPolicy::Standard);
        let expected = rotate_pixels(&horizontal.into_rgba(), clockwise);
        let mut vertical = Canvas::new(SIZE as u32, SIZE as u32).unwrap();
        vertical.save();
        vertical.rotate_at(if clockwise { 90.0 } else { -90.0 }, 64.125, 63.875);
        draw_rich_text(&mut vertical, &rt, (7.5, 44.25), FontPolicy::Standard);
        vertical.restore();
        assert_eq!(vertical.translation(), Some((0.0, 0.0)));
        let actual = vertical.into_rgba();
        assert!(actual.chunks_exact(4).any(|p| p[3] > 100 && p[3] < 255));
        assert!(
            actual
                .chunks_exact(4)
                .all(|p| p[..3].iter().all(|c| *c <= p[3]))
        );
        assert!(
            actual == expected,
            "fractional pivot, clockwise={clockwise}"
        );
    }
}

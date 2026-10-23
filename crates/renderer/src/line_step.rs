//! Line segment geometry shared by every CPU consumer of a line series:
//! the style flags the line shader reads, the mode word its piece helpers
//! take, the per-instance strip vertex count, and the data-space pieces the
//! CPU picker walks.
//!
//! The GPU twins are `step_piece` / `strip_slot` in
//! `data_render/line_columnar.wgsl` (shared byte-for-byte with
//! `line_arc.wgsl`) and `pick_step_piece` in `gpu_pick/line.wgsl`. All of them
//! split a segment `a → b` into the same pieces: the axis-aligned step
//! pieces, with the step-mid riser at the **data-space** midpoint of the two
//! x values, or a polar chart's equal subdivisions in data space (an arc
//! once projected), so the drawn line, its dash phase, its arc length and
//! both pickers agree.

use crate::data_config::LineInterpolation;
use crate::polar::{PolarInterpolation, PolarOptions};

/// `Style.primitive_flags` bit position of the two-bit step mode.
pub(crate) const LINE_STEP_SHIFT: u32 = 2;
/// `Style.primitive_flags` bit the area entry reads: fill to a second series'
/// y column (slots 4/5) instead of the baseline in `Style.dash[1].xy`.
pub(crate) const AREA_FILL_TO_SERIES: u32 = 1 << 4;
/// Bit position of the polar arc pieces in the mode word the shaders'
/// `step_piece_count` / `step_piece` take: bits 0-1 the step mode, bits 4
/// and up the pieces (0 for chords and rectangular charts). The pieces ride
/// the transform (`polar_extent.z`), never the style.
pub(crate) const LINE_ARC_SHIFT: u32 = 4;
/// The most pieces one segment is drawn as: the polar arc limit. Read by
/// the CPU twin of the line shader (test support).
#[cfg(feature = "cpu-twin")]
pub(crate) const MAX_LINE_PIECES: usize = crate::polar::MAX_POLAR_ARC_PIECES as usize;

/// The shader's step mode code: 0 linear, 1 step-pre, 2 step-post, 3 step-mid.
pub(crate) fn step_mode(interpolation: LineInterpolation) -> u32 {
    match interpolation {
        LineInterpolation::Linear => 0,
        LineInterpolation::StepPre => 1,
        LineInterpolation::StepPost => 2,
        LineInterpolation::StepMid => 3,
    }
}

/// The step mode placed at its `primitive_flags` position.
pub(crate) fn step_flag_bits(interpolation: LineInterpolation) -> u32 {
    step_mode(interpolation) << LINE_STEP_SHIFT
}

/// The arc pieces per segment a polar chart subdivides lines into: 0 for a
/// rectangular chart and for polar chords.
pub(crate) fn arc_pieces(polar: Option<&PolarOptions>) -> u32 {
    match polar.map(|polar| polar.interpolation) {
        Some(PolarInterpolation::Arc { pieces }) => pieces.max(1),
        Some(PolarInterpolation::Chord) | None => 0,
    }
}

/// The mode word the piece helpers take: the step mode in bits 0-1 and the
/// polar arc pieces from bit 4, as the shaders compose it from the style and
/// the transform.
pub(crate) fn mode_word(interpolation: LineInterpolation, arc_pieces: u32) -> u32 {
    step_mode(interpolation) | (arc_pieces << LINE_ARC_SHIFT)
}

/// The mode word for a line drawn against `transform`.
pub(crate) fn mode_word_for(
    interpolation: LineInterpolation,
    transform: &crate::data_render::ScatterTransform,
) -> u32 {
    mode_word(interpolation, transform.polar_extent[2].max(0.0) as u32)
}

/// The pieces a mode word draws a segment as. Twin of the shaders'
/// `step_piece_count`: an arc overrides the step mode.
pub(crate) fn piece_count(mode: u32) -> u32 {
    let arc = mode >> LINE_ARC_SHIFT;
    if arc > 0 {
        return arc;
    }
    match mode & 3 {
        1 | 2 => 2,
        3 => 3,
        _ => 1,
    }
}

/// Vertices one instance draws when every piece is a strip of `per_piece`
/// vertices and consecutive pieces are joined by two duplicated vertices:
/// `pieces · per_piece + 2 · (pieces − 1)`. A one-piece instance is the plain
/// strip. Twin of `strip_slot` in `line_columnar.wgsl`.
pub(crate) fn strip_vertex_count(per_piece: u32, mode: u32) -> u32 {
    let pieces = piece_count(mode);
    pieces * per_piece + 2 * (pieces - 1)
}

/// One axis-aligned piece of a stepped segment, in data units (the CPU
/// twin of the line shader, test support).
#[cfg(feature = "cpu-twin")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StepPiece {
    pub a: [f32; 2],
    pub b: [f32; 2],
}

/// The pieces a segment `a → b` is drawn as under `mode`, written into
/// `out` in draw order; returns how many. Linear: the segment itself.
/// Step-pre: riser at `a.x`, run at `b.y`. Step-post: run at `a.y`, riser at
/// `b.x`. Step-mid: run, riser at the data-space midpoint of the x values,
/// run. A polar arc: equal subdivisions of `a → b` in data space. Twin of the
/// shaders' `step_piece` (test support).
#[cfg(feature = "cpu-twin")]
pub(crate) fn line_pieces(
    a: [f32; 2],
    b: [f32; 2],
    mode: u32,
    out: &mut [StepPiece; MAX_LINE_PIECES],
) -> usize {
    let [ax, ay] = a;
    let [bx, by] = b;
    let arc = (mode >> LINE_ARC_SHIFT) as usize;
    if arc > 0 {
        let arc = arc.min(MAX_LINE_PIECES);
        for (k, piece) in out[..arc].iter_mut().enumerate() {
            let t0 = k as f32 / arc as f32;
            let t1 = (k + 1) as f32 / arc as f32;
            // The same `mix` the shaders evaluate on the pair's hi lane.
            *piece = StepPiece {
                a: [ax + (bx - ax) * t0, ay + (by - ay) * t0],
                b: [ax + (bx - ax) * t1, ay + (by - ay) * t1],
            };
        }
        return arc;
    }
    let whole = StepPiece { a, b };
    match mode & 3 {
        1 => {
            out[0] = StepPiece { a, b: [ax, by] };
            out[1] = StepPiece { a: [ax, by], b };
            2
        }
        2 => {
            out[0] = StepPiece { a, b: [bx, ay] };
            out[1] = StepPiece { a: [bx, ay], b };
            2
        }
        3 => {
            // The same expression the shaders use on the pair's hi lane.
            let xm = (ax + bx) * 0.5;
            out[0] = StepPiece { a, b: [xm, ay] };
            out[1] = StepPiece {
                a: [xm, ay],
                b: [xm, by],
            };
            out[2] = StepPiece { a: [xm, by], b };
            3
        }
        _ => {
            out[0] = whole;
            1
        }
    }
}

/// Test helper: the pieces of a rectangular chart's step mode.
#[cfg(all(test, feature = "cpu-twin"))]
pub(crate) fn step_pieces(
    a: [f32; 2],
    b: [f32; 2],
    interpolation: LineInterpolation,
) -> ([StepPiece; MAX_LINE_PIECES], usize) {
    let mut out = [StepPiece { a, b }; MAX_LINE_PIECES];
    let count = line_pieces(a, b, mode_word(interpolation, 0), &mut out);
    (out, count)
}

#[cfg(all(test, feature = "cpu-twin"))]
mod tests {
    use super::*;

    #[test]
    fn strip_counts_match_the_shader_joint_rule() {
        let mode = |i| mode_word(i, 0);
        assert_eq!(strip_vertex_count(4, mode(LineInterpolation::Linear)), 4);
        assert_eq!(strip_vertex_count(4, mode(LineInterpolation::StepPre)), 10);
        assert_eq!(strip_vertex_count(4, mode(LineInterpolation::StepPost)), 10);
        assert_eq!(strip_vertex_count(4, mode(LineInterpolation::StepMid)), 16);
        assert_eq!(strip_vertex_count(18, mode(LineInterpolation::StepMid)), 58);
        // A polar arc of 8 pieces: eight strips joined by seven joints.
        assert_eq!(
            strip_vertex_count(4, mode_word(LineInterpolation::Linear, 8)),
            46
        );
    }

    #[test]
    fn arc_pieces_subdivide_the_segment_in_data_space_and_chain() {
        let a = [10.0, 2.0];
        let b = [90.0, 6.0];
        let mut out = [StepPiece { a, b }; MAX_LINE_PIECES];
        let mode = mode_word(LineInterpolation::StepMid, 4);
        assert_eq!(piece_count(mode), 4, "an arc overrides the step mode");
        assert_eq!(line_pieces(a, b, mode, &mut out), 4);
        assert_eq!(out[0].a, a);
        assert_eq!(out[3].b, b);
        assert_eq!(out[1].a, [30.0, 3.0]);
        for pair in out[..4].windows(2) {
            assert_eq!(pair[0].b, pair[1].a);
        }
        let polar = PolarOptions {
            interpolation: PolarInterpolation::Arc { pieces: 12 },
            ..Default::default()
        };
        assert_eq!(arc_pieces(Some(&polar)), 12);
        assert_eq!(arc_pieces(Some(&PolarOptions::default())), 0);
        assert_eq!(arc_pieces(None), 0);
        assert_eq!(
            mode_word(LineInterpolation::StepPost, 12) >> LINE_ARC_SHIFT,
            12
        );
    }

    #[test]
    fn pieces_are_axis_aligned_and_chain_from_a_to_b() {
        let a = [1.0, 2.0];
        let b = [5.0, -4.0];
        for mode in [
            LineInterpolation::Linear,
            LineInterpolation::StepPre,
            LineInterpolation::StepPost,
            LineInterpolation::StepMid,
        ] {
            let (pieces, count) = step_pieces(a, b, mode);
            // Axis-aligned pieces one segment is drawn as.
            let expected = match mode {
                LineInterpolation::Linear => 1,
                LineInterpolation::StepPre | LineInterpolation::StepPost => 2,
                LineInterpolation::StepMid => 3,
            };
            assert_eq!(count, expected, "{mode:?}");
            assert_eq!(pieces[0].a, a);
            assert_eq!(pieces[count - 1].b, b);
            for pair in pieces[..count].windows(2) {
                assert_eq!(pair[0].b, pair[1].a, "{mode:?} pieces must chain");
            }
            if mode != LineInterpolation::Linear {
                for piece in &pieces[..count] {
                    assert!(
                        piece.a[0] == piece.b[0] || piece.a[1] == piece.b[1],
                        "{mode:?} piece {piece:?} is not axis-aligned"
                    );
                }
            }
        }
        let (mid, _) = step_pieces(a, b, LineInterpolation::StepMid);
        assert_eq!(
            mid[1].a[0], 3.0,
            "the step-mid riser sits at the data midpoint"
        );
        let (post, _) = step_pieces(a, b, LineInterpolation::StepPost);
        assert_eq!(post[0].b, [5.0, 2.0]);
        let (pre, _) = step_pieces(a, b, LineInterpolation::StepPre);
        assert_eq!(pre[0].b, [1.0, -4.0]);
    }

    #[test]
    fn flag_bits_sit_above_the_errorbar_bits() {
        assert_eq!(step_flag_bits(LineInterpolation::Linear), 0);
        assert_eq!(step_flag_bits(LineInterpolation::StepMid), 0b1100);
        assert_eq!(AREA_FILL_TO_SERIES, 0b1_0000);
    }
}

//! Edge-band probing: the pixels of a window a draw-against-hit scan
//! probes. Every pixel near a change of coverage is probed, elsewhere a
//! sparse grid, so a scan costs a few rows of the window instead of all of
//! it while still seeing every place where drawing and hitting can disagree.
//! Shared by `pick_draw_agreement.rs` and `decoration_oracle.rs`.
#![allow(dead_code)]

/// Half width, in export pixels, of the band around a painted edge in which
/// [`edge_band`] probes every pixel.
pub const EDGE_BAND_PX: u32 = 2;
/// Away from the edges, [`edge_band`] probes every `SPARSE_STEP`-th pixel of
/// every `SPARSE_STEP`-th row (image coordinates).
pub const SPARSE_STEP: u32 = 4;

/// The pixels of `window` (export pixels, inclusive start, exclusive end)
/// that a pick-against-paint scan probes, in row-major order.
///
/// An edge pixel is one whose `painted` state differs from one of its four
/// neighbours inside the window. Every pixel within [`EDGE_BAND_PX`] of an
/// edge pixel along both axes is probed: picking and painting can only
/// disagree where coverage changes, and an anti-aliasing fringe, a cap, a
/// corner or a gap is at most a pixel or two from that change. Elsewhere,
/// inside the paint and outside it, a sparse grid of every
/// [`SPARSE_STEP`]-th pixel is probed, which still catches an interior that
/// does not pick or a surrounding that does.
pub fn edge_band(
    (x0, y0, x1, y1): (u32, u32, u32, u32),
    painted: impl Fn(u32, u32) -> bool,
) -> Vec<(u32, u32)> {
    let w = x1.saturating_sub(x0) as usize;
    let h = y1.saturating_sub(y0) as usize;
    let at = |i: usize| (x0 + (i % w) as u32, y0 + (i / w) as u32);
    let state: Vec<bool> = (0..w * h)
        .map(|i| {
            let (x, y) = at(i);
            painted(x, y)
        })
        .collect();
    let band = EDGE_BAND_PX as usize;
    let mut near_edge = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let here = state[y * w + x];
            let edge = (x > 0 && state[y * w + x - 1] != here)
                || (x + 1 < w && state[y * w + x + 1] != here)
                || (y > 0 && state[(y - 1) * w + x] != here)
                || (y + 1 < h && state[(y + 1) * w + x] != here);
            if edge {
                let (bx0, bx1) = (x.saturating_sub(band), (x + band + 1).min(w));
                for by in y.saturating_sub(band)..(y + band + 1).min(h) {
                    near_edge[by * w + bx0..by * w + bx1].fill(true);
                }
            }
        }
    }
    (0..w * h)
        .filter(|&i| {
            let (x, y) = at(i);
            near_edge[i] || (x % SPARSE_STEP == 0 && y % SPARSE_STEP == 0)
        })
        .map(at)
        .collect()
}

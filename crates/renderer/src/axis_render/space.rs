//! The constellation style's deep-space backdrop and the glow bloom of its
//! decoration layer.

use super::*;

// ── Constellation backdrop + glow (style-specific CPU post-processing) ──
//
// LEGIBILITY CONTRACT: this is a science-presentation tool — the backdrop
// must never compete with data ink. Hard rules encoded below:
//   - nebula peak adds ≤ ~12/255 luminance over the base, low-frequency only
//   - a soft vignette keeps the panel CENTER (where data lives) the cleanest
//   - background dust stars are 1 px and far dimmer than any data star
//     (data stars have PSF cores + halos and sit on series ribbons)

/// Deep-space base color (premultiplied; alpha 255 — the panel owns its
/// background in this style, no host compositing needed).
pub(super) const SPACE_BASE: [f32; 3] = [11.0, 15.0, 23.0];

pub(super) fn draw_space_background(
    canvas: &mut Canvas,
    _config: &Config,
    c: &crate::config::MilkywayOptions,
) {
    use crate::data_render::fbm2;

    let (w, h) = canvas.size();
    if w == 0 || h == 0 {
        return;
    }
    let seed = c.seed;

    // Nebula sampled on a quarter-res lattice (it is low-frequency by
    // design) and bilinearly upsampled — 16× cheaper than per-pixel fBm.
    let gw = (w / 4 + 2) as usize;
    let gh = (h / 4 + 2) as usize;
    let mut cool = vec![0.0f32; gw * gh];
    let mut warm = vec![0.0f32; gw * gh];
    for gy in 0..gh {
        for gx in 0..gw {
            let x = gx as f64 * 4.0 / w.max(1) as f64 * 3.0;
            let y = gy as f64 * 4.0 / h.max(1) as f64 * 2.0;
            cool[gy * gw + gx] =
                ((fbm2(x + 7.1, y + 3.7, 5, seed ^ 0x0EB1) - 0.42).max(0.0) * 2.0).min(1.0) as f32;
            warm[gy * gw + gx] =
                ((fbm2(x * 0.7 + 21.0, y * 0.7 + 9.0, 4, seed ^ 0x0EB2) - 0.50).max(0.0) * 2.2)
                    .min(1.0) as f32;
        }
    }
    let sample = |grid: &[f32], px: u32, py: u32| -> f32 {
        let fx = px as f32 / 4.0;
        let fy = py as f32 / 4.0;
        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let (tx, ty) = (fx.fract(), fy.fract());
        let i = |x: usize, y: usize| grid[(y.min(gh - 1)) * gw + x.min(gw - 1)];
        let a = i(x0, y0) * (1.0 - tx) + i(x0 + 1, y0) * tx;
        let b = i(x0, y0 + 1) * (1.0 - tx) + i(x0 + 1, y0 + 1) * tx;
        a * (1.0 - ty) + b * ty
    };

    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let max_r = (cx * cx + cy * cy).sqrt().max(1.0);
    let data = canvas.pixels_mut();
    for py in 0..h {
        for px in 0..w {
            // Vignette: nebula fades toward the panel center so the data
            // region stays the cleanest part of the frame.
            let dx = px as f32 - cx;
            let dy = py as f32 - cy;
            let edge = ((dx * dx + dy * dy).sqrt() / max_r).clamp(0.0, 1.0);
            let vig = 0.35 + 0.65 * edge * edge;

            // Peak nebula contribution stays ≤ ~12/255 per channel at the
            // default `nebula = 1.0`; the slider scales within the same
            // legibility-bounded design.
            let neb = c.nebula.clamp(0.0, 2.0);
            let nc = sample(&cool, px, py) * vig * neb;
            let nw = sample(&warm, px, py) * vig * neb;
            let r = SPACE_BASE[0] + nc * 4.0 + nw * 9.0;
            let g = SPACE_BASE[1] + nc * 6.0 + nw * 5.0;
            let b = SPACE_BASE[2] + nc * 12.0 + nw * 4.0;

            let i = ((py * w + px) * 4) as usize;
            data[i] = r.min(255.0) as u8;
            data[i + 1] = g.min(255.0) as u8;
            data[i + 2] = b.min(255.0) as u8;
            data[i + 3] = 255;
        }
    }

    // Background dust: sparse, dim, 1 px — unmistakably "behind" the data.
    let n_dust = (((w as u64 * h as u64) / 1400) as f32 * c.dust.clamp(0.0, 4.0)).max(0.0) as u32;
    for k in 0..n_dust {
        let hx = crate::sketch::hash01(k, seed ^ 0xD057_0001);
        let hy = crate::sketch::hash01(k, seed ^ 0xD057_0002);
        let hb = crate::sketch::hash01(k, seed ^ 0xD057_0003);
        let px = (hx * w as f32) as u32 % w;
        let py = (hy * h as f32) as u32 % h;
        let add = 14.0 + 52.0 * hb * hb;
        let i = ((py * w + px) * 4) as usize;
        data[i] = (data[i] as f32 + add).min(255.0) as u8;
        data[i + 1] = (data[i + 1] as f32 + add).min(255.0) as u8;
        data[i + 2] = (data[i + 2] as f32 + add * 1.06).min(255.0) as u8;
    }
}

// Glow bloom for the decoration layer (axes / ticks / labels / titles read
// as line-light sources): one blurred copy added back under the crisp
// original. This runs on every decoration re-raster (= every pan / zoom /
// config commit), so it must stay light:
//   - the halo lives at QUARTER resolution (a soft halo can't show the
//     difference; box radius 1 × 2 passes there ≈ the old half-res blur),
//   - the downsample tests each 4×4 block's alpha first — the deco layer is
//     transparent everywhere but the chrome, so most blocks skip the sum,
//   - the upsample-add walks 4-px spans and skips spans whose four
//     contributing quarter-cells are all zero, with 8-bit fixed-point math
//     on the spans that remain.
// Premultiplied-additive with clamp; selection boxes are drawn AFTER this in
// the raster entry, so the interaction overlay stays crisp.
pub(super) const GLOW_PASSES: u32 = 2;

pub(super) const GLOW_RADIUS: usize = 1;

pub(crate) fn apply_decoration_glow(canvas: &mut Canvas, gain: f32) {
    let gain = gain.clamp(0.0, 2.0);
    if gain <= 0.0 {
        return;
    }
    let (w, h) = canvas.size();
    if w < 4 || h < 4 {
        return;
    }
    let (w, h) = (w as usize, h as usize);
    let (qw, qh) = (w.div_ceil(4), h.div_ceil(4));
    let src = canvas.pixels_mut();

    // Downsample 4×4 average into the quarter-res halo buffer. Blocks whose
    // alpha is all zero keep the buffer's zero fill without summing.
    let mut halo = vec![0u8; qw * qh * 4];
    let mut tmp = vec![0u8; qw * qh * 4];
    for qy in 0..qh {
        for qx in 0..qw {
            let (x_lo, y_lo) = (qx * 4, qy * 4);
            let (x_hi, y_hi) = ((x_lo + 4).min(w), (y_lo + 4).min(h));
            let mut any = false;
            'probe: for y in y_lo..y_hi {
                let row = y * w * 4;
                for x in x_lo..x_hi {
                    if src[row + x * 4 + 3] != 0 {
                        any = true;
                        break 'probe;
                    }
                }
            }
            if !any {
                continue;
            }
            let mut sum = [0u32; 4];
            let mut n = 0u32;
            for y in y_lo..y_hi {
                let row = y * w * 4;
                for x in x_lo..x_hi {
                    for c in 0..4 {
                        sum[c] += src[row + x * 4 + c] as u32;
                    }
                    n += 1;
                }
            }
            for c in 0..4 {
                halo[(qy * qw + qx) * 4 + c] = (sum[c] / n.max(1)) as u8;
            }
        }
    }

    // Separable box blur ×2 ≈ gaussian, at quarter res.
    for _ in 0..GLOW_PASSES {
        for y in 0..qh {
            let row = y * qw * 4;
            let mut acc = [0u32; 4];
            for x in 0..qw.min(GLOW_RADIUS + 1) {
                for c in 0..4 {
                    acc[c] += halo[row + x * 4 + c] as u32;
                }
            }
            let mut count = qw.min(GLOW_RADIUS + 1) as u32;
            for x in 0..qw {
                for c in 0..4 {
                    tmp[row + x * 4 + c] = (acc[c] / count.max(1)) as u8;
                }
                if x + GLOW_RADIUS + 1 < qw {
                    for c in 0..4 {
                        acc[c] += halo[row + (x + GLOW_RADIUS + 1) * 4 + c] as u32;
                    }
                    count += 1;
                }
                if x >= GLOW_RADIUS {
                    for c in 0..4 {
                        acc[c] -= halo[row + (x - GLOW_RADIUS) * 4 + c] as u32;
                    }
                    count -= 1;
                }
            }
        }
        for x in 0..qw {
            let mut acc = [0u32; 4];
            for y in 0..qh.min(GLOW_RADIUS + 1) {
                for c in 0..4 {
                    acc[c] += tmp[(y * qw + x) * 4 + c] as u32;
                }
            }
            let mut count = qh.min(GLOW_RADIUS + 1) as u32;
            for y in 0..qh {
                for c in 0..4 {
                    halo[(y * qw + x) * 4 + c] = (acc[c] / count.max(1)) as u8;
                }
                if y + GLOW_RADIUS + 1 < qh {
                    for c in 0..4 {
                        acc[c] += tmp[((y + GLOW_RADIUS + 1) * qw + x) * 4 + c] as u32;
                    }
                    count += 1;
                }
                if y >= GLOW_RADIUS {
                    for c in 0..4 {
                        acc[c] -= tmp[((y - GLOW_RADIUS) * qw + x) * 4 + c] as u32;
                    }
                    count -= 1;
                }
            }
        }
    }

    // Per-cell occupancy of the blurred halo — the span skip below reads
    // this instead of re-testing four bytes per cell per span.
    let mut occupied = vec![false; qw * qh];
    for (cell, occ) in halo.chunks_exact(4).zip(occupied.iter_mut()) {
        *occ = cell.iter().any(|&b| b != 0);
    }

    // Bilinear upsample + additive merge, in 8-bit fixed point. Output is
    // walked in 4-px spans sharing one quarter-cell column pair; a span whose
    // four contributing cells are all zero adds nothing and is skipped — on a
    // chart-sized canvas that's most of the data area.
    let gain_fp = (gain * 256.0) as u32;
    for y in 0..h {
        let y0 = (y / 4).min(qh - 1);
        let y1 = (y0 + 1).min(qh - 1);
        let ty = ((y % 4) * 64) as u32; // fraction in /256
        let (row0, row1) = (y0 * qw, y1 * qw);
        let out_row = y * w * 4;
        let mut x = 0usize;
        while x < w {
            let x0 = (x / 4).min(qw - 1);
            let x1 = (x0 + 1).min(qw - 1);
            let span_end = (x0 * 4 + 4).min(w);
            if !occupied[row0 + x0]
                && !occupied[row0 + x1]
                && !occupied[row1 + x0]
                && !occupied[row1 + x1]
            {
                x = span_end;
                continue;
            }
            let (c00, c01) = ((row0 + x0) * 4, (row0 + x1) * 4);
            let (c10, c11) = ((row1 + x0) * 4, (row1 + x1) * 4);
            while x < span_end {
                let tx = ((x % 4) * 64) as u32;
                for c in 0..4 {
                    let a = halo[c00 + c] as u32 * (256 - tx) + halo[c01 + c] as u32 * tx;
                    let b = halo[c10 + c] as u32 * (256 - tx) + halo[c11 + c] as u32 * tx;
                    let v = (a * (256 - ty) + b * ty) >> 16; // back to 0..=255
                    let i = out_row + x * 4 + c;
                    src[i] = (src[i] as u32 + ((v * gain_fp) >> 8)).min(255) as u8;
                }
                x += 1;
            }
        }
    }
}

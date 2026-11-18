// Probability axes (#209): a probability `p` in (0, 1) drawn at `F⁻¹(p)`
// for the scale code `kind` (`Transform.scale_prob`): 1 normal, 2 Weibull,
// 3 logistic. Placing given data, so projection (CLAUDE.md rule 1).
// SHADER_COMMON.md §5.24 has the accuracy contract. CPU twin, in f64:
// `figgy_model::probability` (product), `cpu_twin` (f32, test support).
//
// Every function reads the column's `(hi, lo)` pair, not its sum: `q`, the
// distance to 1, is `(1 − hi) − lo`, whose first subtraction is exact for
// `hi ≥ 0.5` (Sterbenz), so the upper tail that `hi` alone rounds away
// survives (`hi = 1`, `lo = −1e-10` gives `q = 1e-10`).

// The floor of `p` and `q`, the one `maybe_log` uses: an extent end outside
// (0, 1) lands far past the axis end and is cut by the scissor.
const PROB_FLOOR: f32 = 1e-30;

// `p` and `q = 1 − p` of a pair.
fn prob_pq(pair: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(pair.x + pair.y, (1.0 - pair.x) - pair.y);
}

// A sample with a position: `p > 0` and `q > 0`, which also rules out a
// non-finite value (NaN fails both, ±∞ fails one).
fn prob_in_domain(pair: vec2<f32>) -> bool {
    let pq = prob_pq(pair);
    return pq.x > 0.0 && pq.y > 0.0;
}

// Wichura's AS241 PPND7 (single precision, about 1e-7 relative): `Φ⁻¹` at
// `p`, its tail measured by `r = min(p, q)` so the upper side keeps `q`.
fn prob_normal(pq: vec2<f32>) -> f32 {
    let half = pq.x - 0.5;
    if (abs(half) <= 0.425) {
        let r = 0.180625 - half * half;
        return half * (((59.10937472 * r + 159.29113202) * r + 50.434271938) * r + 3.3871327179)
            / (((67.1875636 * r + 78.757757664) * r + 17.895169469) * r + 1.0);
    }
    var r = sqrt(-log(max(min(pq.x, pq.y), PROB_FLOOR)));
    var z: f32;
    if (r <= 5.0) {
        r = r - 1.6;
        z = (((0.17023821103 * r + 1.3067284816) * r + 2.75681539) * r + 1.4234372777)
            / ((0.12021132975 * r + 0.7370016425) * r + 1.0);
    } else {
        r = r - 5.0;
        z = (((0.017337203997 * r + 0.42868294337) * r + 3.081226386) * r + 6.657905115)
            / ((0.012258202635 * r + 0.24197894225) * r + 1.0);
    }
    return select(z, -z, half < 0.0);
}

// `w = −ln(1 − p)`, the Weibull paper's inner value. Below 0.01 it is the
// series (relative error under 2e-9), because `1 − p` in f32 loses the
// small tail; above, `−ln(q)` from the exact `q`.
fn prob_weibull_w(pq: vec2<f32>) -> f32 {
    let p = max(pq.x, PROB_FLOOR);
    if (p < 0.01) {
        return p * (1.0 + p * (0.5 + p * (1.0 / 3.0 + p * (0.25 + p * 0.2))));
    }
    return -log(max(pq.y, PROB_FLOOR));
}

// `F⁻¹(p)` for scale code `kind` (> 0.5). `p` and `q` are floored, so an
// out-of-domain value gives a far, finite position (an extent's end).
fn prob_forward(pair: vec2<f32>, kind: f32) -> f32 {
    let pq = prob_pq(pair);
    if (kind < 1.5) {
        return prob_normal(pq);
    }
    if (kind < 2.5) {
        return log(prob_weibull_w(pq));
    }
    return log(max(pq.x, PROB_FLOOR)) - log(max(pq.y, PROB_FLOOR));
}

// `dF⁻¹/dp` at the pair, for a derivative in pixels per data unit
// (`axis_px_per_unit_at`): `√(2π)·exp(z²/2)`, `1/(q·w)`, `1/(p·q)`.
fn prob_slope(pair: vec2<f32>, kind: f32) -> f32 {
    let pq = max(prob_pq(pair), vec2<f32>(PROB_FLOOR));
    if (kind < 1.5) {
        let z = prob_normal(pq);
        return 2.5066282746310002 * exp(0.5 * z * z);
    }
    if (kind < 2.5) {
        return 1.0 / (pq.y * prob_weibull_w(pq));
    }
    return 1.0 / (pq.x * pq.y);
}

// Numerical Recipes' `erfcc` (fractional error under 1.2e-7 everywhere).
fn prob_erfcc(x: f32) -> f32 {
    let z = abs(x);
    let t = 1.0 / (1.0 + 0.5 * z);
    let e = t * exp(-z * z - 1.26551223 + t * (1.00002368 + t * (0.37409196 + t * (0.09678418
        + t * (-0.18628806 + t * (0.27886807 + t * (-1.13520398 + t * (1.48851587
        + t * (-0.82215223 + t * 0.17087277)))))))));
    return select(2.0 - e, e, x >= 0.0);
}

// The pair `p` at `y = F⁻¹(p)`: `(p, 0)` on the lower side and `(1, −q)` on
// the upper side, so the tail survives the pair form.
fn prob_inverse(y: f32, kind: f32) -> vec2<f32> {
    // The upper tail's `q` is the lower tail's `p` at `−y` for the
    // symmetric normal and logistic maps.
    if (kind < 1.5) {
        let tail = 0.5 * prob_erfcc(abs(y) * 0.7071067811865476);
        return select(vec2<f32>(tail, 0.0), vec2<f32>(1.0, -tail), y > 0.0);
    }
    if (kind < 2.5) {
        // p = 1 − exp(−eʸ): small p is −expm1(−eʸ) ≈ eʸ − e²ʸ/2.
        let w = exp(y);
        if (w < 0.01) {
            return vec2<f32>(w * (1.0 - w * (0.5 - w * (1.0 / 6.0))), 0.0);
        }
        let q = exp(-w);
        return select(vec2<f32>(1.0 - q, 0.0), vec2<f32>(1.0, -q), q < 0.5);
    }
    let tail = 1.0 / (1.0 + exp(abs(y)));
    return select(vec2<f32>(tail, 0.0), vec2<f32>(1.0, -tail), y > 0.0);
}

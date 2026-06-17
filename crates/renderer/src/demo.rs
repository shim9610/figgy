//! Sample data generators for the bundled examples.
//!
//! Returns plain `(Vec<f64>, Vec<f64>)` only — wrapping in `Column`, building
//! a `Chart`, etc. is shown in the example code, not here.

fn linspace(n: usize, lo: f64, hi: f64) -> Vec<f64> {
    if n < 2 {
        return vec![(lo + hi) * 0.5; n.max(1)];
    }
    (0..n)
        .map(|i| lo + (hi - lo) * (i as f64) / ((n - 1) as f64))
        .collect()
}

/// `y = 50 + 30·sin(x)` over `x ∈ [0, 4π]` (two periods).
pub fn sine_data(n: usize) -> (Vec<f64>, Vec<f64>) {
    let xs = linspace(n, 0.0, 4.0 * std::f64::consts::PI);
    let ys = xs.iter().map(|x| 50.0 + 30.0 * x.sin()).collect();
    (xs, ys)
}

/// RC charging curve `V(t) = 5·(1 − exp(−t))`, `t ∈ [0, 5]`.
pub fn rc_data(n: usize) -> (Vec<f64>, Vec<f64>) {
    let v0 = 5.0_f64;
    let tau = 1.0_f64;
    let ts = linspace(n, 0.0, 5.0 * tau);
    let vs = ts.iter().map(|t| v0 * (1.0 - (-t / tau).exp())).collect();
    (ts, vs)
}

/// RC discharge curve `V(t) = 5·exp(−t)`, same time domain as [`rc_data`].
pub fn rc_discharge_data(n: usize) -> (Vec<f64>, Vec<f64>) {
    let v0 = 5.0_f64;
    let tau = 1.0_f64;
    let ts = linspace(n, 0.0, 5.0 * tau);
    let vs = ts.iter().map(|t| v0 * (-t / tau).exp()).collect();
    (ts, vs)
}

/// Gaussian peak with background: `σ(E) = σ_peak·exp(-(E-E0)²/(2Δ²)) + σ_bg`,
/// `E0=100, Δ=12, σ_peak=1, σ_bg=1e-4, E ∈ [50, 150]`. ~4 decade dynamic range
/// — useful for log-scale Y demos.
pub fn cross_section_data(n: usize) -> (Vec<f64>, Vec<f64>) {
    let e0 = 100.0_f64;
    let delta = 12.0_f64;
    let sigma_peak = 1.0_f64;
    let sigma_bg = 1.0e-4_f64;
    let es = linspace(n, 50.0, 150.0);
    let sigmas = es
        .iter()
        .map(|e| {
            let g = (-((e - e0).powi(2)) / (2.0 * delta * delta)).exp();
            sigma_peak * g + sigma_bg
        })
        .collect();
    (es, sigmas)
}

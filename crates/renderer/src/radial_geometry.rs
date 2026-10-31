//! CPU counterpart of the radial shader's rounded footprint and extruded SDF.
//! Picking uses rendered geometry, including tilt, gap, lift and occlusion.
use super::Sector;
use std::f32::consts::{PI, TAU};
fn round_intersection(a: f32, b: f32, radius: f32) -> f32 {
    if radius < 0.000001 {
        return a.max(b);
    }
    (-radius).min(a.max(b)) + (radius + a).max(0.0).hypot((radius + b).max(0.0))
}
fn footprint(p: [f32; 2], s: &Sector) -> f32 {
    let r = p[0].hypot(p[1]);
    let outer = r - 1.0;
    let inner = if s.geometry[3] > 0.0 {
        s.geometry[3] - r
    } else {
        -10.0
    };
    if s.arc[1] >= TAU - 0.00001 {
        return outer.max(inner);
    }
    let a = s.arc[0] + s.detail[1] * 0.5;
    let b = s.arc[0] + s.arc[1] - s.detail[1] * 0.5;
    let da = p[0] * a.sin() - p[1] * a.cos();
    let db = -p[0] * b.sin() + p[1] * b.cos();
    let wedge = if b - a <= PI { da.max(db) } else { da.min(db) };
    let cap = ((1.0 - s.geometry[3]) * 0.45).min((b - a) * s.geometry[3].max(0.2) * 0.35);
    let wedge = if s.geometry[3] == 0.0 && b - a <= PI {
        round_intersection(da, db, s.rounding[0].min(cap))
    } else {
        wedge
    };
    round_intersection(outer, wedge, s.rounding[1].min(cap)).max(round_intersection(
        inner,
        wedge,
        s.rounding[0].min(cap),
    ))
}
fn solid(p: [f32; 3], s: &Sector) -> f32 {
    let bevel = s.detail[2]
        .min(s.arc[3] * 0.48)
        .min((1.0 - s.geometry[3]) * 0.2);
    let a = footprint([p[0], p[1]], s) + bevel;
    let b = (p[2] - s.arc[3] * 0.5).abs() - (s.arc[3] * 0.5 - bevel);
    a.max(0.0).hypot(b.max(0.0)) + a.max(b).min(0.0) - bevel
}
pub(super) fn hit(s: &Sector, pixel: [f32; 2]) -> Option<f32> {
    let cs = s.arc[2].cos();
    let sn = s.arc[2].sin();
    let mid = s.arc[0] + s.arc[1] * 0.5;
    let x = (pixel[0] - s.geometry[0]) / s.geometry[2] - s.detail[0] * mid.cos();
    let y = (pixel[1] - s.geometry[1]) / s.geometry[2] - s.detail[0] * mid.sin() * cs;
    if x.abs() > 1.001 || y.abs() > 1.0 + s.arc[3] + s.rounding[2] {
        return None;
    }
    let ro = [x, y * cs + sn * 3.0, -y * sn + cs * 3.0 - s.rounding[2]];
    let rd = [0.0, -sn, -cs];
    let mut t = ((s.arc[3] - ro[2]) / rd[2]).max(0.0);
    let end = -ro[2] / rd[2] + 0.0002;
    if s.arc[3] <= 0.0 {
        return (footprint([ro[0], ro[1] + rd[1] * t], s) <= 0.0).then_some(t);
    }
    for _ in 0..128 {
        if t > end {
            return None;
        }
        let d = solid([ro[0], ro[1] + rd[1] * t, ro[2] + rd[2] * t], s);
        if d < 0.00012 {
            return Some(t);
        }
        t += d.max(0.00005);
    }
    None
}

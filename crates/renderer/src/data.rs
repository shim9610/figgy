//! Data containers (re-exported from the model crate) plus the renderer's
//! `ColumnSource` upload adapter.
//!
//! `ColumnSource` lives here, not in the model crate, because it is render
//! plumbing: its contract is "fill a wgpu mapped staging buffer with
//! little-endian numeric lanes" — an optimization detail of the GPU upload
//! path, not part of the chart declaration.

pub use ::model::data::*;

pub const COLUMN_VALUE_F32S: usize = 2;
pub const COLUMN_VALUE_BYTES: usize = std::mem::size_of::<f32>() * COLUMN_VALUE_F32S;

pub fn split_f64_to_f32_pair(v: f64) -> (f32, f32) {
    let hi = v as f32;
    if !v.is_finite() || !hi.is_finite() {
        return (hi, 0.0);
    }
    let lo = (v - hi as f64) as f32;
    (hi, lo)
}

/// Adapter from any column-shaped data into the scalar figgy GPU upload path.
///
/// The contract is scalar stats plus a zero-copy staging write: `len` /
/// `min` / `max` describe the column, and [`Self::write_f32_le_into`] streams
/// the values into a wgpu mapped staging buffer. Nulls encode as `f32::NAN`;
/// null / non-numeric handling is the implementor's responsibility.
pub trait ColumnSource {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn max(&self) -> f64;
    fn min(&self) -> f64;

    /// **Zero-copy stream upload**: write the column's values as little-endian
    /// f32 directly into `dst`, which is a slice into a wgpu mapped staging
    /// buffer. The caller guarantees `dst.len() == self.len() * 4`. Nulls are
    /// encoded as `f32::NAN`. Conversion (f64 → f32, Option → f32) happens
    /// element-wise inline — no intermediate `Vec`.
    fn write_f32_le_into(&self, dst: &mut [u8]);
}

/// High-precision column upload path.
///
/// Each logical value is encoded as two f32 lanes `(hi, lo)` so shaders can
/// subtract split axis bounds before recombining. This preserves small
/// timestamp deltas around large Unix epoch values.
pub trait HiLoColumnSource {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn max(&self) -> f64;
    fn min(&self) -> f64;

    /// Write `len * 8` bytes as little-endian `(hi: f32, lo: f32)` pairs.
    fn write_f32_pair_le_into(&self, dst: &mut [u8]);
}

// Built-in implementations for numeric column types.

impl ColumnSource for Column<f64> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn min(&self) -> f64 {
        self.min
    }
    fn max(&self) -> f64 {
        self.max
    }
    fn write_f32_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * 4);
        for (i, &v) in self.data.iter().enumerate() {
            dst[i * 4..i * 4 + 4].copy_from_slice(&(v as f32).to_le_bytes());
        }
    }
}

impl HiLoColumnSource for Column<f64> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn min(&self) -> f64 {
        self.min
    }
    fn max(&self) -> f64 {
        self.max
    }
    fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * COLUMN_VALUE_BYTES);
        for (i, &v) in self.data.iter().enumerate() {
            let (hi, lo) = split_f64_to_f32_pair(v);
            let base = i * COLUMN_VALUE_BYTES;
            dst[base..base + 4].copy_from_slice(&hi.to_le_bytes());
            dst[base + 4..base + 8].copy_from_slice(&lo.to_le_bytes());
        }
    }
}

impl ColumnSource for Column<f32> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn min(&self) -> f64 {
        self.min as f64
    }
    fn max(&self) -> f64 {
        self.max as f64
    }
    fn write_f32_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * 4);
        // Already f32; on little-endian targets bit patterns match.
        let dst_f32: &mut [f32] = bytemuck::cast_slice_mut(dst);
        dst_f32.copy_from_slice(&self.data);
    }
}

impl HiLoColumnSource for Column<f32> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn min(&self) -> f64 {
        self.min as f64
    }
    fn max(&self) -> f64 {
        self.max as f64
    }
    fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * COLUMN_VALUE_BYTES);
        for (i, &hi) in self.data.iter().enumerate() {
            let base = i * COLUMN_VALUE_BYTES;
            dst[base..base + 4].copy_from_slice(&hi.to_le_bytes());
            dst[base + 4..base + 8].copy_from_slice(&0.0f32.to_le_bytes());
        }
    }
}

impl ColumnSource for Column<Option<f64>> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn min(&self) -> f64 {
        self.min.unwrap_or(f64::NAN)
    }
    fn max(&self) -> f64 {
        self.max.unwrap_or(f64::NAN)
    }
    fn write_f32_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * 4);
        for (i, opt) in self.data.iter().enumerate() {
            let v = opt.map(|x| x as f32).unwrap_or(f32::NAN);
            dst[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
}

impl HiLoColumnSource for Column<Option<f64>> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn min(&self) -> f64 {
        self.min.unwrap_or(f64::NAN)
    }
    fn max(&self) -> f64 {
        self.max.unwrap_or(f64::NAN)
    }
    fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * COLUMN_VALUE_BYTES);
        for (i, opt) in self.data.iter().enumerate() {
            let (hi, lo) = opt.map(split_f64_to_f32_pair).unwrap_or((f32::NAN, 0.0));
            let base = i * COLUMN_VALUE_BYTES;
            dst[base..base + 4].copy_from_slice(&hi.to_le_bytes());
            dst[base + 4..base + 8].copy_from_slice(&lo.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::split_f64_to_f32_pair;

    #[test]
    fn split_pair_preserves_small_delta_near_large_epoch() {
        let a = 1_700_000_000_000.125_f64;
        let b = a + 0.75;
        let direct = (b as f32) - (a as f32);
        let (a_hi, a_lo) = split_f64_to_f32_pair(a);
        let (b_hi, b_lo) = split_f64_to_f32_pair(b);
        let split_delta = (b_hi - a_hi) + (b_lo - a_lo);

        assert_eq!(direct, 0.0);
        assert!((split_delta as f64 - 0.75).abs() < 1.0e-3);
    }
}

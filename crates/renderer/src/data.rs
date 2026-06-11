//! Data containers (re-exported from the model crate) plus the renderer's
//! `ColumnSource` upload adapter.
//!
//! `ColumnSource` lives here, not in the model crate, because it is render
//! plumbing: its contract is "fill a wgpu mapped staging buffer with
//! little-endian NaN-coded f32" — an optimization detail of the GPU upload
//! path, not part of the chart declaration.

pub use ::model::data::*;

/// Adapter from any column-shaped data into the figgy GPU upload path.
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

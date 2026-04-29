//! Data containers and the `ColumnSource` adapter trait.
//!
//! Role assignment (X / Y / X-error / Y-error) lives in
//! [`crate::data_config::SeriesConfig`]; `Column` / `DataCell` themselves are
//! role-agnostic.

/// A bundle of columns belonging to one logical data source.
#[derive(Debug, Clone, PartialEq)]
pub struct DataCell<T> {
    pub data_id: String,
    pub columns: Vec<Column<T>>,
}

/// A single column: data + cached min/max.
///
/// The role (axis or errorbar) is not encoded here — `SeriesConfig.x_column /
/// y_column` and `DataRenderType`'s `ErrorRef` reference columns by id.
#[derive(Debug, Clone, PartialEq)]
pub struct Column<T> {
    pub index: usize,
    pub data: Vec<T>,
    pub min: T,
    pub max: T,
}

/// Type-erased column metadata: index, range, length only — no payload.
///
/// Used for axis auto-scaling and GPU upload sizing. The actual upload is
/// handled by [`crate::data_render::column_pool::ColumnPool`].
#[derive(Debug, Clone, PartialEq)]
pub struct ErasedColumn {
    pub index: usize,
    pub min: f64,
    pub max: f64,
    pub len_values: usize,
}

/// Adapter from any column-shaped data into the figgy render paths
/// (CPU nullable f64 / GPU NaN-coded f32).
///
/// Null / non-numeric handling is the implementor's responsibility; the trait
/// only constrains the final output forms.
pub trait ColumnSource {
    fn index(&self) -> usize;
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn max(&self) -> f64;
    fn min(&self) -> f64;

    /// Observed min/max excluding nulls. `None` for empty columns.
    fn numeric_range(&self) -> Option<(f64, f64)>;

    /// CPU path: f64 iterator that preserves nulls as `None`.
    fn iter_f64_nullable(&self) -> Box<dyn Iterator<Item = Option<f64>> + '_>;

    /// Legacy / non-zero-copy GPU path: contiguous buffer with nulls coded as `f32::NAN`.
    /// Prefer [`Self::write_f32_le_into`] for new code.
    fn to_f32_nan_null(&self) -> Vec<f32>;

    /// **Zero-copy stream upload**: write the column's values as little-endian
    /// f32 directly into `dst`, which is a slice into a wgpu mapped staging
    /// buffer. The caller guarantees `dst.len() == self.len() * 4`. Nulls are
    /// encoded as `f32::NAN`. Conversion (f64 → f32, Option → f32) happens
    /// element-wise inline — no intermediate `Vec`.
    fn write_f32_le_into(&self, dst: &mut [u8]);

    /// Log-scale variant: values `<= 0` are emitted as `None`. Default impl
    /// filters [`Self::iter_f64_nullable`]; override for performance.
    fn iter_f64_log_safe(&self) -> Box<dyn Iterator<Item = Option<f64>> + '_> {
        Box::new(self.iter_f64_nullable().map(|v| match v {
            Some(x) if x > 0.0 => Some(x),
            _ => None,
        }))
    }
}

// Built-in implementations for numeric column types.

impl ColumnSource for Column<f64> {
    fn index(&self) -> usize {
        self.index
    }
    fn len(&self) -> usize {
        self.data.len()
    }
    fn min(&self) -> f64 {
        self.min
    }
    fn max(&self) -> f64 {
        self.max
    }
    fn numeric_range(&self) -> Option<(f64, f64)> {
        if self.data.is_empty() {
            None
        } else {
            Some((self.min, self.max))
        }
    }
    fn iter_f64_nullable(&self) -> Box<dyn Iterator<Item = Option<f64>> + '_> {
        Box::new(self.data.iter().copied().map(Some))
    }
    fn to_f32_nan_null(&self) -> Vec<f32> {
        self.data.iter().map(|&x| x as f32).collect()
    }
    fn write_f32_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * 4);
        for (i, &v) in self.data.iter().enumerate() {
            dst[i * 4..i * 4 + 4].copy_from_slice(&(v as f32).to_le_bytes());
        }
    }
}

impl ColumnSource for Column<f32> {
    fn index(&self) -> usize {
        self.index
    }
    fn len(&self) -> usize {
        self.data.len()
    }
    fn min(&self) -> f64 {
        self.min as f64
    }
    fn max(&self) -> f64 {
        self.max as f64
    }
    fn numeric_range(&self) -> Option<(f64, f64)> {
        if self.data.is_empty() {
            None
        } else {
            Some((self.min as f64, self.max as f64))
        }
    }
    fn iter_f64_nullable(&self) -> Box<dyn Iterator<Item = Option<f64>> + '_> {
        Box::new(self.data.iter().map(|&x| Some(x as f64)))
    }
    fn to_f32_nan_null(&self) -> Vec<f32> {
        self.data.clone()
    }
    fn write_f32_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * 4);
        // Already f32; on little-endian targets bit patterns match.
        let dst_f32: &mut [f32] = bytemuck::cast_slice_mut(dst);
        dst_f32.copy_from_slice(&self.data);
    }
}

impl ColumnSource for Column<Option<f64>> {
    fn index(&self) -> usize {
        self.index
    }
    fn len(&self) -> usize {
        self.data.len()
    }
    fn min(&self) -> f64 {
        self.min.unwrap_or(f64::NAN)
    }
    fn max(&self) -> f64 {
        self.max.unwrap_or(f64::NAN)
    }
    fn numeric_range(&self) -> Option<(f64, f64)> {
        match (self.min, self.max) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        }
    }
    fn iter_f64_nullable(&self) -> Box<dyn Iterator<Item = Option<f64>> + '_> {
        Box::new(self.data.iter().copied())
    }
    fn to_f32_nan_null(&self) -> Vec<f32> {
        self.data
            .iter()
            .map(|v| v.map(|x| x as f32).unwrap_or(f32::NAN))
            .collect()
    }
    fn write_f32_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * 4);
        for (i, opt) in self.data.iter().enumerate() {
            let v = opt.map(|x| x as f32).unwrap_or(f32::NAN);
            dst[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
}

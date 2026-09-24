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

/// Safe write-only access to a column's logical `(hi, lo)` f32 pairs.
///
/// Sources can inspect only the number of pairs and write a complete pair by
/// index. The mapped bytes and the renderer's wgpu dependency stay private.
pub struct ColumnPairWriter<'a> {
    dst: wgpu::WriteOnly<'a, [u8]>,
    observer: Option<&'a mut dyn FnMut(usize, f32, f32)>,
}

impl<'a> ColumnPairWriter<'a> {
    pub(crate) fn new(dst: wgpu::WriteOnly<'a, [u8]>) -> Self {
        assert_eq!(
            dst.len() % COLUMN_VALUE_BYTES,
            0,
            "column pair destination must contain complete f32 pairs"
        );
        Self { dst, observer: None }
    }

    pub(crate) fn new_observed(
        dst: wgpu::WriteOnly<'a, [u8]>,
        observer: &'a mut dyn FnMut(usize, f32, f32),
    ) -> Self {
        let mut writer = Self::new(dst);
        writer.observer = Some(observer);
        writer
    }

    #[cfg(test)]
    pub(crate) fn from_bytes_for_test(dst: &'a mut [u8]) -> Self {
        Self::new(wgpu::WriteOnly::from_mut(dst))
    }

    /// Number of logical `(hi, lo)` pairs available for writing.
    pub fn len(&self) -> usize {
        self.dst.len() / COLUMN_VALUE_BYTES
    }

    /// Returns `true` when no logical pairs are available for writing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Write one little-endian `(hi, lo)` pair.
    ///
    /// Panics when `index` is outside `0..self.len()`.
    #[track_caller]
    pub fn write_pair(&mut self, index: usize, hi: f32, lo: f32) {
        assert!(
            index < self.len(),
            "column pair index {index} out of bounds for length {}",
            self.len()
        );
        let start = index * COLUMN_VALUE_BYTES;
        let hi_bytes = hi.to_le_bytes();
        let lo_bytes = lo.to_le_bytes();
        self.dst
            .slice(start..start + COLUMN_VALUE_BYTES)
            .copy_from_slice(&[hi_bytes[0], hi_bytes[1], hi_bytes[2], hi_bytes[3], lo_bytes[0], lo_bytes[1], lo_bytes[2], lo_bytes[3]]);
        if let Some(observer) = self.observer.as_mut() {
            observer(index, hi, lo);
        }
    }
}

/// Scalar statistics collected while a source writes its GPU pair encoding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnUploadStats {
    /// Smallest finite value greater than zero in the recorded pair stream.
    pub min_positive: Option<f64>,
}

/// Failure reported by a range-capable column adapter before a stream chunk is
/// published. Existing full-column adapters remain source-compatible and
/// report `Unsupported` until they implement the range method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnRangeWriteError {
    Unsupported,
    InvalidRange,
    SourceFailed,
}

impl ColumnRangeWriteError {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Unsupported => "source does not support range writes",
            Self::InvalidRange => "requested source range is invalid",
            Self::SourceFailed => "source failed while writing the requested range",
        }
    }
}

#[derive(Default)]
struct EncodedBounds {
    min: f64,
    max: f64,
    min_positive: Option<f64>,
    any: bool,
}

impl EncodedBounds {
    fn record(&mut self, hi: f32, lo: f32) {
        let gpu_value = hi + lo;
        let value = hi as f64 + lo as f64;
        if !gpu_value.is_finite() || !value.is_finite() {
            return;
        }
        let value = if value == 0.0 { 0.0 } else { value };
        if self.any {
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        } else {
            self.min = value;
            self.max = value;
            self.any = true;
        }
        if value > 0.0 && self.min_positive.is_none_or(|current| value < current) {
            self.min_positive = Some(value);
        }
    }

    fn finish(self) -> Option<crate::StreamBounds> {
        self.any.then_some(crate::StreamBounds {
            min: self.min,
            max: self.max,
            min_positive: self.min_positive,
        })
    }
}

fn checked_source_range(
    total: usize,
    start: u64,
    len: usize,
) -> std::result::Result<std::ops::Range<usize>, ColumnRangeWriteError> {
    let start = usize::try_from(start).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
    let end = start
        .checked_add(len)
        .filter(|end| *end <= total)
        .ok_or(ColumnRangeWriteError::InvalidRange)?;
    Ok(start..end)
}

#[inline]
fn record_min_positive(stats: &mut ColumnUploadStats, value: f64) {
    if !value.is_finite() || value <= 0.0 {
        return;
    }
    if match stats.min_positive {
        Some(current) => value < current,
        None => true,
    } {
        stats.min_positive = Some(value);
    }
}

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
/// `min` / `max` describe the source values, while
/// [`Self::write_f32_pair_le_into_with_stats`] writes the GPU representation
/// and returns encoded-value stats without reading mapped bytes. Nulls encode
/// as `f32::NAN`; null / non-numeric handling is the implementor's
/// responsibility.
pub trait ColumnSource {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn max(&self) -> f64;
    fn min(&self) -> f64;

    /// Legacy scalar encoding helper. The caller guarantees
    /// `dst.len() == self.len() * 4`; nulls encode as `f32::NAN`. Pool upload
    /// uses [`Self::write_f32_pair_le_into_with_stats`] instead.
    fn write_f32_le_into(&self, dst: &mut [u8]);

    /// Write the scalar GPU representation directly as `(value, 0)` f32
    /// pairs. The fallback preserves source compatibility and expands in
    /// place without allocating a per-value buffer. Pool upload uses the
    /// write-only fused capability below instead.
    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        let n = self.len();
        debug_assert_eq!(dst.len(), n * COLUMN_VALUE_BYTES);
        let scalar_bytes = n * std::mem::size_of::<f32>();
        self.write_f32_le_into(&mut dst[..scalar_bytes]);
        if n == 0 {
            return;
        }
        let words: &mut [u32] = bytemuck::cast_slice_mut(dst);
        for i in (0..n).rev() {
            let scalar_bits = words[i];
            words[i * 2] = scalar_bits;
            words[i * 2 + 1] = 0;
        }
    }

    /// Write scalar `(value, 0)` pairs and collect stats in that same pass.
    ///
    /// The caller guarantees `dst.len() == self.len() * 8`. `min_positive`
    /// uses each actual uploaded `value as f32`, widened to f64, and includes
    /// only finite values greater than zero.
    ///
    /// Implementations must write every pair and return the statistics from
    /// that same pass. Keeping this method required makes an incomplete custom
    /// source fail at compile time rather than during an upload.
    fn write_f32_pair_le_into_with_stats(&self, dst: ColumnPairWriter<'_>) -> ColumnUploadStats;

    /// Write `dst.len()` scalar values beginning at the logical `start` index
    /// directly into mapped staging. Returned bounds describe the encoded
    /// `(value as f32, 0)` values written by this call.
    fn write_f32_pair_range_into_with_stats(
        &self,
        _start: u64,
        _dst: ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, ColumnRangeWriteError> {
        Err(ColumnRangeWriteError::Unsupported)
    }
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

    /// Legacy helper that writes `len * 8` bytes as little-endian `(hi: f32,
    /// lo: f32)` pairs. Pool upload uses the write-only fused capability.
    fn write_f32_pair_le_into(&self, dst: &mut [u8]);

    /// Write hi/lo pairs and collect stats from recorded `hi as f64 + lo as
    /// f64` values in that same pass, including only finite values greater
    /// than zero. The caller guarantees `dst.len() == self.len() * 8`. See
    /// [`ColumnSource::write_f32_pair_le_into_with_stats`] for the required
    /// single-pass contract.
    fn write_f32_pair_le_into_with_stats(&self, dst: ColumnPairWriter<'_>) -> ColumnUploadStats;

    /// Write `dst.len()` hi/lo values beginning at the logical `start` index
    /// directly into mapped staging. Returned bounds describe finite GPU-
    /// reconstructible values written by this call.
    fn write_f32_pair_range_into_with_stats(
        &self,
        _start: u64,
        _dst: ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, ColumnRangeWriteError> {
        Err(ColumnRangeWriteError::Unsupported)
    }
}

/// Borrowed range-capable source selected by registered stream encoding.
#[derive(Clone, Copy)]
pub enum StreamColumnSource<'a> {
    Scalar(&'a dyn ColumnSource),
    HiLo(&'a dyn HiLoColumnSource),
}

impl StreamColumnSource<'_> {
    pub fn len(self) -> usize {
        match self {
            Self::Scalar(source) => source.len(),
            Self::HiLo(source) => source.len(),
        }
    }

    pub fn encoding(self) -> crate::StreamEncoding {
        match self {
            Self::Scalar(_) => crate::StreamEncoding::ScalarF32,
            Self::HiLo(_) => crate::StreamEncoding::HiLoF32,
        }
    }

    pub(crate) fn write_range(
        self,
        start: u64,
        dst: ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, ColumnRangeWriteError> {
        match self {
            Self::Scalar(source) => source.write_f32_pair_range_into_with_stats(start, dst),
            Self::HiLo(source) => source.write_f32_pair_range_into_with_stats(start, dst),
        }
    }
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
    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * COLUMN_VALUE_BYTES);
        for (pair, &value) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).zip(&self.data) {
            pair[..4].copy_from_slice(&(value as f32).to_le_bytes());
            pair[4..].copy_from_slice(&0.0f32.to_le_bytes());
        }
    }
    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        debug_assert_eq!(dst.len(), self.data.len());
        let mut stats = ColumnUploadStats { min_positive: None };
        for (index, &value) in self.data.iter().enumerate() {
            let value = value as f32;
            dst.write_pair(index, value, 0.0);
            record_min_positive(&mut stats, value as f64);
        }
        stats
    }
    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, ColumnRangeWriteError> {
        let range = checked_source_range(self.data.len(), start, dst.len())?;
        let mut bounds = EncodedBounds::default();
        for (index, &value) in self.data[range].iter().enumerate() {
            let hi = value as f32;
            dst.write_pair(index, hi, 0.0);
            bounds.record(hi, 0.0);
        }
        Ok(bounds.finish())
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
        for (pair, &value) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).zip(&self.data) {
            let (hi, lo) = split_f64_to_f32_pair(value);
            pair[..4].copy_from_slice(&hi.to_le_bytes());
            pair[4..].copy_from_slice(&lo.to_le_bytes());
        }
    }
    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        debug_assert_eq!(dst.len(), self.data.len());
        let mut stats = ColumnUploadStats { min_positive: None };
        for (index, &v) in self.data.iter().enumerate() {
            let (hi, lo) = split_f64_to_f32_pair(v);
            dst.write_pair(index, hi, lo);
            record_min_positive(&mut stats, hi as f64 + lo as f64);
        }
        stats
    }
    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, ColumnRangeWriteError> {
        let range = checked_source_range(self.data.len(), start, dst.len())?;
        let mut bounds = EncodedBounds::default();
        for (index, &value) in self.data[range].iter().enumerate() {
            let (hi, lo) = split_f64_to_f32_pair(value);
            dst.write_pair(index, hi, lo);
            bounds.record(hi, lo);
        }
        Ok(bounds.finish())
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
    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * COLUMN_VALUE_BYTES);
        for (pair, &value) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).zip(&self.data) {
            pair[..4].copy_from_slice(&value.to_le_bytes());
            pair[4..].copy_from_slice(&0.0f32.to_le_bytes());
        }
    }
    fn write_f32_pair_le_into_with_stats(&self, dst: ColumnPairWriter<'_>) -> ColumnUploadStats {
        <Self as HiLoColumnSource>::write_f32_pair_le_into_with_stats(self, dst)
    }
    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, ColumnRangeWriteError> {
        let range = checked_source_range(self.data.len(), start, dst.len())?;
        let mut bounds = EncodedBounds::default();
        for (index, &hi) in self.data[range].iter().enumerate() {
            dst.write_pair(index, hi, 0.0);
            bounds.record(hi, 0.0);
        }
        Ok(bounds.finish())
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
        <Self as ColumnSource>::write_f32_zero_lo_pair_le_into(self, dst);
    }
    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        debug_assert_eq!(dst.len(), self.data.len());
        let mut stats = ColumnUploadStats { min_positive: None };
        for (index, &hi) in self.data.iter().enumerate() {
            dst.write_pair(index, hi, 0.0);
            record_min_positive(&mut stats, hi as f64);
        }
        stats
    }
    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        dst: ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, ColumnRangeWriteError> {
        <Self as ColumnSource>::write_f32_pair_range_into_with_stats(self, start, dst)
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
    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * COLUMN_VALUE_BYTES);
        for (pair, value) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).zip(&self.data) {
            let value = value.map(|value| value as f32).unwrap_or(f32::NAN);
            pair[..4].copy_from_slice(&value.to_le_bytes());
            pair[4..].copy_from_slice(&0.0f32.to_le_bytes());
        }
    }
    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        debug_assert_eq!(dst.len(), self.data.len());
        let mut stats = ColumnUploadStats { min_positive: None };
        for (index, value) in self.data.iter().enumerate() {
            let value = value.map(|value| value as f32).unwrap_or(f32::NAN);
            dst.write_pair(index, value, 0.0);
            record_min_positive(&mut stats, value as f64);
        }
        stats
    }
    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, ColumnRangeWriteError> {
        let range = checked_source_range(self.data.len(), start, dst.len())?;
        let mut bounds = EncodedBounds::default();
        for (index, value) in self.data[range].iter().enumerate() {
            let hi = value.map(|value| value as f32).unwrap_or(f32::NAN);
            dst.write_pair(index, hi, 0.0);
            bounds.record(hi, 0.0);
        }
        Ok(bounds.finish())
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
        for (pair, value) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).zip(&self.data) {
            let (hi, lo) = value.map(split_f64_to_f32_pair).unwrap_or((f32::NAN, 0.0));
            pair[..4].copy_from_slice(&hi.to_le_bytes());
            pair[4..].copy_from_slice(&lo.to_le_bytes());
        }
    }
    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        debug_assert_eq!(dst.len(), self.data.len());
        let mut stats = ColumnUploadStats { min_positive: None };
        for (index, opt) in self.data.iter().enumerate() {
            let (hi, lo) = opt.map(split_f64_to_f32_pair).unwrap_or((f32::NAN, 0.0));
            dst.write_pair(index, hi, lo);
            record_min_positive(&mut stats, hi as f64 + lo as f64);
        }
        stats
    }
    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, ColumnRangeWriteError> {
        let range = checked_source_range(self.data.len(), start, dst.len())?;
        let mut bounds = EncodedBounds::default();
        for (index, value) in self.data[range].iter().enumerate() {
            let (hi, lo) = value.map(split_f64_to_f32_pair).unwrap_or((f32::NAN, 0.0));
            dst.write_pair(index, hi, lo);
            bounds.record(hi, lo);
        }
        Ok(bounds.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_upload(source: &dyn ColumnSource) -> (Vec<(f32, f32)>, ColumnUploadStats) {
        let mut bytes = vec![0; source.len() * COLUMN_VALUE_BYTES];
        let stats = source
            .write_f32_pair_le_into_with_stats(ColumnPairWriter::from_bytes_for_test(&mut bytes));
        (decode_pairs(&bytes), stats)
    }

    fn hilo_upload(source: &dyn HiLoColumnSource) -> (Vec<(f32, f32)>, ColumnUploadStats) {
        let mut bytes = vec![0; source.len() * COLUMN_VALUE_BYTES];
        let stats = source
            .write_f32_pair_le_into_with_stats(ColumnPairWriter::from_bytes_for_test(&mut bytes));
        (decode_pairs(&bytes), stats)
    }

    fn decode_pairs(bytes: &[u8]) -> Vec<(f32, f32)> {
        bytes
            .chunks_exact(COLUMN_VALUE_BYTES)
            .map(|pair| {
                (
                    f32::from_le_bytes(pair[..4].try_into().unwrap()),
                    f32::from_le_bytes(pair[4..].try_into().unwrap()),
                )
            })
            .collect()
    }

    #[test]
    fn built_in_sources_write_requested_ranges_without_full_column_buffers() {
        let source = Column {
            data: vec![1.0f64, 2.25, 3.5, 4.75, 6.0],
            min: 1.0,
            max: 6.0,
        };
        let mut scalar_bytes = vec![0; 2 * COLUMN_VALUE_BYTES];
        let scalar_bounds = <Column<f64> as ColumnSource>::write_f32_pair_range_into_with_stats(
            &source,
            2,
            ColumnPairWriter::from_bytes_for_test(&mut scalar_bytes),
        )
        .unwrap();
        assert_eq!(decode_pairs(&scalar_bytes), vec![(3.5, 0.0), (4.75, 0.0)]);
        assert_eq!(
            scalar_bounds,
            Some(crate::StreamBounds {
                min: 3.5,
                max: 4.75,
                min_positive: Some(3.5),
            })
        );

        let mut hilo_bytes = vec![0; 2 * COLUMN_VALUE_BYTES];
        let hilo_bounds = <Column<f64> as HiLoColumnSource>::write_f32_pair_range_into_with_stats(
            &source,
            1,
            ColumnPairWriter::from_bytes_for_test(&mut hilo_bytes),
        )
        .unwrap();
        let pairs = decode_pairs(&hilo_bytes);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0 as f64 + pairs[0].1 as f64, 2.25);
        assert_eq!(pairs[1].0 as f64 + pairs[1].1 as f64, 3.5);
        assert_eq!(
            hilo_bounds,
            Some(crate::StreamBounds {
                min: 2.25,
                max: 3.5,
                min_positive: Some(2.25),
            })
        );
    }

    #[test]
    fn pair_writer_reports_logical_length_and_checks_bounds() {
        let mut bytes = [0; 2 * COLUMN_VALUE_BYTES];
        {
            let mut writer = ColumnPairWriter::from_bytes_for_test(&mut bytes);
            assert_eq!(writer.len(), 2);
            assert!(!writer.is_empty());
            writer.write_pair(1, 3.5, -0.25);
        }
        assert_eq!(decode_pairs(&bytes)[1], (3.5, -0.25));

        let mut empty = [];
        assert!(ColumnPairWriter::from_bytes_for_test(&mut empty).is_empty());
    }

    #[test]
    #[should_panic(expected = "column pair index 1 out of bounds for length 1")]
    fn pair_writer_rejects_out_of_bounds_writes() {
        let mut bytes = [0; COLUMN_VALUE_BYTES];
        ColumnPairWriter::from_bytes_for_test(&mut bytes).write_pair(1, 0.0, 0.0);
    }

    #[test]
    fn built_in_fused_scalar_stats_follow_recorded_f32_values() {
        let min_subnormal = f32::from_bits(1);
        let source = Column {
            data: vec![f64::from_bits(1), f64::MAX, min_subnormal as f64],
            min: f64::from_bits(1),
            max: f64::MAX,
        };
        let (pairs, stats) = scalar_upload(&source);

        assert_eq!(pairs[0], (0.0, 0.0));
        assert_eq!(pairs[1], (f32::INFINITY, 0.0));
        assert_eq!(stats.min_positive, Some(min_subnormal as f64));
    }

    #[test]
    fn built_in_fused_hilo_stats_and_precision_use_recorded_pairs() {
        let cancellation = 16_777_215.5_f64;
        let epoch_a = 1_700_000_000_000.125_f64;
        let epoch_b = epoch_a + 0.75;
        let source = Column {
            data: vec![cancellation, epoch_a, epoch_b],
            min: cancellation,
            max: epoch_b,
        };
        let (pairs, stats) = hilo_upload(&source);
        let (cancel_hi, cancel_lo) = split_f64_to_f32_pair(cancellation);

        assert_eq!(pairs[0], (cancel_hi, cancel_lo));
        assert_eq!(
            stats.min_positive,
            Some(cancel_hi as f64 + cancel_lo as f64)
        );
        let epoch_delta = (pairs[2].0 - pairs[1].0) + (pairs[2].1 - pairs[1].1);
        assert!((epoch_delta as f64 - 0.75).abs() < 1.0e-3);
    }

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

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
/// The writer records the column's statistics from the pairs written: the
/// finite minimum and maximum and the smallest value above zero, each the
/// `hi + lo` value the GPU reads. A source supplies values, never statistics.
pub struct ColumnPairWriter<'a> {
    dst: wgpu::WriteOnly<'a, [u8]>,
    observer: Option<&'a mut dyn FnMut(usize, f32, f32)>,
    /// Store each pair as a `u32` index lane instead of the f32 pair: a
    /// ragged column's offsets (`SHADER_COMMON.md` §5.14).
    index_lanes: bool,
    /// Set when the source writes an index outside `0..len()`. The write is
    /// not performed, and the caller that built the writer rejects the
    /// upload once the source returns (#152).
    out_of_range: &'a std::cell::Cell<bool>,
    /// The statistics of the pairs written so far.
    bounds: EncodedBounds,
}

impl<'a> ColumnPairWriter<'a> {
    /// A writer over `dst`. `len()` counts complete pairs only, so a write
    /// never reaches past `dst`; a write outside `0..len()` sets
    /// `out_of_range`, which the caller checks after the source returns.
    pub(crate) fn new(
        dst: wgpu::WriteOnly<'a, [u8]>,
        out_of_range: &'a std::cell::Cell<bool>,
    ) -> Self {
        Self {
            dst,
            observer: None,
            index_lanes: false,
            out_of_range,
            bounds: EncodedBounds::default(),
        }
    }
    /// Store every pair as an index: the integer `hi + lo`, summed in f64,
    /// as `u32` bits in the first lane and 0 in the second, so a shader reads
    /// it as `bitcast<u32>` and every index up to `u32::MAX` is exact. The
    /// observer still sees the pair the source wrote. A value that is not an
    /// integer in `0..=u32::MAX` stores a saturated index; the caller rejects
    /// such a column before it is published.
    pub(crate) fn with_index_lanes(mut self, index_lanes: bool) -> Self {
        self.index_lanes = index_lanes;
        self
    }

    pub(crate) fn new_observed(
        dst: wgpu::WriteOnly<'a, [u8]>,
        out_of_range: &'a std::cell::Cell<bool>,
        observer: &'a mut dyn FnMut(usize, f32, f32),
    ) -> Self {
        let mut writer = Self::new(dst, out_of_range);
        writer.observer = Some(observer);
        writer
    }

    #[cfg(test)]
    pub(crate) fn from_bytes_for_test(
        dst: &'a mut [u8],
        out_of_range: &'a std::cell::Cell<bool>,
    ) -> Self {
        Self::new(wgpu::WriteOnly::from_mut(dst), out_of_range)
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
    /// A write with `index` outside `0..self.len()` is not performed, and
    /// the upload this writer belongs to fails with an error once the source
    /// returns.
    pub fn write_pair(&mut self, index: usize, hi: f32, lo: f32) {
        if index >= self.len() {
            self.out_of_range.set(true);
            return;
        }
        let start = index * COLUMN_VALUE_BYTES;
        let (hi_bytes, lo_bytes) = if self.index_lanes {
            (index_lane(hi, lo).to_le_bytes(), [0; 4])
        } else {
            (hi.to_le_bytes(), lo.to_le_bytes())
        };
        self.dst
            .slice(start..start + COLUMN_VALUE_BYTES)
            .copy_from_slice(&[
                hi_bytes[0],
                hi_bytes[1],
                hi_bytes[2],
                hi_bytes[3],
                lo_bytes[0],
                lo_bytes[1],
                lo_bytes[2],
                lo_bytes[3],
            ]);
        self.bounds.record(hi, lo);
        if let Some(observer) = self.observer.as_mut() {
            observer(index, hi, lo);
        }
    }

    /// The statistics of the pairs written: `None` when none was finite.
    pub(crate) fn bounds(&self) -> Option<crate::StreamBounds> {
        self.bounds.finish()
    }
}

/// The `u32` index lane of a `(hi, lo)` pair: `hi + lo` summed in f64,
/// exact for every integer up to `u32::MAX`, saturated outside that range
/// (NaN stores 0).
pub(crate) fn index_lane(hi: f32, lo: f32) -> u32 {
    (hi as f64 + lo as f64) as u32
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

/// The uniform step of a column's values, measured while they are written
/// (#134): `Some(step)` when at least two values were written in index
/// order, every one finite, and every consecutive difference equals the
/// first within 1e-6 relative; `None` otherwise, including a zero step. A
/// hexagonal grid's coordinate columns are held to it at registration; the
/// pool keeps the one scalar, never the values (rule 2).
#[derive(Debug, Clone, Copy)]
pub(crate) struct UniformStep {
    next: usize,
    prev: f64,
    step: f64,
    count: usize,
    uniform: bool,
}

impl UniformStep {
    pub(crate) fn new() -> Self {
        Self {
            next: 0,
            prev: 0.0,
            step: 0.0,
            count: 0,
            uniform: true,
        }
    }

    pub(crate) fn record(&mut self, index: usize, hi: f32, lo: f32) {
        if !self.uniform {
            return;
        }
        let value = hi as f64 + lo as f64;
        if index != self.next || !value.is_finite() {
            self.uniform = false;
            return;
        }
        match self.count {
            0 => {}
            1 => {
                self.step = value - self.prev;
                if self.step == 0.0 || !self.step.is_finite() {
                    self.uniform = false;
                }
            }
            _ => {
                let step = value - self.prev;
                if (step - self.step).abs() > 1e-6 * self.step.abs() {
                    self.uniform = false;
                }
            }
        }
        self.prev = value;
        self.count += 1;
        self.next += 1;
    }

    pub(crate) fn finish(self) -> Option<f64> {
        (self.uniform && self.count >= 2).then_some(self.step)
    }
}

#[derive(Debug, Default, Clone, Copy)]
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

    fn finish(&self) -> Option<crate::StreamBounds> {
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
/// A source writes its values into mapped staging as `(hi, lo)` f32 pairs
/// and nothing else: the [`ColumnPairWriter`] records the column's
/// statistics (minimum, maximum, smallest positive) from those pairs while
/// they are written, so the renderer owns them and no separate pass over the
/// values is needed. Nulls encode as `f32::NAN`; null / non-numeric handling
/// is the implementor's responsibility.
pub trait ColumnSource {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Write every value as a `(value, 0)` f32 pair, or as a `(hi, lo)` split
    /// for a source that keeps more than f32 precision, each index in
    /// `0..len()` once.
    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>);

    /// Write `dst.len()` values beginning at the logical `start` index
    /// directly into mapped staging: one streamed chunk. The writer records
    /// that chunk's statistics.
    fn write_pair_range(
        &self,
        _start: u64,
        _dst: &mut ColumnPairWriter<'_>,
    ) -> std::result::Result<(), ColumnRangeWriteError> {
        Err(ColumnRangeWriteError::Unsupported)
    }
}

/// High-precision column upload path.
///
/// Each logical value is encoded as two f32 lanes `(hi, lo)` so shaders can
/// subtract split axis bounds before recombining. This preserves small
/// timestamp deltas around large Unix epoch values. As for
/// [`ColumnSource`], the writer records the statistics.
pub trait HiLoColumnSource {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Write every value as a `(hi, lo)` pair, each index in `0..len()` once.
    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>);

    /// Write `dst.len()` hi/lo values beginning at the logical `start` index
    /// directly into mapped staging: one streamed chunk.
    fn write_pair_range(
        &self,
        _start: u64,
        _dst: &mut ColumnPairWriter<'_>,
    ) -> std::result::Result<(), ColumnRangeWriteError> {
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
        dst: &mut ColumnPairWriter<'_>,
    ) -> std::result::Result<(), ColumnRangeWriteError> {
        match self {
            Self::Scalar(source) => source.write_pair_range(start, dst),
            Self::HiLo(source) => source.write_pair_range(start, dst),
        }
    }
}

// Built-in implementations for numeric column types.

/// Write `values[start..]` through `pair`, at writer indices from 0.
fn write_range_with<T: Copy>(
    values: &[T],
    start: u64,
    dst: &mut ColumnPairWriter<'_>,
    pair: impl Fn(T) -> (f32, f32),
) -> std::result::Result<(), ColumnRangeWriteError> {
    let range = checked_source_range(values.len(), start, dst.len())?;
    for (index, &value) in values[range].iter().enumerate() {
        let (hi, lo) = pair(value);
        dst.write_pair(index, hi, lo);
    }
    Ok(())
}

/// Write every value through `pair`.
fn write_all_with<T: Copy>(
    values: &[T],
    dst: &mut ColumnPairWriter<'_>,
    pair: impl Fn(T) -> (f32, f32),
) {
    for (index, &value) in values.iter().enumerate() {
        let (hi, lo) = pair(value);
        dst.write_pair(index, hi, lo);
    }
}

/// A missing value is NaN; a present one keeps the (hi, lo) split (#107).
fn split_optional(value: Option<f64>) -> (f32, f32) {
    value.map_or((f32::NAN, 0.0), split_f64_to_f32_pair)
}

// An f64 column writes the (hi, lo) split on both paths: it keeps about 48
// bits on the GPU, not f32's 24 (#107), and streamed windows use the same
// split (the maintainer's exception to rule 8 for upload precision).
impl ColumnSource for Column<f64> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        write_all_with(&self.data, dst, split_f64_to_f32_pair);
    }
    fn write_pair_range(
        &self,
        start: u64,
        dst: &mut ColumnPairWriter<'_>,
    ) -> std::result::Result<(), ColumnRangeWriteError> {
        write_range_with(&self.data, start, dst, split_f64_to_f32_pair)
    }
}

impl HiLoColumnSource for Column<f64> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        write_all_with(&self.data, dst, split_f64_to_f32_pair);
    }
    fn write_pair_range(
        &self,
        start: u64,
        dst: &mut ColumnPairWriter<'_>,
    ) -> std::result::Result<(), ColumnRangeWriteError> {
        write_range_with(&self.data, start, dst, split_f64_to_f32_pair)
    }
}

impl ColumnSource for Column<f32> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        write_all_with(&self.data, dst, |value| (value, 0.0));
    }
    fn write_pair_range(
        &self,
        start: u64,
        dst: &mut ColumnPairWriter<'_>,
    ) -> std::result::Result<(), ColumnRangeWriteError> {
        write_range_with(&self.data, start, dst, |value| (value, 0.0))
    }
}

impl HiLoColumnSource for Column<f32> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        write_all_with(&self.data, dst, |value| (value, 0.0));
    }
    fn write_pair_range(
        &self,
        start: u64,
        dst: &mut ColumnPairWriter<'_>,
    ) -> std::result::Result<(), ColumnRangeWriteError> {
        write_range_with(&self.data, start, dst, |value| (value, 0.0))
    }
}

/// The borrowed values of a converted legacy document
/// (`Config::from_categorical`'s uploads): written straight from the
/// document into the staging pairs, as `(hi, lo)` so the document's f64
/// values keep their precision. A missing value uploads NaN. A box plot's
/// record and outlier uploads are not one column and hold no values here:
/// they upload through their [`BoxColumn`](::model::bounded_sources::BoxColumn)
/// adapters.
impl ColumnSource for ::model::bounded_sources::UploadValues<'_> {
    fn len(&self) -> usize {
        ::model::bounded_sources::UploadValues::len(self)
    }
    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        for index in 0..ColumnSource::len(self) {
            let (hi, lo) = split_f64_to_f32_pair(self.value(index));
            dst.write_pair(index, hi, lo);
        }
    }
}

/// One column of a converted box plot document (`Config::from_boxplot`'s
/// record fields, outlier values and offsets): every value is computed from
/// the borrowed summaries while it is written into the staging pairs, as
/// `(hi, lo)` so the document's f64 values keep their precision and every
/// offset reaches the pool's `u32` index lanes exactly. A missing box writes
/// NaN.
impl ColumnSource for ::model::bounded_sources::BoxColumn<'_> {
    fn len(&self) -> usize {
        ::model::bounded_sources::BoxColumn::len(self)
    }
    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        self.for_each(|index, value| {
            let (hi, lo) = split_f64_to_f32_pair(value);
            dst.write_pair(index, hi, lo);
        });
    }
}

impl ColumnSource for Column<Option<f64>> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        write_all_with(&self.data, dst, split_optional);
    }
    fn write_pair_range(
        &self,
        start: u64,
        dst: &mut ColumnPairWriter<'_>,
    ) -> std::result::Result<(), ColumnRangeWriteError> {
        write_range_with(&self.data, start, dst, split_optional)
    }
}

impl HiLoColumnSource for Column<Option<f64>> {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        write_all_with(&self.data, dst, split_optional);
    }
    fn write_pair_range(
        &self,
        start: u64,
        dst: &mut ColumnPairWriter<'_>,
    ) -> std::result::Result<(), ColumnRangeWriteError> {
        write_range_with(&self.data, start, dst, split_optional)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Bounds = Option<crate::StreamBounds>;

    fn upload(
        len: usize,
        write: impl FnOnce(&mut ColumnPairWriter<'_>),
    ) -> (Vec<(f32, f32)>, Bounds) {
        let mut bytes = vec![0; len * COLUMN_VALUE_BYTES];
        let out_of_range = std::cell::Cell::new(false);
        let bounds = {
            let mut writer = ColumnPairWriter::from_bytes_for_test(&mut bytes, &out_of_range);
            write(&mut writer);
            writer.bounds()
        };
        (decode_pairs(&bytes), bounds)
    }

    fn scalar_upload(source: &dyn ColumnSource) -> (Vec<(f32, f32)>, Bounds) {
        upload(source.len(), |dst| source.write_pairs(dst))
    }

    fn hilo_upload(source: &dyn HiLoColumnSource) -> (Vec<(f32, f32)>, Bounds) {
        upload(source.len(), |dst| source.write_pairs(dst))
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
        };
        let (scalar_pairs, scalar_bounds) = upload(2, |dst| {
            <Column<f64> as ColumnSource>::write_pair_range(&source, 2, dst).unwrap()
        });
        assert_eq!(scalar_pairs, vec![(3.5, 0.0), (4.75, 0.0)]);
        assert_eq!(
            scalar_bounds,
            Some(crate::StreamBounds {
                min: 3.5,
                max: 4.75,
                min_positive: Some(3.5),
            })
        );

        let (pairs, hilo_bounds) = upload(2, |dst| {
            <Column<f64> as HiLoColumnSource>::write_pair_range(&source, 1, dst).unwrap()
        });
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
        let out_of_range = std::cell::Cell::new(false);
        {
            let mut writer = ColumnPairWriter::from_bytes_for_test(&mut bytes, &out_of_range);
            assert_eq!(writer.len(), 2);
            assert!(!writer.is_empty());
            writer.write_pair(1, 3.5, -0.25);
        }
        assert_eq!(decode_pairs(&bytes)[1], (3.5, -0.25));
        assert!(!out_of_range.get());

        let mut empty = [];
        assert!(ColumnPairWriter::from_bytes_for_test(&mut empty, &out_of_range).is_empty());
    }

    #[test]
    fn pair_writer_rejects_out_of_bounds_writes() {
        let mut bytes = [7; COLUMN_VALUE_BYTES + 3];
        let out_of_range = std::cell::Cell::new(false);
        let mut writer = ColumnPairWriter::from_bytes_for_test(&mut bytes, &out_of_range);
        assert_eq!(writer.len(), 1);
        writer.write_pair(1, 0.0, 0.0);
        assert!(out_of_range.get());
        assert_eq!(bytes, [7; COLUMN_VALUE_BYTES + 3]);
    }

    #[test]
    fn pair_writer_records_statistics_of_the_pairs_written() {
        // Non-finite and out-of-range writes leave the statistics alone; a
        // negative zero counts as zero; the smallest positive is tracked.
        let out_of_range = std::cell::Cell::new(false);
        let mut bytes = [0; 5 * COLUMN_VALUE_BYTES];
        let mut writer = ColumnPairWriter::from_bytes_for_test(&mut bytes, &out_of_range);
        assert_eq!(writer.bounds(), None);
        writer.write_pair(0, f32::NAN, 0.0);
        writer.write_pair(1, f32::INFINITY, 0.0);
        assert_eq!(writer.bounds(), None);
        writer.write_pair(2, -0.0, 0.0);
        writer.write_pair(3, 4.0, 0.5);
        writer.write_pair(4, -2.0, 0.0);
        writer.write_pair(9, 100.0, 0.0);
        assert_eq!(
            writer.bounds(),
            Some(crate::StreamBounds {
                min: -2.0,
                max: 4.5,
                min_positive: Some(4.5),
            })
        );
        assert!(out_of_range.get());
    }

    #[test]
    fn built_in_scalar_stats_follow_recorded_f32_values() {
        let min_subnormal = f32::from_bits(1);
        let source = Column {
            data: vec![f64::from_bits(1), f64::MAX, min_subnormal as f64],
        };
        let (pairs, bounds) = scalar_upload(&source);

        assert_eq!(pairs[0], (0.0, 0.0));
        assert_eq!(pairs[1], (f32::INFINITY, 0.0));
        // The value the GPU reads: a magnitude below f32 range is 0 there,
        // and a value past it is infinite and left out.
        let bounds = bounds.unwrap();
        assert_eq!(bounds.min_positive, Some(min_subnormal as f64));
        assert_eq!((bounds.min, bounds.max), (0.0, min_subnormal as f64));
    }

    #[test]
    fn built_in_hilo_stats_and_precision_use_recorded_pairs() {
        let cancellation = 16_777_215.5_f64;
        let epoch_a = 1_700_000_000_000.125_f64;
        let epoch_b = epoch_a + 0.75;
        let source = Column {
            data: vec![cancellation, epoch_a, epoch_b],
        };
        let (pairs, bounds) = hilo_upload(&source);
        let (cancel_hi, cancel_lo) = split_f64_to_f32_pair(cancellation);

        assert_eq!(pairs[0], (cancel_hi, cancel_lo));
        let bounds = bounds.unwrap();
        assert_eq!(
            bounds.min_positive,
            Some(cancel_hi as f64 + cancel_lo as f64)
        );
        assert_eq!(bounds.min, cancel_hi as f64 + cancel_lo as f64);
        assert_eq!(bounds.max, pairs[2].0 as f64 + pairs[2].1 as f64);
        let epoch_delta = (pairs[2].0 - pairs[1].0) + (pairs[2].1 - pairs[1].1);
        assert!((epoch_delta as f64 - 0.75).abs() < 1.0e-3);
    }

    #[test]
    fn optional_columns_leave_missing_values_out_of_the_statistics() {
        let source = Column {
            data: vec![None, Some(-3.0), None, Some(7.0)],
        };
        let (pairs, bounds) = scalar_upload(&source);
        assert!(pairs[0].0.is_nan() && pairs[2].0.is_nan());
        assert_eq!(
            bounds,
            Some(crate::StreamBounds {
                min: -3.0,
                max: 7.0,
                min_positive: Some(7.0),
            })
        );
        let all_missing = Column::<Option<f64>> {
            data: vec![None, None],
        };
        assert_eq!(scalar_upload(&all_missing).1, None);
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

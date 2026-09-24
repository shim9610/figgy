//! Borrowed adapters for streaming web column inputs into renderer staging
//! buffers without constructing an owned per-value mirror.

use std::{ops::Range, rc::Rc};

use renderer::data::{COLUMN_VALUE_BYTES, split_f64_to_f32_pair};
use renderer::{
    ColumnPairWriter, ColumnRangeWriteError, ColumnSource, ColumnUploadStats, HiLoColumnSource,
    StreamBounds,
};

#[derive(Default)]
struct StreamBoundsBuilder {
    min: f64,
    max: f64,
    min_positive: Option<f64>,
    any: bool,
}

impl StreamBoundsBuilder {
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

    fn finish(self) -> Option<StreamBounds> {
        self.any.then_some(StreamBounds {
            min: self.min,
            max: self.max,
            min_positive: self.min_positive,
        })
    }
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

fn write_pair_bytes(dst: &mut [u8], hi: f32, lo: f32) {
    dst[..4].copy_from_slice(&hi.to_le_bytes());
    dst[4..].copy_from_slice(&lo.to_le_bytes());
}

/// One immutable wasm-boundary allocation that can back one column or many
/// range views without copying its payload.
///
/// `Box<[T]>` is intentional: wasm-bindgen can transfer ownership of the
/// allocation it creates while copying a JavaScript typed array into wasm
/// memory. The outer `Rc` only shares that allocation between batch views.
#[derive(Debug)]
pub(crate) struct OwnedColumnBuffer<T> {
    data: Rc<Box<[T]>>,
}

impl<T> Clone for OwnedColumnBuffer<T> {
    fn clone(&self) -> Self {
        Self {
            data: Rc::clone(&self.data),
        }
    }
}

impl<T> OwnedColumnBuffer<T> {
    pub(crate) fn new(data: Box<[T]>) -> Self {
        Self {
            data: Rc::new(data),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.data.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn slice(&self, range: &Range<usize>) -> Option<&[T]> {
        self.data.get(range.clone())
    }

    #[cfg(test)]
    fn shares_allocation_with(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.data, &other.data)
    }
}

/// Immutable scalar-f32 view over a boundary-owned allocation.
#[derive(Debug, Clone)]
pub(crate) struct OwnedF32Column {
    buffer: OwnedColumnBuffer<f32>,
    range: Range<usize>,
    min: f32,
    max: f32,
}

impl OwnedF32Column {
    pub(crate) fn new(data: Box<[f32]>) -> Self {
        let buffer = OwnedColumnBuffer::new(data);
        let end = buffer.len();
        Self::from_buffer(&buffer, 0..end).expect("the whole owned f32 buffer is a valid range")
    }

    pub(crate) fn from_buffer(
        buffer: &OwnedColumnBuffer<f32>,
        range: Range<usize>,
    ) -> Result<Self, ColumnRangeWriteError> {
        let data = buffer
            .slice(&range)
            .ok_or(ColumnRangeWriteError::InvalidRange)?;
        let borrowed = BorrowedF32Column::new(data);
        Ok(Self {
            buffer: buffer.clone(),
            range,
            min: borrowed.min,
            max: borrowed.max,
        })
    }

    fn data(&self) -> &[f32] {
        self.buffer
            .slice(&self.range)
            .expect("an owned f32 column keeps its validated range")
    }

    fn borrowed(&self) -> BorrowedF32Column<'_> {
        BorrowedF32Column {
            data: self.data(),
            min: self.min,
            max: self.max,
        }
    }
}

impl ColumnSource for OwnedF32Column {
    fn len(&self) -> usize {
        self.range.len()
    }

    fn min(&self) -> f64 {
        self.min as f64
    }

    fn max(&self) -> f64 {
        self.max as f64
    }

    fn write_f32_le_into(&self, dst: &mut [u8]) {
        ColumnSource::write_f32_le_into(&self.borrowed(), dst);
    }

    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        ColumnSource::write_f32_zero_lo_pair_le_into(&self.borrowed(), dst);
    }

    fn write_f32_pair_le_into_with_stats(&self, dst: ColumnPairWriter<'_>) -> ColumnUploadStats {
        ColumnSource::write_f32_pair_le_into_with_stats(&self.borrowed(), dst)
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        ColumnSource::write_f32_pair_range_into_with_stats(&self.borrowed(), start, dst)
    }
}

impl HiLoColumnSource for OwnedF32Column {
    fn len(&self) -> usize {
        self.range.len()
    }

    fn min(&self) -> f64 {
        self.min as f64
    }

    fn max(&self) -> f64 {
        self.max as f64
    }

    fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
        HiLoColumnSource::write_f32_pair_le_into(&self.borrowed(), dst);
    }

    fn write_f32_pair_le_into_with_stats(&self, dst: ColumnPairWriter<'_>) -> ColumnUploadStats {
        HiLoColumnSource::write_f32_pair_le_into_with_stats(&self.borrowed(), dst)
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        ColumnSource::write_f32_pair_range_into_with_stats(&self.borrowed(), start, dst)
    }
}

/// Immutable hi/lo-f64 view over a boundary-owned allocation.
///
/// Its `ColumnSource` pair methods deliberately use the split `(hi, lo)`
/// encoding too, so an f64 matrix batch retains the same precision as a
/// single-column `HiLoColumnSource` upload.
#[derive(Debug, Clone)]
pub(crate) struct OwnedF64Column {
    buffer: OwnedColumnBuffer<f64>,
    range: Range<usize>,
    min: f64,
    max: f64,
}

impl OwnedF64Column {
    pub(crate) fn new(data: Box<[f64]>) -> Self {
        let buffer = OwnedColumnBuffer::new(data);
        let end = buffer.len();
        Self::from_buffer(&buffer, 0..end).expect("the whole owned f64 buffer is a valid range")
    }

    pub(crate) fn from_buffer(
        buffer: &OwnedColumnBuffer<f64>,
        range: Range<usize>,
    ) -> Result<Self, ColumnRangeWriteError> {
        let data = buffer
            .slice(&range)
            .ok_or(ColumnRangeWriteError::InvalidRange)?;
        let borrowed = BorrowedF64Column::new(data);
        Ok(Self {
            buffer: buffer.clone(),
            range,
            min: borrowed.min,
            max: borrowed.max,
        })
    }

    fn data(&self) -> &[f64] {
        self.buffer
            .slice(&self.range)
            .expect("an owned f64 column keeps its validated range")
    }

    fn borrowed(&self) -> BorrowedF64Column<'_> {
        BorrowedF64Column {
            data: self.data(),
            min: self.min,
            max: self.max,
        }
    }
}

impl ColumnSource for OwnedF64Column {
    fn len(&self) -> usize {
        self.range.len()
    }

    fn min(&self) -> f64 {
        self.min
    }

    fn max(&self) -> f64 {
        self.max
    }

    fn write_f32_le_into(&self, dst: &mut [u8]) {
        ColumnSource::write_f32_le_into(&self.borrowed(), dst);
    }

    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        HiLoColumnSource::write_f32_pair_le_into(&self.borrowed(), dst);
    }

    fn write_f32_pair_le_into_with_stats(&self, dst: ColumnPairWriter<'_>) -> ColumnUploadStats {
        HiLoColumnSource::write_f32_pair_le_into_with_stats(&self.borrowed(), dst)
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        HiLoColumnSource::write_f32_pair_range_into_with_stats(&self.borrowed(), start, dst)
    }
}

impl HiLoColumnSource for OwnedF64Column {
    fn len(&self) -> usize {
        self.range.len()
    }

    fn min(&self) -> f64 {
        self.min
    }

    fn max(&self) -> f64 {
        self.max
    }

    fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
        HiLoColumnSource::write_f32_pair_le_into(&self.borrowed(), dst);
    }

    fn write_f32_pair_le_into_with_stats(&self, dst: ColumnPairWriter<'_>) -> ColumnUploadStats {
        HiLoColumnSource::write_f32_pair_le_into_with_stats(&self.borrowed(), dst)
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        HiLoColumnSource::write_f32_pair_range_into_with_stats(&self.borrowed(), start, dst)
    }
}

/// Borrowed `f32` column with upload-time scalar statistics.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BorrowedF32Column<'a> {
    data: &'a [f32],
    min: f32,
    max: f32,
}

impl<'a> BorrowedF32Column<'a> {
    pub(crate) fn new(data: &'a [f32]) -> Self {
        let (mut min, mut max) = (f32::INFINITY, f32::NEG_INFINITY);
        for &value in data {
            if value < min {
                min = value;
            }
            if value > max {
                max = value;
            }
        }
        Self { data, min, max }
    }
}

impl ColumnSource for BorrowedF32Column<'_> {
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
        debug_assert_eq!(dst.len(), std::mem::size_of_val(self.data));
        for (bytes, &value) in dst.chunks_exact_mut(size_of::<f32>()).zip(self.data) {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
    }

    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * COLUMN_VALUE_BYTES);
        for (pair, &value) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).zip(self.data) {
            write_pair_bytes(pair, value, 0.0);
        }
    }

    fn write_f32_pair_le_into_with_stats(&self, dst: ColumnPairWriter<'_>) -> ColumnUploadStats {
        HiLoColumnSource::write_f32_pair_le_into_with_stats(self, dst)
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        let start = usize::try_from(start).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
        let end = start
            .checked_add(dst.len())
            .filter(|end| *end <= self.data.len())
            .ok_or(ColumnRangeWriteError::InvalidRange)?;
        let mut bounds = StreamBoundsBuilder::default();
        for (index, &value) in self.data[start..end].iter().enumerate() {
            dst.write_pair(index, value, 0.0);
            bounds.record(value, 0.0);
        }
        Ok(bounds.finish())
    }
}

impl HiLoColumnSource for BorrowedF32Column<'_> {
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
        ColumnSource::write_f32_zero_lo_pair_le_into(self, dst);
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
}

/// Borrowed f64 input uploaded with the legacy per-value f32 cast semantics.
///
/// This exists for internally generated f64 demo data: it avoids constructing
/// a second f32 value vector while preserving the exact bytes previously
/// produced by converting each f64 value to f32.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BorrowedCastF32Column<'a> {
    data: &'a [f64],
    min: f32,
    max: f32,
}

impl<'a> BorrowedCastF32Column<'a> {
    pub(crate) fn new(data: &'a [f64]) -> Self {
        let (mut min, mut max) = (f32::INFINITY, f32::NEG_INFINITY);
        for &value in data {
            let value = value as f32;
            if value < min {
                min = value;
            }
            if value > max {
                max = value;
            }
        }
        Self { data, min, max }
    }
}

impl ColumnSource for BorrowedCastF32Column<'_> {
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
        debug_assert_eq!(dst.len(), self.data.len() * size_of::<f32>());
        for (bytes, &value) in dst.chunks_exact_mut(size_of::<f32>()).zip(self.data) {
            bytes.copy_from_slice(&(value as f32).to_le_bytes());
        }
    }

    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * COLUMN_VALUE_BYTES);
        for (pair, &value) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).zip(self.data) {
            write_pair_bytes(pair, value as f32, 0.0);
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
}

/// Borrowed f64 column with upload-time scalar statistics.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BorrowedF64Column<'a> {
    data: &'a [f64],
    min: f64,
    max: f64,
}

impl<'a> BorrowedF64Column<'a> {
    pub(crate) fn new(data: &'a [f64]) -> Self {
        let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
        for &value in data {
            if value < min {
                min = value;
            }
            if value > max {
                max = value;
            }
        }
        Self { data, min, max }
    }
}

impl ColumnSource for BorrowedF64Column<'_> {
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
        debug_assert_eq!(dst.len(), self.data.len() * size_of::<f32>());
        for (bytes, &value) in dst.chunks_exact_mut(size_of::<f32>()).zip(self.data) {
            bytes.copy_from_slice(&(value as f32).to_le_bytes());
        }
    }

    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.data.len() * COLUMN_VALUE_BYTES);
        for (pair, &value) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).zip(self.data) {
            write_pair_bytes(pair, value as f32, 0.0);
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
}

impl HiLoColumnSource for BorrowedF64Column<'_> {
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
        for (pair, &value) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).zip(self.data) {
            let (hi, lo) = split_f64_to_f32_pair(value);
            write_pair_bytes(pair, hi, lo);
        }
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        debug_assert_eq!(dst.len(), self.data.len());
        let mut stats = ColumnUploadStats { min_positive: None };
        for (index, &value) in self.data.iter().enumerate() {
            let (hi, lo) = split_f64_to_f32_pair(value);
            dst.write_pair(index, hi, lo);
            record_min_positive(&mut stats, hi as f64 + lo as f64);
        }
        stats
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        let start = usize::try_from(start).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
        let end = start
            .checked_add(dst.len())
            .filter(|end| *end <= self.data.len())
            .ok_or(ColumnRangeWriteError::InvalidRange)?;
        let mut bounds = StreamBoundsBuilder::default();
        for (index, &value) in self.data[start..end].iter().enumerate() {
            let (hi, lo) = split_f64_to_f32_pair(value);
            dst.write_pair(index, hi, lo);
            bounds.record(hi, lo);
        }
        Ok(bounds.finish())
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) enum JsStreamColumn {
    F32(JsF32StreamSource),
    F64(JsF64StreamSource),
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct JsF32StreamSource(js_sys::Float32Array);

#[cfg(target_arch = "wasm32")]
pub(crate) struct JsF64StreamSource(js_sys::Float64Array);

#[cfg(target_arch = "wasm32")]
impl JsStreamColumn {
    pub(crate) fn from_js(value: &wasm_bindgen::JsValue) -> Result<Self, &'static str> {
        use wasm_bindgen::JsCast;

        if value.is_instance_of::<js_sys::Float32Array>() {
            return Ok(Self::F32(JsF32StreamSource(
                value.clone().unchecked_into(),
            )));
        }
        if value.is_instance_of::<js_sys::Float64Array>() {
            return Ok(Self::F64(JsF64StreamSource(
                value.clone().unchecked_into(),
            )));
        }
        Err("stream source must be a Float32Array or Float64Array")
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::F32(values) => values.0.length() as usize,
            Self::F64(values) => values.0.length() as usize,
        }
    }

    pub(crate) fn encoding(&self) -> renderer::StreamEncoding {
        match self {
            Self::F32(_) => renderer::StreamEncoding::ScalarF32,
            Self::F64(_) => renderer::StreamEncoding::HiLoF32,
        }
    }

    pub(crate) fn source(&self) -> renderer::StreamColumnSource<'_> {
        match self {
            Self::F32(values) => renderer::StreamColumnSource::Scalar(values),
            Self::F64(values) => renderer::StreamColumnSource::HiLo(values),
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn js_f32_extreme(values: &JsF32StreamSource, min: bool) -> f64 {
    let mut result = if min { f32::INFINITY } else { f32::NEG_INFINITY };
    for index in 0..values.0.length() {
        let value = values.0.get_index(index);
        result = if min {
            result.min(value)
        } else {
            result.max(value)
        };
    }
    result as f64
}

#[cfg(target_arch = "wasm32")]
impl ColumnSource for JsF32StreamSource {
    fn len(&self) -> usize {
        self.0.length() as usize
    }

    fn min(&self) -> f64 {
        js_f32_extreme(self, true)
    }

    fn max(&self) -> f64 {
        js_f32_extreme(self, false)
    }

    fn write_f32_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.0.length() as usize * size_of::<f32>());
        for (index, bytes) in dst.chunks_exact_mut(size_of::<f32>()).enumerate() {
            bytes.copy_from_slice(&self.0.get_index(index as u32).to_le_bytes());
        }
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        let mut stats = ColumnUploadStats { min_positive: None };
        for index in 0..dst.len() {
            let value = self.0.get_index(index as u32);
            dst.write_pair(index, value, 0.0);
            record_min_positive(&mut stats, value as f64);
        }
        stats
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        let start = u32::try_from(start).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
        let len = u32::try_from(dst.len()).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
        let end = start
            .checked_add(len)
            .filter(|end| *end <= self.0.length())
            .ok_or(ColumnRangeWriteError::InvalidRange)?;
        let mut bounds = StreamBoundsBuilder::default();
        for (target, source) in (start..end).enumerate() {
            let value = self.0.get_index(source);
            dst.write_pair(target, value, 0.0);
            bounds.record(value, 0.0);
        }
        Ok(bounds.finish())
    }
}

#[cfg(target_arch = "wasm32")]
fn js_f64_extreme(values: &JsF64StreamSource, min: bool) -> f64 {
    let mut result = if min { f64::INFINITY } else { f64::NEG_INFINITY };
    for index in 0..values.0.length() {
        let value = values.0.get_index(index);
        result = if min {
            result.min(value)
        } else {
            result.max(value)
        };
    }
    result
}

#[cfg(target_arch = "wasm32")]
impl HiLoColumnSource for JsF64StreamSource {
    fn len(&self) -> usize {
        self.0.length() as usize
    }

    fn min(&self) -> f64 {
        js_f64_extreme(self, true)
    }

    fn max(&self) -> f64 {
        js_f64_extreme(self, false)
    }

    fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.0.length() as usize * COLUMN_VALUE_BYTES);
        for (index, pair) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).enumerate() {
            let (hi, lo) = split_f64_to_f32_pair(self.0.get_index(index as u32));
            write_pair_bytes(pair, hi, lo);
        }
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        let mut stats = ColumnUploadStats { min_positive: None };
        for index in 0..dst.len() {
            let (hi, lo) = split_f64_to_f32_pair(self.0.get_index(index as u32));
            dst.write_pair(index, hi, lo);
            record_min_positive(&mut stats, hi as f64 + lo as f64);
        }
        stats
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        let start = u32::try_from(start).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
        let len = u32::try_from(dst.len()).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
        let end = start
            .checked_add(len)
            .filter(|end| *end <= self.0.length())
            .ok_or(ColumnRangeWriteError::InvalidRange)?;
        let mut bounds = StreamBoundsBuilder::default();
        for (target, source) in (start..end).enumerate() {
            let (hi, lo) = split_f64_to_f32_pair(self.0.get_index(source));
            dst.write_pair(target, hi, lo);
            bounds.record(hi, lo);
        }
        Ok(bounds.finish())
    }
}

/// Borrowed f64 column whose **`ColumnSource`** impl writes the split `(hi, lo)`
/// pair — the batch upload's f64 adapter.
///
/// [`BorrowedF64Column`] already splits, but only through `HiLoColumnSource`; its
/// `ColumnSource` impl deliberately keeps the single-column scalar API's cast
/// semantics, and a test fixes that contract. The pool's *batch* path
/// (`begin_add_columns`) accepts only `&dyn ColumnSource`, so a batch of f64
/// columns going through `BorrowedF64Column` would silently drop the low half and
/// lose precision the single-column API preserves.
///
/// Nothing in the trait mandates `lo == 0`: [`BorrowedF32Column`] already routes
/// its `ColumnSource` pair writer to its `HiLoColumnSource` impl. This type does
/// the same thing for f64 — one behaviour, both traits.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BorrowedSplitF64Column<'a> {
    inner: BorrowedF64Column<'a>,
}

impl<'a> BorrowedSplitF64Column<'a> {
    pub(crate) fn new(data: &'a [f64]) -> Self {
        Self {
            inner: BorrowedF64Column::new(data),
        }
    }
}

impl ColumnSource for BorrowedSplitF64Column<'_> {
    fn len(&self) -> usize {
        HiLoColumnSource::len(&self.inner)
    }

    fn min(&self) -> f64 {
        HiLoColumnSource::min(&self.inner)
    }

    fn max(&self) -> f64 {
        HiLoColumnSource::max(&self.inner)
    }

    fn write_f32_le_into(&self, dst: &mut [u8]) {
        // The f32-only form cannot carry a low half by definition; this is the
        // same cast the single-column scalar path performs.
        ColumnSource::write_f32_le_into(&self.inner, dst);
    }

    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        HiLoColumnSource::write_f32_pair_le_into(&self.inner, dst);
    }

    fn write_f32_pair_le_into_with_stats(&self, dst: ColumnPairWriter<'_>) -> ColumnUploadStats {
        HiLoColumnSource::write_f32_pair_le_into_with_stats(&self.inner, dst)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use renderer::data_render::column_pool::ALIGN;
    use renderer::data_render::{create_instance, request_adapter, request_device};
    use renderer::{AllocError, ColumnHandle, ColumnPool};
    use wgpu::{BufferDescriptor, BufferUsages};

    fn shared_device() -> Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
        static DEVICE: OnceLock<Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)>> = OnceLock::new();
        DEVICE
            .get_or_init(|| {
                let instance = create_instance();
                let adapter = request_adapter(&instance).ok()?;
                let (device, queue) = request_device(&adapter).ok()?;
                Some((Arc::new(device), Arc::new(queue)))
            })
            .as_ref()
            .map(|(device, queue)| (Arc::clone(device), Arc::clone(queue)))
    }

    fn read_uploaded_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool: &ColumnPool,
        handle: ColumnHandle,
    ) -> Vec<u8> {
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("web borrowed column test readback"),
            size: handle.byte_size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("web borrowed column test encoder"),
        });
        encoder.copy_buffer_to_buffer(pool.buffer(), handle.offset, &readback, 0, handle.byte_size);
        let (sender, receiver) = std::sync::mpsc::channel();
        encoder.map_buffer_on_submit(
            &readback,
            wgpu::MapMode::Read,
            0..handle.byte_size,
            move |result| {
                let _ = sender.send(result);
            },
        );
        let submission = queue.submit(std::iter::once(encoder.finish()));
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(30)),
            })
            .expect("borrowed column readback poll");
        receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("borrowed column readback callback")
            .expect("borrowed column readback map");
        let mapped = readback
            .slice(..handle.byte_size)
            .get_mapped_range()
            .expect("borrowed column readback is mapped");
        let bytes = mapped[..handle.len_values * COLUMN_VALUE_BYTES].to_vec();
        drop(mapped);
        readback.unmap();
        bytes
    }

    fn scalar_bytes<T: ColumnSource>(source: &T) -> Vec<u8> {
        let mut bytes = vec![0; source.len() * size_of::<f32>()];
        source.write_f32_le_into(&mut bytes);
        bytes
    }

    fn scalar_pair_bytes<T: ColumnSource>(source: &T) -> Vec<u8> {
        let mut bytes = vec![0; source.len() * COLUMN_VALUE_BYTES];
        source.write_f32_zero_lo_pair_le_into(&mut bytes);
        bytes
    }

    fn pair_bytes<T: HiLoColumnSource>(source: &T) -> Vec<u8> {
        let mut bytes = vec![0; source.len() * COLUMN_VALUE_BYTES];
        source.write_f32_pair_le_into(&mut bytes);
        bytes
    }

    fn scalar_pair_upload<T: ColumnSource>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &T,
    ) -> (Vec<u8>, Option<f64>) {
        let mut pool =
            ColumnPool::new(renderer::GpuAllocCtx::unbudgeted(device, queue), ALIGN).unwrap();
        let handle = pool
            .add_column(
                "scalar".into(),
                source,
                renderer::GpuAllocCtx::unbudgeted(device, queue),
            )
            .unwrap();
        let min_positive = pool.slot("scalar").unwrap().min_positive;
        (
            read_uploaded_bytes(device, queue, &pool, handle),
            min_positive,
        )
    }

    fn hilo_pair_upload<T: HiLoColumnSource>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &T,
    ) -> (Vec<u8>, Option<f64>) {
        let mut pool =
            ColumnPool::new(renderer::GpuAllocCtx::unbudgeted(device, queue), ALIGN).unwrap();
        let handle = pool
            .add_hilo_column(
                "hilo".into(),
                source,
                renderer::GpuAllocCtx::unbudgeted(device, queue),
            )
            .unwrap();
        let min_positive = pool.slot("hilo").unwrap().min_positive;
        (
            read_uploaded_bytes(device, queue, &pool, handle),
            min_positive,
        )
    }

    #[test]
    fn owned_f32_views_share_one_boundary_allocation_and_preserve_ranges() {
        let boxed = vec![-10.0f32, 1.25, 2.5, 3.75, 20.0].into_boxed_slice();
        let payload = boxed.as_ptr();
        let buffer = OwnedColumnBuffer::new(boxed);
        assert_eq!(buffer.data.as_ptr(), payload);
        assert!(!buffer.is_empty());

        let left = OwnedF32Column::from_buffer(&buffer, 1..3).unwrap();
        let right = OwnedF32Column::from_buffer(&buffer, 3..5).unwrap();
        assert!(left.buffer.shares_allocation_with(&right.buffer));
        assert_eq!(left.data().as_ptr(), unsafe { payload.add(1) });
        assert_eq!(right.data().as_ptr(), unsafe { payload.add(3) });
        assert_eq!(ColumnSource::min(&left), 1.25);
        assert_eq!(ColumnSource::max(&left), 2.5);

        let expected_left: Vec<u8> = [1.25f32, 2.5]
            .iter()
            .flat_map(|value| [value.to_le_bytes(), 0.0f32.to_le_bytes()].concat())
            .collect();
        assert_eq!(scalar_pair_bytes(&left), expected_left);

        drop(buffer);
        assert_eq!(right.data(), &[3.75, 20.0]);
        assert_eq!(ColumnSource::min(&right), 3.75);
        assert_eq!(ColumnSource::max(&right), 20.0);
    }

    #[test]
    fn owned_f64_views_use_split_pairs_through_both_traits() {
        let epoch = 1_700_000_000_000.0;
        let boxed = vec![-5.0f64, epoch + 0.125, epoch + 0.875, 9.0].into_boxed_slice();
        let payload = boxed.as_ptr();
        let buffer = OwnedColumnBuffer::new(boxed);
        let source = OwnedF64Column::from_buffer(&buffer, 1..3).unwrap();
        assert_eq!(source.data().as_ptr(), unsafe { payload.add(1) });

        let expected: Vec<u8> = [epoch + 0.125, epoch + 0.875]
            .iter()
            .flat_map(|&value| {
                let (hi, lo) = split_f64_to_f32_pair(value);
                [hi.to_le_bytes(), lo.to_le_bytes()].concat()
            })
            .collect();
        assert_eq!(scalar_pair_bytes(&source), expected);
        assert_eq!(pair_bytes(&source), expected);
        assert_ne!(
            expected,
            [epoch + 0.125, epoch + 0.875]
                .iter()
                .flat_map(|&value| [(value as f32).to_le_bytes(), 0.0f32.to_le_bytes()].concat())
                .collect::<Vec<_>>()
        );
        assert_eq!(ColumnSource::min(&source), epoch + 0.125);
        assert_eq!(ColumnSource::max(&source), epoch + 0.875);
    }

    #[test]
    fn owned_columns_reject_out_of_bounds_views_and_accept_whole_boxes() {
        let f32_buffer = OwnedColumnBuffer::new(vec![1.0f32, 2.0].into_boxed_slice());
        assert!(matches!(
            OwnedF32Column::from_buffer(&f32_buffer, 1..3),
            Err(ColumnRangeWriteError::InvalidRange)
        ));
        let f64_buffer = OwnedColumnBuffer::new(vec![1.0f64, 2.0].into_boxed_slice());
        assert!(matches!(
            OwnedF64Column::from_buffer(&f64_buffer, 2..1),
            Err(ColumnRangeWriteError::InvalidRange)
        ));

        let f32_source = OwnedF32Column::new(vec![3.0f32, 4.0].into_boxed_slice());
        let f64_source = OwnedF64Column::new(vec![5.0f64, 6.0].into_boxed_slice());
        assert_eq!(ColumnSource::len(&f32_source), 2);
        assert_eq!(HiLoColumnSource::len(&f64_source), 2);
    }

    #[test]
    fn borrowed_f32_preserves_stats_and_bits() {
        let values = [
            f32::from_bits(0x7fc0_0042),
            -0.0,
            4.5,
            -2.25,
            f32::from_bits(0xffc0_0011),
        ];
        let source = BorrowedF32Column::new(&values);

        assert_eq!(source.data.as_ptr(), values.as_ptr());
        assert_eq!(ColumnSource::min(&source), -2.25);
        assert_eq!(ColumnSource::max(&source), 4.5);

        let expected: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        assert_eq!(scalar_bytes(&source), expected);

        let expected_pairs: Vec<u8> = values
            .iter()
            .flat_map(|value| [value.to_le_bytes(), 0.0f32.to_le_bytes()].concat())
            .collect();
        assert_eq!(scalar_pair_bytes(&source), expected_pairs);
        assert_eq!(pair_bytes(&source), expected_pairs);
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed fused upload assertions");
            return;
        };
        let (scalar_pairs, scalar_min_positive) = scalar_pair_upload(&device, &queue, &source);
        assert_eq!(scalar_pairs, expected_pairs);
        assert_eq!(scalar_min_positive, Some(4.5));
        let (hilo_pairs, hilo_min_positive) = hilo_pair_upload(&device, &queue, &source);
        assert_eq!(hilo_pairs, expected_pairs);
        assert_eq!(hilo_min_positive, Some(4.5));
    }

    #[test]
    fn borrowed_f64_preserves_stats_scalar_cast_and_split_pairs() {
        let values = [
            f64::from_bits(0x7ff8_0000_0000_0042),
            -0.0,
            1_700_000_000_000.125,
            1_700_000_000_000.875,
            -8.5,
        ];
        let source = BorrowedF64Column::new(&values);

        assert_eq!(source.data.as_ptr(), values.as_ptr());
        assert_eq!(ColumnSource::min(&source), -8.5);
        assert_eq!(ColumnSource::max(&source), 1_700_000_000_000.875);

        let expected_scalar: Vec<u8> = values
            .iter()
            .flat_map(|&value| (value as f32).to_le_bytes())
            .collect();
        assert_eq!(scalar_bytes(&source), expected_scalar);
        let expected_scalar_pairs: Vec<u8> = values
            .iter()
            .flat_map(|&value| [(value as f32).to_le_bytes(), 0.0f32.to_le_bytes()].concat())
            .collect();
        assert_eq!(scalar_pair_bytes(&source), expected_scalar_pairs);

        let expected_pairs: Vec<u8> = values
            .iter()
            .flat_map(|&value| {
                let (hi, lo) = split_f64_to_f32_pair(value);
                [hi.to_le_bytes(), lo.to_le_bytes()].concat()
            })
            .collect();
        assert_eq!(pair_bytes(&source), expected_pairs);
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed fused upload assertions");
            return;
        };
        let (scalar_pairs, scalar_min_positive) = scalar_pair_upload(&device, &queue, &source);
        assert_eq!(scalar_pairs, expected_scalar_pairs);
        assert_eq!(
            scalar_min_positive,
            Some((1_700_000_000_000.125_f64 as f32) as f64)
        );
        let (hilo_pairs, hilo_min_positive) = hilo_pair_upload(&device, &queue, &source);
        assert_eq!(hilo_pairs, expected_pairs);
        let (hi, lo) = split_f64_to_f32_pair(1_700_000_000_000.125);
        assert_eq!(hilo_min_positive, Some(hi as f64 + lo as f64));
    }

    #[test]
    fn borrowed_f64_cast_source_matches_the_previous_f32_vector() {
        let values = [
            f64::from_bits(0x7ff8_0000_0000_0042),
            -0.0,
            1_700_000_000_000.125,
            1_700_000_000_000.875,
            -8.5,
        ];
        let expected = values.map(|value| value as f32);
        let source = BorrowedCastF32Column::new(&values);

        assert_eq!(source.data.as_ptr(), values.as_ptr());
        assert_eq!(
            ColumnSource::min(&source),
            expected.iter().copied().fold(f32::INFINITY, f32::min) as f64
        );
        assert_eq!(
            ColumnSource::max(&source),
            expected.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64
        );
        assert_eq!(
            scalar_bytes(&source),
            expected
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>()
        );
        let expected_pairs: Vec<u8> = expected
            .iter()
            .flat_map(|value| [value.to_le_bytes(), 0.0f32.to_le_bytes()].concat())
            .collect();
        assert_eq!(scalar_pair_bytes(&source), expected_pairs);
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed fused upload assertions");
            return;
        };
        let (pairs, min_positive) = scalar_pair_upload(&device, &queue, &source);
        assert_eq!(pairs, expected_pairs);
        assert_eq!(
            min_positive,
            Some((1_700_000_000_000.125_f64 as f32) as f64)
        );
    }

    #[test]
    fn empty_sources_have_no_positive_upload_stat() {
        let empty_f32 = BorrowedF32Column::new(&[]);
        assert_eq!(ColumnSource::min(&empty_f32), f64::INFINITY);
        assert_eq!(ColumnSource::max(&empty_f32), f64::NEG_INFINITY);

        let empty_f64 = BorrowedF64Column::new(&[]);
        assert_eq!(ColumnSource::min(&empty_f64), f64::INFINITY);
        assert_eq!(ColumnSource::max(&empty_f64), f64::NEG_INFINITY);

        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed fused upload assertions");
            return;
        };
        let mut pool =
            ColumnPool::new(renderer::GpuAllocCtx::unbudgeted(&device, &queue), ALIGN).unwrap();
        assert_eq!(
            pool.add_column(
                "f32-scalar".into(),
                &empty_f32,
                renderer::GpuAllocCtx::unbudgeted(&device, &queue)
            )
            .unwrap_err(),
            AllocError::EmptySource
        );
        assert_eq!(
            pool.add_hilo_column(
                "f32-hilo".into(),
                &empty_f32,
                renderer::GpuAllocCtx::unbudgeted(&device, &queue)
            )
            .unwrap_err(),
            AllocError::EmptySource
        );
        assert_eq!(
            pool.add_column(
                "f64-scalar".into(),
                &empty_f64,
                renderer::GpuAllocCtx::unbudgeted(&device, &queue)
            )
            .unwrap_err(),
            AllocError::EmptySource
        );
        assert_eq!(
            pool.add_hilo_column(
                "f64-hilo".into(),
                &empty_f64,
                renderer::GpuAllocCtx::unbudgeted(&device, &queue)
            )
            .unwrap_err(),
            AllocError::EmptySource
        );
    }

    #[test]
    fn borrowed_stats_filter_non_finite_zero_and_cast_extremes() {
        let min_subnormal = f32::from_bits(1);
        let f32_values = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.0,
            -0.0,
            -min_subnormal,
            min_subnormal,
        ];
        let f32_source = BorrowedF32Column::new(&f32_values);
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed fused upload assertions");
            return;
        };
        let expected_f32_pairs: Vec<u8> = f32_values
            .iter()
            .flat_map(|value| [value.to_le_bytes(), 0.0f32.to_le_bytes()].concat())
            .collect();
        let (f32_pairs, f32_min_positive) = scalar_pair_upload(&device, &queue, &f32_source);
        assert_eq!(f32_pairs, expected_f32_pairs);
        assert_eq!(f32_min_positive, Some(min_subnormal as f64));

        let f64_values = [
            f64::from_bits(1),
            f64::MAX,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -0.0,
            min_subnormal as f64,
        ];
        let cast_source = BorrowedCastF32Column::new(&f64_values);
        let expected_cast_pairs: Vec<u8> = f64_values
            .iter()
            .flat_map(|&value| [(value as f32).to_le_bytes(), 0.0f32.to_le_bytes()].concat())
            .collect();
        let (cast_pairs, cast_min_positive) = scalar_pair_upload(&device, &queue, &cast_source);
        assert_eq!(cast_pairs, expected_cast_pairs);
        assert_eq!(cast_min_positive, Some(min_subnormal as f64));

        let hilo_source = BorrowedF64Column::new(&f64_values);
        let expected_hilo_pairs: Vec<u8> = f64_values
            .iter()
            .flat_map(|&value| {
                let (hi, lo) = split_f64_to_f32_pair(value);
                [hi.to_le_bytes(), lo.to_le_bytes()].concat()
            })
            .collect();
        let (hilo_pairs, hilo_min_positive) = hilo_pair_upload(&device, &queue, &hilo_source);
        assert_eq!(hilo_pairs, expected_hilo_pairs);
        assert_eq!(hilo_min_positive, Some(min_subnormal as f64));
    }

    /// The batch adapter uploads through the **scalar** path and still lands the
    /// same bytes the hi/lo path does.
    ///
    /// This is the precision guarantee of the wasm batch API (design B.8): the
    /// pool's batch entry takes `&dyn ColumnSource`, so an f64 batch can only keep
    /// its low half if the `ColumnSource` impl writes one.
    #[test]
    fn split_f64_scalar_upload_matches_the_hilo_upload() {
        let values = [
            1_700_000_000_000.125,
            1_700_000_000_000.875,
            -8.5,
            0.0,
            f64::from_bits(0x7ff8_0000_0000_0042),
            2.5e-8,
        ];
        let split = BorrowedSplitF64Column::new(&values);
        let hilo = BorrowedF64Column::new(&values);

        // Same statistics, so the pool records the same slot metadata.
        assert_eq!(ColumnSource::len(&split), HiLoColumnSource::len(&hilo));
        assert_eq!(ColumnSource::min(&split), HiLoColumnSource::min(&hilo));
        assert_eq!(ColumnSource::max(&split), HiLoColumnSource::max(&hilo));
        assert_eq!(scalar_pair_bytes(&split), pair_bytes(&hilo));

        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping split f64 fused upload assertions");
            return;
        };
        let (scalar_pairs, scalar_min_positive) = scalar_pair_upload(&device, &queue, &split);
        let (hilo_pairs, hilo_min_positive) = hilo_pair_upload(&device, &queue, &hilo);
        assert_eq!(scalar_pairs, hilo_pairs);
        assert_eq!(scalar_min_positive, hilo_min_positive);

        // And it is genuinely different from the casting impl — otherwise the test
        // above would pass with the low half thrown away.
        let cast = BorrowedF64Column::new(&values);
        assert_ne!(scalar_pair_bytes(&split), scalar_pair_bytes(&cast));
    }
}

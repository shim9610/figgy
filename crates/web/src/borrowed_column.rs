//! Borrowed adapters for streaming web column inputs into renderer staging
//! buffers without constructing an owned per-value mirror.

use renderer::data::{COLUMN_VALUE_BYTES, split_f64_to_f32_pair};
use renderer::{ColumnSource, HiLoColumnSource};

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
        HiLoColumnSource::write_f32_pair_le_into(self, dst);
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
        debug_assert_eq!(dst.len(), self.data.len() * COLUMN_VALUE_BYTES);
        for (pair, &hi) in dst.chunks_exact_mut(COLUMN_VALUE_BYTES).zip(self.data) {
            pair[..4].copy_from_slice(&hi.to_le_bytes());
            pair[4..].copy_from_slice(&0.0f32.to_le_bytes());
        }
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
            pair[..4].copy_from_slice(&(value as f32).to_le_bytes());
            pair[4..].fill(0);
        }
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
            pair[..4].copy_from_slice(&(value as f32).to_le_bytes());
            pair[4..].fill(0);
        }
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
            pair[..4].copy_from_slice(&hi.to_le_bytes());
            pair[4..].copy_from_slice(&lo.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn empty_and_zero_sources_match_existing_scan_semantics() {
        let empty_f32 = BorrowedF32Column::new(&[]);
        assert_eq!(ColumnSource::min(&empty_f32), f64::INFINITY);
        assert_eq!(ColumnSource::max(&empty_f32), f64::NEG_INFINITY);

        let empty_f64 = BorrowedF64Column::new(&[]);
        assert_eq!(ColumnSource::min(&empty_f64), f64::INFINITY);
        assert_eq!(ColumnSource::max(&empty_f64), f64::NEG_INFINITY);
    }
}

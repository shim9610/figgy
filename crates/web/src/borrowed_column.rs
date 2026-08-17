//! Borrowed adapters for streaming web column inputs into renderer staging
//! buffers without constructing an owned per-value mirror.

use renderer::data::{COLUMN_VALUE_BYTES, split_f64_to_f32_pair};
use renderer::{ColumnPairWriter, ColumnSource, ColumnUploadStats, HiLoColumnSource};

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
        let mut pool = ColumnPool::new(device, ALIGN).unwrap();
        let handle = pool
            .add_column("scalar".into(), source, device, queue)
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
        let mut pool = ColumnPool::new(device, ALIGN).unwrap();
        let handle = pool
            .add_hilo_column("hilo".into(), source, device, queue)
            .unwrap();
        let min_positive = pool.slot("hilo").unwrap().min_positive;
        (
            read_uploaded_bytes(device, queue, &pool, handle),
            min_positive,
        )
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
        let mut pool = ColumnPool::new(&device, ALIGN).unwrap();
        assert_eq!(
            pool.add_column("f32-scalar".into(), &empty_f32, &device, &queue)
                .unwrap_err(),
            AllocError::EmptySource
        );
        assert_eq!(
            pool.add_hilo_column("f32-hilo".into(), &empty_f32, &device, &queue)
                .unwrap_err(),
            AllocError::EmptySource
        );
        assert_eq!(
            pool.add_column("f64-scalar".into(), &empty_f64, &device, &queue)
                .unwrap_err(),
            AllocError::EmptySource
        );
        assert_eq!(
            pool.add_hilo_column("f64-hilo".into(), &empty_f64, &device, &queue)
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
}

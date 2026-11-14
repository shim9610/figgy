//! Borrowed adapters for streaming web column inputs into renderer staging
//! buffers without constructing an owned per-value mirror. Each adapter only
//! writes values; the renderer's writer records their statistics.

use renderer::{ColumnPairWriter, ColumnRangeWriteError, ColumnSource, HiLoColumnSource};

/// The slice range a streamed chunk of `len` values from `start` reads.
fn chunk_range(
    total: usize,
    start: u64,
    len: usize,
) -> Result<std::ops::Range<usize>, ColumnRangeWriteError> {
    let start = usize::try_from(start).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
    let end = start
        .checked_add(len)
        .filter(|end| *end <= total)
        .ok_or(ColumnRangeWriteError::InvalidRange)?;
    Ok(start..end)
}

/// Borrowed `f32` column.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BorrowedF32Column<'a> {
    data: &'a [f32],
}

impl<'a> BorrowedF32Column<'a> {
    pub(crate) fn new(data: &'a [f32]) -> Self {
        Self { data }
    }
}

impl ColumnSource for BorrowedF32Column<'_> {
    fn len(&self) -> usize {
        self.data.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        HiLoColumnSource::write_pairs(self, dst)
    }

    fn write_pair_range(
        &self,
        start: u64,
        dst: &mut ColumnPairWriter<'_>,
    ) -> Result<(), ColumnRangeWriteError> {
        let range = chunk_range(self.data.len(), start, dst.len())?;
        for (index, &value) in self.data[range].iter().enumerate() {
            dst.write_pair(index, value, 0.0);
        }
        Ok(())
    }
}

impl HiLoColumnSource for BorrowedF32Column<'_> {
    fn len(&self) -> usize {
        self.data.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        for (index, &hi) in self.data.iter().enumerate().take(dst.len()) {
            dst.write_pair(index, hi, 0.0);
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
}

impl<'a> BorrowedCastF32Column<'a> {
    pub(crate) fn new(data: &'a [f64]) -> Self {
        Self { data }
    }
}

impl ColumnSource for BorrowedCastF32Column<'_> {
    fn len(&self) -> usize {
        self.data.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        for (index, &value) in self.data.iter().enumerate().take(dst.len()) {
            dst.write_pair(index, value as f32, 0.0);
        }
    }
}

/// Borrowed `u32` column: a triangle index array (#210), passed from
/// JavaScript as a `Uint32Array`. Every value is written as the f64
/// `(hi, lo)` split, which holds every `u32` exactly, so the pool's `u32`
/// index lanes receive every index up to `u32::MAX` unchanged.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BorrowedU32Column<'a> {
    data: &'a [u32],
}

impl<'a> BorrowedU32Column<'a> {
    pub(crate) fn new(data: &'a [u32]) -> Self {
        Self { data }
    }
}

impl ColumnSource for BorrowedU32Column<'_> {
    fn len(&self) -> usize {
        self.data.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        for (index, &value) in self.data.iter().enumerate().take(dst.len()) {
            dst.write_value(index, f64::from(value));
        }
    }
}

/// Borrowed f64 column. Both traits write the `(hi, lo)` split (#107), so a
/// batch upload, which takes `&dyn ColumnSource`, keeps the same precision as
/// a single `HiLoColumnSource` upload.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BorrowedF64Column<'a> {
    data: &'a [f64],
}

impl<'a> BorrowedF64Column<'a> {
    pub(crate) fn new(data: &'a [f64]) -> Self {
        Self { data }
    }
}

impl ColumnSource for BorrowedF64Column<'_> {
    fn len(&self) -> usize {
        self.data.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        HiLoColumnSource::write_pairs(self, dst)
    }
}

impl HiLoColumnSource for BorrowedF64Column<'_> {
    fn len(&self) -> usize {
        self.data.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        for (index, &value) in self.data.iter().enumerate().take(dst.len()) {
            dst.write_value(index, value);
        }
    }

    fn write_pair_range(
        &self,
        start: u64,
        dst: &mut ColumnPairWriter<'_>,
    ) -> Result<(), ColumnRangeWriteError> {
        let range = chunk_range(self.data.len(), start, dst.len())?;
        for (index, &value) in self.data[range].iter().enumerate() {
            dst.write_value(index, value);
        }
        Ok(())
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
            return Ok(Self::F32(JsF32StreamSource(value.clone().unchecked_into())));
        }
        if value.is_instance_of::<js_sys::Float64Array>() {
            return Ok(Self::F64(JsF64StreamSource(value.clone().unchecked_into())));
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
    /// The typed array as a batch upload source: a `Float64Array` keeps its
    /// `(hi, lo)` split, as [`BorrowedF64Column`] does for slices.
    pub(crate) fn column_source(&self) -> &dyn ColumnSource {
        match self {
            Self::F32(values) => values,
            Self::F64(values) => values,
        }
    }
}

/// The typed array range a streamed chunk of `len` values from `start` reads.
#[cfg(target_arch = "wasm32")]
fn js_chunk_range(
    total: u32,
    start: u64,
    len: usize,
) -> Result<std::ops::Range<u32>, ColumnRangeWriteError> {
    let start = u32::try_from(start).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
    let len = u32::try_from(len).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
    let end = start
        .checked_add(len)
        .filter(|end| *end <= total)
        .ok_or(ColumnRangeWriteError::InvalidRange)?;
    Ok(start..end)
}

#[cfg(target_arch = "wasm32")]
impl ColumnSource for JsF32StreamSource {
    fn len(&self) -> usize {
        self.0.length() as usize
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        for index in 0..dst.len() {
            dst.write_pair(index, self.0.get_index(index as u32), 0.0);
        }
    }

    fn write_pair_range(
        &self,
        start: u64,
        dst: &mut ColumnPairWriter<'_>,
    ) -> Result<(), ColumnRangeWriteError> {
        let range = js_chunk_range(self.0.length(), start, dst.len())?;
        for (target, source) in range.enumerate() {
            dst.write_pair(target, self.0.get_index(source), 0.0);
        }
        Ok(())
    }
}

/// The batch path accepts only `&dyn ColumnSource`, so the typed array writes
/// its split pairs through this trait too (record fields are batch uploads).
#[cfg(target_arch = "wasm32")]
impl ColumnSource for JsF64StreamSource {
    fn len(&self) -> usize {
        HiLoColumnSource::len(self)
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        HiLoColumnSource::write_pairs(self, dst)
    }
}

#[cfg(target_arch = "wasm32")]
impl HiLoColumnSource for JsF64StreamSource {
    fn len(&self) -> usize {
        self.0.length() as usize
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        for index in 0..dst.len() {
            dst.write_value(index, self.0.get_index(index as u32));
        }
    }

    fn write_pair_range(
        &self,
        start: u64,
        dst: &mut ColumnPairWriter<'_>,
    ) -> Result<(), ColumnRangeWriteError> {
        let range = js_chunk_range(self.0.length(), start, dst.len())?;
        for (target, source) in range.enumerate() {
            dst.write_value(target, self.0.get_index(source));
        }
        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use renderer::data::split_f64_to_f32_pair;
    use std::sync::OnceLock;
    use std::time::Duration;

    use renderer::data::COLUMN_VALUE_BYTES;
    use renderer::data_render::column_pool::ALIGN;
    use renderer::gpu::{create_instance, request_adapter_blocking, request_device_blocking};
    use renderer::{AllocError, ColumnHandle, ColumnPool};
    use wgpu::{BufferDescriptor, BufferUsages};

    fn shared_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        static DEVICE: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();
        DEVICE
            .get_or_init(|| {
                let instance = create_instance();
                let adapter = request_adapter_blocking(&instance).ok()?;
                let (device, queue) = request_device_blocking(&adapter).ok()?;
                Some((device, queue))
            })
            .as_ref()
            .map(|(device, queue)| (device.clone(), queue.clone()))
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

    /// What one upload left: the pool's bytes and the statistics its slot
    /// recorded while the values were written.
    struct Uploaded {
        bytes: Vec<u8>,
        min: f64,
        max: f64,
        min_positive: Option<f64>,
    }

    fn uploaded(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        add: impl FnOnce(&mut ColumnPool, renderer::GpuAllocCtx<'_>) -> ColumnHandle,
    ) -> Uploaded {
        let ledger = std::sync::Arc::new(renderer::gpu_memory::GpuLedger::new());
        let mut pool =
            ColumnPool::new(renderer::GpuAllocCtx::new(device, queue, &ledger), ALIGN).unwrap();
        let handle = add(
            &mut pool,
            renderer::GpuAllocCtx::new(device, queue, &ledger),
        );
        let slot = pool.slot("column").unwrap();
        let (min, max, min_positive) = (slot.min, slot.max, slot.min_positive);
        Uploaded {
            bytes: read_uploaded_bytes(device, queue, &pool, handle),
            min,
            max,
            min_positive,
        }
    }

    fn scalar_upload<T: ColumnSource>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &T,
    ) -> Uploaded {
        uploaded(device, queue, |pool, ctx| {
            pool.add_column("column".into(), source, ctx).unwrap()
        })
    }

    fn hilo_upload<T: HiLoColumnSource>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &T,
    ) -> Uploaded {
        uploaded(device, queue, |pool, ctx| {
            pool.add_hilo_column("column".into(), source, ctx).unwrap()
        })
    }

    fn pairs(values: impl Iterator<Item = (f32, f32)>) -> Vec<u8> {
        values
            .flat_map(|(hi, lo)| [hi.to_le_bytes(), lo.to_le_bytes()].concat())
            .collect()
    }

    #[test]
    fn borrowed_f32_writes_bits_and_the_pool_records_its_statistics() {
        let values = [
            f32::from_bits(0x7fc0_0042),
            -0.0,
            4.5,
            -2.25,
            f32::from_bits(0xffc0_0011),
        ];
        let source = BorrowedF32Column::new(&values);
        assert_eq!(source.data.as_ptr(), values.as_ptr());
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed upload assertions");
            return;
        };
        let expected = pairs(values.iter().map(|&value| (value, 0.0)));
        for upload in [
            scalar_upload(&device, &queue, &source),
            hilo_upload(&device, &queue, &source),
        ] {
            assert_eq!(upload.bytes, expected);
            assert_eq!((upload.min, upload.max), (-2.25, 4.5));
            assert_eq!(upload.min_positive, Some(4.5));
        }
    }

    #[test]
    fn borrowed_f64_writes_the_split_through_both_traits() {
        let values = [
            f64::from_bits(0x7ff8_0000_0000_0042),
            -0.0,
            1_700_000_000_000.125,
            1_700_000_000_000.875,
            -8.5,
        ];
        let source = BorrowedF64Column::new(&values);
        assert_eq!(source.data.as_ptr(), values.as_ptr());
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed upload assertions");
            return;
        };
        let expected = pairs(values.iter().map(|&value| split_f64_to_f32_pair(value)));
        // A batch upload takes `&dyn ColumnSource`: it keeps the low half the
        // hi/lo path keeps, and differs from a plain f32 cast.
        assert_ne!(
            expected,
            pairs(values.iter().map(|&value| (value as f32, 0.0)))
        );

        for upload in [
            scalar_upload(&device, &queue, &source),
            hilo_upload(&device, &queue, &source),
        ] {
            assert_eq!(upload.bytes, expected);
            // The statistics are the f64 values themselves.
            assert_eq!(upload.min, -8.5);
            assert_eq!(upload.max, 1_700_000_000_000.875);
            assert_eq!(upload.min_positive, Some(1_700_000_000_000.125));
        }
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
        let source = BorrowedCastF32Column::new(&values);
        assert_eq!(source.data.as_ptr(), values.as_ptr());
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed upload assertions");
            return;
        };
        let upload = scalar_upload(&device, &queue, &source);
        assert_eq!(
            upload.bytes,
            pairs(values.iter().map(|&value| (value as f32, 0.0)))
        );
        let cast = (1_700_000_000_000.125_f64 as f32) as f64;
        assert_eq!(upload.min_positive, Some(cast));
        assert_eq!(
            (upload.min, upload.max),
            (-8.5, (1_700_000_000_000.875_f64 as f32) as f64)
        );
    }

    #[test]
    fn empty_sources_are_refused_at_registration() {
        let ledger = std::sync::Arc::new(renderer::gpu_memory::GpuLedger::new());
        let empty_f32 = BorrowedF32Column::new(&[]);
        let empty_f64 = BorrowedF64Column::new(&[]);
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed upload assertions");
            return;
        };
        let mut pool =
            ColumnPool::new(renderer::GpuAllocCtx::new(&device, &queue, &ledger), ALIGN).unwrap();
        let ctx = || renderer::GpuAllocCtx::new(&device, &queue, &ledger);
        assert_eq!(
            pool.add_column("f32-scalar".into(), &empty_f32, ctx())
                .unwrap_err(),
            AllocError::EmptySource
        );
        assert_eq!(
            pool.add_hilo_column("f32-hilo".into(), &empty_f32, ctx())
                .unwrap_err(),
            AllocError::EmptySource
        );
        assert_eq!(
            pool.add_column("f64-scalar".into(), &empty_f64, ctx())
                .unwrap_err(),
            AllocError::EmptySource
        );
        assert_eq!(
            pool.add_hilo_column("f64-hilo".into(), &empty_f64, ctx())
                .unwrap_err(),
            AllocError::EmptySource
        );
    }

    #[test]
    fn recorded_statistics_leave_out_non_finite_values_and_follow_the_cast() {
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
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed upload assertions");
            return;
        };
        let upload = scalar_upload(&device, &queue, &BorrowedF32Column::new(&f32_values));
        assert_eq!(
            upload.bytes,
            pairs(f32_values.iter().map(|&value| (value, 0.0)))
        );
        assert_eq!(upload.min_positive, Some(min_subnormal as f64));
        assert_eq!(
            (upload.min, upload.max),
            (-(min_subnormal as f64), min_subnormal as f64)
        );

        // f64::from_bits(1) is 0 once cast, f64::MAX is infinite: the
        // statistics follow the values the GPU reads.
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
        let cast = scalar_upload(&device, &queue, &BorrowedCastF32Column::new(&f64_values));
        assert_eq!(
            cast.bytes,
            pairs(f64_values.iter().map(|&value| (value as f32, 0.0)))
        );
        assert_eq!(cast.min_positive, Some(min_subnormal as f64));
        assert_eq!((cast.min, cast.max), (0.0, min_subnormal as f64));

        let split = hilo_upload(&device, &queue, &BorrowedF64Column::new(&f64_values));
        assert_eq!(
            split.bytes,
            pairs(f64_values.iter().map(|&value| split_f64_to_f32_pair(value)))
        );
        assert_eq!(split.min_positive, Some(min_subnormal as f64));
    }

    /// A `Uint32Array` of triangle indices (#210) reaches the pool's `u32`
    /// index lanes exactly, past 2^24 and up to `u32::MAX`, and the pool
    /// records the triangle count and the index range.
    #[test]
    fn borrowed_u32_indices_land_exactly_in_index_lanes() {
        let indices: [u32; 9] = [
            0,
            1,
            2,
            16_777_216,
            16_777_217,
            16_777_219,
            4_000_000_001,
            u32::MAX - 1,
            u32::MAX,
        ];
        let source = BorrowedU32Column::new(&indices);
        assert_eq!(source.data.as_ptr(), indices.as_ptr());
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed upload assertions");
            return;
        };
        let ledger = std::sync::Arc::new(renderer::gpu_memory::GpuLedger::new());
        let mut pool =
            ColumnPool::new(renderer::GpuAllocCtx::new(&device, &queue, &ledger), ALIGN).unwrap();
        pool.upsert_triangles(
            "mesh",
            &renderer::TriangleSource { indices: &source },
            renderer::GpuAllocCtx::new(&device, &queue, &ledger),
        )
        .unwrap();
        let triangles = pool.triangles("mesh").unwrap();
        assert_eq!(
            triangles.stats(),
            renderer::TriangleStats {
                triangles: 3,
                min_index: 0,
                max_index: u32::MAX,
            }
        );
        let handle = triangles.indices().unwrap();
        let bytes = read_uploaded_bytes(&device, &queue, &pool, handle);
        let expected: Vec<u8> = indices
            .iter()
            .flat_map(|index| [index.to_le_bytes(), [0; 4]].concat())
            .collect();
        assert_eq!(bytes, expected);
    }

    /// PAIR_CHUNK_8 (#152): every adapter writes one `(hi, lo)` pair per value
    /// for 1 and 1000 values.
    #[test]
    fn every_adapter_writes_one_pair_per_value() {
        let Some((device, queue)) = shared_device() else {
            eprintln!("no GPU adapter; skipping borrowed upload assertions");
            return;
        };
        for count in [1usize, 1000] {
            let f64s: Vec<f64> = (0..count)
                .map(|i| (i as f64 + 0.1) * 1.000_000_1e3 - 7.0)
                .collect();
            let f32s: Vec<f32> = f64s.iter().map(|&v| v as f32).collect();
            let split = pairs(f64s.iter().map(|&v| split_f64_to_f32_pair(v)));
            let cast = pairs(f64s.iter().map(|&v| (v as f32, 0.0)));
            let narrow = pairs(f32s.iter().map(|&v| (v, 0.0)));

            let column = BorrowedF32Column::new(&f32s);
            assert_eq!(scalar_upload(&device, &queue, &column).bytes, narrow);
            assert_eq!(hilo_upload(&device, &queue, &column).bytes, narrow);
            let column = BorrowedCastF32Column::new(&f64s);
            assert_eq!(scalar_upload(&device, &queue, &column).bytes, cast);
            let column = BorrowedF64Column::new(&f64s);
            assert_eq!(scalar_upload(&device, &queue, &column).bytes, split);
            assert_eq!(hilo_upload(&device, &queue, &column).bytes, split);
        }
    }
}

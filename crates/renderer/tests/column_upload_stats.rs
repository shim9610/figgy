#[path = "support/gpu.rs"]
mod test_gpu;

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Duration;

use renderer::data::{COLUMN_VALUE_BYTES, split_f64_to_f32_pair};
use renderer::data_render::column_pool::ALIGN;
use renderer::{
    AllocError, Column, ColumnHandle, ColumnPairWriter, ColumnPool, ColumnSource, HiLoColumnSource,
};
use wgpu::{BufferDescriptor, BufferUsages};

fn scalar_upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &dyn ColumnSource,
) -> (Vec<(f32, f32)>, Option<f64>) {
    let ledger = std::sync::Arc::new(renderer::gpu_memory::GpuLedger::new());
    let mut pool =
        ColumnPool::new(renderer::GpuAllocCtx::new(device, queue, &ledger), ALIGN).unwrap();
    let handle = pool
        .add_column(
            "matrix".into(),
            source,
            renderer::GpuAllocCtx::new(device, queue, &ledger),
        )
        .unwrap();
    let min_positive = pool.slot("matrix").unwrap().min_positive;
    (read_pairs(device, queue, &pool, handle), min_positive)
}

fn hilo_upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &dyn HiLoColumnSource,
) -> (Vec<(f32, f32)>, Option<f64>) {
    let ledger = std::sync::Arc::new(renderer::gpu_memory::GpuLedger::new());
    let mut pool =
        ColumnPool::new(renderer::GpuAllocCtx::new(device, queue, &ledger), ALIGN).unwrap();
    let handle = pool
        .add_hilo_column(
            "matrix".into(),
            source,
            renderer::GpuAllocCtx::new(device, queue, &ledger),
        )
        .unwrap();
    let min_positive = pool.slot("matrix").unwrap().min_positive;
    (read_pairs(device, queue, &pool, handle), min_positive)
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

fn assert_pair_bits(actual: &[(f32, f32)], expected: &[(f32, f32)]) {
    assert_eq!(actual.len(), expected.len());
    for (&(actual_hi, actual_lo), &(expected_hi, expected_lo)) in actual.iter().zip(expected) {
        assert_eq!(actual_hi.to_bits(), expected_hi.to_bits());
        assert_eq!(actual_lo.to_bits(), expected_lo.to_bits());
    }
}

struct CustomScalar(Vec<f32>);

impl ColumnSource for CustomScalar {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        for (index, &value) in self.0.iter().enumerate() {
            dst.write_pair(index, value, 0.0);
        }
    }
}

/// A downstream type may implement both source traits; each `dyn` view of it
/// stays object safe.
impl HiLoColumnSource for CustomScalar {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        ColumnSource::write_pairs(self, dst)
    }
}

struct CustomHiLo(Vec<(f32, f32)>);

impl HiLoColumnSource for CustomHiLo {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        for (index, &(hi, lo)) in self.0.iter().enumerate() {
            dst.write_pair(index, hi, lo);
        }
    }
}

struct CountingScalar<'a> {
    values: &'a [f32],
    calls: Cell<usize>,
}

impl ColumnSource for CountingScalar<'_> {
    fn len(&self) -> usize {
        self.values.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(dst.len(), self.values.len());
        for (index, &value) in self.values.iter().enumerate() {
            dst.write_pair(index, value, 0.0);
        }
    }
}

struct CountingHiLo<'a> {
    values: &'a [(f32, f32)],
    calls: Cell<usize>,
}

impl HiLoColumnSource for CountingHiLo<'_> {
    fn len(&self) -> usize {
        self.values.len()
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(dst.len(), self.values.len());
        for (index, &(hi, lo)) in self.values.iter().enumerate() {
            dst.write_pair(index, hi, lo);
        }
    }
}

struct PanickingScalar {
    calls: Cell<usize>,
}

impl ColumnSource for PanickingScalar {
    fn len(&self) -> usize {
        1
    }

    fn write_pairs(&self, dst: &mut ColumnPairWriter<'_>) {
        self.calls.set(self.calls.get() + 1);
        dst.write_pair(0, 9.0, 0.0);
        panic!("injected fused writer panic");
    }
}

#[test]
fn required_fused_writer_keeps_source_traits_object_safe() {
    let both = CustomScalar(vec![3.0, 4.0]);
    let scalar: &dyn ColumnSource = &both;
    assert_eq!(scalar.len(), 2);
    assert!(!scalar.is_empty());
    let scalar_as_hilo: &dyn HiLoColumnSource = &both;
    assert_eq!(scalar_as_hilo.len(), 2);
    assert!(!scalar_as_hilo.is_empty());

    let hilo: &dyn HiLoColumnSource = &CustomHiLo(vec![(3.0, 0.25)]);
    assert_eq!(hilo.len(), 1);
    assert!(!hilo.is_empty());
}

#[test]
fn built_in_scalar_stats_use_actual_uploaded_f32_values() {
    let ledger = std::sync::Arc::new(renderer::gpu_memory::GpuLedger::new());
    let Some((device, queue)) = shared_device() else {
        eprintln!("no GPU adapter; skipping fused upload matrix");
        return;
    };
    let min_subnormal = f32::from_bits(1);
    let f32_column = Column {
        data: vec![
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.0,
            -0.0,
            -min_subnormal,
            min_subnormal,
        ],
    };
    let (pairs, min_positive) = scalar_upload(&device, &queue, &f32_column);
    let expected_pairs: Vec<_> = f32_column.data.iter().map(|&value| (value, 0.0)).collect();
    assert_pair_bits(&pairs, &expected_pairs);
    assert_eq!(min_positive, Some(min_subnormal as f64));

    let f64_column = Column {
        data: vec![
            f64::from_bits(1),
            f64::MAX,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -0.0,
            min_subnormal as f64,
        ],
    };
    let (pairs, min_positive) = scalar_upload(&device, &queue, &f64_column);
    let expected_pairs: Vec<_> = f64_column
        .data
        .iter()
        .map(|&value| (value as f32, 0.0))
        .collect();
    assert_pair_bits(&pairs, &expected_pairs);
    assert_eq!(min_positive, Some(min_subnormal as f64));

    let optional = Column {
        data: vec![None, Some(f64::NAN), Some(-0.0), Some(0.0)],
    };
    let (pairs, min_positive) = scalar_upload(&device, &queue, &optional);
    let expected_pairs: Vec<_> = optional
        .data
        .iter()
        .map(|value| (value.map(|value| value as f32).unwrap_or(f32::NAN), 0.0))
        .collect();
    assert_pair_bits(&pairs, &expected_pairs);
    assert_eq!(min_positive, None);

    let empty: Column<Option<f64>> = Column { data: Vec::new() };
    let empty_f64 = Column {
        data: Vec::<f64>::new(),
    };
    let mut empty_pool =
        ColumnPool::new(renderer::GpuAllocCtx::new(&device, &queue, &ledger), ALIGN).unwrap();
    assert_eq!(
        empty_pool
            .add_column(
                "empty-option".into(),
                &empty,
                renderer::GpuAllocCtx::new(&device, &queue, &ledger)
            )
            .unwrap_err(),
        AllocError::EmptySource
    );
    assert_eq!(
        empty_pool
            .add_column(
                "empty-f64".into(),
                &empty_f64,
                renderer::GpuAllocCtx::new(&device, &queue, &ledger)
            )
            .unwrap_err(),
        AllocError::EmptySource
    );
    assert_eq!(
        empty_pool
            .add_hilo_column(
                "empty-hilo".into(),
                &empty_f64,
                renderer::GpuAllocCtx::new(&device, &queue, &ledger)
            )
            .unwrap_err(),
        AllocError::EmptySource
    );
}

#[test]
fn built_in_hilo_stats_use_recorded_hi_plus_lo() {
    let Some((device, queue)) = shared_device() else {
        eprintln!("no GPU adapter; skipping fused upload matrix");
        return;
    };
    let cancellation = 16_777_215.5_f64;
    let epoch_a = 1_700_000_000_000.125_f64;
    let epoch_b = epoch_a + 0.75;
    let column = Column {
        data: vec![cancellation, epoch_a, epoch_b],
    };
    let (pairs, min_positive) = hilo_upload(&device, &queue, &column);
    let expected_pairs: Vec<_> = column
        .data
        .iter()
        .copied()
        .map(split_f64_to_f32_pair)
        .collect();
    assert_pair_bits(&pairs, &expected_pairs);
    let (cancel_hi, cancel_lo) = split_f64_to_f32_pair(cancellation);
    assert!(cancel_lo < 0.0);
    assert_eq!(min_positive, Some(cancel_hi as f64 + cancel_lo as f64));
    let epoch_delta = (pairs[2].0 - pairs[1].0) + (pairs[2].1 - pairs[1].1);
    assert!((epoch_delta as f64 - 0.75).abs() < 1.0e-3);

    let extremes = Column {
        data: vec![
            f64::from_bits(1),
            f64::MAX,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -0.0,
        ],
    };
    let (pairs, min_positive) = hilo_upload(&device, &queue, &extremes);
    let expected_pairs: Vec<_> = extremes
        .data
        .iter()
        .copied()
        .map(split_f64_to_f32_pair)
        .collect();
    assert_pair_bits(&pairs, &expected_pairs);
    assert_eq!(min_positive, None);

    let optional = Column {
        data: vec![None, Some(cancellation)],
    };
    let (pairs, min_positive) = hilo_upload(&device, &queue, &optional);
    assert_pair_bits(&pairs, &[(f32::NAN, 0.0), (cancel_hi, cancel_lo)]);
    assert_eq!(min_positive, Some(cancel_hi as f64 + cancel_lo as f64));
}

fn shared_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    static DEVICE: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let (device, queue) = test_gpu::device();
            Some((device, queue))
        })
        .as_ref()
        .map(|(device, queue)| (device.clone(), queue.clone()))
}

#[derive(Debug, PartialEq)]
struct SlotSnapshot {
    offset: u64,
    byte_size: u64,
    len_values: usize,
    generation: u32,
    min_bits: u64,
    max_bits: u64,
    min_positive_bits: Option<u64>,
}

#[derive(Debug, PartialEq)]
struct HandleSnapshot {
    generation: u32,
    offset: u64,
    byte_size: u64,
    len_values: usize,
}

#[derive(Debug, PartialEq)]
struct PoolSnapshot {
    used_bytes: u64,
    free_bytes: u64,
    generation: u32,
    slot: Option<SlotSnapshot>,
    handle: Option<HandleSnapshot>,
}

fn snapshot(pool: &ColumnPool, id: &str) -> PoolSnapshot {
    PoolSnapshot {
        used_bytes: pool.used_bytes(),
        free_bytes: pool.free_bytes(),
        generation: pool.generation(),
        slot: pool.slot(id).map(|slot| SlotSnapshot {
            offset: slot.offset,
            byte_size: slot.byte_size,
            len_values: slot.len_values,
            generation: slot.generation,
            min_bits: slot.min.to_bits(),
            max_bits: slot.max.to_bits(),
            min_positive_bits: slot.min_positive.map(f64::to_bits),
        }),
        handle: pool.handle_for(id).map(|handle| HandleSnapshot {
            generation: handle.generation,
            offset: handle.offset,
            byte_size: handle.byte_size,
            len_values: handle.len_values,
        }),
    }
}

fn read_pairs(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pool: &ColumnPool,
    handle: ColumnHandle,
) -> Vec<(f32, f32)> {
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("column upload stats test readback"),
        size: handle.byte_size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("column upload stats test encoder"),
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
        .expect("readback poll");
    receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("readback callback")
        .expect("readback map");
    let mapped = readback
        .slice(..handle.byte_size)
        .get_mapped_range()
        .expect("readback is mapped");
    let pairs = decode_pairs(&mapped[..handle.len_values * COLUMN_VALUE_BYTES]);
    drop(mapped);
    readback.unmap();
    pairs
}

#[test]
fn pool_calls_fused_writer_once_and_panics_roll_back() {
    let ledger = std::sync::Arc::new(renderer::gpu_memory::GpuLedger::new());
    let Some((device, queue)) = shared_device() else {
        eprintln!("no GPU adapter; skipping pool contract assertions");
        return;
    };
    let mut pool = ColumnPool::new(
        renderer::GpuAllocCtx::new(&device, &queue, &ledger),
        3 * ALIGN,
    )
    .unwrap();
    let add_source = CountingScalar {
        values: &[1.0, 2.0],
        calls: Cell::new(0),
    };
    let handle = pool
        .add_column(
            "x".into(),
            &add_source,
            renderer::GpuAllocCtx::new(&device, &queue, &ledger),
        )
        .unwrap();
    assert_eq!(add_source.calls.get(), 1);
    assert_eq!(pool.slot("x").unwrap().min_positive, Some(1.0));
    let before = snapshot(&pool, "x");
    let old_pairs = read_pairs(&device, &queue, &pool, handle);

    let replacement = CountingHiLo {
        values: &[(1.0, -1.0), (2.0, -1.5)],
        calls: Cell::new(0),
    };
    {
        let pending = pool
            .begin_upsert_hilo_column(
                "x".into(),
                &replacement,
                renderer::GpuAllocCtx::new(&device, &queue, &ledger),
            )
            .unwrap();
        assert_eq!(replacement.calls.get(), 1);
        assert_eq!(pending.pool().slot("x").unwrap().min_positive, Some(0.5));
    }
    assert_eq!(replacement.calls.get(), 1);
    assert_eq!(snapshot(&pool, "x"), before);
    assert_eq!(read_pairs(&device, &queue, &pool, handle), old_pairs);

    let panic_add = PanickingScalar {
        calls: Cell::new(0),
    };
    let add_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = pool.add_column(
            "panic".into(),
            &panic_add,
            renderer::GpuAllocCtx::new(&device, &queue, &ledger),
        );
    }));
    assert!(add_result.is_err());
    assert_eq!(panic_add.calls.get(), 1);
    assert!(pool.slot("panic").is_none());
    assert_eq!(snapshot(&pool, "x"), before);

    let panic_upsert = PanickingScalar {
        calls: Cell::new(0),
    };
    let upsert_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = pool.begin_upsert_column(
            "x".into(),
            &panic_upsert,
            renderer::GpuAllocCtx::new(&device, &queue, &ledger),
        );
    }));
    assert!(upsert_result.is_err());
    assert_eq!(panic_upsert.calls.get(), 1);
    assert_eq!(snapshot(&pool, "x"), before);
    assert_eq!(read_pairs(&device, &queue, &pool, handle), old_pairs);
}

/// The pool's statistics are measured from the pairs a source writes: the
/// finite range and smallest positive of the values the GPU reads, through
/// both traits, with non-finite values left out. A source has no way to
/// report others.
#[test]
fn pool_statistics_are_measured_from_the_written_values() {
    let ledger = std::sync::Arc::new(renderer::gpu_memory::GpuLedger::new());
    let Some((device, queue)) = shared_device() else {
        eprintln!("no GPU adapter; skipping measured statistics");
        return;
    };
    let ctx = || renderer::GpuAllocCtx::new(&device, &queue, &ledger);
    let mut pool = ColumnPool::new(ctx(), ALIGN).unwrap();
    pool.add_column(
        "scalar".into(),
        &CustomScalar(vec![f32::NAN, 3.0, -2.5, f32::INFINITY, 0.0, 7.25]),
        ctx(),
    )
    .unwrap();
    let slot = pool.slot("scalar").unwrap();
    assert_eq!(
        (slot.min, slot.max, slot.min_positive),
        (-2.5, 7.25, Some(3.0))
    );

    let (hi, lo) = split_f64_to_f32_pair(1_700_000_000_000.125);
    pool.add_hilo_column(
        "hilo".into(),
        &CustomHiLo(vec![(hi, lo), (-4.0, 0.0), (f32::NAN, 0.0)]),
        ctx(),
    )
    .unwrap();
    let slot = pool.slot("hilo").unwrap();
    assert_eq!(slot.min, -4.0);
    assert_eq!(slot.max, hi as f64 + lo as f64);
    assert_eq!(slot.min_positive, Some(hi as f64 + lo as f64));

    // No finite value: an empty range and no positive value.
    pool.add_column(
        "missing".into(),
        &CustomScalar(vec![f32::NAN, f32::NEG_INFINITY]),
        ctx(),
    )
    .unwrap();
    let slot = pool.slot("missing").unwrap();
    assert_eq!(
        (slot.min, slot.max, slot.min_positive),
        (f64::INFINITY, f64::NEG_INFINITY, None)
    );
}

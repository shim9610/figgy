//! GPU columnar memory pool — column-sized slabs in one big buffer, with an
//! offset table and ping-pong defrag.
//!
//! - One `primary` GPU buffer holds every column, packed by offset.
//! - `slots: HashMap<ColumnId, ColumnSlot>` is the SSoT mapping id → byte
//!   range; `free: Vec<FreeRegion>` tracks holes (first-fit on add, coalesce
//!   with neighbors on remove).
//! - `add_column` is a zero-copy stream upload: a staging buffer created
//!   with `mapped_at_creation` lets `ColumnSource::write_f32_le_into` fill
//!   bytes directly with no intermediate `Vec`.
//! - `defragment` packs survivors into a backup buffer with GPU-internal
//!   copies (no PCIe traffic) then swaps `primary <-> backup`.
//!
//! `ColumnHandle` carries a `generation` value; defrag and `clear` bump
//! `generation`, so callers detect staleness via `is_valid_handle` and
//! re-fetch with `handle_for`.
//!
//! Auto-defrag is opt-in via [`DefragPolicy`] (default `Manual`). With
//! `OnAllocFailure`, an `OutOfSpace` from `add_column` triggers one
//! `defragment()` and a single retry.
//!
//! All offsets and sizes are [`ALIGN`] = 256-byte aligned to satisfy wgpu's
//! storage-binding alignment; vertex slices reuse the same value to keep
//! mode switching free of caveats.

use std::collections::HashMap;

use wgpu::{Buffer, BufferDescriptor, BufferUsages, Device, Queue};

use crate::data::ColumnSource;

// Defined in the model crate (`model::data`); re-exported here so
// `data_render::ColumnId` stays a valid path.
pub use crate::data::ColumnId;

/// Alignment (in bytes) for every offset and size in the pool.
pub const ALIGN: u64 = 256;

#[inline]
fn align_up(x: u64, a: u64) -> u64 {
    (x + a - 1) & !(a - 1)
}

#[inline]
fn try_align_up(x: u64, a: u64) -> Option<u64> {
    x.checked_add(a - 1).map(|v| v & !(a - 1))
}

/// One column's occupied region inside the pool.
#[derive(Debug, Clone)]
pub struct ColumnSlot {
    pub id: ColumnId,
    pub offset: u64,
    pub byte_size: u64,
    pub len_values: usize,
    pub generation: u32,
    /// Captured from `ColumnSource::min/max` at `add_column` time so auto-fit
    /// can read the range without rescanning data that lives on the GPU.
    pub min: f64,
    pub max: f64,
    /// Smallest strictly-positive value, scanned once at upload — the
    /// log-axis auto-fit lower bound when the data contains zeros or
    /// negatives. `None` when no positive value exists.
    ///
    /// Scalar stats like this are the ONLY per-value information retained on
    /// the CPU. The pool deliberately keeps no copy of the data itself —
    /// per-point geometry (dashed-line arc length) is computed on the GPU
    /// (`line_arc.wgsl`). Do not reintroduce CPU shadows.
    pub min_positive: Option<f64>,
}

/// A free region. Adjacent regions are merged on coalesce.
#[derive(Debug, Clone, Copy)]
pub struct FreeRegion {
    pub offset: u64,
    pub size: u64,
}

/// Auto-defrag policy. No `Default` impl — figgy avoids the `Default` trait;
/// callers set this explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefragPolicy {
    /// No automatic defrag. Caller must invoke `defragment()` directly.
    Manual,
    /// On `add_column`'s `OutOfSpace`, attempt one defrag and retry. If that
    /// still fails, the original `OutOfSpace` is returned.
    OnAllocFailure,
}

/// Lightweight handle handed out to the chart layer. `generation` lets
/// callers detect a stale handle after a defrag.
#[derive(Debug, Clone, Copy)]
pub struct ColumnHandle {
    pub generation: u32,
    pub offset: u64,
    pub byte_size: u64,
    pub len_values: usize,
}

impl ColumnHandle {
    /// Pass directly into `pool.buffer().slice(byte_range)`.
    pub fn byte_range(&self) -> std::ops::Range<u64> {
        self.offset..(self.offset + self.byte_size)
    }
}

/// Allocation failure modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocError {
    /// No free region is large enough (try defrag then retry).
    OutOfSpace { requested: u64, largest_free: u64, total_free: u64 },
    /// Requested GPU buffer is larger than the device can allocate.
    ResourceLimit { resource: &'static str, requested: u64, limit: u64 },
    /// Resource creation failed despite satisfying static device limits.
    AllocationFailed { resource: &'static str, reason: String },
    /// A column with this id already exists.
    DuplicateId(ColumnId),
    /// Source has length zero.
    EmptySource,
}

impl std::fmt::Display for AllocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocError::OutOfSpace { requested, largest_free, total_free } => write!(
                f,
                "ColumnPool out of space: need {} bytes, largest free = {}, total free = {}",
                requested, largest_free, total_free
            ),
            AllocError::ResourceLimit { resource, requested, limit } => write!(
                f,
                "{resource} exceeds GPU buffer limit: requested {requested}, limit {limit}"
            ),
            AllocError::AllocationFailed { resource, reason } => {
                write!(f, "{resource} allocation failed: {reason}")
            }
            AllocError::DuplicateId(id) => write!(f, "ColumnPool duplicate id: {id}"),
            AllocError::EmptySource => write!(f, "ColumnPool: empty source not allowed"),
        }
    }
}

impl std::error::Error for AllocError {}

fn create_buffer_checked(
    device: &Device,
    desc: &BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<Buffer, AllocError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_buffer(desc)))
        .map_err(|_| AllocError::AllocationFailed {
            resource,
            reason: "wgpu Device::create_buffer panicked".into(),
        })
}

/// GPU column slab + CPU-side offset table.
pub struct ColumnPool {
    primary: Buffer,
    capacity: u64,
    max_buffer_size: u64,
    slots: HashMap<ColumnId, ColumnSlot>,
    /// Kept sorted by offset (coalesce and first-fit both rely on this).
    free: Vec<FreeRegion>,
    generation: u32,
    /// Ping-pong target for defrag. Lazily created on the first defrag,
    /// then alternates with `primary`.
    backup: Option<Buffer>,
    /// Whether to auto-defrag on alloc failure. Default `Manual`.
    pub defrag_policy: DefragPolicy,
}

impl ColumnPool {
    /// New pool. `capacity_bytes` is rounded up to a multiple of `ALIGN`.
    pub fn new(device: &Device, capacity_bytes: u64) -> Result<Self, AllocError> {
        let max_buffer_size = device.limits().max_buffer_size;
        let requested = capacity_bytes.max(ALIGN);
        let capacity = try_align_up(requested, ALIGN).ok_or(AllocError::ResourceLimit {
            resource: "column pool buffer",
            requested,
            limit: max_buffer_size,
        })?;
        if capacity > max_buffer_size {
            return Err(AllocError::ResourceLimit {
                resource: "column pool buffer",
                requested: capacity,
                limit: max_buffer_size,
            });
        }
        let primary_desc = BufferDescriptor {
            label: Some("figgy column pool primary"),
            size: capacity,
            // VERTEX | STORAGE so the pool can serve both binding kinds.
            // COPY_DST for staging→primary uploads, COPY_SRC for the
            // primary→backup defrag copy.
            usage: BufferUsages::VERTEX
                | BufferUsages::STORAGE
                | BufferUsages::COPY_DST
                | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        };
        let primary = create_buffer_checked(device, &primary_desc, "column pool buffer")?;
        Ok(Self {
            primary,
            capacity,
            max_buffer_size,
            slots: HashMap::new(),
            free: vec![FreeRegion { offset: 0, size: capacity }],
            generation: 0,
            backup: None,
            defrag_policy: DefragPolicy::Manual,
        })
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    pub fn buffer(&self) -> &Buffer {
        &self.primary
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn used_bytes(&self) -> u64 {
        self.slots.values().map(|s| s.byte_size).sum()
    }

    pub fn free_bytes(&self) -> u64 {
        self.free.iter().map(|r| r.size).sum()
    }

    pub fn slot(&self, id: &str) -> Option<&ColumnSlot> {
        self.slots.get(id)
    }


    /// Drop all columns. The primary buffer is reused (capacity unchanged).
    /// Bumps `generation` so any outstanding handles become stale.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.free.push(FreeRegion { offset: 0, size: self.capacity });
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn handle_for(&self, id: &str) -> Option<ColumnHandle> {
        self.slots.get(id).map(|s| ColumnHandle {
            generation: s.generation,
            offset: s.offset,
            byte_size: s.byte_size,
            len_values: s.len_values,
        })
    }

    /// True if the handle is still valid for the current pool state.
    /// After a defrag or clear, returns false.
    pub fn is_valid_handle(&self, h: &ColumnHandle) -> bool {
        h.generation == self.generation
    }

    /// Add a column to the pool with a zero-copy stream upload.
    ///
    /// When `defrag_policy == OnAllocFailure` and the first attempt returns
    /// `OutOfSpace`, this calls `defragment()` once and retries; the original
    /// `OutOfSpace` is returned if the retry still fails.
    pub fn add_column(
        &mut self,
        id: ColumnId,
        source: &dyn ColumnSource,
        device: &Device,
        queue: &Queue,
    ) -> Result<ColumnHandle, AllocError> {
        // Only clone `id` for retry under OnAllocFailure; the default path allocates nothing extra.
        let retry_id = if self.defrag_policy == DefragPolicy::OnAllocFailure {
            Some(id.clone())
        } else {
            None
        };
        match self.try_add_column(id, source, device, queue) {
            Ok(h) => Ok(h),
            Err(e @ AllocError::OutOfSpace { .. }) => match retry_id {
                Some(rid) => {
                    self.defragment(device, queue)?;
                    self.try_add_column(rid, source, device, queue)
                }
                None => Err(e),
            },
            Err(e) => Err(e),
        }
    }

    /// Single attempt without auto-retry. Internal + test use.
    ///
    /// 1. First-fit allocation from the free list.
    /// 2. Create a `mapped_at_creation: true` staging buffer.
    /// 3. `ColumnSource::write_f32_le_into` writes bytes directly into the
    ///    mapped slice (no intermediate Vec).
    /// 4. Unmap, encode a staging→primary copy, submit.
    fn try_add_column(
        &mut self,
        id: ColumnId,
        source: &dyn ColumnSource,
        device: &Device,
        queue: &Queue,
    ) -> Result<ColumnHandle, AllocError> {
        if self.slots.contains_key(&id) {
            return Err(AllocError::DuplicateId(id));
        }
        let n = source.len();
        if n == 0 {
            return Err(AllocError::EmptySource);
        }

        let raw_bytes = (n as u64)
            .checked_mul(4)
            .ok_or(AllocError::ResourceLimit {
                resource: "column staging buffer",
                requested: u64::MAX,
                limit: self.max_buffer_size,
            })?;
        let byte_size = try_align_up(raw_bytes, ALIGN).ok_or(AllocError::ResourceLimit {
            resource: "column staging buffer",
            requested: raw_bytes,
            limit: self.max_buffer_size,
        })?;
        if byte_size > self.max_buffer_size {
            return Err(AllocError::ResourceLimit {
                resource: "column staging buffer",
                requested: byte_size,
                limit: self.max_buffer_size,
            });
        }

        let region_offset = self.alloc_region(byte_size)?;

        // Staging buffer — write into mapped memory directly, no Vec.
        let staging_desc = BufferDescriptor {
            label: Some("figgy column staging"),
            size: byte_size,
            usage: BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        };
        let staging = create_buffer_checked(device, &staging_desc, "column staging buffer")?;
        let mut min_positive = f32::INFINITY;
        {
            // wgpu 27's BufferViewMut derefs to `&mut [u8]`, so the column
            // can serialize itself straight into staging memory.
            let mut view = staging.slice(..).get_mapped_range_mut();
            source.write_f32_le_into(&mut view[..raw_bytes as usize]);
            // Scalar stats only — read the freshly written bytes once, retain
            // nothing (the upload path stays zero-copy and the pool keeps no
            // CPU shadow of the data).
            for b in view[..raw_bytes as usize].chunks_exact(4) {
                let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                if v > 0.0 && v < min_positive {
                    min_positive = v;
                }
            }
        }
        staging.unmap();

        // staging → primary[region_offset..] (GPU-internal copy).
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy column upload encoder"),
        });
        enc.copy_buffer_to_buffer(&staging, 0, &self.primary, region_offset, byte_size);
        queue.submit(std::iter::once(enc.finish()));

        let slot = ColumnSlot {
            id: id.clone(),
            offset: region_offset,
            byte_size,
            len_values: n,
            generation: self.generation,
            min: source.min(),
            max: source.max(),
            min_positive: min_positive.is_finite().then_some(min_positive as f64),
        };
        let handle = ColumnHandle {
            generation: slot.generation,
            offset: slot.offset,
            byte_size: slot.byte_size,
            len_values: slot.len_values,
        };
        self.slots.insert(id, slot);
        Ok(handle)
    }

    /// Remove a column. Returns its region to the free list and coalesces
    /// with neighbors.
    pub fn remove_column(&mut self, id: &str) -> bool {
        let Some(slot) = self.slots.remove(id) else {
            return false;
        };
        self.free.push(FreeRegion {
            offset: slot.offset,
            size: slot.byte_size,
        });
        self.coalesce_free();
        true
    }

    /// Pack every live column tightly from offset 0 of `primary` (ping-pong).
    ///
    /// Algorithm:
    /// 1. Sort slots by current offset (preserves cache locality).
    /// 2. Compute new ALIGN-rounded packed offsets.
    /// 3. If already packed, normalize the free list to one tail region and
    ///    return false.
    /// 4. Lazily create `backup` (same capacity / usage as `primary`).
    /// 5. `copy_buffer_to_buffer` each slot from `primary[old_off..]` into
    ///    `backup[new_off..]` — all GPU-internal, no PCIe traffic.
    /// 6. Submit, then swap `primary <-> backup`.
    /// 7. Bump `generation`, invalidating outstanding handles.
    /// 8. Update slot offsets/generation; free list = single tail region
    ///    `[next..capacity)`.
    ///
    /// Returns true iff something actually moved (caller must re-fetch
    /// handles via `handle_for`).
    pub fn defragment(&mut self, device: &Device, queue: &Queue) -> Result<bool, AllocError> {
        // Empty pool: just normalize the free list.
        if self.slots.is_empty() {
            let already = self.free.len() == 1
                && self.free[0].offset == 0
                && self.free[0].size == self.capacity;
            if already {
                return Ok(false);
            }
            self.free.clear();
            self.free.push(FreeRegion { offset: 0, size: self.capacity });
            self.generation = self.generation.wrapping_add(1);
            return Ok(true);
        }

        // Pack in the current offset order.
        let mut order: Vec<ColumnId> = self.slots.keys().cloned().collect();
        order.sort_by_key(|id| self.slots[id].offset);

        let mut new_offsets: Vec<u64> = Vec::with_capacity(order.len());
        let mut next: u64 = 0;
        for id in &order {
            new_offsets.push(next);
            next = align_up(next + self.slots[id].byte_size, ALIGN);
        }

        // Already packed? Normalize free list and return false.
        let already_packed = order
            .iter()
            .zip(new_offsets.iter())
            .all(|(id, &n)| self.slots[id].offset == n);
        if already_packed {
            let tail_ok = self.free.len() <= 1
                && self.free.first().is_none_or(|r| r.offset == next && r.offset + r.size == self.capacity);
            if !tail_ok {
                self.free.clear();
                if next < self.capacity {
                    self.free.push(FreeRegion { offset: next, size: self.capacity - next });
                }
            }
            return Ok(false);
        }

        // Lazily create backup with the same capacity/usage as primary.
        if self.backup.is_none() {
            let backup_desc = BufferDescriptor {
                label: Some("figgy column pool backup"),
                size: self.capacity,
                usage: BufferUsages::VERTEX
                    | BufferUsages::STORAGE
                    | BufferUsages::COPY_DST
                    | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            };
            self.backup = Some(create_buffer_checked(device, &backup_desc, "column pool backup")?);
        }

        // primary[old_off..] -> backup[new_off..] (GPU-internal copy).
        // The `is_none` branch above guarantees `backup` is `Some`; the
        // graceful exit below exists only to handle invariant violations.
        {
            let Some(backup) = self.backup.as_ref() else {
                return Ok(false);
            };
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy column pool defrag"),
            });
            for (id, &new_off) in order.iter().zip(new_offsets.iter()) {
                let slot = &self.slots[id];
                enc.copy_buffer_to_buffer(&self.primary, slot.offset, backup, new_off, slot.byte_size);
            }
            queue.submit(std::iter::once(enc.finish()));
        }

        // primary <-> backup ping-pong swap.
        let Some(new_primary) = self.backup.take() else {
            return Ok(false);
        };
        let old_primary = std::mem::replace(&mut self.primary, new_primary);
        self.backup = Some(old_primary);

        // Bump generation and update slots.
        self.generation = self.generation.wrapping_add(1);
        for (id, &new_off) in order.iter().zip(new_offsets.iter()) {
            if let Some(slot) = self.slots.get_mut(id) {
                slot.offset = new_off;
                slot.generation = self.generation;
            }
        }

        // Free list collapses to a single tail region.
        self.free.clear();
        if next < self.capacity {
            self.free.push(FreeRegion { offset: next, size: self.capacity - next });
        }
        Ok(true)
    }

    /// Sort `free` by offset and merge adjacent regions.
    fn coalesce_free(&mut self) {
        if self.free.len() < 2 {
            return;
        }
        self.free.sort_by_key(|r| r.offset);
        let mut merged: Vec<FreeRegion> = Vec::with_capacity(self.free.len());
        for r in self.free.drain(..) {
            if let Some(last) = merged.last_mut() {
                if last.offset + last.size == r.offset {
                    last.size += r.size;
                    continue;
                }
            }
            merged.push(r);
        }
        self.free = merged;
    }

    /// First-fit allocation. Splits the chosen region and updates the free
    /// list. On failure returns the largest free region size and the total
    /// free size for diagnostics.
    fn alloc_region(&mut self, size: u64) -> Result<u64, AllocError> {
        let idx = self.free.iter().position(|r| r.size >= size);
        let Some(idx) = idx else {
            let largest = self.free.iter().map(|r| r.size).max().unwrap_or(0);
            let total = self.free.iter().map(|r| r.size).sum();
            return Err(AllocError::OutOfSpace {
                requested: size,
                largest_free: largest,
                total_free: total,
            });
        };
        let region = self.free[idx];
        let chosen = region.offset;
        if region.size == size {
            self.free.remove(idx);
        } else {
            self.free[idx] = FreeRegion {
                offset: region.offset + size,
                size: region.size - size,
            };
        }
        Ok(chosen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Column;
    use crate::data_render::{create_instance, request_adapter, request_device};

    fn mk_pool(cap: u64) -> Option<(wgpu::Device, wgpu::Queue, ColumnPool)> {
        let inst = create_instance();
        let adapter = request_adapter(&inst).ok()?;
        let (device, queue) = request_device(&adapter).ok()?;
        let pool = ColumnPool::new(&device, cap).ok()?;
        Some((device, queue, pool))
    }

    fn col_f64(data: Vec<f64>) -> Column<f64> {
        let min = data.iter().copied().fold(f64::INFINITY, f64::min);
        let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Column { data, min, max }
    }

    #[test]
    fn pool_basic_add_lookup_remove() {
        let Some((device, queue, mut pool)) = mk_pool(64 * 1024) else {
            println!("no adapter — skipping");
            return;
        };

        let c = col_f64((0..100).map(|i| i as f64).collect());
        let h = pool.add_column("x".to_string(), &c, &device, &queue).unwrap();

        // 100 * 4 = 400 bytes, rounded up to ALIGN(256) → 512.
        assert_eq!(h.byte_size, 512);
        assert_eq!(h.offset, 0);
        assert_eq!(h.len_values, 100);

        assert!(pool.handle_for("x").is_some());
        assert_eq!(pool.used_bytes(), 512);

        let removed = pool.remove_column("x");
        assert!(removed);
        assert!(pool.handle_for("x").is_none());
        assert_eq!(pool.used_bytes(), 0);
        assert_eq!(pool.free_bytes(), pool.capacity());
    }

    #[test]
    fn pool_sequential_alloc_offsets() {
        let Some((device, queue, mut pool)) = mk_pool(64 * 1024) else {
            return;
        };
        let a = col_f64((0..50).map(|i| i as f64).collect());     // 200 → 256
        let b = col_f64((0..200).map(|i| i as f64).collect());    // 800 → 1024
        let c = col_f64((0..10).map(|i| i as f64).collect());     // 40 → 256

        let ha = pool.add_column("a".into(), &a, &device, &queue).unwrap();
        let hb = pool.add_column("b".into(), &b, &device, &queue).unwrap();
        let hc = pool.add_column("c".into(), &c, &device, &queue).unwrap();

        assert_eq!(ha.offset, 0);
        assert_eq!(hb.offset, 256);
        assert_eq!(hc.offset, 256 + 1024);
        assert_eq!(pool.used_bytes(), 256 + 1024 + 256);
    }

    #[test]
    fn pool_coalesce_after_remove() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let a = col_f64((0..50).map(|i| i as f64).collect()); // 256
        let b = col_f64((0..50).map(|i| i as f64).collect()); // 256
        let c = col_f64((0..50).map(|i| i as f64).collect()); // 256

        pool.add_column("a".into(), &a, &device, &queue).unwrap();
        pool.add_column("b".into(), &b, &device, &queue).unwrap();
        pool.add_column("c".into(), &c, &device, &queue).unwrap();

        // Remove middle b → free = [256..512] + [768..end] (not adjacent).
        pool.remove_column("b");
        assert_eq!(pool.free.len(), 2);

        // Remove a → [0..512] (coalesced with b's hole) + [768..end].
        pool.remove_column("a");
        assert_eq!(pool.free.len(), 2);
        assert_eq!(pool.free[0].offset, 0);
        assert_eq!(pool.free[0].size, 512);

        // Remove c → everything coalesces back to one free region.
        pool.remove_column("c");
        assert_eq!(pool.free.len(), 1);
        assert_eq!(pool.free[0].offset, 0);
        assert_eq!(pool.free[0].size, pool.capacity());
    }

    #[test]
    fn pool_out_of_space() {
        let Some((device, queue, mut pool)) = mk_pool(1024) else {
            return;
        };
        let big = col_f64((0..500).map(|i| i as f64).collect()); // 2000 → 2048
        let res = pool.add_column("big".into(), &big, &device, &queue);
        assert!(matches!(res, Err(AllocError::OutOfSpace { .. })));
    }

    #[test]
    fn pool_duplicate_id() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let a = col_f64((0..10).map(|i| i as f64).collect());
        pool.add_column("dup".into(), &a, &device, &queue).unwrap();
        let res = pool.add_column("dup".into(), &a, &device, &queue);
        assert!(matches!(res, Err(AllocError::DuplicateId(_))));
    }

    #[test]
    fn pool_handle_byte_range() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let c = col_f64((0..100).map(|i| i as f64).collect());
        let h = pool.add_column("x".into(), &c, &device, &queue).unwrap();
        let r = h.byte_range();
        assert_eq!(r.start, 0);
        assert_eq!(r.end, 512);
    }

    #[test]
    fn pool_defragment_compacts_after_remove() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let a = col_f64((0..50).map(|i| i as f64).collect()); // 256
        let b = col_f64((0..50).map(|i| i as f64).collect()); // 256
        let c = col_f64((0..50).map(|i| i as f64).collect()); // 256

        let _ = pool.add_column("a".into(), &a, &device, &queue).unwrap();
        let _ = pool.add_column("b".into(), &b, &device, &queue).unwrap();
        let hc_before = pool.add_column("c".into(), &c, &device, &queue).unwrap();
        let gen_before = pool.generation();

        // Remove middle b → hole at [256..512]; tail free region unchanged.
        pool.remove_column("b");
        assert_eq!(pool.slots["c"].offset, 512);
        assert_eq!(pool.free.len(), 2);
        assert_eq!(pool.free[0].offset, 256);
        assert_eq!(pool.free[0].size, 256);

        // After defrag → a@0, c@256, free is a single tail region.
        let moved = pool.defragment(&device, &queue).unwrap();
        assert!(moved);
        assert_eq!(pool.slots["a"].offset, 0);
        assert_eq!(pool.slots["c"].offset, 256);
        assert_eq!(pool.free.len(), 1);
        assert_eq!(pool.free[0].offset, 512);
        assert_eq!(pool.free[0].size, pool.capacity() - 512);

        // Generation bumped; old handle is stale.
        assert_ne!(pool.generation(), gen_before);
        assert!(!pool.is_valid_handle(&hc_before));

        // A re-fetched handle is valid.
        let hc_new = pool.handle_for("c").unwrap();
        assert!(pool.is_valid_handle(&hc_new));
        assert_eq!(hc_new.offset, 256);
    }

    #[test]
    fn pool_defragment_no_op_when_packed() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let a = col_f64((0..50).map(|i| i as f64).collect());
        let b = col_f64((0..50).map(|i| i as f64).collect());
        pool.add_column("a".into(), &a, &device, &queue).unwrap();
        pool.add_column("b".into(), &b, &device, &queue).unwrap();
        let g0 = pool.generation();

        let moved = pool.defragment(&device, &queue).unwrap();
        assert!(!moved);
        assert_eq!(pool.generation(), g0); // no-op leaves generation alone.
    }

    #[test]
    fn pool_on_alloc_failure_auto_defrags() {
        // Capacity holds exactly three 256-byte slots.
        let Some((device, queue, mut pool)) = mk_pool(3 * 256) else {
            return;
        };
        pool.defrag_policy = DefragPolicy::OnAllocFailure;

        let a = col_f64((0..50).map(|i| i as f64).collect());
        let b = col_f64((0..50).map(|i| i as f64).collect());
        let c = col_f64((0..50).map(|i| i as f64).collect());
        let d = col_f64((0..50).map(|i| i as f64).collect());

        pool.add_column("a".into(), &a, &device, &queue).unwrap();
        pool.add_column("b".into(), &b, &device, &queue).unwrap();
        pool.add_column("c".into(), &c, &device, &queue).unwrap();

        // Remove middle b → free [256..512] (256 bytes).
        pool.remove_column("b");

        // Adding d uses the [256..512] hole via plain first-fit, so this
        // path doesn't actually exercise the OnAllocFailure retry — it
        // succeeds on the first try.
        pool.add_column("d".into(), &d, &device, &queue).unwrap();
        assert_eq!(pool.slots["d"].offset, 256);
    }

    #[test]
    fn pool_on_alloc_failure_auto_defrag_then_succeed() {
        // Capacity = 4 * 256 = 1024. After three 256-byte slots and the
        // middle one removed, fragmentation makes first-fit for a 512-byte
        // slot fail; OnAllocFailure should defrag and the retry should win.
        let Some((device, queue, mut pool)) = mk_pool(4 * 256) else {
            return;
        };
        pool.defrag_policy = DefragPolicy::OnAllocFailure;

        let small_a = col_f64((0..50).map(|i| i as f64).collect()); // 256
        let small_b = col_f64((0..50).map(|i| i as f64).collect()); // 256
        let small_c = col_f64((0..50).map(|i| i as f64).collect()); // 256
        let big = col_f64((0..120).map(|i| i as f64).collect());    // 480 → 512

        pool.add_column("a".into(), &small_a, &device, &queue).unwrap();
        pool.add_column("b".into(), &small_b, &device, &queue).unwrap();
        pool.add_column("c".into(), &small_c, &device, &queue).unwrap();
        // free = [768..1024]. Remove b → free = [256..512] + [768..1024].
        pool.remove_column("b");
        // big needs 512 contiguous. First-fit fails (largest free is 256),
        // defrag fuses the hole and tail into [512..1024], retry succeeds.
        let res = pool.add_column("big".into(), &big, &device, &queue);
        assert!(res.is_ok(), "auto defrag retry should have succeeded: {res:?}");
        // big lands right after a/c, at offset 512.
        assert_eq!(pool.slots["big"].offset, 512);
    }

    #[test]
    fn pool_manual_policy_does_not_auto_defrag() {
        let Some((device, queue, mut pool)) = mk_pool(4 * 256) else {
            return;
        };
        // Default policy is Manual.
        let small_a = col_f64((0..50).map(|i| i as f64).collect());
        let small_b = col_f64((0..50).map(|i| i as f64).collect());
        let small_c = col_f64((0..50).map(|i| i as f64).collect());
        let big = col_f64((0..120).map(|i| i as f64).collect());

        pool.add_column("a".into(), &small_a, &device, &queue).unwrap();
        pool.add_column("b".into(), &small_b, &device, &queue).unwrap();
        pool.add_column("c".into(), &small_c, &device, &queue).unwrap();
        pool.remove_column("b");
        // Manual policy: no retry → OutOfSpace.
        let res = pool.add_column("big".into(), &big, &device, &queue);
        assert!(matches!(res, Err(AllocError::OutOfSpace { .. })));
    }
}

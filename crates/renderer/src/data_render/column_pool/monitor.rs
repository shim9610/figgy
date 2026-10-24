//! Pool identity, device ceilings, the host budget and the byte, peak and
//! buffer-creation accounting.

use super::*;

/// Opaque identity of one [`ColumnPool`] instance.
///
/// Clones retain the same identity. Equality is allocation identity rather
/// than a numeric stamp, so independently-created pools cannot compare as the
/// same while either identity is live.
#[derive(Clone)]
pub(crate) struct PoolIdentity(Arc<PoolIdentityInner>);

pub(super) struct PoolIdentityInner {
    pub(super) _private: u8,
}

impl PoolIdentity {
    pub(super) fn new() -> Self {
        Self(Arc::new(PoolIdentityInner { _private: 0 }))
    }

    pub(crate) fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl PartialEq for PoolIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.same_instance(other)
    }
}

impl Eq for PoolIdentity {}

impl fmt::Debug for PoolIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PoolIdentity")
            .finish_non_exhaustive()
    }
}

/// Largest single buffer this pool may create on `device`.
///
/// The pool binds its whole primary buffer as ONE storage binding — the
/// arc-length scan and the constellation star pass both do — so a buffer is
/// legal only under **both** device ceilings. Read here, at the allocation
/// site, rather than cached on the pool: one place learns the number, so the
/// renderer's view and the pool's view cannot drift apart.
pub(super) fn buffer_ceiling(device: &Device) -> u64 {
    let limits = device.limits();
    buffer_ceiling_from(
        limits.max_buffer_size,
        limits.max_storage_buffer_binding_size,
    )
}

/// Pure half of [`buffer_ceiling`], split out so the case that matters —
/// a device whose storage ceiling is the tighter of the two — is testable
/// without owning a device that reports it.
pub(super) fn buffer_ceiling_from(
    max_buffer_size: u64,
    max_storage_buffer_binding_size: u64,
) -> u64 {
    max_buffer_size.min(max_storage_buffer_binding_size)
}

pub(super) fn create_buffer_checked(
    device: &Device,
    desc: &BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<Buffer, AllocError> {
    // gpu-alloc: ColumnPool
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_buffer(desc))).map_err(
        |_| AllocError::AllocationFailed {
            resource,
            reason: "wgpu Device::create_buffer panicked".into(),
        },
    )
}

/// A host-declared ceiling and the out-of-pool bytes it is measured against.
///
/// The two travel together because neither means anything alone. Held as
/// separate fields they could disagree — and they did: the caller summed the
/// out-of-pool total on every pool call even with no ceiling set, where the
/// pool never reads it, so the choice was between wasted work and passing a
/// fabricated zero that a future reader outside the budget check would take at
/// face value. As one value there is no zero to fabricate and nothing to
/// compute when there is no ceiling.
#[derive(Clone, Copy, Debug)]
pub struct GpuBudget {
    /// Ceiling for all renderer GPU bytes, pool included.
    pub ceiling_bytes: u64,
    /// GPU bytes held outside this pool (panel textures, MSAA, export targets,
    /// derived buffers), summed by the caller **just before this call** — never
    /// cached inside the pool, because a copy the caller forgets to refresh
    /// diverges silently.
    pub external_bytes: u64,
}

/// Everything an allocating pool call needs from the caller.
///
/// The pool owns its buffers but not the device, so GPU access arrives as an
/// argument. The budget rides along in the same argument so the **allocation
/// site itself** can compare limit, current usage, and ceiling before creating
/// a buffer.
///
/// `plan_relayout_capacity` charges a growth's peak as
/// `external_bytes + old capacity + new capacity` — the two pool buffers
/// coexist across the copy — and refuses the growth when that crosses the
/// ceiling.
#[derive(Clone, Copy)]
pub struct GpuAllocCtx<'a> {
    pub device: &'a Device,
    pub queue: &'a Queue,
    /// `None` = device limits are the only bound.
    pub budget: Option<GpuBudget>,
}

impl<'a> GpuAllocCtx<'a> {
    /// Context with no host ceiling. Device limits still apply.
    pub fn unbudgeted(device: &'a Device, queue: &'a Queue) -> Self {
        Self {
            device,
            queue,
            budget: None,
        }
    }
}

impl ColumnPool {
    #[allow(dead_code)]
    pub(crate) fn identity(&self) -> PoolIdentity {
        self.identity.clone()
    }

    pub fn used_bytes(&self) -> u64 {
        self.slots.values().map(|s| s.byte_size).sum()
    }

    pub fn free_bytes(&self) -> u64 {
        self.free.iter().map(|r| r.size).sum()
    }

    /// Size of the optional defragmentation ping-pong slab.
    pub fn backup_bytes(&self) -> u64 {
        self.backup.as_ref().map_or(0, |buffer| buffer.size())
    }

    /// Release only the optional defragmentation target. The primary and all
    /// live columns remain untouched. Call after submitted work completes.
    pub(crate) fn release_backup(&mut self) -> u64 {
        let Some(backup) = self.backup.take() else {
            return 0;
        };
        let bytes = backup.size();
        drop(backup);
        self.note_buffer_retired(bytes);
        bytes
    }

    /// GPU bytes this pool holds right now: the live slab plus the ping-pong
    /// backup while one exists.
    ///
    /// Derived from the buffers themselves rather than a counter, so it is the
    /// pool's own authority on its footprint and cannot drift from what the
    /// device was asked for.
    pub fn gpu_bytes(&self) -> u64 {
        self.primary
            .size()
            .saturating_add(self.backup.as_ref().map_or(0, |backup| backup.size()))
    }

    /// The ping-pong defragmentation target, when one has been materialized.
    ///
    /// Exposed for the renderer's accounting tests, which recount the pool's
    /// footprint from the buffers themselves rather than trusting the report.
    #[cfg(test)]
    pub(crate) fn backup_buffer_for_test(&self) -> Option<&Buffer> {
        self.backup.as_ref()
    }

    /// Bytes released by this pool that the device may still be holding.
    pub fn retired_bytes(&self) -> u64 {
        self.retired_bytes.total()
    }

    /// Compatibility helper: call only after GPU completion of every pending
    /// retirement, not merely submission. Already submitted batches are left
    /// to their completion callbacks. Renderer hosts use `end_gpu_frame`.
    pub fn clear_retired_bytes(&mut self) {
        self.retired_bytes
            .complete(self.retired_bytes.take_pending());
    }

    pub(crate) fn take_retirement(&self) -> (Arc<RetiredBytes>, u64) {
        (
            Arc::clone(&self.retired_bytes),
            self.retired_bytes.take_pending(),
        )
    }

    /// Highest footprint seen, transients included.
    pub fn peak_bytes(&self) -> u64 {
        self.peak_bytes
    }

    /// Forget the recorded peak; the next allocation starts a fresh mark.
    pub fn reset_peak_bytes(&mut self) {
        self.peak_bytes = self.gpu_bytes().saturating_add(self.retired_bytes());
    }

    /// Device buffers this pool has created since construction.
    pub fn buffer_creations(&self) -> u64 {
        self.buffer_creations
    }

    /// Charge a buffer this pool just created whose bytes coexist with
    /// [`Self::gpu_bytes`] — a staging buffer, a candidate primary, or the
    /// larger slab a growth is copying into.
    pub(super) fn note_buffer_created(&mut self, transient_bytes: u64) {
        self.buffer_creations = self.buffer_creations.saturating_add(1);
        let total = self
            .gpu_bytes()
            .saturating_add(self.retired_bytes())
            .saturating_add(transient_bytes);
        self.peak_bytes = self.peak_bytes.max(total);
    }

    /// Credit a buffer this pool just dropped. The bytes stay in
    /// [`Self::retired_bytes`] until the corresponding queue completion.
    pub(super) fn note_buffer_retired(&mut self, bytes: u64) {
        self.retired_bytes.retire(bytes);
    }
}

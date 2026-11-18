//! Pool identity, device ceilings, the allocation context and the pool's
//! footprint. The bytes, peak and creations are the device ledger's (#160).

use super::*;

/// Opaque identity of one [`ColumnPool`] instance.
///
/// Clones retain the same identity. Equality is allocation identity rather
/// than a numeric stamp, so independently-created pools cannot compare as the
/// same while either identity is live. See "Identity and staleness checks" in
/// `ARCHITECTURE.md`.
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

/// Create a pool buffer charged to `ledger` as [`GpuResourceKind::ColumnPool`]
/// for as long as it lives: the slab, a defragmentation or growth candidate,
/// or upload staging. Dropping it retires its bytes until queue completion.
pub(super) fn create_buffer_checked(
    device: &Device,
    ledger: &Arc<GpuLedger>,
    desc: &BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<TrackedBuffer, AllocError> {
    // gpu-alloc: ColumnPool
    crate::renderer::gpu_checked(device, resource, || device.create_buffer(desc))
        .map(|buffer| TrackedBuffer::new(ledger, GpuResourceKind::ColumnPool, buffer))
        .map_err(|error| pool_gpu_error(resource, error))
}

/// A GPU failure the pool met, in the pool's error type: out of memory is an
/// allocation failure, anything else [`AllocError::Gpu`].
pub(super) fn pool_gpu_error(resource: &'static str, error: crate::FiggyError) -> AllocError {
    match error {
        crate::FiggyError::GpuResourceAllocationFailed { reason, .. } => {
            AllocError::AllocationFailed { resource, reason }
        }
        other => AllocError::Gpu {
            resource,
            reason: other.to_string(),
        },
    }
}

/// Everything an allocating pool call needs from the caller.
///
/// The pool owns its buffers but not the device, so GPU access arrives as an
/// argument. The ledger is the device bundle's (#160): [`ColumnPool::new`]
/// charges every buffer the pool creates to it, and its budget bounds pool
/// growth against everything the device holds, other renderers' pools
/// included.
#[derive(Clone, Copy)]
pub struct GpuAllocCtx<'a> {
    pub device: &'a Device,
    pub queue: &'a Queue,
    pub ledger: &'a Arc<GpuLedger>,
}

impl<'a> GpuAllocCtx<'a> {
    pub fn new(device: &'a Device, queue: &'a Queue, ledger: &'a Arc<GpuLedger>) -> Self {
        Self {
            device,
            queue,
            ledger,
        }
    }
}

impl ColumnPool {
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
        // Dropping the tracked buffer retires its bytes in the ledger.
        backup.size()
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
        self.backup.as_deref()
    }
}

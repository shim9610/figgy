//! Provisional column removal.

use super::*;

impl ColumnPool {
    /// Provisionally remove a column so dependent GPU state can be prepared.
    pub fn begin_remove_column(
        &mut self,
        id: &str,
    ) -> Result<Option<ColumnRemoval<'_>>, AllocError> {
        if !self.slots.contains_key(id) {
            return Ok(None);
        }
        if self.is_member_column(id) {
            return Err(AllocError::MemberColumn(id.to_owned()));
        }
        Ok(Some(self.begin_remove_columns(&[id])?))
    }

    /// Provisionally remove several resident columns as one allocator change.
    ///
    /// Every id must be present and distinct. The returned pool view contains
    /// none of them, allowing dependent state to be prepared before commit;
    /// dropping the guard restores every slot, free region, allocation epoch,
    /// and public generation exactly.
    pub(crate) fn begin_remove_columns<'a>(
        &'a mut self,
        ids: &[&str],
    ) -> Result<ColumnRemoval<'a>, AllocError> {
        if ids.is_empty() {
            return Ok(ColumnRemoval {
                pool: self,
                rollback: None,
            });
        }
        let mut seen = std::collections::HashSet::new();
        seen.try_reserve(ids.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "column batch removal id set",
                reason: error.to_string(),
            })?;
        for id in ids {
            if !seen.insert(*id) {
                return Err(AllocError::DuplicateId((*id).to_owned()));
            }
            if !self.slots.contains_key(*id) {
                return Err(AllocError::AllocationFailed {
                    resource: "column batch removal",
                    reason: "resident column disappeared before removal".into(),
                });
            }
        }

        let next_generation = self.checked_generation_successor()?;
        let mut planned_slots = self.slots.clone();
        let mut removed_regions = Vec::new();
        removed_regions
            .try_reserve_exact(ids.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "column batch removal regions",
                reason: error.to_string(),
            })?;
        for id in ids {
            let slot = planned_slots.remove(*id).ok_or(AllocError::Internal {
                invariant: "validated resident column remains in removal plan",
            })?;
            removed_regions.push(FreeRegion {
                offset: slot.offset,
                size: slot.byte_size,
            });
        }
        for slot in planned_slots.values_mut() {
            slot.generation = next_generation;
        }
        let mut planned_free = self.free.clone();
        planned_free
            .try_reserve(removed_regions.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "column batch removal free list",
                reason: error.to_string(),
            })?;
        planned_free.extend(removed_regions);
        Self::coalesce_free(&mut planned_free);
        let mut planned_allocation_epochs = self.allocation_epochs.clone();
        for id in ids {
            planned_allocation_epochs.remove(*id);
        }

        let old_slots = std::mem::replace(&mut self.slots, planned_slots);
        let old_free = std::mem::replace(&mut self.free, planned_free);
        let old_allocation_epochs =
            std::mem::replace(&mut self.allocation_epochs, planned_allocation_epochs);
        let old_generation = std::mem::replace(&mut self.generation, next_generation);

        Ok(ColumnRemoval {
            pool: self,
            rollback: Some(RemovalRollback {
                slots: old_slots,
                free: old_free,
                generation: old_generation,
                allocation_epochs: old_allocation_epochs,
            }),
        })
    }
}

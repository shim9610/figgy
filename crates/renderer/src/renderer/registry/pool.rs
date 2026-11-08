//! Resident admission, defragmentation and the pool's memory and maintenance.

use super::*;

impl Renderer {
    /// Check whether an exact resident working set fits every declared static
    /// resource constraint without allocating or mutating renderer state.
    ///
    /// The check includes ALIGN-padded column storage, upload staging, a pool
    /// compaction/growth candidate, persistent derived bytes, caller-declared
    /// transition headroom, current live resources, and resources retired but
    /// not yet released by the GPU. It also applies the current pool policies,
    /// the device's combined buffer/storage-binding ceiling, the working-set
    /// cap, and [`Self::memory_budget`].
    ///
    /// Without a host memory budget the result is fail-closed as
    /// [`ResidentAdmissionStatus::MemoryBudgetUnset`]: WebGPU exposes device
    /// limits but not free VRAM. An admissible result is not a reservation, so
    /// the eventual atomic transition must repeat allocation-time checks and
    /// remain failure-atomic.
    pub fn resident_admission(&self, request: ResidentAdmissionRequest<'_>) -> ResidentAdmission {
        use crate::data::COLUMN_VALUE_BYTES;
        use crate::data_render::column_pool::{ALIGN, ResidentBatchCapacityError};

        let usage = self.gpu_memory_usage();
        let Some(current_gpu_bytes) = usage.checked_total_bytes() else {
            return ResidentAdmission {
                status: ResidentAdmissionStatus::ArithmeticOverflow,
                resident_column_bytes: 0,
                resident_working_set_bytes: 0,
                derived_resident_bytes: 0,
                upload_staging_bytes: 0,
                pool_capacity_before: self.pool.capacity(),
                pool_capacity_after: self.pool.capacity(),
                pool_transition_bytes: 0,
                pool_free_bytes: self.pool.free_bytes(),
                pool_largest_free_bytes: self.pool.largest_free_region(),
                current_gpu_bytes: u64::MAX,
                transition_peak_bytes: u64::MAX,
                working_set_limit_bytes: request.working_set_limit_bytes,
                memory_budget_bytes: self.gpu_ledger.budget(),
                device_buffer_limit_bytes: self
                    .caps
                    .max_buffer_size
                    .min(self.caps.max_storage_buffer_binding_size),
            };
        };
        let device_buffer_limit_bytes = self
            .caps
            .max_buffer_size
            .min(self.caps.max_storage_buffer_binding_size);
        let mut report = ResidentAdmission {
            status: ResidentAdmissionStatus::ArithmeticOverflow,
            resident_column_bytes: 0,
            resident_working_set_bytes: 0,
            derived_resident_bytes: 0,
            upload_staging_bytes: 0,
            pool_capacity_before: self.pool.capacity(),
            pool_capacity_after: self.pool.capacity(),
            pool_transition_bytes: 0,
            pool_free_bytes: self.pool.free_bytes(),
            pool_largest_free_bytes: self.pool.largest_free_region(),
            current_gpu_bytes,
            transition_peak_bytes: current_gpu_bytes,
            working_set_limit_bytes: request.working_set_limit_bytes,
            memory_budget_bytes: self.gpu_ledger.budget(),
            device_buffer_limit_bytes,
        };

        for (column, &length) in request.column_value_counts.iter().enumerate() {
            if length > u64::from(u32::MAX) || usize::try_from(length).is_err() {
                report.status =
                    ResidentAdmissionStatus::ColumnLengthLimitExceeded { column, length };
                return report;
            }
            if length == 0 {
                continue;
            }
            let Some(raw_bytes) = length.checked_mul(COLUMN_VALUE_BYTES as u64) else {
                return report;
            };
            let Some(region_bytes) = raw_bytes
                .checked_add(ALIGN - 1)
                .map(|bytes| bytes & !(ALIGN - 1))
            else {
                return report;
            };
            if region_bytes > device_buffer_limit_bytes {
                report.status = ResidentAdmissionStatus::DeviceBufferLimitExceeded;
                return report;
            }
            let Some(total) = report.resident_column_bytes.checked_add(region_bytes) else {
                return report;
            };
            report.resident_column_bytes = total;
        }
        report.upload_staging_bytes = report.resident_column_bytes;
        for (buffer, &bytes) in request.derived_buffer_sizes.iter().enumerate() {
            if bytes > device_buffer_limit_bytes {
                report.status =
                    ResidentAdmissionStatus::DerivedBufferLimitExceeded { buffer, bytes };
                return report;
            }
            let Some(total) = report.derived_resident_bytes.checked_add(bytes) else {
                return report;
            };
            report.derived_resident_bytes = total;
        }
        let Some(working_set_bytes) = report
            .resident_column_bytes
            .checked_add(report.derived_resident_bytes)
            .and_then(|bytes| bytes.checked_add(request.already_resident_working_set_bytes))
        else {
            return report;
        };
        report.resident_working_set_bytes = working_set_bytes;
        if working_set_bytes > request.working_set_limit_bytes {
            report.status = ResidentAdmissionStatus::WorkingSetLimitExceeded;
            return report;
        }

        let pool_plan = match self
            .pool
            .plan_resident_batch_capacity(report.resident_column_bytes, device_buffer_limit_bytes)
        {
            Ok(plan) => plan,
            Err(ResidentBatchCapacityError::Overflow) => return report,
            Err(ResidentBatchCapacityError::DeviceLimit) => {
                report.status = ResidentAdmissionStatus::DeviceBufferLimitExceeded;
                return report;
            }
            Err(ResidentBatchCapacityError::Policy) => {
                report.status = ResidentAdmissionStatus::PoolCapacityUnavailable;
                return report;
            }
        };
        report.pool_capacity_after = pool_plan.capacity_after;
        report.pool_transition_bytes = pool_plan.transition_bytes;

        let Some(transition_peak_bytes) = current_gpu_bytes
            .checked_add(pool_plan.transition_bytes)
            .and_then(|bytes| bytes.checked_add(report.upload_staging_bytes))
            .and_then(|bytes| bytes.checked_add(report.derived_resident_bytes))
            .and_then(|bytes| bytes.checked_add(request.transition_headroom_bytes))
        else {
            return report;
        };
        report.transition_peak_bytes = transition_peak_bytes;
        let Some(memory_budget) = self.gpu_ledger.budget() else {
            report.status = ResidentAdmissionStatus::MemoryBudgetUnset;
            return report;
        };
        report.status = if transition_peak_bytes > memory_budget {
            ResidentAdmissionStatus::MemoryBudgetExceeded
        } else {
            ResidentAdmissionStatus::Admissible
        };
        report
    }

    /// Compact every live column to the start of the pool. `true` iff
    /// anything actually moved.
    pub fn defragment(&mut self) -> Result<bool> {
        let result = self.defragment_unreported();
        self.reported(result)
    }

    pub(super) fn defragment_unreported(&mut self) -> Result<bool> {
        self.health.check()?;
        let defragment = self.pool.begin_defragment(pool_alloc_ctx(
            &self.device,
            &self.queue,
            &self.gpu_ledger,
        ))?;
        let picker = if defragment.relocated() {
            match self.picker.active.as_ref().map(|active| active.chart_id) {
                Some(chart_id) => {
                    let state = self
                        .chart_states
                        .get(&chart_id)
                        .ok_or(FiggyError::UnknownChart { id: chart_id })?;
                    let plan = PickChartPlan::for_active_layer(&state.config, &state.series)?;
                    self.picker.prepare_active_pool_mutation(
                        defragment.pool(),
                        chart_id,
                        plan,
                        &[],
                    )?
                }
                None => None,
            }
        } else {
            None
        };
        let moved = defragment.commit();
        if let Some(picker) = picker {
            picker.commit();
        }
        self.pending_defrag = false;
        Ok(moved)
    }

    /// Release the defrag backup and shrink the resident pool to the live
    /// columns (never below `minimum_capacity`). This never reads columns
    /// back to the CPU. It waits for submitted GPU work before releasing an
    /// old slab and again before reporting retired bytes as reusable.
    ///
    /// Call only at a host quiescence point: no unsubmitted command buffer may
    /// still reference the old pool. A failed candidate leaves live columns
    /// and picker bindings on the previous primary.
    pub async fn release_unused_pool_memory(&mut self, minimum_capacity: u64) -> Result<()> {
        let result = self
            .release_unused_pool_memory_unreported(minimum_capacity)
            .await;
        self.reported(result)
    }

    pub(super) async fn release_unused_pool_memory_unreported(
        &mut self,
        minimum_capacity: u64,
    ) -> Result<()> {
        self.wait_submitted_work().await?;
        self.release_pool_backup();
        self.wait_submitted_work().await?;
        if let Some(result) = self.relayout_pool_to_used(minimum_capacity) {
            let waited = self.wait_submitted_work().await;
            result?;
            waited?;
        }
        Ok(())
    }

    /// [`Self::release_unused_pool_memory`], polling the device on this
    /// thread for each wait.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn release_unused_pool_memory_blocking(&mut self, minimum_capacity: u64) -> Result<()> {
        let result = (|| {
            self.wait_submitted_work_blocking()?;
            self.release_pool_backup();
            self.wait_submitted_work_blocking()?;
            if let Some(result) = self.relayout_pool_to_used(minimum_capacity) {
                let waited = self.wait_submitted_work_blocking();
                result?;
                waited?;
            }
            Ok(())
        })();
        self.reported(result)
    }

    /// The release's first step, once the queued work is done: drop the
    /// backup slab and end the frame.
    pub(super) fn release_pool_backup(&mut self) {
        self.pool.release_backup();
        self.end_gpu_frame();
    }

    /// The release's second step, once the backup's release has completed:
    /// relayout the pool to its used bytes (at least `minimum_capacity`),
    /// and end the frame. `None` when the pool is already that small; the
    /// relayout's result otherwise, to return after its work is waited for.
    pub(super) fn relayout_pool_to_used(&mut self, minimum_capacity: u64) -> Option<Result<()>> {
        let target = self.pool.used_bytes().max(minimum_capacity);
        if target >= self.pool.capacity() {
            return None;
        }
        let result = (|| -> Result<()> {
            let relayout = self.pool.begin_relayout(
                pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger),
                target,
            )?;
            let picker = if relayout.relocated() {
                match self.picker.active.as_ref().map(|active| active.chart_id) {
                    Some(chart_id) => {
                        let state = self
                            .chart_states
                            .get(&chart_id)
                            .ok_or(FiggyError::UnknownChart { id: chart_id })?;
                        let plan = PickChartPlan::for_active_layer(&state.config, &state.series)?;
                        self.picker.prepare_active_pool_mutation(
                            relayout.pool(),
                            chart_id,
                            plan,
                            &[],
                        )?
                    }
                    None => None,
                }
            } else {
                None
            };
            relayout.commit();
            if let Some(picker) = picker {
                picker.commit();
            }
            self.pending_defrag = false;
            Ok(())
        })();
        self.end_gpu_frame();
        Some(result)
    }

    pub fn has_pending_maintenance(&self) -> bool {
        self.pending_defrag
    }

    /// Settle queued stream cancellations before a host chooses its draw path,
    /// then perform pending pool compaction. The return value reports only
    /// whether compaction changed the pool; stream retirement may also occur
    /// when it is false.
    pub fn process_pending_maintenance(&mut self) -> Result<bool> {
        let result = self.process_pending_maintenance_unreported();
        self.reported(result)
    }

    pub(super) fn process_pending_maintenance_unreported(&mut self) -> Result<bool> {
        self.health.check()?;
        self.service_stream_requests();
        if !self.pending_defrag {
            return Ok(false);
        }
        self.defragment()
    }

    pub fn set_defrag_policy(&mut self, policy: DefragPolicy) {
        self.pool.defrag_policy = policy;
    }

    /// Whether an upload that does not fit may enlarge the pool.
    pub fn pool_growth_policy(&self) -> data_render::GrowthPolicy {
        self.pool.growth_policy
    }

    /// Let uploads that do not fit enlarge the pool, or keep the fixed-capacity
    /// behavior.
    ///
    /// `GrowthPolicy::OnAllocFailure` is the default (#107): an upload that
    /// does not fit relayouts the pool into a larger buffer first, which
    /// invalidates every live [`ColumnHandle`] and `PreparedFrame` exactly as a
    /// defragmentation does. Growth is bounded only by the device ceiling and
    /// by [`Self::set_memory_budget`]; when it cannot happen the original
    /// `OutOfSpace` is what the caller sees. `GrowthPolicy::Fixed` keeps the
    /// capacity `try_new` was given: an upload with nowhere to go reports
    /// `OutOfSpace` and the host decides.
    pub fn set_pool_growth_policy(&mut self, policy: data_render::GrowthPolicy) {
        self.pool.growth_policy = policy;
    }

    pub fn handle_for(&self, id: &str) -> Result<ColumnHandle> {
        if self.streaming_sources.contains_key(id) {
            return Err(FiggyError::ColumnNotResident { id: id.to_owned() });
        }
        self.pool
            .handle_for(id)
            .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })
    }

    pub fn is_valid_handle(&self, h: &ColumnHandle) -> bool {
        self.pool.is_valid_handle(h)
    }
}

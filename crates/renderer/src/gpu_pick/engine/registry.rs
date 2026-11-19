//! Slot reuse, registry transitions and column validation.

use super::*;

impl GpuPickEngine {
    #[cfg(test)]
    pub(in crate::gpu_pick) fn current_reuse_slots<'a>(
        &self,
        skip: Option<usize>,
    ) -> Vec<GpuPickRegistrySlot<'a>> {
        self.registry
            .slots
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != skip)
            .map(|(current_index, slot)| GpuPickRegistrySlot::Reuse {
                current_index,
                source_id: slot.identity.source_id.clone(),
                series_id: slot.identity.series_id.clone(),
            })
            .collect()
    }

    pub(in crate::gpu_pick) fn check_pool_identity(
        &self,
        pool: &ColumnPool,
    ) -> Result<PoolIdentity, GpuPickError> {
        let target = pool.identity();
        if let Some(current) = self.registry.pool_identity.as_ref()
            && !current.same_instance(&target)
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        Ok(target)
    }

    pub(in crate::gpu_pick) fn prepare_reused_gpu(
        &self,
        pool: &ColumnPool,
        target_pool_identity: &PoolIdentity,
        current: &PickSeriesSlot,
        next_series_id: &str,
    ) -> Result<PickSeriesGpu, GpuPickError> {
        let series = &current.gpu;
        if !series
            .pool_identity
            .as_ref()
            .is_some_and(|identity| identity.same_instance(target_pool_identity))
        {
            return Err(GpuPickError::ForeignColumnPool);
        }

        let relocated = |id: &ColumnId,
                         expected: ColumnHandle,
                         expected_epoch: u64|
         -> Result<ColumnHandle, GpuPickError> {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: next_series_id.to_owned(),
                    column_id: id.clone(),
                });
            };
            if pool.allocation_epoch(id) != Some(expected_epoch)
                || current.len_values != expected.len_values
                || current.byte_size != expected.byte_size
            {
                return Err(GpuPickError::StaleColumn {
                    series_id: next_series_id.to_owned(),
                    column_id: id.clone(),
                });
            }
            Ok(current)
        };

        let x_handle = relocated(&series.x_column, series.x_handle, series.x_allocation_epoch)?;
        let y_handle = relocated(&series.y_column, series.y_handle, series.y_allocation_epoch)?;
        lane_base(x_handle, next_series_id)?;
        lane_base(y_handle, next_series_id)?;
        let (style_index_handle, style_index_base, style_index_len) =
            if let Some(id) = series.style_index_column.as_ref() {
                let expected = series
                    .style_index_handle
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                let expected_epoch = series
                    .style_index_allocation_epoch
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                let current = relocated(id, expected, expected_epoch)?;
                let base = lane_base(current, next_series_id)?;
                let len =
                    u32::try_from(current.len_values).map_err(|_| GpuPickError::TooManyValues {
                        series_id: next_series_id.to_owned(),
                        count: current.len_values,
                    })?;
                (Some(current), base, len)
            } else {
                (None, 0, 0)
            };

        let (size_handle, size_base, size_len) = if let Some(id) = series.size_column.as_ref() {
            let expected = series.size_handle.ok_or(GpuPickError::InvalidGpuResult)?;
            let expected_epoch = series
                .size_allocation_epoch
                .ok_or(GpuPickError::InvalidGpuResult)?;
            let current = relocated(id, expected, expected_epoch)?;
            let base = lane_base(current, next_series_id)?;
            let len =
                u32::try_from(current.len_values).map_err(|_| GpuPickError::TooManyValues {
                    series_id: next_series_id.to_owned(),
                    count: current.len_values,
                })?;
            (Some(current), base, len)
        } else {
            (None, 0, 0)
        };

        let mut vector_handles = [None; 3];
        let mut vector_base = [0u32; 2];
        if let Some(arrows) = series.vector.as_ref() {
            let ids = [
                Some(&arrows.u_column),
                Some(&arrows.v_column),
                arrows.color_column.as_ref(),
            ];
            for (index, id) in ids.into_iter().enumerate() {
                let Some(id) = id else {
                    continue;
                };
                let expected =
                    series.vector_handles[index].ok_or(GpuPickError::InvalidGpuResult)?;
                let expected_epoch =
                    series.vector_epochs[index].ok_or(GpuPickError::InvalidGpuResult)?;
                let current = relocated(id, expected, expected_epoch)?;
                if index < 2 {
                    vector_base[index] = lane_base(current, next_series_id)?;
                }
                vector_handles[index] = Some(current);
            }
        }

        let mut errorbar_handles = [None; ERRORBAR_COLUMNS];
        if let Some(bars) = series.errorbar.as_ref() {
            for (index, id) in bars.columns.iter().enumerate() {
                let Some(id) = id else {
                    continue;
                };
                let expected = bars.handles[index].ok_or(GpuPickError::InvalidGpuResult)?;
                let expected_epoch = bars.epochs[index].ok_or(GpuPickError::InvalidGpuResult)?;
                errorbar_handles[index] = Some(relocated(id, expected, expected_epoch)?);
            }
        }
        let mut area_handle = None;
        if let Some(fill) = series.area.as_ref()
            && let Some(id) = fill.column.as_ref()
        {
            let expected = fill.handle.ok_or(GpuPickError::InvalidGpuResult)?;
            let expected_epoch = fill.epoch.ok_or(GpuPickError::InvalidGpuResult)?;
            area_handle = Some(relocated(id, expected, expected_epoch)?);
        }

        let ternary_handle = match series.ternary.as_ref() {
            Some(c) => Some(relocated(&c.column, c.handle, c.epoch)?),
            None => None,
        };

        let storage_unchanged = pool.layout_generation() == series.pool_layout_generation
            && same_storage(x_handle, series.x_handle)
            && same_storage(y_handle, series.y_handle)
            && style_index_handle
                .zip(series.style_index_handle)
                .is_none_or(|(current, expected)| same_storage(current, expected))
            && style_index_handle.is_some() == series.style_index_handle.is_some()
            && size_handle
                .zip(series.size_handle)
                .is_none_or(|(current, expected)| same_storage(current, expected))
            && size_handle.is_some() == series.size_handle.is_some()
            && vector_handles
                .iter()
                .zip(&series.vector_handles)
                .all(|(current, expected)| same_optional_storage(*current, *expected))
            && series.errorbar.as_ref().is_none_or(|bars| {
                errorbar_handles
                    .iter()
                    .zip(&bars.handles)
                    .all(|(current, expected)| same_optional_storage(*current, *expected))
            })
            && series
                .area
                .as_ref()
                .is_none_or(|fill| same_optional_storage(area_handle, fill.handle))
            && same_optional_storage(ternary_handle, series.ternary.as_ref().map(|c| c.handle));
        if storage_unchanged {
            return Ok(series.clone());
        }

        let query_data_bg = create_bind_group_checked(
            &self.bundle.device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick rebound query data bg"),
                layout: &self.bundle.query_data_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: pool.buffer().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: series.gate_masks.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: series.workgroup_candidates.as_entire_binding(),
                    },
                ],
            },
            "rebound query data bind group",
        )?;

        let mut rebound = series.clone();
        rebound.pool_identity = Some(target_pool_identity.clone());
        rebound.x_handle = x_handle;
        rebound.y_handle = y_handle;
        rebound.style_index_handle = style_index_handle;
        rebound.pool_layout_generation = pool.layout_generation();
        rebound.style_index_base = style_index_base;
        rebound.style_index_len = style_index_len;
        rebound.size_handle = size_handle;
        rebound.size_base = size_base;
        rebound.size_len = size_len;
        rebound.vector_handles = vector_handles;
        rebound.vector_base = vector_base;
        if let Some(bars) = rebound.errorbar.as_mut() {
            bars.rebase(errorbar_handles, next_series_id)?;
        }
        if let Some(fill) = rebound.area.as_mut()
            && let Some(handle) = area_handle
        {
            fill.params[0] = lane_base(handle, next_series_id)?;
            fill.handle = Some(handle);
        }
        if let Some(c) = rebound.ternary.as_mut()
            && let Some(handle) = ternary_handle
        {
            c.base = lane_base(handle, next_series_id)?;
            c.len = u32::try_from(handle.len_values).unwrap_or(u32::MAX);
            c.handle = handle;
        }
        rebound.query_data_bg = query_data_bg;
        Ok(rebound)
    }

    pub(in crate::gpu_pick) fn prepare_next_registry<'a>(
        &self,
        pool: &ColumnPool,
        final_slots: impl IntoIterator<Item = GpuPickRegistrySlot<'a>>,
    ) -> Result<PickRegistry, GpuPickError> {
        #[cfg(test)]
        if self.fail_next_registry_prepare.replace(false) {
            return Err(GpuPickError::AllocationFailed {
                resource: "injected registry prepare failure",
            });
        }
        let storage_limit = self.bundle.device.limits().max_storage_buffer_binding_size;
        if pool.capacity() > storage_limit {
            return Err(GpuPickError::DeviceLimit {
                resource: "column-pool storage binding",
                requested: pool.capacity(),
                limit: storage_limit,
            });
        }
        let target_pool_identity = self.check_pool_identity(pool)?;

        let iterator = final_slots.into_iter();
        let mut plans = Vec::new();
        plans
            .try_reserve(iterator.size_hint().0)
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry transition plan",
            })?;
        plans.extend(iterator);

        let mut seen = Vec::new();
        seen.try_reserve_exact(self.registry.slots.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry transition index map",
            })?;
        seen.resize(self.registry.slots.len(), false);

        let mut slots = Vec::new();
        slots
            .try_reserve_exact(plans.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry successor slots",
            })?;
        for plan in plans {
            let slot = match plan {
                GpuPickRegistrySlot::Reuse {
                    current_index,
                    source_id,
                    series_id,
                } => {
                    let len = self.registry.slots.len();
                    let Some(current) = self.registry.slots.get(current_index) else {
                        return Err(GpuPickError::InvalidSeriesIndex {
                            index: current_index,
                            len,
                        });
                    };
                    if std::mem::replace(&mut seen[current_index], true) {
                        return Err(GpuPickError::DuplicateSeriesIndex {
                            index: current_index,
                        });
                    }
                    PickSeriesSlot {
                        gpu: self.prepare_reused_gpu(
                            pool,
                            &target_pool_identity,
                            current,
                            &series_id,
                        )?,
                        identity: PickIdentity {
                            source_id,
                            series_id,
                        },
                    }
                }
                GpuPickRegistrySlot::Build(descriptor) => self.prepare_series(pool, descriptor)?,
            };
            slots.push(slot);
        }

        let mut identities = Vec::new();
        identities
            .try_reserve_exact(slots.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry identity snapshot",
            })?;
        identities.extend(slots.iter().map(|slot| slot.identity.clone()));
        let reduce = self.prepare_reduce_resources(slots.len())?;
        let generation = self
            .registry
            .generation
            .checked_add(1)
            .ok_or(GpuPickError::RegistryGenerationExhausted)?;

        Ok(PickRegistry {
            slots,
            identities: identities.into(),
            reduce,
            pool_identity: Some(target_pool_identity),
            pool_layout_generation: pool.layout_generation(),
            generation,
        })
    }

    pub(crate) fn prepare_registry_transition<'engine, 'a>(
        &'engine mut self,
        pool: &ColumnPool,
        final_slots: impl IntoIterator<Item = GpuPickRegistrySlot<'a>>,
    ) -> Result<PreparedPickRegistryTransition<'engine>, GpuPickError> {
        let next = self.prepare_next_registry(pool, final_slots)?;
        Ok(PreparedPickRegistryTransition { engine: self, next })
    }

    /// Replace one registry slot without recreating any other series resources.
    ///
    /// The replacement is built first, so a build/allocation error leaves the
    /// existing slot untouched. The returned id is the unchanged positional
    /// slot; later-series equal-distance precedence is therefore preserved.
    #[cfg(test)]
    pub(in crate::gpu_pick) fn replace_series_at(
        &mut self,
        index: usize,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let len = self.registry.slots.len();
        if index >= len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        let mut final_slots = self.current_reuse_slots(Some(index));
        final_slots.insert(index, GpuPickRegistrySlot::Build(descriptor));
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(GpuPickSeriesId(index as u32))
    }

    /// Insert one registry slot without recreating existing series resources.
    ///
    /// The new series is built completely at the tail before it is moved into
    /// position, so a build/allocation error leaves the registry and its tie
    /// order untouched. An index equal to the registry length appends it.
    #[cfg(test)]
    pub(in crate::gpu_pick) fn insert_series_at(
        &mut self,
        index: usize,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let len = self.registry.slots.len();
        if index > len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        let mut final_slots = self.current_reuse_slots(None);
        final_slots.insert(index, GpuPickRegistrySlot::Build(descriptor));
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(GpuPickSeriesId(index as u32))
    }

    /// Remove one positional registry slot without recreating surviving slots.
    /// Later slots shift left, matching their new production traversal order.
    #[cfg(test)]
    pub(in crate::gpu_pick) fn remove_series_at(
        &mut self,
        index: usize,
    ) -> Result<(), GpuPickError> {
        let len = self.registry.slots.len();
        if index >= len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        let generation = self
            .registry
            .generation
            .checked_add(1)
            .ok_or(GpuPickError::RegistryGenerationExhausted)?;
        let mut slots = self.registry.slots.clone();
        slots.remove(index);
        let identities = slots
            .iter()
            .map(|slot| slot.identity.clone())
            .collect::<Vec<_>>()
            .into();
        let next = PickRegistry {
            slots,
            identities,
            reduce: self.registry.reduce.clone(),
            pool_identity: self.registry.pool_identity.clone(),
            pool_layout_generation: self.registry.pool_layout_generation,
            generation,
        };
        PreparedPickRegistryTransition { engine: self, next }.commit();
        Ok(())
    }

    /// Drop every registered series while retaining reusable pipelines and
    /// reduction storage.
    #[cfg(test)]
    pub(in crate::gpu_pick) fn clear_series(&mut self) {
        let generation = self
            .registry
            .generation
            .checked_add(1)
            .expect("GPU pick registry generation exhausted while clearing");
        let next = PickRegistry {
            slots: Vec::new(),
            identities: Arc::from([]),
            reduce: self.registry.reduce.clone(),
            pool_identity: self.registry.pool_identity.clone(),
            pool_layout_generation: self.registry.pool_layout_generation,
            generation,
        };
        PreparedPickRegistryTransition { engine: self, next }.commit();
    }

    /// Rebind registered columns after an in-place [`ColumnPool::defragment`].
    ///
    /// Defragmentation changes the backing buffer, offsets, and generations,
    /// but preserves every value. This method verifies every referenced column
    /// still exists with exactly its registered logical length, recreates the
    /// whole-pool data bind groups, and updates all handles/bases without
    /// reallocating gate or reduction storage.
    ///
    /// Use this only with the same `ColumnPool` after a relocation-only
    /// defragment. It is not valid after replacing column contents, even when
    /// ids and lengths happen to match; call replace_series_at/rebuild instead.
    #[cfg(test)]
    pub(in crate::gpu_pick) fn rebind_columns(
        &mut self,
        pool: &ColumnPool,
    ) -> Result<(), GpuPickError> {
        let final_slots = self.current_reuse_slots(None);
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(())
    }

    pub(in crate::gpu_pick) fn validate_series_columns(
        pool: &ColumnPool,
        slot: &PickSeriesSlot,
    ) -> Result<(), GpuPickError> {
        let series = &slot.gpu;
        if !series
            .pool_identity
            .as_ref()
            .is_some_and(|identity| identity.same_instance(&pool.identity()))
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        if pool.layout_generation() != series.pool_layout_generation {
            return Err(GpuPickError::StaleColumn {
                series_id: slot.identity.series_id.clone(),
                column_id: series.x_column.clone(),
            });
        }

        for (id, expected, expected_epoch) in [
            (&series.x_column, series.x_handle, series.x_allocation_epoch),
            (&series.y_column, series.y_handle, series.y_allocation_epoch),
        ] {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            };
            if pool.allocation_epoch(id) != Some(expected_epoch) || !same_storage(current, expected)
            {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            }
        }
        for (id, expected, expected_epoch) in [
            (
                series.style_index_column.as_ref(),
                series.style_index_handle,
                series.style_index_allocation_epoch,
            ),
            (
                series.size_column.as_ref(),
                series.size_handle,
                series.size_allocation_epoch,
            ),
            (
                series.vector.as_ref().map(|vector| &vector.u_column),
                series.vector_handles[0],
                series.vector_epochs[0],
            ),
            (
                series.vector.as_ref().map(|vector| &vector.v_column),
                series.vector_handles[1],
                series.vector_epochs[1],
            ),
            (
                series
                    .vector
                    .as_ref()
                    .and_then(|vector| vector.color_column.as_ref()),
                series.vector_handles[2],
                series.vector_epochs[2],
            ),
        ]
        .into_iter()
        .chain(series.errorbar.iter().flat_map(|bars| {
            (0..ERRORBAR_COLUMNS).map(move |index| {
                (
                    bars.columns[index].as_ref(),
                    bars.handles[index],
                    bars.epochs[index],
                )
            })
        }))
        .chain(
            series
                .area
                .iter()
                .map(|fill| (fill.column.as_ref(), fill.handle, fill.epoch)),
        )
        .chain(
            series
                .ternary
                .iter()
                .map(|c| (Some(&c.column), Some(c.handle), Some(c.epoch))),
        )
        .filter_map(|(id, expected, epoch)| Some((id?, expected?, epoch?)))
        {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            };
            if pool.allocation_epoch(id) != Some(expected_epoch) || !same_storage(current, expected)
            {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            }
        }
        Ok(())
    }
}

use super::monitor::buffer_ceiling_from;
use super::transactions::write_scalar_source_as_pairs;
use super::transactions::{ColumnInputMeta, ResidentBatchCapacityPlan};
use super::*;
use crate::data::Column;

// The pool binds its whole primary buffer as one storage binding, so both
// device ceilings apply. Before this was centralized the pool knew only
// `max_buffer_size`, which let a buffer legal by that number alone exceed
// `max_storage_buffer_binding_size` and fail later inside wgpu, on the
// first draw that binds the pool as storage.

#[test]
fn storage_ceiling_wins_when_it_is_the_tighter_one() {
    assert_eq!(buffer_ceiling_from(256 << 20, 128 << 20), 128 << 20);
}

#[test]
fn buffer_ceiling_wins_when_it_is_the_tighter_one() {
    assert_eq!(buffer_ceiling_from(64 << 20, 128 << 20), 64 << 20);
}

#[test]
fn equal_ceilings_are_that_value() {
    assert_eq!(buffer_ceiling_from(128 << 20, 128 << 20), 128 << 20);
}

#[test]
fn resident_batch_capacity_plan_is_pure_and_obeys_fragmentation_policy() {
    let Some((device, queue, mut pool)) = mk_pool(3 * ALIGN) else {
        return;
    };
    // A fixed pool: this test is about what a pool that cannot grow does.
    pool.growth_policy = GrowthPolicy::Fixed;
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    for id in ["a", "b", "c"] {
        pool.add_column(id.into(), &col_f64(vec![1.0]), ctx)
            .unwrap();
    }
    pool.remove_column("a").unwrap();
    pool.remove_column("c").unwrap();
    assert_eq!(pool.free_bytes(), 2 * ALIGN);
    assert_eq!(pool.largest_free_region(), ALIGN);
    let before_capacity = pool.capacity();
    let before_used = pool.used_bytes();
    let ceiling = buffer_ceiling(&device);

    assert_eq!(
        pool.plan_resident_batch_capacity(2 * ALIGN, ceiling),
        Err(ResidentBatchCapacityError::Policy)
    );
    pool.defrag_policy = DefragPolicy::OnAllocFailure;
    assert_eq!(
        pool.plan_resident_batch_capacity(2 * ALIGN, ceiling),
        Ok(ResidentBatchCapacityPlan {
            capacity_after: 3 * ALIGN,
            transition_bytes: 3 * ALIGN,
        })
    );
    assert_eq!(pool.capacity(), before_capacity);
    assert_eq!(pool.used_bytes(), before_used);
    assert_eq!(pool.free_bytes(), 2 * ALIGN);
    assert_eq!(pool.largest_free_region(), ALIGN);
}

// ---- growth (begin_relayout with a larger target) ----

#[test]
fn growth_preserves_every_byte_and_republishes_capacity() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    let values = vec![1.5_f64, -2.5, 3.0, 0.25];
    pool.add_column("x".into(), &col_f64(values.clone()), ctx)
        .expect("first column fits");
    let before = read_column_values(ctx, &pool, pool.handle_for("x").expect("x present"));

    let grown = pool
        .grow_to(ctx, 8 * ALIGN)
        .expect("growth within the device ceiling");
    assert!(grown, "growth relocates, so it reports a change");
    assert_eq!(pool.capacity(), 8 * ALIGN, "new capacity is published");
    assert_eq!(
        read_column_values(ctx, &pool, pool.handle_for("x").expect("x present")),
        before,
        "growth copies bytes, it does not rewrite them"
    );
}

#[test]
fn growth_invalidates_old_handles_and_requires_refetch() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    let stale = pool
        .add_column("x".into(), &col_f64(vec![1.0, 2.0]), ctx)
        .expect("first column fits");
    let layout_before = pool.layout_generation();

    pool.grow_to(ctx, 8 * ALIGN).expect("growth");

    assert!(
        !pool.is_valid_handle(&stale),
        "a handle taken before growth must not be reused"
    );
    assert!(
        pool.handle_for("x").is_some(),
        "the column is still there under the same id"
    );
    assert!(
        pool.layout_generation() > layout_before,
        "layout generation must advance so live PreparedFrames go stale"
    );
}

#[test]
fn growth_releases_the_old_buffer_instead_of_keeping_it_as_backup() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    // A backup only appears when a defrag actually relocates, so leave a
    // gap first: an already-packed pool returns early without allocating.
    pool.add_column("a".into(), &col_f64(vec![1.0]), ctx)
        .expect("first column fits");
    pool.add_column("b".into(), &col_f64(vec![2.0]), ctx)
        .expect("second column fits");
    pool.remove_column("a")
        .expect("remove leaves a gap at offset 0");
    pool.defragment(ctx)
        .expect("defrag relocates b into the gap");
    assert!(pool.backup.is_some(), "defrag leaves a backup to ping-pong");

    pool.grow_to(ctx, 8 * ALIGN).expect("growth");

    assert!(
        pool.backup.is_none(),
        "a backup sized for the old capacity cannot serve the new one"
    );
}

#[test]
fn growth_beyond_the_budget_is_refused_and_leaves_the_pool_intact() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let unbudgeted = GpuAllocCtx::unbudgeted(&device, &queue);
    pool.add_column("x".into(), &col_f64(vec![1.0, 2.0]), unbudgeted)
        .expect("first column fits");
    let before = read_column_values(unbudgeted, &pool, pool.handle_for("x").expect("x present"));
    let capacity_before = pool.capacity();
    let layout_before = pool.layout_generation();

    // Peak is old + new; a budget just under that must refuse.
    let target = 8 * ALIGN;
    let budget = capacity_before + target - 1;
    let budgeted = GpuAllocCtx {
        device: &device,
        queue: &queue,
        budget: Some(GpuBudget {
            ceiling_bytes: budget,
            external_bytes: 0,
        }),
    };
    let error = pool
        .grow_to(budgeted, target)
        .expect_err("growth past the budget must be refused");
    match error {
        AllocError::BudgetExceeded {
            requested, limit, ..
        } => {
            assert_eq!(limit, budget);
            assert!(requested > limit);
        }
        other => panic!("expected BudgetExceeded, got {other:?}"),
    }
    assert_eq!(pool.capacity(), capacity_before, "capacity unchanged");
    assert_eq!(
        pool.layout_generation(),
        layout_before,
        "a refused growth must not advance the layout"
    );
    assert_eq!(
        read_column_values(unbudgeted, &pool, pool.handle_for("x").expect("x present")),
        before,
        "data survives a refused growth"
    );
}

#[test]
fn external_bytes_count_against_the_same_budget() {
    let Some((device, queue, pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let target = 8 * ALIGN;
    let peak = pool.capacity() + target;
    // Exactly enough for the buffers alone...
    let tight = GpuAllocCtx {
        device: &device,
        queue: &queue,
        budget: Some(GpuBudget {
            ceiling_bytes: peak,
            external_bytes: 0,
        }),
    };
    assert!(
        pool.plan_relayout_capacity(tight, target).is_ok(),
        "peak equal to the budget is allowed"
    );
    // ...but one byte held outside the pool tips it over.
    let with_external = GpuAllocCtx {
        device: &device,
        queue: &queue,
        budget: Some(GpuBudget {
            ceiling_bytes: peak,
            external_bytes: 1,
        }),
    };
    assert!(
        pool.plan_relayout_capacity(with_external, target).is_err(),
        "bytes held outside the pool must count against the budget"
    );
}

#[test]
fn pool_retirement_is_budgeted_until_its_exact_batch_completes() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let target = 8 * ALIGN;
    let ctx = GpuAllocCtx {
        device: &device,
        queue: &queue,
        budget: Some(GpuBudget {
            ceiling_bytes: pool.gpu_bytes() + target,
            external_bytes: 0,
        }),
    };
    pool.note_buffer_retired(ALIGN);
    let (counter, first) = pool.take_retirement();
    assert_eq!(pool.retired_bytes(), ALIGN);
    assert!(pool.plan_relayout_capacity(ctx, target).is_err());
    pool.note_buffer_retired(2 * ALIGN);
    counter.complete(first);
    assert_eq!(pool.retired_bytes(), 2 * ALIGN);
    assert!(pool.plan_relayout_capacity(ctx, target).is_err());
    let (counter, second) = pool.take_retirement();
    counter.complete(second);
    assert_eq!(pool.retired_bytes(), 0);
    assert!(pool.plan_relayout_capacity(ctx, target).is_ok());
}

#[test]
fn dropping_a_growth_guard_restores_capacity_and_the_old_buffer() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    pool.add_column("x".into(), &col_f64(vec![1.0, 2.0]), ctx)
        .expect("first column fits");
    let handle_before = pool.handle_for("x").expect("x present");
    let before = read_column_values(ctx, &pool, handle_before);
    let capacity_before = pool.capacity();
    let layout_before = pool.layout_generation();

    {
        let guard = pool
            .begin_relayout(ctx, 8 * ALIGN)
            .expect("growth prepares");
        assert_eq!(
            guard.pool().capacity(),
            8 * ALIGN,
            "guard shows the new size"
        );
        // Dropped without commit.
    }

    assert_eq!(
        pool.capacity(),
        capacity_before,
        "an abandoned growth must un-publish the new capacity"
    );
    assert_eq!(pool.layout_generation(), layout_before);
    assert!(
        pool.is_valid_handle(&handle_before),
        "the pre-growth handle is valid again after rollback"
    );
    assert_eq!(
        read_column_values(ctx, &pool, pool.handle_for("x").expect("x present")),
        before,
        "rollback restores the original buffer, bytes intact"
    );
}

#[test]
fn growth_beyond_the_device_ceiling_is_refused_and_leaves_the_pool_intact() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    let handle = pool
        .add_column("keep".into(), &col_f64(vec![1.0, 2.0, 3.0]), ctx)
        .expect("first column fits");
    let capacity = pool.capacity();
    let layout_generation = pool.layout_generation();

    let ceiling = buffer_ceiling(&device);
    let over = ceiling
        .checked_add(ALIGN)
        .expect("ceiling + ALIGN fits u64");
    let Err(error) = pool.grow_to(ctx, over) else {
        panic!("a target above the device ceiling must be refused");
    };
    match error {
        AllocError::ResourceLimit {
            requested, limit, ..
        } => {
            assert_eq!(requested, over);
            assert_eq!(limit, ceiling, "the combined ceiling is the limit");
        }
        other => panic!("expected ResourceLimit, got {other:?}"),
    }

    // Refused before anything was published: same capacity, same layout
    // generation, same handle, same bytes.
    assert_eq!(pool.capacity(), capacity);
    assert_eq!(pool.layout_generation(), layout_generation);
    let refetched = pool.handle_for("keep").expect("column still registered");
    assert_eq!(refetched.offset, handle.offset);
    assert_eq!(refetched.generation, handle.generation);
    assert_eq!(refetched.byte_size, handle.byte_size);
    assert_eq!(refetched.len_values, handle.len_values);
    let stored = read_column_values(ctx, &pool, handle);
    assert_eq!(stored, vec![1.0, 2.0, 3.0]);
}

#[test]
fn growth_peak_is_the_old_and_new_buffers_and_settles_to_the_new_one() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    pool.add_column("keep".into(), &col_f64(vec![1.0, 2.0]), ctx)
        .expect("first column fits");
    let before = pool.capacity();
    assert_eq!(
        pool.gpu_bytes(),
        before,
        "no backup yet, so the footprint is the slab"
    );
    pool.reset_peak_bytes();

    pool.grow_to(ctx, 8 * ALIGN)
        .expect("growth fits the device");
    let after = pool.capacity();
    assert!(after >= 8 * ALIGN);

    // `commit` released the old slab, so the held footprint is the new
    // slab alone. The preceding upload staging and the old slab both stay
    // retired until completion, and the peak includes all three buffers.
    assert_eq!(
        pool.gpu_bytes(),
        after,
        "the old buffer must be gone once the growth committed"
    );
    assert_eq!(
        pool.peak_bytes(),
        ALIGN + before + after,
        "peak must include submitted staging plus old and new slabs"
    );
    assert_eq!(
        pool.retired_bytes(),
        ALIGN + before,
        "submitted staging and the old slab count until queue completion"
    );
    let (retired, bytes) = pool.take_retirement();
    queue.on_submitted_work_done(move || retired.complete(bytes));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("retired pool copy completes");
    assert_eq!(pool.retired_bytes(), 0);
}

#[test]
fn relayout_shrinks_without_changing_live_values() {
    let Some((device, queue, mut pool)) = mk_pool(8 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    let source = col_f64(vec![1.0, 2.0, 3.0]);
    pool.add_column("keep".into(), &source, ctx).unwrap();
    let stale = pool.handle_for("keep").unwrap();
    assert!(matches!(
        pool.grow_to(ctx, 2 * ALIGN),
        Err(AllocError::AllocationFailed { .. })
    ));
    assert!(pool.begin_relayout(ctx, 2 * ALIGN).unwrap().commit());
    assert_eq!(pool.capacity(), 2 * ALIGN);
    assert_eq!(pool.backup_bytes(), 0);
    assert!(!pool.is_valid_handle(&stale));
    assert_eq!(
        read_column_values(ctx, &pool, pool.handle_for("keep").unwrap()),
        source.data
    );
    assert!(pool.retired_bytes() >= 8 * ALIGN);
}

#[test]
fn refused_shrink_preserves_primary_and_handle() {
    let Some((device, queue, mut pool)) = mk_pool(8 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    let source = col_f64(vec![7.0, 8.0]);
    pool.add_column("keep".into(), &source, ctx).unwrap();
    let handle = pool.handle_for("keep").unwrap();
    let target = 2 * ALIGN;
    let budgeted = GpuAllocCtx {
        device: &device,
        queue: &queue,
        budget: Some(GpuBudget {
            ceiling_bytes: pool.gpu_bytes() + target - 1,
            external_bytes: 0,
        }),
    };
    assert!(matches!(
        pool.begin_relayout(budgeted, target),
        Err(AllocError::BudgetExceeded { .. })
    ));
    assert_eq!(pool.capacity(), 8 * ALIGN);
    assert!(pool.is_valid_handle(&handle));
    assert_eq!(read_column_values(ctx, &pool, handle), source.data);
}

#[test]
fn same_capacity_relayout_is_the_existing_defragment() {
    let Some((device, queue, mut pool)) = mk_pool(8 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    pool.add_column("a".into(), &col_f64(vec![1.0]), ctx)
        .unwrap();
    pool.add_column("b".into(), &col_f64(vec![2.0]), ctx)
        .unwrap();
    pool.remove_column("a").unwrap();
    let capacity = pool.capacity();

    let relayout = pool.grow_to(ctx, capacity).expect("same-capacity relayout");

    assert!(relayout, "a gap was packed, so it reports a change");
    assert_eq!(pool.capacity(), capacity, "capacity is untouched");
    assert_eq!(pool.handle_for("b").map(|h| h.offset), Some(0));
}

#[test]
fn fixed_growth_policy_keeps_out_of_space_final() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    assert_eq!(
        pool.growth_policy,
        GrowthPolicy::OnAllocFailure,
        "default is OnAllocFailure (#107)"
    );
    pool.growth_policy = GrowthPolicy::Fixed;
    let too_big = col_f64(vec![0.0; (4 * ALIGN / COLUMN_VALUE_BYTES as u64) as usize]);

    let error = pool
        .add_column("big".into(), &too_big, ctx)
        .expect_err("a column larger than the pool cannot fit");

    assert!(
        matches!(error, AllocError::OutOfSpace { .. }),
        "with growth off the answer stays OutOfSpace, got {error:?}"
    );
    assert_eq!(pool.capacity(), 2 * ALIGN, "pool did not grow");
}

#[test]
fn upsert_path_growth_makes_an_oversized_upload_fit() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    pool.growth_policy = GrowthPolicy::OnAllocFailure;
    let n = (4 * ALIGN / COLUMN_VALUE_BYTES as u64) as usize;
    let values: Vec<f64> = (0..n).map(|i| i as f64).collect();

    // `upsert_column` is the route every renderer/host upload takes, so
    // growth has to be reachable from here and not only from `add_column`.
    let handle = pool
        .upsert_column("big".into(), &col_f64(values), ctx)
        .expect("the upsert path grows too");

    assert!(pool.capacity() >= 4 * ALIGN, "pool enlarged");
    assert_eq!(handle.len_values, n);
    let stored = read_column_values(ctx, &pool, pool.handle_for("big").expect("big present"));
    assert_eq!(stored.len(), n);
    assert_eq!(stored[n - 1], (n - 1) as f64);
}

#[test]
fn upsert_growth_stays_out_of_the_way_when_packing_is_enough() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    pool.growth_policy = GrowthPolicy::OnAllocFailure;
    let per_column = (ALIGN / COLUMN_VALUE_BYTES as u64) as usize;
    let block: Vec<f64> = (0..per_column).map(|i| i as f64).collect();
    for id in ["a", "b", "c", "d"] {
        pool.add_column(id.into(), &col_f64(block.clone()), ctx)
            .expect("four aligned columns fill the pool exactly");
    }
    let capacity = pool.capacity();

    // Replacing one column in place needs no new bytes; growing here would
    // inflate bytes-per-value for every host that overwrites a column.
    pool.upsert_column("b".into(), &col_f64(block.clone()), ctx)
        .expect("same-size replacement fits without growth");
    assert_eq!(
        pool.capacity(),
        capacity,
        "in-place replacement must not grow"
    );

    // Two freed slots, fragmented: compaction covers a double-size column,
    // so growth must still stay out of it.
    pool.remove_column("a").expect("remove a");
    pool.remove_column("c").expect("remove c");
    pool.defrag_policy = DefragPolicy::OnAllocFailure;
    let double: Vec<f64> = (0..(per_column * 2)).map(|i| i as f64).collect();
    pool.upsert_column("wide".into(), &col_f64(double), ctx)
        .expect("compaction gathers the two freed slots");
    assert_eq!(
        pool.capacity(),
        capacity,
        "compaction was enough; the pool must not have grown"
    );
}

#[test]
fn growth_policy_on_alloc_failure_makes_the_upload_fit() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
    pool.growth_policy = GrowthPolicy::OnAllocFailure;
    let n = (4 * ALIGN / COLUMN_VALUE_BYTES as u64) as usize;
    let values: Vec<f64> = (0..n).map(|i| i as f64).collect();

    let handle = pool
        .add_column("big".into(), &col_f64(values.clone()), ctx)
        .expect("growth makes room for a column the pool could not hold");

    assert!(pool.capacity() >= 4 * ALIGN, "pool enlarged");
    assert_eq!(handle.len_values, n);
    let stored = read_column_values(ctx, &pool, pool.handle_for("big").expect("big present"));
    assert_eq!(stored.len(), n);
    assert_eq!(stored[0], 0.0);
    assert_eq!(stored[n - 1], (n - 1) as f64);
}

#[test]
fn pool_refuses_capacity_above_the_device_ceiling() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        eprintln!("no GPU adapter; skipping device ceiling check");
        return;
    };
    let ceiling = buffer_ceiling(&device);
    let over = ceiling
        .checked_add(ALIGN)
        .expect("ceiling + ALIGN fits u64");
    let Err(error) = ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), over) else {
        panic!("capacity above the ceiling must be refused, not created");
    };
    match error {
        AllocError::ResourceLimit {
            requested, limit, ..
        } => {
            assert!(requested > limit, "requested {requested} vs limit {limit}");
            assert_eq!(limit, ceiling, "limit must be the combined ceiling");
        }
        other => panic!("expected ResourceLimit, got {other:?}"),
    }
}

struct RawScalarSource<'a> {
    bits: &'a [u32],
}

impl ColumnSource for RawScalarSource<'_> {
    fn len(&self) -> usize {
        self.bits.len()
    }

    fn min(&self) -> f64 {
        0.0
    }

    fn max(&self) -> f64 {
        0.0
    }

    fn write_f32_le_into(&self, dst: &mut [u8]) {
        assert_eq!(dst.len(), self.bits.len() * std::mem::size_of::<f32>());
        for (chunk, bits) in dst.chunks_exact_mut(4).zip(self.bits) {
            chunk.copy_from_slice(&bits.to_le_bytes());
        }
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        assert_eq!(dst.len(), self.bits.len());
        let mut min_positive: Option<f64> = None;
        for (index, bits) in self.bits.iter().enumerate() {
            let value = f32::from_bits(*bits);
            dst.write_pair(index, value, 0.0);
            if value.is_finite()
                && value > 0.0
                && match min_positive {
                    Some(current) => (value as f64) < current,
                    None => true,
                }
            {
                min_positive = Some(value as f64);
            }
        }
        ColumnUploadStats { min_positive }
    }
}

struct PanickingScalarSource;

impl ColumnSource for PanickingScalarSource {
    fn len(&self) -> usize {
        1
    }

    fn min(&self) -> f64 {
        1.0
    }

    fn max(&self) -> f64 {
        1.0
    }

    fn write_f32_le_into(&self, _dst: &mut [u8]) {
        panic!("injected scalar writer panic");
    }

    fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
        dst[0] = 0xa5;
        panic!("injected scalar writer panic");
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        dst.write_pair(0, 9.0, 0.0);
        panic!("injected scalar writer panic");
    }
}

struct PanickingHiLoSource;

impl HiLoColumnSource for PanickingHiLoSource {
    fn len(&self) -> usize {
        1
    }

    fn min(&self) -> f64 {
        1.0
    }

    fn max(&self) -> f64 {
        1.0
    }

    fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
        dst[0] = 0x5a;
        panic!("injected hi/lo writer panic");
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        dst.write_pair(0, 9.0, 0.25);
        panic!("injected hi/lo writer panic");
    }
}

fn free_state(free: &[FreeRegion]) -> Vec<(u64, u64)> {
    free.iter()
        .map(|region| (region.offset, region.size))
        .collect()
}

#[test]
fn region_reservation_restores_or_commits_exact_free_list_edit() {
    let original = vec![
        FreeRegion {
            offset: 0,
            size: ALIGN,
        },
        FreeRegion {
            offset: 3 * ALIGN,
            size: 2 * ALIGN,
        },
        FreeRegion {
            offset: 8 * ALIGN,
            size: ALIGN,
        },
    ];

    let mut exact = original.clone();
    {
        let reservation = alloc_region(&mut exact, ALIGN).unwrap();
        assert_eq!(reservation.offset(), 0);
    }
    assert_eq!(free_state(&exact), free_state(&original));

    let reservation = alloc_region(&mut exact, ALIGN).unwrap();
    assert_eq!(reservation.offset(), 0);
    reservation.commit();
    assert_eq!(
        free_state(&exact),
        vec![(3 * ALIGN, 2 * ALIGN), (8 * ALIGN, ALIGN)]
    );

    let mut split = vec![
        FreeRegion {
            offset: ALIGN,
            size: 3 * ALIGN,
        },
        FreeRegion {
            offset: 8 * ALIGN,
            size: ALIGN,
        },
    ];
    let split_original = free_state(&split);
    {
        let reservation = alloc_region(&mut split, ALIGN).unwrap();
        assert_eq!(reservation.offset(), ALIGN);
    }
    assert_eq!(free_state(&split), split_original);

    let reservation = alloc_region(&mut split, ALIGN).unwrap();
    assert_eq!(reservation.offset(), ALIGN);
    reservation.commit();
    assert_eq!(
        free_state(&split),
        vec![(2 * ALIGN, 2 * ALIGN), (8 * ALIGN, ALIGN)]
    );
}

#[test]
fn scalar_source_expands_to_pairs_without_changing_bits() {
    let bits = [
        0x0000_0000,
        0x8000_0000,
        0x3f80_0001,
        0x7fc1_2345,
        0x7f80_0000,
        0xff80_0000,
    ];
    let mut dst = vec![0xa5; bits.len() * COLUMN_VALUE_BYTES];
    let source = RawScalarSource { bits: &bits };

    let stats = write_scalar_source_as_pairs(
        &source,
        ColumnPairWriter::from_bytes_for_test(dst.as_mut_slice()),
    );

    assert_eq!(stats.min_positive, Some(f32::from_bits(0x3f80_0001) as f64));
    for (pair, bits) in dst.chunks_exact(COLUMN_VALUE_BYTES).zip(bits) {
        assert_eq!(&pair[..4], &bits.to_le_bytes());
        assert_eq!(&pair[4..], &0.0f32.to_le_bytes());
    }
}

#[test]
fn scalar_source_expansion_accepts_empty_input() {
    let source = RawScalarSource { bits: &[] };
    let mut dst = [];

    let stats = write_scalar_source_as_pairs(
        &source,
        ColumnPairWriter::from_bytes_for_test(dst.as_mut_slice()),
    );

    assert!(dst.is_empty());
    assert_eq!(stats.min_positive, None);
}

fn mk_pool(
    cap: u64,
) -> Option<(
    std::sync::Arc<wgpu::Device>,
    std::sync::Arc<wgpu::Queue>,
    ColumnPool,
)> {
    // Shared device across the test binary — see `data_render::shared_device`.
    let (device, queue) = crate::data_render::shared_device()?;
    let pool = ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), cap)
        .expect("required GPU test: pool initialization failed");
    Some((device, queue, pool))
}

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

#[test]
fn independently_created_pools_have_distinct_identities() {
    let Some((device, queue, pool_a)) = mk_pool(ALIGN) else {
        return;
    };
    let pool_b = ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), ALIGN).unwrap();

    let identity_a = pool_a.identity();
    assert_eq!(identity_a, identity_a.clone());
    assert_ne!(identity_a, pool_b.identity());
}

#[test]
fn pool_identity_survives_upsert_drop_commit_and_defragmentation() {
    let Some((device, queue, mut pool)) = mk_pool(3 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0]);
    for id in ["identity-a", "identity-b", "identity-c"] {
        pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
    }
    let identity = pool.identity();

    {
        let replacement = col_f64(vec![2.0]);
        let pending = pool
            .begin_upsert_column(
                "identity-a".into(),
                &replacement,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(pending.pool().identity(), identity);
    }
    assert_eq!(pool.identity(), identity);

    let replacement = col_f64(vec![3.0]);
    let pending = pool
        .begin_upsert_column(
            "identity-a".into(),
            &replacement,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(pending.pool().identity(), identity);
    pending.commit();
    assert_eq!(pool.identity(), identity);

    assert!(pool.remove_column("identity-b").unwrap());
    assert!(
        pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap()
    );
    assert_eq!(pool.identity(), identity);
}

#[derive(Debug, PartialEq)]
struct SlotState {
    offset: u64,
    byte_size: u64,
    len_values: usize,
    generation: u32,
    min_bits: u64,
    max_bits: u64,
    min_positive_bits: Option<u64>,
}

#[derive(Debug, PartialEq)]
struct HandleState {
    generation: u32,
    offset: u64,
    byte_size: u64,
    len_values: usize,
}

#[derive(Debug, PartialEq)]
struct NamedSlotState {
    id: ColumnId,
    slot: SlotState,
}

#[derive(Debug, PartialEq)]
struct PoolState {
    buffer: u64,
    backup: Option<u64>,
    generation: u32,
    layout_generation: u64,
    allocation_epoch_counter: u64,
    allocation_epochs: Vec<(ColumnId, u64)>,
    used: u64,
    free_bytes: u64,
    free: Vec<(u64, u64)>,
    slot: Option<SlotState>,
    handle: Option<HandleState>,
}

fn buffer_identity(buffer: &Buffer) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(buffer, &mut hasher);
    std::hash::Hasher::finish(&hasher)
}

fn pool_state(pool: &ColumnPool, id: &str) -> PoolState {
    let mut allocation_epochs = pool
        .allocation_epochs
        .iter()
        .map(|(id, epoch)| (id.clone(), *epoch))
        .collect::<Vec<_>>();
    allocation_epochs.sort_by(|a, b| a.0.cmp(&b.0));

    PoolState {
        buffer: buffer_identity(pool.buffer()),
        backup: pool.backup.as_ref().map(buffer_identity),
        generation: pool.generation(),
        layout_generation: pool.layout_generation(),
        allocation_epoch_counter: pool.allocation_epoch_counter,
        allocation_epochs,
        used: pool.used_bytes(),
        free_bytes: pool.free_bytes(),
        free: pool
            .free
            .iter()
            .map(|region| (region.offset, region.size))
            .collect(),
        slot: pool.slot(id).map(|slot| SlotState {
            offset: slot.offset,
            byte_size: slot.byte_size,
            len_values: slot.len_values,
            generation: slot.generation,
            min_bits: slot.min.to_bits(),
            max_bits: slot.max.to_bits(),
            min_positive_bits: slot.min_positive.map(f64::to_bits),
        }),
        handle: pool.handle_for(id).map(|handle| HandleState {
            generation: handle.generation,
            offset: handle.offset,
            byte_size: handle.byte_size,
            len_values: handle.len_values,
        }),
    }
}

#[derive(Debug, PartialEq)]
struct FullPoolState {
    identity: PoolIdentity,
    base: PoolState,
    slots: Vec<NamedSlotState>,
}

fn full_pool_state(pool: &ColumnPool) -> FullPoolState {
    let mut slots = pool
        .slots
        .values()
        .map(|slot| NamedSlotState {
            id: slot.id.clone(),
            slot: SlotState {
                offset: slot.offset,
                byte_size: slot.byte_size,
                len_values: slot.len_values,
                generation: slot.generation,
                min_bits: slot.min.to_bits(),
                max_bits: slot.max.to_bits(),
                min_positive_bits: slot.min_positive.map(f64::to_bits),
            },
        })
        .collect::<Vec<_>>();
    slots.sort_by(|left, right| left.id.cmp(&right.id));
    FullPoolState {
        identity: pool.identity(),
        base: pool_state(pool, "demo_x"),
        slots,
    }
}

fn demo_batch_columns(sources: [&dyn ColumnSource; 4]) -> [(ColumnId, &dyn ColumnSource); 4] {
    let ids = ["demo_x", "demo_sin", "demo_t", "demo_rc"];
    std::array::from_fn(|index| (ids[index].to_string(), sources[index]))
}

#[test]
fn demo_batch_capacity_failures_at_each_replacement_preserve_live_pool() {
    let Some((device, queue, _)) = mk_pool(4 * ALIGN) else {
        return;
    };
    for failed_index in 0..4 {
        let mut pool =
            ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), 4 * ALIGN).unwrap();
        let before = full_pool_state(&pool);
        let mut columns = Vec::new();
        for index in 0..4 {
            let aligned_regions = if index == failed_index {
                5usize - failed_index
            } else {
                1
            };
            columns.push(col_f64(vec![1.0; aligned_regions * ALIGN as usize / 8]));
        }
        let sources: [&dyn ColumnSource; 4] = [&columns[0], &columns[1], &columns[2], &columns[3]];
        assert!(matches!(
            pool.begin_demo_batch_upsert(
                demo_batch_columns(sources),
                GpuAllocCtx::unbudgeted(&device, &queue),
            ),
            Err(AllocError::OutOfSpace { .. })
        ));
        assert_eq!(full_pool_state(&pool), before, "replacement {failed_index}");
    }
}

#[test]
fn demo_batch_epoch_failures_at_each_replacement_preserve_live_pool() {
    let Some((device, queue, _)) = mk_pool(8 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0, 2.0]);
    for failed_index in 0..4 {
        let mut pool =
            ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), 8 * ALIGN).unwrap();
        pool.add_column(
            "survivor".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.allocation_epoch_counter = u64::MAX - failed_index as u64;
        let before = full_pool_state(&pool);
        let sources: [&dyn ColumnSource; 4] = [&values, &values, &values, &values];
        let error = match pool.begin_demo_batch_upsert(
            demo_batch_columns(sources),
            GpuAllocCtx::unbudgeted(&device, &queue),
        ) {
            Ok(_) => panic!("replacement {failed_index} epoch must fail"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            AllocError::CounterExhausted {
                counter: "allocation epoch",
            }
        );
        assert_eq!(full_pool_state(&pool), before, "replacement {failed_index}");
    }
}

#[test]
fn demo_batch_generation_and_layout_overflow_preserve_live_pool() {
    let Some((device, queue, _)) = mk_pool(8 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0, 2.0]);
    for counter in ["public generation", "layout generation"] {
        let mut pool =
            ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), 8 * ALIGN).unwrap();
        pool.add_column(
            "survivor".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        if counter == "public generation" {
            pool.generation = u32::MAX;
        } else {
            pool.layout_generation = u64::MAX;
        }
        let before = full_pool_state(&pool);
        let sources: [&dyn ColumnSource; 4] = [&values, &values, &values, &values];
        let error = match pool.begin_demo_batch_upsert(
            demo_batch_columns(sources),
            GpuAllocCtx::unbudgeted(&device, &queue),
        ) {
            Ok(_) => panic!("{counter} must reject overflow"),
            Err(error) => error,
        };
        assert_eq!(error, AllocError::CounterExhausted { counter });
        assert_eq!(full_pool_state(&pool), before);
    }
}

#[test]
fn demo_batch_drop_preserves_fragmented_pool_with_existing_backup() {
    let Some((device, queue, mut pool)) = mk_pool(10 * ALIGN) else {
        return;
    };
    let old = col_f64(vec![1.0, 2.0]);
    for id in ["survivor-a", "demo_x", "hole", "demo_rc", "survivor-b"] {
        pool.add_column(id.into(), &old, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
    }
    assert!(pool.remove_column("hole").unwrap());
    assert!(
        pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap()
    );
    assert!(pool.backup.is_some());
    assert!(pool.remove_column("demo_x").unwrap());
    let before = full_pool_state(&pool);
    let retired_before = pool.retired_bytes();
    let survivor_a = read_column_values(
        GpuAllocCtx::unbudgeted(&device, &queue),
        &pool,
        pool.handle_for("survivor-a").unwrap(),
    );
    let replacements = [
        col_f64(vec![10.0, 11.0]),
        col_f64(vec![20.0, 21.0]),
        col_f64(vec![30.0, 31.0]),
        col_f64(vec![40.0, 41.0]),
    ];
    {
        let sources: [&dyn ColumnSource; 4] = [
            &replacements[0],
            &replacements[1],
            &replacements[2],
            &replacements[3],
        ];
        let pending = pool
            .begin_demo_batch_upsert(
                demo_batch_columns(sources),
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_ne!(buffer_identity(pending.pool().buffer()), before.base.buffer);
    }
    assert_eq!(full_pool_state(&pool), before);
    assert_eq!(
        pool.retired_bytes(),
        retired_before + pool.capacity() + 4 * ALIGN,
        "abandoned candidate and four submitted staging buffers stay budgeted"
    );
    assert_eq!(
        read_column_values(
            GpuAllocCtx::unbudgeted(&device, &queue),
            &pool,
            pool.handle_for("survivor-a").unwrap(),
        ),
        survivor_a
    );
}

#[test]
fn demo_batch_commit_publishes_complete_packed_pool_and_preserves_survivor_epochs() {
    let Some((device, queue, mut pool)) = mk_pool(10 * ALIGN) else {
        return;
    };
    let old = col_f64(vec![1.0, 2.0]);
    for id in ["survivor", "demo_x", "hole", "demo_sin"] {
        pool.add_column(id.into(), &old, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
    }
    assert!(pool.remove_column("hole").unwrap());
    assert!(
        pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap()
    );
    let before = full_pool_state(&pool);
    let primary = buffer_identity(pool.buffer());
    let old_backup = pool.backup.as_ref().map(buffer_identity);
    let generation = pool.generation();
    let layout_generation = pool.layout_generation();
    let retired_before = pool.retired_bytes();
    let survivor_epoch = pool.allocation_epoch("survivor").unwrap();
    let old_demo_x_epoch = pool.allocation_epoch("demo_x").unwrap();
    let replacements = [
        col_f64(vec![10.0, 11.0]),
        col_f64(vec![20.0, 21.0]),
        col_f64(vec![30.0, 31.0]),
        col_f64(vec![40.0, 41.0]),
    ];
    let sources: [&dyn ColumnSource; 4] = [
        &replacements[0],
        &replacements[1],
        &replacements[2],
        &replacements[3],
    ];
    let handles = pool
        .begin_demo_batch_upsert(
            demo_batch_columns(sources),
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap()
        .commit();

    let committed = full_pool_state(&pool);
    assert_eq!(committed.identity, before.identity);
    assert_eq!(pool.generation(), generation + 1);
    assert_eq!(pool.layout_generation(), layout_generation + 1);
    assert_eq!(pool.backup.as_ref().map(buffer_identity), Some(primary));
    assert_ne!(pool.backup.as_ref().map(buffer_identity), old_backup);
    assert_eq!(
        pool.retired_bytes(),
        retired_before + pool.capacity() + 4 * ALIGN,
        "the superseded backup and four staging buffers stay budgeted"
    );
    assert_eq!(pool.allocation_epoch("survivor"), Some(survivor_epoch));
    assert_ne!(pool.allocation_epoch("demo_x"), Some(old_demo_x_epoch));
    let replacement_epochs =
        ["demo_x", "demo_sin", "demo_t", "demo_rc"].map(|id| pool.allocation_epoch(id).unwrap());
    assert!(replacement_epochs.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(pool.free.len(), 1);
    assert_eq!(pool.free[0].offset, pool.used_bytes());
    for (index, id) in ["demo_x", "demo_sin", "demo_t", "demo_rc"]
        .iter()
        .enumerate()
    {
        assert_eq!(pool.handle_for(id).unwrap().offset, handles[index].offset);
        assert_eq!(
            read_column_values(
                GpuAllocCtx::unbudgeted(&device, &queue),
                &pool,
                handles[index]
            ),
            replacements[index].data
        );
    }
    assert_eq!(
        read_column_values(
            GpuAllocCtx::unbudgeted(&device, &queue),
            &pool,
            pool.handle_for("survivor").unwrap(),
        ),
        old.data
    );
}

#[test]
fn mixed_streamed_promotion_candidate_rolls_back_then_commits_as_one_pool() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let per_region = (ALIGN / COLUMN_VALUE_BYTES as u64) as usize;
    let resident = col_f64(vec![1.0; per_region]);
    for id in ["keep-a", "hole", "keep-c"] {
        pool.add_column(
            id.into(),
            &resident,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    }
    pool.remove_column("hole").unwrap();
    pool.defrag_policy = DefragPolicy::OnAllocFailure;
    let scalar = col_f64(vec![3.25]);
    let hilo = col_f64(vec![1_700_000_000_000.125]);
    let uploads = [
        StreamedColumnUpload {
            id: "stream-scalar",
            source: StreamColumnSource::Scalar(&scalar),
            len_values: scalar.data.len(),
        },
        StreamedColumnUpload {
            id: "stream-hilo",
            source: StreamColumnSource::HiLo(&hilo),
            len_values: hilo.data.len(),
        },
    ];
    let before = full_pool_state(&pool);
    let retired_before = pool.retired_bytes();

    {
        let pending = pool
            .begin_streamed_promotion(
                &uploads,
                4 * ALIGN,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert!(pending.relocated());
        assert!(pending.pool().slot("stream-scalar").is_some());
        assert!(pending.pool().slot("stream-hilo").is_some());
    }
    assert_eq!(full_pool_state(&pool), before);
    assert_eq!(
        pool.retired_bytes(),
        retired_before + 6 * ALIGN,
        "candidate slab and the two-column staging stay retired after rollback"
    );

    pool.begin_streamed_promotion(
        &uploads,
        4 * ALIGN,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap()
    .commit();
    assert!(pool.slot("stream-scalar").is_some());
    assert!(pool.slot("stream-hilo").is_some());
    assert_eq!(pool.layout_generation(), before.base.layout_generation + 1);
    assert_eq!(
        read_column_values(
            GpuAllocCtx::unbudgeted(&device, &queue),
            &pool,
            pool.handle_for("stream-scalar").unwrap(),
        ),
        vec![3.25]
    );
    assert_eq!(
        read_column_values(
            GpuAllocCtx::unbudgeted(&device, &queue),
            &pool,
            pool.handle_for("stream-hilo").unwrap(),
        ),
        hilo.data
    );
}

#[test]
fn column_removal_guard_drop_restores_exact_pool_state() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0]);
    pool.add_column(
        "remove-a".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "remove-b".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let identity = pool.identity();
    let before_a = pool_state(&pool, "remove-a");
    let before_b = pool_state(&pool, "remove-b");

    {
        let removal = pool
            .begin_remove_column("remove-a")
            .unwrap()
            .expect("existing column must produce a guard");
        assert_eq!(removal.pool().identity(), identity);
        assert!(removal.pool().handle_for("remove-a").is_none());
        assert!(removal.pool().handle_for("remove-b").is_some());
    }

    assert_eq!(pool.identity(), identity);
    assert_eq!(pool_state(&pool, "remove-a"), before_a);
    assert_eq!(pool_state(&pool, "remove-b"), before_b);
}

#[test]
fn batch_removal_guard_validates_rolls_back_and_commits_all_ids_together() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0]);
    for id in ["batch-a", "batch-b", "batch-c"] {
        pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
    }
    let before = full_pool_state(&pool);
    assert!(matches!(
        pool.begin_remove_columns(&["batch-a", "batch-a"]),
        Err(AllocError::DuplicateId(_))
    ));
    assert_eq!(full_pool_state(&pool), before);
    assert!(pool.begin_remove_columns(&["batch-a", "missing"]).is_err());
    assert_eq!(full_pool_state(&pool), before);

    {
        let removal = pool.begin_remove_columns(&["batch-a", "batch-c"]).unwrap();
        assert!(removal.pool().slot("batch-a").is_none());
        assert!(removal.pool().slot("batch-b").is_some());
        assert!(removal.pool().slot("batch-c").is_none());
    }
    assert_eq!(full_pool_state(&pool), before);

    let survivor_epoch = pool.allocation_epoch("batch-b");
    pool.begin_remove_columns(&["batch-a", "batch-c"])
        .unwrap()
        .commit();
    assert!(pool.slot("batch-a").is_none());
    assert!(pool.slot("batch-c").is_none());
    assert!(pool.slot("batch-b").is_some());
    assert_eq!(pool.allocation_epoch("batch-b"), survivor_epoch);
    assert_eq!(pool.generation(), before.base.generation + 1);
    assert_eq!(pool.free_bytes(), 3 * ALIGN);
}

#[test]
fn column_removal_guard_commit_publishes_without_changing_identity() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0]);
    pool.add_column(
        "remove-commit".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let identity = pool.identity();

    let removal = pool
        .begin_remove_column("remove-commit")
        .unwrap()
        .expect("existing column must produce a guard");
    assert!(removal.commit());

    assert_eq!(pool.identity(), identity);
    assert!(pool.handle_for("remove-commit").is_none());
    assert_eq!(pool.free_bytes(), pool.capacity());
    assert!(pool.begin_remove_column("missing").unwrap().is_none());
}

#[test]
fn defragment_guard_fresh_candidate_drop_restores_primary_and_no_backup() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0]);
    for id in ["fresh-a", "fresh-b", "fresh-c"] {
        pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
    }
    pool.remove_column("fresh-b").unwrap();
    let identity = pool.identity();
    let before_a = pool_state(&pool, "fresh-a");
    let before_c = pool_state(&pool, "fresh-c");
    let retired_before = pool.retired_bytes();
    assert!(pool.backup.is_none());

    {
        let defrag = pool
            .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert!(defrag.changed());
        assert!(defrag.relocated());
        assert_eq!(defrag.pool().identity(), identity);
        assert_eq!(defrag.pool().slots["fresh-c"].offset, ALIGN);
        assert!(defrag.pool().backup.is_some());
    }

    assert_eq!(pool.identity(), identity);
    assert!(pool.backup.is_none());
    assert_eq!(pool_state(&pool, "fresh-a"), before_a);
    assert_eq!(pool_state(&pool, "fresh-c"), before_c);
    assert_eq!(
        pool.retired_bytes(),
        retired_before + pool.capacity(),
        "abandoned submitted candidate stays budgeted"
    );
}

#[test]
fn defragment_guard_existing_backup_drop_restores_both_buffers() {
    let Some((device, queue, mut pool)) = mk_pool(5 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0]);
    for id in ["reuse-a", "reuse-b", "reuse-c", "reuse-d"] {
        pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
    }
    pool.remove_column("reuse-b").unwrap();
    assert!(
        pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap()
    );
    assert!(pool.backup.is_some());

    pool.remove_column("reuse-c").unwrap();
    let before_a = pool_state(&pool, "reuse-a");
    let before_d = pool_state(&pool, "reuse-d");
    let primary_before = buffer_identity(pool.buffer());
    let backup_before = pool.backup.as_ref().map(buffer_identity);

    {
        let defrag = pool
            .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert!(defrag.changed());
        assert!(defrag.relocated());
        assert_eq!(defrag.pool().slots["reuse-d"].offset, ALIGN);
        assert_eq!(
            buffer_identity(defrag.pool().buffer()),
            backup_before.unwrap()
        );
    }

    assert_eq!(buffer_identity(pool.buffer()), primary_before);
    assert_eq!(pool.backup.as_ref().map(buffer_identity), backup_before);
    assert_eq!(pool_state(&pool, "reuse-a"), before_a);
    assert_eq!(pool_state(&pool, "reuse-d"), before_d);
}

#[test]
fn defragment_guard_distinguishes_noop_normalization_and_relocation() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let identity = pool.identity();

    let no_op = pool
        .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    assert!(!no_op.changed());
    assert!(!no_op.relocated());
    assert!(!no_op.commit());

    pool.free = vec![
        FreeRegion {
            offset: 0,
            size: ALIGN,
        },
        FreeRegion {
            offset: ALIGN,
            size: pool.capacity() - ALIGN,
        },
    ];
    let empty_before = pool_state(&pool, "missing");
    {
        let normalized = pool
            .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert!(normalized.changed());
        assert!(!normalized.relocated());
        assert_eq!(normalized.pool().identity(), identity);
        assert_eq!(normalized.pool().free.len(), 1);
    }
    assert_eq!(pool_state(&pool, "missing"), empty_before);

    let normalized = pool
        .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    assert!(normalized.changed());
    assert!(!normalized.relocated());
    assert!(normalized.commit());
    assert_eq!(pool.identity(), identity);
    assert_eq!(pool.free.len(), 1);

    let values = col_f64(vec![1.0]);
    pool.add_column(
        "packed".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.free = vec![
        FreeRegion {
            offset: ALIGN,
            size: ALIGN,
        },
        FreeRegion {
            offset: 2 * ALIGN,
            size: 2 * ALIGN,
        },
    ];
    let generation = pool.generation();
    let layout_generation = pool.layout_generation();
    let normalized = pool
        .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    assert!(normalized.changed());
    assert!(!normalized.relocated());
    assert!(!normalized.commit());
    assert_eq!(pool.generation(), generation);
    assert_eq!(pool.layout_generation(), layout_generation);
    assert_eq!(pool.free.len(), 1);
}

#[test]
fn legacy_staging_failure_restores_allocator_and_allows_retry() {
    let Some((device, queue, mut pool)) = mk_pool(ALIGN) else {
        return;
    };
    let column = col_f64(vec![1.0, 2.0, 3.0]);
    let before = pool_state(&pool, "x");

    let error = pool
        .try_add_column_pairs_with(
            ColumnInputMeta {
                id: "x".into(),
                len_values: column.data.len(),
                min: column.min,
                max: column.max,
            },
            GpuAllocCtx::unbudgeted(&device, &queue),
            |dst| write_scalar_source_as_pairs(&column, dst),
            |_| {
                Err(AllocError::AllocationFailed {
                    resource: "column staging buffer",
                    reason: "injected legacy staging failure".into(),
                })
            },
        )
        .unwrap_err();

    assert!(matches!(error, AllocError::AllocationFailed { .. }));
    assert_eq!(pool_state(&pool, "x"), before);

    let handle = pool
        .add_column(
            "x".into(),
            &column,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(handle.offset, 0);
    assert_eq!(pool.slot("x").unwrap().len_values, column.data.len());
}

#[test]
fn legacy_writer_panics_restore_scalar_and_hilo_allocator_state() {
    let Some((device, queue, mut scalar_pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let scalar_before = pool_state(&scalar_pool, "scalar");
    let scalar_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = scalar_pool.add_column(
            "scalar".into(),
            &PanickingScalarSource,
            GpuAllocCtx::unbudgeted(&device, &queue),
        );
    }));
    assert!(scalar_result.is_err());
    assert_eq!(pool_state(&scalar_pool, "scalar"), scalar_before);

    let retry = col_f64(vec![2.0]);
    let scalar_handle = scalar_pool
        .add_column(
            "scalar".into(),
            &retry,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(scalar_handle.offset, 0);

    let mut hilo_pool =
        ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), 2 * ALIGN).unwrap();
    let hilo_before = pool_state(&hilo_pool, "hilo");
    let hilo_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = hilo_pool.add_hilo_column(
            "hilo".into(),
            &PanickingHiLoSource,
            GpuAllocCtx::unbudgeted(&device, &queue),
        );
    }));
    assert!(hilo_result.is_err());
    assert_eq!(pool_state(&hilo_pool, "hilo"), hilo_before);

    let hilo_handle = hilo_pool
        .add_hilo_column(
            "hilo".into(),
            &retry,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(hilo_handle.offset, 0);
}

// ── Batch upload (W6) ───────────────────────────────────────────────────

/// The whole batch lands in one contiguous region, in declaration order,
/// with each column's own bytes and statistics — and it costs **one** GPU
/// buffer, where adding the same columns one at a time costs one each. This
/// is the measurement the batch exists for, so it is asserted rather than
/// described.
#[test]
fn a_column_batch_uploads_into_one_region_for_one_buffer() {
    let Some((device, queue, mut pool)) = mk_pool(64 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let a = col_f64(vec![1.0, 2.0, 3.0]);
    let b = col_f64(vec![10.0, 20.0]);
    let c = col_f64(vec![-5.0, 0.0, 5.0, 7.5]);

    let before_creations = pool.buffer_creations();
    pool.add_columns(
        &[
            ("a", &a as &dyn ColumnSource),
            ("b", &b as &dyn ColumnSource),
            ("c", &c as &dyn ColumnSource),
        ],
        ctx(),
    )
    .expect("batch upload");

    // One staging buffer for the batch, not one per column.
    assert_eq!(
        pool.buffer_creations() - before_creations,
        1,
        "a batch must create one staging buffer"
    );

    // Contiguous, in order, ALIGN-aligned.
    let mut expected_offset = 0u64;
    for (id, len) in [("a", 3usize), ("b", 2), ("c", 4)] {
        let slot = pool.slot(id).expect("batch column is registered");
        assert_eq!(slot.offset, expected_offset, "{id} offset");
        assert_eq!(slot.len_values, len, "{id} length");
        assert_eq!(slot.byte_size % ALIGN, 0, "{id} padding");
        expected_offset += slot.byte_size;
    }
    assert_eq!(pool.used_bytes(), expected_offset);

    // Values and statistics survive the shared staging buffer.
    let handle = |id: &str| pool.handle_for(id).expect("handle");
    assert_eq!(read_column_values(ctx(), &pool, handle("a")), a.data);
    assert_eq!(read_column_values(ctx(), &pool, handle("b")), b.data);
    assert_eq!(read_column_values(ctx(), &pool, handle("c")), c.data);
    assert_eq!(pool.slot("c").unwrap().min, -5.0);
    assert_eq!(pool.slot("c").unwrap().max, 7.5);
    assert_eq!(
        pool.slot("c").unwrap().min_positive,
        Some(5.0),
        "min_positive comes from the write, not the source"
    );

    // The same columns added one at a time cost one staging buffer each.
    let singles_before = pool.buffer_creations();
    for (id, source) in [("single-a", &a), ("single-b", &b), ("single-c", &c)] {
        pool.add_column(id.into(), source, ctx())
            .expect("single add");
    }
    assert_eq!(
        pool.buffer_creations() - singles_before,
        3,
        "one staging buffer per single add"
    );
}

/// A rejected batch changes nothing: no slot, no epoch, and the free list is
/// byte-identical — the single reservation rolled back.
#[test]
fn a_rejected_batch_leaves_the_pool_untouched() {
    let Some((device, queue, mut pool)) = mk_pool(8 * ALIGN) else {
        return;
    };
    // A fixed pool: this test is about what a pool that cannot grow does.
    pool.growth_policy = GrowthPolicy::Fixed;
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let a = col_f64(vec![1.0, 2.0]);
    let empty: Column<f64> = Column {
        data: Vec::new(),
        min: 0.0,
        max: 0.0,
    };
    pool.add_column("live".into(), &a, ctx()).expect("seed");
    let free_before = pool.free.clone();
    let slots_before = pool.slots.len();
    let epoch_before = pool.allocation_epoch_counter;

    // An id already in the pool.
    assert!(matches!(
        pool.add_columns(
            &[
                ("fresh", &a as &dyn ColumnSource),
                ("live", &a as &dyn ColumnSource),
            ],
            ctx()
        ),
        Err(AllocError::DuplicateId(_))
    ));
    // A duplicate inside the batch.
    assert!(matches!(
        pool.add_columns(
            &[
                ("twice", &a as &dyn ColumnSource),
                ("twice", &a as &dyn ColumnSource),
            ],
            ctx()
        ),
        Err(AllocError::DuplicateId(_))
    ));
    // An empty source.
    assert!(matches!(
        pool.add_columns(
            &[
                ("fresh", &a as &dyn ColumnSource),
                ("nothing", &empty as &dyn ColumnSource),
            ],
            ctx()
        ),
        Err(AllocError::EmptySource)
    ));
    // More than the pool can hold.
    let big: Vec<Column<f64>> = (0..16).map(|_| col_f64(vec![1.0; 64])).collect();
    let too_much: Vec<(&str, &dyn ColumnSource)> = big
        .iter()
        .enumerate()
        .map(|(i, source)| {
            (
                [
                    "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8", "t9", "ta", "tb", "tc",
                    "td", "te", "tf",
                ][i],
                source as &dyn ColumnSource,
            )
        })
        .collect();
    assert!(matches!(
        pool.add_columns(&too_much, ctx()),
        Err(AllocError::OutOfSpace { .. })
    ));

    assert_eq!(pool.slots.len(), slots_before, "no slot was published");
    assert!(pool.slot("fresh").is_none());
    assert_eq!(
        pool.free, free_before,
        "the reservation rolled back exactly"
    );
    assert_eq!(
        pool.allocation_epoch_counter, epoch_before,
        "no epoch was consumed"
    );
    // And the surviving column still reads back.
    let handle = pool.handle_for("live").expect("handle");
    assert_eq!(read_column_values(ctx(), &pool, handle), a.data);
}

/// An empty batch is a no-op, not an error.
#[test]
fn an_empty_batch_does_nothing() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let before = (pool.slots.len(), pool.buffer_creations());
    pool.add_columns(&[], GpuAllocCtx::unbudgeted(&device, &queue))
        .expect("empty batch");
    assert_eq!((pool.slots.len(), pool.buffer_creations()), before);
}

/// Chunking is only about the device's staging ceiling: the plan splits at
/// that boundary, every chunk mirrors the region layout, and the offsets
/// stay contiguous across chunks. Tested at the planner because a real
/// device's ceiling is far too large to reach from a unit test.
#[test]
fn the_staging_plan_splits_only_at_the_device_ceiling() {
    let one = col_f64(vec![1.0; 32]); // 256 B raw = exactly one ALIGN
    let sources: Vec<Column<f64>> = (0..5).map(|_| one.clone()).collect();
    let columns: Vec<(&str, &dyn ColumnSource)> = vec![
        ("c0", &sources[0] as &dyn ColumnSource),
        ("c1", &sources[1] as &dyn ColumnSource),
        ("c2", &sources[2] as &dyn ColumnSource),
        ("c3", &sources[3] as &dyn ColumnSource),
        ("c4", &sources[4] as &dyn ColumnSource),
    ];
    assert_eq!(batch_region_bytes(&columns, u64::MAX).unwrap(), 5 * ALIGN);

    // Ceiling above the batch: one chunk, one copy.
    let whole = plan_batch_chunks(&columns, 64 * ALIGN).unwrap();
    assert_eq!(whole.len(), 1);
    assert_eq!(whole[0].byte_size, 5 * ALIGN);
    assert_eq!(whole[0].region_offset, 0);
    assert_eq!(whole[0].column_offsets, vec![0, 256, 512, 768, 1024]);

    // Ceiling of two columns: three chunks, contiguous in the region.
    let split = plan_batch_chunks(&columns, 2 * ALIGN).unwrap();
    assert_eq!(split.len(), 3);
    assert_eq!(split[0].columns, 0..2);
    assert_eq!(split[1].columns, 2..4);
    assert_eq!(split[2].columns, 4..5);
    let mut expected_offset = 0;
    for chunk in &split {
        assert_eq!(chunk.region_offset, expected_offset);
        assert!(chunk.byte_size <= 2 * ALIGN);
        assert_eq!(chunk.column_offsets[0], 0, "each chunk starts at its own 0");
        expected_offset += chunk.byte_size;
    }
    assert_eq!(expected_offset, 5 * ALIGN, "chunks tile the region exactly");
}

/// A single column larger than the device can stage is refused whether it
/// arrives alone or in a batch — the batch does not make it possible.
#[test]
fn a_column_over_the_device_ceiling_is_refused_in_a_batch_too() {
    let huge = col_f64(vec![1.0; 64]);
    let columns: Vec<(&str, &dyn ColumnSource)> = vec![("huge", &huge as &dyn ColumnSource)];
    assert!(matches!(
        batch_region_bytes(&columns, ALIGN),
        Err(AllocError::ResourceLimit { .. })
    ));
}

/// The guard's whole reason to exist: a caller with its own fallible work
/// can look at the provisional pool and then abandon it, and the pool is
/// byte-for-byte what it was — without the batch ever cloning a registry.
#[test]
fn an_abandoned_batch_guard_un_inserts_every_column() {
    let Some((device, queue, mut pool)) = mk_pool(64 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let live = col_f64(vec![1.0, 2.0]);
    let a = col_f64(vec![3.0, 4.0, 5.0]);
    let b = col_f64(vec![6.0]);
    pool.add_column("live".into(), &live, ctx()).expect("seed");

    let free_before = pool.free.clone();
    let used_before = pool.used_bytes();
    let epoch_before = pool.allocation_epoch_counter;
    let generation_before = pool.generation();
    let layout_before = pool.layout_generation();

    let guard = pool
        .begin_add_columns(
            &[
                ("a", &a as &dyn ColumnSource),
                ("b", &b as &dyn ColumnSource),
            ],
            ctx(),
        )
        .expect("batch begins");
    // Provisionally visible through the guard's own pool view.
    assert!(guard.pool().slot("a").is_some(), "provisional slot a");
    assert!(guard.pool().slot("b").is_some(), "provisional slot b");
    assert!(guard.pool().used_bytes() > used_before);
    drop(guard);

    assert!(pool.slot("a").is_none(), "abandoned batch left slot a");
    assert!(pool.slot("b").is_none(), "abandoned batch left slot b");
    assert_eq!(pool.slots.len(), 1, "only the seeded column survives");
    assert_eq!(pool.allocation_epochs.len(), 1, "epochs rolled back too");
    assert_eq!(pool.free, free_before, "the region went back exactly");
    assert_eq!(pool.used_bytes(), used_before);
    assert_eq!(pool.allocation_epoch_counter, epoch_before);
    assert_eq!(pool.generation(), generation_before, "no invalidation");
    assert_eq!(pool.layout_generation(), layout_before);

    // The seeded column is untouched, and the freed region is reusable.
    let handle = pool.handle_for("live").expect("handle");
    assert_eq!(read_column_values(ctx(), &pool, handle), live.data);
    pool.add_columns(&[("a", &a as &dyn ColumnSource)], ctx())
        .expect("the region is allocatable again");
    assert_eq!(
        pool.slot("a").expect("re-added").offset,
        pool.slot("live").expect("seed").byte_size,
        "the retry takes the same offset the abandoned batch had"
    );
}

/// Committing publishes exactly once: the guard is the only gate, and a
/// committed batch behaves like `add_columns` did.
#[test]
fn a_committed_batch_guard_publishes_every_column() {
    let Some((device, queue, mut pool)) = mk_pool(64 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let a = col_f64(vec![1.5, 2.5]);
    let b = col_f64(vec![-1.0, 0.0, 1.0]);
    pool.begin_add_columns(
        &[
            ("a", &a as &dyn ColumnSource),
            ("b", &b as &dyn ColumnSource),
        ],
        ctx(),
    )
    .expect("batch begins")
    .commit();

    let handle = |id: &str| pool.handle_for(id).expect("handle");
    assert_eq!(read_column_values(ctx(), &pool, handle("a")), a.data);
    assert_eq!(read_column_values(ctx(), &pool, handle("b")), b.data);
    assert_eq!(
        pool.allocation_epochs.len(),
        2,
        "one epoch per published column"
    );
}

/// Fragmentation alone must not fail a batch under `OnAllocFailure`: the
/// batch needs one *contiguous* region, and compaction is what produces it.
/// The pre-decision replaces the old attempt-then-retry rung, so the
/// observable outcome is what is asserted.
#[test]
fn a_batch_compacts_when_fragmentation_is_the_only_obstacle() {
    let Some((device, queue, mut pool)) = mk_pool(6 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    pool.defrag_policy = DefragPolicy::OnAllocFailure;
    let one = col_f64(vec![1.0; 32]); // exactly one ALIGN
    for id in ["a", "b", "c", "d", "e", "f"] {
        pool.add_column(id.to_string(), &one, ctx())
            .expect("fill the pool");
    }
    // Free two ALIGNs that are not adjacent.
    pool.remove_column("b").expect("remove b");
    pool.remove_column("e").expect("remove e");
    assert_eq!(pool.free_bytes(), 2 * ALIGN);
    assert_eq!(pool.largest_free_region(), ALIGN, "fragmented");

    let x = col_f64(vec![7.0; 32]);
    let y = col_f64(vec![8.0; 32]);
    pool.add_columns(
        &[
            ("x", &x as &dyn ColumnSource),
            ("y", &y as &dyn ColumnSource),
        ],
        ctx(),
    )
    .expect("compaction made one region out of two");
    assert_eq!(pool.used_bytes(), 6 * ALIGN);
    assert_eq!(pool.free_bytes(), 0);
    let handle = |id: &str| pool.handle_for(id).expect("handle");
    assert_eq!(read_column_values(ctx(), &pool, handle("x")), x.data);
    assert_eq!(read_column_values(ctx(), &pool, handle("y")), y.data);
    // Compaction relocated the survivors, so their bytes must still read.
    assert_eq!(read_column_values(ctx(), &pool, handle("a")), one.data);
}

/// With growth off, the same fragmented pool that cannot fit the batch says
/// so — and stays exactly as it was.
#[test]
fn a_batch_too_big_for_a_fixed_pool_is_refused_without_compacting() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    // A fixed pool: this test is about what a pool that cannot grow does.
    pool.growth_policy = GrowthPolicy::Fixed;
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    pool.defrag_policy = DefragPolicy::OnAllocFailure;
    let one = col_f64(vec![1.0; 32]);
    for id in ["a", "b", "c"] {
        pool.add_column(id.to_string(), &one, ctx()).expect("fill");
    }
    let layout_before = pool.layout_generation();
    let big = col_f64(vec![1.0; 64]); // two ALIGNs, only one is free
    assert!(matches!(
        pool.add_columns(&[("big", &big as &dyn ColumnSource)], ctx()),
        Err(AllocError::OutOfSpace { .. })
    ));
    assert_eq!(
        pool.layout_generation(),
        layout_before,
        "a batch that cannot fit must not pay for a pointless compaction"
    );
    assert!(pool.slot("big").is_none());
}

/// A batch bigger than the pool grows it, once, under `OnAllocFailure`.
#[test]
fn a_batch_grows_the_pool_when_compaction_is_not_enough() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    pool.growth_policy = GrowthPolicy::OnAllocFailure;
    let one = col_f64(vec![1.0; 32]);
    pool.add_column("seed".into(), &one, ctx()).expect("seed");
    let capacity_before = pool.capacity();

    let columns: Vec<Column<f64>> = (0..4).map(|_| one.clone()).collect();
    let batch: Vec<(&str, &dyn ColumnSource)> = vec![
        ("g0", &columns[0] as &dyn ColumnSource),
        ("g1", &columns[1] as &dyn ColumnSource),
        ("g2", &columns[2] as &dyn ColumnSource),
        ("g3", &columns[3] as &dyn ColumnSource),
    ];
    pool.add_columns(&batch, ctx()).expect("growth made it fit");
    assert!(
        pool.capacity() >= capacity_before + 3 * ALIGN,
        "capacity {} did not grow enough from {capacity_before}",
        pool.capacity()
    );
    assert_eq!(pool.used_bytes(), 5 * ALIGN);
    let handle = |id: &str| pool.handle_for(id).expect("handle");
    assert_eq!(read_column_values(ctx(), &pool, handle("seed")), one.data);
    assert_eq!(read_column_values(ctx(), &pool, handle("g3")), one.data);
}

fn read_column_values(ctx: GpuAllocCtx<'_>, pool: &ColumnPool, handle: ColumnHandle) -> Vec<f64> {
    read_column_words(ctx, pool, handle)
        .chunks_exact(2)
        .map(|pair| f32::from_bits(pair[0]) as f64 + f32::from_bits(pair[1]) as f64)
        .collect()
}

/// The two lanes of every value of a column, as `u32` bits: a ragged
/// column's offsets are `u32` index lanes (`SHADER_COMMON.md` §5.14).
fn read_column_words(ctx: GpuAllocCtx<'_>, pool: &ColumnPool, handle: ColumnHandle) -> Vec<u32> {
    let (device, queue) = (ctx.device, ctx.queue);
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("figgy column pool test readback"),
        size: handle.byte_size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("figgy column pool test readback"),
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
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("readback poll");
    receiver
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("readback callback")
        .expect("map readback");
    let mapped = readback
        .slice(..handle.byte_size)
        .get_mapped_range()
        .expect("column readback is mapped after map_async");
    let words = mapped[..handle.len_values * COLUMN_VALUE_BYTES]
        .chunks_exact(4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .collect();
    drop(mapped);
    readback.unmap();
    words
}

#[test]
fn upsert_prepare_failures_leave_old_state_and_retry() {
    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    // A fixed pool: this test is about what a pool that cannot grow does.
    pool.growth_policy = GrowthPolicy::Fixed;
    let old = col_f64(vec![1.0, 2.0, 3.0]);
    pool.add_column("x".into(), &old, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    let blocker = col_f64(vec![10.0, 11.0, 12.0]);
    pool.add_column(
        "blocker".into(),
        &blocker,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let before = pool_state(&pool, "x");

    let replacement = col_f64(vec![4.0, 5.0, 6.0]);
    let staging_error = match pool.begin_upsert_column_pairs_with(
        ColumnInputMeta {
            id: "x".into(),
            len_values: replacement.data.len(),
            min: replacement.min,
            max: replacement.max,
        },
        GpuAllocCtx::unbudgeted(&device, &queue),
        |dst| write_scalar_source_as_pairs(&replacement, dst),
        |_| {
            Err(AllocError::AllocationFailed {
                resource: "column staging buffer",
                reason: "injected test failure".into(),
            })
        },
    ) {
        Ok(_) => panic!("injected staging failure unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(matches!(staging_error, AllocError::AllocationFailed { .. }));
    assert_eq!(pool_state(&pool, "x"), before);

    let empty = col_f64(Vec::new());
    let validation_error = match pool.begin_upsert_column(
        "x".into(),
        &empty,
        GpuAllocCtx::unbudgeted(&device, &queue),
    ) {
        Ok(_) => panic!("empty replacement unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(validation_error, AllocError::EmptySource);
    assert_eq!(pool_state(&pool, "x"), before);

    let too_large = col_f64((0..50).map(|value| value as f64).collect());
    let capacity_error = match pool.begin_upsert_column(
        "x".into(),
        &too_large,
        GpuAllocCtx::unbudgeted(&device, &queue),
    ) {
        Ok(_) => panic!("oversized replacement unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(matches!(capacity_error, AllocError::OutOfSpace { .. }));
    assert_eq!(pool_state(&pool, "x"), before);

    let handle = pool
        .upsert_column(
            "x".into(),
            &replacement,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(handle.len_values, replacement.data.len());
    assert_eq!(pool.slot("x").unwrap().min, 4.0);
}

#[test]
fn dropped_upsert_restores_old_mapping_stats_and_bytes() {
    let Some((device, queue, mut pool)) = mk_pool(ALIGN) else {
        return;
    };
    let old = col_f64(vec![1.25, 2.5, 4.75]);
    let old_handle = pool
        .add_column("x".into(), &old, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    let before = pool_state(&pool, "x");
    let retired_before = pool.retired_bytes();

    {
        let replacement = col_f64(vec![100.0, 200.0, 300.0]);
        let pending = pool
            .begin_upsert_column(
                "x".into(),
                &replacement,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_ne!(pending.handle().generation, old_handle.generation);
        assert_eq!(pending.pool().slot("x").unwrap().min, 100.0);
    }

    assert_eq!(pool_state(&pool, "x"), before);
    assert_eq!(
        pool.retired_bytes(),
        retired_before + 2 * ALIGN,
        "abandoned candidate and submitted staging stay budgeted"
    );
    assert!(pool.is_valid_handle(&old_handle));
    assert_eq!(
        read_column_values(GpuAllocCtx::unbudgeted(&device, &queue), &pool, old_handle),
        old.data
    );
}

#[test]
fn same_size_replacement_reuses_full_capacity_and_updates_stats() {
    let Some((device, queue, mut pool)) = mk_pool(ALIGN) else {
        return;
    };
    let old = col_f64(vec![-4.0, -2.0, 8.0]);
    let old_handle = pool
        .add_column("x".into(), &old, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    assert_eq!(pool.free_bytes(), 0);

    let replacement = col_f64(vec![-10.0, 0.0, 3.5]);
    let pending = pool
        .begin_upsert_column(
            "x".into(),
            &replacement,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert!(pending.replaced_existing());
    assert_eq!(pending.pool().free_bytes(), 0);
    let new_handle = pending.commit();

    assert_eq!(pool.capacity(), ALIGN);
    assert_eq!(pool.used_bytes(), ALIGN);
    assert_eq!(pool.free_bytes(), 0);
    assert_ne!(new_handle.generation, old_handle.generation);
    assert!(!pool.is_valid_handle(&old_handle));
    let slot = pool.slot("x").unwrap();
    assert_eq!(
        (slot.min, slot.max, slot.min_positive),
        (-10.0, 3.5, Some(3.5))
    );
    assert_eq!(
        read_column_values(GpuAllocCtx::unbudgeted(&device, &queue), &pool, new_handle),
        replacement.data
    );
}

#[test]
fn upsert_inserts_and_replaces_larger_and_smaller_columns() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let small = col_f64((0..20).map(|value| value as f64).collect());
    let inserted = pool
        .upsert_column("x".into(), &small, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    assert_eq!(inserted.byte_size, ALIGN);

    let larger = col_f64((0..50).map(|value| value as f64).collect());
    let larger_handle = pool
        .upsert_hilo_column(
            "x".into(),
            &larger,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(larger_handle.byte_size, 2 * ALIGN);
    assert_eq!(pool.used_bytes(), 2 * ALIGN);
    assert_eq!(pool.slot("x").unwrap().len_values, 50);

    let smaller = col_f64(vec![7.0, 8.0]);
    let smaller_handle = pool
        .upsert_column(
            "x".into(),
            &smaller,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(smaller_handle.byte_size, ALIGN);
    assert_eq!(pool.used_bytes(), ALIGN);
    assert_eq!(pool.free_bytes(), 3 * ALIGN);
    assert_eq!(
        read_column_values(
            GpuAllocCtx::unbudgeted(&device, &queue),
            &pool,
            smaller_handle
        ),
        smaller.data
    );
}

#[test]
fn pool_basic_add_lookup_remove() {
    let Some((device, queue, mut pool)) = mk_pool(64 * 1024) else {
        println!("no adapter — skipping");
        return;
    };

    let c = col_f64((0..100).map(|i| i as f64).collect());
    let h = pool
        .add_column(
            "x".to_string(),
            &c,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();

    // 100 logical f32-pair values = 800 bytes, rounded up to ALIGN(256) -> 1024.
    assert_eq!(h.byte_size, 1024);
    assert_eq!(h.offset, 0);
    assert_eq!(h.len_values, 100);

    assert!(pool.handle_for("x").is_some());
    assert_eq!(pool.used_bytes(), 1024);

    let removed = pool.remove_column("x").unwrap();
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
    let a = col_f64((0..25).map(|i| i as f64).collect()); // 200 -> 256
    let b = col_f64((0..120).map(|i| i as f64).collect()); // 960 -> 1024
    let c = col_f64((0..10).map(|i| i as f64).collect()); // 80 -> 256

    let ha = pool
        .add_column("a".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    let hb = pool
        .add_column("b".into(), &b, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    let hc = pool
        .add_column("c".into(), &c, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();

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
    let a = col_f64((0..25).map(|i| i as f64).collect()); // 256
    let b = col_f64((0..25).map(|i| i as f64).collect()); // 256
    let c = col_f64((0..25).map(|i| i as f64).collect()); // 256

    pool.add_column("a".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    pool.add_column("b".into(), &b, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    pool.add_column("c".into(), &c, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();

    // Remove middle b → free = [256..512] + [768..end] (not adjacent).
    pool.remove_column("b").unwrap();
    assert_eq!(pool.free.len(), 2);

    // Remove a → [0..512] (coalesced with b's hole) + [768..end].
    pool.remove_column("a").unwrap();
    assert_eq!(pool.free.len(), 2);
    assert_eq!(pool.free[0].offset, 0);
    assert_eq!(pool.free[0].size, 512);

    // Remove c → everything coalesces back to one free region.
    pool.remove_column("c").unwrap();
    assert_eq!(pool.free.len(), 1);
    assert_eq!(pool.free[0].offset, 0);
    assert_eq!(pool.free[0].size, pool.capacity());
}

#[test]
fn pool_duplicate_id() {
    let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
        return;
    };
    let a = col_f64((0..10).map(|i| i as f64).collect());
    pool.add_column("dup".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    let res = pool.add_column("dup".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue));
    assert!(matches!(res, Err(AllocError::DuplicateId(_))));
}

#[test]
fn pool_handle_byte_range() {
    let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
        return;
    };
    let c = col_f64((0..50).map(|i| i as f64).collect());
    let h = pool
        .add_column("x".into(), &c, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    let r = h.byte_range();
    assert_eq!(r.start, 0);
    assert_eq!(r.end, 512);
}

#[test]
fn pool_defragment_compacts_after_remove() {
    let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
        return;
    };
    let a = col_f64((0..25).map(|i| i as f64).collect()); // 256
    let b = col_f64((0..25).map(|i| i as f64).collect()); // 256
    let c = col_f64((0..25).map(|i| i as f64).collect()); // 256

    let _ = pool
        .add_column("a".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    let _ = pool
        .add_column("b".into(), &b, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    let hc_before = pool
        .add_column("c".into(), &c, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    let gen_before = pool.generation();

    // Remove middle b → hole at [256..512]; tail free region unchanged.
    pool.remove_column("b").unwrap();
    assert_eq!(pool.slots["c"].offset, 512);
    assert_eq!(pool.free.len(), 2);
    assert_eq!(pool.free[0].offset, 256);
    assert_eq!(pool.free[0].size, 256);

    // After defrag → a@0, c@256, free is a single tail region.
    let moved = pool
        .defragment(GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
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
    let a = col_f64((0..25).map(|i| i as f64).collect());
    let b = col_f64((0..25).map(|i| i as f64).collect());
    pool.add_column("a".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    pool.add_column("b".into(), &b, GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    let g0 = pool.generation();

    let moved = pool
        .defragment(GpuAllocCtx::unbudgeted(&device, &queue))
        .unwrap();
    assert!(!moved);
    assert_eq!(pool.generation(), g0); // no-op leaves generation alone.
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

    let small_a = col_f64((0..25).map(|i| i as f64).collect()); // 256
    let small_b = col_f64((0..25).map(|i| i as f64).collect()); // 256
    let small_c = col_f64((0..25).map(|i| i as f64).collect()); // 256
    let big = col_f64((0..50).map(|i| i as f64).collect()); // 400 -> 512

    pool.add_column(
        "a".into(),
        &small_a,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "b".into(),
        &small_b,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "c".into(),
        &small_c,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    // free = [768..1024]. Remove b → free = [256..512] + [768..1024].
    pool.remove_column("b").unwrap();
    // big needs 512 contiguous. First-fit fails (largest free is 256),
    // defrag fuses the hole and tail into [512..1024], retry succeeds.
    let res = pool.add_column("big".into(), &big, GpuAllocCtx::unbudgeted(&device, &queue));
    assert!(
        res.is_ok(),
        "auto defrag retry should have succeeded: {res:?}"
    );
    // big lands right after a/c, at offset 512.
    assert_eq!(pool.slots["big"].offset, 512);
}

#[test]
fn pool_manual_policy_does_not_auto_defrag() {
    let Some((device, queue, mut pool)) = mk_pool(4 * 256) else {
        return;
    };
    // A fixed pool: this test is about what a pool that cannot grow does.
    pool.growth_policy = GrowthPolicy::Fixed;
    // Default policy is Manual.
    let small_a = col_f64((0..25).map(|i| i as f64).collect());
    let small_b = col_f64((0..25).map(|i| i as f64).collect());
    let small_c = col_f64((0..25).map(|i| i as f64).collect());
    let big = col_f64((0..50).map(|i| i as f64).collect());

    pool.add_column(
        "a".into(),
        &small_a,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "b".into(),
        &small_b,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "c".into(),
        &small_c,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.remove_column("b").unwrap();
    // Manual policy: no retry → OutOfSpace.
    let res = pool.add_column("big".into(), &big, GpuAllocCtx::unbudgeted(&device, &queue));
    assert!(matches!(res, Err(AllocError::OutOfSpace { .. })));
}

#[test]
fn remove_epochs_prevent_offset_reuse_aba() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0]);

    let a_handle = pool
        .add_column(
            "epoch-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    let a_epoch = pool.allocation_epoch("epoch-a").unwrap();
    let b_handle = pool
        .add_column(
            "epoch-b".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    let b_epoch = pool.allocation_epoch("epoch-b").unwrap();
    let layout_generation = pool.layout_generation();

    assert!(pool.remove_column("epoch-a").unwrap());
    assert_eq!(pool.layout_generation(), layout_generation);
    assert_eq!(pool.allocation_epoch("epoch-b"), Some(b_epoch));
    assert!(!pool.is_valid_handle(&a_handle));
    assert!(!pool.is_valid_handle(&b_handle));
    assert!(pool.is_valid_handle(&pool.handle_for("epoch-b").unwrap()));

    let c_handle = pool
        .add_column(
            "epoch-c".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(c_handle.offset, a_handle.offset);
    assert_ne!(pool.allocation_epoch("epoch-c"), Some(a_epoch));
    assert!(!pool.is_valid_handle(&a_handle));

    assert!(pool.remove_column("epoch-c").unwrap());
    let a_readded = pool
        .add_column(
            "epoch-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    assert_eq!(a_readded.offset, a_handle.offset);
    assert_ne!(pool.allocation_epoch("epoch-a"), Some(a_epoch));
    assert!(!pool.is_valid_handle(&a_handle));
}

#[test]
fn defragment_preserves_allocation_epochs_and_bumps_layout() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0]);

    pool.add_column(
        "layout-a".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "layout-b".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "layout-c".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    assert!(pool.remove_column("layout-b").unwrap());

    let a_epoch = pool.allocation_epoch("layout-a");
    let c_epoch = pool.allocation_epoch("layout-c");
    let c_handle = pool.handle_for("layout-c").unwrap();
    let generation = pool.generation();
    let layout_generation = pool.layout_generation();

    assert!(
        pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap()
    );
    assert_eq!(pool.generation(), generation.checked_add(1).unwrap());
    assert_eq!(
        pool.layout_generation(),
        layout_generation.checked_add(1).unwrap()
    );
    assert_eq!(pool.allocation_epoch("layout-a"), a_epoch);
    assert_eq!(pool.allocation_epoch("layout-c"), c_epoch);
    assert!(!pool.is_valid_handle(&c_handle));
    assert!(pool.is_valid_handle(&pool.handle_for("layout-c").unwrap()));
}

#[test]
fn clear_and_counter_exhaustion_preserve_stamp_contracts() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let values = col_f64(vec![1.0]);

    let old_handle = pool
        .add_column(
            "clear-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
    let old_epoch = pool.allocation_epoch("clear-a").unwrap();
    let allocation_epoch_counter = pool.allocation_epoch_counter;
    let generation = pool.generation();
    let layout_generation = pool.layout_generation();
    pool.clear().unwrap();
    assert_eq!(pool.generation(), generation.checked_add(1).unwrap());
    assert_eq!(
        pool.layout_generation(),
        layout_generation.checked_add(1).unwrap()
    );
    assert_eq!(pool.allocation_epoch_counter, allocation_epoch_counter);
    assert_eq!(pool.allocation_epoch("clear-a"), None);
    assert!(!pool.is_valid_handle(&old_handle));
    pool.add_column(
        "clear-a".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    assert!(pool.allocation_epoch("clear-a").unwrap() > old_epoch);

    pool.generation = u32::MAX;
    for slot in pool.slots.values_mut() {
        slot.generation = u32::MAX;
    }
    let before_public_exhaustion = pool_state(&pool, "clear-a");
    assert_eq!(
        pool.remove_column("clear-a").unwrap_err(),
        AllocError::CounterExhausted {
            counter: "public generation"
        }
    );
    assert_eq!(pool_state(&pool, "clear-a"), before_public_exhaustion);
    assert_eq!(
        pool.clear().unwrap_err(),
        AllocError::CounterExhausted {
            counter: "public generation"
        }
    );
    assert_eq!(pool_state(&pool, "clear-a"), before_public_exhaustion);
    let replacement_error = match pool.begin_upsert_column(
        "clear-a".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    ) {
        Err(error) => error,
        Ok(_) => panic!("replacement must fail when public generation is exhausted"),
    };
    assert_eq!(
        replacement_error,
        AllocError::CounterExhausted {
            counter: "public generation"
        }
    );
    assert_eq!(pool_state(&pool, "clear-a"), before_public_exhaustion);

    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    pool.add_column(
        "epoch-full-a".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.allocation_epoch_counter = u64::MAX;
    let before_epoch_exhaustion = pool_state(&pool, "epoch-full-a");
    assert_eq!(
        pool.add_column(
            "epoch-full-b".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue)
        )
        .unwrap_err(),
        AllocError::CounterExhausted {
            counter: "allocation epoch"
        }
    );
    assert_eq!(pool_state(&pool, "epoch-full-a"), before_epoch_exhaustion);

    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    pool.add_column(
        "layout-prefix".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "layout-live".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    assert!(pool.remove_column("layout-prefix").unwrap());
    pool.layout_generation = u64::MAX;
    let before_layout_exhaustion = pool_state(&pool, "layout-live");
    assert_eq!(
        pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap_err(),
        AllocError::CounterExhausted {
            counter: "layout generation"
        }
    );
    assert_eq!(pool_state(&pool, "layout-live"), before_layout_exhaustion);
    assert_eq!(
        pool.clear().unwrap_err(),
        AllocError::CounterExhausted {
            counter: "layout generation"
        }
    );
    assert_eq!(pool_state(&pool, "layout-live"), before_layout_exhaustion);
}

#[test]
fn every_relocating_counter_failure_is_fail_closed() {
    let values = col_f64(vec![1.0]);

    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    pool.add_column(
        "epoch-upsert-a".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.allocation_epoch_counter = u64::MAX;
    let before_epoch_upsert = pool_state(&pool, "epoch-upsert-a");
    let epoch_error = match pool.begin_upsert_column(
        "epoch-upsert-a".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    ) {
        Err(error) => error,
        Ok(_) => panic!("upsert must fail when allocation epoch is exhausted"),
    };
    assert_eq!(
        epoch_error,
        AllocError::CounterExhausted {
            counter: "allocation epoch"
        }
    );
    assert_eq!(pool_state(&pool, "epoch-upsert-a"), before_epoch_upsert);

    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    for id in ["public-compact-a", "public-compact-b"] {
        pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
    }
    pool.generation = u32::MAX;
    for slot in pool.slots.values_mut() {
        slot.generation = u32::MAX;
    }
    let before_public_compaction = pool_state(&pool, "public-compact-a");
    let public_compaction_error = match pool.begin_upsert_column(
        "public-compact-a".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    ) {
        Err(error) => error,
        Ok(_) => panic!("compacting upsert must fail when public generation is exhausted"),
    };
    assert_eq!(
        public_compaction_error,
        AllocError::CounterExhausted {
            counter: "public generation"
        }
    );
    assert_eq!(
        pool_state(&pool, "public-compact-a"),
        before_public_compaction
    );

    let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
        return;
    };
    for id in ["layout-compact-a", "layout-compact-b"] {
        pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
    }
    pool.layout_generation = u64::MAX;
    let before_layout_compaction = pool_state(&pool, "layout-compact-a");
    let layout_compaction_error = match pool.begin_upsert_column(
        "layout-compact-a".into(),
        &values,
        GpuAllocCtx::unbudgeted(&device, &queue),
    ) {
        Err(error) => error,
        Ok(_) => panic!("compacting upsert must fail when layout generation is exhausted"),
    };
    assert_eq!(
        layout_compaction_error,
        AllocError::CounterExhausted {
            counter: "layout generation"
        }
    );
    assert_eq!(
        pool_state(&pool, "layout-compact-a"),
        before_layout_compaction
    );

    let Some((device, queue, mut pool)) = mk_pool(3 * ALIGN) else {
        return;
    };
    for id in ["public-defrag-a", "public-defrag-b", "public-defrag-c"] {
        pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
    }
    assert!(pool.remove_column("public-defrag-a").unwrap());
    pool.generation = u32::MAX;
    for slot in pool.slots.values_mut() {
        slot.generation = u32::MAX;
    }
    let before_public_defrag = pool_state(&pool, "public-defrag-b");
    assert_eq!(
        pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap_err(),
        AllocError::CounterExhausted {
            counter: "public generation"
        }
    );
    assert_eq!(pool_state(&pool, "public-defrag-b"), before_public_defrag);
}

#[test]
fn successful_stamp_transitions_separate_replacement_from_pool_relocation() {
    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    let first = col_f64(vec![1.0]);
    let replacement = col_f64(vec![2.0]);
    pool.add_column(
        "direct-target".into(),
        &first,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    pool.add_column(
        "direct-survivor".into(),
        &first,
        GpuAllocCtx::unbudgeted(&device, &queue),
    )
    .unwrap();
    let old_target_handle = pool.handle_for("direct-target").unwrap();
    let old_survivor_handle = pool.handle_for("direct-survivor").unwrap();
    let old_target_epoch = pool.allocation_epoch("direct-target").unwrap();
    let survivor_epoch = pool.allocation_epoch("direct-survivor").unwrap();
    let generation = pool.generation();
    let layout_generation = pool.layout_generation();

    let new_target_handle = pool
        .upsert_column(
            "direct-target".into(),
            &replacement,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();

    assert_eq!(pool.generation(), generation.checked_add(1).unwrap());
    assert_eq!(pool.layout_generation(), layout_generation);
    assert_ne!(
        pool.allocation_epoch("direct-target"),
        Some(old_target_epoch)
    );
    assert_eq!(
        pool.allocation_epoch("direct-survivor"),
        Some(survivor_epoch)
    );
    assert!(!pool.is_valid_handle(&old_target_handle));
    assert!(!pool.is_valid_handle(&old_survivor_handle));
    assert!(pool.is_valid_handle(&new_target_handle));
    assert!(pool.is_valid_handle(&pool.handle_for("direct-survivor").unwrap()));

    let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
        return;
    };
    for id in ["relocate-a", "relocate-b", "relocate-c", "relocate-d"] {
        pool.add_column(id.into(), &first, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
    }
    assert!(pool.remove_column("relocate-b").unwrap());
    assert!(pool.remove_column("relocate-d").unwrap());
    pool.defrag_policy = DefragPolicy::OnAllocFailure;
    let a_handle = pool.handle_for("relocate-a").unwrap();
    let c_handle = pool.handle_for("relocate-c").unwrap();
    let a_epoch = pool.allocation_epoch("relocate-a").unwrap();
    let c_epoch = pool.allocation_epoch("relocate-c").unwrap();
    let generation = pool.generation();
    let layout_generation = pool.layout_generation();
    let large = col_f64(vec![3.0; 33]);

    let new_handle = pool
        .upsert_column(
            "relocate-new".into(),
            &large,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();

    assert_eq!(pool.generation(), generation.checked_add(1).unwrap());
    assert_eq!(
        pool.layout_generation(),
        layout_generation.checked_add(1).unwrap()
    );
    assert_eq!(pool.allocation_epoch("relocate-a"), Some(a_epoch));
    assert_eq!(pool.allocation_epoch("relocate-c"), Some(c_epoch));
    assert!(pool.allocation_epoch("relocate-new").is_some());
    assert!(!pool.is_valid_handle(&a_handle));
    assert!(!pool.is_valid_handle(&c_handle));
    assert!(pool.is_valid_handle(&new_handle));
    assert!(pool.is_valid_handle(&pool.handle_for("relocate-a").unwrap()));
    assert!(pool.is_valid_handle(&pool.handle_for("relocate-c").unwrap()));
}

// ── Records and ragged columns ──────────────────────────────────────────────

fn box_fields<'a>(columns: &'a [(&'a str, Column<f64>)]) -> Vec<(&'a str, &'a dyn ColumnSource)> {
    columns
        .iter()
        .map(|(name, column)| (*name, column as &dyn ColumnSource))
        .collect()
}

fn five_fields(rows: usize, base: f64) -> Vec<(&'static str, Column<f64>)> {
    ["median", "q1", "q3", "whisker_low", "whisker_high"]
        .iter()
        .enumerate()
        .map(|(field, name)| {
            (
                *name,
                col_f64(
                    (0..rows)
                        .map(|row| base + field as f64 * 10.0 + row as f64)
                        .collect(),
                ),
            )
        })
        .collect()
}

fn invalid_reason(result: crate::Result<impl std::fmt::Debug>) -> String {
    match result {
        Err(crate::FiggyError::InvalidSeriesConfig { reason, .. }) => reason,
        other => panic!("expected InvalidSeriesConfig, got {other:?}"),
    }
}

/// A record's fields upload in one batch (one staging buffer), keep their
/// schema slots, and carry the statistics the write observer measured.
#[test]
fn a_record_uploads_its_fields_in_one_batch_with_statistics() {
    let Some((device, queue, mut pool)) = mk_pool(64 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let mut columns = five_fields(3, 1.0);
    columns.push(("mean", col_f64(vec![2.0, f64::NAN, f64::INFINITY])));
    let fields = box_fields(&columns);
    let creations = pool.buffer_creations();

    let id = pool
        .upsert_record(
            "box",
            &RecordSource {
                schema: BOX_PLOT_RECORD,
                fields: &fields,
            },
            ctx(),
        )
        .expect("record upsert");
    assert_eq!(id, "box");
    assert_eq!(
        pool.buffer_creations() - creations,
        1,
        "a record is one staging buffer"
    );

    let record = pool.record("box").expect("resident record");
    assert_eq!(record.slot().len, 3);
    assert_eq!(
        read_column_values(ctx(), &pool, record.field("median").unwrap()),
        [1.0, 2.0, 3.0]
    );
    assert_eq!(
        read_column_values(ctx(), &pool, record.field("whisker_high").unwrap()),
        [41.0, 42.0, 43.0]
    );
    assert!(record.field("ci_low").is_none(), "an absent optional field");
    assert!(record.field("no_such_field").is_none());
    assert_eq!(
        record.field_at(0).unwrap().offset,
        record.field("median").unwrap().offset,
        "slot 0 is the schema's first field"
    );

    let stats = pool.record_stats("box").unwrap();
    assert_eq!(stats, record.stats());
    assert_eq!(stats.len, 3);
    assert_eq!(
        stats.present,
        [
            true, true, true, true, true, true, false, false, false, false, false, false
        ]
    );
    assert_eq!(
        (stats.min[1], stats.max[1], stats.finite[1], stats.nan[1]),
        (11.0, 13.0, 3, 0)
    );
    // mean: one finite value, one NaN, one infinity (neither finite nor NaN).
    assert_eq!(
        (stats.min[5], stats.max[5], stats.finite[5], stats.nan[5]),
        (2.0, 2.0, 1, 1)
    );
    assert_eq!(stats.min[6], f64::INFINITY);
    assert_eq!(stats.max[6], f64::NEG_INFINITY);
    assert_eq!((stats.finite[6], stats.nan[6]), (0, 0));

    // The members are pool columns, in one region of the batch's layout.
    let members: Vec<&str> = record
        .slot()
        .fields
        .iter()
        .flatten()
        .map(String::as_str)
        .collect();
    assert_eq!(members.len(), 6);
    assert!(members.iter().all(|member| pool.is_member_column(member)));
    assert_eq!(
        record.slot().epoch,
        pool.allocation_epoch(members[0]).unwrap()
    );
    assert_eq!(pool.used_bytes(), 6 * ALIGN, "six one-block fields");
    assert_eq!(pool.composite_kind("box"), Some("a record"));
    assert!(!pool.is_member_column("box"));
}

/// Replacing a record (same id, new length) uploads the new fields, frees
/// the old ones, bumps the epoch and leaves only the new bytes resident.
#[test]
fn a_record_replacement_takes_a_new_length_and_a_new_epoch() {
    let Some((device, queue, mut pool)) = mk_pool(64 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let first = five_fields(3, 1.0);
    let fields = box_fields(&first);
    let source = RecordSource {
        schema: BOX_PLOT_RECORD,
        fields: &fields,
    };
    pool.upsert_record("box", &source, ctx()).unwrap();
    let old_epoch = pool.record("box").unwrap().slot().epoch;
    let old_members = pool.record("box").unwrap().slot().fields.clone();
    let old_generation = pool.generation();

    let second = five_fields(40, 100.0);
    let fields = box_fields(&second);
    pool.upsert_record(
        "box",
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
        ctx(),
    )
    .unwrap();

    let record = pool.record("box").unwrap();
    assert_eq!(record.slot().len, 40);
    assert_eq!(record.stats().len, 40);
    assert!(
        record.slot().epoch > old_epoch,
        "a replacement bumps the epoch"
    );
    assert!(
        pool.generation() > old_generation,
        "freed members invalidate handles"
    );
    for member in old_members.iter().flatten() {
        assert!(pool.slot(member).is_none(), "{member:?} was freed");
        assert!(!pool.is_member_column(member));
    }
    let median = read_column_values(ctx(), &pool, record.field("median").unwrap());
    assert_eq!(median.len(), 40);
    assert_eq!(median[39], 139.0);
    // 40 pairs = 320 bytes: two blocks per field, nothing else resident.
    assert_eq!(pool.used_bytes(), 5 * 2 * ALIGN);
}

/// Deleting a record frees every field and returns the pool to one free
/// region; deleting it again reports `false`.
#[test]
fn removing_a_record_or_ragged_column_frees_its_members() {
    let Some((device, queue, mut pool)) = mk_pool(16 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let columns = five_fields(4, 0.0);
    let fields = box_fields(&columns);
    pool.upsert_record(
        "box",
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
        ctx(),
    )
    .unwrap();
    let values = col_f64(vec![1.0, 2.0, 3.0]);
    let offsets = col_f64(vec![0.0, 1.0, 3.0]);
    pool.upsert_ragged(
        "outliers",
        &RaggedSource {
            values: &values,
            offsets: &offsets,
        },
        ctx(),
    )
    .unwrap();
    assert_eq!(pool.slots.len(), 7);

    assert!(pool.remove_record("box").unwrap());
    assert!(pool.record("box").is_none());
    assert!(pool.record_stats("box").is_none());
    assert!(!pool.remove_record("box").unwrap());
    assert_eq!(pool.slots.len(), 2);

    assert!(pool.remove_ragged("outliers").unwrap());
    assert!(pool.ragged("outliers").is_none());
    assert!(pool.ragged_stats("outliers").is_none());
    assert!(!pool.remove_ragged("outliers").unwrap());
    assert!(pool.slots.is_empty());
    assert!(pool.allocation_epochs.is_empty());
    assert_eq!(pool.used_bytes(), 0);
    assert_eq!(free_state(&pool.free), [(0, 16 * ALIGN)]);
}

/// The schema is checked by name and the lengths are checked before
/// anything is allocated; every rejection names its reason and leaves the
/// pool exactly as it was.
#[test]
fn record_schema_and_length_errors_are_explicit_and_change_nothing() {
    let Some((device, queue, mut pool)) = mk_pool(64 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let other = col_f64(vec![1.0]);
    pool.add_column("taken".into(), &other, ctx()).unwrap();
    let values = col_f64(vec![1.0]);
    let offsets = col_f64(vec![0.0, 1.0]);
    pool.upsert_ragged(
        "ragged",
        &RaggedSource {
            values: &values,
            offsets: &offsets,
        },
        ctx(),
    )
    .unwrap();
    let before = full_pool_state(&pool);

    let base = five_fields(3, 0.0);
    let short = col_f64(vec![1.0, 2.0]);
    let empty = col_f64(Vec::new());
    let mut cases: Vec<(&str, Vec<(&str, &dyn ColumnSource)>, &str)> = Vec::new();
    let mut unknown = box_fields(&base);
    unknown.push(("mode", &short));
    cases.push(("box", unknown, "'mode' is not a field"));
    let mut repeated = box_fields(&base);
    repeated.push(("median", &short));
    cases.push(("box", repeated, "given twice"));
    let missing: Vec<_> = box_fields(&base)
        .into_iter()
        .filter(|(name, _)| *name != "q3")
        .collect();
    cases.push(("box", missing, "required record field 'q3' is missing"));
    let mut mismatch = box_fields(&base);
    mismatch.push(("mean", &short));
    cases.push(("box", mismatch, "'mean' has 2 rows but 'median' has 3"));
    let empties: Vec<(&str, &dyn ColumnSource)> =
        ["median", "q1", "q3", "whisker_low", "whisker_high"]
            .iter()
            .map(|name| (*name, &empty as &dyn ColumnSource))
            .collect();
    cases.push(("box", empties, "at least one row"));
    cases.push(("taken", box_fields(&base), "the id is a column"));
    cases.push(("ragged", box_fields(&base), "the id is a ragged column"));
    cases.push(("", box_fields(&base), "must not be empty"));
    cases.push(("a\u{1f}b", box_fields(&base), "U+001F"));

    for (id, fields, expected) in &cases {
        let reason = invalid_reason(pool.upsert_record(
            *id,
            &RecordSource {
                schema: BOX_PLOT_RECORD,
                fields,
            },
            ctx(),
        ));
        assert!(
            reason.contains(expected),
            "{id:?}: {reason:?} lacks {expected:?}"
        );
        assert_eq!(full_pool_state(&pool), before, "{expected}: pool changed");
        assert!(pool.record(id).is_none());
    }

    // A schema of optional fields only still needs one field.
    const OPTIONAL: &RecordSchema = &[("a", false), ("b", false)];
    let reason = invalid_reason(pool.upsert_record(
        "optional",
        &RecordSource {
            schema: OPTIONAL,
            fields: &[],
        },
        ctx(),
    ));
    assert!(reason.contains("at least one field"), "{reason}");
    assert_eq!(full_pool_state(&pool), before);
}

/// `MAX_RECORD_FIELDS` bounds both the schema and the source.
#[test]
fn a_record_holds_at_most_max_record_fields() {
    let Some((device, queue, mut pool)) = mk_pool(64 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    const NAMES: [&str; 13] = [
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m",
    ];
    const WIDE: &RecordSchema = &[
        ("a", true),
        ("b", false),
        ("c", false),
        ("d", false),
        ("e", false),
        ("f", false),
        ("g", false),
        ("h", false),
        ("i", false),
        ("j", false),
        ("k", false),
        ("l", false),
        ("m", false),
    ];
    let column = col_f64(vec![1.0, 2.0]);
    let fields: Vec<(&str, &dyn ColumnSource)> = NAMES
        .iter()
        .map(|name| (*name, &column as &dyn ColumnSource))
        .collect();
    let reason = invalid_reason(pool.upsert_record(
        "wide",
        &RecordSource {
            schema: WIDE,
            fields: &fields[..1],
        },
        ctx(),
    ));
    assert!(
        reason.contains("schema has 13 fields") && reason.contains("MAX_RECORD_FIELDS (12)"),
        "{reason}"
    );

    let full: &RecordSchema = &WIDE[..MAX_RECORD_FIELDS];
    let reason = invalid_reason(pool.upsert_record(
        "wide",
        &RecordSource {
            schema: full,
            fields: &fields,
        },
        ctx(),
    ));
    assert!(reason.contains("source has 13 fields"), "{reason}");
    assert!(pool.slots.is_empty());

    // Exactly MAX_RECORD_FIELDS fields fit.
    pool.upsert_record(
        "wide",
        &RecordSource {
            schema: full,
            fields: &fields[..MAX_RECORD_FIELDS],
        },
        ctx(),
    )
    .unwrap();
    let stats = pool.record_stats("wide").unwrap();
    assert_eq!(stats.present, [true; MAX_RECORD_FIELDS]);
    assert_eq!(pool.slots.len(), MAX_RECORD_FIELDS);
}

/// A failure after the batch was staged rolls the batch back: an
/// out-of-space replacement and a failing dependent preparation both leave
/// the previous record, its values and the allocator exactly as they were.
#[test]
fn a_failed_record_upsert_rolls_the_batch_back() {
    let Some((device, queue, mut pool)) = mk_pool(10 * ALIGN) else {
        return;
    };
    // A fixed pool: this test is about what a pool that cannot grow does.
    pool.growth_policy = GrowthPolicy::Fixed;
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let first = five_fields(3, 1.0);
    let fields = box_fields(&first);
    pool.upsert_record(
        "box",
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
        ctx(),
    )
    .unwrap();
    let before = full_pool_state(&pool);
    let stats = pool.record_stats("box").unwrap();
    let epoch = pool.record("box").unwrap().slot().epoch;

    // Ten blocks of new fields do not fit beside the five live ones.
    let wide = five_fields(40, 0.0);
    let fields = box_fields(&wide);
    let result = pool.upsert_record(
        "box",
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
        ctx(),
    );
    assert!(
        matches!(
            result,
            Err(crate::FiggyError::Pool(AllocError::OutOfSpace { .. }))
        ),
        "{result:?}"
    );
    assert_eq!(full_pool_state(&pool), before);
    assert_eq!(pool.record_stats("box"), Some(stats));

    // The batch fits, but the dependent preparation fails after staging.
    let same = five_fields(3, 50.0);
    let fields = box_fields(&same);
    let result = pool.upsert_record_with(
        "box".into(),
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
        ctx(),
        |provisional, relocated| {
            assert!(!relocated);
            let record_members = provisional.slots.len();
            assert_eq!(
                record_members, 5,
                "the provisional pool holds only the new fields"
            );
            Err::<(), _>(crate::FiggyError::InvalidSeriesConfig {
                series_id: "box".into(),
                reason: "injected".into(),
            })
        },
    );
    assert_eq!(invalid_reason(result), "injected");
    assert_eq!(full_pool_state(&pool), before);
    let record = pool.record("box").unwrap();
    assert_eq!(record.slot().epoch, epoch);
    assert_eq!(
        read_column_values(ctx(), &pool, record.field("median").unwrap()),
        [1.0, 2.0, 3.0]
    );
}

/// A ragged column's values and offsets upload in one batch; the observer
/// measures the values' range and counts, the offsets' monotonicity and the
/// longest group in the same pass.
#[test]
fn a_ragged_column_uploads_with_offset_statistics() {
    let Some((device, queue, mut pool)) = mk_pool(16 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let values = col_f64(vec![4.0, -2.0, f64::NAN, 9.0, 1.0]);
    let offsets = col_f64(vec![0.0, 2.0, 2.0, 5.0]);
    let creations = pool.buffer_creations();
    let id = pool
        .upsert_ragged(
            "outliers",
            &RaggedSource {
                values: &values,
                offsets: &offsets,
            },
            ctx(),
        )
        .unwrap();
    assert_eq!(id, "outliers");
    assert_eq!(pool.buffer_creations() - creations, 1, "one staging buffer");

    let ragged = pool.ragged("outliers").unwrap();
    assert_eq!((ragged.slot().groups, ragged.slot().total), (3, 5));
    assert_eq!(
        ragged.stats(),
        RaggedStats {
            groups: 3,
            total: 5,
            min: -2.0,
            max: 9.0,
            finite: 4,
            nan: 1,
            non_decreasing: true,
            max_group: 3,
        }
    );
    assert_eq!(pool.ragged_stats("outliers"), Some(ragged.stats()));
    // The offsets are `u32` index lanes: the index, then 0.
    assert_eq!(
        read_column_words(ctx(), &pool, ragged.offsets()),
        [0, 0, 2, 0, 2, 0, 5, 0]
    );
    let read = read_column_values(ctx(), &pool, ragged.values());
    assert_eq!((read[0], read[1], read[3], read[4]), (4.0, -2.0, 9.0, 1.0));
    assert!(read[2].is_nan());
    assert_eq!(
        ragged.slot().epoch,
        pool.allocation_epoch(&ragged.slot().values).unwrap()
    );
    assert_eq!(pool.composite_kind("outliers"), Some("a ragged column"));
    assert!(pool.is_member_column(&ragged.slot().offsets));
    assert_eq!(pool.used_bytes(), 2 * ALIGN);
}

/// Offsets that decrease, do not start at 0, do not end at the value count
/// or are not integers are rejected after the observer measured them; the
/// abandoned batch leaves the pool exactly as it was, an old ragged column
/// included.
#[test]
fn invalid_ragged_offsets_are_rejected_and_roll_the_batch_back() {
    let Some((device, queue, mut pool)) = mk_pool(16 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let values = col_f64(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let good = col_f64(vec![0.0, 5.0]);
    pool.upsert_ragged(
        "outliers",
        &RaggedSource {
            values: &values,
            offsets: &good,
        },
        ctx(),
    )
    .unwrap();
    let before = full_pool_state(&pool);
    let stats = pool.ragged_stats("outliers").unwrap();

    let cases: [(&[f64], &str); 6] = [
        (&[0.0, 3.0, 2.0, 5.0], "non-decreasing"),
        (&[1.0, 3.0, 5.0], "start at 0"),
        (
            &[0.0, 2.0, 4.0],
            "last ragged offset is 4 but there are 5 values",
        ),
        (&[0.0, 2.5, 5.0], "finite non-negative integers"),
        (&[0.0, f64::NAN, 5.0], "finite non-negative integers"),
        (&[0.0], "at least two"),
    ];
    for (offsets, expected) in cases {
        let offsets = col_f64(offsets.to_vec());
        for id in ["outliers", "fresh"] {
            let reason = invalid_reason(pool.upsert_ragged(
                id,
                &RaggedSource {
                    values: &values,
                    offsets: &offsets,
                },
                ctx(),
            ));
            assert!(
                reason.contains(expected),
                "{id}: {reason:?} lacks {expected:?}"
            );
            assert_eq!(full_pool_state(&pool), before, "{expected}");
        }
    }
    let empty = col_f64(Vec::new());
    let reason = invalid_reason(pool.upsert_ragged(
        "fresh",
        &RaggedSource {
            values: &empty,
            offsets: &good,
        },
        ctx(),
    ));
    assert!(reason.contains("at least one value"), "{reason}");
    assert!(pool.ragged("fresh").is_none());
    assert_eq!(pool.ragged_stats("outliers"), Some(stats));
    let ragged = pool.ragged("outliers").unwrap();
    assert_eq!(
        read_column_values(ctx(), &pool, ragged.values()),
        [1.0, 2.0, 3.0, 4.0, 5.0]
    );
}

/// A ragged replacement takes a new group count and a new epoch and frees
/// the old members; a record and a ragged column cannot share an id.
#[test]
fn a_ragged_replacement_takes_new_groups_and_a_new_epoch() {
    let Some((device, queue, mut pool)) = mk_pool(16 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let values = col_f64(vec![1.0, 2.0]);
    let offsets = col_f64(vec![0.0, 1.0, 2.0]);
    pool.upsert_ragged(
        "outliers",
        &RaggedSource {
            values: &values,
            offsets: &offsets,
        },
        ctx(),
    )
    .unwrap();
    let old = pool.ragged("outliers").unwrap().slot().clone();

    let values = col_f64((0..70).map(f64::from).collect());
    let offsets = col_f64(vec![0.0, 0.0, 70.0, 70.0]);
    pool.upsert_ragged(
        "outliers",
        &RaggedSource {
            values: &values,
            offsets: &offsets,
        },
        ctx(),
    )
    .unwrap();
    let new = pool.ragged("outliers").unwrap();
    assert!(new.slot().epoch > old.epoch);
    assert_eq!(
        (new.stats().groups, new.stats().total, new.stats().max_group),
        (3, 70, 70)
    );
    assert!(pool.slot(&old.values).is_none() && pool.slot(&old.offsets).is_none());
    // 70 pairs = 560 bytes: three blocks of values, one of offsets.
    assert_eq!(pool.used_bytes(), 4 * ALIGN);

    let columns = five_fields(2, 0.0);
    let fields = box_fields(&columns);
    let reason = invalid_reason(pool.upsert_record(
        "outliers",
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
        ctx(),
    ));
    assert!(reason.contains("the id is a ragged column"), "{reason}");
    pool.upsert_record(
        "box",
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
        ctx(),
    )
    .unwrap();
    let reason = invalid_reason(pool.upsert_ragged(
        "box",
        &RaggedSource {
            values: &values,
            offsets: &offsets,
        },
        ctx(),
    ));
    assert!(reason.contains("the id is a record"), "{reason}");
}

/// The monitor counts a record's and a ragged column's member columns like
/// any column: used bytes, one staging buffer per upload, the staging peak,
/// and the freed bytes after removal.
#[test]
fn the_monitor_accounts_records_and_ragged_columns() {
    let Some((device, queue, mut pool)) = mk_pool(32 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let creations = pool.buffer_creations();
    let columns = five_fields(40, 0.0);
    let fields = box_fields(&columns);
    pool.upsert_record(
        "box",
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
        ctx(),
    )
    .unwrap();
    let record_bytes = 5 * 2 * ALIGN;
    assert_eq!(pool.used_bytes(), record_bytes);
    assert_eq!(pool.free_bytes(), 32 * ALIGN - record_bytes);
    assert_eq!(pool.buffer_creations() - creations, 1);
    assert!(
        pool.peak_bytes() >= pool.gpu_bytes() + record_bytes,
        "the staging buffer of the whole record coexists with the pool"
    );

    let values = col_f64((0..33).map(f64::from).collect());
    let offsets = col_f64(vec![0.0, 33.0]);
    pool.upsert_ragged(
        "outliers",
        &RaggedSource {
            values: &values,
            offsets: &offsets,
        },
        ctx(),
    )
    .unwrap();
    let ragged_bytes = 2 * ALIGN + ALIGN;
    assert_eq!(pool.used_bytes(), record_bytes + ragged_bytes);
    assert_eq!(pool.buffer_creations() - creations, 2);
    assert_eq!(
        pool.gpu_bytes(),
        32 * ALIGN,
        "members live in the one primary buffer"
    );

    pool.remove_record("box").unwrap();
    assert_eq!(pool.used_bytes(), ragged_bytes);
    pool.remove_ragged("outliers").unwrap();
    assert_eq!(pool.used_bytes(), 0);
    assert_eq!(pool.free_bytes(), 32 * ALIGN);
    assert_eq!(
        pool.buffer_creations() - creations,
        2,
        "removal creates no buffer"
    );
}

/// Defragmentation moves member columns like any column; the record and the
/// ragged column follow because they name their members, not offsets.
#[test]
fn records_and_ragged_columns_survive_defragmentation_and_clear() {
    let Some((device, queue, mut pool)) = mk_pool(32 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let filler = col_f64(vec![0.0; 100]);
    pool.add_column("filler".into(), &filler, ctx()).unwrap();
    let columns = five_fields(3, 1.0);
    let fields = box_fields(&columns);
    pool.upsert_record(
        "box",
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
        ctx(),
    )
    .unwrap();
    let before = pool.record("box").unwrap().field("median").unwrap().offset;
    pool.remove_column("filler").unwrap();
    assert!(pool.defragment(ctx()).unwrap());
    let record = pool.record("box").unwrap();
    let after = record.field("median").unwrap();
    assert!(after.offset < before, "the member moved");
    assert_eq!(read_column_values(ctx(), &pool, after), [1.0, 2.0, 3.0]);

    pool.clear().unwrap();
    assert!(pool.record("box").is_none());
    assert!(pool.composite_kind("box").is_none());
}

/// A shader reads a record field and a ragged group by their pool lane
/// bases (`offset / 4`), the offsets as `u32` index lanes: the convention
/// `SHADER_COMMON.md` states for records and ragged columns.
#[test]
fn a_shader_reads_a_record_field_and_a_ragged_group_by_pool_offset() {
    let Some((device, queue, mut pool)) = mk_pool(16 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let columns = five_fields(3, 1.5);
    let fields = box_fields(&columns);
    pool.upsert_record(
        "box",
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
        ctx(),
    )
    .unwrap();
    let values = col_f64(vec![10.0, 20.0, 30.0, 40.0, 50.0]);
    let offsets = col_f64(vec![0.0, 2.0, 2.0, 5.0]);
    pool.upsert_ragged(
        "outliers",
        &RaggedSource {
            values: &values,
            offsets: &offsets,
        },
        ctx(),
    )
    .unwrap();
    let record = pool.record("box").unwrap();
    let ragged = pool.ragged("outliers").unwrap();
    let lane = |handle: ColumnHandle| (handle.offset / 4) as u32;
    // (field lane base, row, values lane base, offsets lane base, group, pad)
    let params: [u32; 8] = [
        lane(record.field("q3").unwrap()),
        2,
        lane(ragged.values()),
        lane(ragged.offsets()),
        2,
        0,
        0,
        0,
    ];

    const SHADER: &str = r#"
struct Params { field_base: u32, row: u32, values_base: u32, offsets_base: u32, group: u32 }
@group(0) @binding(0) var<storage, read> pool: array<f32>;
@group(0) @binding(1) var<storage, read> params: Params;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

fn pair_at(base: u32, index: u32) -> f32 {
    return pool[base + 2u * index] + pool[base + 2u * index + 1u];
}

fn offset_at(base: u32, index: u32) -> u32 {
    return bitcast<u32>(pool[base + 2u * index]);
}

@compute @workgroup_size(1)
fn main() {
    out[0] = pair_at(params.field_base, params.row);
    let start = offset_at(params.offsets_base, params.group);
    let end = offset_at(params.offsets_base, params.group + 1u);
    out[1] = f32(end - start);
    for (var i = 0u; i < 4u; i++) {
        if (start + i < end) {
            out[2u + i] = pair_at(params.values_base, start + i);
        } else {
            out[2u + i] = -1.0;
        }
    }
}
"#;
    let read: Vec<f32> = run_pool_read_back(&device, &queue, &pool, SHADER, &params, 6)
        .into_iter()
        .map(f32::from_bits)
        .collect();
    // q3 of row 2 is 1.5 + 20 + 2; group 2 is values[2..5].
    assert_eq!(read, [23.5, 3.0, 30.0, 40.0, 50.0, -1.0]);
}

/// Run the one-workgroup compute `shader` over the pool (binding 0, as
/// `array<f32>`), `params` (binding 1) and an output of `out_words` words
/// (binding 2), and read the output back as `u32` bits.
fn run_pool_read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pool: &ColumnPool,
    shader: &str,
    params: &[u32],
    out_words: u64,
) -> Vec<u32> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("figgy record read-back test"),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("figgy record read-back test"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let params_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("figgy record read-back params"),
        size: (params.len() * 4) as u64,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&params_buffer, 0, bytemuck::cast_slice(params));
    let out_bytes = out_words * 4;
    let out = device.create_buffer(&BufferDescriptor {
        label: Some("figgy record read-back out"),
        size: out_bytes,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("figgy record read-back readback"),
        size: out_bytes,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy record read-back test"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pool.buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: out.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("figgy record read-back test"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("figgy record read-back test"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&out, 0, &readback, 0, out_bytes);
    let (sender, receiver) = std::sync::mpsc::channel();
    encoder.map_buffer_on_submit(
        &readback,
        wgpu::MapMode::Read,
        0..out_bytes,
        move |result| {
            let _ = sender.send(result);
        },
    );
    let submission = queue.submit(std::iter::once(encoder.finish()));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("read-back poll");
    receiver
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("read-back callback")
        .expect("map read-back");
    let mapped = readback
        .slice(..out_bytes)
        .get_mapped_range()
        .expect("read-back is mapped after map_async");
    let read: Vec<u32> = mapped
        .chunks_exact(4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .collect();
    drop(mapped);
    readback.unmap();
    read
}

/// Ragged offsets at and past 2^24, where the f32 offsets the `u32` index
/// lanes replace lost integers, up to the end of the `u32` index space.
const INDEX_LANE_PROBES: [u32; 6] = [
    0,
    (1 << 24) - 1,
    1 << 24,
    (1 << 24) + 1,
    u32::MAX - 1,
    u32::MAX,
];

/// A column whose pairs are the exact `(hi, lo)` split of its values, as the
/// document conversion and the WASM wrapper write ragged offsets.
struct SplitPairSource<'a>(&'a [f64]);

impl ColumnSource for SplitPairSource<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn min(&self) -> f64 {
        self.0.iter().copied().fold(f64::INFINITY, f64::min)
    }

    fn max(&self) -> f64 {
        self.0.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    }

    fn write_f32_le_into(&self, dst: &mut [u8]) {
        for (chunk, value) in dst.chunks_exact_mut(4).zip(self.0) {
            chunk.copy_from_slice(&(*value as f32).to_le_bytes());
        }
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        for (index, &value) in self.0.iter().enumerate() {
            let (hi, lo) = crate::data::split_f64_to_f32_pair(value);
            dst.write_pair(index, hi, lo);
        }
        ColumnUploadStats { min_positive: None }
    }
}

/// The CPU write of an index lane: the `(hi, lo)` split of every probe is
/// stored as its exact `u32` bits and a zero lane, where the f32 read it
/// replaces, `u32(hi + lo)`, loses 2^24 + 1 and saturates at `u32::MAX - 1`.
/// Without index lanes the writer still stores the f32 pair.
/// A `Column<f64>` registered through `ColumnSource` writes its
/// (hi, lo) split (#107): an odd offset past 2^24 reaches the index lane
/// exactly, as it does from a hi/lo source, and a value that is not an f32
/// keeps its low part.
#[test]
fn f64_columns_upload_their_exact_split() {
    let probes: [f64; 4] = [
        f64::from((1u32 << 24) + 1),
        f64::from(u32::MAX - 1),
        0.1,
        -1.0e10 + 0.25,
    ];
    let column = col_f64(probes.to_vec());
    let mut lanes = [0u8; 4 * COLUMN_VALUE_BYTES];
    write_scalar_source_as_pairs(
        &column,
        ColumnPairWriter::from_bytes_for_test(&mut lanes).with_index_lanes(true),
    );
    let word =
        |bytes: &[u8], k: usize| u32::from_le_bytes(bytes[4 * k..4 * k + 4].try_into().unwrap());
    assert_eq!(
        word(&lanes, 0),
        (1 << 24) + 1,
        "an odd offset past 2^24 is exact"
    );
    assert_eq!(word(&lanes, 2), u32::MAX - 1);
    let mut pairs = [0u8; 4 * COLUMN_VALUE_BYTES];
    write_scalar_source_as_pairs(&column, ColumnPairWriter::from_bytes_for_test(&mut pairs));
    let float = |k: usize| f32::from_bits(word(&pairs, k));
    // The split keeps about 48 significant bits: integers below 2^48 are
    // exact, any other value is within 2^-47 of itself, where an f32 alone
    // keeps 2^-24.
    for (k, &probe) in probes.iter().enumerate() {
        let (hi, lo) = (float(2 * k), float(2 * k + 1));
        let gpu = f64::from(hi) + f64::from(lo);
        if probe.fract() == 0.0 && probe.abs() < 2f64.powi(48) {
            assert_eq!(gpu, probe, "probe {k}: hi {hi}, lo {lo}");
        }
        assert!(
            (gpu - probe).abs() <= probe.abs() * 2f64.powi(-47),
            "probe {k}: {gpu} vs {probe}"
        );
        assert!(
            (gpu - probe).abs() <= (f64::from(probe as f32) - probe).abs(),
            "probe {k}: the split is no worse than f32"
        );
    }
    assert_ne!(float(5), 0.0, "0.1 keeps a low part");
}

#[test]
fn index_lanes_store_offsets_exactly_at_and_past_2_pow_24() {
    let values = INDEX_LANE_PROBES.map(f64::from);
    let source = SplitPairSource(&values);
    let mut bytes = [0u8; INDEX_LANE_PROBES.len() * COLUMN_VALUE_BYTES];
    write_scalar_source_as_pairs(
        &source,
        ColumnPairWriter::from_bytes_for_test(&mut bytes).with_index_lanes(true),
    );
    let word = |bytes: &[u8], k: usize| {
        u32::from_le_bytes([
            bytes[4 * k],
            bytes[4 * k + 1],
            bytes[4 * k + 2],
            bytes[4 * k + 3],
        ])
    };
    for (k, &probe) in INDEX_LANE_PROBES.iter().enumerate() {
        assert_eq!((word(&bytes, 2 * k), word(&bytes, 2 * k + 1)), (probe, 0));
        let (hi, lo) = crate::data::split_f64_to_f32_pair(f64::from(probe));
        assert_eq!(crate::data::index_lane(hi, lo), probe);
    }
    let f32_read = |probe: u32| {
        let (hi, lo) = crate::data::split_f64_to_f32_pair(f64::from(probe));
        (hi + lo) as u32
    };
    assert_eq!(
        f32_read((1 << 24) + 1),
        1 << 24,
        "the f32 read loses 2^24 + 1"
    );
    assert_eq!(
        f32_read(u32::MAX - 1),
        u32::MAX,
        "and saturates near u32::MAX"
    );

    let mut pairs = [0u8; INDEX_LANE_PROBES.len() * COLUMN_VALUE_BYTES];
    write_scalar_source_as_pairs(&source, ColumnPairWriter::from_bytes_for_test(&mut pairs));
    let (hi, lo) = crate::data::split_f64_to_f32_pair(f64::from((1u32 << 24) + 1));
    assert_eq!(
        (word(&pairs, 6), word(&pairs, 7)),
        (hi.to_bits(), lo.to_bits()),
        "a column without index lanes keeps its f32 pairs"
    );
}

/// The GPU read of an index lane: the pool stores the probes of an index
/// column (a ragged column's offsets) as `u32` lanes and a shader reads them
/// back exactly with `bitcast<u32>`. The same probes as an ordinary column,
/// read the way ragged offsets were read before (`u32(hi + lo)`), are the
/// control: that read loses 2^24 + 1.
#[test]
fn a_shader_reads_index_lanes_exactly_past_2_pow_24() {
    let Some((device, queue, mut pool)) = mk_pool(16 * ALIGN) else {
        return;
    };
    let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
    let values = INDEX_LANE_PROBES.map(f64::from);
    let source = SplitPairSource(&values);
    let columns: [(&str, &dyn ColumnSource); 2] = [("lanes", &source), ("pairs", &source)];
    pool.begin_add_columns_observed(&columns, ctx(), None, Some(0))
        .unwrap()
        .commit();
    let lanes = pool.handle_for("lanes").unwrap();
    let pairs = pool.handle_for("pairs").unwrap();
    let words = read_column_words(ctx(), &pool, lanes);
    for (k, &probe) in INDEX_LANE_PROBES.iter().enumerate() {
        assert_eq!((words[2 * k], words[2 * k + 1]), (probe, 0));
    }
    let count = INDEX_LANE_PROBES.len() as u32;
    let params = [
        (lanes.offset / 4) as u32,
        (pairs.offset / 4) as u32,
        count,
        0,
    ];

    const SHADER: &str = r#"
struct Params { lanes: u32, pairs: u32, count: u32 }
@group(0) @binding(0) var<storage, read> pool: array<f32>;
@group(0) @binding(1) var<storage, read> params: Params;
@group(0) @binding(2) var<storage, read_write> out: array<u32>;

@compute @workgroup_size(1)
fn main() {
    for (var i = 0u; i < params.count; i++) {
        out[i] = bitcast<u32>(pool[params.lanes + 2u * i]);
        out[params.count + i] = u32(pool[params.pairs + 2u * i] + pool[params.pairs + 2u * i + 1u]);
    }
}
"#;
    let read = run_pool_read_back(&device, &queue, &pool, SHADER, &params, 2 * count as u64);
    assert_eq!(read[..INDEX_LANE_PROBES.len()], INDEX_LANE_PROBES);
    assert_ne!(
        read[INDEX_LANE_PROBES.len() + 3],
        (1 << 24) + 1,
        "the f32 read loses 2^24 + 1 on the GPU: {:?}",
        &read[INDEX_LANE_PROBES.len()..]
    );
}

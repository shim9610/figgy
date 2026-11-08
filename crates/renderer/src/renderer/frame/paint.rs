//! Painting prepared frames and the per-item paint, with the registered styles.

use super::*;

impl Renderer {
    /// Immutable half of the frame: pure command recording against a
    /// [`PreparedFrame`] from [`Self::prepare`].
    ///
    /// `&self` — safe to call from host paint callbacks that only hand out
    /// shared access (egui `CallbackTrait::paint` via `&CallbackResources`,
    /// iced `shader::Primitive::draw` via `&Storage`), with no lock around
    /// the renderer. Repeatable: the same token may be recorded into more
    /// than one pass (egui re-record, iced mid-frame pass restart).
    ///
    /// All chart/view/series/style input comes from `prepared`; the host does
    /// not reconstruct or pass it again. Renderer-level stamps detect
    /// invalidating `&mut self` calls since `prepare`. On mismatch nothing is
    /// recorded and [`FiggyError::StalePreparedFrame`] is returned; recovery
    /// is a fresh `prepare`. A recorded command buffer is outside Rust's
    /// borrow graph, so the host must submit it before mutating or preparing
    /// the same [`ChartView`] again; validation can only run while recording.
    pub fn paint_prepared(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        prepared: &PreparedFrame,
    ) -> Result<()> {
        self.health.check()?;
        self.validate_prepared(prepared)?;
        self.paint_prepared_items(pass, target_size, &prepared.items, true, &|_| {
            super::composite::DataSteps::ALL
        })
    }

    /// Serial of the cached series bodies of `view`'s panel, `None` while it
    /// has none: the count of draws a test can read per panel.
    #[cfg(test)]
    pub(crate) fn resident_display_serial(&self, view: &ChartView) -> Option<u64> {
        let id = Arc::as_ptr(&view.content_revision) as usize;
        self.resident_displays
            .iter()
            .find(|display| display.view.as_ptr() as usize == id)
            .map(|display| display.serial)
    }

    #[cfg(test)]
    pub(crate) fn resident_display_count(&self) -> usize {
        self.resident_displays.len()
    }

    /// Stamp checks guarding `paint_prepared` — see [`PreparedFrame`] field
    /// docs for what each stamp catches. Kept separate from recording so a
    /// stale token fails before any pass state is touched.
    pub(in crate::renderer) fn validate_prepared(&self, prepared: &PreparedFrame) -> Result<()> {
        let stale = |reason: String| FiggyError::StalePreparedFrame { reason };
        if prepared.renderer_identity != self.renderer_identity {
            return Err(stale(
                "prepared frame belongs to a different renderer".into(),
            ));
        }
        if prepared.target_pipeline_generation != self.target_pipeline_generation {
            return Err(stale(format!(
                "prepared at target pipeline generation {}, renderer is now at {} \
                 (the target format or sample count changed between prepare and paint_prepared)",
                prepared.target_pipeline_generation, self.target_pipeline_generation
            )));
        }
        for stamp in &prepared.registered_charts {
            let unchanged = self.chart_states.get(&stamp.chart_id).is_some_and(|state| {
                state.revisions.desired == stamp.desired && state.revisions.data == stamp.data
            });
            if !unchanged {
                return Err(stale(format!(
                    "registered chart {:?} changed or was removed between prepare and paint_prepared",
                    stamp.chart_id,
                )));
            }
        }
        if prepared.pool_layout_generation != self.pool.layout_generation() {
            return Err(stale(format!(
                "prepared at pool layout generation {}, pool is now at {} \
                 (the pool layout changed between prepare and paint_prepared)",
                prepared.pool_layout_generation,
                self.pool.layout_generation()
            )));
        }
        for (idx, item) in prepared.items.iter().enumerate() {
            let current = item
                .view
                .content_revision
                .load(std::sync::atomic::Ordering::Acquire);
            if current != item.view.expected_content_revision {
                return Err(stale(format!(
                    "panel {idx} view changed between prepare and paint_prepared"
                )));
            }
            if let Some(display) = &item.resident_display
                && !self.resident_displays.iter().any(|held| {
                    held.view.as_ptr() as usize == display.view && held.serial == display.serial
                })
            {
                return Err(stale(format!(
                    "panel {idx} cached composite was drawn again after prepare"
                )));
            }
            if let Some(display) = &item.stream_display
                && !self.stream_display_packet_is_current(display)
            {
                return Err(stale(format!(
                    "panel {idx} stream display changed or was cancelled after prepare"
                )));
            }
        }
        for (id, allocation_epoch) in &prepared.column_sources {
            if self.pool.allocation_epoch(id) != Some(*allocation_epoch) {
                return Err(stale(format!(
                    "column {:?} changed allocation between prepare and paint_prepared",
                    id
                )));
            }
        }
        // A field's grid columns, walked from the shared receipt: reads only, so
        // a 5000-column matrix costs no allocation to validate.
        for sources in &prepared.grid_sources {
            for (id, allocation_epoch) in sources.ids.iter().zip(&sources.epochs) {
                if self.pool.allocation_epoch(id) != Some(*allocation_epoch) {
                    return Err(stale(format!(
                        "grid column {:?} changed allocation between prepare and paint_prepared",
                        id
                    )));
                }
            }
        }
        Ok(())
    }

    /// Read the instance-count word from a prepared contour label's indirect
    /// draw arguments. This exists only for native unit tests and wasm browser
    /// conformance tests, where pixels cannot distinguish all 1024 instances.
    #[cfg(any(test, target_arch = "wasm32"))]
    #[doc(hidden)]
    pub async fn contour_label_instance_count_for_test(
        &self,
        prepared: &PreparedFrame,
        item_index: usize,
        series_index: usize,
    ) -> Result<Option<u32>> {
        self.validate_prepared(prepared)?;
        let Some(indirect) = prepared
            .items
            .get(item_index)
            .and_then(|item| item.series.get(series_index))
            .and_then(|series| series.contour.as_ref())
            .and_then(|contour| contour.label.as_ref())
            .map(|label| label.snapshot.indirect())
        else {
            return Ok(None);
        };

        // gpu-alloc: Readback
        let readback = TrackedBuffer::new(
            &self.gpu_ledger,
            GpuResourceKind::Readback,
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("figgy contour label indirect count readback"),
                size: 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy contour label indirect count copy"),
            });
        encoder.copy_buffer_to_buffer(indirect, 4, &readback, 0, 4);
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = futures_channel::oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            if tx.send(result).is_err() {
                // The receiver was dropped: the waiting call was cancelled.
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        gpu_poll(
            &self.health,
            &self.device,
            wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            },
        )?;
        rx.await
            .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                resource: "contour label indirect count readback",
                reason: format!("map_async sender dropped: {error}"),
            })?
            .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                resource: "contour label indirect count readback",
                reason: format!("map_async: {error:?}"),
            })?;
        let mapped =
            slice
                .get_mapped_range()
                .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                    resource: "contour label indirect count readback",
                    reason: format!("get_mapped_range: {error:?}"),
                })?;
        let count = u32::from_le_bytes(
            *mapped
                .first_chunk::<4>()
                .ok_or(FiggyError::internal("four-byte readback"))?,
        );
        drop(mapped);
        readback.unmap();
        Ok(Some(count))
    }

    pub(in crate::renderer) fn prepare_arc_items(
        &mut self,
        items: &[ChartDrawItem<'_>],
    ) -> Result<Vec<PreparedArcItem>> {
        self.preparation_parts().1.prepare_arc_items(items)
    }

    pub(in crate::renderer) fn resolve_prepared_items(
        &mut self,
        items: &[ChartDrawItem<'_>],
        prepared_arcs: &[PreparedArcItem],
        prepared_contours: &[PreparedContourItem],
        pipelines: &TargetPipelines,
    ) -> Result<(Vec<PreparedItem>, HashMap<ColumnId, u64>)> {
        self.preparation_parts().1.resolve_prepared_items(
            items,
            prepared_arcs,
            prepared_contours,
            pipelines,
        )
    }

    pub(in crate::renderer) fn paint_prepared_items<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        prepared: &'a [PreparedItem],
        chrome: bool,
        steps: &dyn Fn(usize) -> super::composite::DataSteps,
    ) -> Result<()> {
        for item in prepared {
            if let Some(display) = &item.stream_display
                && display.size != target_size
            {
                return Err(FiggyError::StalePreparedFrame {
                    reason: "stream display size differs from paint target".into(),
                });
            }
        }
        // Every panel's grid and data, then every panel's decoration
        // (`composite::FRAME_SLOTS`): on every layer the decoration is above
        // the data. A panel nested in another's data area (an inset
        // subplot, #203) is drawn after every other panel, all its slots
        // over them, so it covers its parent's data, decoration and
        // annotations.
        let nested = |index: usize| nested_panel(prepared, index);
        let count = prepared.len();
        for tier in [false, true] {
            // The panels of a group draw slot by slot, so a parent's
            // decoration lies over its overlays' data. An inset draws as one
            // unit, all its slots before the next one (#205): an inset over
            // another covers its chrome as well as its data, as a pick finds.
            let slot_major = (!tier).then(|| {
                super::composite::FRAME_SLOTS
                    .into_iter()
                    .flat_map(move |slot| (0..count).map(move |index| (slot, index)))
            });
            // Insets in their tree order (#205): an inset inside another is
            // drawn over it, and a later sibling over both, as a pick finds.
            let panel_major = tier.then(|| {
                crate::layout::inset_paint_order(count, |i| panel_rects(prepared, i)).flat_map(
                    |index| {
                        super::composite::FRAME_SLOTS
                            .into_iter()
                            .map(move |slot| (slot, index))
                    },
                )
            });
            for (slot, index) in slot_major
                .into_iter()
                .flatten()
                .chain(panel_major.into_iter().flatten())
            {
                {
                    let item = &prepared[index];
                    if nested(index) != tier {
                        continue;
                    }
                    if let Some(display) = &item.stream_display {
                        // The stream display is the whole panel in one texture;
                        // it is drawn once, in the first slot, as the panel's
                        // GPU data.
                        if slot == crate::sink::Slot::Below
                            && steps(index).gpu
                            && let Some(panel) =
                                data_render::clamp_rect_to_target(item.view.panel_rect, target_size)
                        {
                            // D covers the whole target, unlike a panel-sized axis texture.
                            pass.set_viewport(
                                0.0,
                                0.0,
                                target_size.0 as f32,
                                target_size.1 as f32,
                                0.0,
                                1.0,
                            );
                            pass.set_scissor_rect(panel.x, panel.y, panel.width, panel.height);
                            pass.set_pipeline(&item.axis_pipeline);
                            pass.set_bind_group(0, &display.bind_group, &[]);
                            pass.draw(0..3, 0..1);
                        }
                        continue;
                    }
                    let panel_rect = item.view.panel_rect;
                    // The series layers are read by the data slot only.
                    let series_list: Vec<_>;
                    let own: OwnPieceLayers<'_>;
                    let series_layers = match slot {
                        crate::sink::Slot::Below => {
                            series_list = item.series.iter().map(PreparedSeries::layers).collect();
                            own = item.own_piece_layers();
                            super::composite::PieceSeries::of(&series_list, &own)
                        }
                        crate::sink::Slot::Above => super::composite::PieceSeries::shared(&[]),
                    };
                    // Without the chrome only the data slot is drawn.
                    let (annotation_underlay, annotation_overlay) = if chrome {
                        item.view.annotation_layers(&item.axis_pipeline)
                    } else {
                        (None, None)
                    };

                    // The single style decision per panel: the chart's `DrawStyle`
                    // resolves to a cached styled pipeline set (compiled by the
                    // prepare phase) or `None` for precise — the precise path stays
                    // untouched.
                    // Bundle every series's primitives for the panel into one call.

                    // Cached bodies cover the whole panel; a panel the target clips
                    // draws them directly, as before.
                    let cached = item
                        .resident_display
                        .as_ref()
                        .filter(|_| {
                            panel_rect.x.checked_add(panel_rect.width) <= Some(target_size.0)
                                && panel_rect.y.checked_add(panel_rect.height)
                                    <= Some(target_size.1)
                        })
                        .map(|display| AxisLayer {
                            pipeline: &item.axis_pipeline,
                            bind_group: &display.bind_group,
                        });
                    super::composite::draw_chart_panel(
                        pass,
                        target_size,
                        panel_rect,
                        &item.data_pieces,
                        &super::composite::PanelChrome {
                            below: chrome.then_some(AxisLayer {
                                pipeline: &item.axis_pipeline,
                                bind_group: &item.view.grid_bind_group,
                            }),
                            annotation_underlay,
                            above: chrome.then_some(AxisLayer {
                                pipeline: &item.axis_pipeline,
                                bind_group: &item.view.decoration_bind_group,
                            }),
                            annotation_overlay,
                        },
                        series_layers,
                        cached,
                        slot,
                        steps(index),
                    );
                }
            }
        }
        Ok(())
    }
}

impl Renderer {
    /// The GPU styles of chart `id`'s series as registered
    /// (`ChartState::prepared_styles`), built when missing or stale: the
    /// frame prepares them before its draws, and a 3D chart's point pick
    /// binds them (#132).
    pub(in crate::renderer) fn ensure_registered_styles(&mut self, id: ChartId) -> Result<()> {
        let allocation =
            |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource: "registered chart styles",
                reason: error.to_string(),
            };
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        if state
            .prepared_styles
            .as_ref()
            .is_some_and(|styles| styles.series_revision == state.revisions.series)
        {
            return Ok(());
        }
        let mut styles = Vec::new();
        styles
            .try_reserve_exact(state.series.len())
            .map_err(allocation)?;
        for series in &state.series {
            styles.push(self.create_style_for_series_scaled_inner(series, 1.0)?);
        }
        let series_revision = state.revisions.series;
        self.chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?
            .prepared_styles = Some(RegisteredChartStyles {
            series_revision,
            styles,
        });
        Ok(())
    }
}

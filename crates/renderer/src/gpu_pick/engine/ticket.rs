//! The pick ticket and its readback.

use super::*;

pub(in crate::gpu_pick) enum GpuPickTicketState {
    ReadyNone,
    Pending {
        /// The candidates' `MAP_READ` buffer, mapped once the pick completes.
        readback: GpuReadback,
        identities: Arc<[PickIdentity]>,
        /// How many line offset member candidates follow the final one in
        /// `readback`, in stacking order; zero for every other pick.
        members: usize,
        /// Credited when the ticket resolves or is abandoned — either way the
        /// MAP_READ buffer goes with it.
        _charge: GpuByteCharge,
        _scratch_charge: Option<SharedCharge>,
    },
}

/// Owned, detached asynchronous scalar readback.
pub struct GpuPickTicket {
    pub(in crate::gpu_pick) state: GpuPickTicketState,
    /// The monotonic series of the pick (#271): the envelope each one's
    /// columns were written from at submission, which maps a picked row
    /// back to its logical sample index. Empty for every other chart.
    pub(crate) monotonic: Vec<crate::renderer::MonotonicPickRows>,
}

pub(crate) struct GpuIndexedPick {
    pub(crate) source_id: Option<String>,
    pub(crate) series_id: String,
    pub(crate) series_order: u32,
    pub(crate) point_index: u32,
    /// A monotonic series' logical sample index for the picked row (#271);
    /// `None` for every other series, whose index is `point_index`.
    pub(crate) logical_index: Option<usize>,
    pub(crate) distance_px: f32,
}

impl From<GpuIndexedPick> for PickedPoint {
    fn from(picked: GpuIndexedPick) -> Self {
        Self {
            source_id: picked.source_id,
            series_id: picked.series_id,
            point_index: picked.logical_index.unwrap_or(picked.point_index as usize),
            distance_px: picked.distance_px,
        }
    }
}

impl GpuPickTicket {
    pub(in crate::gpu_pick) fn ready_none() -> Self {
        Self {
            state: GpuPickTicketState::ReadyNone,
            monotonic: Vec::new(),
        }
    }

    /// Wait for the pick: the bundle's poll thread completes the wait on
    /// native (see [`crate::RendererDevice::new`]).
    pub async fn resolve(self) -> Result<Option<PickedPoint>, GpuPickError> {
        Ok(self.resolve_indexed().await?.map(PickedPoint::from))
    }

    /// [`Self::resolve`], polling the device on this thread until the pick
    /// completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve_blocking(mut self) -> Result<Option<PickedPoint>, GpuPickError> {
        self.wait_blocking()?;
        Ok(self.finish()?.map(PickedPoint::from))
    }

    /// The pick if it has completed, or the ticket back, without blocking.
    pub fn try_resolve(
        mut self,
    ) -> Result<crate::TryResolve<Option<PickedPoint>, Self>, GpuPickError> {
        if !self.poll_ready()? {
            return Ok(crate::TryResolve::Pending(self));
        }
        Ok(crate::TryResolve::Ready(
            self.finish()?.map(PickedPoint::from),
        ))
    }

    pub(crate) async fn resolve_indexed(mut self) -> Result<Option<GpuIndexedPick>, GpuPickError> {
        self.wait().await?;
        self.finish()
    }

    /// Await the readback; nothing to wait for without one.
    pub(crate) async fn wait(&mut self) -> Result<(), GpuPickError> {
        if let GpuPickTicketState::Pending { readback, .. } = &mut self.state {
            readback.wait().await?;
        }
        Ok(())
    }

    /// Poll the device on this thread until the readback completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn wait_blocking(&mut self) -> Result<(), GpuPickError> {
        if let GpuPickTicketState::Pending { readback, .. } = &mut self.state {
            readback.wait_blocking()?;
        }
        Ok(())
    }

    /// Whether the readback has completed, without blocking.
    pub(crate) fn poll_ready(&mut self) -> Result<bool, GpuPickError> {
        match &mut self.state {
            GpuPickTicketState::Pending { readback, .. } => Ok(readback.poll_ready()?),
            GpuPickTicketState::ReadyNone => Ok(true),
        }
    }

    /// After a wait: decode the candidates. Dropping the ticket afterwards
    /// credits the readback buffer.
    pub(crate) fn finish(&mut self) -> Result<Option<GpuIndexedPick>, GpuPickError> {
        let GpuPickTicketState::Pending {
            readback,
            identities,
            members,
            ..
        } = &mut self.state
        else {
            return Ok(None);
        };
        let members = *members;

        // The unmap is the ticket's last GPU call, so the read checks the
        // device health after it (#158).
        let candidate = readback.finish_checked(|mapped| {
            let size = CANDIDATE_BYTES as usize;
            let candidate_at = |index: usize| {
                *bytemuck::from_bytes::<PickCandidateGpu>(&mapped[index * size..][..size])
            };
            // Member panels composite in `line_offsets.series` order, so
            // the last member painted under the cursor (a hit at distance
            // 0) is what the user sees there; the final candidate's tie
            // rule cannot say so, since a later member's fill covers an
            // earlier member's line. With no member painted there, the
            // nearest hit of any series answers.
            (1..=members)
                .rev()
                .map(candidate_at)
                .find(|candidate| {
                    candidate.valid == 2 || (candidate.valid == 1 && candidate.distance_px == 0.0)
                })
                .unwrap_or_else(|| candidate_at(0))
        })?;

        match candidate.valid {
            0 => Ok(None),
            2 => Err(GpuPickError::TraversalOverflow),
            1 => {
                let identity = identities
                    .get(candidate.series_order as usize)
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                if !candidate.distance_px.is_finite() {
                    return Err(GpuPickError::InvalidGpuResult);
                }
                // A monotonic series' row maps back to the logical index of
                // the kept point it was uploaded from (#271).
                let logical_index = match self
                    .monotonic
                    .iter()
                    .find(|rows| rows.series_id == identity.series_id)
                {
                    Some(rows) => match rows.logical(candidate.point_index as usize) {
                        Some(index) => Some(index),
                        None => return Ok(None),
                    },
                    None => None,
                };
                Ok(Some(GpuIndexedPick {
                    source_id: identity.source_id.clone(),
                    series_id: identity.series_id.clone(),
                    series_order: candidate.series_order,
                    point_index: candidate.point_index,
                    logical_index,
                    distance_px: candidate.distance_px,
                }))
            }
            _ => Err(GpuPickError::InvalidGpuResult),
        }
    }
}

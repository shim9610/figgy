//! Snap guides while dragging a decoration (#440).
//!
//! While a decoration is dragged, its drawn box snaps to useful alignments:
//! the chart centre and the plot centre, its own axis' centre line (an axis
//! title centred on its axis), and the edges and centres of the other
//! movable elements of the same hit map. A snap is a display parameter, not
//! data (rule 1): the move still goes through the element's nudge path and
//! is stored as its usual SSOT offset.
//!
//! The gesture state is one fixed-size [`DragSnap`] the host keeps from the
//! press to the release. It holds the part of the pointer's travel a snap
//! absorbed (`residual`), so an element held on a guide leaves it once the
//! pointer has moved past the threshold, and the guides of the last move,
//! which the host hands to the overlay that draws the selection box. Guides
//! are never stored in the SSOT and never exported.

use super::RectF;

/// How close, in logical pixels on screen, a drawn edge or centre has to
/// come to a target line before it snaps. A host divides it by the display
/// scale ([`DragSnap::new`]) so the reach is the same on screen at every
/// zoom.
pub const SNAP_THRESHOLD_PX: f32 = 6.0;

/// One guide line, in the chart frame (the frame the drag deltas and the
/// selection box are in): a vertical guide has `from[0] == to[0]`, a
/// horizontal one `from[1] == to[1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapGuide {
    pub from: [f32; 2],
    pub to: [f32; 2],
}

/// The guides a snapped move shows: at most one vertical line (an x
/// alignment) and one horizontal line (a y alignment).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SnapGuides {
    pub vertical: Option<SnapGuide>,
    pub horizontal: Option<SnapGuide>,
}

impl SnapGuides {
    /// No guide.
    pub const NONE: Self = Self {
        vertical: None,
        horizontal: None,
    };

    pub fn is_empty(&self) -> bool {
        self.vertical.is_none() && self.horizontal.is_none()
    }

    /// The drawn guides, vertical first.
    pub fn iter(&self) -> impl Iterator<Item = SnapGuide> {
        [self.vertical, self.horizontal].into_iter().flatten()
    }

    /// These guides in another frame: every point `p` mapped to
    /// `to + (p - origin) * scale`, as a document is placed on a display.
    pub fn mapped(&self, origin: [f32; 2], scale: f32, to: [f32; 2]) -> Self {
        let point = |p: [f32; 2]| {
            [
                to[0] + (p[0] - origin[0]) * scale,
                to[1] + (p[1] - origin[1]) * scale,
            ]
        };
        let map = |g: SnapGuide| SnapGuide {
            from: point(g.from),
            to: point(g.to),
        };
        Self {
            vertical: self.vertical.map(map),
            horizontal: self.horizontal.map(map),
        }
    }
}

/// What a dragged element can snap to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapTarget {
    /// The centre lines of an area (the chart, the plot, an axis): the
    /// element's centre snaps to them, and the guide spans the area.
    Centre(RectF),
    /// The edges and centre lines of another element: any edge or the
    /// centre of the moving element snaps to any of them, and the guide
    /// spans both boxes.
    Edges(RectF),
}

/// The areas whose centres every dragged element snaps to: the chart (or
/// panel) and its plot. `None` for one the state does not know.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SnapAreas {
    pub chart: Option<RectF>,
    pub plot: Option<RectF>,
}

/// The snap state of one drag gesture, kept by the host from the press to
/// the release (#440). Fixed-size: snapping keeps no list of targets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragSnap {
    threshold: f32,
    enabled: bool,
    /// Pointer travel the current snap absorbed, per axis: the raw position
    /// is the drawn one plus this.
    residual: [f32; 2],
    guides: SnapGuides,
}

impl Default for DragSnap {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl DragSnap {
    /// A gesture snapping within [`SNAP_THRESHOLD_PX`] on screen, for a
    /// display that shows one logical document pixel as `display_scale`
    /// screen pixels. A non-finite or non-positive scale reads as 1.
    pub fn new(display_scale: f32) -> Self {
        let scale = if display_scale.is_finite() && display_scale > 0.0 {
            display_scale
        } else {
            1.0
        };
        Self {
            threshold: SNAP_THRESHOLD_PX / scale,
            enabled: true,
            residual: [0.0, 0.0],
            guides: SnapGuides::NONE,
        }
    }

    /// A gesture that never snaps: the snapping option is off.
    pub fn off() -> Self {
        let mut snap = Self::new(1.0);
        snap.enabled = false;
        snap
    }

    /// The reach in the frame of the drag deltas.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Turn snapping on or off for the rest of the gesture: a host clears it
    /// while the Alt / Option modifier is held. Turning it off drops the
    /// guides and the absorbed travel, so the element follows the pointer
    /// from where it is drawn.
    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.residual = [0.0, 0.0];
            self.guides = SnapGuides::NONE;
        }
        self.enabled = enabled;
    }

    /// The guides of the last move; empty after a move that did not snap,
    /// a rejected move or [`Self::release`].
    pub fn guides(&self) -> SnapGuides {
        self.guides
    }

    /// End the gesture: no guide is shown any more.
    pub fn release(&mut self) {
        self.residual = [0.0, 0.0];
        self.guides = SnapGuides::NONE;
    }

    /// The delta to apply for a pointer move of `(dx, dy)` of an element
    /// drawn in `moving`, with the snap targets `targets` calls back with,
    /// and the guides and absorbed travel that move leaves. Nothing is
    /// stored until [`Self::settle`] says whether the move was applied.
    pub fn propose(
        &self,
        moving: RectF,
        dx: f32,
        dy: f32,
        targets: &mut dyn FnMut(&mut dyn FnMut(SnapTarget)),
    ) -> SnapProposal {
        let raw = [self.residual[0] + dx, self.residual[1] + dy];
        if !self.enabled || !raw[0].is_finite() || !raw[1].is_finite() {
            return SnapProposal {
                delta: [dx, dy],
                residual: [0.0, 0.0],
                guides: SnapGuides::NONE,
            };
        }
        let candidate = moving.translated(raw[0], raw[1]);
        let mut x = AxisSnap::new(self.threshold);
        let mut y = AxisSnap::new(self.threshold);
        targets(&mut |target| match target {
            SnapTarget::Centre(area) => {
                let (cx, cy) = area.center();
                let (mx, my) = candidate.center();
                x.offer(cx - mx, cx, area);
                y.offer(cy - my, cy, area);
            }
            SnapTarget::Edges(other) => {
                for line in [other.x, other.x + other.width * 0.5, other.x + other.width] {
                    for anchor in [
                        candidate.x,
                        candidate.x + candidate.width * 0.5,
                        candidate.x + candidate.width,
                    ] {
                        x.offer(line - anchor, line, other);
                    }
                }
                for line in [
                    other.y,
                    other.y + other.height * 0.5,
                    other.y + other.height,
                ] {
                    for anchor in [
                        candidate.y,
                        candidate.y + candidate.height * 0.5,
                        candidate.y + candidate.height,
                    ] {
                        y.offer(line - anchor, line, other);
                    }
                }
            }
        });
        let delta = [raw[0] + x.shift(), raw[1] + y.shift()];
        let snapped = moving.translated(delta[0], delta[1]);
        let guides = SnapGuides {
            vertical: x.best.map(|(_, line, span)| {
                let top = span.y.min(snapped.y);
                let bottom = (span.y + span.height).max(snapped.y + snapped.height);
                SnapGuide {
                    from: [line, top],
                    to: [line, bottom],
                }
            }),
            horizontal: y.best.map(|(_, line, span)| {
                let left = span.x.min(snapped.x);
                let right = (span.x + span.width).max(snapped.x + snapped.width);
                SnapGuide {
                    from: [left, line],
                    to: [right, line],
                }
            }),
        };
        SnapProposal {
            delta,
            residual: [raw[0] - delta[0], raw[1] - delta[1]],
            guides,
        }
    }

    /// Keep what `proposal` leaves when its move was applied (`moved`); a
    /// rejected move keeps the absorbed travel and shows no guide.
    pub fn settle(&mut self, proposal: &SnapProposal, moved: bool) {
        if moved {
            self.residual = proposal.residual;
            self.guides = proposal.guides;
        } else {
            self.guides = SnapGuides::NONE;
        }
    }
}

/// The outcome of [`DragSnap::propose`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapProposal {
    /// The delta the element moves by.
    pub delta: [f32; 2],
    residual: [f32; 2],
    guides: SnapGuides,
}

/// The nearest alignment on one axis: the shift, the target line and the
/// box the guide spans.
struct AxisSnap {
    threshold: f32,
    best: Option<(f32, f32, RectF)>,
}

impl AxisSnap {
    fn new(threshold: f32) -> Self {
        Self {
            threshold,
            best: None,
        }
    }

    fn offer(&mut self, shift: f32, line: f32, span: RectF) {
        if !shift.is_finite() || shift.abs() > self.threshold {
            return;
        }
        if self
            .best
            .is_none_or(|(best, _, _)| shift.abs() < best.abs())
        {
            self.best = Some((shift, line, span));
        }
    }

    fn shift(&self) -> f32 {
        self.best.map_or(0.0, |(shift, _, _)| shift)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> RectF {
        RectF {
            x,
            y,
            width,
            height,
        }
    }

    fn drag(snap: &mut DragSnap, moving: &mut RectF, dx: f32, dy: f32, area: RectF) {
        let proposal = snap.propose(*moving, dx, dy, &mut |offer| {
            offer(SnapTarget::Centre(area))
        });
        *moving = moving.translated(proposal.delta[0], proposal.delta[1]);
        snap.settle(&proposal, true);
    }

    #[test]
    fn a_box_snaps_to_a_centre_and_leaves_it_past_the_threshold() {
        let area = rect(0.0, 0.0, 200.0, 100.0);
        // Centre at x = 90: 10 px left of the area's centre.
        let mut moving = rect(80.0, 10.0, 20.0, 10.0);
        let mut snap = DragSnap::new(1.0);
        drag(&mut snap, &mut moving, 5.0, 0.0, area);
        assert_eq!(moving.center().0, 100.0, "5 px short of centre snaps");
        let guide = snap.guides().vertical.expect("a vertical guide");
        assert_eq!(guide.from, [100.0, 0.0]);
        assert_eq!(guide.to, [100.0, 100.0]);
        // Held on the guide while the pointer stays within the reach.
        drag(&mut snap, &mut moving, 3.0, 0.0, area);
        assert_eq!(moving.center().0, 100.0);
        // The pointer is now 2 px short of the centre; 9 more take it past
        // the reach, and the element is where the pointer put it.
        drag(&mut snap, &mut moving, 9.0, 0.0, area);
        assert_eq!(moving.center().0, 107.0);
        assert!(snap.guides().vertical.is_none());
    }

    #[test]
    fn a_disabled_gesture_follows_the_pointer() {
        let area = rect(0.0, 0.0, 200.0, 100.0);
        let mut moving = rect(80.0, 10.0, 20.0, 10.0);
        let mut snap = DragSnap::new(1.0);
        snap.set_enabled(false);
        drag(&mut snap, &mut moving, 5.0, 0.0, area);
        assert_eq!(moving.center().0, 95.0);
        assert!(snap.guides().is_empty());
        assert!(DragSnap::off().propose(moving, 1.0, 0.0, &mut |_| {}).delta == [1.0, 0.0]);
    }

    #[test]
    fn the_reach_follows_the_display_scale() {
        assert_eq!(DragSnap::new(2.0).threshold(), 3.0);
        assert_eq!(DragSnap::new(0.0).threshold(), SNAP_THRESHOLD_PX);
    }

    #[test]
    fn edges_snap_to_edges() {
        let other = rect(50.0, 50.0, 40.0, 20.0);
        // Left edge at 48: 2 px from the other's left edge.
        let moving = rect(48.0, 0.0, 10.0, 10.0);
        let snap = DragSnap::new(1.0);
        let proposal = snap.propose(moving, 0.0, 0.0, &mut |offer| {
            offer(SnapTarget::Edges(other))
        });
        assert_eq!(proposal.delta, [2.0, 0.0]);
        assert_eq!(
            proposal.guides.vertical,
            Some(SnapGuide {
                from: [50.0, 0.0],
                to: [50.0, 70.0]
            })
        );
    }

    // #440: the option is absent from JSON while unset (a document written
    // before it reads as on) and round trips when set.
    #[cfg(feature = "serde")]
    #[test]
    fn the_snap_option_round_trips() {
        use crate::config::Config;
        let unset = crate::defaults::config();
        let json = serde_json::to_value(&unset).unwrap();
        assert!(json.get("snap_to_guides").is_none());
        assert!(unset.snaps_while_dragging());
        assert_eq!(serde_json::from_value::<Config>(json).unwrap(), unset);
        for value in [false, true] {
            let mut config = unset.clone();
            config.snap_to_guides = Some(value);
            let json = serde_json::to_value(&config).unwrap();
            assert_eq!(json["snap_to_guides"], value);
            let back = serde_json::from_value::<Config>(json).unwrap();
            assert_eq!(back, config);
            assert_eq!(back.snaps_while_dragging(), value);
            assert_eq!(
                serde_json::to_string(&back).unwrap(),
                serde_json::to_string(&config).unwrap()
            );
        }
    }
}

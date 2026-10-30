use crate::select::{emit_element, on_drawn_panel};
use crate::sink::Parts;
use crate::text::MeasureText;

use super::{
    Config, RectF, Side, axis_anchor, axis_mut, axis_ref, colorbar_rect, point_on_rect_side,
    screen_offset_to_axis_title,
};

// Nudge types.

#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    ChartTitle,
    AxisTitle(Side),
    AxisLabel(Side),
    /// The axis line itself — a *detached* move. Shifts only perpendicular to
    /// its direction (y-axes horizontally, x-axes vertically) via
    /// `AxisOptions::line_offset`; the data area, grid, and data transform
    /// stay put, so tick positions along the axis stay aligned with the data.
    Axis(Side),
    /// One edge of the data area — moves by adjusting that side's
    /// `out_margin`, dragging the whole data area boundary with it. Used by
    /// the data-area resize handles.
    DataAreaEdge(Side),
    /// The whole data area — translates without resizing by shifting opposite
    /// margins in tandem (left grows as right shrinks, etc.). Ticks, grid,
    /// and the data transform all derive from the data area, so everything
    /// moves as one.
    DataArea,
    /// The legend box — moves freely via `legend.offset_{x,y}` relative to
    /// its corner anchor.
    Legend,
    /// The colourbar strip — moves freely via `colorbar.offset_{x,y}` relative
    /// to its `side` + `align` anchor, the same rule as [`Self::Legend`].
    ColorBar,
    /// The colourbar axis line and ticks. Like a chart axis, a drag detaches it
    /// only perpendicular to its direction through `colorbar.axis.line_offset`.
    ColorBarAxis,
    /// The colourbar tick-label band. A drag updates the z axis' label offsets.
    ColorBarLabel,
    /// The colourbar title. A drag updates the z axis' title offsets in the
    /// title's rotated local frame.
    ColorBarTitle,
    /// One edge of the colourbar strip, named by the **screen** side of the
    /// strip it is on — the resize-handle target, mirroring
    /// [`Self::DataAreaEdge`].
    ///
    /// Which dimension it changes follows the bar's orientation, not the
    /// element: on a vertical bar the left/right edges are `thickness_px` and
    /// the top/bottom edges are `length_frac`; on a horizontal bar it is the
    /// other way round. The element stays orientation-free because the handle
    /// that produced it only knows screen directions.
    ColorBarEdge(Side),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NudgeResult {
    Moved,
    Rejected(NudgeReject),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NudgeReject {
    OutOfBounds,
}

// Containment.

impl Config {
    /// Whether `element` stays grabbable where it is drawn: at least one of
    /// the parts it draws keeps its centre inside the chart area, so a
    /// moved element can always be selected again. `None` when `measure`
    /// cannot lay the element out. The parts come from the element's one
    /// placement ([`emit_element`]), on the panel the base layer is drawn
    /// with: a layered document expands its base panel for this on every
    /// step (SSOT configuration, the renderer's own expansion); a
    /// single-layer document copies nothing.
    fn grabbable(&self, element: &Element, measure: &dyn MeasureText) -> Option<bool> {
        let chart_area = RectF::from_rect(&self.chart_area.0);
        let mut inside = false;
        let mut sink = Parts(&mut |part| {
            let (cx, cy) = part.extent().center();
            inside |= chart_area.contains(cx, cy);
        });
        let laid_out = on_drawn_panel(self, |panel| {
            emit_element(panel, element, measure, &mut sink)
        });
        laid_out.then_some(inside)
    }

    /// The stored offset an element is moved through: `None` for an element
    /// with none (a missing colourbar), or one that moves by another rule.
    fn offset_of(&self, element: &Element) -> Option<(f32, f32)> {
        Some(match element {
            Element::ChartTitle => (self.chart_title.offset_x, self.chart_title.offset_y),
            Element::AxisTitle(side) => {
                let a = axis_ref(self, side);
                (a.title_option.offset_x, a.title_option.offset_y)
            }
            Element::AxisLabel(side) => {
                let a = axis_ref(self, side);
                (a.label_style.label_offset_x, a.label_style.label_offset_y)
            }
            Element::Legend => (self.legend.offset_x, self.legend.offset_y),
            Element::ColorBar => {
                let bar = self.colorbar.as_ref()?;
                (bar.offset_x, bar.offset_y)
            }
            Element::ColorBarLabel => {
                let bar = self.colorbar.as_ref()?;
                (
                    bar.axis.label_style.label_offset_x,
                    bar.axis.label_style.label_offset_y,
                )
            }
            Element::ColorBarTitle => {
                let bar = self.colorbar.as_ref()?;
                (
                    bar.axis.title_option.offset_x,
                    bar.axis.title_option.offset_y,
                )
            }
            Element::Axis(_)
            | Element::ColorBarAxis
            | Element::DataAreaEdge(_)
            | Element::DataArea
            | Element::ColorBarEdge(_) => return None,
        })
    }

    /// Store `offset` as the element's offset: the exact value, so a
    /// rejected move restores the previous state bit for bit.
    fn set_offset(&mut self, element: &Element, offset: (f32, f32)) {
        match element {
            Element::ChartTitle => {
                self.chart_title.offset_x = offset.0;
                self.chart_title.offset_y = offset.1;
            }
            Element::AxisTitle(side) => {
                let a = axis_mut(self, side);
                a.title_option.offset_x = offset.0;
                a.title_option.offset_y = offset.1;
            }
            Element::AxisLabel(side) => {
                let a = axis_mut(self, side);
                a.label_style.label_offset_x = offset.0;
                a.label_style.label_offset_y = offset.1;
            }
            Element::Legend => {
                self.legend.offset_x = offset.0;
                self.legend.offset_y = offset.1;
            }
            Element::ColorBar => {
                if let Some(bar) = self.colorbar.as_mut() {
                    bar.offset_x = offset.0;
                    bar.offset_y = offset.1;
                }
            }
            Element::ColorBarLabel => {
                if let Some(bar) = self.colorbar.as_mut() {
                    bar.axis.label_style.label_offset_x = offset.0;
                    bar.axis.label_style.label_offset_y = offset.1;
                }
            }
            Element::ColorBarTitle => {
                if let Some(bar) = self.colorbar.as_mut() {
                    bar.axis.title_option.offset_x = offset.0;
                    bar.axis.title_option.offset_y = offset.1;
                }
            }
            Element::Axis(_)
            | Element::ColorBarAxis
            | Element::DataAreaEdge(_)
            | Element::DataArea
            | Element::ColorBarEdge(_) => {}
        }
    }

    /// A screen delta in the frame the element's offset is stored in: the
    /// rotated local frame of a left or right title, the screen frame of
    /// everything else.
    fn local_delta(&self, element: &Element, dx: f32, dy: f32) -> Option<(f32, f32)> {
        Some(match element {
            // A ternary title is placed along its edge with its offset in
            // screen space (`Config::ternary_title_placement`).
            Element::AxisTitle(_) if self.ternary.is_some() => (dx, dy),
            Element::AxisTitle(side) => screen_offset_to_axis_title(side.clone(), (dx, dy)),
            Element::ColorBarTitle => {
                let side = self.colorbar.as_ref()?.side.clone();
                screen_offset_to_axis_title(side, (dx, dy))
            }
            _ => (dx, dy),
        })
    }
}

// Nudge methods.

impl Config {
    /// Move `element` by a screen delta. The move accumulates in the config
    /// field the element is drawn from and is contained on what is drawn:
    /// `measure` lays the element out (the renderer's `CpuTextMeasure`), and
    /// the move is rejected, leaving the config untouched, when the element
    /// would no longer be grabbable afterwards or `measure` cannot place it.
    pub fn nudge(
        &mut self,
        element: Element,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        self.nudge_from(element, measure, None, dx, dy)
    }

    /// [`Self::nudge`] with the pointer position `at` the delta was measured
    /// from, when the host has one. Only a polar chart's axes and tick labels
    /// read it: the ray turns by the pointer's angle around the centre and
    /// the angle labels move along the spoke under the pointer. Without it
    /// the ray turns as if grabbed on the outer ring and the angle labels
    /// use the spoke at the zero angle.
    pub fn nudge_from(
        &mut self,
        element: Element,
        measure: &dyn MeasureText,
        at: Option<[f32; 2]>,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        if self.polar.is_some() {
            match &element {
                Element::Axis(side) => return self.nudge_polar_axis(side.clone(), at, dx, dy),
                Element::AxisLabel(side) => {
                    return self.nudge_polar_axis_label(side.clone(), measure, at, dx, dy);
                }
                _ => {}
            }
        }
        if self.ternary.is_some() {
            match &element {
                Element::Axis(side) => return self.nudge_ternary_axis(side, measure, dx, dy),
                Element::AxisLabel(side) => {
                    return self.nudge_ternary_axis_label(side, measure, dx, dy);
                }
                _ => {}
            }
        }
        // Axis / data-area-edge moves have their own rules (perpendicular
        // only); everything else moves via stored offsets below.
        if let Element::Axis(side) = element {
            return self.nudge_axis(side, dx, dy);
        }
        if let Element::ColorBarAxis = element {
            return self.nudge_colorbar_axis(dx, dy);
        }
        if let Element::DataAreaEdge(side) = element {
            return self.nudge_data_area_edge(side, dx, dy);
        }
        if let Element::DataArea = element {
            return self.nudge_data_area(dx, dy);
        }
        if let Element::ColorBarEdge(side) = element {
            return self.nudge_colorbar_edge(side, dx, dy);
        }
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        let (Some(previous), Some((local_dx, local_dy))) =
            (self.offset_of(&element), self.local_delta(&element, dx, dy))
        else {
            return reject;
        };
        // Write, then keep the element grabbable where it is now drawn, or
        // put the previous offset back exactly.
        self.set_offset(&element, (previous.0 + local_dx, previous.1 + local_dy));
        if self.grabbable(&element, measure) != Some(true) {
            self.set_offset(&element, previous);
            return reject;
        }
        NudgeResult::Moved
    }

    /// Whole-data-area rule (drag-to-move): translate without resizing by
    /// shifting opposite margins in tandem. Each axis component applies
    /// independently and clamps at the chart edge (a blocked horizontal move
    /// doesn't kill a valid vertical one), so the area slides along the
    /// boundary like any draggable box.
    fn nudge_data_area(&mut self, dx: f32, dy: f32) -> NudgeResult {
        let mut moved = false;

        if dx != 0.0 {
            let new_left = self.left_y.out_margin + dx;
            // The right band is group geometry (overlay axes share it), so
            // the move starts from the group's band, not the parent's own.
            let new_right = super::right_out_margin_effective(self) - dx;
            if new_left >= 0.0 && new_right >= 0.0 {
                self.left_y.out_margin = new_left;
                self.right_y.out_margin = new_right;
                super::sync_overlay_right_margins(self);
                moved = true;
            }
        }
        if dy != 0.0 {
            let new_top = self.top_x.out_margin + dy;
            let new_bottom = self.bottom_x.out_margin - dy;
            if new_top >= 0.0 && new_bottom >= 0.0 {
                self.top_x.out_margin = new_top;
                self.bottom_x.out_margin = new_bottom;
                moved = true;
            }
        }

        if moved {
            NudgeResult::Moved
        } else {
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        }
    }

    /// Polar: the radius axis ray turns around the centre. With the pointer,
    /// the ray follows its angle exactly; without it, a point on the outer
    /// ring follows the drag's component across the ray. The angle axis is
    /// the outer ring, the data boundary, and has no move.
    fn nudge_polar_axis(
        &mut self,
        side: Side,
        at: Option<[f32; 2]>,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        if side != Side::Left {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        let Some(geometry) = self.polar_geometry() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let Some(polar) = self.polar.as_mut() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let turned = at
            .and_then(|p| {
                let before = geometry.screen_angle_of(p)?;
                let after = geometry.screen_angle_of([p[0] + dx, p[1] + dy])?;
                let mut turn = (after - before).to_degrees();
                if turn > 180.0 {
                    turn -= 360.0;
                } else if turn < -180.0 {
                    turn += 360.0;
                }
                Some(turn)
            })
            .or_else(|| {
                if geometry.radius <= 0.0 {
                    return None;
                }
                let s = polar.ray_side();
                Some(f64::from((dx * s[0] + dy * s[1]) / geometry.radius).to_degrees())
            });
        let Some(turn) = turned else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        polar.turn_ray(turn);
        NudgeResult::Moved
    }

    /// Polar: the angle tick labels move along their spokes by the drag's
    /// radial component at the pointer (`label_offset_y`, outward positive);
    /// the radius tick labels move in the ray frame (`label_offset_x` along,
    /// `label_offset_y` across). The moved labels must stay grabbable, as
    /// every other element.
    fn nudge_polar_axis_label(
        &mut self,
        side: Side,
        measure: &dyn MeasureText,
        at: Option<[f32; 2]>,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        let Some(geometry) = self.polar_geometry() else {
            return reject;
        };
        let Some(polar) = self.polar.as_ref() else {
            return reject;
        };
        let element = Element::AxisLabel(side.clone());
        let delta = match side {
            Side::Bottom => {
                let spoke = at
                    .and_then(|p| geometry.spoke_direction(p))
                    .unwrap_or_else(|| {
                        let a = polar.zero_angle_degrees.to_radians();
                        [a.cos() as f32, -(a.sin() as f32)]
                    });
                (0.0, dx * spoke[0] + dy * spoke[1])
            }
            Side::Left => {
                let dir = polar.ray_direction();
                let across_dir = polar.ray_side();
                (
                    dx * dir[0] + dy * dir[1],
                    dx * across_dir[0] + dy * across_dir[1],
                )
            }
            Side::Top | Side::Right => return reject,
        };
        let Some(previous) = self.offset_of(&element) else {
            return reject;
        };
        self.set_offset(&element, (previous.0 + delta.0, previous.1 + delta.1));
        if self.grabbable(&element, measure) != Some(true) {
            self.set_offset(&element, previous);
            return reject;
        }
        NudgeResult::Moved
    }

    /// Ternary: an edge moves along its outward normal by the drag's
    /// component on it (`line_offset`, outward positive), carrying its ticks
    /// and labels; the top axis is not drawn. The moved edge must stay
    /// grabbable, as every other element.
    fn nudge_ternary_axis(
        &mut self,
        side: &Side,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        let (Some(k), Some(g)) = (
            crate::ternary::side_component(side),
            self.ternary_geometry(),
        ) else {
            return reject;
        };
        let n = g.outward_normal(k);
        let d = dx * n[0] + dy * n[1];
        if d == 0.0 {
            // A drag along the edge: nothing to move, but not an error.
            return NudgeResult::Moved;
        }
        let previous = axis_ref(self, side).line_offset;
        axis_mut(self, side).line_offset = previous + d;
        if self.grabbable(&Element::Axis(side.clone()), measure) != Some(true) {
            axis_mut(self, side).line_offset = previous;
            return reject;
        }
        NudgeResult::Moved
    }

    /// Ternary: an edge's tick labels move in the edge frame,
    /// `label_offset_x` along the edge (from its 0 towards its 100 % corner)
    /// and `label_offset_y` along its outward normal.
    fn nudge_ternary_axis_label(
        &mut self,
        side: &Side,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        let (Some(k), Some(g)) = (
            crate::ternary::side_component(side),
            self.ternary_geometry(),
        ) else {
            return reject;
        };
        let along = g.edge_direction(k);
        let n = g.outward_normal(k);
        let element = Element::AxisLabel(side.clone());
        let Some(previous) = self.offset_of(&element) else {
            return reject;
        };
        self.set_offset(
            &element,
            (
                previous.0 + dx * along[0] + dy * along[1],
                previous.1 + dx * n[0] + dy * n[1],
            ),
        );
        if self.grabbable(&element, measure) != Some(true) {
            self.set_offset(&element, previous);
            return reject;
        }
        NudgeResult::Moved
    }

    /// Detached-axis drag rule: the axis line (with its ticks and labels)
    /// moves only **perpendicular to itself** — y-axes horizontally, x-axes
    /// vertically — via `line_offset`. The parallel component is discarded.
    /// The data area / grid / data transform are untouched, so tick positions
    /// along the axis stay aligned with the data; the constraint is only that
    /// the axis line, where [`axis_anchor`] draws it, stays inside the chart
    /// area (it may cross into the data area, e.g. a y-axis at x = 0).
    fn nudge_axis(&mut self, side: Side, dx: f32, dy: f32) -> NudgeResult {
        let d = match side {
            Side::Left | Side::Right => dx,
            Side::Top | Side::Bottom => dy,
        };
        if d == 0.0 {
            // Pure parallel drag — nothing to move, but not an error.
            return NudgeResult::Moved;
        }
        let Ok(da) = self.data_area() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let new_offset = axis_ref(self, &side).line_offset + d;

        // The shifted axis line must stay inside the chart area.
        let (anchor, _) = axis_anchor(
            side.clone(),
            &RectF::from_rect(&da),
            super::axis_cross(self, &side),
        );
        let line_pos = match side {
            Side::Left | Side::Right => anchor.0 + new_offset,
            Side::Top | Side::Bottom => anchor.1 + new_offset,
        };
        let ca = &self.chart_area;
        let (lo, hi) = match side {
            Side::Left | Side::Right => (ca.x as f32, (ca.x + ca.width) as f32),
            Side::Top | Side::Bottom => (ca.y as f32, (ca.y + ca.height) as f32),
        };
        if line_pos < lo || line_pos > hi {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        axis_mut(self, &side).line_offset = new_offset;
        NudgeResult::Moved
    }

    /// Colourbar-axis drag rule: detach the z-axis chrome perpendicular to the
    /// strip without moving the strip, its title, the data area, or the z
    /// transform. Labels follow the line/ticks because they share
    /// `colorbar.axis.line_offset` in the renderer.
    fn nudge_colorbar_axis(&mut self, dx: f32, dy: f32) -> NudgeResult {
        let Some(bar) = self.colorbar.as_ref() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let side = bar.side.clone();
        let d = match side {
            Side::Left | Side::Right => dx,
            Side::Top | Side::Bottom => dy,
        };
        if d == 0.0 {
            return NudgeResult::Moved;
        }
        let Ok(da) = self.data_area() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let rect = colorbar_rect(&self.chart_area, &da, self.chart_title.top_margin, bar);
        let new_offset = bar.axis.line_offset + d;
        // The axis runs along the strip's outer long edge, where
        // `point_on_rect_side` puts it.
        let (line_x, line_y) = point_on_rect_side(0.0, &side, &rect);
        let line_pos = match side {
            Side::Left | Side::Right => line_x + new_offset,
            Side::Top | Side::Bottom => line_y + new_offset,
        };
        let ca = &self.chart_area;
        let (lo, hi) = match side {
            Side::Left | Side::Right => (ca.x as f32, (ca.x + ca.width) as f32),
            Side::Top | Side::Bottom => (ca.y as f32, (ca.y + ca.height) as f32),
        };
        if line_pos < lo || line_pos > hi {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        self.colorbar.as_mut().expect("colourbar").axis.line_offset = new_offset;
        NudgeResult::Moved
    }

    /// Colourbar-edge rule (resize handles): the dragged edge grows the
    /// dimension it belongs to.
    ///
    /// Which dimension that is comes from the bar's orientation, not the handle:
    /// on a vertical bar the left/right edges are `thickness_px` and the
    /// top/bottom edges are `length_frac`. The sign is "away from the strip's
    /// centre grows it", the same shape as `nudge_data_area_edge`'s per-side sign.
    ///
    /// The anchor decides how a size change moves the edge under the pointer, so
    /// the delta is scaled to keep the two together: with `BarAlign::Center` both
    /// ends move by half, so the size has to change by twice the drag. With
    /// `Start` / `End` one end is pinned and the free end carries the whole
    /// change — dragging the *pinned* end's handle still resizes, from the free
    /// end, which is what an anchored box does.
    ///
    /// `thickness_px` needs no such factor. The band anchor pins the edge named
    /// by `bar.side`, so changing thickness alone already makes the opposite,
    /// inner edge follow its handle. When the pinned outer edge itself is
    /// dragged, the strip is translated by the same pointer delta as the size
    /// change. That keeps the opposite edge fixed and puts the grabbed edge
    /// under the pointer instead of leaving it behind at the band anchor.
    fn nudge_colorbar_edge(&mut self, side: Side, dx: f32, dy: f32) -> NudgeResult {
        let Some(bar) = self.colorbar.as_ref() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        // Growth is the drag projected onto the edge's outward normal.
        let growth = match side {
            Side::Left => -dx,
            Side::Right => dx,
            Side::Top => -dy,
            Side::Bottom => dy,
        };
        if growth == 0.0 {
            // Motion parallel to this edge — nothing to resize, not an error.
            return NudgeResult::Moved;
        }
        let bar_vertical = matches!(bar.side, Side::Left | Side::Right);
        let edge_vertical = matches!(side, Side::Left | Side::Right);
        // A vertical bar's left/right edges are its thickness; so are a
        // horizontal bar's top/bottom edges.
        let edge_is_thickness = bar_vertical == edge_vertical;

        if edge_is_thickness {
            let old_thickness = bar.thickness_px;
            let old_offset_x = bar.offset_x;
            let old_offset_y = bar.offset_y;
            let new = old_thickness + growth;
            if new < 0.0 {
                return NudgeResult::Rejected(NudgeReject::OutOfBounds);
            }
            let dragged_outer_edge = bar.side == side;
            let new_offset_x = if dragged_outer_edge && bar_vertical {
                old_offset_x + dx
            } else {
                old_offset_x
            };
            let new_offset_y = if dragged_outer_edge && !bar_vertical {
                old_offset_y + dy
            } else {
                old_offset_y
            };
            // Growing the strip grows its band, which shrinks the data area —
            // so the same check `nudge_data_area_edge` makes applies, by the
            // same write-then-revert (no Config clone on a pointer-move path
            // beyond the base-panel expansion of a layered document).
            // Thickness and the outer-handle offset are one transaction: a
            // rejected resize must not leave the strip translated.
            {
                let bar = self.colorbar.as_mut().expect("colourbar");
                bar.thickness_px = new;
                bar.offset_x = new_offset_x;
                bar.offset_y = new_offset_y;
            }
            if self.data_area().is_err() {
                let bar = self.colorbar.as_mut().expect("colourbar");
                bar.thickness_px = old_thickness;
                bar.offset_x = old_offset_x;
                bar.offset_y = old_offset_y;
                return NudgeResult::Rejected(NudgeReject::OutOfBounds);
            }
            return NudgeResult::Moved;
        }

        let Ok(da) = self.data_area() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let span = if bar_vertical {
            da.height as f32
        } else {
            da.width as f32
        };
        if span <= 0.0 {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        let factor = match bar.align {
            crate::config::BarAlign::Center => 2.0,
            crate::config::BarAlign::Start | crate::config::BarAlign::End => 1.0,
        };
        let new = bar.length_frac + factor * growth / span;
        // `Config::validate`'s range: a strip of zero length has no pixel to
        // hold a colour, and one longer than its side runs past the axis it is
        // labelled against.
        if !(new > 0.0 && new <= 1.0) {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        self.colorbar.as_mut().expect("colourbar").length_frac = new;
        NudgeResult::Moved
    }

    /// Data-area-edge rule (resize handles): one boundary moves by adjusting
    /// that side's `out_margin`, perpendicular only. The data area — and with
    /// it both the tick raster and the GPU data transform — derives from the
    /// margins, so data and ticks move as one.
    fn nudge_data_area_edge(&mut self, side: Side, dx: f32, dy: f32) -> NudgeResult {
        // Screen delta → out_margin delta. Moving an edge toward the chart
        // center grows its margin; toward the chart edge shrinks it.
        let dm = match side {
            Side::Left => dx,
            Side::Right => -dx,
            Side::Top => dy,
            Side::Bottom => -dy,
        };
        if dm == 0.0 {
            return NudgeResult::Moved;
        }
        let old = match side {
            // The right band is group geometry shared with the overlay axes.
            Side::Right => super::right_out_margin_effective(self),
            Side::Top | Side::Bottom | Side::Left => axis_ref(self, &side).out_margin,
        };
        let new = old + dm;
        if new < 0.0 {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        let backup = self.clone();
        axis_mut(self, &side).out_margin = new;
        if side == Side::Right {
            super::sync_overlay_right_margins(self);
        }
        // The shifted margin must still leave a valid data area.
        if self.data_area().is_err() {
            *self = backup;
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        NudgeResult::Moved
    }

    pub fn nudge_x(&mut self, element: Element, measure: &dyn MeasureText, dx: f32) -> NudgeResult {
        self.nudge(element, measure, dx, 0.0)
    }

    pub fn nudge_y(&mut self, element: Element, measure: &dyn MeasureText, dy: f32) -> NudgeResult {
        self.nudge(element, measure, 0.0, dy)
    }
}

// Invariant tests.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;
    use crate::select::tests::LaysOut;
    use crate::text::rich_segments_from_text;

    /// The default config with a visible chart title: a title that draws
    /// nothing cannot be grabbed, so it has no move.
    fn titled_config() -> Config {
        let mut cfg = default_config();
        cfg.chart_title.text.segments = rich_segments_from_text("T");
        cfg
    }

    // data_area() must be identical before and after nudge().
    #[test]
    fn nudge_preserves_data_area() {
        let mut cfg = titled_config();
        let da_before = cfg.data_area().unwrap();
        let r = cfg.nudge(Element::ChartTitle, &LaysOut, 5.0, 3.0);
        assert_eq!(r, NudgeResult::Moved);
        let da_after = cfg.data_area().unwrap();
        assert_eq!(da_before, da_after);
    }

    // On Rejected, Config must remain unchanged.
    #[test]
    fn nudge_rejected_preserves_state() {
        let mut cfg = titled_config();
        let before = cfg.clone();
        let r = cfg.nudge(Element::ChartTitle, &LaysOut, -1e6, 0.0);
        assert_eq!(r, NudgeResult::Rejected(NudgeReject::OutOfBounds));
        assert_eq!(cfg, before);
    }

    // On Moved, only the target offset must change. The tick labels are
    // contained on their drawn parts, so the measure must lay them out: one
    // that cannot place them rejects the move and leaves the config as it was.
    #[test]
    fn nudge_moved_changes_only_target_offset() {
        let mut cfg = default_config();
        let before = cfg.clone();
        assert_eq!(
            cfg.nudge(
                Element::AxisLabel(Side::Left),
                &crate::select::tests::FixedMeasure,
                1.0,
                0.0
            ),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
        assert_eq!(cfg, before);
        assert_eq!(
            cfg.nudge(Element::AxisLabel(Side::Left), &LaysOut, 1.0, 0.0),
            NudgeResult::Moved
        );
        let mut expected = before.clone();
        expected.left_y.label_style.label_offset_x += 1.0;
        assert_eq!(cfg, expected);
    }

    // A y-axis moves only horizontally, *detached*: the parallel (dy)
    // component is discarded, the motion lands in line_offset, and the data
    // area does NOT move — the axis floats away while tick positions along
    // the axis stay aligned with the data. A drag on the axis element takes
    // the same path.
    #[test]
    fn y_axis_nudge_and_drag_detach_horizontally_and_keep_data_area() {
        use crate::drag::Draggable;
        use crate::select::AxisElement;

        let mut cfg = default_config();
        let da_before = cfg.data_area().unwrap();
        let margin_before = cfg.left_y.out_margin;

        let r = cfg.nudge(Element::Axis(Side::Left), &LaysOut, 10.0, 999.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.left_y.line_offset, 10.0);
        // Layout untouched — margin and data area identical.
        assert_eq!(cfg.left_y.out_margin, margin_before);
        assert_eq!(cfg.data_area().unwrap(), da_before);

        let mut dragged = default_config();
        let el = AxisElement { side: Side::Left };
        assert_eq!(
            el.drag_by(&mut dragged, &LaysOut, 12.0, -40.0),
            NudgeResult::Moved
        );
        assert_eq!(dragged.left_y.line_offset, 12.0);
        assert_eq!(dragged.left_y.out_margin, margin_before);
        assert_eq!(dragged.data_area().unwrap(), da_before);
    }

    // An x-axis moves only vertically (the dx component is discarded), by
    // nudge or by drag, and neither its margin nor the data area moves.
    #[test]
    fn x_axis_nudge_and_drag_detach_vertically_and_keep_data_area() {
        use crate::drag::Draggable;
        use crate::select::AxisElement;

        let mut cfg = default_config();
        let da_before = cfg.data_area().unwrap();
        let margin_before = cfg.bottom_x.out_margin;

        let r = cfg.nudge(Element::Axis(Side::Bottom), &LaysOut, 999.0, -8.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.bottom_x.line_offset, -8.0);
        assert_eq!(cfg.bottom_x.out_margin, margin_before);
        assert_eq!(cfg.data_area().unwrap(), da_before);

        let mut dragged = default_config();
        let el = AxisElement { side: Side::Bottom };
        assert_eq!(
            el.drag_by(&mut dragged, &LaysOut, -40.0, -6.0),
            NudgeResult::Moved
        );
        assert_eq!(dragged.bottom_x.line_offset, -6.0);
        assert_eq!(dragged.bottom_x.out_margin, margin_before);
        assert_eq!(dragged.data_area().unwrap(), da_before);
    }

    // The detached axis may cross INTO the data area (e.g. y-axis at x = 0)
    // but never leave the chart area.
    #[test]
    fn axis_nudge_allows_crossing_into_data_area() {
        let mut cfg = default_config();
        let r = cfg.nudge(Element::Axis(Side::Left), &LaysOut, 50.0, 0.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.left_y.line_offset, 50.0);
    }

    #[test]
    fn axis_nudge_rejects_leaving_chart_area() {
        let mut cfg = default_config();
        let before = cfg.clone();
        let r = cfg.nudge(Element::Axis(Side::Left), &LaysOut, -1e6, 0.0);
        assert_eq!(r, NudgeResult::Rejected(NudgeReject::OutOfBounds));
        assert_eq!(cfg, before);
    }

    // Data-area edges (resize path) still move via margins.
    #[test]
    fn data_area_edge_nudge_moves_margin_and_data_area() {
        let mut cfg = default_config();
        let da_before = cfg.data_area().unwrap();
        let margin_before = cfg.left_y.out_margin;

        let r = cfg.nudge(Element::DataAreaEdge(Side::Left), &LaysOut, 10.0, 999.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.left_y.out_margin, margin_before + 10.0);
        let da_after = cfg.data_area().unwrap();
        assert_eq!(da_after.x, da_before.x + 10);
        assert_eq!(da_after.width, da_before.width - 10);
    }

    #[test]
    fn data_area_edge_rejects_negative_margin() {
        let mut cfg = default_config();
        let before = cfg.clone();
        let r = cfg.nudge(Element::DataAreaEdge(Side::Left), &LaysOut, -1e6, 0.0);
        assert_eq!(r, NudgeResult::Rejected(NudgeReject::OutOfBounds));
        assert_eq!(cfg, before);
    }

    #[test]
    fn data_area_edge_rejects_margin_overflow() {
        let mut cfg = default_config();
        let before = cfg.clone();
        let w = cfg.chart_area.width as f32;
        let r = cfg.nudge(Element::DataAreaEdge(Side::Left), &LaysOut, w * 2.0, 0.0);
        assert_eq!(r, NudgeResult::Rejected(NudgeReject::OutOfBounds));
        assert_eq!(cfg, before);
    }
}

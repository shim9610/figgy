#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    /// Whether this rect lies inside `outer`, edges included.
    pub fn within(&self, outer: &Rect) -> bool {
        self.x >= outer.x
            && self.y >= outer.y
            && self.x.saturating_add(self.width) <= outer.x.saturating_add(outer.width)
            && self.y.saturating_add(self.height) <= outer.y.saturating_add(outer.height)
    }

    pub fn scissor(&self) -> (u32, u32, u32, u32) {
        (self.x, self.y, self.width, self.height)
    }

    pub fn viewport(&self) -> (f32, f32, f32, f32) {
        (
            self.x as f32,
            self.y as f32,
            self.width as f32,
            self.height as f32,
        )
    }
}

/// The panel of a list that inset `index` is nested in (#203, #205): the
/// innermost one whose data area holds the inset's panel rect, a panel with
/// a rect of its own, the first of several that share one. `rects(i)` gives panel `i`'s panel rect and data
/// area. Panels that share a rect (overlays, members) are never nested in
/// one another. `None` for a panel of its own.
pub fn inset_parent(
    count: usize,
    rects: impl Fn(usize) -> (Rect, Rect),
    index: usize,
) -> Option<usize> {
    let (rect, _) = rects(index);
    (0..count)
        .filter(|&outer| outer != index)
        .filter_map(|outer| {
            let (own, data) = rects(outer);
            (own != rect && rect.within(&data)).then_some((outer, data))
        })
        .min_by_key(|&(outer, data)| (u64::from(data.width) * u64::from(data.height), outer))
        .map(|(outer, _)| outer)
}

/// The order insets are painted in, bottom to top (#205): a tree walk in
/// which each inset comes after the panel it is nested in and before its
/// next sibling, siblings in list order. An inset inside another is drawn
/// over it, and a later sibling over both. Panels of their own are not in
/// the walk. A pointer belongs to the last inset of this order that holds
/// it. The walk allocates nothing; each step scans the list.
pub fn inset_paint_order<F: Fn(usize) -> (Rect, Rect)>(
    count: usize,
    rects: F,
) -> impl Iterator<Item = usize> {
    // `None` for a panel of its own, `Some(None)` for a top-level inset and
    // `Some(Some(p))` for an inset in inset `p`: siblings share it.
    fn level(
        count: usize,
        rects: &impl Fn(usize) -> (Rect, Rect),
        index: usize,
    ) -> Option<Option<usize>> {
        inset_parent(count, rects, index).map(|p| inset_parent(count, rects, p).map(|_| p))
    }
    let start = (0..count).find(|&index| level(count, &rects, index) == Some(None));
    std::iter::successors(start, move |&index| {
        if let Some(child) = (0..count).find(|&c| inset_parent(count, &rects, c) == Some(index)) {
            return Some(child);
        }
        let mut at = index;
        loop {
            let up = level(count, &rects, at)?;
            if let Some(next) = (at + 1..count).find(|&n| level(count, &rects, n) == Some(up)) {
                return Some(next);
            }
            at = up?;
        }
    })
}

/// Float-precision pixel rect — element bounds, selection boxes, and other
/// sub-pixel screen geometry. `Rect` (u32) stays the type for buffer/texture
/// areas; this one is for visual geometry derived from f32 layout math.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl RectF {
    pub fn from_rect(r: &Rect) -> Self {
        Self {
            x: r.x as f32,
            y: r.y as f32,
            width: r.width as f32,
            height: r.height as f32,
        }
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.width && py >= self.y && py <= self.y + self.height
    }

    /// Grow by `pad` on every side (negative shrinks; width/height floor at 0).
    pub fn expanded(&self, pad: f32) -> Self {
        Self {
            x: self.x - pad,
            y: self.y - pad,
            width: (self.width + pad * 2.0).max(0.0),
            height: (self.height + pad * 2.0).max(0.0),
        }
    }

    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    /// The rect at `scale` times its size about the origin.
    pub fn scaled(&self, scale: f32) -> Self {
        Self {
            x: self.x * scale,
            y: self.y * scale,
            width: self.width * scale,
            height: self.height * scale,
        }
    }

    /// The centre of the rect.
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.width * 0.5, self.y + self.height * 0.5)
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChartArea(pub Rect);

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataArea(pub Rect);

impl std::ops::Deref for ChartArea {
    type Target = Rect;
    fn deref(&self) -> &Rect {
        &self.0
    }
}
impl std::ops::DerefMut for ChartArea {
    fn deref_mut(&mut self) -> &mut Rect {
        &mut self.0
    }
}
impl std::ops::Deref for DataArea {
    type Target = Rect;
    fn deref(&self) -> &Rect {
        &self.0
    }
}
impl std::ops::DerefMut for DataArea {
    fn deref_mut(&mut self) -> &mut Rect {
        &mut self.0
    }
}

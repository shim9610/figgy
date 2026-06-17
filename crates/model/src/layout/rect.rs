// ============================================================================
// §2.1 Rect + ChartArea/DataArea newtypes
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
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

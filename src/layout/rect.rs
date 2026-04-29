// ============================================================================
// §2.1 Rect + ChartArea/DataArea newtypes
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct ChartArea(pub Rect);

#[derive(Debug, Clone, PartialEq)]
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

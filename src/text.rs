use crate::color::Color;

/// A rich text value composed of styled segments.
#[derive(Debug, Clone, PartialEq)]
pub struct RichText {
    pub segments: Vec<RichSegment>,
    pub color: Color,
    pub font_size: f32,
    /// 폰트 패밀리명 (예: "Noto Sans"). skia 백엔드에서 `Typeface` 조회에 사용.
    pub font: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RichSegment {
    pub text: char,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub greek: bool,
}




use crate::color::Color;
use crate::line::LineStylePreset;
use crate::text::RichText;



/// wgpu에 전달할 렌더링 영역(rect) 정의.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartArea {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl ChartArea {
    /// `RenderPass::set_scissor_rect` 인자 튜플.
    pub fn scissor(&self) -> (u32, u32, u32, u32) {
        (self.x, self.y, self.width, self.height)
    }

    /// `RenderPass::set_viewport` 의 x/y/width/height 튜플 (depth 제외).
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
pub struct ChartTitleOptions {
    pub text: RichText,
    pub visible: bool,
    pub offset_x: f32,
    pub offset_y: f32,
    pub top_margin: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChartType {
    ScatterLine,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    chart_id : String,
    chart_type : ChartType,
    area : ChartArea,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataArea {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct GridOptions {
    pub show_major_x: bool,
    pub major_x_color: Color,
    pub major_x_width: f32,
    pub major_x_style: LineStylePreset,

    pub show_major_y: bool,
    pub major_y_color: Color,
    pub major_y_width: f32,
    pub major_y_style: LineStylePreset,

    pub show_minor_x: bool,
    pub minor_x_color: Color,
    pub minor_x_width: f32,
    pub minor_x_style: LineStylePreset,

    pub show_minor_y: bool,
    pub minor_y_color: Color,
    pub minor_y_width: f32,
    pub minor_y_style: LineStylePreset,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AxisScale {
    Linear,
    Logarithmic,
}
#[derive(Debug, Clone, PartialEq)]
pub enum TickVisibility {
    None,
    Outside,
    Inside,
    Both,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AxsisTitleOptions {
    pub text: RichText,
    pub visible: bool,
    pub offset_x: f32,
    pub offset_y: f32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct LabelStyle {
    pub visible: bool,
    pub color: Color,
    pub font_size: f32,
    pub label_visible: bool,
    pub label_font: String,
    pub label_offset_x: f32,
    pub label_offset_y: f32,
}


#[derive(Debug, Clone, PartialEq)]
pub struct AxisOptions {
    pub scale: AxisScale,
    pub min: f64,
    pub max: f64,
    pub major_spacing: f64,
    pub minor_count: usize,
    pub inverted: bool,
    pub label_style: LabelStyle,
    pub tick : TickVisibility,
    pub title_option : AxsisTitleOptions,
    pub out_margin: f32,
}



#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub charts : Chart,
    pub top_x : AxisOptions,
    pub bottom_x : AxisOptions,
    pub left_y  : AxisOptions,
    pub right_y : AxisOptions,
    pub chart_title : ChartTitleOptions,
    pub grid : GridOptions,
    pub data_area : DataArea,
 
}
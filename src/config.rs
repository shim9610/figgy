use crate::color::Color;
use crate::format::LabelFormat;
use crate::layout::ChartArea;
use crate::line::LineStylePreset;
use crate::text::RichText;

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
    pub chart_id: String,
    pub chart_type: ChartType,
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
pub struct AxisTitleOptions {
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
    pub format: LabelFormat,
    pub significant_digits: u8,
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
    pub tick: TickVisibility,
    pub title_option: AxisTitleOptions,
    /// Outer margin past the axis title band. Always counted, regardless of title visibility.
    pub out_margin: f32,

    // Axis line appearance. Tick marks reuse these (color / width / style).
    pub line_visible: bool,
    pub line_color: Color,
    pub line_width: f32,
    pub line_style: LineStylePreset,

    // Tick mark lengths. `margins()` uses `major_tick_length`.
    pub major_tick_length: f32,
    pub minor_tick_length: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub chart_area: ChartArea,
    pub chart: Chart,
    pub top_x: AxisOptions,
    pub bottom_x: AxisOptions,
    pub left_y: AxisOptions,
    pub right_y: AxisOptions,
    pub chart_title: ChartTitleOptions,
    pub grid: GridOptions,
    pub legend: Legend,
}

impl Config {
    /// Multiply every **pixel-based visual dim** by `scale`. Data ranges, colors,
    /// and scale enums are untouched. For resolution-invariant high-DPI export.
    pub fn scaled(&self, scale: f32) -> Self {
        let mut c = self.clone();
        c.scale_in_place(scale);
        c
    }

    pub fn scale_in_place(&mut self, s: f32) {
        let scale_u32 = |v: u32| ((v as f32) * s).round() as u32;
        self.chart_area.0.x = scale_u32(self.chart_area.0.x);
        self.chart_area.0.y = scale_u32(self.chart_area.0.y);
        self.chart_area.0.width = scale_u32(self.chart_area.0.width);
        self.chart_area.0.height = scale_u32(self.chart_area.0.height);

        self.chart_title.text.font_size *= s;
        self.chart_title.top_margin *= s;
        self.chart_title.offset_x *= s;
        self.chart_title.offset_y *= s;

        for axis in [&mut self.top_x, &mut self.bottom_x, &mut self.left_y, &mut self.right_y] {
            axis.label_style.font_size *= s;
            axis.label_style.label_offset_x *= s;
            axis.label_style.label_offset_y *= s;
            axis.title_option.text.font_size *= s;
            axis.title_option.offset_x *= s;
            axis.title_option.offset_y *= s;
            axis.out_margin *= s;
            axis.line_width *= s;
            axis.major_tick_length *= s;
            axis.minor_tick_length *= s;
        }

        self.grid.major_x_width *= s;
        self.grid.major_y_width *= s;
        self.grid.minor_x_width *= s;
        self.grid.minor_y_width *= s;

        self.legend.font_size *= s;
        self.legend.padding *= s;
        self.legend.line_height *= s;
        self.legend.sample_width *= s;
        self.legend.sample_text_gap *= s;
        for entry in &mut self.legend.entries {
            entry.line_width *= s;
        }
    }
}

// Legend types.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LegendCorner { TopLeft, TopRight, BottomLeft, BottomRight }

#[derive(Debug, Clone, PartialEq)]
pub enum LegendEntryKind {
    /// Line sample only.
    Line,
    /// Point sample only.
    Scatter,
    /// Line + point sample.
    LineScatter,
}

/// One legend row (sample + label).
#[derive(Debug, Clone, PartialEq)]
pub struct LegendEntry {
    pub label: crate::text::RichText,
    pub color: crate::color::Color,
    pub line_width: f32,
    pub kind: LegendEntryKind,
}

/// Legend box rendered in one corner of the chart's data area.
#[derive(Debug, Clone, PartialEq)]
pub struct Legend {
    pub visible: bool,
    pub entries: Vec<LegendEntry>,
    pub corner: LegendCorner,
    /// Inset from the data_area inner corner + padding inside the box.
    pub padding: f32,
    /// Label font size.
    pub font_size: f32,
    /// Per-entry row height.
    pub line_height: f32,
    /// Width of the color sample (line / dot).
    pub sample_width: f32,
    /// Gap between sample and label.
    pub sample_text_gap: f32,
    /// Box background and border.
    pub bg_color: crate::color::Color,
    pub border_color: crate::color::Color,
}



pub struct ChartResistry {
    pub chart_id: String,
    pub chart_type: ChartType,
    pub chart_xy_index : Vec<(usize,usize)>,
}


pub struct ColumnResistry {
    pub rander_target: Vec<(ChartArea,ChartResistry)>,
   // pub column_index_map: HashMap<ColumnId,usize>
}


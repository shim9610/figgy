//! The document editor of a monotonic chart (#271).
//!
//! A monotonic chart's x-to-pixel mapping (the x range, x scale, breaks and
//! direction, and the plot area its width comes from) is written only by
//! its monotonic calls, which take the source. [`MonotonicConfig`] edits
//! everything else of its [`Config`]: it has no setter for the mapping, and
//! every edit that would move it is refused ([`same_mapping`]).

use crate::config::{AxisOptions, ChartTitleOptions, Config, GridOptions, LabelStyle};
use crate::config::{AxisTitleOptions, DataSelectionsConfig, TickVisibility};
use crate::drag::Draggable;
use crate::layer::{AnnotationLayer, LayerRef};
use crate::layout::{Element, NudgeReject, NudgeResult, Side};
use crate::legend::Legend;
use crate::line::LineStylePreset;
use crate::text::MeasureText;
use crate::{Color, RuleKind};

/// The editor of a monotonic chart's document (#271): read access to the
/// whole [`Config`], and edits of everything but the x-to-pixel mapping. A
/// renderer lends one to an edit closure over its own copy of the document
/// and checks the result with [`same_mapping`] before it keeps it.
pub struct MonotonicConfig<'a> {
    config: &'a mut Config,
}

/// One axis of a [`MonotonicConfig`]: its title, labels, ticks and line;
/// a y axis also its range and scale. An x axis has no range or scale
/// setter.
pub struct MonotonicAxis<'a> {
    axis: &'a mut AxisOptions,
    is_x: bool,
}

impl<'a> MonotonicConfig<'a> {
    /// The editor over `config`.
    pub fn new(config: &'a mut Config) -> Self {
        Self { config }
    }

    /// The whole document, read only.
    pub fn config(&self) -> &Config {
        self.config
    }

    pub fn legend(&mut self) -> &mut Legend {
        &mut self.config.legend
    }

    pub fn chart_title(&mut self) -> &mut ChartTitleOptions {
        &mut self.config.chart_title
    }

    pub fn grid(&mut self) -> &mut GridOptions {
        &mut self.config.grid
    }

    pub fn annotations(&mut self) -> &mut AnnotationLayer {
        &mut self.config.annotations
    }

    /// The data selection (the picked highlight).
    pub fn picked_data(&mut self) -> &mut Option<DataSelectionsConfig> {
        &mut self.config.picked_data
    }

    pub fn set_active_layer(&mut self, layer: LayerRef) {
        self.config.set_active_layer(layer);
    }

    pub fn set_background(&mut self, color: Option<Color>) {
        self.config.background = color;
    }

    pub fn set_plot_background(&mut self, color: Option<Color>) {
        self.config.plot_background = color;
    }

    /// The axis on `side`.
    pub fn axis(&mut self, side: Side) -> MonotonicAxis<'_> {
        let (axis, is_x) = match side {
            Side::Bottom => (&mut self.config.bottom_x, true),
            Side::Top => (&mut self.config.top_x, true),
            Side::Left => (&mut self.config.left_y, false),
            Side::Right => (&mut self.config.right_y, false),
        };
        MonotonicAxis { axis, is_x }
    }

    /// Overlay `index`'s y axis.
    pub fn overlay_axis(&mut self, index: usize) -> Option<MonotonicAxis<'_>> {
        self.config
            .overlays
            .get_mut(index)
            .map(|overlay| MonotonicAxis {
                axis: &mut overlay.y_axis,
                is_x: false,
            })
    }

    /// Overlay `index`'s own x axis (`OverlayLayer::x_axis`). Its range is
    /// the monotonic calls', as the chart's x axes' are.
    pub fn overlay_x_axis(&mut self, index: usize) -> Option<MonotonicAxis<'_>> {
        self.config
            .overlays
            .get_mut(index)
            .map(|overlay| MonotonicAxis {
                axis: &mut overlay.x_axis,
                is_x: true,
            })
    }

    /// Overlay `index`'s own legend.
    pub fn overlay_legend(&mut self, index: usize) -> Option<&mut Option<Legend>> {
        self.config
            .overlays
            .get_mut(index)
            .map(|overlay| &mut overlay.legend)
    }

    /// Subplot `index`'s document, edited under the same rules.
    pub fn subplot(&mut self, index: usize) -> Option<MonotonicConfig<'_>> {
        self.config
            .subplots
            .get_mut(index)
            .map(|subplot| MonotonicConfig::new(&mut subplot.config))
    }

    /// Moves a decoration as [`Config::nudge`] does (a drag of the title,
    /// the legend, an axis title or tick labels, or the plot without
    /// resizing it). A move that would change the mapping (an axis or a
    /// plot edge, a subplot frame) is rejected and changes nothing.
    pub fn nudge(
        &mut self,
        element: Element,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        self.kept_mapping(|config| config.nudge(element, measure, dx, dy))
    }

    /// Applies a pointer drag of `drag` from `at` as
    /// [`Draggable::drag_from`] does, under the rule of [`Self::nudge`].
    pub fn drag(
        &mut self,
        drag: &dyn Draggable,
        measure: &dyn MeasureText,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        self.kept_mapping(|config| drag.drag_from(config, measure, at, dx, dy))
    }

    /// Replaces the document with `config`, which must keep the mapping
    /// ([`same_mapping`]): the bridge for a whole document edited as data
    /// (a document read back as JSON, say). Refused with
    /// [`RuleKind::MonotonicMappingEdit`] otherwise, changing nothing.
    pub fn assign(&mut self, config: Config) -> Result<(), RuleKind> {
        if !same_mapping(self.config, &config) {
            return Err(RuleKind::MonotonicMappingEdit);
        }
        *self.config = config;
        Ok(())
    }

    fn kept_mapping(&mut self, edit: impl FnOnce(&mut Config) -> NudgeResult) -> NudgeResult {
        let before = self.config.clone();
        let moved = edit(self.config);
        if !same_mapping(&before, self.config) {
            *self.config = before;
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        moved
    }
}

impl MonotonicAxis<'_> {
    /// The axis, read only.
    pub fn options(&self) -> &AxisOptions {
        self.axis
    }

    pub fn title(&mut self) -> &mut AxisTitleOptions {
        &mut self.axis.title_option
    }

    /// The tick labels: their style, format (the axis units) and offsets.
    pub fn labels(&mut self) -> &mut LabelStyle {
        &mut self.axis.label_style
    }

    pub fn ticks(&mut self) -> &mut TickVisibility {
        &mut self.axis.tick
    }

    pub fn set_major_spacing(&mut self, spacing: f64) {
        self.axis.major_spacing = spacing;
    }

    pub fn set_minor_count(&mut self, count: usize) {
        self.axis.minor_count = count;
    }

    pub fn set_tick_lengths(&mut self, major: f32, minor: f32) {
        self.axis.major_tick_length = major;
        self.axis.minor_tick_length = minor;
    }

    pub fn set_line(&mut self, visible: bool, color: Color, width: f32, style: LineStylePreset) {
        self.axis.line_visible = visible;
        self.axis.line_color = color;
        self.axis.line_width = width;
        self.axis.line_style = style;
    }

    /// A y axis' range. An x axis' range is the monotonic calls': refused
    /// with [`RuleKind::MonotonicMappingEdit`].
    pub fn set_range(&mut self, min: f64, max: f64) -> Result<(), RuleKind> {
        if self.is_x {
            return Err(RuleKind::MonotonicMappingEdit);
        }
        self.axis.min = min;
        self.axis.max = max;
        Ok(())
    }
}

/// Whether `a` and `b` map x to the same device pixels on every panel
/// (#271): the same document size (the display scale follows it), and per
/// panel (the chart and each subplot) the same plot width and the same x
/// range, scale, breaks, direction and categories, for the chart's x axis
/// and for each overlay's, with the same link flag. Where the plot lies does
/// not matter: a moved plot keeps its mapping. What a [`MonotonicConfig`]
/// edit must keep.
pub fn same_mapping(a: &Config, b: &Config) -> bool {
    let (da, db) = (a.document_area(), b.document_area());
    (da.width, da.height) == (db.width, db.height)
        && panel_mapping_eq(a, b)
        && a.subplots.len() == b.subplots.len()
        && a.subplots
            .iter()
            .zip(&b.subplots)
            .all(|(a, b)| same_mapping(&a.config, &b.config))
}

fn panel_mapping_eq(a: &Config, b: &Config) -> bool {
    let width = |config: &Config| config.data_area().ok().map(|area| area.0.width);
    width(a) == width(b)
        && x_mapping_eq(&a.bottom_x, &b.bottom_x)
        && a.overlays.len() == b.overlays.len()
        && a.overlays
            .iter()
            .zip(&b.overlays)
            .all(|(a, b)| a.link_x == b.link_x && (a.link_x || x_mapping_eq(&a.x_axis, &b.x_axis)))
}

fn x_mapping_eq(x: &AxisOptions, y: &AxisOptions) -> bool {
    x.min.to_bits() == y.min.to_bits()
        && x.max.to_bits() == y.max.to_bits()
        && x.scale == y.scale
        && x.inverted == y.inverted
        && x.breaks == y.breaks
        && x.categories == y.categories
        && x.category_window == y.category_window
}

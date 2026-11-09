//! Chart conveniences: a sized chart, a theme, a legend from the series,
//! and margins fitted to the text (#265).

use crate::theme::{SubplotScope, Theme};
use crate::{Chart, SeriesConfig};

impl Chart {
    /// A chart of the documented default look whose chart area is `width` ×
    /// `height` logical pixels at the origin.
    pub fn sized(width: u32, height: u32) -> Self {
        let mut config = crate::config::Config::default();
        config.chart_area = crate::layout::ChartArea(crate::layout::Rect {
            x: 0,
            y: 0,
            width,
            height,
        });
        Self::new(config)
    }

    /// A chart of the documented default look whose chart area is `width` ×
    /// `height` logical pixels with its top-left corner at `(x, y)`: a cell
    /// of a subplot grid, or a chart placed in a larger surface.
    pub fn placed(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self::sized(width, height).with_area(x, y, width, height)
    }

    /// The chart with its chart area moved to `(x, y)` and resized to
    /// `width` × `height` logical pixels, for a chart built from a
    /// document (`Config::from_categorical` and its siblings).
    pub fn with_area(mut self, x: u32, y: u32, width: u32, height: u32) -> Self {
        self.set_area(x, y, width, height);
        self
    }

    /// Move the chart area of a chart that already exists to `(x, y)` and
    /// resize it to `width` × `height` logical pixels, as a window resize
    /// does.
    pub fn set_area(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.config_mut().chart_area = crate::layout::ChartArea(crate::layout::Rect {
            x,
            y,
            width,
            height,
        });
    }

    /// The chart with an overlay layer: a secondary y axis on the right
    /// that scales the series `series` names.
    pub fn with_overlay(mut self, y_axis: crate::config::AxisOptions, series: &[&str]) -> Self {
        self.config_mut().overlays.push(crate::OverlayLayer {
            y_axis,
            series: series.iter().map(|id| (*id).to_owned()).collect(),
            legend: None,
        });
        self
    }

    /// A legend entry for a meaning that is not a series (a direction, a
    /// state, a condition): a filled square in `color` and `label`. The
    /// legend is shown.
    pub fn with_legend_swatch(mut self, label: &str, color: crate::Color) -> Self {
        let legend = &mut self.config_mut().legend;
        let mark = crate::text::RichSegment::fielded(
            crate::legend::scatter_shape_char(&crate::data_config::ScatterShape::SquareFilled),
            crate::legend::SYMBOL_FIELD_EM,
            Some(color),
        );
        crate::config::append_legend_entry(&mut legend.content, vec![mark], label);
        legend.visible = true;
        self
    }

    /// The chart with `annotation` appended to its annotation layer.
    pub fn with_annotation(mut self, annotation: crate::Annotation) -> Self {
        self.config_mut().annotations.items.push(annotation);
        self
    }

    /// The chart with the tick labels of `bottom_x` and `left_y` written in
    /// `format`; call it after `set_x_range` / `set_y_range`, which choose one.
    pub fn with_label_format(mut self, format: crate::format::LabelFormat) -> Self {
        let config = self.config_mut();
        config.bottom_x.label_style.format = format.clone();
        config.left_y.label_style.format = format;
        self
    }

    /// The chart with all four grid lines (major and minor, x and y) off.
    pub fn without_grid(mut self) -> Self {
        let grid = &mut self.config_mut().grid;
        grid.show_major_x = false;
        grid.show_major_y = false;
        grid.show_minor_x = false;
        grid.show_minor_y = false;
        self
    }

    /// The chart with its colour bar's axis title shown as plain `text` in
    /// the title's current style; without a colour bar nothing changes.
    pub fn with_colorbar_title(mut self, text: &str) -> Self {
        if let Some(bar) = self.config_mut().colorbar.as_mut() {
            let title = &mut bar.axis.title_option;
            title.text.segments = crate::text::rich_segments_from_text(text);
            title.visible = true;
        }
        self
    }

    /// The chart in the polar coordinates `polar` (`Config::set_polar`).
    pub fn with_polar(mut self, polar: crate::PolarOptions) -> Self {
        self.config_mut().set_polar(Some(polar));
        self
    }

    /// The chart with `subplot` appended to its subplots.
    pub fn with_subplot(mut self, subplot: crate::Subplot) -> Self {
        self.config_mut().subplots.push(subplot);
        self
    }

    /// One legend entry per labelled series, in order: its symbol from the
    /// series itself ([`crate::legend::series_symbol_segments`], so a colour
    /// is written once) and its label. Unlabelled series get no entry. The
    /// legend is shown.
    pub fn with_legend_from(mut self, series: &[SeriesConfig]) -> Self {
        let config = self.config_mut();
        for item in series {
            let Some(label) = &item.label else {
                continue;
            };
            let colormap = config.colorbar.as_ref().map(|bar| &bar.colormap);
            let symbol = crate::legend::series_symbol_segments(item, colormap);
            crate::legend::append_legend_entry_rich(
                &mut config.legend.content,
                symbol,
                label.segments.clone(),
            );
        }
        config.legend.visible = true;
        self
    }

    /// Apply `theme` to the chart and its series ([`Theme::apply`]): the
    /// chrome of the chart, its overlays and the subplots `subplots` names,
    /// and the series drawn there. The layout is then fitted to the new text
    /// sizes ([`Self::fit_layout`]).
    pub fn apply_theme(
        &mut self,
        theme: &Theme,
        series: &mut [SeriesConfig],
        subplots: SubplotScope,
    ) {
        theme.apply(self.config_mut(), series, subplots);
        self.fit_layout();
    }

    /// The chart with `theme`'s chrome on it and on every subplot, its
    /// layout fitted. Its series take the theme through
    /// [`Theme::style_series`], or together with the chart through
    /// [`Self::apply_theme`].
    pub fn with_theme(mut self, theme: &Theme) -> Self {
        theme.apply_chrome(self.config_mut(), SubplotScope::All);
        self.fit_layout();
        self
    }

    /// Fit the axis bands and margins to their labels and titles
    /// ([`crate::layout::auto_layout`]), measured with the font the chart's
    /// draw style renders.
    pub fn fit_layout(&mut self) {
        let measure = crate::CpuTextMeasure::for_style(&self.config().draw_style);
        let fitted = crate::layout::auto_layout(self.config(), &measure);
        *self.config_mut() = fitted;
    }
}

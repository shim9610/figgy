//! Chart conveniences: a sized chart, a legend from the series, and
//! margins fitted to the text (#265).

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

    /// Fit the axis bands and margins to their labels and titles
    /// ([`crate::layout::auto_layout`]), measured with the font the chart's
    /// draw style renders.
    pub fn fit_layout(&mut self) {
        let measure = crate::CpuTextMeasure::for_style(&self.config().draw_style);
        let fitted = crate::layout::auto_layout(self.config(), &measure);
        *self.config_mut() = fitted;
    }
}

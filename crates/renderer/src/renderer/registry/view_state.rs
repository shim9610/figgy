//! The view state a host saves and restores per chart: axis ranges and the selection.

use super::*;

/// Logical data-to-screen state derived from the renderer-owned `Config`.
///
/// This value is never stored as a parallel truth source. The getter derives
/// it from `Config`; the setter applies it back to those same four axes.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartViewState {
    pub chart_area: Rect,
    pub top_x: AxisViewState,
    pub bottom_x: AxisViewState,
    pub left_y: AxisViewState,
    pub right_y: AxisViewState,
    /// One entry per overlay layer, in `Config::overlays` order: its own Y
    /// axis (`OverlayLayer::y_axis`). A view state applied to a chart must
    /// carry exactly as many entries as the chart has overlays.
    pub overlay_y: Vec<AxisViewState>,
    /// One entry per overlay layer, in `Config::overlays` order: its own X
    /// axis (`OverlayLayer::x_axis`), which scales its series while its
    /// `link_x` is off. As many entries as `overlay_y`. A linked overlay
    /// follows `bottom_x`, so one call moves every linked layer.
    pub overlay_x: Vec<AxisViewState>,
    /// One entry per subplot, in `Config::subplots` order, each the view
    /// state of that subplot's chart (its own `subplots` empty). A linked
    /// axis reads as its target's range; writing a new range to it moves the
    /// whole link group, in the same call as everything else.
    pub subplots: Vec<ChartViewState>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AxisViewState {
    pub scale: AxisScale,
    pub min: f64,
    pub max: f64,
    pub inverted: bool,
}

impl AxisViewState {
    pub(in crate::renderer) fn from_axis(axis: &AxisOptions) -> Self {
        let AxisOptions {
            scale,
            min,
            max,
            // Re-derived from the range when a view state is applied.
            major_spacing: _,
            minor_count: _,
            inverted,
            // Style and band: not part of the data-to-screen mapping.
            label_style: _,
            tick: _,
            title_option: _,
            out_margin: _,
            line_offset: _,
            line_visible: _,
            line_color: _,
            line_width: _,
            line_style: _,
            major_tick_length: _,
            minor_tick_length: _,
            // The range of a categorical axis (`data_range`).
            categories: _,
            category_groups: _,
            // The window is the range of a categorical axis (`data_range`).
            category_window: _,
            // Where the line is drawn, not how data maps.
            cross_at: _,
            // Which ticks a log axis draws, not how data maps.
            log_ticks: _,
            // A view moves the range; the cut ranges stay on the axis as
            // they are, like the tick policy (#200).
            breaks: _,
            // What the axis prints, not how data maps (#273).
            unit: _,
        } = axis;
        let (min, max) = match axis.categories {
            Some(_) => axis.data_range(),
            None => (*min, *max),
        };
        Self {
            scale: scale.clone(),
            min,
            max,
            inverted: *inverted,
        }
    }

    pub(in crate::renderer) fn apply_to(&self, axis: &mut AxisOptions) {
        let Self {
            scale,
            min,
            max,
            inverted,
        } = self;
        axis.scale = scale.clone();
        crate::chart::apply_view_range(axis, *min, *max, scale);
        axis.inverted = *inverted;
    }
}

impl ChartViewState {
    pub(in crate::renderer) fn from_config(config: &Config) -> Self {
        let Config {
            chart_area,
            top_x,
            bottom_x,
            left_y,
            right_y,
            // Chrome, selections, z scale, style and routing: not part of
            // the data-to-screen mapping.
            chart_title: _,
            grid: _,
            legend: _,
            picked_data: _,
            colorbar: _,
            draw_style: _,
            // The parallel axes' ranges place each column on its axis; the
            // panel's own axes are the categorical positions and the
            // normalized band.
            coordinates: _,
            overlays,
            subplots,
            active_layer: _,
            annotations: _,
            // How bars share a slot, not how data maps to the screen.
            bar_groups: _,
            // The member shifts are display parameters of the group, kept
            // as they are when a view state moves the ranges.
            line_offsets: _,
            plot_background: _,
            background: _,
        } = config;
        Self {
            chart_area: chart_area.0,
            top_x: AxisViewState::from_axis(top_x),
            bottom_x: AxisViewState::from_axis(bottom_x),
            left_y: AxisViewState::from_axis(left_y),
            right_y: AxisViewState::from_axis(right_y),
            overlay_y: overlays
                .iter()
                .map(|overlay| AxisViewState::from_axis(&overlay.y_axis))
                .collect(),
            overlay_x: overlays
                .iter()
                .map(|overlay| {
                    let OverlayLayer {
                        y_axis: _,
                        // The links are layer structure, not a range.
                        link_x: _,
                        x_axis,
                        link_y: _,
                        series: _,
                        legend: _,
                    } = overlay;
                    AxisViewState::from_axis(x_axis)
                })
                .collect(),
            // Each subplot as it draws, its linked axes following their
            // targets.
            subplots: (0..subplots.len())
                .filter_map(|index| config.subplot_config(index))
                .map(|subplot| Self::from_config(&subplot))
                .collect(),
        }
    }

    pub(in crate::renderer) fn apply_to(&self, config: &mut Config) {
        let Self {
            chart_area,
            top_x,
            bottom_x,
            left_y,
            right_y,
            overlay_y,
            overlay_x,
            subplots,
        } = self;
        let Config {
            chart_area: config_area,
            top_x: config_top_x,
            bottom_x: config_bottom_x,
            left_y: config_left_y,
            right_y: config_right_y,
            // Not part of a view state (see `from_config`).
            chart_title: _,
            grid: _,
            legend: _,
            picked_data: _,
            colorbar: _,
            draw_style: _,
            coordinates: _,
            overlays: config_overlays,
            subplots: config_subplots,
            active_layer: _,
            annotations: _,
            bar_groups: _,
            line_offsets: _,
            plot_background: _,
            background: _,
        } = config;
        config_area.0 = *chart_area;
        top_x.apply_to(config_top_x);
        bottom_x.apply_to(config_bottom_x);
        left_y.apply_to(config_left_y);
        right_y.apply_to(config_right_y);
        for ((y_state, x_state), overlay) in overlay_y.iter().zip(overlay_x).zip(config_overlays) {
            let OverlayLayer {
                y_axis,
                link_x: _,
                x_axis,
                link_y: _,
                series: _,
                legend: _,
            } = overlay;
            y_state.apply_to(y_axis);
            x_state.apply_to(x_axis);
        }
        for (state, subplot) in subplots.iter().zip(config_subplots) {
            let Subplot {
                config,
                series: _,
                link_x: _,
                link_y: _,
                marker: _,
                // A mode, not a range or an area.
                text_scale: _,
            } = subplot;
            state.apply_to(config);
        }
    }
}

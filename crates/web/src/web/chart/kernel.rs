//! The chart kernel every `FiggyChart` constructor builds.

use super::*;

pub(in crate::web) async fn create_chart_kernel(
    canvas: HtmlCanvasElement,
    on_event: Option<js_sys::Function>,
    shader_set: renderer::ShaderSet,
) -> Result<FiggyChart, JsValue> {
    console_error_panic_hook::set_once();

    let (mut renderer, w, h) = {
        let on_event_ref = on_event.as_ref();
        let mut failed = None;
        let mut observer = |event| observe_init_progress(on_event_ref, event, &mut failed);
        let (w, h) = (canvas.width().max(1), canvas.height().max(1));
        // No pipeline compile is awaited (#299): the renderer schedules its
        // shader set's render pipelines on its compile queue, which the
        // browser works through in the background, and the first frame
        // draws with the one pipeline an empty chart needs.
        let renderer = Renderer::for_window_with_shader_set_async_observed(
            wgpu::SurfaceTarget::Canvas(canvas),
            (w, h),
            POOL_CAPACITY,
            shader_set,
            &mut observer,
        )
        .await
        .map_err(js_err);
        if let Some(error) = failed {
            return Err(error);
        }
        let mut renderer = renderer?;
        renderer.set_defrag_policy(DefragPolicy::OnAllocFailure);
        (renderer, w, h)
    };

    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "chart resources", InitPhase::Started),
    )?;

    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    });
    // A new chart keeps its layout fitted to what it holds (#357): the
    // registration fits it, and every edit call fits it again, until the
    // user moves part of it. `frame()` and the exports never fit.
    config.layout_state = renderer::LayoutState::Fitted;
    let chart_id = renderer
        .register_chart(config.clone(), Vec::new())
        .map_err(js_err)?;
    // The canvas shows the document scaled to its surface (#160).
    renderer
        .set_chart_display(chart_id, Some((w, h)))
        .map_err(js_err)?;
    let chart = Chart::new(config);
    let view = renderer
        .create_chart_view(
            &chart,
            Rect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            },
        )
        .map_err(js_err)?;

    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "chart resources", InitPhase::Finished),
    )?;
    let mut chart = FiggyChart {
        renderer,
        chart_id,
        view,
        surface_size: (w, h),
        legend_auto_managed: true,
        color_seq: 0,
        hitmap: HitMap::standard_chart(),
        hitmap_layer: (renderer::LayerRef::Base, 0, None, 0, None),
        hitmap_categories: Vec::new(),
        dragging: false,
        drag_pointer: [0.0; 2],
        resizing: None,
        // A new series takes the default palette's next colour (#357).
        cycle: renderer::ColorCycle::DEFAULT,
        clear_color: Color::WHITE,
        view_dirty: false,
        redraw_pending: true,
        last_presented_stamp: None,
        selected_annotation: None,
        annotation_selection_dirty: false,
        annotation_pointer: false,
        value_label_pointer: false,
        display_held: false,
        feynman_pointer: None,
        plot_resize: None,
        plot_resize_request: None,
        monotonic: None,
    };

    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "first frame", InitPhase::Started),
    )?;
    chart.first_frame_ready().await?;
    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "first frame", InitPhase::Finished),
    )?;
    Ok(chart)
}

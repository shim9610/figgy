//! Range links whose bars are not in a frame's pass (#406): a registered
//! frame leaves a series out while its pipelines compile in a browser
//! (#299). Links whose bars were left out draw nothing in that pass and
//! never look up their own empty column ids; once the bars are drawn, the
//! links are drawn with them. Natively every series is ready, so the test
//! leaves a series out through `pending_series_override`.
use super::*;
use crate::data_config::{BarGroups, CategoryBarDirection, CategoryBarMode};
use crate::layout::{ChartArea, Rect};

const GREEN: Color = Color::from_rgb8(0, 255, 0);
const ORANGE: Color = Color::from_rgb8(255, 128, 0);

fn is_green(p: &[u8]) -> bool {
    p[1] > 200 && p[0] < 60 && p[2] < 60
}

fn is_orange(p: &[u8]) -> bool {
    p[0] > 200 && (90..170).contains(&p[1]) && p[2] < 60
}

fn count(pixels: &[u8], inked: fn(&[u8]) -> bool) -> usize {
    pixels.chunks_exact(4).filter(|p| inked(p)).count()
}

/// Two horizontal range bars (positions 0 and 1, from 0 to 2 and from 5 to
/// 8) and a link from the first to the second, installed together.
fn chart(r: &mut Renderer, links_first: bool) -> ChartId {
    for (id, data) in [
        ("pos", vec![0.0f32, 1.0]),
        ("start", vec![0.0, 5.0]),
        ("end", vec![2.0, 8.0]),
    ] {
        if r.pool.handle_for(id).is_none() {
            r.add_column(id, &crate::Column { data }).unwrap();
        }
    }
    let mut config = crate::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    config.legend.visible = false;
    config.chart_title.visible = false;
    config.bar_groups = Some(BarGroups {
        mode: CategoryBarMode::Grouped,
        direction: CategoryBarDirection::Horizontal,
        group_width: 0.8,
        bar_gap: 4.0,
    });
    config.bottom_x.min = -1.0;
    config.bottom_x.max = 9.0;
    config.left_y.min = -0.5;
    config.left_y.max = 1.5;
    let bars = SeriesConfig::range_bars("bars", "pos", "start", "end")
        .color(GREEN)
        .style_range_bars(|bars| {
            bars.style.material = crate::Material::Flat;
            bars.style.outline = false;
            bars.style.corner_radius = 0.0;
            bars.slot_width = Some(1.0);
        });
    let mut links = SeriesConfig::range_links("links", "bars", [(0, 1)]);
    if let DataRenderType::RangeLinks(style) = &mut links.render_type {
        style.color = ORANGE;
        style.width = 3.0;
    }
    let series = if links_first {
        vec![links, bars]
    } else {
        vec![bars, links]
    };
    r.register_chart(config, series).unwrap()
}

/// Prepare and paint `id`'s registered frame: whether it drew every
/// series, and its pixels.
fn frame(r: &mut Renderer, id: ChartId, view: &mut ChartView) -> (bool, Vec<u8>) {
    let frame = r
        .prepare_registered(&mut [RegisteredChartDrawItem { chart_id: id, view }])
        .expect("a frame without the links' bars prepares");
    let pixels = super::paint_frame_pixels(r, &frame, 1);
    drop(frame);
    r.end_gpu_frame();
    (r.frame_complete(), pixels)
}

#[test]
fn links_whose_bars_are_not_in_the_pass_draw_nothing_until_the_bars_draw() {
    for links_first in [false, true] {
        let (device, queue) = data_render::shared_device().expect("range link GPU required");
        let mut r = Renderer::try_new_with_sample_count(
            RendererDevice::new(device, queue).expect("figgy poll thread"),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
            1,
        )
        .unwrap();
        let id = chart(&mut r, links_first);
        let config = r.chart_config(id).unwrap().clone();
        let mut view = r
            .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
            .unwrap();

        // The bars wait for their pipelines: the links are in the pass,
        // their bars are not. Nothing of either draws, and the frame is
        // not complete.
        r.pending_series_override = Some("bars".into());
        let (complete, pixels) = frame(&mut r, id, &mut view);
        assert!(!complete, "{links_first}: the bars were left out");
        assert_eq!(count(&pixels, is_green), 0, "{links_first}: no bars");
        assert_eq!(count(&pixels, is_orange), 0, "{links_first}: no links");

        // The links wait instead: the bars draw alone.
        r.pending_series_override = Some("links".into());
        let (complete, pixels) = frame(&mut r, id, &mut view);
        assert!(!complete, "{links_first}: the links were left out");
        assert!(count(&pixels, is_green) > 0, "{links_first}: the bars draw");
        assert_eq!(count(&pixels, is_orange), 0, "{links_first}: no links");

        // Both ready: both draw, as a frame that never waited does.
        r.pending_series_override = None;
        let (complete, pixels) = frame(&mut r, id, &mut view);
        assert!(complete, "{links_first}: every series drew");
        assert!(count(&pixels, is_green) > 0, "{links_first}: the bars draw");
        assert!(
            count(&pixels, is_orange) > 0,
            "{links_first}: the links draw once their bars do"
        );
    }
}

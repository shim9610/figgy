use renderer::Color;
use renderer::text::{MeasureText, RichText, TextExtents};
use renderer::text_render::{
    CpuTextMeasure, FontPolicy, TextMetrics, measure_plain_text, measure_rich_text,
};

fn accepts_model_extents(_: TextExtents) {}

#[test]
fn public_text_metrics_path_and_measure_returns_remain_source_compatible() {
    let literal = TextMetrics {
        width: 12.0,
        ascent: 8.0,
        descent: 3.0,
    };
    assert_eq!(literal.height(), 11.0);
    accepts_model_extents(literal);

    let rich = RichText::plain("compat", Color::BLACK, 14.0, "");
    let rich_metrics: TextMetrics = measure_rich_text(&rich, FontPolicy::Standard);
    let plain_metrics: TextMetrics =
        measure_plain_text("compat", "", 14.0, false, false, FontPolicy::Standard);
    assert!(rich_metrics.width > 0.0);
    assert!(plain_metrics.width > 0.0);

    let measured: TextExtents = CpuTextMeasure::default().measure_rich(&rich);
    assert_eq!(measured, rich_metrics);
}

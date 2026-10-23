//! Title measurement shared by bounded layout, drawing and decoration hits.
use crate::{
    FontPolicy,
    decoration::{DecorationPainter, scaled_rich_text},
    raster::Canvas,
    text_render::measure_rich_text,
};
use model::{
    BoundedTitleOptions, DecorationId, TextExtents,
    sink::{PlacedText, Sink, Slot, TextRef},
};

pub(crate) struct TitleLayout {
    title: BoundedTitleOptions,
    metrics: TextExtents,
    baseline: f32,
    pub top: f32,
}
impl TitleLayout {
    pub fn new(title: BoundedTitleOptions, scale: f32, radial: bool) -> Self {
        let m = measure_rich_text(&scaled_rich_text(&title.text, scale), FontPolicy::Standard);
        let metrics = TextExtents {
            width: m.width / scale,
            ascent: m.ascent / scale,
            descent: m.descent / scale,
        };
        let baseline = (if radial {
            42.
        } else {
            title.text.font_size + 12.
        })
        .max(metrics.ascent + 12.);
        let top = if !title.has_text() {
            if radial { 24. } else { 32. }
        } else {
            (if radial {
                70.
            } else {
                title.text.font_size + 42.
            })
            .max(baseline + metrics.descent + 20.)
        };
        Self {
            title,
            metrics,
            baseline,
            top,
        }
    }
    pub fn draw(
        &self,
        painter: &mut DecorationPainter<'_>,
        canvas: &mut Canvas,
        width: f32,
    ) -> Result<(), &'static str> {
        if !self.title.has_text() {
            return Ok(());
        }
        if self.metrics.width > width - 32. {
            return Err("title does not fit; increase chart width or shorten it");
        }
        // The extents measured for the layout are the placed extents: the
        // title is measured once.
        painter.on(DecorationId::ChartTitle, canvas).text(
            Slot::Above,
            &PlacedText {
                text: TextRef::Rich(&self.title.text),
                origin: ((width - self.metrics.width) * 0.5, self.baseline),
                extents: self.metrics,
                rotation: None,
            },
        );
        Ok(())
    }
}

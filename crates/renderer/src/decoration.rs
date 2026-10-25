#![doc = include_str!("../INTERACTION.md")]
//! Decoration hit maps are the common `HitMap` of the chart's state: the
//! Cartesian `HitMap<Config>` and the radial frame's `HitMap<RadialChart>`.
//! Data picking remains on the chart frame. Coordinates and drag deltas are
//! chart-local logical pixels, not framebuffer pixels.

pub(crate) fn scaled_rich_text(text: &model::text::RichText, scale: f32) -> model::text::RichText {
    let mut text = text.clone();
    text.font_size *= scale;
    for segment in &mut text.segments {
        if let Some(size) = &mut segment.font_size {
            *size *= scale;
        }
    }
    text
}

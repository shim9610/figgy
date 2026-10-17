//! Small egui helpers shared by the scene panels.

use renderer::Color;
use renderer::line::LineStylePreset;
use renderer::text::{RichText, rich_segments_from_text};

pub fn choice<T: Clone + PartialEq + std::fmt::Debug>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    options: &[T],
) -> bool {
    let before = value.clone();
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .selected_text(format!("{value:?}"))
            .show_ui(ui, |ui| {
                for option in options {
                    ui.selectable_value(value, option.clone(), format!("{option:?}"));
                }
            });
    });
    *value != before
}

pub fn color(ui: &mut egui::Ui, label: &str, c: &mut Color) -> bool {
    let mut rgba = [
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        (c.a * 255.0).round() as u8,
    ];
    let changed = ui
        .horizontal(|ui| {
            let changed = ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed();
            ui.label(label);
            changed
        })
        .inner;
    if changed {
        *c = Color::from_rgba(
            rgba[0] as f32 / 255.0,
            rgba[1] as f32 / 255.0,
            rgba[2] as f32 / 255.0,
            rgba[3] as f32 / 255.0,
        );
    }
    changed
}

pub fn slider(
    ui: &mut egui::Ui,
    label: &str,
    v: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    ui.add(egui::Slider::new(v, range).text(label)).changed()
}

/// Edit the plain text of a rich text, keeping its colour, size and font.
pub fn rich_text(ui: &mut egui::Ui, label: &str, text: &mut RichText) -> bool {
    let mut plain: String = text.segments.iter().map(|s| s.text).collect();
    let changed = ui
        .horizontal(|ui| {
            ui.label(label);
            ui.text_edit_singleline(&mut plain).changed()
        })
        .inner;
    if changed {
        text.segments = rich_segments_from_text(&plain);
    }
    changed
}

pub const LINE_STYLES: &[LineStylePreset] = &[
    LineStylePreset::Solid,
    LineStylePreset::Dash,
    LineStylePreset::Dot,
    LineStylePreset::DashDot,
    LineStylePreset::DashDotDot,
];

/// Collapsible section that starts open or closed.
pub fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(title)
        .default_open(open)
        .show(ui, body);
}

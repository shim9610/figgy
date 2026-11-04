//! Grotrian diagrams of four atoms from NIST data (#205), saved as PNG at
//! 1x and 2x. The diagrams are built in `support/grotrian_sets.rs`, which
//! the playground shows as scenes too. Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example nist_grotrian_gallery -- target/nist_grotrian_gallery

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/grotrian_sets.rs"]
mod grotrian_sets;
#[allow(dead_code)]
#[path = "support/nist_csv.rs"]
mod nist_csv;
use gallery::{Result, output_directory, with_renderer};
use renderer::{Chart, Color, Renderer, SeriesConfig, encode_png};
use std::path::Path;

fn save(
    r: &mut Renderer,
    (c, series): (Chart, Vec<SeriesConfig>),
    out: &Path,
    name: &str,
) -> Result {
    // The host fits the axis bands to their labels and titles.
    let measure = renderer::CpuTextMeasure::for_style(&c.config().draw_style);
    let c = &Chart::new(renderer::layout::auto_layout(c.config(), &measure));
    let series = &series[..];
    for scale in [1.0, 2.0] {
        let image = r.export_rgba_blocking(
            renderer::ExportSource::chart(c, series),
            renderer::ExportOptions::document(scale).with_clear(Color::WHITE),
        )?;
        let path = out.join(format!("{name}-{scale}x.png"));
        std::fs::write(&path, encode_png(&image)?)?;
        println!("{}: {}x{}", path.display(), image.width, image.height);
    }
    Ok(())
}

fn main() -> Result {
    let out = output_directory("target/nist_grotrian_gallery")?;
    with_renderer(|r| {
        save(r, grotrian_sets::helium()?, &out, "he-i")?;
        save(r, grotrian_sets::argon()?, &out, "ar-i")?;
        save(r, grotrian_sets::argon_all()?, &out, "ar-i-all")?;
        save(r, grotrian_sets::copper()?, &out, "cu-i")?;
        save(r, grotrian_sets::uranium()?, &out, "u-i")
    })
}

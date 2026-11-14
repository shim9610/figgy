//! The SVG and PDF export (#48): decorations as vector content from the one
//! emit, the data layer as an image of the PNG export's own pixels.
//!
//! - Every catalogue scene at 1x and 2x, in both formats: the file's
//!   elements, read back in order, are the parts the export was handed (a
//!   `Parts` recording of the same passes, `renderer::written_document_parts`)
//!   within 0.01 px: strokes, rectangles, triangles, each text block's origin
//!   and rotation, each glyph pen and rule, each image's pixels. The PDF's
//!   header, cross-reference table, trailer, object graph, ExtGStates and
//!   image soft masks are checked, and its images are the SVG's pixels.
//! - With every decoration hidden, each embedded data image is the PNG
//!   export's crop, byte for byte, the DataLess series aside: they are
//!   vector parts between the images of the GPU data under them and the
//!   value labels over them (#234). With decorations shown, the CPU composite
//!   of the grid raster, the data image and the decoration raster is the
//!   PNG export within 1/255 on all but 0.05 % of the channels (8-bit
//!   blend rounding, at most 3/255).
//! - A document with a DataLess series is checked through its file's own
//!   picture: `support/svg_raster.rs` paints the SVG's images, paths and
//!   glyphs in order, independently of the renderer, and that picture is
//!   the PNG export within a pixel or two at edges (antialiasing, unhinted
//!   glyph outlines, SVG's butt caps) and within 8/255 inside each paint.
//! - Size budgets, determinism, the clear colour, a streamed source's
//!   rejection, the sketch and Milkyway styles and the radial chart.
#![cfg(not(target_arch = "wasm32"))]

use test_gpu::BlockOn as _;
#[path = "support/pdf_structure.rs"]
mod pdf_structure;
#[path = "support/playground_scenes.rs"]
mod scenes;
#[path = "support/svg_parts.rs"]
mod svg_parts;
#[path = "support/svg_raster.rs"]
mod svg_raster;
#[path = "support/gpu.rs"]
mod test_gpu;

use renderer::axis_render::{AxisLayerKind, try_raster_chart_layer_to_rgba_with_selection_at};
use renderer::config::{AxisOptions, DrawStyle, SketchOptions, TickVisibility};
use renderer::layout::Rect;
use renderer::radial::{
    RadialChart, RadialKind, RadialRenderer, RadialSlice, RadialSplit, written_vector_parts,
};
use renderer::{
    Anchor, Annotation, AnnotationKind, Chart, Color, Config, DecorationId, FiggyError,
    RasterImage, Renderer, RendererDevice, SeriesConfig, WrittenPart, encode_png, place_document,
    written_document_parts,
};
use scenes::SceneKind;
use svg_parts::{ReadPart, assert_parts_match};

const SCALES: [f32; 2] = [1.0, 2.0];
/// The large-data scene's points, as the decoration oracle builds it.
const BIG_DATA_POINTS: usize = 100_000;
const POOL_BYTES: u64 = 64 * 1024 * 1024;
/// Bytes outside the images, per file.
const SVG_BUDGET: usize = 256 * 1024;
const PDF_BUDGET: usize = 128 * 1024;
/// The images of a file against the PNG export's size.
const IMAGE_BUDGET: f64 = 1.05;
/// Read-back tolerance in pixels: the writers round to 0.001.
const TOLERANCE: f32 = 0.01;

fn gpu() -> RendererDevice {
    let (device, queue) = test_gpu::device();
    RendererDevice::new(device, queue).expect("figgy poll thread")
}

fn renderer(gpu: &RendererDevice) -> Renderer {
    Renderer::try_new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm, POOL_BYTES).expect("renderer")
}

fn cartesian_scenes() -> impl Iterator<Item = SceneKind> {
    SceneKind::ALL.into_iter().filter(|k| !k.is_radial())
}

fn decode_png(bytes: &[u8]) -> RasterImage {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().expect("a PNG");
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgba).expect("PNG pixels");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    RasterImage {
        width: info.width,
        height: info.height,
        rgba,
    }
}

fn crop(image: &RasterImage, r: Rect) -> RasterImage {
    let mut rgba = Vec::with_capacity(r.width as usize * r.height as usize * 4);
    for y in r.y..r.y + r.height {
        let start = ((y * image.width + r.x) * 4) as usize;
        rgba.extend_from_slice(&image.rgba[start..start + r.width as usize * 4]);
    }
    RasterImage {
        width: r.width,
        height: r.height,
        rgba,
    }
}

fn overlap(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// Data images written over one another (a nested subplot's group over its
/// parent's) composite to the export: at every pixel the export is the
/// topmost image with ink there, exactly, where that image is opaque or
/// nothing under it has ink; only anti-aliased edges over ink are skipped,
/// and they are few.
fn assert_composite(export: &RasterImage, images: &[(Rect, Vec<u8>)], context: &str) {
    let decoded: Vec<(Rect, RasterImage)> = images
        .iter()
        .map(|(rect, bytes)| (*rect, decode_png(bytes)))
        .collect();
    let (mut checked, mut blended) = (0usize, 0usize);
    for y in 0..export.height {
        for x in 0..export.width {
            let stack: Vec<&[u8]> = decoded
                .iter()
                .filter(|(rect, _)| {
                    x >= rect.x
                        && y >= rect.y
                        && x < rect.x + rect.width
                        && y < rect.y + rect.height
                })
                .map(|(rect, image)| {
                    let i = (((y - rect.y) * image.width + (x - rect.x)) * 4) as usize;
                    &image.rgba[i..i + 4]
                })
                .collect();
            let Some(top) = stack
                .iter()
                .rposition(|px| px[3] != 0)
                .or(stack.len().checked_sub(1))
            else {
                continue;
            };
            let under = stack[..top].iter().any(|px| px[3] != 0);
            if stack[top][3] != 255 && under {
                blended += 1;
                continue;
            }
            let i = ((y * export.width + x) * 4) as usize;
            assert_eq!(
                &export.rgba[i..i + 4],
                stack[top],
                "{context}: the export at ({x}, {y}) is not its topmost data image"
            );
            checked += 1;
        }
    }
    assert!(
        blended * 1000 < checked.max(1),
        "{context}: {blended} blended pixels of {checked}"
    );
}

/// What one document's files measure.
struct Sizes {
    svg_other: usize,
    pdf_other: usize,
    svg_images: usize,
    pdf_images: usize,
    png: usize,
}

/// Export `chart` as SVG and PDF at `scale` and check both against the
/// parts the export was handed, the PDF's structure, the images' agreement
/// and the page size.
fn check_document(
    r: &mut Renderer,
    context: &str,
    chart: &Chart,
    series: &[SeriesConfig],
    monotonic: &[scenes::MonotonicData],
    scale: f32,
    amplitude: f32,
) -> Sizes {
    let options = renderer::ExportOptions::document(scale);
    let svg = scenes::export_scene(r, chart, series, monotonic, |r, source| {
        r.export_svg(source, options).block_on()
    })
    .unwrap();
    let pdf = scenes::export_scene(r, chart, series, monotonic, |r, source| {
        r.export_pdf(source, options).block_on()
    })
    .unwrap();
    let image = scenes::export_scene(r, chart, series, monotonic, |r, source| {
        r.export_rgba(source, options).block_on()
    })
    .unwrap();
    let png = encode_png(&image).unwrap();
    let mut written = Vec::new();
    written_document_parts(chart, series, scale, &mut |part| written.push(part)).unwrap();

    let logical = chart.config().document_area();
    let page = [logical.width as f32 * 0.75, logical.height as f32 * 0.75];
    let file = svg_parts::read(&svg);
    assert_eq!(file.size, (image.width, image.height), "{context}: viewBox");
    assert_eq!(file.page, page, "{context}: SVG size in points");
    assert_parts_match(
        &file.parts,
        &written,
        TOLERANCE,
        amplitude,
        &format!("{context} SVG"),
    );
    assert!(
        file.glyph_defs <= file.glyph_uses,
        "{context}: glyphs defined once"
    );

    let pdf_file = pdf_structure::read(&pdf);
    assert_eq!(pdf_file.page, page, "{context}: MediaBox");
    let parts = pdf_structure::content_parts(&pdf_file, file.size);
    assert_parts_match(
        &parts,
        &written,
        TOLERANCE,
        amplitude,
        &format!("{context} PDF"),
    );

    // The PDF's images are the SVG's pixels, in drawing order.
    assert_eq!(
        pdf_file.images.len(),
        file.images.len(),
        "{context}: images"
    );
    for (k, (rect, bytes)) in file.images.iter().enumerate() {
        let svg_image = decode_png(bytes);
        assert_eq!(
            (svg_image.width, svg_image.height),
            (rect.width, rect.height)
        );
        let pdf_image = &pdf_file.images[&format!("Im{}", k + 1)];
        assert_eq!(
            (pdf_image.width, pdf_image.height),
            (rect.width, rect.height)
        );
        assert!(
            pdf_image.rgba == svg_image.rgba,
            "{context}: image {k} differs between the SVG and the PDF"
        );
    }
    Sizes {
        svg_other: file.other_bytes,
        pdf_other: pdf_file.other_bytes,
        svg_images: file.images.iter().map(|(_, b)| b.len()).sum(),
        pdf_images: pdf.len() - pdf_file.other_bytes,
        png: png.len(),
    }
}

// Every catalogue scene, both formats, both scales: the read-back, the
// structure and the size budgets.
#[test]
fn every_catalogue_scene_reads_back_as_its_emitted_parts_within_budget() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    let mut report = String::new();
    for kind in cartesian_scenes() {
        let scene = scenes::build(&mut r, kind, BIG_DATA_POINTS).unwrap();
        let chart = Chart::new(scene.config.clone());
        for scale in SCALES {
            let context = format!("{kind:?} at {scale}x");
            let sizes = check_document(
                &mut r,
                &context,
                &chart,
                &scene.series,
                &scene.monotonic,
                scale,
                0.0,
            );
            report += &format!(
                "{context}: SVG {} KiB + images {} KiB, PDF {} KiB + images {} KiB, PNG {} KiB\n",
                sizes.svg_other / 1024,
                sizes.svg_images / 1024,
                sizes.pdf_other / 1024,
                sizes.pdf_images / 1024,
                sizes.png / 1024
            );
            assert!(
                sizes.svg_other <= SVG_BUDGET,
                "{context}: SVG {}",
                sizes.svg_other
            );
            assert!(
                sizes.pdf_other <= PDF_BUDGET,
                "{context}: PDF {}",
                sizes.pdf_other
            );
            for (format, images) in [("SVG", sizes.svg_images), ("PDF", sizes.pdf_images)] {
                assert!(
                    images as f64 <= IMAGE_BUDGET * sizes.png as f64,
                    "{context}: {format} images {images} bytes, PNG {}",
                    sizes.png
                );
            }
        }
    }
    eprintln!("{report}");
}

// The sketch stroker's curves and the Milkyway backdrop: a sketch stroke
// reads back as a curve from near its segment's ends, with the sketch
// width floor; the backdrop is an image of the panel under its chrome.
#[test]
fn the_sketch_and_milkyway_styles_read_back() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    let scene = scenes::build(&mut r, SceneKind::Lines, BIG_DATA_POINTS).unwrap();
    let mut sketch = scene.config.clone();
    sketch.draw_style = DrawStyle::Sketch(SketchOptions {
        amplitude_px: 1.5,
        wavelength_px: 60.0,
        seed: 7,
    });
    for scale in SCALES {
        let chart = Chart::new(sketch.clone());
        check_document(
            &mut r,
            &format!("sketch Lines at {scale}x"),
            &chart,
            &scene.series,
            &scene.monotonic,
            scale,
            1.5 * scale,
        );
    }
    let mut milkyway = scene.config.clone();
    milkyway.draw_style = DrawStyle::Milkyway(Default::default());
    let chart = Chart::new(milkyway);
    check_document(
        &mut r,
        "Milkyway Lines",
        &chart,
        &scene.series,
        &scene.monotonic,
        1.0,
        0.0,
    );
    let svg = svg_parts::read(
        &r.export_svg(
            renderer::ExportSource::chart(&chart, &scene.series),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
        .unwrap(),
    );
    let ReadPart::Image(backdrop) = svg.parts[0] else {
        panic!("the backdrop comes first: {:?}", svg.parts[0]);
    };
    assert_eq!(
        (backdrop.width, backdrop.height),
        (svg.size.0, svg.size.1),
        "the backdrop covers the panel"
    );
}

fn hide_axis(axis: &mut AxisOptions) {
    axis.line_visible = false;
    axis.tick = TickVisibility::None;
    axis.label_style.visible = false;
    axis.label_style.label_visible = false;
    axis.title_option.visible = false;
}

/// `config` with every decoration hidden: titles, legend, grid, axes,
/// colourbar and annotations, in every overlay and subplot.
fn hide_decorations(config: &mut Config) {
    config.chart_title.visible = false;
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        hide_axis(axis);
    }
    if let Some(bar) = config.colorbar.as_mut() {
        bar.visible = false;
    }
    config.annotations.visible = false;
    // The plot fill is chrome (the slot below the grid).
    config.plot_background = None;
    config.background = None;
    for overlay in &mut config.overlays {
        hide_axis(&mut overlay.y_axis);
        hide_axis(&mut overlay.x_axis);
    }
    for subplot in &mut config.subplots {
        // A subplot has no annotation layer of its own to hide.
        let annotations = subplot.config.annotations.clone();
        hide_decorations(&mut subplot.config);
        subplot.config.annotations = annotations;
    }
}

// With every decoration hidden, the PNG export draws the data only, and
// each embedded data image is its crop at the image's place, byte for byte
// as a PNG; nothing of the export lies outside the images. The DataLess
// series are vector parts, not image pixels (#234): the images are the
// export of the other series.
#[test]
fn the_data_images_are_the_png_exports_pixels() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    for kind in cartesian_scenes() {
        let scene = scenes::build(&mut r, kind, BIG_DATA_POINTS).unwrap();
        let mut config = scene.config.clone();
        hide_decorations(&mut config);
        let chart = Chart::new(config);
        let gpu_series: Vec<SeriesConfig> = scene
            .series
            .iter()
            .filter(|series| !series.is_dataless())
            .cloned()
            .collect();
        for scale in SCALES {
            let context = format!("{kind:?} at {scale}x");
            let options = renderer::ExportOptions::document(scale);
            let export = scenes::export_scene(
                &mut r,
                &chart,
                &gpu_series,
                &scene.monotonic,
                |r, source| r.export_rgba(source, options).block_on(),
            )
            .unwrap();
            let text = scenes::export_scene(
                &mut r,
                &chart,
                &scene.series,
                &scene.monotonic,
                |r, source| r.export_svg(source, options).block_on(),
            )
            .unwrap();
            let svg = svg_parts::read(&text);
            assert_eq!(
                svg.images.is_empty(),
                gpu_series.is_empty(),
                "{context}: a data image for the GPU series"
            );
            let mut covered = vec![false; (export.width * export.height) as usize];
            let nested = svg
                .images
                .iter()
                .enumerate()
                .any(|(i, (a, _))| svg.images[i + 1..].iter().any(|(b, _)| overlap(*a, *b)));
            if nested {
                // An inset (#203): its group's image is written over its
                // parent's, so the export is their composite.
                assert_composite(&export, &svg.images, &context);
            }
            for (rect, bytes) in &svg.images {
                assert!(
                    nested || *bytes == encode_png(&crop(&export, *rect)).unwrap(),
                    "{context}: the data image at {rect:?} is not the export's crop"
                );
                for y in rect.y..rect.y + rect.height {
                    for x in rect.x..rect.x + rect.width {
                        covered[(y * export.width + x) as usize] = true;
                    }
                }
            }
            let outside = export
                .rgba
                .chunks_exact(4)
                .zip(&covered)
                .position(|(px, covered)| !covered && px[3] != 0);
            assert!(
                outside.is_none(),
                "{context}: the export paints pixel {:?} outside the data images",
                outside.map(|i| (i as u32 % export.width, i as u32 / export.width))
            );
            // The images leave the CPU layer out; the file's own picture,
            // its images and its DataLess vector parts rasterized in order
            // (`svg_raster`), is the PNG export of the whole document.
            if scene.series.iter().any(SeriesConfig::is_dataless) {
                let full = scenes::export_scene(
                    &mut r,
                    &chart,
                    &scene.series,
                    &scene.monotonic,
                    |r, source| r.export_rgba(source, options).block_on(),
                )
                .unwrap();
                svg_raster::assert_same_picture(
                    &full,
                    &svg_raster::rasterize(&text),
                    reach(scale),
                    8,
                    &format!("{context}: the file's own picture"),
                );
            }
        }
    }
}

/// How far an independent rasterizer's edges may lie from the PNG's:
/// antialiasing, glyph outlines against glyph rasters and SVG's butt caps
/// move an edge by about a pixel at 1x.
fn reach(scale: f32) -> u32 {
    scale.ceil() as u32 + 1
}

/// One energy level in the box plot scene's `Mid` column at 40, drawn
/// 7 px wide: the only stroke of that width in its file.
fn mid_level() -> SeriesConfig {
    use renderer::energy_levels::{EnergyLevel, EnergyLevelSeries, LevelRole};
    let mut levels = EnergyLevelSeries {
        levels: vec![EnergyLevel {
            id: "mid".into(),
            column: "Mid".into(),
            configuration: String::new(),
            term: String::new(),
            j: String::new(),
            g: None,
            prefix: String::new(),
            energy: 40.0,
            suffix: String::new(),
            uncertainty: None,
            reference: String::new(),
            role: LevelRole::Level,
            label: None,
            label_offset: [0.0; 2],
        }],
        ..EnergyLevelSeries::default()
    };
    levels.style.level.color = Color::from_rgb8(255, 0, 255);
    levels.style.level.width = LEVEL_WIDTH;
    SeriesConfig {
        series_id: "levels".into(),
        source_id: None,
        label: None,
        x_column: String::new(),
        y_column: String::new(),
        z_column: None,
        render_type: renderer::DataRenderType::EnergyLevels(levels),
    }
}

const LEVEL_WIDTH: f32 = 7.0;

// Beside GPU value labels (#234, D11), a group's DataLess series is written
// where the frame draws it: the box plots as one image, the energy level
// as a vector stroke, then the median labels as a second image. The first
// image is the export of the boxes without labels, byte for byte, and the
// two images composite to the export of the labelled boxes.
#[test]
fn a_dataless_series_is_written_between_the_gpu_data_and_the_value_labels() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    let scene = scenes::build(&mut r, SceneKind::BoxPlot, BIG_DATA_POINTS).unwrap();
    let mut config = scene.config.clone();
    hide_decorations(&mut config);
    let chart = Chart::new(config);
    let mut with_level = scene.series.clone();
    with_level.push(mid_level());
    let mut unlabelled = scene.series.clone();
    for series in &mut unlabelled {
        if let renderer::DataRenderType::BoxPlot { labels, .. } = &mut series.render_type {
            labels.placement = renderer::BoxPlotLabels::None;
        }
    }
    let export = |r: &mut Renderer, series: &[SeriesConfig], scale: f32| {
        r.export_rgba(
            renderer::ExportSource::chart(&chart, series),
            renderer::ExportOptions::document(scale),
        )
        .block_on()
        .unwrap()
    };
    for scale in SCALES {
        let context = format!("box plots with a level at {scale}x");
        let text = r
            .export_svg(
                renderer::ExportSource::chart(&chart, &with_level),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        assert!(text.contains("stroke=\"#ff00ff\""), "{context}: the level");
        let svg = svg_parts::read(&text);
        let images: Vec<usize> = svg
            .parts
            .iter()
            .enumerate()
            .filter(|(_, part)| matches!(part, ReadPart::Image(_)))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(images.len(), 2, "{context}: the body and label images");
        let level = svg
            .parts
            .iter()
            .position(|part| {
                matches!(part, ReadPart::Part(renderer::DrawnPart::Segment { .. }, Some(w))
                    if (w - LEVEL_WIDTH * scale).abs() < TOLERANCE)
            })
            .expect("the level's stroke");
        assert!(
            images[0] < level && level < images[1],
            "{context}: image {}, level {level}, image {}",
            images[0],
            images[1]
        );
        let body = export(&mut r, &unlabelled, scale);
        let (rect, bytes) = &svg.images[0];
        assert!(
            *bytes == encode_png(&crop(&body, *rect)).unwrap(),
            "{context}: the first image is the boxes without their labels"
        );
        let labels = decode_png(&svg.images[1].1);
        assert!(
            labels.rgba.chunks_exact(4).any(|px| px[3] != 0),
            "{context}: the second image holds the labels"
        );
        let labelled = export(&mut r, &scene.series, scale);
        assert_composite(&labelled, &svg.images, &context);
        // The file stacks as the PNG does: the body image, the level's
        // vector parts, then the label image, rasterized in that order
        // (`svg_raster`), are the export of the whole document.
        let full = export(&mut r, &with_level, scale);
        svg_raster::assert_same_picture(
            &full,
            &svg_raster::rasterize(&text),
            reach(scale),
            8,
            &format!("{context}: body, level, labels"),
        );

        // Without value labels the group writes one image, under the level:
        // no image of an empty label step (§3.4 of the #238 audit).
        let mut unlabelled_level = unlabelled.clone();
        unlabelled_level.push(mid_level());
        let svg = svg_parts::read(
            &r.export_svg(
                renderer::ExportSource::chart(&chart, &unlabelled_level),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap(),
        );
        let images: Vec<usize> = svg
            .parts
            .iter()
            .enumerate()
            .filter(|(_, part)| matches!(part, ReadPart::Image(_)))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(images.len(), 1, "{context} unlabelled: one image");
        let level_part = svg
            .parts
            .iter()
            .position(|part| {
                matches!(part, ReadPart::Part(renderer::DrawnPart::Segment { .. }, Some(w))
                    if (w - LEVEL_WIDTH * scale).abs() < TOLERANCE)
            })
            .expect("the level's stroke");
        assert!(
            images[0] < level_part,
            "{context} unlabelled: the image under the level"
        );
    }
}

// With decorations shown, the grid raster, the data image and the
// decoration raster composite to the PNG export: the data image is the
// data slot between the two chrome slots, up to the 8-bit rounding of
// blending it once instead of primitive by primitive.
#[test]
fn the_chrome_rasters_and_the_data_image_composite_to_the_png_export() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    let mut checked = 0;
    for kind in cartesian_scenes() {
        let scene = scenes::build(&mut r, kind, BIG_DATA_POINTS).unwrap();
        if !scene.config.overlays.is_empty() || !scene.config.subplots.is_empty() {
            // One panel: its two chrome rasters are the public layer rasters.
            continue;
        }
        let mut config = scene.config.clone();
        config.annotations.visible = false;
        let chart = Chart::new(config.clone());
        for scale in SCALES {
            let context = format!("{kind:?} at {scale}x");
            let options = renderer::ExportOptions::document(scale);
            let export = scenes::export_scene(
                &mut r,
                &chart,
                &scene.series,
                &scene.monotonic,
                |r, source| r.export_rgba(source, options).block_on(),
            )
            .unwrap();
            let target = Rect {
                x: 0,
                y: 0,
                width: export.width,
                height: export.height,
            };
            let placed = place_document(&config, scale, target);
            assert_eq!(
                placed.chart_area.0, target,
                "{context}: one panel covers the target"
            );
            // The rasters at the export's scale: a broken axis' piece frames
            // snap on the logical grid the export used (#205).
            let below = try_raster_chart_layer_to_rgba_with_selection_at(
                &placed,
                AxisLayerKind::Grid,
                &[],
                scale,
            )
            .unwrap();
            let above = try_raster_chart_layer_to_rgba_with_selection_at(
                &placed,
                AxisLayerKind::Decoration,
                &[],
                scale,
            )
            .unwrap();
            let text = scenes::export_scene(
                &mut r,
                &chart,
                &scene.series,
                &scene.monotonic,
                |r, source| r.export_svg(source, options).block_on(),
            )
            .unwrap();
            // A DataLess series is vector parts between the body image and
            // the value-label image (D11), so its file is checked as a
            // whole: rasterized element by element (`svg_raster`), chrome
            // and data, it is the PNG export.
            if scene.series.iter().any(SeriesConfig::is_dataless) {
                svg_raster::assert_same_picture(
                    &export,
                    &svg_raster::rasterize(&text),
                    reach(scale),
                    8,
                    &format!("{context}: the file's own picture"),
                );
                checked += 1;
                continue;
            }
            let svg = svg_parts::read(&text);
            let data: Vec<(Rect, RasterImage)> = svg
                .parts
                .iter()
                .filter_map(|p| match p {
                    ReadPart::Image(rect) => Some(*rect),
                    _ => None,
                })
                .zip(&svg.images)
                .filter(|(rect, _)| {
                    // The data image covers the data area; a ramp is part of
                    // the decoration raster.
                    let area = placed.data_area().unwrap().0;
                    rect.x <= area.x
                        && rect.y <= area.y
                        && rect.x + rect.width >= area.x + area.width
                        && rect.y + rect.height >= area.y + area.height
                })
                .map(|(rect, (_, bytes))| (rect, decode_png(bytes)))
                .collect();
            assert_eq!(data.len(), 1, "{context}: one data image");
            let layers = data;
            // In premultiplied units of 1/255, without rounding between the
            // layers: the GPU rounds each blend to its 8-bit target.
            let straight = |px: &[u8]| {
                let a = f32::from(px[3]);
                [
                    f32::from(px[0]) * a / 255.0,
                    f32::from(px[1]) * a / 255.0,
                    f32::from(px[2]) * a / 255.0,
                    a,
                ]
            };
            let over = |dst: &mut [f32; 4], src: [f32; 4]| {
                for k in 0..4 {
                    dst[k] = src[k] + dst[k] * (1.0 - src[3] / 255.0);
                }
            };
            let mut worst = 0.0f32;
            let mut over_one = 0usize;
            for y in 0..export.height {
                for x in 0..export.width {
                    let i = ((y * export.width + x) * 4) as usize;
                    let b = &below[i..i + 4];
                    let mut px = [b[0], b[1], b[2], b[3]].map(f32::from);
                    for (rect, image) in &layers {
                        if x >= rect.x
                            && y >= rect.y
                            && x < rect.x + rect.width
                            && y < rect.y + rect.height
                        {
                            let j = (((y - rect.y) * rect.width + x - rect.x) * 4) as usize;
                            over(&mut px, straight(&image.rgba[j..j + 4]));
                        }
                    }
                    let a = &above[i..i + 4];
                    over(&mut px, [a[0], a[1], a[2], a[3]].map(f32::from));
                    let want = straight(&export.rgba[i..i + 4]);
                    for k in 0..4 {
                        let d = (px[k] - want[k]).abs();
                        worst = worst.max(d);
                        over_one += usize::from(d > 1.0);
                    }
                }
            }
            // The PNG blends each data primitive onto the grid in the 8-bit
            // target; the file blends the data image, rounded once on its
            // own, over the grid. That rounding is all that may differ:
            // within 1/255 but for a few channels, none past 4/255. Where
            // many translucent strokes overlap (BigData) the per-blend
            // rounding adds up: within 3/255 on lavapipe, 3.67/255 on WARP
            // and 3.77/255 on Metal, whose 8-bit blends need not round to
            // nearest (#63).
            let channels = (export.width * export.height * 4) as usize;
            assert!(worst <= 4.0, "{context}: composite differs by {worst}/255");
            assert!(
                over_one * 2000 <= channels,
                "{context}: {over_one} of {channels} channels differ by more than 1/255"
            );
            checked += 1;
        }
    }
    assert!(checked >= 20, "{checked} single-panel cases");
}

// The same document writes the same bytes twice, in both formats.
#[test]
fn the_same_document_writes_the_same_bytes() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    for kind in [
        SceneKind::Subplots,
        SceneKind::Annotations,
        SceneKind::Field,
    ] {
        let scene = scenes::build(&mut r, kind, BIG_DATA_POINTS).unwrap();
        let chart = Chart::new(scene.config.clone());
        for scale in SCALES {
            let svg = r
                .export_svg(
                    renderer::ExportSource::chart(&chart, &scene.series),
                    renderer::ExportOptions::document(scale),
                )
                .block_on()
                .unwrap();
            assert_eq!(
                svg,
                r.export_svg(
                    renderer::ExportSource::chart(&chart, &scene.series),
                    renderer::ExportOptions::document(scale)
                )
                .block_on()
                .unwrap(),
                "{kind:?}"
            );
            let pdf = r
                .export_pdf(
                    renderer::ExportSource::chart(&chart, &scene.series),
                    renderer::ExportOptions::document(scale),
                )
                .block_on()
                .unwrap();
            assert!(
                pdf == r
                    .export_pdf(
                        renderer::ExportSource::chart(&chart, &scene.series),
                        renderer::ExportOptions::document(scale)
                    )
                    .block_on()
                    .unwrap(),
                "{kind:?}"
            );
        }
    }
}

// The clear colour is a rectangle under everything, and the rest of the
// file is unchanged.
#[test]
fn the_clear_colour_is_a_background_rectangle() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    let scene = scenes::build(&mut r, SceneKind::Annotations, BIG_DATA_POINTS).unwrap();
    let chart = Chart::new(scene.config.clone());
    let clear = Color::new(0.1, 0.2, 0.3, 1.0);
    let plain = svg_parts::read(
        &r.export_svg(
            renderer::ExportSource::chart(&chart, &scene.series),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
        .unwrap(),
    );
    let svg = r.export_svg(
        renderer::ExportSource::chart(&chart, &scene.series),
        renderer::ExportOptions::document(1.0).with_clear(clear),
    );
    let svg = test_gpu::block_on(svg).unwrap();
    assert!(
        svg.contains("<path fill=\"#1a334d\" d=\"M0 0L"),
        "the background's colour"
    );
    let cleared = svg_parts::read(&svg);
    let full = renderer::layout::RectF {
        x: 0.0,
        y: 0.0,
        width: plain.size.0 as f32,
        height: plain.size.1 as f32,
    };
    assert_eq!(
        cleared.parts[0],
        ReadPart::Part(renderer::DrawnPart::Rect(full), None)
    );
    assert_eq!(&cleared.parts[1..], &plain.parts[..]);
    let pdf = test_gpu::block_on(r.export_pdf(
        renderer::ExportSource::chart(&chart, &scene.series),
        renderer::ExportOptions::document(1.0).with_clear(clear),
    ))
    .unwrap();
    let file = pdf_structure::read(&pdf);
    let parts = pdf_structure::content_parts(&file, plain.size);
    assert_eq!(
        parts[0],
        ReadPart::Part(renderer::DrawnPart::Rect(full), None)
    );
}

// A series with a streamed source is rejected, as by the PNG export.
#[test]
fn a_streamed_source_is_rejected() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    let streamed = |id: &str| renderer::StreamColumn {
        id: id.into(),
        len: 3,
        revision: 1,
        encoding: renderer::StreamEncoding::ScalarF32,
        replay: renderer::StreamReplay::RandomAccess,
        statistics: renderer::StreamStatistics::Unknown,
    };
    r.register_streamed_columns(vec![streamed("st_x"), streamed("st_y")])
        .unwrap();
    let scene = scenes::build(&mut r, SceneKind::Lines, BIG_DATA_POINTS).unwrap();
    let mut series = scene.series.clone();
    series.push(scenes::series(
        "streamed",
        "st_x",
        "st_y",
        renderer::data_config::DataRenderType::Xy(renderer::data_config::XySeries {
            scatter: None,
            line: Some(scenes::line_style(Color::BLACK, 1.0)),
            errors: None,
            stem: None,
        }),
    ));
    let chart = Chart::new(scene.config.clone());
    let rejected = |result: Result<(), FiggyError>| {
        assert!(
            matches!(&result, Err(FiggyError::ColumnNotResident { id }) if id == "st_x"),
            "{:?}",
            result
        );
    };
    rejected(
        r.export_svg(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
        .map(|_| ()),
    );
    rejected(
        r.export_pdf(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
        .map(|_| ()),
    );
    rejected(
        r.export_png(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
        .map(|_| ()),
    );
}

fn radial_cases() -> Vec<(&'static str, RadialChart)> {
    let plain = scenes::radial::radial_chart();
    let mut split = scenes::radial::radial_chart();
    split.split = Some(RadialSplit {
        slice_index: 1,
        children: vec![
            RadialSlice::new("A", 15.0, Color::from_rgb8(200, 40, 40)),
            RadialSlice::new("B", 10.0, Color::from_rgb8(40, 200, 40)),
        ],
        kind: RadialKind::Pie,
    });
    split.format_labels();
    let mut annotated = scenes::radial::radial_chart();
    annotated
        .annotations
        .items
        .push(Annotation::new(AnnotationKind::Text {
            anchor: Anchor::Ratio { x: 0.1, y: 0.9 },
            text: renderer::text::RichText::plain("note", Color::BLACK, 14.0, ""),
            box_style: None,
            align: Default::default(),
        }));
    annotated.decorations.selected = Some(DecorationId::ChartTitle);
    vec![
        ("radial", plain),
        ("split", split),
        ("annotated, title selected", annotated),
        ("sunburst", scenes::radial::sunburst_chart()),
    ]
}

// The radial chart: one image of the frame, then its title, labels,
// leader lines, split connectors and annotations, read back in both
// formats at both scales; no selection box is drawn.
#[test]
fn the_radial_chart_reads_back_as_its_emitted_parts() {
    let gpu = gpu();
    let mut rr = RadialRenderer::try_new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let size = (800, 520);
    let selection = {
        let c = renderer::select::SELECTION_COLOR;
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
    };
    for (name, chart) in radial_cases() {
        for scale in SCALES {
            let context = format!("{name} at {scale}x");
            let mut written: Vec<WrittenPart> = Vec::new();
            written_vector_parts(&chart, size, scale, &mut |part| written.push(part)).unwrap();
            let svg = rr
                .export_svg(&chart, size, renderer::ExportOptions::document(scale))
                .block_on()
                .unwrap();
            assert!(
                !svg.contains(&selection),
                "{context}: a selection box was drawn"
            );
            let file = svg_parts::read(&svg);
            let physical = (
                (size.0 as f32 * scale).round() as u32,
                (size.1 as f32 * scale).round() as u32,
            );
            assert_eq!(file.size, physical, "{context}: viewBox");
            assert_eq!(file.page, [600.0, 390.0], "{context}: size in points");
            assert_parts_match(
                &file.parts,
                &written,
                TOLERANCE,
                0.0,
                &format!("{context} SVG"),
            );
            assert_eq!(file.images.len(), 1, "{context}: one frame image");
            let pdf = pdf_structure::read(
                &rr.export_pdf(&chart, size, renderer::ExportOptions::document(scale))
                    .block_on()
                    .unwrap(),
            );
            assert_eq!(pdf.page, [600.0, 390.0]);
            let parts = pdf_structure::content_parts(&pdf, physical);
            assert_parts_match(&parts, &written, TOLERANCE, 0.0, &format!("{context} PDF"));
            assert!(
                pdf.images["Im1"].rgba == decode_png(&file.images[0].1).rgba,
                "{context}: the PDF's frame image is the SVG's"
            );
            // Retire this case's frame and export buffers before the next:
            // four documents at two scales would otherwise outgrow the
            // radial renderer's default budget.
            rr.end_frame();
            gpu.device()
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            assert_eq!(
                svg,
                rr.export_svg(&chart, size, renderer::ExportOptions::document(scale))
                    .block_on()
                    .unwrap(),
                "{context}: deterministic"
            );
        }
    }
}

/// The radial vector parts are recorded from a validated chart only, as the
/// exports they mirror (#152): a split naming a slice past the chart's
/// slices is refused with `RadialError::Invalid` and records nothing.
#[test]
fn the_radial_vector_parts_refuse_a_chart_that_does_not_validate() {
    for slice_index in [scenes::radial::radial_chart().slices.len(), usize::MAX] {
        let mut chart = scenes::radial::radial_chart();
        chart.split = Some(RadialSplit {
            slice_index,
            children: vec![RadialSlice::new("A", 1.0, Color::BLACK)],
            kind: RadialKind::Pie,
        });
        let mut written = 0usize;
        let result = written_vector_parts(&chart, (800, 520), 1.0, &mut |_| written += 1);
        assert!(
            matches!(result, Err(renderer::radial::RadialError::Invalid(_))),
            "split.slice_index {slice_index}: {result:?}"
        );
        assert_eq!(written, 0, "split.slice_index {slice_index}");
    }
}

/// RASTER_IMAGE_RGBA_LEN (#152): every image the writers embed (a data
/// group's crop of the panel readback, the Milkyway backdrop, the
/// colourbar strip, the radial frame) has `width * height * 4` bytes. The
/// document exports succeed in both formats at 1x and 2x for odd sizes
/// (1x1, 3x5 and, when the device holds it, 4097x3), the subplot, offset
/// spectra and field scenes and the Milkyway style; the radial exports at
/// odd sizes from its smallest canvas up, and refuse a smaller one.
#[test]
fn odd_sizes_subplots_and_milkyway_export_their_images() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    let max = gpu.device().limits().max_texture_dimension_2d;
    let mut documents = Vec::new();
    for (width, height) in [(1, 1), (3, 5), (4097, 3)] {
        let mut config = renderer::default::default_config();
        config.chart_area = renderer::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width,
            height,
        });
        let mut sky = config.clone();
        sky.draw_style = DrawStyle::Milkyway(Default::default());
        let name = format!("{width}x{height}");
        documents.push((format!("Milkyway {name}"), sky, Vec::new()));
        documents.push((name, config, Vec::new()));
    }
    for kind in [
        SceneKind::Subplots,
        SceneKind::OffsetSpectra,
        SceneKind::Field,
    ] {
        let scene = scenes::build(&mut r, kind, 1000).unwrap();
        let mut sky = scene.config.clone();
        sky.draw_style = DrawStyle::Milkyway(Default::default());
        for subplot in &mut sky.subplots {
            subplot.config.draw_style = sky.draw_style;
        }
        documents.push((format!("{kind:?}"), scene.config, scene.series.clone()));
        documents.push((format!("Milkyway {kind:?}"), sky, scene.series));
    }
    let mut exported = 0;
    for (name, config, series) in &documents {
        let area = config.document_area();
        let chart = Chart::new(config.clone());
        for scale in SCALES {
            let fits = |n: u32| (n as f32 * scale).round() as u32 <= max;
            if !(fits(area.width) && fits(area.height)) {
                continue;
            }
            let context = format!("{name} at {scale}x");
            let svg = r
                .export_svg(
                    renderer::ExportSource::chart(&chart, series),
                    renderer::ExportOptions::document(scale),
                )
                .block_on()
                .unwrap_or_else(|e| panic!("{context}: {e}"));
            let file = svg_parts::read(&svg);
            for (rect, png) in &file.images {
                let image = decode_png(png);
                assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
                assert_eq!(
                    (image.width, image.height),
                    (rect.width, rect.height),
                    "{context}"
                );
            }
            pdf_structure::read(
                &r.export_pdf(
                    renderer::ExportSource::chart(&chart, series),
                    renderer::ExportOptions::document(scale),
                )
                .block_on()
                .unwrap_or_else(|e| panic!("{context}: {e}")),
            );
            r.end_gpu_frame();
            exported += 1;
        }
    }
    assert!(exported >= documents.len());

    let mut rr = RadialRenderer::try_new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    for (name, chart) in radial_cases() {
        for scale in SCALES {
            for size in [(1, 1), (3, 5)] {
                // Below the radial chart's smallest canvas: refused.
                for result in [
                    rr.export_svg(&chart, size, renderer::ExportOptions::document(scale))
                        .block_on()
                        .map(|_| ()),
                    rr.export_pdf(&chart, size, renderer::ExportOptions::document(scale))
                        .block_on()
                        .map(|_| ()),
                ] {
                    assert!(
                        matches!(result, Err(renderer::radial::RadialError::Invalid(_))),
                        "{name} {size:?} at {scale}x: {result:?}"
                    );
                }
            }
        }
        for size in [(801, 523), (1001, 641)] {
            for scale in SCALES {
                let context = format!("{name} {size:?} at {scale}x");
                let svg = rr
                    .export_svg(&chart, size, renderer::ExportOptions::document(scale))
                    .block_on()
                    .unwrap_or_else(|e| panic!("{context}: {e:?}"));
                let file = svg_parts::read(&svg);
                assert_eq!(file.images.len(), 1, "{context}");
                let image = decode_png(&file.images[0].1);
                assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
                pdf_structure::read(
                    &rr.export_pdf(&chart, size, renderer::ExportOptions::document(scale))
                        .block_on()
                        .unwrap_or_else(|e| panic!("{context}: {e:?}")),
                );
                rr.end_frame();
            }
        }
    }
}

// An axis with a unit (#273) exports what it draws: the SVG, the PDF and
// the PNG at 1x and 2x of a chart whose axes print 16-bit ADC counts as
// volts are those of the same chart with its range written in volts, and
// differ from the counts printed as they are.
#[test]
fn an_axis_unit_exports_its_converted_labels() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    let chart = |counts: bool, unit: bool| {
        let mut config = renderer::default::default_config();
        for axis in [&mut config.bottom_x, &mut config.left_y] {
            axis.min = 0.0;
            axis.max = if counts { 32768.0 } else { 5.0 };
            axis.major_spacing = 1.0;
            axis.minor_count = 4;
            axis.unit = unit.then(|| renderer::config::AxisUnit::new(5.0 / 32768.0, 0.0));
            axis.label_style.format = renderer::format::LabelFormat::Decimal;
            axis.label_style.significant_digits = 1;
        }
        Chart::new(config)
    };
    let converted = chart(true, true);
    let written_in_volts = chart(false, false);
    let raw_counts = chart(true, false);
    for scale in SCALES {
        let options = renderer::ExportOptions::document(scale);
        let svg = |r: &mut Renderer, chart: &Chart| {
            r.export_svg(renderer::ExportSource::chart(chart, &[]), options)
                .block_on()
                .unwrap()
        };
        let pdf = |r: &mut Renderer, chart: &Chart| {
            r.export_pdf(renderer::ExportSource::chart(chart, &[]), options)
                .block_on()
                .unwrap()
        };
        let png = |r: &mut Renderer, chart: &Chart| {
            r.export_rgba(renderer::ExportSource::chart(chart, &[]), options)
                .block_on()
                .unwrap()
                .rgba
        };
        let text = svg(&mut r, &converted);
        assert_eq!(text, svg(&mut r, &written_in_volts), "SVG at {scale}x");
        assert_ne!(text, svg(&mut r, &raw_counts), "SVG at {scale}x");
        assert_eq!(
            pdf(&mut r, &converted),
            pdf(&mut r, &written_in_volts),
            "PDF at {scale}x"
        );
        let image = png(&mut r, &converted);
        assert!(
            image == png(&mut r, &written_in_volts),
            "PNG at {scale}x differs"
        );
        assert!(image != png(&mut r, &raw_counts), "PNG at {scale}x");
        // The parts the SVG was handed are the ones it reads back as.
        let mut written = Vec::new();
        written_document_parts(&converted, &[], scale, &mut |part| written.push(part)).unwrap();
        assert_parts_match(
            &svg_parts::read(&text).parts,
            &written,
            TOLERANCE,
            0.0,
            &format!("converted axes at {scale}x"),
        );
    }
}

/// An 800x600 chart and a 400x300 subplot beside it with a title, axis
/// titles, a legend and a colourbar: half the parent's size, so its text is
/// drawn at 0.79 of its stored sizes (#292).
fn small_cell_document() -> Config {
    let titled = |title: &str| {
        Chart::sized(800, 600)
            .with_title(title)
            .with_x_title("Hour")
            .with_y_title("Level")
            .config()
            .clone()
    };
    let mut config = titled("Parent");
    let mut cell = titled("Cell");
    cell.chart_area.0 = Rect {
        x: 800,
        y: 0,
        width: 400,
        height: 300,
    };
    cell.legend.visible = true;
    cell.legend.content.segments = renderer::text::rich_segments_from_text("legend");
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.title_option.visible = true;
    bar.axis.title_option.text.segments = renderer::text::rich_segments_from_text("z");
    cell.colorbar = Some(bar);
    config.subplots.push(renderer::Subplot::new(cell, &[]));
    renderer::layout::auto_layout(
        &config,
        &renderer::CpuTextMeasure::for_style(&config.draw_style),
    )
}

// A subplot's text scale (#292) reaches every export: the SVG, the PDF and
// the PNG at 1x and 2x of a document whose small cell is `Auto` are those
// of the same document with the cell `Fixed` and its text sizes written at
// the drawn size, and differ from the cell `Fixed` at its stored sizes.
#[test]
fn a_scaled_subplot_exports_the_text_sizes_it_draws() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    let auto = small_cell_document();
    let scale = auto.subplot_text_scale(0);
    assert!((scale - 0.5f32.cbrt()).abs() < 1e-6, "{scale}");
    let fixed = |written: Config| {
        let mut document = auto.clone();
        document.subplots[0].config = written;
        document.subplots[0].text_scale = renderer::SubplotTextScale::Fixed;
        document
    };
    let written_scaled = fixed(auto.subplot_config(0).unwrap());
    assert_eq!(
        written_scaled.subplots[0].config.chart_title.text.font_size,
        auto.subplots[0].config.chart_title.text.font_size * scale
    );
    let stored = fixed(auto.subplots[0].config.clone());
    let [auto, written_scaled, stored] = [auto, written_scaled, stored].map(Chart::new);
    for export in SCALES {
        let options = renderer::ExportOptions::document(export);
        let svg = |r: &mut Renderer, chart: &Chart| {
            r.export_svg(renderer::ExportSource::chart(chart, &[]), options)
                .block_on()
                .unwrap()
        };
        let pdf = |r: &mut Renderer, chart: &Chart| {
            r.export_pdf(renderer::ExportSource::chart(chart, &[]), options)
                .block_on()
                .unwrap()
        };
        let png = |r: &mut Renderer, chart: &Chart| {
            r.export_rgba(renderer::ExportSource::chart(chart, &[]), options)
                .block_on()
                .unwrap()
                .rgba
        };
        let text = svg(&mut r, &auto);
        assert_eq!(text, svg(&mut r, &written_scaled), "SVG at {export}x");
        assert_ne!(text, svg(&mut r, &stored), "SVG at {export}x");
        let document = pdf(&mut r, &auto);
        assert_eq!(document, pdf(&mut r, &written_scaled), "PDF at {export}x");
        assert_ne!(document, pdf(&mut r, &stored), "PDF at {export}x");
        let image = png(&mut r, &auto);
        assert!(
            image == png(&mut r, &written_scaled),
            "PNG at {export}x differs"
        );
        assert!(image != png(&mut r, &stored), "PNG at {export}x");
        // The parts the SVG was handed are the ones it reads back as.
        let mut written = Vec::new();
        written_document_parts(&auto, &[], export, &mut |part| written.push(part)).unwrap();
        assert_parts_match(
            &svg_parts::read(&text).parts,
            &written,
            TOLERANCE,
            0.0,
            &format!("a scaled subplot at {export}x"),
        );
    }
}

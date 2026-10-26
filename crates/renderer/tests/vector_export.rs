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
//!   export's crop, byte for byte; with decorations shown, the CPU composite
//!   of the grid raster, the data image and the decoration raster is the
//!   PNG export within 1/255 on all but 0.05 % of the channels (8-bit
//!   blend rounding, at most 3/255).
//! - Size budgets, determinism, the clear colour, a streamed source's
//!   rejection, the sketch and Milkyway styles and the radial chart.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/pdf_structure.rs"]
mod pdf_structure;
#[path = "support/playground_scenes.rs"]
mod scenes;
#[path = "support/svg_parts.rs"]
mod svg_parts;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::Arc;

use renderer::axis_render::{AxisLayerKind, try_raster_chart_layer_to_rgba};
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
    RendererDevice::new(Arc::new(device), Arc::new(queue))
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
    scale: f32,
    amplitude: f32,
) -> Sizes {
    let svg = r.export_document_svg(chart, series, scale).unwrap();
    let pdf = r.export_document_pdf(chart, series, scale).unwrap();
    let image = r.export_document_rgba(chart, series, scale).unwrap();
    let png = encode_png(&image).unwrap();
    let mut written = Vec::new();
    written_document_parts(chart, scale, &mut |part| written.push(part)).unwrap();

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
            let sizes = check_document(&mut r, &context, &chart, &scene.series, scale, 0.0);
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
            scale,
            1.5 * scale,
        );
    }
    let mut milkyway = scene.config.clone();
    milkyway.draw_style = DrawStyle::Milkyway(Default::default());
    let chart = Chart::new(milkyway);
    check_document(&mut r, "Milkyway Lines", &chart, &scene.series, 1.0, 0.0);
    let svg = svg_parts::read(&r.export_document_svg(&chart, &scene.series, 1.0).unwrap());
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
    for overlay in &mut config.overlays {
        hide_axis(&mut overlay.y_axis);
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
// as a PNG; nothing of the export lies outside the images.
#[test]
fn the_data_images_are_the_png_exports_pixels() {
    let gpu = gpu();
    let mut r = renderer(&gpu);
    for kind in cartesian_scenes() {
        let scene = scenes::build(&mut r, kind, BIG_DATA_POINTS).unwrap();
        let mut config = scene.config.clone();
        hide_decorations(&mut config);
        let chart = Chart::new(config);
        for scale in SCALES {
            let context = format!("{kind:?} at {scale}x");
            let export = r
                .export_document_rgba(&chart, &scene.series, scale)
                .unwrap();
            let svg =
                svg_parts::read(&r.export_document_svg(&chart, &scene.series, scale).unwrap());
            assert!(!svg.images.is_empty(), "{context}: a data image");
            let mut covered = vec![false; (export.width * export.height) as usize];
            for (rect, bytes) in &svg.images {
                assert!(
                    *bytes == encode_png(&crop(&export, *rect)).unwrap(),
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
        }
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
            let export = r
                .export_document_rgba(&chart, &scene.series, scale)
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
            let below = try_raster_chart_layer_to_rgba(&placed, AxisLayerKind::Grid).unwrap();
            let above = try_raster_chart_layer_to_rgba(&placed, AxisLayerKind::Decoration).unwrap();
            let svg =
                svg_parts::read(&r.export_document_svg(&chart, &scene.series, scale).unwrap());
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
            let (rect, image) = &data[0];
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
                    if x >= rect.x
                        && y >= rect.y
                        && x < rect.x + rect.width
                        && y < rect.y + rect.height
                    {
                        let j = (((y - rect.y) * rect.width + x - rect.x) * 4) as usize;
                        over(&mut px, straight(&image.rgba[j..j + 4]));
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
            // within 1/255 but for a few channels, none past 3/255.
            let channels = (export.width * export.height * 4) as usize;
            assert!(worst <= 3.0, "{context}: composite differs by {worst}/255");
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
            let svg = r.export_document_svg(&chart, &scene.series, scale).unwrap();
            assert_eq!(
                svg,
                r.export_document_svg(&chart, &scene.series, scale).unwrap(),
                "{kind:?}"
            );
            let pdf = r.export_document_pdf(&chart, &scene.series, scale).unwrap();
            assert!(
                pdf == r.export_document_pdf(&chart, &scene.series, scale).unwrap(),
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
    let plain = svg_parts::read(&r.export_document_svg(&chart, &scene.series, 1.0).unwrap());
    let svg = r.export_document_svg_with_clear_async(&chart, &scene.series, 1.0, clear);
    let svg = pollster::block_on(svg).unwrap();
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
    let pdf = pollster::block_on(r.export_document_pdf_with_clear_async(
        &chart,
        &scene.series,
        1.0,
        clear,
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
        renderer::data_config::DataRenderType::Line {
            line: scenes::line_style(Color::BLACK, 1.0),
        },
    ));
    let chart = Chart::new(scene.config.clone());
    let rejected = |result: Result<(), FiggyError>| {
        assert!(
            matches!(&result, Err(FiggyError::ColumnNotResident { id }) if id == "st_x"),
            "{:?}",
            result
        );
    };
    rejected(r.export_document_svg(&chart, &series, 1.0).map(|_| ()));
    rejected(r.export_document_pdf(&chart, &series, 1.0).map(|_| ()));
    rejected(
        r.export_document_png_bytes(&chart, &series, 1.0)
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
        }));
    annotated.decorations.selected = Some(DecorationId::ChartTitle);
    vec![
        ("radial", plain),
        ("split", split),
        ("annotated, title selected", annotated),
    ]
}

// The radial chart: one image of the frame, then its title, labels,
// leader lines, split connectors and annotations, read back in both
// formats at both scales; no selection box is drawn.
#[test]
fn the_radial_chart_reads_back_as_its_emitted_parts() {
    let gpu = gpu();
    let mut rr = RadialRenderer::new(gpu, wgpu::TextureFormat::Rgba8Unorm).unwrap();
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
            let svg = rr.export_svg(&chart, size, scale).unwrap();
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
            let pdf = pdf_structure::read(&rr.export_pdf(&chart, size, scale).unwrap());
            assert_eq!(pdf.page, [600.0, 390.0]);
            let parts = pdf_structure::content_parts(&pdf, physical);
            assert_parts_match(&parts, &written, TOLERANCE, 0.0, &format!("{context} PDF"));
            assert!(
                pdf.images["Im1"].rgba == decode_png(&file.images[0].1).rgba,
                "{context}: the PDF's frame image is the SVG's"
            );
            assert_eq!(
                svg,
                rr.export_svg(&chart, size, scale).unwrap(),
                "{context}: deterministic"
            );
        }
    }
}

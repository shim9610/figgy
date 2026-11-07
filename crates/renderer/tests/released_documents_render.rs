//! A document written by a published figgy-model release draws the same
//! before and after this source saves it (CLAUDE.md): each released chart
//! document is exported as it was read and again after a save and a read,
//! at 1x and 2x, and the two images must be identical. `released_documents.rs`
//! checks the model values; this checks what the user sees.
#![cfg(feature = "serde")]
// Legacy documents are read through their deprecated reader types.
#![allow(deprecated)]

use test_gpu::BlockOn as _;

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::config::Config;
use renderer::{
    BOX_PLOT_RECORD, BoxPlotChart, CategoricalChart, Chart, ColumnSource, ExportOptions,
    ExportSource, RadarChart, RadialChart, RadialRenderer, RaggedSource, RecordSource, Renderer,
    RendererDevice, SeriesConfig, SeriesUpload, UploadValues,
};

const SCALES: [f32; 2] = [1.0, 2.0];
/// The canvas a radial document is exported on; its documents carry no size.
const RADIAL_SIZE: (u32, u32) = (1000, 700);

fn device() -> RendererDevice {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(test_gpu::device);
    RendererDevice::new(device.clone(), queue.clone()).expect("figgy poll thread")
}

fn read<T: serde::de::DeserializeOwned>(file: &str) -> T {
    let path = format!(
        "{}/tests/fixtures/released/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{file}: {err}"))
}

/// The document after this source saves it and reads the save back.
fn resaved<T: serde::Serialize + serde::de::DeserializeOwned>(document: &T) -> T {
    let saved = serde_json::to_string(document).expect("serialize");
    serde_json::from_str(&saved).expect("the saved document parses")
}

/// The same image, or the same refusal: a document the renderer cannot draw
/// (an empty one) must be refused for the same reason after the save.
fn assert_same_image(
    file: &str,
    scale: f32,
    before: &Result<Vec<u8>, String>,
    after: &Result<Vec<u8>, String>,
) {
    assert_same_result(file, scale, before, after);
    assert!(
        file.contains("-default") || before.is_ok(),
        "{file} {scale}x: an edited document must draw, or the comparison proves nothing: {before:?}"
    );
}

/// The same image, or the same refusal: a document the renderer cannot draw
/// must be refused for the same reason after the save.
fn assert_same_result(
    file: &str,
    scale: f32,
    before: &Result<Vec<u8>, String>,
    after: &Result<Vec<u8>, String>,
) {
    match (before, after) {
        (Ok(before), Ok(after)) => {
            assert_eq!(before.len(), after.len(), "{file} {scale}x: image sizes");
            let differing = before
                .chunks_exact(4)
                .zip(after.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(
                differing, 0,
                "{file} {scale}x: {differing} pixels differ after the save"
            );
        }
        (Err(before), Err(after)) => assert_eq!(before, after, "{file} {scale}x"),
        _ => panic!("{file} {scale}x: drawn {before:?}, after the save {after:?}"),
    }
}

/// Register what a converted legacy document uploads.
fn upload(r: &mut Renderer, uploads: &[SeriesUpload<'_>]) -> Result<(), String> {
    for upload in uploads {
        match &upload.values {
            UploadValues::BoxRecord(_) => {
                let fields = upload.values.box_fields().expect("box fields");
                let pairs: [(&str, &dyn ColumnSource); 9] = std::array::from_fn(|k| {
                    (
                        renderer::BoxField::ALL[k].name(),
                        &fields[k] as &dyn ColumnSource,
                    )
                });
                r.upsert_record(
                    upload.column.clone(),
                    &RecordSource {
                        schema: BOX_PLOT_RECORD,
                        fields: &pairs,
                    },
                )
                .map_err(|e| e.to_string())?;
            }
            UploadValues::BoxOutliers(_) => {
                let (values, offsets) = upload.values.box_outliers().expect("outliers");
                r.upsert_ragged(
                    upload.column.clone(),
                    &RaggedSource {
                        values: &values,
                        offsets: &offsets,
                    },
                )
                .map_err(|e| e.to_string())?;
            }
            _ => {
                r.add_column(upload.column.clone(), &upload.values)
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Export a chart document as the renderer draws it, at `scale`.
fn export_chart(
    config: &Config,
    series: &[SeriesConfig],
    uploads: &[SeriesUpload<'_>],
    scale: f32,
) -> Result<Vec<u8>, String> {
    let mut r = Renderer::try_new(device(), wgpu::TextureFormat::Rgba8Unorm, 4 * 1024 * 1024)
        .expect("renderer");
    upload(&mut r, uploads)?;
    let chart = Chart::new(config.clone());
    r.export_rgba(
        ExportSource::chart(&chart, series),
        ExportOptions::document(scale),
    )
    .block_on()
    .map(|image| image.rgba)
    .map_err(|e| e.to_string())
}

fn check_config(file: &str) {
    let before: Config = read(file);
    let after = resaved(&before);
    for scale in SCALES {
        assert_same_image(
            file,
            scale,
            &export_chart(&before, &[], &[], scale),
            &export_chart(&after, &[], &[], scale),
        );
    }
}

fn check_radial(file: &str) {
    let mut before: RadialChart = read(file);
    let mut after = resaved(&before);
    // The renderer draws the label strings a document stores; documents
    // written before the strings existed draw them once loaded.
    before.normalize_labels();
    after.normalize_labels();
    let mut r = RadialRenderer::try_new(device(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut export = |chart: &RadialChart, scale: f32| {
        let image = r
            .export_rgba(chart, RADIAL_SIZE, ExportOptions::document(scale))
            .block_on()
            .map(|image| image.rgba)
            .map_err(|e| e.to_string());
        r.end_frame();
        image
    };
    for scale in SCALES {
        // As written: the edited documents set a plot area starting at
        // y = 60 under a title, whose band every release (0.16.1 to now)
        // makes at least 70 px tall, so every release refuses to draw them
        // (an explicit area must fit the measured plot region). The save
        // must keep the same answer.
        assert_same_result(file, scale, &export(&before, scale), &export(&after, scale));
        // Drawn: the same documents with the automatic plot area, so the
        // comparison covers every other field in pixels.
        let mut drawn = [before.clone(), after.clone()];
        for chart in &mut drawn {
            chart.decorations.data_area = None;
        }
        let [drawn_before, drawn_after] = drawn;
        assert_same_image(
            file,
            scale,
            &export(&drawn_before, scale),
            &export(&drawn_after, scale),
        );
    }
}

/// A legacy document drawn through its reader: `convert` opens it as the
/// chart and the uploads the renderer draws.
fn check_reader<T>(
    file: &str,
    convert: impl Fn(&T) -> (Config, Vec<SeriesConfig>, Vec<SeriesUpload<'_>>),
) where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let before: T = read(file);
    let after = resaved(&before);
    let (before_config, before_series, before_uploads) = convert(&before);
    let (after_config, after_series, after_uploads) = convert(&after);
    for scale in SCALES {
        assert_same_image(
            file,
            scale,
            &export_chart(&before_config, &before_series, &before_uploads, scale),
            &export_chart(&after_config, &after_series, &after_uploads, scale),
        );
    }
}

#[test]
fn released_configs_draw_the_same_after_a_save() {
    for version in ["0.10.0", "0.11.0"] {
        for name in ["config-default", "config-series", "config-edited"] {
            check_config(&format!("model-{version}-{name}.json"));
        }
    }
}

#[test]
fn released_radial_charts_draw_the_same_after_a_save() {
    for version in ["0.10.0", "0.11.0", "0.12.1"] {
        for name in ["radial-default", "radial-edited"] {
            check_radial(&format!("model-{version}-{name}.json"));
        }
    }
}

#[test]
fn released_legacy_documents_draw_the_same_after_a_save() {
    for version in ["0.10.0", "0.11.0", "0.12.1"] {
        for name in ["categorical-default", "categorical-edited"] {
            check_reader::<CategoricalChart>(
                &format!("model-{version}-{name}.json"),
                Config::from_categorical,
            );
        }
        for name in ["boxplot-default", "boxplot-edited"] {
            check_reader::<BoxPlotChart>(
                &format!("model-{version}-{name}.json"),
                Config::from_boxplot,
            );
        }
    }
    for name in ["radar-default", "radar-edited"] {
        check_reader::<RadarChart>(&format!("model-0.12.1-{name}.json"), Config::from_radar);
    }
}

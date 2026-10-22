//! Tier 2 of #46: leaf mutations of the whole `Config` against the chart
//! revisions. For every leaf of each SSOT baseline document, edit the leaf
//! through `set_chart_config` and read the revisions the edit produced.
//!
//! - The cache revisions `raster` (`raster_config_equal`), `view`
//!   (`stream_config_equal`, with `overlay_geometry_equal` and
//!   `subplot_geometry_equal`), `selection` (`selection_config_revision`),
//!   `annotation` (`annotation_config_revision`) and `data`: an edit that
//!   moves none of them must not change the rendered frame, unless the
//!   frame reads the field live, in which case a host holding its view and
//!   following `ChartRenderStamp` must still draw exactly what a fresh
//!   renderer draws. A stale first frame with no cache revision moved is a
//!   gap.
//! - The stream prefix is keyed by the `view` and `data` revisions only (a
//!   raster refresh composites chrome over the kept prefix). An edit that
//!   moves neither must leave the streamed data prefix as it was.
//!
//! The other direction (a revision that moves although nothing changed) is
//! not tested here; the single-property matrix in the parent module does
//! that for the classified fields.
use super::*;
use crate::renderer::config_mutation::{Outcome, Tally, run};

/// The series id, the column ids and a bundled font family other than the
/// default: valid replacements for the string leaves of these documents.
/// The first is also the value a synthesized string takes.
const STRINGS: &[&str] = &["s", "x", "a", "Comic Neue"];

fn cache_revisions(r: &Renderer, id: ChartId) -> [RenderRevision; 5] {
    let revisions = r.chart_states[&id].revisions;
    [
        revisions.raster,
        revisions.view,
        revisions.selection,
        revisions.annotation,
        revisions.data,
    ]
}

/// A host that keeps one view and follows `ChartRenderStamp`: refresh the
/// axis raster when the stamp says so, else the annotation rasters when the
/// stamp says so, then prepare and paint.
struct Host {
    r: Renderer,
    id: ChartId,
    view: ChartView,
    stamp: Option<ChartRenderStamp>,
}

impl Host {
    fn new(config: &Config, series: &[SeriesConfig]) -> Self {
        let (mut r, id) = fixture(config, true, false);
        r.set_chart_series(id, series.to_vec()).unwrap();
        let view = r
            .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
            .unwrap();
        let mut host = Self {
            r,
            id,
            view,
            stamp: None,
        };
        host.stamp = Some(host.r.chart_render_stamp(id).unwrap());
        host.frame();
        host
    }

    fn frame(&mut self) -> Vec<u8> {
        let stamp = self.r.chart_render_stamp(self.id).unwrap();
        let config = self.r.chart_config(self.id).unwrap().clone();
        let chart = Chart::new(config.clone());
        if stamp.needs_raster_since(self.stamp.as_ref()) {
            self.r
                .refresh_axis(&mut self.view, &chart, config.chart_area.0)
                .unwrap();
        } else if stamp.needs_annotation_raster_since(self.stamp.as_ref()) {
            self.r
                .refresh_annotations(&mut self.view, &chart, config.chart_area.0, None)
                .unwrap();
        }
        self.stamp = Some(stamp);
        let prepared = self
            .r
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: self.id,
                view: &self.view,
            }])
            .unwrap();
        paint_frame_pixels(&self.r, &prepared, 1)
    }
}

/// A fresh renderer's first resident frame of `config` with `series`.
fn fresh_frame(config: &Config, series: &[SeriesConfig]) -> Vec<u8> {
    let (mut r, id) = fixture(config, true, false);
    r.set_chart_series(id, series.to_vec()).unwrap();
    frame(&mut r, id, true).0
}

fn differing_bytes(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(a, b)| a != b).count() + a.len().abs_diff(b.len())
}

/// The streamed data prefix of `config` drawn by a fresh chart on `oracle`,
/// or `None` when the stream pass does not accept the document.
fn stream_prefix(oracle: &mut Renderer, config: &Config) -> Option<Vec<u8>> {
    let id = oracle
        .register_chart(config.clone(), vec![declaration("s", "x", "a")])
        .ok()?;
    let view = oracle
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    if oracle
        .request_auto_streaming_chart(id, &view, options())
        .is_err()
    {
        oracle.remove_chart(id).unwrap();
        return None;
    }
    frame(oracle, id, false);
    let job = oracle.active_stream_job(id).unwrap();
    let prefix = oracle.chart_stream_prefix_for_test(job).unwrap();
    let pixels = read_draw_target(oracle, &prefix);
    oracle.remove_chart(id).unwrap();
    Some(pixels)
}

/// The selection baseline of the parent matrix: a picked point on a
/// scatter-line series.
fn picked_suite() -> (Config, Vec<SeriesConfig>) {
    use crate::config::{PickedPointRef, PickedPointsConfig};
    let mut c = baseline();
    c.picked_points = Some(PickedPointsConfig {
        refs: vec![PickedPointRef {
            source_id: None,
            series_id: "s".into(),
            point_index: 2,
        }],
        ..Default::default()
    });
    let mut series = declaration("s", "x", "a");
    let line = super::super::super::extract_line(&series.render_type)
        .unwrap()
        .clone();
    series.render_type = DataRenderType::ScatterLine {
        line,
        scatter: DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_size: 4.0,
            point_shape: ScatterShape::CircleFilled,
            point_style_index_column: None,
            point_style_table: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        },
    };
    (c, vec![series])
}

/// Every baseline of the parent matrix, with whether the stream pass
/// accepts it (overlays and subplots are rejected there by contract).
fn suites() -> Vec<(&'static str, Config, Vec<SeriesConfig>, bool)> {
    let line = vec![declaration("s", "x", "a")];
    let (picked, picked_series) = picked_suite();
    vec![
        ("baseline", baseline(), line.clone(), true),
        ("colorbar", baseline_with_colorbar(), line.clone(), true),
        ("overlay", baseline_with_overlay(), line.clone(), false),
        ("subplot", baseline_with_subplot(), line.clone(), false),
        ("annotations", baseline_with_annotations(), line, true),
        ("picked", picked, picked_series, true),
    ]
}

#[test]
fn every_config_leaf_that_changes_the_frame_moves_a_cache_revision() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut total = Tally::default();
    let mut live = Vec::new();
    for (name, config, series, _) in suites() {
        let mut host = Host::new(&config, &series);
        let base = fresh_frame(&config, &series);
        let tally = run(&format!("{name}: "), &config, STRINGS, |label, next| {
            let before = cache_revisions(&host.r, host.id);
            if host.r.set_chart_config(host.id, next.clone()).is_err() {
                return Outcome::Rejected;
            }
            let moved = cache_revisions(&host.r, host.id) != before;
            let outcome = if moved {
                Outcome::Held
            } else {
                let expected = fresh_frame(&next, &series);
                if expected == base {
                    Outcome::Held
                } else {
                    let actual = host.frame();
                    if actual == expected {
                        live.push(label.to_string());
                        Outcome::Held
                    } else {
                        Outcome::Gap(format!(
                            "no cache revision moved, the fresh frame differs from the \
                             baseline in {} bytes and the host's first frame differs from \
                             the fresh frame in {} bytes",
                            differing_bytes(&expected, &base),
                            differing_bytes(&actual, &expected)
                        ))
                    }
                }
            };
            host.r.set_chart_config(host.id, config.clone()).unwrap();
            if !moved && matches!(outcome, Outcome::Gap(_)) {
                // The host drew the edit into caches keyed as the baseline.
                host = Host::new(&config, &series);
            } else if !moved {
                host.frame();
            }
            outcome
        });
        eprintln!("{}", tally.summary(&format!("revisions/{name}")));
        total.absorb(tally);
    }
    eprintln!("{}", total.summary("revisions (all suites)"));
    eprintln!(
        "{} leaves change the frame with no cache revision and are read live:\n{}",
        live.len(),
        live.join("\n")
    );
    eprintln!("skipped:\n{}", total.skipped.join("\n"));
    // Release the font lock before the verdict, so a confirmed gap fails
    // this test alone instead of poisoning the lock for every later test.
    drop(_fonts);
    assert!(
        total.gaps.is_empty(),
        "{} config leaves change the frame without moving a cache revision:\n{}",
        total.gaps.len(),
        total.gaps.join("\n")
    );
}

#[test]
fn every_config_leaf_that_changes_the_stream_prefix_moves_the_view_revision() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut oracle, _) = fixture(&baseline(), false, false);
    let mut total = Tally::default();
    for (name, config, series, streams) in suites() {
        if !streams {
            continue;
        }
        let (mut r, id) = fixture(&config, false, false);
        r.set_chart_series(id, series.clone()).unwrap();
        let base = stream_prefix(&mut oracle, &config).expect("the baseline streams");
        let tally = run(&format!("{name}: "), &config, STRINGS, |_, next| {
            let revisions = r.chart_states[&id].revisions;
            if r.set_chart_config(id, next.clone()).is_err() {
                return Outcome::Rejected;
            }
            let after = r.chart_states[&id].revisions;
            r.set_chart_config(id, config.clone()).unwrap();
            if after.view != revisions.view || after.data != revisions.data {
                return Outcome::Held;
            }
            // The stream pass has its own admission rules; a document it
            // rejects has no prefix to compare.
            let Some(prefix) = stream_prefix(&mut oracle, &next) else {
                return Outcome::Rejected;
            };
            if prefix == base {
                Outcome::Held
            } else {
                Outcome::Gap(format!(
                    "view and data revisions kept, the streamed prefix differs in {} bytes",
                    differing_bytes(&prefix, &base)
                ))
            }
        });
        eprintln!("{}", tally.summary(&format!("stream view/{name}")));
        total.absorb(tally);
    }
    eprintln!("{}", total.summary("stream view (all suites)"));
    eprintln!("skipped:\n{}", total.skipped.join("\n"));
    // Release the font lock before the verdict, so a confirmed gap fails
    // this test alone instead of poisoning the lock for every later test.
    drop(_fonts);
    assert!(
        total.gaps.is_empty(),
        "{} config leaves change the stream prefix without moving the view revision:\n{}",
        total.gaps.len(),
        total.gaps.join("\n")
    );
}

/// `prepare_load_demo_chart_plan` (registry.rs) bumps the raster revision
/// of the target unconditionally and never recomputes its annotation
/// revision. For every leaf of the annotation layer, load a demo whose
/// document differs from the chart's only in that leaf, and require a
/// host following `ChartRenderStamp` to draw exactly what a fresh
/// renderer draws: the stamp must ask for the annotation rasters to be
/// redone, through `needs_raster_since` (a `refresh_axis` redoes them) or
/// `needs_annotation_raster_since`.
#[test]
fn every_annotation_leaf_of_a_loaded_demo_reaches_the_first_frame() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let config = baseline_with_annotations();
    let series = vec![declaration("s", "x", "a")];
    let demo = |values: [f32; 3]| crate::Column {
        data: values.to_vec(),
        min: values[0],
        max: values[2],
    };
    let columns = [
        demo([0.0, 1.0, 2.0]),
        demo([20.0, 50.0, 80.0]),
        demo([0.0, 2.5, 5.0]),
        demo([0.0, 4.0, 5.0]),
    ];
    let mut host = Host::new(&config, &series);
    let mut annotation_kept = Vec::new();
    let tally = run(
        "annotations: ",
        &config.annotations,
        STRINGS,
        |label, layer| {
            let mut next = config.clone();
            next.annotations = layer;
            let before = host.r.chart_states[&host.id].revisions;
            let committed = match host.r.begin_load_demo(
                host.id,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
                |_| Ok((next.clone(), series.clone())),
            ) {
                Ok(guard) => {
                    guard.commit();
                    true
                }
                Err(_) => false,
            };
            if !committed {
                return Outcome::Rejected;
            }
            let after = host.r.chart_states[&host.id].revisions;
            if next.annotations.items != config.annotations.items
                && after.annotation == before.annotation
            {
                annotation_kept.push(label.to_string());
            }
            let stamp = host.r.chart_render_stamp(host.id).unwrap();
            let asked = stamp.needs_raster_since(host.stamp.as_ref())
                || stamp.needs_annotation_raster_since(host.stamp.as_ref());
            let actual = host.frame();
            let expected = fresh_frame(&next, &series);
            let outcome = if actual == expected {
                Outcome::Held
            } else {
                Outcome::Gap(format!(
                    "the host's first frame after the load differs from a fresh render in {} \
                     bytes (stamp asked for a raster refresh: {asked})",
                    differing_bytes(&actual, &expected)
                ))
            };
            // Back to the baseline document, through the same loader.
            host.r
                .begin_load_demo(
                    host.id,
                    &columns[0],
                    &columns[1],
                    &columns[2],
                    &columns[3],
                    |_| Ok((config.clone(), series.clone())),
                )
                .unwrap()
                .commit();
            host.frame();
            outcome
        },
    );
    eprintln!("{}", tally.summary("load demo annotations"));
    eprintln!(
        "{} loads changed annotations.items and kept the annotation revision \
         (the raster revision moved for every load):\n{}",
        annotation_kept.len(),
        annotation_kept.join("\n")
    );
    eprintln!("skipped:\n{}", tally.skipped.join("\n"));
    // Release the font lock before the verdict, so a confirmed gap fails
    // this test alone instead of poisoning the lock for every later test.
    drop(_fonts);
    assert!(
        tally.gaps.is_empty(),
        "{} annotation leaves of a loaded demo do not reach the first frame:\n{}",
        tally.gaps.len(),
        tally.gaps.join("\n")
    );
}

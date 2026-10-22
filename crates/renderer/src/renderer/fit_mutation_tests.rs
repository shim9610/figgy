//! Tier 2 of #46: leaf mutations against [`FitCommitToken`]. A host fits by
//! taking a token, computing the extents of the series the active layer
//! fits (`Config::series_fit_roles`, `begin_series_fit_extent`, the
//! histogram metadata), and committing them only while the token still
//! matches the renderer's state (`commit_auto_fit_all_if_current`, or the
//! token comparison the web host does for a non-base layer). An edit in
//! between that keeps the token lets the stale extents through.
//!
//! For every leaf of the chart's `Config` and of each `SeriesConfig`: take
//! the baseline token and extents, apply the edit, take the token again.
//! When the tokens are equal, the fit committed from the stale extents must
//! give the same axes as a fit computed after the edit; otherwise the token
//! is missing the leaf. The fixture is the resident-display panel (every
//! extent mode: paired, error bars, histogram metadata, matrix lattice and
//! data-scaled vector tips) with an overlay layer and a subplot, fitted on
//! the base, the overlay and the subplot layer in turn.
use super::*;
use crate::LayerRef;
use crate::chart::FitExtent;

type SeriesExtent = Option<(FitExtent, FitExtent)>;

fn fit_extent(extent: &crate::gpu_errorbar::GpuErrorbarExtent) -> FitExtent {
    FitExtent {
        min: extent.min,
        max: extent.max,
        min_positive: extent.min_positive,
    }
}

/// One series' fit extent the way the web host computes it.
fn series_extent(r: &mut Renderer, series: &SeriesConfig) -> SeriesExtent {
    if let DataRenderType::Histogram { bar } = &series.render_type {
        let vertical = matches!(bar.orientation, BarOrientation::Vertical);
        let (edges, counts) = if vertical {
            (&series.x_column, &series.y_column)
        } else {
            (&series.y_column, &series.x_column)
        };
        let edge = Chart::slot_extent(r.pool(), edges).ok()?;
        let mut count = Chart::slot_extent(r.pool(), counts).ok()?;
        count.union(&FitExtent {
            min: bar.baseline,
            max: bar.baseline,
            min_positive: (bar.baseline > 0.0).then_some(bar.baseline),
        });
        return Some(if vertical {
            (edge, count)
        } else {
            (count, edge)
        });
    }
    pollster::block_on(r.ensure_errorbar_extent_engine()).unwrap();
    let ticket = r.begin_series_fit_extent(series).ok()??;
    let extent = pollster::block_on(ticket.resolve()).ok()??;
    Some((fit_extent(&extent.x), fit_extent(&extent.y)))
}

fn fit_layer(config: &Config) -> LayerRef {
    match config.active_layer() {
        LayerRef::Annotations => LayerRef::Base,
        layer => layer,
    }
}

/// The axes `config` gets from fitting `layer` to the extents of the
/// series that layer fits under `roles`.
fn fitted(
    config: &Config,
    layer: LayerRef,
    roles: &Config,
    series: &[SeriesConfig],
    extents: &[SeriesExtent],
) -> Config {
    let mut x = FitExtent::EMPTY;
    let mut y = FitExtent::EMPTY;
    for (series, extent) in series.iter().zip(extents) {
        let (fits_x, fits_y) = roles.series_fit_roles(layer, &series.series_id);
        if let Some((ex, ey)) = extent {
            if fits_x {
                x.union(ex);
            }
            if fits_y {
                y.union(ey);
            }
        }
    }
    let mut chart = Chart::new(config.clone());
    chart.auto_fit_layer_extent(layer, &x, &y, 0.05);
    chart.config().clone()
}

/// The base panel with the vector series on an overlay layer and the
/// histogram in a subplot below, `active` receiving the fit.
fn fit_config(active: LayerRef) -> (Config, (u32, u32)) {
    let (mut c, size) = subplot_config();
    c.overlays.push(overlay(0.05, 1.5, "vec"));
    c.overlays[0].y_axis.out_margin = 18.0;
    c.active_layer = active;
    (c, size)
}

fn run_fit_suite(name: &str, active: LayerRef) -> Tally {
    let (config, _) = fit_config(active);
    let series = chart_series();
    let mut r = renderer(wgpu::TextureFormat::Rgba8Unorm, 1);
    add_columns(&mut r);
    let id = r.register_chart(config.clone(), series.clone()).unwrap();
    assert_eq!(fit_layer(r.chart_config(id).unwrap()), active);
    let token = r.begin_fit_commit(id).unwrap();
    let extents: Vec<SeriesExtent> = series.iter().map(|s| series_extent(&mut r, s)).collect();
    assert!(extents.iter().all(Option::is_some), "every series fits");

    let check = |r: &mut Renderer, next: Config, list: Vec<SeriesConfig>, changed| {
        if r.set_chart_state(id, next, list.clone()).is_err() {
            return Outcome::Rejected;
        }
        let outcome = match r.begin_fit_commit(id) {
            Err(_) => Outcome::Rejected,
            Ok(now) if now != token => Outcome::Held,
            Ok(_) => {
                let state = r.chart_config(id).unwrap().clone();
                let mut now = extents.clone();
                if let Some(index) = changed {
                    now[index] = series_extent(r, &list[index]);
                }
                // The host read the layer, the roles and the extents before
                // the edit and commits them onto the edited config.
                let stale = fitted(&state, active, &config, &series, &extents);
                let fresh = fitted(&state, active, &state, &list, &now);
                if stale == fresh {
                    Outcome::Held
                } else {
                    Outcome::Gap(format!(
                        "the token stayed equal and a fit taken before the edit commits other \
                         axes than a fit taken after it: {}",
                        axis_difference(&stale, &fresh)
                    ))
                }
            }
        };
        r.set_chart_state(id, config.clone(), series.clone())
            .unwrap();
        outcome
    };
    let mut tally = run(&format!("{name} config: "), &config, STRINGS, |_, next| {
        check(&mut r, next, series.clone(), None)
    });
    for index in 0..series.len() {
        tally.absorb(run(
            &format!("{name} series[{index}] ({}): ", series[index].series_id),
            &series[index],
            STRINGS,
            |_, next| {
                let mut list = series.clone();
                list[index] = next;
                check(&mut r, config.clone(), list, Some(index))
            },
        ));
    }
    eprintln!("{}", tally.summary(&format!("fit/{name}")));
    tally
}

/// The fitted ranges that differ, named by axis.
fn axis_difference(stale: &Config, fresh: &Config) -> String {
    let range = |a: &AxisOptions| (a.min, a.max);
    let mut axes = vec![
        ("bottom_x".to_string(), &stale.bottom_x, &fresh.bottom_x),
        ("left_y".to_string(), &stale.left_y, &fresh.left_y),
        ("top_x".to_string(), &stale.top_x, &fresh.top_x),
        ("right_y".to_string(), &stale.right_y, &fresh.right_y),
    ];
    for (i, (a, b)) in stale.overlays.iter().zip(&fresh.overlays).enumerate() {
        axes.push((format!("overlays[{i}].y_axis"), &a.y_axis, &b.y_axis));
    }
    for (i, (a, b)) in stale.subplots.iter().zip(&fresh.subplots).enumerate() {
        axes.push((
            format!("subplots[{i}].bottom_x"),
            &a.config.bottom_x,
            &b.config.bottom_x,
        ));
        axes.push((
            format!("subplots[{i}].left_y"),
            &a.config.left_y,
            &b.config.left_y,
        ));
    }
    let differing: Vec<String> = axes
        .into_iter()
        .filter(|(_, a, b)| range(a) != range(b))
        .map(|(name, a, b)| format!("{name} {:?} (stale) vs {:?} (fresh)", range(a), range(b)))
        .collect();
    if differing.is_empty() {
        "other config fields".into()
    } else {
        differing.join(", ")
    }
}

#[test]
fn every_leaf_that_changes_a_fit_changes_the_fit_token() {
    let _fonts = font_lock();
    let mut tally = run_fit_suite("base", LayerRef::Base);
    tally.absorb(run_fit_suite("overlay", LayerRef::Overlay(0)));
    tally.absorb(run_fit_suite("subplot", LayerRef::Subplot(0)));
    eprintln!("{}", tally.summary("fit (all)"));
    eprintln!("skipped:\n{}", tally.skipped.join("\n"));
    assert!(
        tally.gaps.is_empty(),
        "{} leaves change a committed fit without changing FitCommitToken:\n{}",
        tally.gaps.len(),
        tally.gaps.join("\n")
    );
}

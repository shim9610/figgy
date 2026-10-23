use super::*;
use crate::config::{DrawStyle, SketchOptions};
use crate::data_config::ContourLabelConfig;
use crate::data_render::contour_label_atlas::{
    ContourLabelAtlasError, ContourLabelAtlasInput, contour_label_atlas_grid,
};
use crate::default::default_config;
use crate::layout::LABEL_GAP;
use crate::layout::{TitleBand, axis_title_placement, colorbar_rect, label_origin};
use crate::render_warning::SpacingRejection;
use crate::text::{RichSegment, RichText};
use crate::tick::{MAX_LOG_DECADE_STEP, MAX_MINOR_SUBDIVISIONS};

fn contour_label_fixture() -> (Config, ContourLabelConfig) {
    let config = default_config();
    let labels = ContourLabelConfig {
        visible: true,
        font_size: 12.0,
        color: Color::BLACK,
        format: LabelFormat::Decimal,
        significant_digits: 3,
        spacing_px: 80.0,
        anchors: Vec::new(),
        bg_color: Some(Color::WHITE),
        bg_padding_px: 0.0,
    };
    (config, labels)
}

#[test]
fn a_thousand_and_twenty_four_labels_pack_into_a_2d_atlas() {
    let (config, labels) = contour_label_fixture();
    let texts = vec!["1".to_owned(); 1024];
    let input = ContourLabelAtlasInput::from_config(&config, &labels, texts.clone(), 1.0)
        .expect("visible non-empty labels have an atlas input");
    let atlas = bake_contour_label_atlas(&input, 4096)
        .expect("the device limit admits the grid")
        .expect("visible non-empty labels bake an atlas");

    assert_eq!(atlas.cells.len(), 1024);
    assert!(atlas.columns > 1, "1024 labels must not remain one column");
    assert!(atlas.rows > 1, "1024 labels must not remain one row");
    assert!(atlas.columns * atlas.rows >= 1024);
    assert!(atlas.width <= 4096 && atlas.height <= 4096);
    assert_eq!(atlas.width, atlas.columns * atlas.cell_stride_w);
    assert_eq!(atlas.height, atlas.rows * atlas.cell_stride_h);
    assert_eq!(atlas.gutter, 1);
    assert_eq!(
        atlas.rgba.len(),
        atlas.width as usize * atlas.height as usize * 4
    );
}

#[test]
fn every_label_cell_has_a_transparent_texel_gutter() {
    let (config, labels) = contour_label_fixture();
    let texts = vec!["A".to_owned(); 4];
    let input = ContourLabelAtlasInput::from_config(&config, &labels, texts.clone(), 1.0)
        .expect("visible non-empty labels have an atlas input");
    let atlas = bake_contour_label_atlas(&input, 128)
        .expect("the device limit admits the grid")
        .expect("visible non-empty labels bake an atlas");
    let alpha = |x: u32, y: u32| {
        let offset = ((y * atlas.width + x) * 4 + 3) as usize;
        atlas.rgba[offset]
    };

    for index in 0..texts.len() as u32 {
        let column = index % atlas.columns;
        let row = index / atlas.columns;
        let left = column * atlas.cell_stride_w;
        let top = row * atlas.cell_stride_h;
        assert_eq!(alpha(left, top), 0, "cell {index} top-left gutter");
        assert_eq!(
            alpha(left + atlas.cell_stride_w - 1, top + atlas.gutter),
            0,
            "cell {index} trailing gutter"
        );
        assert_eq!(
            alpha(left + atlas.gutter, top + atlas.gutter),
            255,
            "cell {index} opaque background begins inside its gutter"
        );
    }
}

#[test]
fn an_atlas_past_the_device_limit_is_an_explicit_error() {
    let error = contour_label_atlas_grid(5, 34, 18, 64).expect_err("capacity is only three");
    assert_eq!(
        error,
        ContourLabelAtlasError::DeviceLimit {
            level_count: 5,
            capacity: 3,
            cell_stride_w: 34,
            cell_stride_h: 18,
            max_dimension: 64,
        }
    );
    assert!(error.to_string().contains("capacity 3"));
}

#[test]
fn disabled_or_empty_labels_remain_a_no_op() {
    let (config, mut labels) = contour_label_fixture();
    labels.visible = false;
    assert!(
        ContourLabelAtlasInput::from_config(&config, &labels, vec!["1".into()], 1.0).is_none(),
        "disabled labels have no atlas input"
    );
    labels.visible = true;
    assert!(
        ContourLabelAtlasInput::from_config(&config, &labels, vec![], 1.0).is_none(),
        "empty labels have no atlas input"
    );
}

/// Majors anchor to absolute spacing multiples (nice values even when
/// the range ends are arbitrary), and minors cover the partial edge
/// intervals beyond the outermost majors.
#[test]
fn ticks_anchor_to_grid_and_minors_cover_edges() {
    let mut axis = default_config().bottom_x.clone();
    axis.min = 1200.0;
    axis.max = 1950.0;
    axis.major_spacing = 200.0;
    axis.minor_count = 3; // minors every 50

    let majors = major_tick_values(&axis);
    assert_eq!(majors, vec![1200.0, 1400.0, 1600.0, 1800.0]);

    let minors = minor_tick_values(&axis);
    for m in [1850.0, 1900.0, 1950.0] {
        assert!(
            minors.iter().any(|v| (v - m).abs() < 1e-6),
            "edge minor {m} missing: {minors:?}"
        );
    }
    assert!(
        minors.iter().all(|v| (v / 200.0).fract().abs() > 1e-9),
        "majors leaked into minors: {minors:?}"
    );

    // Arbitrary (uniform-margin) range ends: tick VALUES stay nice.
    axis.min = 0.137;
    axis.max = 2.63;
    axis.major_spacing = 0.5;
    let majors = major_tick_values(&axis);
    assert_eq!(majors, vec![0.5, 1.0, 1.5, 2.0, 2.5]);
}

/// Spacing and minor_count are host SSoT input — 0 (a live input
/// passing through mid-edit), a mis-typed magnitude, or a non-finite
/// range must neither blank the axis nor let a walker run away.
#[test]
fn tick_walkers_guard_hostile_spacing() {
    let mut axis = default_config().bottom_x.clone();
    axis.min = 0.0;
    axis.max = 10.0;
    axis.minor_count = 4;

    // Zero spacing falls back to the auto spacing, not a blank axis.
    axis.major_spacing = 0.0;
    let majors = major_tick_values(&axis);
    assert_eq!(majors, vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);

    // A tiny positive spacing would walk ~10^10 steps — capped to auto.
    axis.major_spacing = 1e-9;
    let majors = major_tick_values(&axis);
    assert!(
        !majors.is_empty() && majors.len() <= 12,
        "tiny spacing must cap to auto, got {} ticks",
        majors.len()
    );

    // Dense but sane custom spacing is honored verbatim.
    axis.major_spacing = 0.05;
    assert_eq!(major_tick_values(&axis).len(), 201);

    // Hostile minor_count stays bounded (and must not overflow).
    axis.major_spacing = 1.0;
    axis.minor_count = usize::MAX - 1;
    let minors = minor_tick_values(&axis);
    assert!(
        !minors.is_empty() && minors.len() <= 1_100,
        "minor walk must stay bounded, got {}",
        minors.len()
    );

    // Non-finite range ends: nothing to walk — `inf as i64` saturation
    // must never reach the loop bounds.
    axis.minor_count = 4;
    axis.max = f64::INFINITY;
    assert!(major_tick_values(&axis).is_empty());
    assert!(minor_tick_values(&axis).is_empty());
    axis.min = f64::NAN;
    axis.max = 10.0;
    assert!(major_tick_values(&axis).is_empty());
    assert!(minor_tick_values(&axis).is_empty());
}

#[test]
fn linear_tick_walkers_bound_every_finite_extreme_range() {
    let minimum_subnormal = f64::from_bits(1);
    let below_max = f64::from_bits(f64::MAX.to_bits() - 1);
    let mut axis = default_config().bottom_x.clone();
    axis.scale = AxisScale::Linear;
    axis.minor_count = usize::MAX;

    for span in [minimum_subnormal, f64::MAX] {
        let spacing = crate::tick::axis_major_spacing(&AxisScale::Linear, 0.0, span);
        assert!(spacing.is_finite() && spacing > 0.0);
    }

    for (min, max, requested_spacing) in [
        (0.0, minimum_subnormal, 0.0),
        (-f64::MAX, f64::MAX, 0.0),
        (below_max, f64::MAX, minimum_subnormal),
        (-f64::MAX, -below_max, minimum_subnormal),
    ] {
        axis.min = min;
        axis.max = max;
        axis.major_spacing = requested_spacing;

        let spacing = effective_major_spacing(&axis);
        assert!(
            spacing.is_finite() && spacing > 0.0,
            "invalid spacing {spacing} for {min:?}..{max:?}"
        );

        let majors = major_tick_values(&axis);
        assert!(majors.len() <= MAX_MAJOR_INTERVALS + 1, "{majors:?}");
        assert!(majors.iter().all(|value| value.is_finite()), "{majors:?}");
        assert!(
            majors.windows(2).all(|pair| pair[0] < pair[1]),
            "{majors:?}"
        );

        let minors = minor_tick_values(&axis);
        assert!(
            minors.len() <= MAX_MAJOR_INTERVALS * MAX_MINOR_SUBDIVISIONS + 1,
            "{} minor ticks for {min:?}..{max:?}",
            minors.len()
        );
        assert!(minors.iter().all(|value| value.is_finite()));
        assert!(minors.windows(2).all(|pair| pair[0] < pair[1]));
    }
}

#[test]
fn log_tick_walkers_guard_manual_ranges() {
    let close = |a: f64, b: f64| (a - b).abs() <= b.abs() * 1.0e-9 + 1.0e-30;
    let mut axis = default_config().bottom_x.clone();
    axis.scale = AxisScale::Logarithmic;
    axis.major_spacing = 1.0;
    axis.minor_count = 8;
    axis.min = 0.0;
    axis.max = 1000.0;

    let majors = major_tick_values(&axis);
    assert!(majors.iter().all(|v| v.is_finite() && *v > 0.0));
    assert!(close(majors[0], 1.0e-12), "{majors:?}");
    assert!(majors.iter().any(|v| close(*v, 1.0)), "{majors:?}");

    let minors = minor_tick_values(&axis);
    assert!(minors.iter().all(|v| v.is_finite() && *v > 0.0));
    assert!(minors.iter().any(|v| close(*v, 2.0e-12)), "{minors:?}");

    let da = DataArea(crate::layout::Rect {
        x: 10,
        y: 20,
        width: 100,
        height: 50,
    });
    let (x, _) = value_to_screen(1.0, &axis, Side::Bottom, &da);
    assert!(x.is_finite() && x > 10.0 && x < 110.0, "x={x}");

    axis.min = 1.0e-15;
    axis.max = 1.0e-12;
    let majors = major_tick_values(&axis);
    assert!(close(majors[0], 1.0e-15), "{majors:?}");
    assert!(close(*majors.last().unwrap(), 1.0e-12), "{majors:?}");

    axis.min = 10.0;
    axis.max = 1.0;
    let majors = major_tick_values(&axis);
    assert_eq!(majors.len(), 2, "{majors:?}");
    assert!(close(majors[0], 10.0), "{majors:?}");
    assert!(close(majors[1], 100.0), "{majors:?}");
}

#[test]
fn log_tick_walkers_bound_every_finite_extreme_range() {
    let mut axis = default_config().bottom_x.clone();
    axis.scale = AxisScale::Logarithmic;
    axis.minor_count = usize::MAX;

    for (min, max, requested_spacing) in [
        (f64::from_bits(1), f64::MAX, 1.0),
        (10.0, 100.0, f64::MAX),
        (f64::MAX, f64::MAX, f64::MAX),
        (f64::MAX, 1.0, f64::MAX),
        (0.0, f64::MAX, f64::NAN),
    ] {
        axis.min = min;
        axis.max = max;
        axis.major_spacing = requested_spacing;

        let (guarded_min, guarded_max) = crate::chart::guarded_log_range(min, max);
        assert!(guarded_min.is_finite() && guarded_min > 0.0);
        assert!(guarded_max.is_finite() && guarded_max > guarded_min);
        assert!(effective_log_decade_step(&axis) > 0);

        let majors = major_tick_values(&axis);
        assert!(majors.len() <= MAX_LOG_DECADES, "{majors:?}");
        assert!(
            majors.iter().all(|value| value.is_finite() && *value > 0.0),
            "{majors:?}"
        );
        assert!(
            majors.windows(2).all(|pair| pair[0] < pair[1]),
            "{majors:?}"
        );

        let minors = minor_tick_values(&axis);
        assert!(minors.len() <= MAX_LOG_DECADES * 8);
        assert!(minors.iter().all(|value| value.is_finite() && *value > 0.0));
        assert!(minors.windows(2).all(|pair| pair[0] < pair[1]));
    }
}

// Parity with the replaced code; removed with the old walkers in the follow-up PR.
//
// The tick plan moved to `crate::tick` (#58 item 14). These tests compare the
// renderer's remaining Vec walkers (on `grid::linear_grid_values` until
// `grid.rs` moves too) and the bounded families' inline walks with the
// model's over wide grids of inputs, and pin the differences the plan
// accepted. The comparison of the Cartesian spacing rule went with
// `chart::nice_spacing`; `tick.rs` keeps its golden tables.
mod tick_plan_parity {
    use super::*;
    use crate::tick as model_tick;

    fn ulps(a: f64, b: f64) -> u64 {
        if a == b {
            return 0;
        }
        if !(a.is_finite() && b.is_finite()) || a.is_sign_negative() != b.is_sign_negative() {
            return u64::MAX;
        }
        a.to_bits().abs_diff(b.to_bits())
    }

    /// Seeded xorshift64: the same inputs on every run.
    struct XorShift(u64);

    impl XorShift {
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        /// Uniform in [0, 1).
        fn unit(&mut self) -> f64 {
            (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    fn walker_axes() -> Vec<AxisOptions> {
        let below_max = f64::from_bits(f64::MAX.to_bits() - 1);
        let ranges: [(f64, f64); 18] = [
            (0.0, 10.0),
            (0.137, 9.42),
            (-3.7, 12.9),
            (1e-9, 3e-9),
            (1e9, 1e9 + 1000.0),
            (-1e300, 1e300),
            (-f64::MAX, f64::MAX),
            (0.0, f64::from_bits(1)),
            (below_max, f64::MAX),
            (10.0, 1.0),
            (f64::NAN, 1.0),
            (1.0, f64::INFINITY),
            (1e16, 1e16 + 4.0),
            (0.003, 250.0),
            (1e-300, 1e300),
            (f64::from_bits(1), f64::MAX),
            (0.0, 1000.0),
            (1e-320, 1e-300),
        ];
        let mut axes = Vec::new();
        for scale in [AxisScale::Linear, AxisScale::Logarithmic] {
            for (min, max) in ranges {
                let valid = model_tick::axis_major_spacing(&scale, min, max);
                for spacing in [
                    valid,
                    1.0,
                    2.7,
                    0.0,
                    -1.0,
                    f64::NAN,
                    f64::INFINITY,
                    1e-300,
                    1e300,
                    f64::MAX,
                ] {
                    for minor_count in [0, 1, 4, 9, 99, 100, 1000] {
                        let mut axis = default_config().bottom_x.clone();
                        axis.scale = scale.clone();
                        axis.min = min;
                        axis.max = max;
                        axis.major_spacing = spacing;
                        axis.minor_count = minor_count;
                        axes.push(axis);
                    }
                }
            }
        }
        axes
    }

    /// The model's walkers draw the renderer's ticks. Linear walks are
    /// bit-identical. Log decades use `powf` instead of `powi`
    /// (difference 5): values move by a few ulp, and only ranges that reach
    /// subnormal decades gain ticks.
    #[test]
    fn model_walkers_match_the_renderer_walkers() {
        let axes = walker_axes();
        let mut log_value_differences = 0usize;
        let mut subnormal_gains = 0usize;
        let mut worst_log = 0u64;
        for axis in &axes {
            let label = format!(
                "{:?} {:?}..{:?} spacing {:?} minors {}",
                axis.scale, axis.min, axis.max, axis.major_spacing, axis.minor_count
            );
            assert_eq!(
                walkable_range(axis),
                model_tick::walkable_range(axis),
                "{label}"
            );

            let old_majors = major_tick_values(axis);
            let log_range = crate::chart::guarded_log_range(axis.min, axis.max);
            let new_majors: Vec<f64> = model_tick::major_ticks(axis, log_range).collect();
            let old_minors = minor_tick_values(axis);
            let new_minors: Vec<f64> = model_tick::minor_ticks(axis, log_range).collect();
            match axis.scale {
                AxisScale::Linear => {
                    assert_eq!(old_majors, new_majors, "{label}");
                    assert_eq!(old_minors, new_minors, "{label}");
                }
                AxisScale::Logarithmic => {
                    for (old, new) in [(&old_majors, &new_majors), (&old_minors, &new_minors)] {
                        if old.len() != new.len() {
                            // `powi` loses the subnormal decades `powf` keeps.
                            assert!(new.len() > old.len(), "{label}");
                            assert!(new[0] < f64::MIN_POSITIVE, "{label}: {new:?}");
                            subnormal_gains += 1;
                            continue;
                        }
                        for (a, b) in old.iter().zip(new) {
                            let distance = ulps(*a, *b);
                            assert!(distance <= 16, "{label}: {a:?} vs {b:?}");
                            if distance > 0 {
                                log_value_differences += 1;
                                worst_log = worst_log.max(distance);
                            }
                        }
                    }
                }
            }
        }
        eprintln!(
            "walker parity: {} axes, linear walks equal; {log_value_differences} log values differ by 1..={worst_log} ulp; \
             {subnormal_gains} log walks gain subnormal decades",
            axes.len()
        );
    }

    /// Values for the `Plain` parity tests: both signs, every decade from 1e-12
    /// to 1e12, the exponent switch at 0.001 and 1e6 with their neighbours,
    /// rounding edges, ±0, NaN, ±inf and a seeded log-uniform sample.
    fn plain_parity_values() -> Vec<f64> {
        let mut values = vec![
            0.0,
            -0.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            999_999.999_999_6,
            999_999.999_999_4,
            0.000_999_999_6,
            0.000_999_999_4,
            0.000_000_5,
            2.5e-7,
            1_234.567_890_5,
            0.125,
            0.05,
            0.15,
            2.675,
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            f64::MAX,
        ];
        for edge in [0.001f64, 1e6] {
            for delta in 0..=3u64 {
                values.push(f64::from_bits(edge.to_bits() + delta));
                values.push(f64::from_bits(edge.to_bits() - delta));
            }
        }
        for exponent in -12..=12 {
            for mantissa in [
                "1",
                "1.5",
                "2.345",
                "5",
                "9.999995",
                "9.9999996",
                "9.99999949",
            ] {
                values.push(format!("{mantissa}e{exponent}").parse().unwrap());
            }
        }
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let unit = (seed >> 11) as f64 / (1u64 << 53) as f64;
            values.push(10f64.powf(-13.0 + unit * 26.0));
        }
        let negatives: Vec<f64> = values.iter().map(|v| -v).collect();
        values.extend(negatives);
        values
    }

    /// `PlainNumberFormat::AUTO` is the bounded axes' `plain_number`, which
    /// it replaces.
    #[test]
    fn plain_auto_matches_plain_number() {
        let values = plain_parity_values();
        for &value in &values {
            assert_eq!(
                crate::format::PlainNumberFormat::AUTO.format(value),
                plain_number(value),
                "{value:?}"
            );
        }
        eprintln!("plain number parity: {} values, all equal", values.len());
    }

    /// The bounded families' current inline walks (radar rings, box plot and
    /// categorical value ticks) against the `LinearGrid` walks that will
    /// replace them. Differences 3 (the grid's 1e-9 slack keeps end ticks)
    /// and 4 (categorical values become `k · step`) are counted, with the
    /// categorical labels that change.
    #[test]
    fn bounded_walks_match_linear_grid() {
        let mut rng = XorShift(0x2545_F491_4F6C_DD1D);
        // [user range, widened plan]
        let mut radar_gains = [0usize; 2];
        let mut boxplot_gains = [0usize; 2];
        let mut categorical_ticks = 0usize;
        let mut categorical_value_differences = 0usize;
        let mut categorical_label_differences = 0usize;
        let cases = 100_000;
        for case in 0..cases {
            let magnitude = 10f64.powf(-6.0 + rng.unit() * 12.0);
            let a = (rng.unit() - 0.5) * 2.0 * magnitude;
            let b = a + rng.unit() * magnitude + f64::EPSILON * a.abs().max(1.0);
            let Ok(plan) = AxisOptions::compute_nice_ticks(AxisScale::Linear, a, b, 5) else {
                continue;
            };
            let step = plan.major_spacing;

            // Radar rings over a user range with a user or planned spacing.
            let user_range = case % 2 == 0;
            let (min, max) = if user_range {
                (a, b)
            } else {
                (plan.min, plan.max)
            };
            let span = max - min;
            if span / step <= 100.0 {
                let first = (min / step).ceil() as i64;
                let last = (max / step).floor() as i64;
                let old: Vec<f64> = (first..=last)
                    .map(|k| k as f64 * step)
                    .filter(|v| *v > min + span * 1e-9)
                    .collect();
                let new: Vec<f64> = model_tick::LinearGrid::new(min, max, step, 100)
                    .into_iter()
                    .flat_map(|grid| grid.iter())
                    .map(|(_, v)| v)
                    .filter(|v| *v > min + span * 1e-9)
                    .collect();
                if old.len() != new.len() {
                    assert_eq!(new.len(), old.len() + 1, "{old:?} vs {new:?}");
                    radar_gains[usize::from(!user_range)] += 1;
                }
            }

            // Box plot ticks over a fixed range.
            let start = (min / step).ceil() * step;
            let count = ((max - start) / step).floor().max(0.0) as usize;
            if count <= 100 {
                let old: Vec<f64> = (0..=count).map(|i| start + i as f64 * step).collect();
                let new: Vec<f64> = model_tick::LinearGrid::new(min, max, step, 100)
                    .into_iter()
                    .flat_map(|grid| grid.iter())
                    .map(|(_, v)| v)
                    .collect();
                if old.len() != new.len() {
                    // The slack can keep a tick at either end.
                    assert!(
                        new.len() > old.len() && new.len() <= old.len() + 2,
                        "{min:?}..{max:?} by {step:?}: {old:?} vs {new:?}"
                    );
                    boxplot_gains[usize::from(!user_range)] += 1;
                }
            }

            // Categorical ticks over the widened plan.
            let tick_count = ((plan.max - plan.min) / step).round() as usize;
            let old: Vec<f64> = (0..=tick_count)
                .map(|i| plan.min + i as f64 * step)
                .collect();
            let new: Vec<f64> = model_tick::LinearGrid::new(plan.min, plan.max, step, 100)
                .unwrap()
                .iter()
                .map(|(_, v)| v)
                .collect();
            assert_eq!(old.len(), new.len(), "{plan:?}");
            categorical_ticks += old.len();
            for (x, y) in old.iter().zip(&new) {
                if x != y {
                    categorical_value_differences += 1;
                    assert!(
                        ulps(*x, *y) <= 1 || (x - y).abs() <= step * 1e-12,
                        "{x:?} vs {y:?}"
                    );
                }
                if plain_number(*x) != plain_number(*y) {
                    // A value on a rounding tie of the exponent form
                    // (`7.005e-5` as `7.00e-5` or `7.01e-5`).
                    categorical_label_differences += 1;
                }
            }
        }
        eprintln!(
            "bounded parity: {cases} ranges, half user ranges and half widened plans; \
             radar gains an outer ring in {radar_gains:?}, box plot gains end ticks in \
             {boxplot_gains:?}; {categorical_value_differences} of {categorical_ticks} \
             categorical tick values move, {categorical_label_differences} of their labels change"
        );
    }
}

/// The default config with a 0..10 bottom axis at `major_spacing`. Every
/// other tick setting draws as written.
fn spacing_config(major_spacing: f64) -> Config {
    let mut cfg = default_config();
    cfg.bottom_x.min = 0.0;
    cfg.bottom_x.max = 10.0;
    cfg.bottom_x.major_spacing = major_spacing;
    cfg
}

fn spacing_warning(requested: f64, used: f64, reason: SpacingRejection) -> RenderWarning {
    RenderWarning::MajorSpacingReplaced {
        axis: WarningAxis::BottomX,
        requested,
        used,
        reason,
    }
}

#[test]
fn a_replaced_major_spacing_warns_with_the_spacing_drawn() {
    let too_many = SpacingRejection::TooManyTicks {
        limit: MAX_MAJOR_INTERVALS,
    };
    for (requested, reason) in [
        (1e-9, too_many.clone()),
        // Positive, but the interval count overflows.
        (f64::from_bits(1), too_many),
        (0.0, SpacingRejection::NotPositive),
        (-2.0, SpacingRejection::NotPositive),
        (f64::INFINITY, SpacingRejection::NonFinite),
    ] {
        let cfg = spacing_config(requested);
        // The raster walks the automatic spacing for 0..10, which is 2.
        assert_eq!(
            major_tick_values(&cfg.bottom_x),
            [0.0, 2.0, 4.0, 6.0, 8.0, 10.0]
        );
        assert_eq!(
            tick_warnings(&cfg),
            [spacing_warning(requested, 2.0, reason)],
            "{requested:?}"
        );
    }

    // NaN never compares equal, so match on the warning instead.
    let warnings = tick_warnings(&spacing_config(f64::NAN));
    assert!(
        matches!(
            warnings.as_slice(),
            [RenderWarning::MajorSpacingReplaced {
                axis: WarningAxis::BottomX,
                requested,
                used: 2.0,
                reason: SpacingRejection::NonFinite,
            }] if requested.is_nan()
        ),
        "{warnings:?}"
    );

    // A quarter of an ulp at 1.0: few enough intervals, but
    // `min + spacing == min`, so the walk could not advance.
    let mut cfg = spacing_config(f64::EPSILON / 4.0);
    cfg.bottom_x.min = 1.0;
    cfg.bottom_x.max = 1.0 + 4.0 * f64::EPSILON;
    let used = effective_major_spacing(&cfg.bottom_x);
    assert_ne!(used, cfg.bottom_x.major_spacing);
    let majors = major_tick_values(&cfg.bottom_x);
    assert!(
        majors.windows(2).all(|pair| pair[1] - pair[0] == used),
        "{majors:?}"
    );
    assert_eq!(
        tick_warnings(&cfg),
        [spacing_warning(
            f64::EPSILON / 4.0,
            used,
            SpacingRejection::BelowResolution
        )]
    );
}

#[test]
fn tick_settings_drawn_as_written_raise_no_warning() {
    assert!(tick_warnings(&default_config()).is_empty());

    // The densest spacing the walker honours, the largest minor count it
    // honours, whole log decades and a drawn colourbar.
    let mut cfg = spacing_config(0.01);
    cfg.bottom_x.minor_count = MAX_MINOR_SUBDIVISIONS - 1;
    cfg.left_y.scale = AxisScale::Logarithmic;
    cfg.left_y.min = 1.0;
    cfg.left_y.max = 1.0e6;
    cfg.left_y.major_spacing = 3.0;
    cfg.colorbar = Some(crate::default::default_colorbar_options());
    assert_eq!(
        major_tick_values(&cfg.bottom_x).len(),
        MAX_MAJOR_INTERVALS + 1
    );
    assert!(tick_warnings(&cfg).is_empty(), "{:?}", tick_warnings(&cfg));
}

#[test]
fn an_oversized_minor_count_warns_with_the_count_drawn() {
    let mut cfg = spacing_config(2.0);
    cfg.bottom_x.minor_count = 500;
    assert_eq!(
        tick_warnings(&cfg),
        [RenderWarning::MinorCountClamped {
            axis: WarningAxis::BottomX,
            requested: 500,
            used: 99,
        }]
    );
    // 99 minors in each of the five major intervals is what is drawn.
    assert_eq!(minor_tick_values(&cfg.bottom_x).len(), 5 * 99);

    cfg.bottom_x.minor_count = usize::MAX;
    assert_eq!(
        tick_warnings(&cfg),
        [RenderWarning::MinorCountClamped {
            axis: WarningAxis::BottomX,
            requested: usize::MAX,
            used: 99,
        }]
    );

    // A log axis draws 2a..9a whatever the count, by design, not by guard.
    cfg.bottom_x.scale = AxisScale::Logarithmic;
    cfg.bottom_x.min = 1.0;
    cfg.bottom_x.major_spacing = 1.0;
    assert!(tick_warnings(&cfg).is_empty());
}

#[test]
fn a_log_decade_step_that_is_not_whole_warns_with_the_step_drawn() {
    let mut cfg = default_config();
    cfg.left_y.scale = AxisScale::Logarithmic;
    cfg.left_y.min = 1.0;
    cfg.left_y.max = 1.0e6;
    let log_warning = |requested: f64, used: f64| RenderWarning::LogDecadeStepAdjusted {
        axis: WarningAxis::LeftY,
        requested,
        used,
    };

    cfg.left_y.major_spacing = 2.5;
    assert_eq!(tick_warnings(&cfg), [log_warning(2.5, 2.0)]);
    assert_eq!(major_tick_values(&cfg.left_y), [1.0, 1.0e2, 1.0e4, 1.0e6]);

    for requested in [0.5, 0.0, -3.0, f64::INFINITY] {
        cfg.left_y.major_spacing = requested;
        assert_eq!(tick_warnings(&cfg), [log_warning(requested, 1.0)]);
    }
    cfg.left_y.major_spacing = 1.0e12;
    assert_eq!(
        tick_warnings(&cfg),
        [log_warning(1.0e12, f64::from(i32::MAX))]
    );
    cfg.left_y.major_spacing = f64::NAN;
    assert!(matches!(
        tick_warnings(&cfg).as_slice(),
        [RenderWarning::LogDecadeStepAdjusted { used: 1.0, .. }]
    ));

    cfg.left_y.major_spacing = 2.0;
    assert!(tick_warnings(&cfg).is_empty());
}

#[test]
fn calendar_ticks_walk_no_spacing_and_raise_no_warning() {
    use crate::format::{TimestampLabelFormat, TimestampTickPolicy};

    // One hour of Unix seconds with a stale spacing and minor count.
    let mut cfg = spacing_config(1e-9);
    cfg.bottom_x.max = 3_600.0;
    cfg.bottom_x.minor_count = 500;
    cfg.bottom_x.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat::default());
    let da = cfg.data_area().unwrap();
    let fp = FontPolicy::for_style(&cfg.draw_style);
    assert!(timestamp_tick_plan(&cfg.bottom_x, Side::Bottom, &da, fp).is_some());
    assert!(tick_warnings(&cfg).is_empty(), "{:?}", tick_warnings(&cfg));

    // The numeric policy walks both settings, so both are reported.
    cfg.bottom_x.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat {
        tick_policy: TimestampTickPolicy::NumericSpacing,
        ..TimestampLabelFormat::default()
    });
    let warned: Vec<_> = tick_warnings(&cfg)
        .iter()
        .filter_map(RenderWarning::axis)
        .collect();
    assert_eq!(warned, [WarningAxis::BottomX, WarningAxis::BottomX]);

    // Calendar ticks the planner cannot fit under its tick limit fall back
    // to the numeric walker, which is then reported like any other axis.
    cfg.bottom_x.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat::default());
    cfg.bottom_x.min = -3.0e13;
    cfg.bottom_x.max = 3.0e13;
    cfg.bottom_x.minor_count = 4;
    cfg.bottom_x.major_spacing = 0.0;
    assert!(timestamp_tick_plan(&cfg.bottom_x, Side::Bottom, &da, fp).is_none());
    let used = effective_major_spacing(&cfg.bottom_x);
    assert_eq!(
        tick_warnings(&cfg),
        [spacing_warning(0.0, used, SpacingRejection::NotPositive)]
    );
}

#[test]
fn every_axis_the_decoration_layer_walks_is_reported() {
    // draw_axis walks all four axes whatever their tick and label
    // visibility, so hidden ticks still report.
    let mut cfg = default_config();
    for axis in [
        &mut cfg.bottom_x,
        &mut cfg.top_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.major_spacing = 0.0;
        axis.tick = TickVisibility::None;
        axis.label_style.visible = false;
    }
    let warned: Vec<_> = tick_warnings(&cfg)
        .iter()
        .filter_map(RenderWarning::axis)
        .collect();
    assert_eq!(
        warned,
        [
            WarningAxis::BottomX,
            WarningAxis::TopX,
            WarningAxis::LeftY,
            WarningAxis::RightY
        ]
    );

    // The colourbar's axis is reported while the bar is drawn.
    let mut cfg = default_config();
    let mut bar = crate::default::default_colorbar_options();
    bar.axis.major_spacing = 1e-9;
    cfg.colorbar = Some(bar);
    let bar_axis = &cfg.colorbar.as_ref().unwrap().axis;
    assert_eq!(
        tick_warnings(&cfg),
        [RenderWarning::MajorSpacingReplaced {
            axis: WarningAxis::ColorBar,
            requested: 1e-9,
            used: effective_major_spacing(bar_axis),
            reason: SpacingRejection::TooManyTicks {
                limit: MAX_MAJOR_INTERVALS,
            },
        }]
    );
    cfg.colorbar.as_mut().unwrap().visible = false;
    assert!(tick_warnings(&cfg).is_empty());
}

#[test]
fn warnings_follow_the_raster_frame_of_an_offset_chart_area() {
    let drawn = |cfg: &Config| {
        try_raster_chart_layer_to_rgba(cfg, AxisLayerKind::Decoration)
            .unwrap()
            .chunks_exact(4)
            .any(|px| px[3] != 0)
    };
    let mut cfg = spacing_config(0.0);
    cfg.chart_area = crate::layout::ChartArea(crate::layout::Rect {
        x: 40,
        y: 30,
        width: 320,
        height: 240,
    });
    assert!(drawn(&cfg));
    assert_eq!(
        tick_warnings(&cfg),
        [spacing_warning(0.0, 2.0, SpacingRejection::NotPositive)]
    );

    // A negative left margin keeps the data area on the surface at this
    // offset, but in the raster's own frame it would start left of the
    // origin: the raster draws no axes, and nothing is reported.
    cfg.left_y.out_margin -= cfg.margins().left + 20.0;
    assert_eq!(cfg.margins().left, -20.0);
    assert!(cfg.data_area().is_ok());
    assert!(!drawn(&cfg));
    assert!(tick_warnings(&cfg).is_empty());

    // No raster at all for an empty chart area, and no warnings.
    cfg.chart_area.0.width = 0;
    assert!(try_raster_chart_layer_to_rgba(&cfg, AxisLayerKind::Decoration).is_err());
    assert!(tick_warnings(&cfg).is_empty());
}

#[test]
fn warning_messages_name_the_setting_and_both_values_on_one_line() {
    for (warning, parts) in [
        (
            spacing_warning(1e-9, 2.0, SpacingRejection::TooManyTicks { limit: 1000 }),
            ["bottom_x.major_spacing", "1e-9", "2.0"],
        ),
        (
            RenderWarning::MinorCountClamped {
                axis: WarningAxis::TopX,
                requested: 500,
                used: 99,
            },
            ["top_x.minor_count", "500", "99"],
        ),
        (
            RenderWarning::LogDecadeStepAdjusted {
                axis: WarningAxis::ColorBar,
                requested: 0.5,
                used: 1.0,
            },
            ["colorbar.axis.major_spacing", "0.5", "1-decade"],
        ),
        (
            // Whole but past the cap: the line states the full range.
            RenderWarning::LogDecadeStepAdjusted {
                axis: WarningAxis::LeftY,
                requested: 3.0e9,
                used: f64::from(MAX_LOG_DECADE_STEP),
            },
            ["left_y.major_spacing", "3000000000.0", "2147483647-decade"],
        ),
    ] {
        let message = warning.to_string();
        assert!(!message.contains('\n'), "{message}");
        for part in parts {
            assert!(message.contains(part), "{message:?} lacks {part:?}");
        }
    }
}

/// One axis, one decimal form: every tick label shares the spacing-derived
/// decimal count instead of per-value significant digits ("0 / 50.0 /
/// 100" mixing was the bug).
#[test]
fn tick_decimals_follow_spacing_uniformly() {
    assert_eq!(decimals_from_spacing(50.0), 0);
    assert_eq!(decimals_from_spacing(0.5), 1);
    assert_eq!(decimals_from_spacing(0.02), 2);
    assert_eq!(decimals_from_spacing(2.5), 0);
    assert_eq!(decimals_from_spacing(1.3128152), 0);
    assert_eq!(decimals_from_spacing(0.025), 2);

    let f = |v: f64, sp: f64| {
        format_tick_value(
            v,
            &LabelFormat::Decimal,
            3,
            &AxisScale::Linear,
            sp,
            0.0,
            150.0,
        )
    };
    assert_eq!(f(50.0, 50.0), "50");
    assert_eq!(f(100.0, 50.0), "100");
    assert_eq!(f(150.0, 50.0), "150");
    assert_eq!(f(-0.5, 0.5), "-0.5");
    assert_eq!(f(1.5, 0.5), "1.5");
}

#[test]
fn numeric_labels_do_not_collide_on_small_scaled_or_inverted_axes() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let da = DataArea(crate::layout::Rect {
        x: 60,
        y: 20,
        width: 220,
        height: 160,
    });
    for fp in [FontPolicy::Standard, FontPolicy::Handwritten] {
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            for inverted in [false, true] {
                for (scale, format, min, max, spacing) in [
                    (AxisScale::Linear, LabelFormat::Decimal, -1e6, 1e6, 1e5),
                    (
                        AxisScale::Linear,
                        LabelFormat::Scientific,
                        -1e-6,
                        1e-6,
                        1e-7,
                    ),
                    (AxisScale::Linear, LabelFormat::Power, -1e6, 1e6, 1e5),
                    (AxisScale::Logarithmic, LabelFormat::Power, 1e-9, 1e9, 1.0),
                ] {
                    let mut axis = default_config().bottom_x;
                    axis.scale = scale;
                    axis.label_style.format = format;
                    axis.label_style.font_size = 24.0;
                    axis.min = min;
                    axis.max = max;
                    axis.major_spacing = spacing;
                    axis.inverted = inverted;
                    let original = axis.clone();
                    let majors = major_tick_values(&axis);
                    let labels =
                        visible_numeric_labels(&axis, side.clone(), fp, &majors, |value| {
                            value_to_screen(value, &axis, side.clone(), &da)
                        });
                    assert!(
                        labels.len() >= 2 && labels.len() < majors.len(),
                        "crowded axes must retain useful, separated labels"
                    );
                    let mut intervals: Vec<_> = labels
                        .iter()
                        .map(|label| {
                            let b = label.bounds;
                            match side {
                                Side::Top | Side::Bottom => (b.x, b.x + b.width),
                                Side::Left | Side::Right => (b.y, b.y + b.height),
                            }
                        })
                        .collect();
                    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
                    for pair in intervals.windows(2) {
                        assert!(
                            pair[1].0 - pair[0].1 >= 12.0 - 1e-4,
                            "numeric glyph bounds must leave half an em of space"
                        );
                    }
                    let distances: Vec<_> = labels
                        .windows(2)
                        .map(|pair| match side {
                            Side::Top | Side::Bottom => {
                                (pair[1].position.0 - pair[0].position.0).abs()
                            }
                            Side::Left | Side::Right => {
                                (pair[1].position.1 - pair[0].position.1).abs()
                            }
                        })
                        .collect();
                    assert!(
                        distances.iter().all(|d| (d - distances[0]).abs() < 0.01),
                        "label spacing must be regular, including logarithmic axes"
                    );
                    assert_eq!(
                        axis, original,
                        "display density must not rewrite requested ticks or font size"
                    );
                }
            }
        }
    }
}

#[test]
fn numeric_label_density_recovers_on_resize_zoom_and_font_changes() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut axis = default_config().bottom_x;
    axis.min = 0.0;
    axis.max = 1e6;
    axis.major_spacing = 1e5;
    axis.label_style.font_size = 24.0;
    let labels = |axis: &AxisOptions, width| {
        let da = DataArea(crate::layout::Rect {
            x: 60,
            y: 20,
            width,
            height: 160,
        });
        visible_numeric_labels(
            axis,
            Side::Bottom,
            FontPolicy::Standard,
            &major_tick_values(axis),
            |value| value_to_screen(value, axis, Side::Bottom, &da),
        )
    };
    let crowded = labels(&axis, 220);
    let wide = labels(&axis, 2000);
    assert_eq!(
        wide.len(),
        major_tick_values(&axis).len(),
        "wide axes restore every label"
    );
    assert!(wide.len() > crowded.len());
    let mut smaller_font = axis.clone();
    smaller_font.label_style.font_size = 10.0;
    assert!(labels(&smaller_font, 220).len() > crowded.len());
    let mut zoomed = axis.clone();
    zoomed.max = 2e5;
    assert_eq!(
        labels(&zoomed, 400).len(),
        3,
        "zoom restores labels without requiring another fit"
    );
    let mut hidpi = axis.clone();
    hidpi.label_style.font_size *= 2.0;
    hidpi.major_tick_length *= 2.0;
    let scaled = labels(&hidpi, 440);
    assert_eq!(
        scaled.len(),
        crowded.len(),
        "HiDPI scales glyphs and gaps together"
    );
}

#[test]
fn decimal_significant_digits_control_uniform_precision() {
    let f = |digits| {
        format_tick_value(
            1.0,
            &LabelFormat::Decimal,
            digits,
            &AxisScale::Linear,
            1.0,
            0.0,
            10.0,
        )
    };
    assert_eq!(f(1), "1");
    assert_eq!(f(3), "1.0");
    assert_eq!(
        format_tick_value(
            0.5,
            &LabelFormat::Decimal,
            1,
            &AxisScale::Linear,
            0.5,
            0.0,
            1.0,
        ),
        "0.5",
        "spacing remains a floor so adjacent values cannot collapse"
    );
}

#[test]
fn arbitrary_fit_spacing_respects_requested_precision() {
    let format = |value, digits| {
        format_tick_value(
            value,
            &LabelFormat::Decimal,
            digits,
            &AxisScale::Linear,
            1.3128152,
            0.0,
            6.564076,
        )
    };
    assert_eq!(format(6.564076, 2), "6.6");
    assert_eq!(format(6.564076, 3), "6.56");
    assert_eq!(format(5.2512608, 3), "5.25");
    assert_eq!(format(6.564076, 7), "6.564076");
    for step in [0.025, 0.5, 1.3128152, 2.5] {
        let labels: Vec<_> = (0..6)
            .map(|i| {
                format_tick_value(
                    i as f64 * step,
                    &LabelFormat::Decimal,
                    1,
                    &AxisScale::Linear,
                    step,
                    0.0,
                    step * 5.0,
                )
            })
            .collect();
        assert!(
            labels.windows(2).all(|pair| pair[0] != pair[1]),
            "{step}: {labels:?}"
        );
    }
    let (config, mut labels) = contour_label_fixture();
    labels.significant_digits = 3;
    assert_eq!(
        contour_label_texts(&config, &labels, &[0.0, 1.3128152, 2.6256304]),
        ["0", "1.31", "2.63"]
    );
}

#[test]
fn logarithmic_labels_respect_precision_without_losing_tiny_values() {
    let format = |value, digits, style| {
        format_tick_value(
            value,
            &style,
            digits,
            &AxisScale::Logarithmic,
            1.0,
            1e-30,
            1e30,
        )
    };
    assert_eq!(format(3.162277660168, 2, LabelFormat::Decimal), "3.2");
    assert_eq!(format(3.162277660168, 4, LabelFormat::Decimal), "3.162");
    assert_eq!(
        format(3.162277660168, 4, LabelFormat::Scientific),
        "3.162e0"
    );
    for value in [1e-30, 0.01, 0.1, 1.0, 10.0, 1e30] {
        assert_eq!(format(value, 3, LabelFormat::Decimal), value.to_string());
    }
    assert_eq!(
        format(f64::MAX, 2, LabelFormat::Decimal)
            .parse::<f64>()
            .unwrap(),
        f64::MAX
    );
    let tiny = format(3.162277660168e-30, 2, LabelFormat::Decimal);
    assert_eq!(tiny.parse::<f64>().unwrap(), 3.2e-30);
}

/// `LabelFormat::Plain` reads the value alone: the same text on a linear and
/// a logarithmic axis, whatever the spacing, range and significant digits.
#[test]
fn plain_labels_ignore_scale_spacing_and_digits() {
    for plain in [
        crate::format::PlainNumberFormat::AUTO,
        crate::format::PlainNumberFormat {
            decimals: Some(2),
            trim_zeros: false,
        },
        crate::format::PlainNumberFormat {
            decimals: Some(2),
            trim_zeros: true,
        },
    ] {
        let format = LabelFormat::Plain(plain);
        for value in [0.0, 2.5, 1000.0, 1.5e7, 3e-4, -12.25] {
            let expected = plain.format(value);
            for (scale, spacing, min, max, digits) in [
                (AxisScale::Linear, 0.5, 0.0, 10.0, 3),
                (AxisScale::Linear, 1e-9, -1e9, 1e9, 15),
                (AxisScale::Logarithmic, 1.0, 1e-30, 1e30, 1),
            ] {
                assert_eq!(
                    format_tick_value(value, &format, digits, &scale, spacing, min, max),
                    expected,
                    "{plain:?} {value:?} on {scale:?}"
                );
            }
        }
    }
}

#[test]
fn fitted_numeric_labels_fit_default_title_and_colorbar_bands() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = default_config();
    config.left_y.title_option.text.segments = "Positive samples (log)"
        .chars()
        .map(RichSegment::plain)
        .collect();
    config.left_y.min = 0.0;
    config.left_y.max = 6.564076;
    config.left_y.major_spacing = 1.3128152;
    config.colorbar = Some(crate::default::default_colorbar_options());
    config.colorbar.as_mut().unwrap().axis.min = 0.0;
    config.colorbar.as_mut().unwrap().axis.max = 6.564076;
    config.colorbar.as_mut().unwrap().axis.major_spacing = 1.3128152;
    for scale in [1.0, 2.0] {
        let config = config.scaled(scale);
        let da = config.data_area().unwrap();
        let fp = FontPolicy::Standard;
        let axis = &config.left_y;
        let metrics = measure_rich_text(&axis.title_option.text, fp);
        let title = axis_title_placement(
            Side::Left,
            &config.chart_area,
            &da,
            TitleBand {
                out_margin: axis.out_margin,
                chart_title_margin: config.chart_title.top_margin,
                edge_inset: config.colorbar_band(&Side::Left),
            },
            (0.0, 0.0),
            metrics,
        )
        .rect(metrics);
        let labels =
            visible_numeric_labels(axis, Side::Left, fp, &major_tick_values(axis), |value| {
                value_to_screen(value, axis, Side::Left, &da)
            });
        let bounds = |label: &NumericTickLabel, axis: &AxisOptions, side| {
            let NumericTickText::Plain(text) = &label.text else {
                panic!("decimal label")
            };
            let ls = &axis.label_style;
            let metrics = measure_plain_text(text, &ls.label_font, ls.font_size, false, false, fp);
            crate::layout::label_rect(
                label_origin(
                    side,
                    label.position,
                    axis.major_tick_length,
                    (ls.label_offset_x, ls.label_offset_y),
                    metrics,
                ),
                metrics,
            )
        };
        for label in &labels {
            let rect = bounds(label, axis, Side::Left);
            assert!(
                rect.x > title.x + title.width,
                "label/title overlap at scale {scale}: label {rect:?}, title {title:?}"
            );
        }
        let bar = config.colorbar.as_ref().unwrap();
        let strip = strip_rect(&config);
        let labels = visible_numeric_labels(
            &bar.axis,
            Side::Right,
            fp,
            &major_tick_values(&bar.axis),
            |value| {
                point_on_rect_side(
                    ((value - bar.axis.min) / (bar.axis.max - bar.axis.min)) as f32,
                    &Side::Right,
                    &strip,
                )
            },
        );
        for label in &labels {
            let rect = bounds(label, &bar.axis, Side::Right);
            assert!(
                rect.x + rect.width <= (config.chart_area.0.x + config.chart_area.0.width) as f32,
                "colorbar label clipped at scale {scale}: {rect:?}"
            );
        }
    }
}

#[test]
fn contour_decimal_format_uses_level_spacing_not_bottom_x() {
    let (mut config, mut labels) = contour_label_fixture();
    config.bottom_x.major_spacing = 2.0;
    config.colorbar = Some(crate::default::default_colorbar_options());
    labels.significant_digits = 1;
    let texts = contour_label_texts(&config, &labels, &[-1.0, -0.5, 0.0, 0.5, 1.0]);
    assert_eq!(texts, ["-1.0", "-0.5", "0", "0.5", "1.0"]);
}

/// `raster_chart_to_rgba` returns a buffer of the expected size and is
/// not entirely empty. Checking that anything was actually drawn would
/// require visual inspection, but a non-zero pixel proves the path ran.
#[test]
fn raster_produces_expected_size_and_nonzero() {
    let config = default_config();
    let w = config.chart_area.0.width as usize;
    let h = config.chart_area.0.height as usize;

    let rgba = raster_chart_to_rgba(&config);
    assert_eq!(rgba.len(), w * h * 4, "buffer size must be w*h*4");

    // Background is transparent, so any axis/label pixel must lift alpha above 0.
    let any_opaque = rgba.chunks_exact(4).any(|px| px[3] > 0);
    assert!(
        any_opaque,
        "expected at least some non-transparent pixel (axes drawn)"
    );
}

/// Selection-blue (b dominant over r and g) — axis chrome is black/gray
/// (r == g == b), so a blue-dominant pixel can only come from the
/// selection overlay.
fn has_selection_blue(rgba: &[u8]) -> bool {
    rgba.chunks_exact(4)
        .any(|px| px[3] > 0 && px[2] > px[0].saturating_add(40) && px[2] > px[1].saturating_add(30))
}

#[test]
fn selection_overlay_rasters_blue_box() {
    use crate::select::{DataAreaElement, SelectionGeometry};
    use crate::text_render::CpuTextMeasure;

    let config = default_config();
    let sel = DataAreaElement
        .selection_box(&config, &CpuTextMeasure::for_style(&config.draw_style))
        .expect("data area selection box");

    let plain = try_raster_chart_layer_to_rgba(&config, AxisLayerKind::Decoration).unwrap();
    assert!(
        !has_selection_blue(&plain),
        "no blue-dominant pixels expected without a selection overlay"
    );

    let selected =
        try_raster_chart_layer_to_rgba_with_selection(&config, AxisLayerKind::Decoration, &[sel])
            .unwrap();
    assert!(
        has_selection_blue(&selected),
        "selection overlay must contribute blue-dominant pixels"
    );
}

/// The grid layer sits below the data — selection must never draw there.
#[test]
fn selection_overlay_skips_grid_layer() {
    use crate::select::{DataAreaElement, SelectionGeometry};
    use crate::text_render::CpuTextMeasure;

    let config = default_config();
    let sel = DataAreaElement
        .selection_box(&config, &CpuTextMeasure::for_style(&config.draw_style))
        .unwrap();
    let grid = try_raster_chart_layer_to_rgba_with_selection(&config, AxisLayerKind::Grid, &[sel])
        .unwrap();
    assert!(!has_selection_blue(&grid));
}

// Sketch (hand-drawn) mode — CPU deco layer only, no GPU adapter needed.

/// Sketch-mode test config: legend and minor grid switched on so the
/// sketched legend border and the dashed (Dot) minor grid lines are
/// exercised alongside axis lines, ticks, and major grid lines.
fn sketch_test_config(seed: u32) -> Config {
    let mut cfg = default_config();
    cfg.legend.visible = true;
    cfg.legend.content = RichText::plain("series A", Color::BLACK, 14.0, "");
    cfg.grid.show_minor_x = true;
    cfg.grid.show_minor_y = true;
    cfg.draw_style = DrawStyle::Sketch(SketchOptions {
        seed,
        ..SketchOptions::default()
    });
    cfg
}

/// (a) Divergence: enabling sketch mode changes both deco rasters.
#[test]
fn sketch_mode_diverges_from_precise() {
    let sketched = sketch_test_config(0);
    let mut precise = sketched.clone();
    precise.draw_style = DrawStyle::Precise;
    for layer in [AxisLayerKind::Grid, AxisLayerKind::Decoration] {
        let a = try_raster_chart_layer_to_rgba(&precise, layer).unwrap();
        let b = try_raster_chart_layer_to_rgba(&sketched, layer).unwrap();
        assert_ne!(
            a, b,
            "{layer:?} raster must change when sketch mode is enabled"
        );
    }
}

/// (b) Determinism: identical sketch config twice → byte-identical raster.
#[test]
fn sketch_mode_is_deterministic() {
    let cfg = sketch_test_config(0);
    for layer in [AxisLayerKind::Grid, AxisLayerKind::Decoration] {
        let a = try_raster_chart_layer_to_rgba(&cfg, layer).unwrap();
        let b = try_raster_chart_layer_to_rgba(&cfg, layer).unwrap();
        assert_eq!(a, b, "{layer:?} sketch raster must be deterministic");
    }
}

/// (c) Seed separation: seed 0 vs seed 1 → different wobble pixels.
#[test]
fn sketch_seed_changes_raster() {
    for layer in [AxisLayerKind::Grid, AxisLayerKind::Decoration] {
        let a = try_raster_chart_layer_to_rgba(&sketch_test_config(0), layer).unwrap();
        let b = try_raster_chart_layer_to_rgba(&sketch_test_config(1), layer).unwrap();
        assert_ne!(a, b, "{layer:?} raster must depend on the sketch seed");
    }
}

/// The selection overlay never wobbles. The box is the data area's real
/// selection box, 2 px outside the data-area border. The axis chrome along
/// that border (lines, ticks, labels, grid) is switched off, so the box sits
/// over pixels that are fully transparent in both modes (the sketched legend
/// sits inside the top-right corner), and its rendered pixels — footprint
/// AND values — must be byte-identical whether sketch mode is on or off.
#[test]
fn selection_overlay_stays_precise_in_sketch_mode() {
    use crate::select::{DataAreaElement, SelectionGeometry};
    use crate::text_render::CpuTextMeasure;

    let mut sketched = sketch_test_config(0);
    for axis in [
        &mut sketched.top_x,
        &mut sketched.bottom_x,
        &mut sketched.left_y,
        &mut sketched.right_y,
    ] {
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.label_style.visible = false;
    }
    sketched.grid.show_major_x = false;
    sketched.grid.show_major_y = false;
    sketched.grid.show_minor_x = false;
    sketched.grid.show_minor_y = false;
    let mut precise = sketched.clone();
    precise.draw_style = DrawStyle::Precise;

    let sel = DataAreaElement
        .selection_box(&precise, &CpuTextMeasure::for_style(&precise.draw_style))
        .expect("the data area is selectable");
    assert_eq!(
        Some(sel.rect()),
        DataAreaElement
            .selection_box(&sketched, &CpuTextMeasure::for_style(&sketched.draw_style))
            .map(|b| b.rect()),
        "the data area box does not depend on the draw style"
    );

    let layer = AxisLayerKind::Decoration;
    let base_p = try_raster_chart_layer_to_rgba(&precise, layer).unwrap();
    let with_p =
        try_raster_chart_layer_to_rgba_with_selection(&precise, layer, std::slice::from_ref(&sel))
            .unwrap();
    let base_s = try_raster_chart_layer_to_rgba(&sketched, layer).unwrap();
    let with_s = try_raster_chart_layer_to_rgba_with_selection(&sketched, layer, &[sel]).unwrap();

    // Bytes the selection overlay touched (with vs without selection).
    let footprint = |base: &[u8], with: &[u8]| -> Vec<usize> {
        base.iter()
            .zip(with)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| i)
            .collect()
    };
    let fp_p = footprint(&base_p, &with_p);
    let fp_s = footprint(&base_s, &with_s);
    assert!(!fp_p.is_empty(), "selection overlay must draw something");
    assert_eq!(
        fp_p, fp_s,
        "selection footprint must not move in sketch mode"
    );
    for &i in &fp_p {
        assert_eq!(
            with_p[i], with_s[i],
            "selection ink must be byte-identical in sketch mode (byte {i})"
        );
    }
}

// ── Colourbar ──────────────────────────────────────────────────────────

const BAR_W: u32 = 480;
const BAR_H: u32 = 360;

/// A chart whose only chrome of interest is the colourbar. The chart area
/// starts at the origin because the raster entry shifts it there anyway, so
/// the rect computed here is the rect that was drawn.
fn colorbar_config(side: Side) -> Config {
    let mut cfg = default_config();
    cfg.chart_area = crate::layout::ChartArea(crate::layout::Rect {
        x: 0,
        y: 0,
        width: BAR_W,
        height: BAR_H,
    });
    cfg.chart_title.visible = false;
    let mut bar = crate::default::default_colorbar_options();
    bar.side = side;
    bar.length_frac = 1.0;
    // No border and no axis line: the strip's interior is then pure ramp,
    // so a sampled pixel is the ramp's answer and nothing else.
    bar.border_width = 0.0;
    bar.axis.min = 0.0;
    bar.axis.max = 100.0;
    bar.axis.major_spacing = 25.0;
    cfg.colorbar = Some(bar);
    cfg
}

/// `colorbar_config` on the given scale: the linear bar as is, or a log
/// bar over five decades. Bands are even in screen space on either scale,
/// so the strip's pixels must not depend on it.
fn colorbar_config_on(side: Side, scale: &AxisScale) -> Config {
    let mut cfg = colorbar_config(side);
    if *scale == AxisScale::Logarithmic {
        let axis = &mut cfg.colorbar.as_mut().expect("colourbar").axis;
        axis.scale = AxisScale::Logarithmic;
        axis.min = 1.0e-2;
        axis.max = 1.0e3;
        axis.major_spacing = 1.0;
    }
    cfg
}

const BAR_SCALES: [AxisScale; 2] = [AxisScale::Linear, AxisScale::Logarithmic];

fn strip_rect(cfg: &Config) -> RectF {
    let da = cfg.data_area().expect("data area");
    colorbar_rect(
        &cfg.chart_area,
        &da,
        cfg.chart_title.top_margin,
        cfg.colorbar.as_ref().expect("colourbar"),
    )
}

fn raster(cfg: &Config) -> Vec<u8> {
    try_raster_chart_to_rgba(cfg).expect("colourbar raster")
}

fn pixel_at(rgba: &[u8], x: f32, y: f32) -> [u8; 4] {
    let (px, py) = (x.floor() as u32, y.floor() as u32);
    assert!(px < BAR_W && py < BAR_H, "sample ({x}, {y}) is off-canvas");
    let i = ((py * BAR_W + px) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

fn magenta_pixels(rgba: &[u8], rect: RectF) -> usize {
    let x0 = rect.x.floor().max(0.0) as u32;
    let y0 = rect.y.floor().max(0.0) as u32;
    let x1 = (rect.x + rect.width).ceil().min(BAR_W as f32) as u32;
    let y1 = (rect.y + rect.height).ceil().min(BAR_H as f32) as u32;
    let mut count = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = pixel_at(rgba, x as f32, y as f32);
            if p[0] > 180 && p[1] < 100 && p[2] > 180 && p[3] > 180 {
                count += 1;
            }
        }
    }
    count
}

fn assert_close(found: [u8; 4], want: Color, tolerance: i32, what: &str) {
    let expected = [
        (want.r * 255.0).round() as i32,
        (want.g * 255.0).round() as i32,
        (want.b * 255.0).round() as i32,
        (want.a * 255.0).round() as i32,
    ];
    for channel in 0..4 {
        let delta = found[channel] as i32 - expected[channel];
        assert!(
            delta.abs() <= tolerance,
            "{what}: channel {channel} is {} , expected {} (+-{tolerance}); \
             found {found:?} want {expected:?}",
            found[channel],
            expected[channel]
        );
    }
}

/// The strip runs from the ramp's low end at `axis.min` to its high end at
/// `axis.max`, and the screen direction follows the side: a vertical bar has
/// min at the bottom, like a y axis.
///
/// Tolerance is 3/255: a band's colour is sampled at its centre, so the
/// outermost band is half a band short of t = 0 / t = 1.
#[test]
fn the_colorbar_strip_runs_from_min_to_max_along_its_side() {
    for side in [Side::Right, Side::Left] {
        let cfg = colorbar_config(side.clone());
        let bar = cfg.colorbar.clone().expect("colourbar");
        let rect = strip_rect(&cfg);
        let rgba = raster(&cfg);
        let mid_x = rect.x + rect.width * 0.5;

        assert_close(
            pixel_at(&rgba, mid_x, rect.y + 0.5),
            bar.colormap.sample(1.0),
            3,
            &format!("{side:?} top of a vertical bar is axis.max"),
        );
        assert_close(
            pixel_at(&rgba, mid_x, rect.y + rect.height - 0.5),
            bar.colormap.sample(0.0),
            3,
            &format!("{side:?} bottom of a vertical bar is axis.min"),
        );
    }

    for side in [Side::Top, Side::Bottom] {
        let cfg = colorbar_config(side.clone());
        let bar = cfg.colorbar.clone().expect("colourbar");
        let rect = strip_rect(&cfg);
        let rgba = raster(&cfg);
        let mid_y = rect.y + rect.height * 0.5;

        assert_close(
            pixel_at(&rgba, rect.x + 0.5, mid_y),
            bar.colormap.sample(0.0),
            3,
            &format!("{side:?} left of a horizontal bar is axis.min"),
        );
        assert_close(
            pixel_at(&rgba, rect.x + rect.width - 0.5, mid_y),
            bar.colormap.sample(1.0),
            3,
            &format!("{side:?} right of a horizontal bar is axis.max"),
        );
    }
}

/// The strip's interior pixels along its centre line, walked from the
/// `axis.min` end. Both outer end pixels are skipped: the strip's own ends
/// are anti-aliased against the background, as any edge is.
fn strip_centre_line(cfg: &Config) -> Vec<[u8; 4]> {
    let bar = cfg.colorbar.as_ref().expect("colourbar");
    let rect = strip_rect(cfg);
    let rgba = raster(cfg);
    let vertical = matches!(bar.side, Side::Left | Side::Right);
    let (lo, hi) = if vertical {
        (
            rect.y.ceil() as u32 + 1,
            (rect.y + rect.height).floor() as u32 - 1,
        )
    } else {
        (
            rect.x.ceil() as u32 + 1,
            (rect.x + rect.width).floor() as u32 - 1,
        )
    };
    let along: Vec<u32> = if vertical {
        (lo..hi).rev().collect()
    } else {
        (lo..hi).collect()
    };
    along
        .into_iter()
        .map(|pos| {
            if vertical {
                pixel_at(&rgba, rect.x + rect.width * 0.5, pos as f32 + 0.5)
            } else {
                pixel_at(&rgba, pos as f32 + 0.5, rect.y + rect.height * 0.5)
            }
        })
        .collect()
}

/// Every interior pixel of a strip carries exactly the band's own colour,
/// whatever the strip's sub-pixel phase. With band edges at fractional
/// positions, each shared edge pixel was two partial anti-aliased covers
/// whose composite stayed translucent, and the drifting phase of those
/// edges showed as plateaus with jumps at about a third and two thirds of
/// a grayscale bar. A fractional length and a fractional start (an offset
/// of half a pixel) both produce that phase.
#[test]
fn a_strip_at_any_sub_pixel_phase_is_opaque_and_monotonic() {
    for scale in &BAR_SCALES {
        for side in [Side::Right, Side::Top] {
            for (length_frac, half_pixel_offset) in
                [(0.75, false), (0.6137, false), (0.9, true), (1.0, true)]
            {
                let mut cfg = colorbar_config_on(side.clone(), scale);
                let bar = cfg.colorbar.as_mut().expect("colourbar");
                bar.length_frac = length_frac;
                bar.colormap = crate::colormap::ColorMap::GrayScale;
                if half_pixel_offset {
                    bar.offset_x = 0.5;
                    bar.offset_y = 0.5;
                }
                let mut previous = 0u8;
                for (i, p) in strip_centre_line(&cfg).into_iter().enumerate() {
                    let what = format!(
                        "{scale:?} {side:?} frac {length_frac} offset {half_pixel_offset} px {i}"
                    );
                    assert_eq!(p[3], 255, "{what}: translucent {p:?}");
                    assert!(
                        p[0] >= previous,
                        "{what}: gray falls {previous} -> {}",
                        p[0]
                    );
                    previous = p[0];
                }
            }
        }
    }
}

/// A translucent ramp keeps its own alpha in the strip, as the GPU field
/// keeps it in a cell: no pixel is painted by two bands, so none comes out
/// more opaque than the stop it samples, including at the first band.
#[test]
fn a_translucent_ramp_keeps_its_alpha_in_the_strip() {
    for scale in &BAR_SCALES {
        for side in [Side::Right, Side::Top] {
            let mut cfg = colorbar_config_on(side.clone(), scale);
            let bar = cfg.colorbar.as_mut().expect("colourbar");
            bar.length_frac = 0.75;
            bar.offset_x = 0.5;
            bar.offset_y = 0.5;
            bar.colormap = crate::colormap::ColorMap::Custom {
                stops: vec![
                    Color::new(0.0, 0.0, 0.0, 0.5),
                    Color::new(1.0, 1.0, 1.0, 0.5),
                ],
            };
            let mut previous = 0u8;
            for (i, p) in strip_centre_line(&cfg).into_iter().enumerate() {
                assert!(
                    (p[3] as i32 - 128).abs() <= 1,
                    "{scale:?} {side:?} px {i}: alpha {} is not the stop's 128",
                    p[3]
                );
                // Premultiplied at a constant alpha, the gray still rises.
                assert!(
                    p[0] >= previous,
                    "{scale:?} {side:?} px {i}: gray falls {previous} -> {}",
                    p[0]
                );
                previous = p[0];
            }
        }
    }
}

/// The strip's long side edges are covered once along their whole length:
/// at a fractional cross position the edge column's partial coverage is
/// the same for every band rather than heavier where bands overlap.
#[test]
fn a_strip_side_edge_has_one_coverage_along_its_length() {
    for scale in &BAR_SCALES {
        let mut cfg = colorbar_config_on(Side::Right, scale);
        let bar = cfg.colorbar.as_mut().expect("colourbar");
        bar.length_frac = 0.75;
        bar.offset_x = 0.5;
        bar.colormap = crate::colormap::ColorMap::GrayScale;
        let rect = strip_rect(&cfg);
        assert_ne!(
            rect.x.fract(),
            0.0,
            "{scale:?}: the probe needs a fractional side edge"
        );
        let rgba = raster(&cfg);
        let column = rect.x.floor();
        let rows = (rect.y.ceil() as u32 + 1)..((rect.y + rect.height).floor() as u32 - 1);
        let alphas: Vec<u8> = rows
            .map(|y| pixel_at(&rgba, column + 0.5, y as f32 + 0.5)[3])
            .collect();
        let (min, max) = (alphas.iter().min().unwrap(), alphas.iter().max().unwrap());
        assert!(
            max - min <= 1,
            "{scale:?}: side-edge coverage varies along the strip: {min}..{max}"
        );
    }
}

/// The §B.5 pin: the colour drawn where a value's tick lands is the colour
/// that value's z normalization asks for. The strip and the tick positions
/// come from two functions with deliberately different out-of-range
/// behaviour (`normalized_z` clamps and rejects, `axis_fraction` does
/// neither), and in range they must agree exactly — that agreement is what
/// the GPU field sampler will also be held to.
#[test]
fn colorbar_ticks_match_the_strip_ramp() {
    for (scale, min, max, probes) in [
        (
            AxisScale::Linear,
            0.0f64,
            100.0f64,
            vec![0.0, 12.5, 25.0, 50.0, 99.0, 100.0],
        ),
        (
            AxisScale::Logarithmic,
            1.0e-3,
            1.0e3,
            vec![1.0e-3, 1.0e-2, 1.0, 1.0e2, 1.0e3],
        ),
    ] {
        for inverted in [false, true] {
            let mut cfg = colorbar_config(Side::Right);
            {
                let bar = cfg.colorbar.as_mut().expect("colourbar");
                bar.axis.scale = scale.clone();
                bar.axis.min = min;
                bar.axis.max = max;
                bar.axis.inverted = inverted;
            }
            let bar = cfg.colorbar.as_ref().expect("colourbar");
            for z in &probes {
                let normalized = bar
                    .normalized_z(*z)
                    .unwrap_or_else(|| panic!("{z} is inside [{min}, {max}]"));
                let mut on_screen = axis_fraction(*z, &bar.axis);
                if inverted {
                    on_screen = 1.0 - on_screen;
                }
                assert!(
                    (normalized - on_screen).abs() < 1e-6,
                    "{scale:?} inverted={inverted} z={z}: colour t {normalized} vs \
                     position t {on_screen}"
                );
            }
            // Out of range, the two part ways on purpose.
            assert_eq!(bar.normalized_z(f64::NAN), None);
            assert_eq!(bar.normalized_z(max * 10.0), Some(1.0));
        }
    }
}

/// `inverted` moves where a value is drawn. It does not reverse the ramp:
/// the same z keeps the same colour, at the other end of the bar.
#[test]
fn an_inverted_colorbar_axis_swaps_the_ends_and_not_the_ramp() {
    let mut cfg = colorbar_config(Side::Right);
    cfg.colorbar.as_mut().expect("colourbar").axis.inverted = true;
    let bar = cfg.colorbar.clone().expect("colourbar");
    let rect = strip_rect(&cfg);
    let rgba = raster(&cfg);
    let mid_x = rect.x + rect.width * 0.5;

    // Inverted: axis.max is now at the bottom, so t = 1 is the bottom.
    assert_close(
        pixel_at(&rgba, mid_x, rect.y + 0.5),
        bar.colormap.sample(0.0),
        3,
        "inverted top is axis.min's colour",
    );
    assert_close(
        pixel_at(&rgba, mid_x, rect.y + rect.height - 0.5),
        bar.colormap.sample(1.0),
        3,
        "inverted bottom is axis.max's colour",
    );
    // The colour a value has did not change — only where it sits.
    assert_eq!(bar.normalized_z(bar.axis.max), Some(1.0));
}

/// `visible: false` is byte-identical to having no colourbar at all: the
/// band was never reserved, so there is nothing drawn and nothing blank.
#[test]
fn a_hidden_colorbar_draws_exactly_nothing() {
    let mut hidden = colorbar_config(Side::Right);
    hidden.colorbar.as_mut().expect("colourbar").visible = false;
    let mut absent = hidden.clone();
    absent.colorbar = None;

    assert_eq!(raster(&hidden), raster(&absent));

    // And a visible one does change the picture — otherwise the comparison
    // above would pass for the wrong reason.
    let shown = colorbar_config(Side::Right);
    assert_ne!(raster(&shown), raster(&absent));
}

/// The colourbar's ticks and labels are the axis helpers, so a logarithmic
/// bar gets decade majors and 10ⁿ labels with no second implementation.
#[test]
fn a_logarithmic_colorbar_ticks_by_decade() {
    let mut cfg = colorbar_config(Side::Right);
    {
        let bar = cfg.colorbar.as_mut().expect("colourbar");
        bar.axis.scale = AxisScale::Logarithmic;
        bar.axis.min = 1.0e-2;
        bar.axis.max = 1.0e2;
        bar.axis.major_spacing = 1.0;
        bar.axis.label_style.format = LabelFormat::Power;
    }
    let bar = cfg.colorbar.as_ref().expect("colourbar");
    let majors = major_tick_values(&bar.axis);
    assert_eq!(majors, vec![1.0e-2, 1.0e-1, 1.0, 1.0e1, 1.0e2]);

    // The Power label is a RichText with a superscript exponent, which is
    // what `format_tick_power` exists to produce.
    let label = format_tick_power(1.0e-2, 3, &bar.axis.label_style);
    assert!(
        label.segments.iter().any(|seg| seg.superscript),
        "a decade label must carry a superscript exponent: {label:?}"
    );

    // Ink lands in the label margin outside the strip.
    let rect = strip_rect(&cfg);
    let rgba = raster(&cfg);
    let label_band_x = rect.x + rect.width + bar.axis.major_tick_length + 2.0;
    let inked = (0..BAR_H).any(|y| {
        (0..(BAR_W - label_band_x as u32))
            .any(|dx| pixel_at(&rgba, label_band_x + dx as f32, y as f32)[3] != 0)
    });
    assert!(inked, "no tick label ink outside the strip");
}

#[test]
fn colorbar_ticks_honor_direction_and_configured_line_style() {
    let configured = |tick, style| {
        let mut cfg = colorbar_config(Side::Right);
        let bar = cfg.colorbar.as_mut().expect("colourbar");
        bar.axis.min = 0.0;
        bar.axis.max = 1.0;
        bar.axis.major_spacing = 0.5;
        bar.axis.minor_count = 0;
        bar.axis.tick = tick;
        bar.axis.major_tick_length = 12.0;
        bar.axis.line_width = 2.0;
        bar.axis.line_color = Color::from_rgb8(255, 0, 255);
        bar.axis.line_style = style;
        bar.axis.line_visible = false;
        bar.axis.label_style.visible = false;
        cfg
    };

    let outside = configured(TickVisibility::Outside, LineStylePreset::Solid);
    let outside_rect = strip_rect(&outside);
    let outside_rgba = raster(&outside);
    let middle_y = outside_rect.y + outside_rect.height * 0.5;
    let outside_probe = RectF {
        x: outside_rect.x + outside_rect.width + 1.0,
        y: middle_y - 2.0,
        width: 10.0,
        height: 4.0,
    };
    let inside_probe = RectF {
        x: outside_rect.x + outside_rect.width - 11.0,
        y: middle_y - 2.0,
        width: 10.0,
        height: 4.0,
    };
    assert!(magenta_pixels(&outside_rgba, outside_probe) > 0);
    assert_eq!(magenta_pixels(&outside_rgba, inside_probe), 0);

    let inside = configured(TickVisibility::Inside, LineStylePreset::Solid);
    let inside_rgba = raster(&inside);
    assert!(magenta_pixels(&inside_rgba, inside_probe) > 0);
    assert_eq!(magenta_pixels(&inside_rgba, outside_probe), 0);

    let dotted = configured(TickVisibility::Outside, LineStylePreset::ShortDot);
    let dotted_count = magenta_pixels(&raster(&dotted), outside_probe);
    let solid_count = magenta_pixels(&outside_rgba, outside_probe);
    assert!(
        dotted_count < solid_count,
        "tick dash style was ignored: dotted={dotted_count}, solid={solid_count}"
    );
}

#[test]
fn detached_colorbar_axis_paint_stays_inside_its_selection_bounds() {
    use crate::select::{ColorBarAxisElement, SelectionGeometry};
    use crate::text_render::CpuTextMeasure;

    let mut cfg = colorbar_config(Side::Right);
    {
        let bar = cfg.colorbar.as_mut().expect("colourbar");
        bar.axis.min = 0.0;
        bar.axis.max = 1.0;
        bar.axis.major_spacing = 0.5;
        bar.axis.minor_count = 0;
        bar.axis.tick = TickVisibility::Outside;
        bar.axis.major_tick_length = 12.0;
        bar.axis.line_width = 2.0;
        bar.axis.line_color = Color::from_rgb8(255, 0, 255);
        bar.axis.line_visible = false;
        bar.axis.label_style.visible = false;
        bar.axis.line_offset = -9.0;
    }
    let bounds = ColorBarAxisElement
        .bounds(&cfg, &CpuTextMeasure::for_style(&cfg.draw_style))
        .expect("axis selection bounds");
    assert!(
        magenta_pixels(&raster(&cfg), bounds) > 0,
        "detached tick paint and selection bounds diverged"
    );
}

/// A colourbar and a visible axis on the same side must not be drawn on top
/// of each other: the axis band hugs the data area and the bar is outside
/// it, so the axis' outermost ink stays clear of the strip.
#[test]
fn a_colorbar_does_not_cover_the_axis_labels_on_its_side() {
    let mut cfg = colorbar_config(Side::Right);
    cfg.right_y.label_style.visible = true;
    cfg.right_y.label_style.label_visible = true;
    cfg.right_y.out_margin = 90.0;
    let da = cfg.data_area().expect("data area");
    let rect = strip_rect(&cfg);

    let axis_reach = (da.x + da.width) as f32 + cfg.right_y.major_tick_length + LABEL_GAP;
    assert!(
        rect.x > axis_reach,
        "strip starts at {} but the axis' ticks reach {axis_reach}",
        rect.x
    );
    // The whole axis label band fits before the strip.
    assert!(
        rect.x >= (da.x + da.width) as f32 + cfg.right_y.out_margin,
        "strip at {} overlaps the {} px axis label margin",
        rect.x,
        cfg.right_y.out_margin
    );
}

/// A degenerate strip is skipped rather than drawn as a zero-area rect or a
/// division by zero in the band walk.
#[test]
fn a_degenerate_colorbar_is_skipped() {
    for (frac, thickness) in [(0.0, 18.0), (0.5, 0.0)] {
        let mut cfg = colorbar_config(Side::Right);
        {
            let bar = cfg.colorbar.as_mut().expect("colourbar");
            bar.length_frac = frac;
            bar.thickness_px = thickness;
        }
        // Draws without panicking, and paints no strip.
        let rgba = raster(&cfg);
        assert_eq!(rgba.len(), (BAR_W * BAR_H * 4) as usize);
    }
}

/// The offset moves what is painted, not just what is selectable: the strip
/// is drawn at `ColorBarElement`'s bounds, so a drag lands the highlight box
/// and the handles on the bar the user sees.
#[test]
fn a_colorbar_offset_moves_the_painted_strip() {
    use crate::select::{ColorBarElement, SelectionGeometry};
    use crate::text_render::CpuTextMeasure;

    let mut cfg = colorbar_config(Side::Right);
    let base = strip_rect(&cfg);
    {
        let bar = cfg.colorbar.as_mut().expect("colourbar");
        bar.offset_x = -40.0;
        bar.offset_y = 12.0;
    }
    let moved = strip_rect(&cfg);
    assert_eq!(moved.x, base.x - 40.0);
    assert_eq!(moved.y, base.y + 12.0);

    // Painted where the offset says, and nowhere else.
    let bar = cfg.colorbar.clone().expect("colourbar");
    let rgba = raster(&cfg);
    assert_close(
        pixel_at(&rgba, moved.x + moved.width * 0.5, moved.y + 0.5),
        bar.colormap.sample(1.0),
        3,
        "the strip's high end moved with the offset",
    );
    assert_eq!(
        pixel_at(&rgba, base.x + base.width * 0.5, base.y + base.height * 0.5)[3],
        0,
        "nothing is painted at the un-offset position"
    );

    // Interaction bounds are the same rect the paint used — no second
    // geometry to drift.
    assert_eq!(
        ColorBarElement.bounds(&cfg, &CpuTextMeasure::for_style(&cfg.draw_style)),
        Some(moved)
    );
}

/// A polar tick-label selection boxes every label where it is drawn and
/// leaves the ring alone, instead of one box around the whole chart.
#[test]
fn a_polar_tick_label_selection_boxes_each_label() {
    use crate::layout::{DrawnPart, Element};
    use crate::select::{AxisLabelElement, SELECTION_PADDING, SelectionGeometry, SelectionOutline};
    use crate::sink::Parts;
    use crate::text::MeasureText;
    use crate::text_render::CpuTextMeasure;

    let mut cfg = default_config();
    cfg.polar = Some(crate::polar::PolarOptions::default());
    let mut chart = crate::Chart::new(cfg);
    chart.set_x_range(0.0, 360.0);
    chart.set_y_range(0.0, 3.0);
    let cfg = chart.config().clone();
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let width = cfg.chart_area.0.width as usize;

    // Polar angle labels are the bottom labels, radius labels the left ones.
    for side in [Side::Bottom, Side::Left] {
        let mut labels = Vec::new();
        assert!(measure.emit(
            &cfg,
            &Element::AxisLabel(side.clone()),
            &mut Parts(&mut |part| if let DrawnPart::Text(r) = part {
                labels.push(r)
            }),
        ));
        assert!(labels.len() >= 4, "{side:?}: {} labels", labels.len());

        // The real selection box: its rect is the whole-ring box a plain
        // outline would draw, its outline the labels themselves.
        let sel = AxisLabelElement { side: side.clone() }
            .selection_box(&cfg, &measure)
            .expect("polar tick labels are selectable");
        let ring = sel.rect();
        match sel.outline() {
            SelectionOutline::Parts(parts) => assert_eq!(
                parts.len(),
                labels.len(),
                "{side:?}: the outline holds every drawn label"
            ),
            other => panic!("{side:?}: polar tick labels outline their parts, got {other:?}"),
        }
        let layer = AxisLayerKind::Decoration;
        let base = try_raster_chart_layer_to_rgba(&cfg, layer).unwrap();
        let with = try_raster_chart_layer_to_rgba_with_selection(&cfg, layer, &[sel]).unwrap();
        let touched = |x: f32, y: f32| {
            let i = (y.round() as usize * width + x.round() as usize) * 4;
            base[i..i + 4] != with[i..i + 4]
        };

        for label in &labels {
            let r = label.expanded(SELECTION_PADDING);
            assert!(
                r.width * r.height < ring.width * ring.height * 0.25,
                "{side:?}: a label box is not the ring: {r:?} in {ring:?}"
            );
            assert!(
                touched(r.x + r.width * 0.5, r.y),
                "{side:?}: the top edge of {r:?} is outlined"
            );
        }
        // No ring-sized box: its corners stay as the base raster drew them.
        for (x, y) in [
            (ring.x, ring.y),
            (ring.x + ring.width, ring.y + ring.height),
        ] {
            let off = labels
                .iter()
                .all(|l| !l.expanded(SELECTION_PADDING + 2.0).contains(x, y));
            if off {
                assert!(
                    !touched(x, y),
                    "{side:?}: ring corner ({x}, {y}) is outlined"
                );
            }
        }
    }
}

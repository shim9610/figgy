//! Unit tests of the series configuration types.

use super::*;

#[cfg(test)]
mod vector_style_tests {
    use super::*;

    fn style(scale: VectorScale) -> VectorStyle {
        VectorStyle {
            color: Color::BLACK,
            width_px: 1.5,
            scale,
            pivot: VectorPivot::Tail,
            head_length_px: 8.0,
            head_width_px: 6.0,
            min_length_px: 0.0,
            color_map: None,
        }
    }

    #[test]
    fn validation_rejects_bad_pixels_and_scales() {
        assert_eq!(style(VectorScale::default()).validate(), Ok(()));
        let mut bad = style(VectorScale::default());
        bad.width_px = -1.0;
        assert_eq!(bad.validate(), Err(VectorStyle::INVALID_PIXELS_REASON));
        let mut bad = style(VectorScale::default());
        bad.head_width_px = f32::NAN;
        assert_eq!(bad.validate(), Err(VectorStyle::INVALID_PIXELS_REASON));
        let bad = style(VectorScale::Data { factor: 0.0 });
        assert_eq!(bad.validate(), Err(VectorStyle::INVALID_SCALE_REASON));
        let bad = style(VectorScale::Pixels {
            px_per_unit: f64::INFINITY,
        });
        assert_eq!(bad.validate(), Err(VectorStyle::INVALID_SCALE_REASON));
        // A negative data factor flips every arrow; it is a choice, not an error.
        assert_eq!(style(VectorScale::Data { factor: -2.0 }).validate(), Ok(()));
    }

    #[test]
    fn fit_scale_targets_the_longest_vector_in_either_mode() {
        let mut pixels = style(VectorScale::Pixels { px_per_unit: 1.0 });
        // The longest possible vector is (|u| = 3, |v| = 4): length 5.
        assert!(pixels.fit_scale((-3.0, 1.0), (0.0, 4.0), 40.0));
        assert_eq!(pixels.scale, VectorScale::Pixels { px_per_unit: 8.0 });
        let mut data = style(VectorScale::Data { factor: 1.0 });
        assert!(data.fit_scale((-3.0, 1.0), (0.0, 4.0), 0.5));
        assert_eq!(data.scale, VectorScale::Data { factor: 0.1 });
    }

    /// The 3D fit (#211) takes the longest vector over three components.
    #[test]
    fn fit_scale_3d_targets_the_longest_vector_in_3d() {
        let mut pixels = style(VectorScale::Pixels { px_per_unit: 1.0 });
        // |u| = 2, |v| = 3, |w| = 6: length 7.
        assert!(pixels.fit_scale_3d((-2.0, 1.0), (0.0, 3.0), (-6.0, 5.0), 35.0));
        assert_eq!(pixels.scale, VectorScale::Pixels { px_per_unit: 5.0 });
        let mut data = style(VectorScale::Data { factor: 1.0 });
        assert!(data.fit_scale_3d((-2.0, 1.0), (0.0, 3.0), (-6.0, 5.0), 14.0));
        assert_eq!(data.scale, VectorScale::Data { factor: 2.0 });
        assert!(!data.fit_scale_3d((0.0, 0.0), (0.0, 0.0), (0.0, 0.0), 1.0));
        assert!(!data.fit_scale_3d((-1.0, 1.0), (0.0, 1.0), (f64::NAN, 1.0), 1.0));
        assert!(!data.fit_scale_3d((-1.0, 1.0), (0.0, 1.0), (0.0, 1.0), -1.0));
        assert_eq!(data.scale, VectorScale::Data { factor: 2.0 });
    }

    #[test]
    fn fit_scale_leaves_the_style_alone_for_empty_or_invalid_bounds() {
        let mut style = style(VectorScale::Pixels { px_per_unit: 3.0 });
        assert!(!style.fit_scale((0.0, 0.0), (0.0, 0.0), 40.0));
        assert!(!style.fit_scale((f64::NAN, 1.0), (0.0, 1.0), 40.0));
        assert!(!style.fit_scale((-1.0, 1.0), (-1.0, 1.0), 0.0));
        assert_eq!(style.scale, VectorScale::Pixels { px_per_unit: 3.0 });
    }
}

#[cfg(test)]
mod point_size_map_tests {
    use super::*;

    fn map(scale: SizeScale) -> PointSizeMap {
        PointSizeMap {
            column: "s".into(),
            domain_min: 0.0,
            domain_max: 4.0,
            min_px: 2.0,
            max_px: 10.0,
            scale,
        }
    }

    #[test]
    fn radius_scaling_clamps_to_the_pixel_range() {
        let radius = map(SizeScale::Radius);
        assert_eq!(radius.radius_px(0.0), Some(2.0));
        assert_eq!(radius.radius_px(2.0), Some(6.0));
        assert_eq!(radius.radius_px(4.0), Some(10.0));
        assert_eq!(radius.radius_px(-5.0), Some(2.0));
        assert_eq!(radius.radius_px(99.0), Some(10.0));
        assert_eq!(radius.radius_px(f64::NAN), None);
        assert_eq!(radius.radius_px(f64::INFINITY), None);
    }

    #[test]
    fn area_scaling_is_linear_in_area() {
        let area = map(SizeScale::Area);
        let lo = 2.0f32 * 2.0;
        let hi = 10.0f32 * 10.0;
        let mid = area.radius_px(2.0).unwrap();
        assert!((mid * mid - (lo + hi) / 2.0).abs() < 1e-4);
        assert_eq!(area.radius_px(0.0), Some(2.0));
        assert_eq!(area.radius_px(4.0), Some(10.0));
    }

    #[test]
    fn validation_rejects_bad_domains_and_pixel_ranges() {
        assert_eq!(map(SizeScale::Area).validate(), Ok(()));
        let mut bad = map(SizeScale::Area);
        bad.domain_max = bad.domain_min;
        assert_eq!(bad.validate(), Err(PointSizeMap::INVALID_DOMAIN_REASON));
        let mut bad = map(SizeScale::Area);
        bad.domain_min = f64::NAN;
        assert_eq!(bad.validate(), Err(PointSizeMap::INVALID_DOMAIN_REASON));
        let mut bad = map(SizeScale::Area);
        bad.min_px = 11.0;
        assert_eq!(bad.validate(), Err(PointSizeMap::INVALID_PIXELS_REASON));
        let mut bad = map(SizeScale::Area);
        bad.min_px = -1.0;
        assert_eq!(bad.validate(), Err(PointSizeMap::INVALID_PIXELS_REASON));
        let mut bad = map(SizeScale::Area);
        bad.max_px = f32::INFINITY;
        assert_eq!(bad.validate(), Err(PointSizeMap::INVALID_PIXELS_REASON));
    }

    #[test]
    fn fit_domain_writes_only_valid_ranges() {
        let mut m = map(SizeScale::Area);
        assert!(m.fit_domain(-3.0, 7.0));
        assert_eq!((m.domain_min, m.domain_max), (-3.0, 7.0));
        assert!(!m.fit_domain(5.0, 5.0));
        assert!(!m.fit_domain(f64::NAN, 1.0));
        assert_eq!((m.domain_min, m.domain_max), (-3.0, 7.0));
    }
}

#[cfg(test)]
mod matrix_tests {
    use super::*;

    fn matrix(
        orientation: MatrixOrientation,
        grid_layout: GridLayout,
        columns: usize,
    ) -> MatrixRef {
        MatrixRef {
            columns: (0..columns).map(|i| format!("z{i}")).collect(),
            orientation,
            grid_layout,
        }
    }

    // The declared shape, matching the data exactly: nothing is trimmed.
    #[test]
    fn a_well_formed_grid_uses_everything_it_declares() {
        let centers = matrix(MatrixOrientation::ColumnsAreX, GridLayout::Centers, 10);
        assert_eq!(centers.effective_extent(10, 5, 5), (10, 5));

        // Edges bound cells, so the coordinate columns are one longer.
        let edges = matrix(MatrixOrientation::ColumnsAreX, GridLayout::Edges, 10);
        assert_eq!(edges.effective_extent(11, 6, 5), (10, 5));
    }

    // A mismatch is drawn to the smallest common extent and reported. It
    // never errors, and it never draws past the data.
    #[test]
    fn a_mismatched_grid_shrinks_to_the_smallest_common_extent() {
        let m = matrix(MatrixOrientation::ColumnsAreX, GridLayout::Centers, 10);
        // Fewer x coordinates than constituent columns.
        assert_eq!(m.effective_extent(4, 5, 5), (4, 5));
        // A short constituent column limits the depth.
        assert_eq!(m.effective_extent(10, 5, 2), (10, 2));
        // Fewer y coordinates than the columns are deep.
        assert_eq!(m.effective_extent(10, 3, 5), (10, 3));
        // Nothing at all: zero cells, still not an error.
        assert_eq!(m.effective_extent(0, 0, 0), (0, 0));
    }

    // `Edges` with a single coordinate bounds no cell. Saturating, not wrapping:
    // `0 - 1` as a usize would ask for 18 quintillion cells.
    #[test]
    fn one_edge_bounds_no_cell() {
        let m = matrix(MatrixOrientation::ColumnsAreX, GridLayout::Edges, 10);
        assert_eq!(m.effective_extent(1, 1, 5), (0, 0));
        assert_eq!(m.effective_extent(0, 0, 5), (0, 0));
    }

    // Orientation swaps which coordinate column bounds which direction. It is
    // the declaration that decides — never the lengths.
    #[test]
    fn orientation_decides_which_coordinate_bounds_the_columns() {
        let columns_are_x = matrix(MatrixOrientation::ColumnsAreX, GridLayout::Centers, 10);
        let columns_are_y = matrix(MatrixOrientation::ColumnsAreY, GridLayout::Centers, 10);
        // x = 4, y = 20: one reads 4 usable columns, the other 10.
        assert_eq!(columns_are_x.effective_extent(4, 20, 20), (4, 20));
        assert_eq!(columns_are_y.effective_extent(4, 20, 20), (10, 4));
    }
}

// The wire form of a series: every field added after a release is additive
// schema, absent at its default so a document written before it parses and
// re-serializes byte-identically, and round-trips when set.
//
// One test runs every check in turn (#47 §3), including the family modules'
// round trips below, which stay beside their fixtures: the renderer's
// `released_documents.rs` holds the fixture documents, so this module keeps
// the model's own coverage without a copy of them per field. Each check is a
// plain function so a failure names it.
#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::{
        DataBarStyleConfig, DataErrorBarPointStyleConfig, DataErrorBarPointStyleOverride,
        DataErrorBarStyleConfig, DataRenderType, DataScatterPointStyleConfig,
        DataScatterPointStyleOverride, DataScatterStyleConfig, GridLayout, MatrixOrientation,
        MatrixRef, PointColorMap, ScatterShape, SeriesConfig, SizeScale, VectorColorMap,
        VectorComponents, VectorPivot, VectorScale, VectorStyle,
    };
    use crate::color::Color;

    fn base_series_json() -> serde_json::Value {
        serde_json::json!({
            "series_id": "s",
            "label": null,
            "x_column": "x",
            "y_column": "y",
            "render_type": {
                "Scatter": {
                    "scatter": {
                        "point_color": { "r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0 },
                        "point_shape": "Circle",
                        "point_size": 3.0
                    }
                }
            }
        })
    }

    fn old_series_json_without_additive_fields_still_parses() {
        let cfg: SeriesConfig =
            serde_json::from_value(base_series_json()).expect("old series shape parses");
        assert_eq!(cfg.source_id, None);
        let DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(scatter),
            line: None,
            errors: None,
            ..
        }) = &cfg.render_type
        else {
            panic!("expected scatter");
        };
        assert_eq!(scatter.point_style_table, None);
        assert_eq!(scatter.point_style_index_column, None);
        assert_eq!(scatter.point_style_overrides, None);
        assert_eq!(scatter.point_color_map, None);
        assert_eq!(scatter.point_size_map, None);

        let json = serde_json::to_value(cfg.clone()).expect("serialize");
        assert!(json.get("source_id").is_none());
        // The height column (#132) is omitted while unset and round-trips.
        assert!(json.get("z_column").is_none());
        let mut tall = cfg;
        tall.z_column = Some("z".into());
        let json_tall = serde_json::to_value(&tall).expect("serialize");
        assert_eq!(json_tall["z_column"], "z");
        assert_eq!(
            serde_json::from_value::<SeriesConfig>(json_tall).expect("parse"),
            tall
        );
        let scatter = &json["render_type"]["Scatter"]["scatter"];
        assert!(scatter.get("point_style_table").is_none());
        assert!(scatter.get("point_style_index_column").is_none());
        assert!(scatter.get("point_style_overrides").is_none());
        assert!(scatter.get("point_color_map").is_none());
        assert!(scatter.get("point_size_map").is_none());
        assert_eq!(json, base_series_json());
    }

    fn value_maps_round_trip_and_default_the_size_scale() {
        let mut json = base_series_json();
        let scatter = &mut json["render_type"]["Scatter"]["scatter"];
        scatter["point_color_map"] = serde_json::json!({ "column": "c" });
        scatter["point_size_map"] = serde_json::json!({
            "column": "s",
            "domain_min": 0.0,
            "domain_max": 10.0,
            "min_px": 2.0,
            "max_px": 12.0
        });
        let cfg: SeriesConfig = serde_json::from_value(json).expect("maps parse");
        let DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(scatter),
            line: None,
            errors: None,
            ..
        }) = &cfg.render_type
        else {
            panic!("expected scatter");
        };
        assert_eq!(
            scatter.point_color_map,
            Some(PointColorMap { column: "c".into() })
        );
        let size = scatter.point_size_map.as_ref().expect("size map");
        assert_eq!(size.scale, SizeScale::Area);

        let back: SeriesConfig =
            serde_json::from_value(serde_json::to_value(&cfg).expect("serialize"))
                .expect("round trip");
        assert_eq!(back, cfg);
    }

    fn point_style_mapping_round_trips_with_flattened_override() {
        let scatter = DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::Circle,
            point_size: 3.0,
            point_style_table: Some(vec![
                DataScatterPointStyleConfig {
                    point_color: Some(Color::from_rgb8(255, 0, 0)),
                    point_shape: None,
                    point_size: Some(5.0),
                },
                DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::DiamondFilled),
                    point_size: None,
                },
            ]),
            point_style_index_column: Some("style_idx".into()),
            point_style_overrides: Some(vec![DataScatterPointStyleOverride {
                index: 7,
                style: DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::StarFilled),
                    point_size: Some(9.0),
                },
            }]),
            point_color_map: None,
            point_size_map: None,
        };

        let json = serde_json::to_value(&scatter).expect("serialize scatter style");
        assert_eq!(json["point_style_index_column"], "style_idx");
        assert_eq!(json["point_style_overrides"][0]["index"], 7);
        assert_eq!(
            json["point_style_overrides"][0]["point_shape"],
            "StarFilled"
        );
        assert_eq!(json["point_style_overrides"][0]["point_size"], 9.0);
        assert!(json["point_style_overrides"][0].get("style").is_none());

        let back: DataScatterStyleConfig =
            serde_json::from_value(json).expect("parse scatter style");
        assert_eq!(back, scatter);
    }

    fn errorbar_style_mapping_round_trips_with_flattened_override() {
        let err = DataErrorBarStyleConfig {
            error_bar_color: Color::BLACK,
            error_bar_width: 1.0,
            error_bar_cap_size: 3.0,
            cap_width: 1.0,
            error_bar_style_table: Some(vec![
                DataErrorBarPointStyleConfig {
                    error_bar_color: Some(Color::from_rgb8(255, 0, 0)),
                    error_bar_width: Some(2.0),
                    error_bar_cap_size: None,
                    cap_width: None,
                },
                DataErrorBarPointStyleConfig {
                    error_bar_color: None,
                    error_bar_width: None,
                    error_bar_cap_size: Some(8.0),
                    cap_width: Some(3.0),
                },
            ]),
            error_bar_style_index_column: Some("err_style_idx".into()),
            error_bar_style_overrides: Some(vec![DataErrorBarPointStyleOverride {
                index: 4,
                style: DataErrorBarPointStyleConfig {
                    error_bar_color: None,
                    error_bar_width: Some(4.0),
                    error_bar_cap_size: None,
                    cap_width: Some(2.0),
                },
            }]),
        };

        let json = serde_json::to_value(&err).expect("serialize errorbar style");
        assert_eq!(json["error_bar_style_index_column"], "err_style_idx");
        assert_eq!(json["error_bar_style_overrides"][0]["index"], 4);
        assert_eq!(json["error_bar_style_overrides"][0]["error_bar_width"], 4.0);
        assert_eq!(json["error_bar_style_overrides"][0]["cap_width"], 2.0);
        assert!(json["error_bar_style_overrides"][0].get("style").is_none());

        let back: DataErrorBarStyleConfig =
            serde_json::from_value(json).expect("parse errorbar style");
        assert_eq!(back, err);
    }

    // The line fields added for area fill and step lines are all defaulted and
    // omitted at their defaults, so a document written before them parses and
    // serializes byte-identically.
    fn legacy_line_style_json_round_trips_without_fill_or_interpolation_keys() {
        use super::{DataLineStyleConfig, LineInterpolation};

        let legacy = serde_json::json!({
            "line_style": "Dash",
            "line_color": { "r": 0.5, "g": 0.25, "b": 0.75, "a": 1.0 },
            "line_width": 2.0
        });
        let parsed: DataLineStyleConfig =
            serde_json::from_value(legacy.clone()).expect("parse legacy line style");
        assert_eq!(parsed.fill, None);
        assert_eq!(parsed.fill_color, None);
        assert_eq!(parsed.interpolation, LineInterpolation::Linear);
        assert_eq!(
            serde_json::to_value(&parsed).expect("serialize legacy line style"),
            legacy,
            "defaults must not add keys to a legacy document"
        );
    }

    fn line_fill_and_interpolation_round_trip_and_keep_their_spelling() {
        use super::{DataLineStyleConfig, LineFill, LineInterpolation};

        let line = DataLineStyleConfig {
            line_style: crate::line::LineStylePreset::Solid,
            line_color: Color::from_rgb8(10, 20, 30),
            line_width: 1.5,
            fill: Some(LineFill::ToSeries("lower".into())),
            fill_color: Some(Color::from_rgba(0.5, 0.5, 0.5, 0.25)),
            interpolation: LineInterpolation::StepMid,
        };
        let json = serde_json::to_value(&line).expect("serialize line style");
        assert_eq!(json["fill"], serde_json::json!({ "ToSeries": "lower" }));
        assert_eq!(json["interpolation"], "StepMid");
        assert_eq!(json["fill_color"]["a"], 0.25);
        let back: DataLineStyleConfig = serde_json::from_value(json).expect("parse line style");
        assert_eq!(back, line);

        let baseline = DataLineStyleConfig {
            fill: Some(LineFill::ToBaseline(-1.0)),
            ..DataLineStyleConfig::default()
        };
        let json = serde_json::to_value(&baseline).expect("serialize baseline fill");
        assert_eq!(json["fill"], serde_json::json!({ "ToBaseline": -1.0 }));
        assert!(json.get("fill_color").is_none());
        assert!(json.get("interpolation").is_none());
        let back: DataLineStyleConfig = serde_json::from_value(json).expect("parse baseline fill");
        assert_eq!(back, baseline);

        for (mode, spelling) in [
            (LineInterpolation::StepPre, "StepPre"),
            (LineInterpolation::StepPost, "StepPost"),
            (LineInterpolation::StepMid, "StepMid"),
        ] {
            assert_eq!(
                serde_json::to_value(mode).expect("serialize interpolation"),
                serde_json::json!(spelling)
            );
        }
    }

    fn legacy_histogram_style_defaults_to_full_width_without_overrides() {
        let legacy = serde_json::json!({
            "fill_color": { "r": 0.2, "g": 0.3, "b": 0.4, "a": 1.0 },
            "border_color": { "r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0 },
            "border_width": 1.0,
            "baseline": 0.0,
            "gap_px": 2.0,
            "orientation": "Vertical"
        });
        let parsed: DataBarStyleConfig =
            serde_json::from_value(legacy).expect("parse legacy histogram style");
        assert_eq!(parsed.width_ratio, 1.0);
        assert_eq!(parsed.bar_style_overrides, None);
    }

    fn vector_render_types_round_trip_and_default_the_optional_fields() {
        let style = VectorStyle {
            color: Color::BLACK,
            width_px: 1.5,
            scale: VectorScale::Pixels { px_per_unit: 20.0 },
            pivot: VectorPivot::Middle,
            head_length_px: 8.0,
            head_width_px: 6.0,
            min_length_px: 2.0,
            color_map: Some(VectorColorMap::Column("speed".into())),
        };
        let points = SeriesConfig {
            series_id: "flow".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::Vector {
                components: VectorComponents::Points {
                    u: "u".into(),
                    v: "v".into(),
                    w: None,
                },
                style: style.clone(),
            },
        };
        let json = serde_json::to_string(&points).unwrap();
        assert!(json.contains("\"Vector\""));
        assert!(json.contains("\"Points\""));
        assert!(json.contains("\"Pixels\""));
        assert!(json.contains("\"Middle\""));
        assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), points);

        let grid = |columns: &[&str]| MatrixRef {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Centers,
        };
        let gridded = SeriesConfig {
            render_type: DataRenderType::Vector {
                components: VectorComponents::Grid {
                    u: grid(&["u0", "u1"]),
                    v: grid(&["v0", "v1"]),
                },
                style: VectorStyle {
                    scale: VectorScale::Data { factor: 0.5 },
                    color_map: Some(VectorColorMap::Grid(grid(&["m0", "m1"]))),
                    ..style
                },
            },
            ..points
        };
        let json = serde_json::to_string(&gridded).unwrap();
        assert!(json.contains("\"Grid\""));
        assert!(json.contains("\"Data\""));
        assert_eq!(
            serde_json::from_str::<SeriesConfig>(&json).unwrap(),
            gridded
        );

        // The optional fields default: the pivot is the tail, the minimum
        // length is zero and omitted, and there is no colour map.
        let minimal = r#"{"series_id":"q","source_id":null,"label":null,"x_column":"x","y_column":"y","render_type":{"Vector":{"components":{"Points":{"u":"u","v":"v"}},"style":{"color":{"r":0.0,"g":0.0,"b":0.0,"a":1.0},"width_px":1.0,"scale":{"Pixels":{"px_per_unit":1.0}},"head_length_px":6.0,"head_width_px":4.0}}}}"#;
        let parsed: SeriesConfig = serde_json::from_str(minimal).unwrap();
        let DataRenderType::Vector { style, .. } = &parsed.render_type else {
            panic!("expected a vector series");
        };
        assert_eq!(style.pivot, VectorPivot::Tail);
        assert_eq!(style.min_length_px, 0.0);
        assert_eq!(style.color_map, None);
        let again = serde_json::to_string(&parsed).unwrap();
        assert!(!again.contains("min_length_px"));
        assert!(!again.contains("color_map"));
        assert!(again.contains("\"pivot\":\"Tail\""));

        // A 2D document has no `w` (#211) and re-serializes without one; a
        // 3D vector writes it and reads it back.
        assert!(!again.contains("\"w\""));
        let DataRenderType::Vector { components, .. } = &parsed.render_type else {
            unreachable!()
        };
        assert!(matches!(
            components,
            VectorComponents::Points { w: None, .. }
        ));
        let mut with_w = parsed.clone();
        if let DataRenderType::Vector {
            components: VectorComponents::Points { w, .. },
            ..
        } = &mut with_w.render_type
        {
            *w = Some("w".into());
        }
        let json = serde_json::to_string(&with_w).unwrap();
        assert!(
            json.contains("\"Points\":{\"u\":\"u\",\"v\":\"v\",\"w\":\"w\"}"),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), with_w);
    }

    fn a_hexagonal_grid_round_trips_and_counts_a_cell_per_coordinate() {
        use super::{GridLayout, MatrixOrientation, MatrixRef};
        let json = serde_json::to_string(&GridLayout::Hexagonal).unwrap();
        assert_eq!(json, "\"Hexagonal\"");
        assert_eq!(
            serde_json::from_str::<GridLayout>(&json).unwrap(),
            GridLayout::Hexagonal
        );
        let matrix = MatrixRef {
            columns: vec!["z0".into(), "z1".into(), "z2".into()],
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Hexagonal,
        };
        assert_eq!(matrix.coordinate_cells(3, 4), (3, 4));
        assert_eq!(matrix.effective_extent(3, 4, 2), (3, 2));
    }

    fn field_render_types_round_trip() {
        use super::{
            ContourConfig, ContourLabelAnchor, ContourLabelConfig, DataBarBinStyleConfig,
            DataBarStyleConfig, DataBarStyleOverride, DataLineStyleConfig, FieldFillConfig,
            FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
        };
        use crate::format::LabelFormat;
        use crate::line::LineStylePreset;

        let matrix = MatrixRef {
            columns: vec!["z0".into(), "z1".into()],
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Edges,
        };
        let fill = FieldFillConfig {
            mode: FillMode::Bands,
            shading: Shading::Interpolated,
            opacity: 0.8,
        };
        let contour = ContourConfig {
            levels: vec![1.0, 2.0, 4.0],
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.5,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: Some(vec![Color::from_rgb8(255, 0, 0)]),
            labels: Some(ContourLabelConfig {
                visible: true,
                font_size: 11.0,
                color: Color::BLACK,
                format: LabelFormat::Decimal,
                significant_digits: 3,
                spacing_px: 120.0,
                anchors: vec![ContourLabelAnchor {
                    level_index: 1,
                    x: 0.25,
                    y: 0.5,
                    tx: 1.0,
                    ty: 0.0,
                }],
                bg_color: None,
                bg_padding_px: 0.0,
            }),
        };
        let cases = [
            DataRenderType::Histogram {
                bar: DataBarStyleConfig {
                    fill_color: Color::from_rgb8(70, 130, 180),
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    baseline: 0.0,
                    gap_px: 1.0,
                    width_ratio: 0.8,
                    orientation: super::BarOrientation::Vertical,
                    bar_style_overrides: Some(vec![DataBarStyleOverride {
                        index: 2,
                        style: DataBarBinStyleConfig {
                            fill_color: Some(Color::from_rgb8(255, 0, 0)),
                            border_color: None,
                            border_width: Some(3.0),
                            gap_px: None,
                            width_ratio: Some(0.5),
                        },
                    }]),
                },
            },
            DataRenderType::Field(crate::data_config::FieldSeries {
                matrix: matrix.clone(),
                fill: Some(fill.clone()),
                contour: None,
                surface: None,
            }),
            DataRenderType::Field(crate::data_config::FieldSeries {
                matrix: matrix.clone(),
                fill: None,
                contour: Some(contour.clone()),
                surface: None,
            }),
            DataRenderType::Field(crate::data_config::FieldSeries {
                matrix,
                fill: Some(fill),
                contour: Some(contour),
                surface: None,
            }),
        ];
        for case in cases {
            let json = serde_json::to_value(&case).expect("serialize render type");
            let back: DataRenderType = serde_json::from_value(json).expect("parse render type");
            assert_eq!(back, case);
        }
    }

    // `per_level_color: None` means "every level uses `line.line_color`". It has
    // to survive as an absent key, not as an empty list — an empty list is a
    // table with no entries, which is a different statement.
    fn contour_optional_keys_are_omitted_when_absent() {
        use super::{ContourConfig, DataLineStyleConfig, FillMode};
        use crate::line::LineStylePreset;

        let contour = ContourConfig {
            levels: vec![1.0],
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: None,
            labels: None,
        };
        let json = serde_json::to_value(&contour).expect("serialize contour");
        assert!(json.get("per_level_color").is_none());
        assert!(json.get("labels").is_none());
        let back: ContourConfig = serde_json::from_value(json).expect("parse contour");
        assert_eq!(back, contour);
        assert_eq!(back.per_level_color, None);

        // And the discriminants stay spelled the way hosts write them.
        assert_eq!(
            serde_json::to_value(FillMode::Continuous).expect("serialize fill mode"),
            serde_json::json!("Continuous")
        );
    }

    fn contour_label_spacing_serde_requires_a_finite_positive_value() {
        use super::ContourLabelConfig;

        let json = |spacing_px: f32| {
            serde_json::json!({
                "visible": false,
                "font_size": 12.0,
                "format": "Decimal",
                "significant_digits": 3,
                "spacing_px": spacing_px,
                "anchors": []
            })
        };
        let parsed = serde_json::from_value::<ContourLabelConfig>(json(0.5)).unwrap();
        assert_eq!(
            parsed.color,
            Color::BLACK,
            "older JSON without the independent label colour defaults to black"
        );
        for spacing_px in [0.0, -1.0] {
            let error = serde_json::from_value::<ContourLabelConfig>(json(spacing_px))
                .expect_err("invalid spacing must fail during deserialization");
            assert!(error.to_string().contains("spacing_px must be finite"));
        }
    }

    /// The one serde test of this file: the additive keys of every series
    /// kind, in turn.
    #[test]
    fn serde_round_trips_the_additive_keys_and_parses_old_documents() {
        old_series_json_without_additive_fields_still_parses();
        value_maps_round_trip_and_default_the_size_scale();
        point_style_mapping_round_trips_with_flattened_override();
        errorbar_style_mapping_round_trips_with_flattened_override();
        legacy_line_style_json_round_trips_without_fill_or_interpolation_keys();
        line_fill_and_interpolation_round_trip_and_keep_their_spelling();
        legacy_histogram_style_defaults_to_full_width_without_overrides();
        vector_render_types_round_trip_and_default_the_optional_fields();
        field_render_types_round_trip();
        contour_optional_keys_are_omitted_when_absent();
        contour_label_spacing_serde_requires_a_finite_positive_value();
        super::category_bar_tests::value_label_offsets_round_trip_and_are_omitted_when_empty();
        super::category_bar_tests::category_bars_round_trip_and_default_their_optional_fields();
        super::category_bar_tests::bar_groups_and_cross_at_are_additive_on_the_wire();
        super::category_bar_tests::log_ticks_are_additive_on_the_wire();
        super::box_plot_tests::box_plot_series_round_trip_and_omit_their_defaults();
        super::box_plot_tests::violins_and_points_round_trip_and_are_absent_by_default();
        super::radar_tests::radar_series_round_trip_and_omit_their_defaults();
        super::streamline_tests::streamlines_round_trip();
        super::range_bar_tests::range_bars_round_trip_and_omit_unset_options();
        a_hexagonal_grid_round_trips_and_counts_a_cell_per_coordinate();
        an_image_parses_with_its_defaults_and_round_trips();
    }

    /// An image (#139) parses with its range, shading and opacity defaulted
    /// (`[0, 1]`, `Flat`, `1`), writes them only when they differ, and round
    /// trips either way.
    fn an_image_parses_with_its_defaults_and_round_trips() {
        use super::{DataRenderType, Shading};
        let matrix = |ch: &str| {
            serde_json::json!({
                "columns": [format!("{ch}0"), format!("{ch}1")],
                "orientation": "ColumnsAreX",
                "grid_layout": "Centers"
            })
        };
        let mut json = base_series_json();
        json["render_type"] = serde_json::json!({
            "RgbImage": { "red": matrix("r"), "green": matrix("g"), "blue": matrix("b") }
        });
        let parsed: SeriesConfig = serde_json::from_value(json).unwrap();
        let DataRenderType::RgbImage {
            red,
            green,
            blue,
            range,
            shading,
            opacity,
        } = &parsed.render_type
        else {
            panic!("expected an image");
        };
        assert_eq!(red.columns, vec!["r0".to_string(), "r1".to_string()]);
        assert_eq!(green.columns[1], "g1");
        assert_eq!(blue.grid_layout, GridLayout::Centers);
        assert_eq!(*range, [0.0, 1.0]);
        assert_eq!(*shading, Shading::Flat);
        assert_eq!(*opacity, 1.0);
        assert_eq!(
            parsed
                .render_type
                .rgb_image()
                .map(|g| g[2].columns[0].as_str()),
            Some("b0")
        );
        let again = serde_json::to_string(&parsed).unwrap();
        for key in ["range", "shading", "opacity"] {
            assert!(
                !again.contains(key),
                "the default {key} is omitted: {again}"
            );
        }
        let back: SeriesConfig = serde_json::from_str(&again).unwrap();
        assert_eq!(back, parsed);
        // Explicit values survive the trip.
        let mut bytes = serde_json::to_value(&parsed).unwrap();
        bytes["render_type"]["RgbImage"]["range"] = serde_json::json!([0.0, 255.0]);
        bytes["render_type"]["RgbImage"]["shading"] = serde_json::json!("Interpolated");
        bytes["render_type"]["RgbImage"]["opacity"] = serde_json::json!(0.5);
        let parsed: SeriesConfig = serde_json::from_value(bytes.clone()).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), bytes);
    }

    /// A stem is an additive key on the marker variants (#206): absent, it
    /// reads back as `None` and is not written; present, its defaults
    /// (`Vertical`, `Solid`) are omitted and the value round-trips.
    #[test]
    fn a_stem_round_trips_and_omits_its_defaults() {
        let mut json = base_series_json();
        json["render_type"]["Scatter"]["stem"] = serde_json::json!({
            "baseline": 0.5,
            "color": { "r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0 },
            "width": 1.5
        });
        let cfg: SeriesConfig = serde_json::from_value(json.clone()).expect("a stem parses");
        let DataRenderType::Xy(super::XySeries {
            stem: Some(stem), ..
        }) = &cfg.render_type
        else {
            panic!("expected a stem: {:?}", cfg.render_type);
        };
        assert_eq!(stem.baseline, 0.5);
        assert_eq!(stem.direction, super::CategoryBarDirection::Vertical);
        assert_eq!(stem.line_style, crate::line::LineStylePreset::Solid);
        let back = serde_json::to_value(&cfg).unwrap();
        assert_eq!(back["render_type"], json["render_type"]);

        // The other direction and a dash are written.
        let mut horizontal = cfg.clone();
        if let DataRenderType::Xy(xy) = &mut horizontal.render_type {
            let stem = xy.stem.as_mut().unwrap();
            stem.direction = super::CategoryBarDirection::Horizontal;
            stem.line_style = crate::line::LineStylePreset::Dash;
        }
        let written = serde_json::to_value(&horizontal).unwrap();
        let stem_json = &written["render_type"]["Scatter"]["stem"];
        assert_eq!(stem_json["direction"], "Horizontal");
        assert_eq!(stem_json["line_style"], "Dash");
        let reread: SeriesConfig = serde_json::from_value(written).unwrap();
        assert_eq!(reread, horizontal);

        // Without a stem nothing is written.
        let plain: SeriesConfig = serde_json::from_value(base_series_json()).unwrap();
        let written = serde_json::to_value(&plain).unwrap();
        assert!(written["render_type"]["Scatter"].get("stem").is_none());

        // Every marker variant carries the key.
        let mut errored = base_series_json();
        errored["render_type"] = serde_json::json!({
            "LineScatterErrorbarXY": {
                "scatter": base_series_json()["render_type"]["Scatter"]["scatter"],
                "line": serde_json::to_value(super::DataLineStyleConfig::default()).unwrap(),
                "err_x": { "Symmetric": { "column": "ex" } },
                "err_y": { "Symmetric": { "column": "ey" } },
                "err_style": {
                    "error_bar_color": { "r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0 },
                    "error_bar_width": 1.0,
                    "error_bar_cap_size": 4.0,
                    "cap_width": 1.0
                },
                "stem": json["render_type"]["Scatter"]["stem"]
            }
        });
        let cfg: SeriesConfig = serde_json::from_value(errored.clone()).unwrap();
        assert!(matches!(
            &cfg.render_type,
            DataRenderType::Xy(super::XySeries {
                stem: Some(_),
                errors: Some(_),
                line: Some(_),
                ..
            })
        ));
        assert_eq!(
            serde_json::to_value(&cfg).unwrap()["render_type"],
            errored["render_type"]
        );
    }
}

#[cfg(test)]
mod line_style_tests {
    use super::*;

    #[test]
    fn line_fill_defaults_to_the_zero_baseline_and_the_line_colour_at_quarter_alpha() {
        assert_eq!(LineFill::default(), LineFill::ToBaseline(0.0));
        let line = DataLineStyleConfig {
            line_color: Color::from_rgba(0.2, 0.4, 0.6, 0.8),
            ..DataLineStyleConfig::default()
        };
        assert_eq!(
            line.effective_fill_color(),
            Color::from_rgba(0.2, 0.4, 0.6, 0.8 * DataLineStyleConfig::DEFAULT_FILL_ALPHA)
        );
        let explicit = DataLineStyleConfig {
            fill_color: Some(Color::WHITE),
            ..line
        };
        assert_eq!(explicit.effective_fill_color(), Color::WHITE);
    }

    #[test]
    fn the_default_interpolation_is_linear() {
        assert!(LineInterpolation::default().is_linear());
    }
}

#[cfg(test)]
mod contour_level_tests {
    use super::*;
    use crate::config::AxisScale;

    fn axis(min: f64, max: f64, scale: AxisScale) -> AxisOptions {
        let mut axis = crate::config::AxisOptions::omitted_colorbar();
        axis.min = min;
        axis.max = max;
        axis.scale = scale;
        axis
    }

    fn contour() -> ContourConfig {
        ContourConfig {
            levels: Vec::new(),
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: None,
            labels: None,
        }
    }

    #[test]
    fn contour_label_spacing_rust_validation_applies_to_all_placement_states() {
        for spacing_px in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let labels = ContourLabelConfig {
                visible: false,
                font_size: 12.0,
                color: Color::BLACK,
                format: LabelFormat::Decimal,
                significant_digits: 3,
                spacing_px,
                anchors: vec![ContourLabelAnchor {
                    level_index: 0,
                    x: 0.0,
                    y: 0.0,
                    tx: 1.0,
                    ty: 0.0,
                }],
                bg_color: None,
                bg_padding_px: 0.0,
            };
            assert_eq!(
                labels.validate(),
                Err(ContourLabelConfig::INVALID_SPACING_REASON)
            );
        }
    }

    #[test]
    fn explicit_contour_label_anchors_past_the_limit_are_rejected() {
        let anchors = |count: usize| ContourLabelConfig {
            visible: true,
            font_size: 12.0,
            color: Color::BLACK,
            format: LabelFormat::Decimal,
            significant_digits: 3,
            spacing_px: 120.0,
            anchors: (0..count)
                .map(|i| ContourLabelAnchor {
                    level_index: 0,
                    x: i as f64,
                    y: 0.0,
                    tx: 1.0,
                    ty: 0.0,
                })
                .collect(),
            bg_color: None,
            bg_padding_px: 0.0,
        };
        assert_eq!(anchors(0).validate(), Ok(()));
        assert_eq!(anchors(MAX_CONTOUR_LABEL_ANCHORS).validate(), Ok(()));
        assert_eq!(
            anchors(MAX_CONTOUR_LABEL_ANCHORS + 1).validate(),
            Err(ContourLabelConfig::TOO_MANY_ANCHORS_REASON)
        );
    }

    /// Asking for more levels gets more, and they stay inside. That is the whole
    /// "how dense" control: the count in, the level list out.
    #[test]
    fn asking_for_more_levels_gets_more() {
        let sparse = ContourConfig::nice_levels(&axis(0.0, 1.0, AxisScale::Linear), 3).unwrap();
        let dense = ContourConfig::nice_levels(&axis(0.0, 1.0, AxisScale::Linear), 11).unwrap();
        assert!(
            dense.len() > sparse.len(),
            "sparse {sparse:?} vs dense {dense:?}"
        );
        for level in sparse.iter().chain(dense.iter()) {
            assert!(*level > 0.0 && *level < 1.0, "{level} is not inside");
        }
    }

    #[test]
    fn nice_levels_accepts_exactly_the_linear_limit() {
        let levels =
            ContourConfig::nice_levels(&axis(0.0, 1025.0, AxisScale::Linear), 2001).unwrap();

        assert_eq!(levels.len(), MAX_CONTOUR_LEVELS);
        assert_eq!(levels.first(), Some(&1.0));
        assert_eq!(levels.last(), Some(&1024.0));
        assert!(levels.windows(2).all(|pair| pair[1] - pair[0] == 1.0));
    }

    #[test]
    fn a_tiny_linear_spacing_is_bounded_by_the_level_limit() {
        assert_eq!(
            ContourConfig::nice_levels(&axis(0.0, 1.0, AxisScale::Linear), usize::MAX),
            Err(TickError::InvalidRange)
        );
    }

    #[test]
    fn logarithmic_levels_are_not_silently_truncated() {
        let levels = ContourConfig::nice_levels(
            &axis(f64::from_bits(1), f64::MAX, AxisScale::Logarithmic),
            MAX_CONTOUR_LEVELS,
        )
        .unwrap();

        assert_eq!(levels.len(), 632);
        assert_eq!(levels.first(), Some(&1e-323));
        assert_eq!(levels.last(), Some(&1e308));
        assert!(levels.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// A range whose ends are not round still gets round levels: the walk anchors
    /// to absolute multiples of the spacing, not to `axis.min`.
    #[test]
    fn an_awkward_range_still_gets_round_levels() {
        let levels = ContourConfig::nice_levels(&axis(0.137, 9.42, AxisScale::Linear), 5).unwrap();
        assert!(!levels.is_empty());
        for level in &levels {
            assert!(
                (level / 2.0).fract().abs() < 1e-9,
                "{level} is not a multiple of the spacing"
            );
        }
    }

    /// A logarithmic colourbar gets decades, from the same axis machinery — no
    /// second rule lives here.
    #[test]
    fn a_logarithmic_range_gets_decades() {
        let levels =
            ContourConfig::nice_levels(&axis(1.0, 10_000.0, AxisScale::Logarithmic), 5).unwrap();
        assert_eq!(levels, vec![10.0, 100.0, 1000.0]);
    }

    #[test]
    fn target_count_zero_keeps_its_existing_one_tick_correction() {
        let corrected =
            ContourConfig::nice_levels(&axis(1.0, 10_000.0, AxisScale::Logarithmic), 0).unwrap();
        let explicit_one =
            ContourConfig::nice_levels(&axis(1.0, 10_000.0, AxisScale::Logarithmic), 1).unwrap();
        assert_eq!(corrected, explicit_one);
    }

    /// A degenerate range has no levels to pick, and says so rather than
    /// inventing one.
    #[test]
    fn a_degenerate_range_is_refused() {
        assert!(ContourConfig::nice_levels(&axis(1.0, 1.0, AxisScale::Linear), 5).is_err());
        assert!(ContourConfig::nice_levels(&axis(-1.0, 10.0, AxisScale::Logarithmic), 5).is_err());
    }

    /// `set_nice_levels` writes the answer into the config — what is drawn stays
    /// exactly what the config holds. Existing per-level colours are left for the
    /// draw's own fallback rather than discarded. The levels are round numbers on
    /// the spacing's own absolute grid, and they sit **inside** the range — a level
    /// on the boundary coincides with the field's edge and would leave the banded
    /// fill one band short.
    #[test]
    fn set_nice_levels_writes_the_levels_and_keeps_the_colours() {
        let mut cfg = contour();
        cfg.per_level_color = Some(vec![Color::BLACK]);
        cfg.set_nice_levels(&axis(0.0, 100.0, AxisScale::Linear), 5)
            .unwrap();
        assert_eq!(cfg.levels, vec![20.0, 40.0, 60.0, 80.0]);
        assert_eq!(cfg.per_level_color, Some(vec![Color::BLACK]));
    }

    #[test]
    fn set_nice_levels_preserves_the_config_when_the_limit_is_exceeded() {
        let mut cfg = contour();
        cfg.levels = vec![42.0];
        cfg.per_level_color = Some(vec![Color::BLACK]);

        assert_eq!(
            cfg.set_nice_levels(&axis(0.0, 1026.0, AxisScale::Linear), 2001),
            Err(TickError::InvalidRange)
        );
        assert_eq!(cfg.levels, vec![42.0]);
        assert_eq!(cfg.per_level_color, Some(vec![Color::BLACK]));
    }

    #[test]
    fn set_nice_levels_preserves_all_state_on_invalid_bounds() {
        let mut cfg = contour();
        cfg.levels = vec![42.0];
        cfg.per_level_color = Some(vec![Color::BLACK]);
        let before = cfg.clone();

        assert_eq!(
            cfg.set_nice_levels(&axis(1.0, f64::INFINITY, AxisScale::Logarithmic), 5),
            Err(TickError::InvalidRange)
        );
        assert_eq!(cfg, before);
    }

    #[test]
    fn set_nice_levels_replaces_levels_with_a_successful_empty_result() {
        let mut cfg = contour();
        cfg.levels = vec![42.0];
        cfg.per_level_color = Some(vec![Color::BLACK]);

        cfg.set_nice_levels(&axis(2.0, 9.0, AxisScale::Logarithmic), 5)
            .unwrap();
        assert!(cfg.levels.is_empty());
        assert_eq!(cfg.per_level_color, Some(vec![Color::BLACK]));
    }

    /// Colouring the levels from the ramp goes through the same `color_for_z` the
    /// field's cells do, so a line and the fill it bounds agree about the value.
    #[test]
    fn colormap_colours_come_from_color_for_z() {
        let mut bar = crate::config::ColorBarOptions::omitted();
        bar.axis.min = 0.0;
        bar.axis.max = 100.0;
        let mut cfg = contour();
        cfg.set_nice_levels(&bar.axis, 5).unwrap();
        cfg.set_colormap_colors(&bar);
        let colors = cfg.per_level_color.as_ref().expect("colours written");
        assert_eq!(colors.len(), cfg.levels.len());
        for (level, color) in cfg.levels.iter().zip(colors) {
            assert_eq!(*color, bar.color_for_z(*level));
        }
    }

    /// Ranges the previous index walk got wrong, pinned for the shared grid:
    /// indices past `i64` (it saturated into an empty list) now get the
    /// level-limit error, and a step that moves neither range end at `f64`
    /// precision (it repeated the few representable values) gets no levels.
    #[test]
    fn nice_levels_refuse_what_the_grid_cannot_walk() {
        assert_eq!(
            ContourConfig::nice_levels(&axis(1e-9, 3e-9, AxisScale::Linear), usize::MAX),
            Err(TickError::InvalidRange)
        );
        assert_eq!(
            ContourConfig::nice_levels(&axis(1e16, 1e16 + 4.0, AxisScale::Linear), 5),
            Ok(Vec::new())
        );
        assert_eq!(
            ContourConfig::nice_levels(&axis(1e16, 1e16 + 4.0, AxisScale::Linear), 2001),
            Err(TickError::InvalidRange)
        );
    }
}

#[cfg(test)]
mod category_bar_tests {
    use super::*;
    use crate::config::Config;
    use crate::layer::{OverlayLayer, Subplot};

    pub(crate) fn categories(n: usize) -> Vec<Category> {
        (0..n)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect()
    }

    pub(crate) fn bars(id: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "positions".into(),
            y_column: format!("{id}.values"),
            z_column: None,
            render_type: DataRenderType::CategoryBars {
                color: Color::from_rgb8(40, 120, 200),
                style: CategoryBarStyle::default(),
                labels: CategoryBarSeriesLabels::default(),
                label_format: CategoryBarLabelFormat::Value,
                overrides: None,
            },
        }
    }

    pub(crate) fn chart(n: usize) -> Config {
        let mut config = crate::config::Config::omitted();
        config.bottom_x.categories = Some(categories(n));
        config
    }

    fn edit(series: &mut SeriesConfig, f: impl FnOnce(&mut CategoryBarStyle, &mut Color)) {
        let DataRenderType::CategoryBars { style, color, .. } = &mut series.render_type else {
            unreachable!()
        };
        f(style, color);
    }

    fn overrides(series: &mut SeriesConfig) -> &mut Vec<CategoryBarOverride> {
        let DataRenderType::CategoryBars { overrides, .. } = &mut series.render_type else {
            unreachable!()
        };
        overrides.get_or_insert_with(Vec::new)
    }

    fn labels(series: &mut SeriesConfig) -> &mut CategoryBarSeriesLabels {
        let DataRenderType::CategoryBars { labels, .. } = &mut series.render_type else {
            unreachable!()
        };
        labels
    }

    #[test]
    fn a_valid_group_passes_and_other_series_are_ignored() {
        let config = chart(3);
        assert_eq!(
            config.validate_category_bars(&[bars("a"), bars("b")]),
            Ok(())
        );
        assert_eq!(
            crate::config::Config::omitted().validate_category_bars(&[]),
            Ok(())
        );
    }

    #[test]
    fn bars_need_a_categorical_axis_in_their_direction() {
        let config = crate::config::Config::omitted();
        assert!(config.validate_category_bars(&[bars("a")]).is_err());
        let mut config = chart(3);
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..Default::default()
        });
        assert!(config.validate_category_bars(&[bars("a")]).is_err());
        config.left_y.categories = Some(categories(3));
        config.bottom_x.categories = None;
        assert_eq!(config.validate_category_bars(&[bars("a")]), Ok(()));
    }

    #[test]
    fn group_limits_and_columns_are_checked() {
        let config = chart(4);
        // No series cap (#107 R3): 40 series, past the old 16, are one
        // group.
        let many: Vec<_> = (0..40).map(|i| bars(&format!("s{i}"))).collect();
        assert_eq!(config.validate_category_bars(&many), Ok(()));
        // No bar cap (#107): 8 series over 65 categories (520 bars, past
        // the old 512) and 1 series over 600 categories are accepted.
        let eight: Vec<_> = (0..8).map(|i| bars(&format!("s{i}"))).collect();
        assert_eq!(chart(65).validate_category_bars(&eight), Ok(()));
        assert_eq!(chart(600).validate_category_bars(&eight[..1]), Ok(()));
        let mut shared = bars("b");
        shared.y_column = "a.values".into();
        assert!(
            chart(3)
                .validate_category_bars(&[bars("a"), shared])
                .is_err()
        );
    }

    #[test]
    fn the_value_axis_is_linear_and_numeric() {
        let mut config = chart(3);
        config.left_y.scale = AxisScale::Logarithmic;
        config.left_y.min = 1.0;
        config.left_y.max = 10.0;
        assert!(config.validate_category_bars(&[bars("a")]).is_err());
        let mut config = chart(3);
        config.left_y.categories = Some(categories(2));
        assert!(config.validate_category_bars(&[bars("a")]).is_err());
    }

    #[test]
    fn bar_groups_are_validated_on_the_chart_and_every_subplot() {
        for groups in [
            BarGroups {
                group_width: 0.05,
                ..Default::default()
            },
            BarGroups {
                group_width: 0.96,
                ..Default::default()
            },
            BarGroups {
                bar_gap: -1.0,
                ..Default::default()
            },
            BarGroups {
                bar_gap: f32::NAN,
                ..Default::default()
            },
            BarGroups {
                bar_gap: 24.5,
                ..Default::default()
            },
        ] {
            assert!(groups.validate().is_err(), "{groups:?}");
            let mut config = chart(3);
            config.bar_groups = Some(groups.clone());
            assert!(config.validate_category_bars(&[]).is_err());
            let mut config = chart(3);
            config.subplots.push(Subplot {
                config: chart(3),
                ..Default::default()
            });
            config.subplots[0].config.bar_groups = Some(groups);
            assert!(config.validate_category_bars(&[]).is_err());
        }
        assert_eq!(BarGroups::default().validate(), Ok(()));
    }

    #[test]
    fn overlays_and_subplots_hold_groups_of_their_own() {
        let mut config = chart(3);
        config.overlays.push(OverlayLayer {
            series: vec!["b".into()],
            ..Default::default()
        });
        assert_eq!(
            config.validate_category_bars(&[bars("a"), bars("b")]),
            Ok(())
        );
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..Default::default()
        });
        config.left_y.categories = Some(categories(3));
        config.bottom_x.categories = None;
        assert_eq!(config.validate_category_bars(&[bars("a")]), Ok(()));
        assert!(
            config
                .validate_category_bars(&[bars("a"), bars("b")])
                .unwrap_err()
                .to_string()
                .contains("overlay")
        );

        let mut config = crate::config::Config::omitted();
        config.subplots.push(Subplot {
            config: chart(2),
            series: vec!["a".into()],
            ..Default::default()
        });
        assert_eq!(config.validate_category_bars(&[bars("a")]), Ok(()));
        // The parent has no categorical axis for a series of its own.
        assert!(
            config
                .validate_category_bars(&[bars("a"), bars("b")])
                .is_err()
        );
        assert_eq!(
            config.bar_groups_of(crate::layer::LayerRef::Subplot(0)),
            BarGroups::default()
        );
    }

    /// A polar chart draws its category bars as a rose (#133): vertical
    /// groups over the categorical angle axis; a horizontal group is named.
    #[test]
    fn polar_charts_draw_vertical_roses_only() {
        let mut config = chart(3);
        config.set_polar(Some(Default::default()));
        assert_eq!(config.validate_category_bars(&[bars("a")]), Ok(()));
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..BarGroups::default()
        });
        assert_eq!(
            config.validate_category_bars(&[bars("a")]),
            Err(crate::RuleKind::RoseDirection)
        );
    }

    #[test]
    fn series_rules_hold_colours_styles_labels_and_overrides() {
        let cats = categories(2);
        assert_eq!(validate_category_bar_series(&bars("a"), &cats), Ok(()));
        let mut s = bars("a");
        edit(&mut s, |_, c| c.a = 0.5);
        assert!(validate_category_bar_series(&s, &cats).is_err());
        let mut s = bars("a");
        edit(&mut s, |style, _| style.material = Material::Toon);
        assert_eq!(
            validate_category_bar_series(&s, &cats),
            Err(crate::RuleKind::CategoricalChartBarMaterial)
        );
        let edits: Vec<Box<dyn Fn(&mut CategoryBarStyle)>> = vec![
            Box::new(|s| s.corner_radius = 32.5),
            Box::new(|s| s.outline_width = -1.0),
            Box::new(|s| s.outline_color.a = 1.5),
            Box::new(|s| s.texture_strength = 1.5),
            Box::new(|s| s.texture_scale = 0.05),
            Box::new(|s| s.gloss = f32::NAN),
            Box::new(|s| s.emphasis_brightness = 0.6),
        ];
        for change in &edits {
            let mut s = bars("a");
            edit(&mut s, |style, _| change(style));
            assert!(validate_category_bar_series(&s, &cats).is_err());
            let mut s = bars("a");
            let mut style = CategoryBarStyle::default();
            change(&mut style);
            let mut o = CategoryBarOverride::new(CategoryBarTarget::new("c0", "a"));
            o.style = Some(style);
            overrides(&mut s).push(o);
            assert!(validate_category_bar_series(&s, &cats).is_err());
        }
        let mut s = bars("a");
        labels(&mut s).decimals = 7;
        assert!(validate_category_bar_series(&s, &cats).is_err());
        let mut s = bars("a");
        labels(&mut s).value_suffix = "a\nb".into();
        assert!(validate_category_bar_series(&s, &cats).is_err());
        let mut s = bars("a");
        labels(&mut s).text = vec!["1".into()];
        assert!(validate_category_bar_series(&s, &cats).is_err());
        labels(&mut s).text = vec!["1".into(), "x".repeat(161)];
        assert!(validate_category_bar_series(&s, &cats).is_err());
        labels(&mut s).text = vec!["1".into(), String::new()];
        assert_eq!(validate_category_bar_series(&s, &cats), Ok(()));

        for target in [
            CategoryBarTarget::new("missing", "a"),
            CategoryBarTarget::new("c0", "other"),
        ] {
            let mut s = bars("a");
            overrides(&mut s).push(CategoryBarOverride::new(target));
            assert!(validate_category_bar_series(&s, &cats).is_err());
        }
        let mut s = bars("a");
        let target = CategoryBarTarget::new("c1", "a");
        overrides(&mut s).push(CategoryBarOverride::new(target.clone()));
        assert_eq!(validate_category_bar_series(&s, &cats), Ok(()));
        overrides(&mut s).push(CategoryBarOverride::new(target));
        assert!(validate_category_bar_series(&s, &cats).is_err());
        let mut s = bars("a");
        let mut o = CategoryBarOverride::new(CategoryBarTarget::new("c1", "a"));
        o.color = Some(Color::new(1.0, 0.0, 0.0, 0.5));
        overrides(&mut s).push(o);
        assert!(validate_category_bar_series(&s, &cats).is_err());
    }

    #[test]
    fn value_label_offsets_are_validated_and_written_by_a_drag() {
        use crate::layout::{NudgeReject, NudgeResult};
        let cats = categories(3);
        let mut series = bars("a");
        assert_eq!(validate_category_bar_series(&series, &cats), Ok(()));
        assert_eq!(series.value_label_offset(1), [0.0, 0.0]);
        // The first drag fills one zero offset per category.
        assert_eq!(
            series.drag_value_label(1, cats.len(), 4.0, -2.5),
            NudgeResult::Moved
        );
        assert_eq!(
            labels(&mut series).offsets,
            [[0.0, 0.0], [4.0, -2.5], [0.0, 0.0]]
        );
        assert_eq!(
            series.drag_value_label(1, cats.len(), 1.0, 0.5),
            NudgeResult::Moved
        );
        assert_eq!(series.value_label_offset(1), [5.0, -2.0]);
        assert_eq!(validate_category_bar_series(&series, &cats), Ok(()));
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        let before = series.clone();
        for (index, dx, dy) in [
            (3, 1.0, 1.0),
            (0, f32::NAN, 0.0),
            (0, 0.0, f32::INFINITY),
            (0, MAX_VALUE_LABEL_OFFSET + 1.0, 0.0),
        ] {
            assert_eq!(series.drag_value_label(index, cats.len(), dx, dy), reject);
            assert_eq!(series, before);
        }
        let mut line = bars("l");
        line.render_type = DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig::default()),
            errors: None,
            stem: None,
        });
        assert_eq!(line.drag_value_label(0, 3, 1.0, 1.0), reject);
        assert_eq!(line.value_label_offsets(), None);

        for offsets in [
            vec![[0.0, 0.0]; 2],
            vec![[0.0, f32::NAN]; 3],
            vec![[MAX_VALUE_LABEL_OFFSET * 2.0, 0.0]; 3],
        ] {
            labels(&mut series).offsets = offsets;
            assert!(validate_category_bar_series(&series, &cats).is_err());
        }
        let mut boxes = box_plot_tests::boxes("b");
        let DataRenderType::BoxPlot { labels: l, .. } = &mut boxes.render_type else {
            unreachable!()
        };
        l.offsets = vec![[0.0, 0.0]; 2];
        assert!(validate_box_plot_series(&boxes, &cats).is_err());
        let mut radar = radar_tests::radar("r");
        let DataRenderType::Radar { labels: l, .. } = &mut radar.render_type else {
            unreachable!()
        };
        l.offsets = vec![[f32::NAN, 0.0]; 3];
        assert!(validate_radar_series(&radar, &cats).is_err());
    }

    #[test]
    fn a_value_label_id_names_its_series_and_category() {
        assert_eq!(value_label_id("a", 3), "series:a:label:3");
        assert_eq!(parse_value_label_id("series:a:label:3"), Some(("a", 3)));
        // A series id may hold the separator itself.
        let id = value_label_id("x:label:y", 12);
        assert_eq!(parse_value_label_id(&id), Some(("x:label:y", 12)));
        for other in [
            "axis_left",
            "series:a:label:",
            "series:a:label:-1",
            "series:a:label:1x",
            "a:label:1",
        ] {
            assert_eq!(parse_value_label_id(other), None, "{other}");
        }
    }

    /// Run by `serde_tests`' one test.
    #[cfg(feature = "serde")]
    pub(super) fn value_label_offsets_round_trip_and_are_omitted_when_empty() {
        let mut series = bars("a");
        let json = serde_json::to_value(&series).unwrap();
        let fields = json["render_type"]["CategoryBars"]["labels"]
            .as_object()
            .unwrap();
        assert!(!fields.contains_key("offsets"));
        labels(&mut series).offsets = vec![[1.5, -2.0], [0.0, 0.0], [3.0, 4.0]];
        let json = serde_json::to_string(&series).unwrap();
        assert!(json.contains(r#""offsets":[[1.5,-2.0],[0.0,0.0],[3.0,4.0]]"#));
        assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), series);
    }

    #[test]
    fn format_labels_writes_each_format_from_the_host_values() {
        let cats = categories(3);
        let values = [
            [Some(2.0), None, Some(1.0)],
            [Some(3.0), Some(0.0), Some(-1.0)],
        ];
        let mut series = vec![bars("a"), bars("b")];
        labels(&mut series[0]).value_suffix = " kg".into();
        let read = |k: usize, i: usize| values[k][i];
        format_labels(&mut series, &cats, read);
        assert_eq!(labels(&mut series[0]).text, ["2 kg", "", "1 kg"]);
        assert_eq!(labels(&mut series[1]).text, ["3", "0", "-1"]);

        let set = |s: &mut SeriesConfig, f: CategoryBarLabelFormat| {
            let DataRenderType::CategoryBars { label_format, .. } = &mut s.render_type else {
                unreachable!()
            };
            *label_format = f;
        };
        for s in &mut series {
            set(s, CategoryBarLabelFormat::Percent);
        }
        format_labels(&mut series, &cats, read);
        // A negative value leaves its category without shares.
        assert_eq!(labels(&mut series[0]).text, ["40%", "", "—"]);
        assert_eq!(labels(&mut series[1]).text, ["60%", "—", "—"]);
        for s in &mut series {
            set(s, CategoryBarLabelFormat::ValuePercent);
        }
        format_labels(&mut series, &cats, read);
        assert_eq!(labels(&mut series[0]).text[0], "2 kg (40%)");
        for s in &mut series {
            set(s, CategoryBarLabelFormat::CategoryValue);
        }
        let mut o = CategoryBarOverride::new(CategoryBarTarget::new("c2", "b"));
        o.label_format = Some(CategoryBarLabelFormat::Value);
        overrides(&mut series[1]).push(o);
        labels(&mut series[1]).decimals = 2;
        format_labels(&mut series, &cats, |k, i| values[k][i].map(|v| v + 0.3));
        assert_eq!(labels(&mut series[0]).text[0], "C0: 2.3 kg");
        assert_eq!(labels(&mut series[1]).text, ["C0: 3.3", "C1: 0.3", "-0.7"]);
        // Every series that is not a category bar series is left alone.
        let mut line = bars("a");
        line.render_type = DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig::default()),
            errors: None,
            stem: None,
        });
        let before = line.clone();
        format_labels(std::slice::from_mut(&mut line), &cats, read);
        assert_eq!(line, before);
    }

    /// Run by `serde_tests`' one test.
    #[cfg(feature = "serde")]
    pub(super) fn category_bars_round_trip_and_default_their_optional_fields() {
        let mut series = bars("a");
        let mut o = CategoryBarOverride::new(CategoryBarTarget::new("c0", "a"));
        o.color = Some(Color::WHITE);
        o.style = Some(CategoryBarStyle {
            material: Material::Paper,
            ..Default::default()
        });
        o.labels = Some(CategoryBarLabels::Inside);
        o.label_format = Some(CategoryBarLabelFormat::Percent);
        overrides(&mut series).push(o);
        labels(&mut series).text = vec!["1".into(), "2".into()];
        let json = serde_json::to_string(&series).unwrap();
        assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), series);

        let minimal = serde_json::json!({
            "series_id": "a",
            "label": null,
            "x_column": "positions",
            "y_column": "a.values",
            "render_type": {"CategoryBars": {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}}}
        });
        let parsed: SeriesConfig = serde_json::from_value(minimal).unwrap();
        let bars = parsed.render_type.category_bars().unwrap();
        assert_eq!(bars.style, &CategoryBarStyle::default());
        assert_eq!(bars.labels, &CategoryBarSeriesLabels::default());
        assert!(bars.overrides.is_empty());
        let written = serde_json::to_value(&parsed).unwrap();
        let fields = &written["render_type"]["CategoryBars"];
        assert!(fields.get("overrides").is_none());
        assert!(fields["labels"].get("text").is_none());
    }

    /// Run by `serde_tests`' one test.
    #[cfg(feature = "serde")]
    pub(super) fn bar_groups_and_cross_at_are_additive_on_the_wire() {
        let config = crate::config::Config::omitted();
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("bar_groups").is_none());
        assert!(json["bottom_x"].get("cross_at").is_none());
        let mut edited = config.clone();
        edited.bar_groups = Some(BarGroups {
            mode: CategoryBarMode::PercentStacked,
            direction: CategoryBarDirection::Horizontal,
            group_width: 0.5,
            bar_gap: 2.0,
        });
        edited.bottom_x.cross_at = Some(0.0);
        let text = serde_json::to_string(&edited).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&text).unwrap(), edited);
        let groups: BarGroups = serde_json::from_str("{}").unwrap();
        assert_eq!(groups, BarGroups::default());
    }

    /// `log_ticks` is written only when it is not the default, so a
    /// document written before the field existed re-serializes unchanged,
    /// and an `Adaptive` axis round-trips (#42).
    /// Run by `serde_tests`' one test.
    #[cfg(feature = "serde")]
    pub(super) fn log_ticks_are_additive_on_the_wire() {
        use crate::config::LogTickPolicy;
        let mut config = crate::config::Config::omitted();
        config.colorbar = Some(crate::config::ColorBarOptions::omitted());
        let json = serde_json::to_value(&config).unwrap();
        for axis in ["top_x", "bottom_x", "left_y", "right_y"] {
            assert!(json[axis].get("log_ticks").is_none(), "{axis}");
        }
        assert!(json["colorbar"]["axis"].get("log_ticks").is_none());
        let mut edited = config.clone();
        edited.left_y.log_ticks = LogTickPolicy::Adaptive;
        edited.colorbar.as_mut().unwrap().axis.log_ticks = LogTickPolicy::Adaptive;
        let json = serde_json::to_value(&edited).unwrap();
        assert_eq!(json["left_y"]["log_ticks"], "Adaptive");
        assert_eq!(json["colorbar"]["axis"]["log_ticks"], "Adaptive");
        assert!(json["bottom_x"].get("log_ticks").is_none());
        let text = serde_json::to_string(&edited).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&text).unwrap(), edited);
        // The written default parses too.
        let mut explicit = serde_json::to_value(&config).unwrap();
        explicit["left_y"]["log_ticks"] = "Decades".into();
        let parsed: Config = serde_json::from_value(explicit).unwrap();
        assert_eq!(parsed, config);
        assert_eq!(config.scaled(2.0).left_y.log_ticks, LogTickPolicy::Decades);
        assert_eq!(
            edited.scaled(2.0).left_y.log_ticks,
            LogTickPolicy::Adaptive,
            "a tick policy is not a pixel dimension"
        );
    }

    #[test]
    fn the_bar_gap_scales_and_the_rest_of_the_groups_stay() {
        let mut config = chart(3);
        config.bar_groups = Some(BarGroups {
            mode: CategoryBarMode::Stacked,
            direction: CategoryBarDirection::Vertical,
            group_width: 0.6,
            bar_gap: 3.0,
        });
        config.bottom_x.cross_at = Some(1.5);
        let scaled = config.scaled(2.0);
        let groups = scaled.bar_groups.unwrap();
        assert_eq!((groups.bar_gap, groups.group_width), (6.0, 0.6));
        assert_eq!(groups.mode, CategoryBarMode::Stacked);
        assert_eq!(scaled.bottom_x.cross_at, Some(1.5));
    }

    #[test]
    fn the_columns_follow_the_direction() {
        let s = bars("a");
        assert_eq!(
            s.category_bar_columns(CategoryBarDirection::Vertical),
            ("positions", "a.values")
        );
        assert_eq!(
            s.category_bar_columns(CategoryBarDirection::Horizontal),
            ("a.values", "positions")
        );
    }
}

#[cfg(test)]
mod box_plot_tests {
    use super::*;
    use crate::config::{Config, DataSelectionsConfig, PickedDataRef};
    use crate::layer::{OverlayLayer, Subplot};

    fn categories(n: usize) -> Vec<Category> {
        (0..n)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect()
    }

    pub(crate) fn boxes(id: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "positions".into(),
            y_column: format!("{id}.record"),
            z_column: None,
            render_type: DataRenderType::BoxPlot {
                color: Color::from_rgb8(40, 120, 200),
                style: BoxPlotStyle::default(),
                labels: BoxPlotSeriesLabels::default(),
                outliers: None,
                overrides: None,
                violin: None,
                points: None,
            },
        }
    }

    fn chart(n: usize) -> Config {
        let mut config = crate::config::Config::omitted();
        config.bottom_x.categories = Some(categories(n));
        config
    }

    fn render(series: &mut SeriesConfig) -> &mut DataRenderType {
        &mut series.render_type
    }

    fn style(series: &mut SeriesConfig) -> &mut BoxPlotStyle {
        let DataRenderType::BoxPlot { style, .. } = render(series) else {
            unreachable!()
        };
        style
    }

    fn overrides(series: &mut SeriesConfig) -> &mut Vec<BoxPlotOverride> {
        let DataRenderType::BoxPlot { overrides, .. } = render(series) else {
            unreachable!()
        };
        overrides.get_or_insert_with(Vec::new)
    }

    fn labels(series: &mut SeriesConfig) -> &mut BoxPlotSeriesLabels {
        let DataRenderType::BoxPlot { labels, .. } = render(series) else {
            unreachable!()
        };
        labels
    }

    #[test]
    fn a_valid_group_passes_and_other_series_are_ignored() {
        let config = chart(3);
        assert_eq!(config.validate_box_plots(&[boxes("a"), boxes("b")]), Ok(()));
        let mut plain = boxes("a");
        plain.render_type = DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig::default()),
            errors: None,
            stem: None,
        });
        // No box plot series: nothing to hold, not even a categorical axis.
        assert_eq!(
            crate::config::Config::omitted().validate_box_plots(&[plain]),
            Ok(())
        );
        assert_eq!(boxes("a").box_plot_columns(), ("positions", "a.record"));
    }

    #[test]
    fn box_groups_are_grouped_only() {
        let mut config = chart(3);
        for mode in [CategoryBarMode::Stacked, CategoryBarMode::PercentStacked] {
            config.bar_groups = Some(BarGroups {
                mode,
                ..BarGroups::default()
            });
            assert!(
                config
                    .validate_box_plots(&[boxes("a")])
                    .unwrap_err()
                    .to_string()
                    .contains("Grouped"),
                "{mode:?}"
            );
        }
        config.bar_groups = Some(BarGroups::default());
        assert_eq!(config.validate_box_plots(&[boxes("a")]), Ok(()));
    }

    #[test]
    fn boxes_need_the_categorical_axis_of_their_direction_and_a_linear_value_axis() {
        let mut config = chart(3);
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..BarGroups::default()
        });
        assert!(config.validate_box_plots(&[boxes("a")]).is_err());
        config.left_y.categories = Some(categories(3));
        config.bottom_x.categories = None;
        assert_eq!(config.validate_box_plots(&[boxes("a")]), Ok(()));
        config.bottom_x.scale = AxisScale::Logarithmic;
        assert_eq!(
            config.validate_box_plots(&[boxes("a")]),
            Err(crate::RuleKind::BoxPlotsValueAxis)
        );
        // A probability value axis too (#209).
        config.bottom_x.scale = AxisScale::Probability(crate::config::ProbabilityScale::Normal);
        assert_eq!(
            config.validate_box_plots(&[boxes("a")]),
            Err(crate::RuleKind::BoxPlotsValueAxis)
        );
        let mut polar = chart(3);
        polar.set_polar(Some(Default::default()));
        assert_eq!(
            polar.validate_box_plots(&[boxes("a")]),
            Err(crate::RuleKind::PolarBoxPlot)
        );
    }

    #[test]
    fn neither_group_size_nor_box_count_is_bounded() {
        let config = chart(2);
        // No series cap (#107 R3): 20 series, past the old 16.
        let many: Vec<_> = (0..20).map(|i| boxes(&format!("s{i}"))).collect();
        assert_eq!(config.validate_box_plots(&many), Ok(()));
        // No box cap (#107): 8 series over 65 categories (520 boxes, past
        // the old 512) and 1 series over 600 categories are accepted.
        let eight: Vec<_> = (0..8).map(|i| boxes(&format!("s{i}"))).collect();
        assert_eq!(chart(65).validate_box_plots(&eight), Ok(()));
        assert_eq!(chart(600).validate_box_plots(&eight[..1]), Ok(()));
    }

    #[test]
    fn a_layer_draws_category_bars_or_box_plots() {
        let config = chart(3);
        let bars = SeriesConfig {
            render_type: DataRenderType::CategoryBars {
                color: Color::from_rgb8(1, 2, 3),
                style: CategoryBarStyle::default(),
                labels: CategoryBarSeriesLabels::default(),
                label_format: CategoryBarLabelFormat::Value,
                overrides: None,
            },
            ..boxes("bars")
        };
        assert_eq!(
            config.validate_box_plots(&[bars, boxes("a")]),
            Err(crate::RuleKind::BarsAndBoxesInLayer)
        );
    }

    #[test]
    fn horizontal_boxes_stay_off_overlays_and_subplots_have_their_own_groups() {
        let mut config = chart(3);
        config.overlays.push(OverlayLayer::default());
        config.overlays[0].series.push("over".into());
        assert_eq!(config.validate_box_plots(&[boxes("over")]), Ok(()));
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..BarGroups::default()
        });
        config.left_y.categories = Some(categories(3));
        assert!(
            config
                .validate_box_plots(&[boxes("over")])
                .unwrap_err()
                .to_string()
                .contains("overlay")
        );
        let mut config = chart(3);
        let mut sub = chart(2);
        sub.bar_groups = Some(BarGroups {
            mode: CategoryBarMode::Stacked,
            ..BarGroups::default()
        });
        config.subplots.push(Subplot {
            config: sub,
            series: vec!["below".into()],
            link_x: None,
            link_y: None,
            marker: None,
        });
        // The subplot's own groups decide: stacked there is rejected.
        assert!(config.validate_box_plots(&[boxes("a")]).is_ok());
        assert!(config.validate_box_plots(&[boxes("below")]).is_err());
    }

    #[test]
    fn styles_labels_ids_and_overrides_are_validated() {
        let config = chart(3);
        let cats = categories(3);
        let mut s = boxes("a");
        style(&mut s).material = crate::Material::Toon;
        assert_eq!(
            validate_box_plot_series(&s, &cats),
            Err(crate::RuleKind::BoxMaterial)
        );
        for edit in [
            |s: &mut BoxPlotStyle| s.corner_radius = 4.0,
            |s: &mut BoxPlotStyle| s.notch_depth = 0.5,
            |s: &mut BoxPlotStyle| s.mean_size = 0.0,
            |s: &mut BoxPlotStyle| s.whisker_width = -1.0,
            |s: &mut BoxPlotStyle| s.outlier_color.a = 2.0,
            |s: &mut BoxPlotStyle| s.gloss = f32::NAN,
        ] {
            let mut s = boxes("a");
            edit(style(&mut s));
            assert_eq!(
                validate_box_plot_series(&s, &cats),
                Err(crate::RuleKind::BoxStyle)
            );
        }
        let mut s = boxes("a");
        let DataRenderType::BoxPlot { color, .. } = render(&mut s) else {
            unreachable!()
        };
        color.a = 0.5;
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        labels(&mut s).decimals = 7;
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        labels(&mut s).text = vec!["1".into(); 2];
        assert_eq!(
            validate_box_plot_series(&s, &cats),
            Err(crate::RuleKind::BoxLabelTextCount)
        );
        labels(&mut s).text = vec!["1".into(), "two\nlines".into(), "3".into()];
        assert!(validate_box_plot_series(&s, &cats).is_err());
        labels(&mut s).text = vec!["1".into(), "2".into(), String::new()];
        assert_eq!(validate_box_plot_series(&s, &cats), Ok(()));
        let mut s = boxes("a");
        s.y_column.clear();
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        let DataRenderType::BoxPlot { outliers, .. } = render(&mut s) else {
            unreachable!()
        };
        *outliers = Some(String::new());
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        overrides(&mut s).push(BoxPlotOverride::new(BoxPlotTarget::new("c1", "a")));
        assert_eq!(validate_box_plot_series(&s, &cats), Ok(()));
        overrides(&mut s).push(BoxPlotOverride::new(BoxPlotTarget::new("c1", "a")));
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        overrides(&mut s).push(BoxPlotOverride::new(BoxPlotTarget::new("zz", "a")));
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        overrides(&mut s).push(BoxPlotOverride::new(BoxPlotTarget::new("c0", "b")));
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("c0", "a"));
        o.style = Some(BoxPlotStyle {
            cap_ratio: 2.0,
            ..BoxPlotStyle::default()
        });
        overrides(&mut s).push(o);
        assert!(validate_box_plot_series(&s, &cats).is_err());
        // The chart-level check runs every series' rules.
        assert!(config.validate_box_plots(&[s]).is_err());
    }

    #[test]
    fn format_box_plot_labels_writes_median_and_count_with_overrides() {
        let mut series = [boxes("a"), boxes("b")];
        let cats = categories(3);
        let values = |k: usize, i: usize| match (k, i) {
            (_, 1) => None,
            (0, i) => Some((1.5 + i as f64, Some(40))),
            (_, i) => Some((10.0 * i as f64, None)),
        };
        format_box_plot_labels(&mut series, &cats, values);
        assert_eq!(labels(&mut series[0]).text, ["", "", ""]);
        labels(&mut series[0]).placement = BoxPlotLabels::Median;
        labels(&mut series[0]).decimals = 2;
        labels(&mut series[0]).value_suffix = " ms".into();
        labels(&mut series[1]).placement = BoxPlotLabels::MedianAndCount;
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("c2", "a"));
        o.labels = Some(BoxPlotLabels::MedianAndCount);
        overrides(&mut series[0]).push(o);
        format_box_plot_labels(&mut series, &cats, values);
        assert_eq!(
            labels(&mut series[0]).text,
            ["1.50 ms", "", "3.50 ms (n=40)"]
        );
        assert_eq!(labels(&mut series[1]).text, ["0.0 (n=—)", "", "20.0 (n=—)"]);
    }

    /// Run by `serde_tests`' one test.
    #[cfg(feature = "serde")]
    pub(super) fn box_plot_series_round_trip_and_omit_their_defaults() {
        let plain = boxes("a");
        let json = serde_json::to_value(&plain).unwrap();
        let fields = json["render_type"]["BoxPlot"].as_object().unwrap();
        assert!(!fields.contains_key("outliers") && !fields.contains_key("overrides"));
        assert!(!fields["labels"].as_object().unwrap().contains_key("text"));
        let back: SeriesConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, plain);
        let mut full = boxes("a");
        let DataRenderType::BoxPlot {
            outliers,
            labels: l,
            ..
        } = render(&mut full)
        else {
            unreachable!()
        };
        *outliers = Some("a.outliers".into());
        l.text = vec!["1".into(), "2".into(), "3".into()];
        style(&mut full).notched = true;
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("c1", "a"));
        o.color = Some(Color::WHITE);
        o.labels = Some(BoxPlotLabels::Median);
        overrides(&mut full).push(o);
        let json = serde_json::to_string(&full).unwrap();
        assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), full);
        // A minimal document: style and labels default.
        let minimal: SeriesConfig = serde_json::from_str(
            r#"{"series_id":"m","label":null,"x_column":"p","y_column":"r",
                "render_type":{"BoxPlot":{"color":{"r":0,"g":0,"b":0,"a":1}}}}"#,
        )
        .unwrap();
        assert_eq!(
            minimal.render_type.box_plot().unwrap().style,
            &BoxPlotStyle::default()
        );
    }

    fn half(curve: &str) -> ViolinHalf {
        ViolinHalf {
            curve: curve.into(),
            density: format!("{curve}.density"),
            density_scale: 0.4,
            style: ViolinStyle::default(),
        }
    }

    fn violin(series: &mut SeriesConfig, body: ViolinBody) {
        let DataRenderType::BoxPlot { violin, .. } = render(series) else {
            unreachable!()
        };
        *violin = Some(Violin::new(body));
    }

    fn points(series: &mut SeriesConfig) -> &mut BoxPoints {
        let DataRenderType::BoxPlot { points, .. } = render(series) else {
            unreachable!()
        };
        points.get_or_insert_with(|| BoxPoints {
            positions: "p.cat".into(),
            values: "p.value".into(),
            style: BoxPointsStyle::default(),
        })
    }

    /// BAR_LANE_LAYOUT_FITS (#152): the renderer's violin lane table holds
    /// two halves, so `halves()` yields at most two, in body order, for the
    /// symmetric body and every split with or without each side.
    #[test]
    fn a_violin_has_at_most_two_halves() {
        let sides =
            |violin: &Violin| -> Vec<ViolinSide> { violin.halves().map(|(s, _)| s).collect() };
        assert_eq!(
            sides(&Violin::new(ViolinBody::Symmetric(half("c")))),
            [ViolinSide::Both]
        );
        for (low, high) in [(false, false), (true, false), (false, true), (true, true)] {
            let violin = Violin::new(ViolinBody::Split {
                low: low.then(|| half("low")),
                high: high.then(|| half("high")),
            });
            let found = sides(&violin);
            assert!(found.len() <= 2, "{found:?}");
            let mut expected = Vec::new();
            if low {
                expected.push(ViolinSide::Low);
            }
            if high {
                expected.push(ViolinSide::High);
            }
            assert_eq!(found, expected);
            for (side, h) in violin.halves() {
                let curve = match side {
                    ViolinSide::Low => "low",
                    ViolinSide::High => "high",
                    ViolinSide::Both => unreachable!(),
                };
                assert_eq!(h.curve, curve);
            }
        }
    }

    #[test]
    fn violins_and_points_validate_and_a_violin_needs_no_record() {
        let config = chart(3);
        let mut a = boxes("a");
        violin(&mut a, ViolinBody::Symmetric(half("curve")));
        points(&mut a);
        assert_eq!(config.validate_box_plots(&[a.clone()]), Ok(()));
        // A violin alone: no record, no box.
        a.y_column.clear();
        assert_eq!(config.validate_box_plots(&[a.clone()]), Ok(()));
        assert!(!a.render_type.box_plot().unwrap().draws_boxes(&a.y_column));
        let mut plain = boxes("p");
        plain.y_column.clear();
        assert!(config.validate_box_plots(&[plain]).is_err());
        // A split violin with one half (a raincloud), and none.
        let mut split = boxes("s");
        violin(
            &mut split,
            ViolinBody::Split {
                low: None,
                high: Some(half("curve")),
            },
        );
        assert_eq!(config.validate_box_plots(&[split.clone()]), Ok(()));
        let mut empty = boxes("e");
        violin(
            &mut empty,
            ViolinBody::Split {
                low: None,
                high: None,
            },
        );
        assert!(config.validate_box_plots(&[empty]).is_err());
        let bad_half = |edit: fn(&mut ViolinHalf)| {
            let mut s = boxes("b");
            let mut h = half("curve");
            edit(&mut h);
            violin(&mut s, ViolinBody::Symmetric(h));
            config.validate_box_plots(&[s]).is_err()
        };
        assert!(bad_half(|h| h.density_scale = 0.0));
        assert!(bad_half(|h| h.density_scale = f64::NAN));
        assert!(bad_half(|h| h.density = h.curve.clone()));
        assert!(bad_half(|h| h.style.outline_width = 9.0));
        assert!(bad_half(
            |h| h.style.fill = Some(Color::new(1.0, 0.0, 0.0, 0.5))
        ));
        let bad_points = |edit: fn(&mut BoxPoints)| {
            let mut s = boxes("b");
            edit(points(&mut s));
            config.validate_box_plots(&[s]).is_err()
        };
        assert!(bad_points(|p| p.values = p.positions.clone()));
        assert!(bad_points(|p| p.style.size = 0.5));
        assert!(bad_points(|p| p.style.jitter = 1.5));
        assert!(bad_points(|p| p.style.offset = -1.5));
        let mut wide = boxes("w");
        violin(&mut wide, ViolinBody::Symmetric(half("curve")));
        let DataRenderType::BoxPlot {
            violin: Some(v), ..
        } = render(&mut wide)
        else {
            unreachable!()
        };
        v.box_width = 1.5;
        assert!(config.validate_box_plots(&[wide]).is_err());
    }

    #[test]
    fn violin_halves_and_columns_follow_the_body() {
        let symmetric = Violin::new(ViolinBody::Symmetric(half("c")));
        let sides: Vec<_> = symmetric
            .halves()
            .map(|(side, h)| (side, h.curve.as_str()))
            .collect();
        assert_eq!(sides, [(ViolinSide::Both, "c")]);
        let split = Violin::new(ViolinBody::Split {
            low: Some(half("lo")),
            high: Some(half("hi")),
        });
        let sides: Vec<_> = split
            .halves()
            .map(|(side, h)| (side, h.curve.as_str()))
            .collect();
        assert_eq!(sides, [(ViolinSide::Low, "lo"), (ViolinSide::High, "hi")]);
        let mut s = boxes("a");
        violin(
            &mut s,
            ViolinBody::Split {
                low: Some(half("lo")),
                high: Some(half("hi")),
            },
        );
        points(&mut s);
        let DataRenderType::BoxPlot { outliers, .. } = render(&mut s) else {
            unreachable!()
        };
        *outliers = Some("out".into());
        let boxes = s.render_type.box_plot().unwrap();
        assert_eq!(
            boxes.ragged_columns().collect::<Vec<_>>(),
            ["out", "lo", "hi"]
        );
        assert_eq!(
            boxes.plain_columns().collect::<Vec<_>>(),
            ["lo.density", "hi.density", "p.cat", "p.value"]
        );
        let mut h = half("c");
        assert!(h.fit_density_scale(2.5));
        assert_eq!(h.density_scale, 2.5);
        assert!(!h.fit_density_scale(0.0));
        assert!(!h.fit_density_scale(f64::INFINITY));
        assert_eq!(h.density_scale, 2.5);
    }

    #[test]
    fn step_selection_walks_box_points_and_their_series() {
        let mut a = boxes("a");
        points(&mut a);
        let mut b = boxes("b");
        points(&mut b).values = "b.value".into();
        let plain = boxes("c");
        let series = [a, plain, b];
        let mut config = chart(3);
        config.picked_data = Some(DataSelectionsConfig {
            refs: vec![PickedDataRef::Point {
                source_id: None,
                series_id: "a".into(),
                point_index: 4,
            }],
            ..Default::default()
        });
        let len = |id: &str| match id {
            "p.cat" => Some(10),
            "p.value" => Some(8),
            "b.value" => Some(5),
            _ => Some(3),
        };
        let at = |config: &Config| match &config.picked_data.as_ref().unwrap().refs[0] {
            PickedDataRef::Point {
                series_id,
                point_index,
                ..
            } => (series_id.clone(), *point_index),
            other => panic!("{other:?}"),
        };
        // Along the points, bounded by the shorter of the two columns.
        assert_eq!(config.step_selection(&series, 9, 0, len), Ok(true));
        assert_eq!(at(&config), ("a".into(), 7));
        // Across to the next series with points, skipping the plain boxes.
        assert_eq!(config.step_selection(&series, 0, 1, len), Ok(true));
        assert_eq!(at(&config), ("b".into(), 4));
    }

    /// Run by `serde_tests`' one test.
    #[cfg(feature = "serde")]
    pub(super) fn violins_and_points_round_trip_and_are_absent_by_default() {
        let plain = boxes("a");
        let json = serde_json::to_value(&plain).unwrap();
        let fields = &json["render_type"]["BoxPlot"];
        assert!(fields.get("violin").is_none() && fields.get("points").is_none());
        let mut full = boxes("a");
        violin(
            &mut full,
            ViolinBody::Split {
                low: None,
                high: Some(half("hi")),
            },
        );
        points(&mut full).style.offset = -0.6;
        let json = serde_json::to_value(&full).unwrap();
        let body = &json["render_type"]["BoxPlot"]["violin"]["body"]["Split"];
        assert!(body.get("low").is_none() && body.get("high").is_some());
        assert_eq!(serde_json::from_value::<SeriesConfig>(json).unwrap(), full);
        // A picked split violin's half is written only when named.
        let whole = PickedDataRef::HistogramBin {
            source_id: None,
            series_id: "a".into(),
            bin_index: 2,
            half: None,
        };
        assert!(serde_json::to_value(&whole).unwrap().get("half").is_none());
        let low = PickedDataRef::HistogramBin {
            source_id: None,
            series_id: "a".into(),
            bin_index: 2,
            half: Some(ViolinSide::Low),
        };
        let json = serde_json::to_value(&low).unwrap();
        assert_eq!(json["kind"], "histogram_bin");
        assert_eq!(json["half"], "Low");
        assert_eq!(serde_json::from_value::<PickedDataRef>(json).unwrap(), low);
        // A minimal violin: the box width and the style default.
        let minimal: Violin = serde_json::from_str(
            r#"{"body":{"Symmetric":{"curve":"c","density":"d","density_scale":1.0}}}"#,
        )
        .unwrap();
        assert_eq!(minimal.box_width, Violin::DEFAULT_BOX_WIDTH);
        let ViolinBody::Symmetric(h) = &minimal.body else {
            unreachable!()
        };
        assert_eq!(h.style, ViolinStyle::default());
    }
}

#[cfg(test)]
mod radar_tests {
    use super::*;
    use crate::config::Config;
    use crate::layer::Subplot;
    use crate::polar::{PolarInterpolation, PolarOptions};

    fn categories(n: usize) -> Vec<Category> {
        (0..n)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect()
    }

    pub(crate) fn radar(id: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "positions".into(),
            y_column: format!("values.{id}"),
            z_column: None,
            render_type: DataRenderType::Radar {
                color: Color::from_rgb8(40, 120, 200),
                style: RadarSeriesStyle::default(),
                labels: RadarSeriesLabels::default(),
            },
        }
    }

    fn polar_config(n: usize) -> Config {
        let mut config = crate::config::Config::omitted();
        config.set_polar(Some(PolarOptions::default()));
        config.bottom_x.categories = Some(categories(n));
        config
    }

    fn style_mut(series: &mut SeriesConfig) -> &mut RadarSeriesStyle {
        match &mut series.render_type {
            DataRenderType::Radar { style, .. } => style,
            _ => unreachable!(),
        }
    }

    fn labels_mut(series: &mut SeriesConfig) -> &mut RadarSeriesLabels {
        match &mut series.render_type {
            DataRenderType::Radar { labels, .. } => labels,
            _ => unreachable!(),
        }
    }

    #[test]
    fn radar_series_need_a_polar_chart_with_a_categorical_angle_axis() {
        let config = polar_config(5);
        assert_eq!(config.validate_radar(&[radar("a"), radar("b")]), Ok(()));
        assert_eq!(
            crate::config::Config::omitted().validate_radar(&[]),
            Ok(()),
            "a chart without radar series passes"
        );
        let mut rectangular = config.clone();
        rectangular.set_polar(None);
        assert!(
            rectangular
                .validate_radar(&[radar("a")])
                .unwrap_err()
                .to_string()
                .contains("polar")
        );
        let mut numeric = config.clone();
        numeric.bottom_x.categories = None;
        assert!(
            numeric
                .validate_radar(&[radar("a")])
                .unwrap_err()
                .to_string()
                .contains("categorical angle axis")
        );
        let mut arcs = config.clone();
        arcs.polar_mut().unwrap().interpolation = PolarInterpolation::Arc { pieces: 4 };
        assert!(matches!(
            arcs.validate_radar(&[radar("a")]).unwrap_err(),
            crate::RuleKind::RadarInterpolation
        ));
        let mut sector = config.clone();
        sector.polar_mut().unwrap().sector = Some([0.0, 180.0]);
        sector.polar_mut().unwrap().inner_radius_fraction = 0.2;
        assert_eq!(sector.validate_radar(&[radar("a")]), Ok(()));
    }

    #[test]
    fn a_chart_draws_any_number_of_radar_series() {
        // No series cap (#107 R3): 20 series, past the old 16.
        let config = polar_config(4);
        let many: Vec<_> = (0..20).map(|i| radar(&format!("s{i}"))).collect();
        assert_eq!(config.validate_radar(&many), Ok(()));
    }

    #[test]
    fn a_subplot_draws_radar_series_against_its_own_polar_axes() {
        let mut config = crate::config::Config::omitted();
        let mut cell = polar_config(3);
        cell.chart_area = config.chart_area.clone();
        config.subplots.push(Subplot {
            config: cell,
            series: vec!["a".into()],
            ..Default::default()
        });
        assert_eq!(config.validate_radar(&[radar("a")]), Ok(()));
        assert!(
            config.validate_radar(&[radar("b")]).is_err(),
            "the parent chart is rectangular"
        );
    }

    #[test]
    fn style_and_labels_are_validated() {
        let config = polar_config(3);
        let check = |series: SeriesConfig| config.validate_radar(&[series]);
        for bad in [
            (|s: &mut RadarSeriesStyle| s.line_width_px = Some(13.0)) as fn(&mut _),
            |s| s.line_width_px = Some(f32::NAN),
            |s| s.fill = Some(Color::new(2.0, 0.0, 0.0, 1.0)),
            |s| s.marker_size_px = Some(0.5),
            |s| s.marker_size_px = Some(25.0),
            |s| s.fill_opacity = Some(1.5),
        ] {
            let mut series = radar("a");
            bad(style_mut(&mut series));
            assert!(check(series).is_err());
        }
        let mut series = radar("a");
        *style_mut(&mut series) = RadarSeriesStyle {
            line_width_px: Some(0.0),
            fill: Some(Color::new(0.0, 0.0, 0.0, 0.0)),
            marker: Some(RadarMarker::Square),
            marker_size_px: Some(24.0),
            fill_opacity: Some(1.0),
        };
        assert_eq!(check(series), Ok(()));
        let mut series = radar("a");
        labels_mut(&mut series).decimals = 7;
        assert!(check(series).is_err());
        let mut series = radar("a");
        labels_mut(&mut series).value_suffix = "x".repeat(25);
        assert!(check(series).is_err());
        let mut series = radar("a");
        labels_mut(&mut series).text = vec!["1".into(); 2];
        assert!(
            check(series)
                .unwrap_err()
                .to_string()
                .contains("one string per category"),
            "text follows the categories"
        );
        let mut series = radar("a");
        labels_mut(&mut series).text = vec!["1".into(), "a\nb".into(), "3".into()];
        assert!(check(series).is_err());
        let mut series = radar("a");
        labels_mut(&mut series).text = vec!["1".into(), "".into(), "3".into()];
        assert_eq!(check(series), Ok(()));
        let mut series = radar("a");
        series.render_type = DataRenderType::Radar {
            color: Color::new(0.0, 0.0, 1.5, 1.0),
            style: RadarSeriesStyle::default(),
            labels: RadarSeriesLabels::default(),
        };
        assert!(check(series).is_err());
        let mut series = radar("a");
        series.y_column.clear();
        assert!(check(series).is_err());
    }

    #[test]
    #[allow(deprecated)] // reads the legacy radar document's style
    fn unset_style_fields_take_the_defaults() {
        let style = RadarSeriesStyle::default();
        let color = Color::from_rgba(0.2, 0.4, 0.6, 0.8);
        assert_eq!(style.line_width(), 2.0);
        assert_eq!(style.marker(), RadarMarker::Circle);
        assert_eq!(style.marker_size(), 7.0);
        assert!((style.fill_color(color).a - 0.2).abs() < 1e-6);
        let chart = crate::radar::RadarStyle {
            line_width_px: 3.0,
            fill_opacity: 0.5,
            marker: RadarMarker::Square,
            marker_size_px: 9.0,
            emphasis_brightness: 0.1,
            ..Default::default()
        };
        let resolved = style.resolved_from(&chart);
        assert_eq!(resolved.line_width_px, Some(3.0));
        assert_eq!(resolved.fill_opacity, Some(0.5));
        assert_eq!(resolved.marker, Some(RadarMarker::Square));
        assert_eq!(resolved.marker_size_px, Some(9.0));
        assert_eq!(resolved.fill, None);
        let own = RadarSeriesStyle {
            marker: Some(RadarMarker::None),
            ..Default::default()
        }
        .resolved_from(&chart);
        assert_eq!(own.marker, Some(RadarMarker::None));
    }

    #[test]
    fn a_radar_draws_as_a_filled_line_with_vertex_markers() {
        let series = radar("a");
        let r = series.render_type.radar().unwrap();
        let line = r.line_style();
        assert_eq!(line.line_style, LineStylePreset::Solid);
        assert_eq!(line.line_color, r.color);
        assert_eq!(line.line_width, 2.0);
        assert_eq!(line.fill, Some(LineFill::ToBaseline(RADAR_FILL_BASELINE)));
        assert_eq!(line.fill_color, Some(r.style.fill_color(r.color)));
        assert!(line.interpolation.is_linear());
        let (hi, lo) = (
            RADAR_FILL_BASELINE as f32,
            (RADAR_FILL_BASELINE - f64::from(RADAR_FILL_BASELINE as f32)) as f32,
        );
        assert!(hi.is_finite() && lo.is_finite());
        let scatter = r.scatter_style().unwrap();
        assert_eq!(scatter.point_shape, ScatterShape::CircleFilled);
        assert_eq!(scatter.point_size, 3.5, "a circle's diameter is the size");
        assert_eq!(scatter.point_color, r.color);
        let mut square = radar("a");
        style_mut(&mut square).marker = Some(RadarMarker::Square);
        let scatter = square.render_type.radar().unwrap().scatter_style().unwrap();
        assert_eq!(scatter.point_shape, ScatterShape::SquareFilled);
        assert!(
            (scatter.point_size * SQUARE_HALF_SIDE * 2.0 - 7.0).abs() < 1e-5,
            "a square's side is the size"
        );
        let mut bare = radar("a");
        *style_mut(&mut bare) = RadarSeriesStyle {
            line_width_px: Some(0.0),
            marker: Some(RadarMarker::None),
            fill_opacity: Some(0.0),
            ..Default::default()
        };
        let r = bare.render_type.radar().unwrap();
        assert_eq!(r.line_style().line_color.a, 0.0, "no outline at width 0");
        assert_eq!(r.line_style().fill, None, "no fill when it is transparent");
        assert!(r.scatter_style().is_none());
    }

    #[test]
    fn format_radar_labels_writes_one_string_per_category() {
        let cats = categories(3);
        let mut series = vec![radar("a"), radar("b")];
        let values = [[1.24, f64::NAN, 3.0], [0.0, 2.0, -1.5]];
        let value = |k: usize, i: usize| Some(values[k][i]);
        format_radar_labels(&mut series, &cats, value);
        for s in &series {
            assert_eq!(s.render_type.radar().unwrap().labels.text, ["", "", ""]);
        }
        for s in &mut series {
            let labels = labels_mut(s);
            labels.placement = RadarLabels::Emphasized;
            labels.decimals = 1;
            labels.value_suffix = " u".into();
        }
        format_radar_labels(&mut series, &cats, value);
        assert_eq!(
            series[0].render_type.radar().unwrap().labels.text,
            ["1.2 u", "", "3.0 u"]
        );
        assert_eq!(
            series[1].render_type.radar().unwrap().labels.text,
            ["0.0 u", "2.0 u", "-1.5 u"]
        );
        format_radar_labels(&mut series, &cats, |_, i| (i != 1).then_some(5.0));
        assert_eq!(
            series[0].render_type.radar().unwrap().labels.text,
            ["5.0 u", "", "5.0 u"]
        );
    }

    /// Run by `serde_tests`' one test.
    #[cfg(feature = "serde")]
    pub(super) fn radar_series_round_trip_and_omit_their_defaults() {
        let mut series = radar("a");
        let json = serde_json::to_value(&series.render_type).unwrap();
        assert_eq!(
            json["Radar"]["labels"],
            serde_json::json!({"placement": "None", "decimals": 1, "value_suffix": ""})
        );
        let style = &json["Radar"]["style"];
        assert!(style.get("fill_opacity").is_none());
        let back: DataRenderType = serde_json::from_value(json).unwrap();
        assert_eq!(back, series.render_type);
        let minimal: DataRenderType = serde_json::from_value(serde_json::json!({
            "Radar": {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}}
        }))
        .unwrap();
        assert_eq!(minimal.radar().unwrap().style, &RadarSeriesStyle::default());
        assert_eq!(
            minimal.radar().unwrap().labels,
            &RadarSeriesLabels::default()
        );
        *style_mut(&mut series) = RadarSeriesStyle {
            line_width_px: Some(3.0),
            fill: Some(Color::WHITE),
            marker: Some(RadarMarker::Square),
            marker_size_px: Some(9.0),
            fill_opacity: Some(0.4),
        };
        *labels_mut(&mut series) = RadarSeriesLabels {
            placement: RadarLabels::Values,
            decimals: 2,
            value_suffix: " pt".into(),
            text: vec!["1.00 pt".into(), "".into()],
            offsets: vec![[2.0, -1.5], [0.0, 0.0]],
        };
        let text = serde_json::to_string(&series).unwrap();
        let back: SeriesConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(back, series);
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
        // A legacy series style keeps its spelling: its four fields are
        // written as before and the two new ones are omitted while unset.
        let legacy = RadarSeriesStyle {
            marker: Some(RadarMarker::None),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&legacy).unwrap(),
            r#"{"line_width_px":null,"fill":null,"marker":"None","marker_size_px":null}"#
        );
    }
}

#[cfg(test)]
mod step_selection_tests {
    use super::*;
    use crate::config::{Config, DataSelectionsConfig, PickedDataRef};
    use std::collections::HashMap;

    fn entry(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: Some(format!("{id}-source")),
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            z_column: None,
            render_type,
        }
    }

    fn line() -> DataRenderType {
        DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig::default()),
            errors: None,
            stem: None,
        })
    }

    fn histogram(orientation: BarOrientation) -> DataRenderType {
        DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: Color::BLACK,
                border_color: Color::BLACK,
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 1.0,
                width_ratio: 1.0,
                orientation,
                bar_style_overrides: None,
            },
        }
    }

    fn bars() -> DataRenderType {
        DataRenderType::CategoryBars {
            color: Color::BLACK,
            style: CategoryBarStyle::default(),
            labels: CategoryBarSeriesLabels::default(),
            label_format: CategoryBarLabelFormat::default(),
            overrides: None,
        }
    }

    fn boxes() -> DataRenderType {
        DataRenderType::BoxPlot {
            color: Color::BLACK,
            style: BoxPlotStyle::default(),
            labels: BoxPlotSeriesLabels::default(),
            outliers: None,
            overrides: None,
            violin: None,
            points: None,
        }
    }

    fn matrix(columns: &[&str], orientation: MatrixOrientation, layout: GridLayout) -> MatrixRef {
        MatrixRef {
            columns: columns.iter().map(|c| (*c).into()).collect(),
            orientation,
            grid_layout: layout,
        }
    }

    fn vector(components: VectorComponents) -> DataRenderType {
        DataRenderType::Vector {
            components,
            style: VectorStyle {
                color: Color::BLACK,
                width_px: 1.0,
                scale: VectorScale::Pixels { px_per_unit: 1.0 },
                pivot: VectorPivot::default(),
                head_length_px: 4.0,
                head_width_px: 4.0,
                min_length_px: 0.0,
                color_map: None,
            },
        }
    }

    fn selected(reference: PickedDataRef) -> Config {
        let mut config = crate::config::Config::omitted();
        config.picked_data = Some(DataSelectionsConfig {
            refs: vec![reference],
            ..Default::default()
        });
        config
    }

    fn point(series_id: &str, point_index: usize) -> PickedDataRef {
        PickedDataRef::Point {
            source_id: Some(format!("{series_id}-source")),
            series_id: series_id.into(),
            point_index,
        }
    }

    fn bin(series_id: &str, bin_index: usize) -> PickedDataRef {
        PickedDataRef::HistogramBin {
            source_id: Some(format!("{series_id}-source")),
            series_id: series_id.into(),
            bin_index,
            half: None,
        }
    }

    fn cell(series_id: &str, x_index: usize, y_index: usize) -> PickedDataRef {
        PickedDataRef::MatrixCell {
            source_id: Some(format!("{series_id}-source")),
            series_id: series_id.into(),
            x_index,
            y_index,
        }
    }

    /// Step `config` once and return the moved selection, `None` when it
    /// did not move.
    fn step(
        config: &mut Config,
        series: &[SeriesConfig],
        columns: &HashMap<&str, usize>,
        dx: i32,
        dy: i32,
    ) -> Option<PickedDataRef> {
        let moved = config
            .step_selection(series, dx, dy, |c| columns.get(c).copied())
            .unwrap();
        moved.then(|| config.picked_data.as_ref().unwrap().refs[0].clone())
    }

    #[test]
    fn a_point_steps_along_its_series_and_across_point_series_and_clamps() {
        let series = [
            entry("a", "x", "ya", line()),
            entry("h", "e", "c", histogram(BarOrientation::Vertical)),
            entry("b", "x", "yb", line()),
            entry(
                "v",
                "x",
                "yb",
                vector(VectorComponents::Points {
                    u: "u".into(),
                    v: "vv".into(),
                    w: None,
                }),
            ),
        ];
        let columns = HashMap::from([
            ("x", 10),
            ("ya", 10),
            ("yb", 8),
            ("e", 4),
            ("c", 3),
            ("u", 5),
            ("vv", 6),
        ]);
        let mut config = selected(point("a", 2));
        assert_eq!(
            step(&mut config, &series, &columns, 3, 0),
            Some(point("a", 5))
        );
        assert_eq!(
            step(&mut config, &series, &columns, 100, 0),
            Some(point("a", 9))
        );
        // At the end of the series nothing moves.
        assert_eq!(step(&mut config, &series, &columns, 1, 0), None);
        // Across the point series, the histogram skipped, the index clamped
        // to the next series' shorter columns.
        assert_eq!(
            step(&mut config, &series, &columns, 0, 1),
            Some(point("b", 7))
        );
        // A point vector counts the points of all four columns.
        assert_eq!(
            step(&mut config, &series, &columns, 0, 1),
            Some(point("v", 4))
        );
        assert_eq!(step(&mut config, &series, &columns, 0, 1), None);
        assert_eq!(
            step(&mut config, &series, &columns, -100, -100),
            Some(point("a", 0))
        );
        assert_eq!(step(&mut config, &series, &columns, -1, -1), None);
    }

    #[test]
    fn a_histogram_bin_steps_by_its_orientation() {
        let series = [
            entry("v1", "e", "c", histogram(BarOrientation::Vertical)),
            entry("h1", "hc", "he", histogram(BarOrientation::Horizontal)),
            entry("v2", "e", "c2", histogram(BarOrientation::Vertical)),
        ];
        let columns = HashMap::from([("e", 6), ("c", 5), ("c2", 3), ("he", 4), ("hc", 3)]);
        let mut config = selected(bin("v1", 1));
        assert_eq!(
            step(&mut config, &series, &columns, 2, 0),
            Some(bin("v1", 3))
        );
        // `dy` moves across the vertical histograms only, clamping.
        assert_eq!(
            step(&mut config, &series, &columns, 0, 1),
            Some(bin("v2", 2))
        );
        assert_eq!(step(&mut config, &series, &columns, 0, 1), None);
        // A horizontal histogram's bins run along `dy`; its edges are y.
        let mut config = selected(bin("h1", 0));
        assert_eq!(
            step(&mut config, &series, &columns, 0, 5),
            Some(bin("h1", 2))
        );
        assert_eq!(step(&mut config, &series, &columns, 1, 0), None);
        // Bins need edges and counts.
        let empty = HashMap::from([("e", 1), ("c", 5)]);
        let mut config = selected(bin("v1", 0));
        assert_eq!(step(&mut config, &series, &empty, 1, 0), None);
    }

    #[test]
    fn a_category_bar_or_box_steps_along_its_categories_and_across_its_layer() {
        let series = [
            entry("a", "pos", "va", bars()),
            entry("box", "pos", "rec", boxes()),
            entry("b", "pos", "vb", bars()),
            entry("c", "pos", "vc", bars()),
        ];
        let columns = HashMap::from([("pos", 4), ("va", 4), ("vb", 4), ("vc", 4)]);
        let mut config = selected(bin("a", 0));
        // Series `c` sits on an overlay, another bar group.
        config.overlays.push(crate::layer::OverlayLayer {
            y_axis: crate::config::AxisOptions::omitted_y(),
            series: vec!["c".into()],
            legend: None,
        });
        assert_eq!(
            step(&mut config, &series, &columns, 2, 0),
            Some(bin("a", 2))
        );
        assert_eq!(
            step(&mut config, &series, &columns, 0, 1),
            Some(bin("b", 2))
        );
        assert_eq!(step(&mut config, &series, &columns, 0, 1), None);
        // Horizontal bars run their categories along `dy`.
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..Default::default()
        });
        config.overlays.clear();
        let horizontal = [
            entry("a", "va", "pos", bars()),
            entry("b", "vb", "pos", bars()),
        ];
        let mut config2 = config.clone();
        assert_eq!(
            step(&mut config2, &horizontal, &columns, 0, 1),
            Some(bin("b", 3))
        );
        assert_eq!(
            step(&mut config2, &horizontal, &columns, -1, -3),
            Some(bin("a", 0))
        );
        // A box steps over the box plot series, its count from its
        // positions (its record is not a registered column).
        let mut config = selected(bin("box", 0));
        assert_eq!(
            step(&mut config, &series, &columns, 9, 0),
            Some(bin("box", 3))
        );
        assert_eq!(step(&mut config, &series, &columns, 0, 1), None);
    }

    #[test]
    fn a_matrix_cell_steps_over_its_grid_and_a_vector_grid_over_u() {
        let field = DataRenderType::Field(crate::data_config::FieldSeries {
            matrix: matrix(
                &["z0", "z1", "z2"],
                MatrixOrientation::ColumnsAreX,
                GridLayout::Centers,
            ),
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            }),
            contour: None,
            surface: None,
        });
        let grid = vector(VectorComponents::Grid {
            u: matrix(
                &["u0", "u1"],
                MatrixOrientation::ColumnsAreY,
                GridLayout::Edges,
            ),
            v: matrix(
                &["v0", "v1"],
                MatrixOrientation::ColumnsAreY,
                GridLayout::Edges,
            ),
        });
        let series = [
            entry("f", "x", "y", field),
            entry("g", "gx", "gy", grid),
            entry("l", "x", "y", line()),
        ];
        let columns = HashMap::from([
            ("x", 3),
            ("y", 5),
            ("z0", 5),
            ("z1", 4),
            ("z2", 5),
            ("gx", 9),
            ("gy", 3),
            ("u0", 8),
            ("u1", 8),
        ]);
        let mut config = selected(cell("f", 0, 0));
        // Three columns along x, the shortest column (4) along y.
        assert_eq!(
            step(&mut config, &series, &columns, 5, 9),
            Some(cell("f", 2, 3))
        );
        // A grid vector: columns along y, edges cut one from each axis.
        let mut config = selected(cell("g", 0, 0));
        assert_eq!(
            step(&mut config, &series, &columns, 20, 20),
            Some(cell("g", 7, 1))
        );
        // A matrix selection on a series that draws no matrix is an error.
        let mut config = selected(cell("l", 0, 0));
        assert_eq!(
            config.step_selection(&series, 1, 0, |c| columns.get(c).copied()),
            Err(crate::RuleKind::SelectedMatrixGone)
        );
    }

    #[test]
    fn no_selection_an_unknown_series_and_labels_do_not_step() {
        let series = [entry("a", "x", "y", line())];
        let columns = HashMap::from([("x", 3), ("y", 3)]);
        let mut config = crate::config::Config::omitted();
        assert_eq!(step(&mut config, &series, &columns, 1, 0), None);
        let mut config = selected(point("gone", 0));
        assert_eq!(step(&mut config, &series, &columns, 1, 0), None);
        let label = PickedDataRef::ValueLabel {
            source_id: None,
            series_id: "a".into(),
            category_index: 0,
            part: None,
        };
        let mut config = selected(label.clone());
        assert_eq!(step(&mut config, &series, &columns, 1, 0), None);
        assert_eq!(config.picked_data.unwrap().refs, [label]);
        // An unregistered column counts as empty: the index clamps to 0.
        let mut config = selected(point("a", 2));
        assert_eq!(
            step(&mut config, &series, &HashMap::new(), 1, 0),
            Some(point("a", 0))
        );
    }

    /// D4: a streamline vertex steps like a line's, the NaN separator rows
    /// included, and across to the other point-drawing series.
    #[test]
    fn step_selection_walks_streamline_vertices() {
        let streamlines = DataRenderType::Streamlines {
            line: DataLineStyleConfig::default(),
            arrows: Some(StreamlineArrows::default()),
            color_map: Some(PointColorMap {
                column: "speed".into(),
            }),
        };
        let series = [
            entry("a", "x", "ya", line()),
            entry("flow", "sx", "sy", streamlines),
        ];
        // Two streamlines of three points with one NaN row between them.
        let columns = HashMap::from([("x", 3), ("ya", 3), ("sx", 7), ("sy", 7), ("speed", 7)]);
        let mut config = selected(point("flow", 2));
        // Onto the separator row and past it, then clamped at the end.
        assert_eq!(
            step(&mut config, &series, &columns, 1, 0),
            Some(point("flow", 3))
        );
        assert_eq!(
            step(&mut config, &series, &columns, 1, 0),
            Some(point("flow", 4))
        );
        assert_eq!(
            step(&mut config, &series, &columns, 9, 0),
            Some(point("flow", 6))
        );
        // Across to the line, clamped to its length.
        assert_eq!(
            step(&mut config, &series, &columns, 0, -1),
            Some(point("a", 2))
        );
        assert_eq!(
            step(&mut config, &series, &columns, 0, 1),
            Some(point("flow", 2))
        );
    }
    // A line offset is a display shift: stepping walks the source indices
    // and the series order exactly as without one.
    #[test]
    fn step_selection_ignores_line_offsets() {
        let series = [
            entry("a", "x", "ya", line()),
            entry("b", "x", "yb", line()),
            entry("c", "x", "yb", line()),
        ];
        let columns = HashMap::from([("x", 10), ("ya", 10), ("yb", 8)]);
        let walk = |offsets: Option<LineOffsets>| {
            let mut config = selected(point("a", 2));
            config.line_offsets = offsets;
            let mut moves = Vec::new();
            for (dx, dy) in [(3, 0), (0, 1), (100, 0), (0, 1), (-1, -1), (0, -100)] {
                moves.push(step(&mut config, &series, &columns, dx, dy));
            }
            moves
        };
        let plain = walk(None);
        assert_eq!(plain[0], Some(point("a", 5)));
        assert_eq!(
            walk(Some(LineOffsets {
                series: vec!["c".into(), "a".into(), "b".into()],
                step_y: 2.5,
                step_x: 0.5,
            })),
            plain
        );
    }

    // The layout rules and the member rules of a line offset group.
    #[test]
    fn line_offsets_are_validated_with_and_without_the_series() {
        let series = [
            entry("a", "x", "ya", line()),
            entry("b", "x", "yb", line()),
            entry("h", "e", "c", histogram(BarOrientation::Vertical)),
        ];
        let group = |ids: &[&str]| LineOffsets {
            series: ids.iter().map(|id| (*id).into()).collect(),
            step_y: 1.0,
            step_x: 0.0,
        };
        let mut config = crate::config::Config::omitted();
        config.line_offsets = Some(group(&["a", "b"]));
        assert_eq!(config.validate_line_offsets(&series), Ok(()));
        assert_eq!(config.line_offset_of("a"), Some([0.0, 0.0]));
        assert_eq!(config.line_offset_of("b"), Some([0.0, 1.0]));
        assert_eq!(config.line_offset_of("h"), None);
        let rejected = |offsets: LineOffsets, edit: fn(&mut Config)| {
            let mut config = crate::config::Config::omitted();
            config.line_offsets = Some(offsets);
            edit(&mut config);
            config.validate_line_offsets(&series).is_err()
        };
        assert!(rejected(group(&["a", "a"]), |_| {}), "duplicate");
        assert!(rejected(group(&["a", ""]), |_| {}), "empty id");
        assert!(rejected(group(&["a", "zz"]), |_| {}), "unknown");
        assert!(rejected(group(&["h"]), |_| {}), "no line");
        let mut nan = group(&["a"]);
        nan.step_y = f64::INFINITY;
        assert!(rejected(nan, |_| {}), "non-finite");
        // No member cap (#107): 40 members, past the old 32, are accepted.
        let many: Vec<String> = (0..40).map(|i| format!("s{i}")).collect();
        assert_eq!(
            LineOffsets {
                series: many,
                ..Default::default()
            }
            .validate(),
            Ok(())
        );
        assert!(
            rejected(group(&["a"]), |c| {
                c.set_polar(Some(crate::polar::PolarOptions::default()))
            }),
            "polar"
        );
        assert!(
            rejected(group(&["a"]), |c| c.subplots.push(crate::layer::Subplot {
                series: vec!["a".into()],
                ..Default::default()
            })),
            "drawn by a subplot"
        );
        // A fill between a member and another series spans two panels.
        let mut filled = series.clone();
        if let DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(line),
            errors: None,
            ..
        }) = &mut filled[1].render_type
        {
            line.fill = Some(LineFill::ToSeries("a".into()));
        }
        let mut config = crate::config::Config::omitted();
        config.line_offsets = Some(group(&["a"]));
        assert!(config.validate_line_offsets(&filled).is_err());
        config.line_offsets = Some(group(&["a", "b"]));
        assert!(config.validate_line_offsets(&filled).is_err());
        config.line_offsets = None;
        assert_eq!(config.validate_line_offsets(&filled), Ok(()));
    }
}

#[cfg(test)]
mod streamline_tests {
    use super::*;

    fn streamlines(arrows: Option<StreamlineArrows>, color: Option<&str>) -> SeriesConfig {
        SeriesConfig {
            series_id: "flow".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::Streamlines {
                line: DataLineStyleConfig {
                    line_width: 1.5,
                    ..DataLineStyleConfig::default()
                },
                arrows,
                color_map: color.map(|column| PointColorMap {
                    column: column.into(),
                }),
            },
        }
    }

    #[test]
    fn streamline_rules_hold_the_line_and_the_arrows() {
        assert_eq!(validate_streamline_series(&streamlines(None, None)), Ok(()));
        let full = streamlines(Some(StreamlineArrows::default()), Some("speed"));
        assert_eq!(validate_streamline_series(&full), Ok(()));
        let edit = |f: fn(&mut DataLineStyleConfig)| {
            let mut series = full.clone();
            let DataRenderType::Streamlines { line, .. } = &mut series.render_type else {
                unreachable!()
            };
            f(line);
            validate_streamline_series(&series)
        };
        assert!(edit(|line| line.interpolation = LineInterpolation::StepMid).is_err());
        assert!(edit(|line| line.fill = Some(LineFill::ToBaseline(0.0))).is_err());
        for arrows in [
            StreamlineArrows {
                spacing_px: 0.0,
                ..StreamlineArrows::default()
            },
            StreamlineArrows {
                spacing_px: f32::NAN,
                ..StreamlineArrows::default()
            },
            StreamlineArrows {
                head_length_px: -1.0,
                ..StreamlineArrows::default()
            },
            StreamlineArrows {
                head_width_px: f32::INFINITY,
                ..StreamlineArrows::default()
            },
            // A head longer than the spacing would overlap the next.
            StreamlineArrows {
                spacing_px: 6.0,
                head_length_px: 8.0,
                head_width_px: 7.0,
            },
        ] {
            assert_eq!(
                validate_streamline_series(&streamlines(Some(arrows), None)),
                Err(StreamlineArrows::INVALID_REASON),
                "{arrows:?}"
            );
        }
        assert!(validate_streamline_series(&streamlines(None, Some(""))).is_err());
        let mut unnamed = full.clone();
        unnamed.y_column.clear();
        assert!(validate_streamline_series(&unnamed).is_err());
        // Other kinds pass untouched.
        let mut line = full;
        line.render_type = DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig::default()),
            errors: None,
            stem: None,
        });
        assert_eq!(validate_streamline_series(&line), Ok(()));
    }

    #[test]
    fn a_streamline_series_draws_its_line_in_the_line_colour() {
        let series = streamlines(Some(StreamlineArrows::default()), None);
        let flow = series.render_type.streamlines().unwrap();
        assert_eq!(flow.line.line_width, 1.5);
        assert_eq!(flow.arrows, Some(&StreamlineArrows::default()));
        assert_eq!(flow.color_map, None);
        let mut cycled = series.clone();
        crate::preset::ColorCycle::Vivid.apply_to_series(&mut cycled, 1);
        let DataRenderType::Streamlines { line, .. } = &cycled.render_type else {
            unreachable!()
        };
        assert_ne!(line.line_color, flow.line.line_color);
    }

    /// Run by `serde_tests`' one test.
    #[cfg(feature = "serde")]
    pub(super) fn streamlines_round_trip() {
        for series in [
            streamlines(None, None),
            streamlines(Some(StreamlineArrows::default()), None),
            streamlines(
                Some(StreamlineArrows {
                    spacing_px: 64.5,
                    head_length_px: 9.0,
                    head_width_px: 6.25,
                }),
                Some("speed"),
            ),
        ] {
            let json = serde_json::to_string(&series).unwrap();
            assert!(json.contains("\"Streamlines\""));
            assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), series);
            assert_eq!(
                serde_json::to_string(&serde_json::from_str::<SeriesConfig>(&json).unwrap())
                    .unwrap(),
                json
            );
        }
        // The optional fields are omitted when absent and default on read.
        let bare = serde_json::to_string(&streamlines(None, None)).unwrap();
        assert!(!bare.contains("arrows"));
        assert!(!bare.contains("color_map"));
        let minimal = r#"{"series_id":"flow","label":null,"x_column":"x","y_column":"y","render_type":{"Streamlines":{"line":{"line_style":"Solid","line_color":{"r":0.0,"g":0.0,"b":0.0,"a":1.0},"line_width":1.0}}}}"#;
        let parsed: SeriesConfig = serde_json::from_str(minimal).unwrap();
        let flow = parsed.render_type.streamlines().unwrap();
        assert_eq!((flow.arrows, flow.color_map), (None, None));
        assert_eq!(serde_json::to_string(&parsed).unwrap(), minimal);
    }
}

#[cfg(test)]
mod range_bar_tests {
    use super::*;
    use crate::config::Config;
    use crate::layer::{OverlayLayer, Subplot};

    fn ranges(id: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "pos".into(),
            y_column: format!("{id}.end"),
            z_column: None,
            render_type: DataRenderType::RangeBars {
                color: Color::from_rgb8(40, 120, 200),
                style: CategoryBarStyle::default(),
                start: format!("{id}.start"),
                whiskers: None,
                color_by: RangeColor::Single,
                progress: None,
                connector: None,
                slot_width: None,
                labels: CategoryBarSeriesLabels::default(),
            },
        }
    }

    /// The fields of a range bar series, mutable.
    fn edit(series: &mut SeriesConfig) -> &mut DataRenderType {
        &mut series.render_type
    }

    fn chart() -> Config {
        let mut config = crate::config::Config::omitted();
        config.bottom_x.categories = Some(vec![
            Category::new("a", "A"),
            Category::new("b", "B"),
            Category::new("c", "C"),
        ]);
        config
    }

    fn every_option(series: &mut SeriesConfig) {
        let id = series.series_id.clone();
        let DataRenderType::RangeBars {
            whiskers,
            color_by,
            progress,
            connector,
            labels,
            ..
        } = edit(series)
        else {
            unreachable!()
        };
        *whiskers = Some(RangeWhiskers {
            low: format!("{id}.low"),
            high: format!("{id}.high"),
            width: 2.0,
            color: Some(Color::BLACK),
        });
        *color_by = RangeColor::Direction {
            up: Color::from_rgb8(0, 160, 0),
            down: Color::from_rgb8(200, 0, 0),
            total: Some(RangeTotal {
                column: format!("{id}.total"),
                color: Color::from_rgb8(0, 0, 200),
            }),
        };
        *progress = Some(RangeProgress {
            column: format!("{id}.progress"),
            remaining_opacity: 0.4,
        });
        *connector = Some(RangeConnector {
            color: Color::from_rgb8(90, 90, 90),
            width: 1.5,
        });
        labels.placement = CategoryBarLabels::Outside;
        labels.text = vec!["+1".into(), "-2".into(), "3".into()];
        labels.offsets = vec![[0.0, 0.0], [1.0, -2.0], [0.0, 0.0]];
    }

    #[test]
    fn a_valid_group_passes_on_categorical_and_numeric_position_axes() {
        let config = chart();
        let mut one = ranges("one");
        every_option(&mut one);
        assert_eq!(
            config.validate_range_bars(std::slice::from_ref(&one)),
            Ok(())
        );
        // Any number of series stand side by side (without connectors).
        let many: Vec<SeriesConfig> = (0..40).map(|k| ranges(&format!("s{k}"))).collect();
        assert_eq!(config.validate_range_bars(&many), Ok(()));
        // A numeric position axis needs a slot width; a categorical one
        // has none.
        let mut numeric = crate::config::Config::omitted();
        assert!(numeric.validate_range_bars(&[ranges("t")]).is_err());
        let mut timed = ranges("t");
        let DataRenderType::RangeBars { slot_width, .. } = edit(&mut timed) else {
            unreachable!()
        };
        *slot_width = Some(86_400.0);
        assert_eq!(
            numeric.validate_range_bars(std::slice::from_ref(&timed)),
            Ok(())
        );
        assert!(
            config
                .validate_range_bars(std::slice::from_ref(&timed))
                .is_err()
        );
        // A logarithmic value axis is drawn (prices); a logarithmic
        // position axis is not.
        let mut log = config.clone();
        log.left_y.scale = AxisScale::Logarithmic;
        assert_eq!(log.validate_range_bars(&[ranges("one")]), Ok(()));
        numeric.bottom_x.scale = AxisScale::Logarithmic;
        assert!(numeric.validate_range_bars(&[timed]).is_err());
        // A categorical value axis is refused.
        let mut values = config.clone();
        values.left_y.categories = Some(vec![Category::new("x", "X")]);
        assert!(values.validate_range_bars(&[ranges("one")]).is_err());
    }

    #[test]
    fn layer_rules_hold_the_group() {
        let config = chart();
        // Polar charts draw none.
        let mut polar = config.clone();
        polar.set_polar(Some(Default::default()));
        assert!(polar.validate_range_bars(&[ranges("one")]).is_err());
        // The group stands side by side.
        let mut stacked = config.clone();
        stacked.bar_groups = Some(BarGroups {
            mode: CategoryBarMode::Stacked,
            ..BarGroups::default()
        });
        assert!(stacked.validate_range_bars(&[ranges("one")]).is_err());
        // A layer draws one kind of bars.
        let mut bars = ranges("bars");
        bars.render_type = DataRenderType::CategoryBars {
            color: Color::BLACK,
            style: CategoryBarStyle::default(),
            labels: CategoryBarSeriesLabels::default(),
            label_format: CategoryBarLabelFormat::Value,
            overrides: None,
        };
        assert!(config.validate_range_bars(&[bars, ranges("one")]).is_err());
        // A connector needs its series alone in its layer.
        let mut joined = ranges("one");
        let DataRenderType::RangeBars { connector, .. } = edit(&mut joined) else {
            unreachable!()
        };
        *connector = Some(RangeConnector {
            color: Color::BLACK,
            width: 1.0,
        });
        assert_eq!(
            config.validate_range_bars(std::slice::from_ref(&joined)),
            Ok(())
        );
        assert!(
            config
                .validate_range_bars(&[joined, ranges("two")])
                .is_err()
        );
        // Horizontal range bars cannot sit on an overlay layer.
        let mut overlay = config.clone();
        overlay.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..BarGroups::default()
        });
        overlay.left_y.categories = overlay.bottom_x.categories.take();
        overlay.overlays.push(OverlayLayer {
            series: vec!["one".into()],
            ..Default::default()
        });
        assert!(overlay.validate_range_bars(&[ranges("one")]).is_err());
        // A subplot's own axes decide for its range bars.
        let mut parent = crate::config::Config::omitted();
        parent.subplots.push(Subplot {
            config: chart(),
            series: vec!["one".into()],
            ..Default::default()
        });
        assert_eq!(parent.validate_range_bars(&[ranges("one")]), Ok(()));
    }

    #[test]
    fn series_rules_reject_bad_columns_widths_and_colours() {
        let ok = || {
            let mut s = ranges("one");
            every_option(&mut s);
            s
        };
        assert_eq!(validate_range_bar_series(&ok()), Ok(()));
        let mut s = ok();
        s.y_column = "one.start".into();
        assert!(
            validate_range_bar_series(&s).is_err(),
            "a column read twice"
        );
        let mut s = ok();
        s.x_column.clear();
        assert!(validate_range_bar_series(&s).is_err(), "an empty column id");
        let reject = |f: &dyn Fn(&mut DataRenderType)| {
            let mut s = ok();
            f(&mut s.render_type);
            validate_range_bar_series(&s)
        };
        let whiskers = |w: f32| {
            reject(&|rt| {
                if let DataRenderType::RangeBars {
                    whiskers: Some(x), ..
                } = rt
                {
                    x.width = w;
                }
            })
        };
        assert!(whiskers(24.5).is_err());
        assert!(whiskers(f32::NAN).is_err());
        assert_eq!(whiskers(24.0), Ok(()));
        assert!(
            reject(&|rt| {
                if let DataRenderType::RangeBars {
                    connector: Some(c), ..
                } = rt
                {
                    c.width = -1.0;
                }
            })
            .is_err()
        );
        assert!(
            reject(&|rt| {
                if let DataRenderType::RangeBars { color_by, .. } = rt {
                    *color_by = RangeColor::State {
                        column: "one.state".into(),
                        styles: vec![RangeStateStyle {
                            color: Color::from_rgba8(0, 0, 0, 10),
                        }],
                    };
                }
            })
            .is_err()
        );
        assert!(
            reject(&|rt| {
                if let DataRenderType::RangeBars {
                    progress: Some(p), ..
                } = rt
                {
                    p.remaining_opacity = 1.5;
                }
            })
            .is_err()
        );
        assert!(
            reject(&|rt| {
                if let DataRenderType::RangeBars { slot_width, .. } = rt {
                    *slot_width = Some(0.0);
                }
            })
            .is_err()
        );
        assert!(
            reject(&|rt| {
                if let DataRenderType::RangeBars { labels, .. } = rt {
                    labels.offsets = vec![[0.0, 0.0]];
                }
            })
            .is_err(),
            "one offset per label"
        );
        assert!(
            reject(&|rt| {
                if let DataRenderType::RangeBars { labels, .. } = rt {
                    labels.text[0] = "two\nlines".into();
                }
            })
            .is_err()
        );
        // Thousands of state styles and labels are configuration, not a
        // cap (rule 10).
        assert_eq!(
            reject(&|rt| {
                if let DataRenderType::RangeBars {
                    color_by, labels, ..
                } = rt
                {
                    *color_by = RangeColor::State {
                        column: "one.state".into(),
                        styles: vec![
                            RangeStateStyle {
                                color: Color::BLACK
                            };
                            5000
                        ],
                    };
                    labels.text = vec!["x".into(); 5000];
                    labels.offsets.clear();
                }
            }),
            Ok(())
        );
    }

    #[test]
    fn step_selection_walks_range_bars_by_row_and_series() {
        use crate::config::{DataSelectionsConfig, PickedDataRef};
        let mut config = chart();
        let series = [ranges("one"), ranges("two")];
        let pick = |series: &str, row: usize| PickedDataRef::HistogramBin {
            source_id: None,
            series_id: series.into(),
            bin_index: row,
            half: None,
        };
        config.picked_data = Some(DataSelectionsConfig {
            refs: vec![pick("one", 0)],
            ..DataSelectionsConfig::default()
        });
        // Rows: the shortest column of the series bounds them.
        let len = |c: &str| match c {
            "pos" | "one.end" | "two.end" | "two.start" => Some(5),
            "one.start" => Some(4),
            _ => None,
        };
        assert_eq!(config.step_selection(&series, 1, 0, len), Ok(true));
        assert_eq!(config.picked_data.as_ref().unwrap().refs[0], pick("one", 1));
        assert_eq!(config.step_selection(&series, 9, 0, len), Ok(true));
        assert_eq!(config.picked_data.as_ref().unwrap().refs[0], pick("one", 3));
        assert_eq!(config.step_selection(&series, 0, 1, len), Ok(true));
        assert_eq!(config.picked_data.as_ref().unwrap().refs[0], pick("two", 3));
    }

    #[test]
    fn value_labels_drag_by_row() {
        let mut s = ranges("one");
        every_option(&mut s);
        let count = s.value_label_text().map_or(0, <[String]>::len);
        assert_eq!(count, 3);
        assert_eq!(
            s.drag_value_label(2, count, 4.0, -1.0),
            crate::layout::NudgeResult::Moved
        );
        assert_eq!(s.value_label_offset(2), [4.0, -1.0]);
    }

    /// Run by `serde_tests`' one test.
    #[cfg(feature = "serde")]
    pub(super) fn range_bars_round_trip_and_omit_unset_options() {
        let mut full = ranges("one");
        every_option(&mut full);
        let json = serde_json::to_string(&full).unwrap();
        assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), full);
        let bare = serde_json::to_value(ranges("one")).unwrap();
        let fields = bare["render_type"]["RangeBars"].as_object().unwrap();
        for absent in [
            "whiskers",
            "color_by",
            "progress",
            "connector",
            "slot_width",
        ] {
            assert!(!fields.contains_key(absent), "{absent}");
        }
        let minimal = serde_json::json!({
            "series_id": "one",
            "label": null,
            "x_column": "pos",
            "y_column": "one.end",
            "render_type": {"RangeBars": {
                "color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0},
                "start": "one.start"
            }}
        });
        let parsed: SeriesConfig = serde_json::from_value(minimal).unwrap();
        let bars = parsed.render_type.range_bars().unwrap();
        assert_eq!(bars.color_by, &RangeColor::Single);
        assert_eq!(bars.style, &CategoryBarStyle::default());
        assert!(bars.whiskers.is_none() && bars.progress.is_none() && bars.connector.is_none());
        let again = serde_json::to_string(&parsed).unwrap();
        assert_eq!(
            serde_json::from_str::<SeriesConfig>(&again).unwrap(),
            parsed
        );
    }
}

#[cfg(test)]
mod hexagonal_tests {
    use super::{
        ContourConfig, DataLineStyleConfig, DataRenderType, FieldFillConfig, FillMode, GridLayout,
        LineInterpolation, MatrixOrientation, MatrixRef, SeriesConfig, Shading, VectorComponents,
        VectorScale, VectorStyle,
    };
    use crate::color::Color;
    use crate::config::AxisScale;
    use crate::line::LineStylePreset;
    use crate::view3d::View3d;

    fn matrix(layout: GridLayout) -> MatrixRef {
        MatrixRef {
            columns: vec!["z0".into(), "z1".into()],
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: layout,
        }
    }

    fn fill(shading: Shading) -> FieldFillConfig {
        FieldFillConfig {
            mode: FillMode::Continuous,
            shading,
            opacity: 1.0,
        }
    }

    fn series(render_type: DataRenderType) -> SeriesConfig {
        SeriesConfig {
            series_id: "hex".into(),
            source_id: None,
            z_column: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type,
        }
    }

    fn heatmap(layout: GridLayout, shading: Shading) -> SeriesConfig {
        series(DataRenderType::Field(crate::data_config::FieldSeries {
            matrix: matrix(layout),
            fill: Some(fill(shading)),
            contour: None,
            surface: None,
        }))
    }

    fn contour() -> ContourConfig {
        ContourConfig {
            levels: vec![0.5],
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: LineInterpolation::Linear,
            },
            per_level_color: None,
            labels: None,
        }
    }

    /// A hexagonal grid is a flat heatmap on linear axes of a flat chart;
    /// every other reading of it is named.
    #[test]
    fn the_rules_name_each_rejection() {
        let config = crate::config::Config::omitted();
        let ok = |series: &[SeriesConfig]| config.validate_hexagonal_series(series);
        assert_eq!(ok(&[heatmap(GridLayout::Hexagonal, Shading::Flat)]), Ok(()));
        assert_eq!(
            ok(&[heatmap(GridLayout::Centers, Shading::Interpolated)]),
            Ok(()),
            "a rectangular grid keeps its freedoms"
        );
        assert_eq!(
            ok(&[heatmap(GridLayout::Hexagonal, Shading::Interpolated)]),
            Err(crate::RuleKind::HexagonalShading)
        );
        assert_eq!(
            ok(&[series(DataRenderType::Field(
                crate::data_config::FieldSeries {
                    matrix: matrix(GridLayout::Hexagonal),
                    fill: None,
                    contour: Some(contour()),
                    surface: None
                }
            ))]),
            Err(crate::RuleKind::HexagonalContours)
        );
        assert_eq!(
            ok(&[series(DataRenderType::Field(
                crate::data_config::FieldSeries {
                    matrix: matrix(GridLayout::Hexagonal),
                    fill: Some(fill(Shading::Flat)),
                    contour: Some(contour()),
                    surface: None
                }
            ))]),
            Err(crate::RuleKind::HexagonalContours)
        );
        assert_eq!(
            ok(&[series(DataRenderType::Vector {
                components: VectorComponents::Grid {
                    u: matrix(GridLayout::Hexagonal),
                    v: matrix(GridLayout::Hexagonal),
                },
                style: VectorStyle {
                    color: Color::BLACK,
                    width_px: 1.0,
                    scale: VectorScale::Pixels { px_per_unit: 1.0 },
                    head_length_px: 4.0,
                    head_width_px: 3.0,
                    pivot: Default::default(),
                    min_length_px: 0.0,
                    color_map: None,
                },
            })]),
            Err(crate::RuleKind::HexagonalGridVector)
        );
        let mut log = config.clone();
        log.left_y.scale = AxisScale::Logarithmic;
        assert_eq!(
            log.validate_hexagonal_series(&[heatmap(GridLayout::Hexagonal, Shading::Flat)]),
            Err(crate::RuleKind::HexagonalAxes)
        );
        for style in [
            crate::config::AxisBreakStyle::Continuous,
            crate::config::AxisBreakStyle::Panels,
        ] {
            let mut broken = config.clone();
            broken.bottom_x.breaks = Some(crate::config::AxisBreaks {
                ranges: vec![crate::config::AxisBreak::new(0.2, 0.4)],
                style,
                ..Default::default()
            });
            let expected = match style {
                crate::config::AxisBreakStyle::Continuous => {
                    Err(crate::RuleKind::HexagonalBrokenAxis)
                }
                crate::config::AxisBreakStyle::Panels => Ok(()),
            };
            assert_eq!(
                broken.validate_hexagonal_series(&[heatmap(GridLayout::Hexagonal, Shading::Flat)]),
                expected,
                "{style:?}"
            );
        }
        let mut three_d = config.clone();
        three_d.set_view_3d(Some(View3d::default()));
        let hex = [heatmap(GridLayout::Hexagonal, Shading::Flat)];
        assert_eq!(
            three_d.validate_hexagonal_series(&hex),
            Err(crate::RuleKind::View3dHexagonalGrid)
        );
        assert_eq!(
            three_d.validate_view3d_series(&hex),
            Err(crate::RuleKind::View3dHexagonalGrid),
            "the 3D rules reject it too, whichever runs first"
        );
    }
}

#[cfg(test)]
mod image_tests {
    use super::{DataRenderType, GridLayout, MatrixOrientation, MatrixRef, SeriesConfig, Shading};
    use crate::view3d::View3d;

    fn matrix(ch: &str, orientation: MatrixOrientation, layout: GridLayout) -> MatrixRef {
        MatrixRef {
            columns: vec![format!("{ch}0"), format!("{ch}1")],
            orientation,
            grid_layout: layout,
        }
    }

    fn image(range: [f64; 2], opacity: f32) -> SeriesConfig {
        SeriesConfig {
            series_id: "img".into(),
            source_id: None,
            z_column: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::RgbImage {
                red: matrix("r", MatrixOrientation::ColumnsAreX, GridLayout::Centers),
                green: matrix("g", MatrixOrientation::ColumnsAreX, GridLayout::Centers),
                blue: matrix("b", MatrixOrientation::ColumnsAreX, GridLayout::Centers),
                range,
                shading: Shading::Flat,
                opacity,
            },
        }
    }

    /// An image's channels share one lattice, its range is finite and
    /// ordered, its opacity 0..=1, and it draws flat; every other reading is
    /// named.
    #[test]
    fn the_rules_name_each_rejection() {
        let config = crate::config::Config::omitted();
        let ok = |series: &[SeriesConfig]| config.validate_rgb_image_series(series);
        assert_eq!(ok(&[image([0.0, 1.0], 1.0)]), Ok(()));
        assert_eq!(
            ok(&[image([0.0, 255.0], 0.0)]),
            Ok(()),
            "bytes, fully transparent"
        );
        let mut short = image([0.0, 1.0], 1.0);
        if let DataRenderType::RgbImage { green, .. } = &mut short.render_type {
            green.columns.pop();
        }
        assert_eq!(ok(&[short]), Err(crate::RuleKind::ImageChannelColumns));
        let mut turned = image([0.0, 1.0], 1.0);
        if let DataRenderType::RgbImage { blue, .. } = &mut turned.render_type {
            blue.orientation = MatrixOrientation::ColumnsAreY;
        }
        assert_eq!(ok(&[turned]), Err(crate::RuleKind::ImageChannelLattice));
        let mut edges = image([0.0, 1.0], 1.0);
        if let DataRenderType::RgbImage { red, .. } = &mut edges.render_type {
            red.grid_layout = GridLayout::Edges;
        }
        assert_eq!(ok(&[edges]), Err(crate::RuleKind::ImageChannelLattice));
        for range in [
            [1.0, 1.0],
            [2.0, 1.0],
            [0.0, f64::INFINITY],
            [f64::NAN, 1.0],
        ] {
            assert_eq!(
                ok(&[image(range, 1.0)]),
                Err(crate::RuleKind::ImageRange),
                "{range:?}"
            );
        }
        for opacity in [-0.1, 1.5, f32::NAN] {
            assert_eq!(
                ok(&[image([0.0, 1.0], opacity)]),
                Err(crate::RuleKind::ImageOpacity),
                "{opacity}"
            );
        }
        let mut three_d = config.clone();
        three_d.set_view_3d(Some(View3d::default()));
        let flat = [image([0.0, 1.0], 1.0)];
        assert_eq!(
            three_d.validate_rgb_image_series(&flat),
            Err(crate::RuleKind::View3dImage)
        );
        assert_eq!(
            three_d.validate_view3d_series(&flat),
            Err(crate::RuleKind::View3dImage),
            "the 3D rules reject it too, whichever runs first"
        );
    }
}

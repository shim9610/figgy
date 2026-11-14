//! Consumers can opt into serde through the renderer without importing model.
#![cfg(feature = "serde")]
// Legacy documents are read through their deprecated reader types.
#![allow(deprecated)]

#[test]
fn renderer_serde_feature_covers_reexported_chart_models() {
    let config = renderer::default::default_config();
    let json = serde_json::to_string(&config).unwrap();
    let restored: renderer::Config = serde_json::from_str(&json).unwrap();
    assert_eq!(
        serde_json::to_value(config).unwrap(),
        serde_json::to_value(restored).unwrap()
    );

    let radial = renderer::RadialChart {
        slices: vec![renderer::RadialSlice::new("A", 1., renderer::Color::BLACK)],
        ..Default::default()
    };
    let restored: renderer::RadialChart =
        serde_json::from_str(&serde_json::to_string(&radial).unwrap()).unwrap();
    assert_eq!(radial, restored);

    let categorical = renderer::CategoricalChart {
        categories: vec![renderer::Category::new("a", "A")],
        series: vec![renderer::BarSeries::new(
            "s",
            "S",
            vec![Some(3.)],
            renderer::Color::BLACK,
        )],
        ..Default::default()
    };
    let restored: renderer::CategoricalChart =
        serde_json::from_str(&serde_json::to_string(&categorical).unwrap()).unwrap();
    assert_eq!(categorical, restored);

    let boxplot = renderer::BoxPlotChart {
        categories: vec![renderer::Category::new("a", "A")],
        series: vec![renderer::BoxPlotSeries::new(
            "s",
            "S",
            vec![Some(renderer::BoxSummary::new(0., 1., 2., 3., 4.))],
            renderer::Color::BLACK,
        )],
        ..Default::default()
    };
    let restored: renderer::BoxPlotChart =
        serde_json::from_str(&serde_json::to_string(&boxplot).unwrap()).unwrap();
    assert_eq!(boxplot, restored);

    let radar = renderer::RadarChart {
        categories: vec![
            renderer::Category::new("a", "A"),
            renderer::Category::new("b", "B"),
            renderer::Category::new("c", "C"),
        ],
        series: vec![renderer::RadarSeries::new(
            "s",
            "S",
            vec![1., 2., 3.],
            renderer::Color::BLACK,
        )],
        ..Default::default()
    };
    let restored: renderer::RadarChart =
        serde_json::from_str(&serde_json::to_string(&radar).unwrap()).unwrap();
    assert_eq!(radar, restored);
}

/// Bars, boxes and sectors take the one `Material`, re-exported by the
/// renderer, and write it as the bare variant name.
#[test]
fn one_material_type_serves_every_bounded_family() {
    let material = renderer::Material::Wood;
    let mut radial = renderer::RadialStyle::default();
    let mut bars = renderer::CategoryBarStyle::default();
    let mut boxes = renderer::BoxPlotStyle::default();
    radial.material = material;
    bars.material = material;
    boxes.material = material;
    for json in [
        serde_json::to_value(&radial).unwrap(),
        serde_json::to_value(&bars).unwrap(),
        serde_json::to_value(&boxes).unwrap(),
    ] {
        assert_eq!(json["material"], "Wood");
    }
}

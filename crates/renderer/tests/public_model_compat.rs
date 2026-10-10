//! Consumers can opt into serde through the renderer without importing model.
#![cfg(feature = "serde")]

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
}

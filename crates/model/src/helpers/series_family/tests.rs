use super::*;
use crate::Config;
use crate::config::Category;
use crate::data_config::{
    BoxPlotOverride, BoxPlotTarget, BoxPoints, DataLineStyleConfig, MatrixRef, PointColorMap,
    RangeProgress, Shading, SizeScale, VectorColorMap, ViolinBody, ViolinHalf, XySeries,
};
use crate::tri_mesh::{MeshValueLocation, MeshValues};

fn grid(prefix: &str) -> MatrixRef {
    MatrixRef::columns_are_x((0..3).map(|i| format!("{prefix}{i}")))
}

fn valid(config: &Config, series: &SeriesConfig) {
    if let Err(error) = config.validate_series(std::slice::from_ref(series)) {
        panic!("{:?}: {error:?}", series.render_type);
    }
}

fn categorical() -> Config {
    let mut config = Config::default();
    config.bottom_x.categories = Some(
        (0..3)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect(),
    );
    config
}

fn with_colorbar() -> Config {
    let mut config = Config::default();
    config.colorbar = Some(crate::defaults::color_bar_options());
    config
}

fn xy(series: &SeriesConfig) -> &XySeries {
    match &series.render_type {
        DataRenderType::Xy(xy) => xy,
        other => panic!("an x-y series, not {other:?}"),
    }
}

// Markers drop, a ternary series fills its region and a size map changes
// its scale through the marker closure; an x-y series takes no region and
// a series without markers takes no marker edit.
#[test]
fn marker_and_region_modifiers() {
    let series = SeriesConfig::scatter_line("a", "x", "y").no_markers();
    assert!(xy(&series).scatter.is_none());
    assert!(xy(&series).line.is_some());
    valid(&Config::default(), &series);

    let mut ternary = Config::default();
    ternary.set_ternary(Some(crate::defaults::ternary_options()));
    let fill = Color::from_rgb8(200, 160, 60);
    let region = SeriesConfig::ternary("t", "p", "q", "r")
        .with_line(2.0)
        .no_markers()
        .region(fill, 0.75);
    let DataRenderType::Ternary(TernarySeries {
        scatter,
        line,
        region: style,
        ..
    }) = &region.render_type
    else {
        panic!("a ternary series");
    };
    assert!(scatter.is_none() && line.is_some());
    let style = style.as_ref().unwrap();
    assert_eq!((style.color, style.opacity), (fill, 0.75));
    valid(&ternary, &region);

    let radius = |scatter: &mut crate::data_config::DataScatterStyleConfig| {
        if let Some(size) = &mut scatter.point_size_map {
            size.scale = SizeScale::Radius;
        }
    };
    let sized = SeriesConfig::scatter("s", "x", "y")
        .size_by("m", [0.0, 1.0], [2.0, 9.0])
        .style_markers(radius);
    let size = xy(&sized).scatter.as_ref().unwrap().point_size_map.as_ref();
    assert_eq!(size.unwrap().scale, SizeScale::Radius);
    valid(&Config::default(), &sized);

    let line = SeriesConfig::line("l", "x", "y");
    assert_eq!(line.clone().region(fill, 0.5), line);
    assert_eq!(
        line.clone()
            .style_markers(|scatter| scatter.point_size_map = None),
        line
    );
}

// Range bars take their colouring, whiskers, connector and labels from
// modifiers, and progress, slot and bar style from the parts closure;
// category bars take the labels too, and a box plot takes none.
#[test]
fn range_bar_modifiers() {
    let up = Color::from_rgb8(40, 150, 90);
    let down = Color::from_rgb8(200, 60, 50);
    let grey = Color::from_rgb8(150, 150, 146);
    let colors = RangeColor::Direction {
        up,
        down,
        total: None,
    };
    let text: Vec<String> = ["+1", "-2", "+3"].map(String::from).to_vec();
    let bars = SeriesConfig::range_bars("r", "i", "start", "end")
        .color(up)
        .range_colors(colors.clone())
        .whiskers("low", "high", 1.5)
        .connector(grey, 1.5)
        .bar_labels(CategoryBarLabels::Outside, text.clone())
        .style_range_bars(|parts| {
            parts.progress = Some(RangeProgress {
                column: "done".into(),
                remaining_opacity: 0.35,
            });
            parts.style.corner_radius = 2.0;
        });
    let DataRenderType::RangeBars(RangeBarSeries {
        style,
        whiskers,
        color_by,
        progress,
        connector,
        slot_width,
        labels,
        ..
    }) = &bars.render_type
    else {
        panic!("range bars");
    };
    assert_eq!(*color_by, colors);
    let whiskers = whiskers.as_ref().unwrap();
    assert_eq!(
        (
            whiskers.low.as_str(),
            whiskers.high.as_str(),
            whiskers.width,
            whiskers.color
        ),
        ("low", "high", 1.5, None)
    );
    let progress = progress.as_ref().unwrap();
    assert_eq!(
        (progress.column.as_str(), progress.remaining_opacity),
        ("done", 0.35)
    );
    assert_eq!(
        connector.as_ref().map(|c| (c.color, c.width)),
        Some((grey, 1.5))
    );
    assert_eq!(*slot_width, None);
    assert_eq!(style.corner_radius, 2.0);
    assert_eq!(
        (labels.placement, &labels.text),
        (CategoryBarLabels::Outside, &text)
    );
    valid(&categorical(), &bars);

    // A numeric position axis takes a slot width.
    let candles = SeriesConfig::range_bars("c", "t", "open", "close")
        .color(up)
        .style_range_bars(|parts| parts.slot_width = Some(86_400.0));
    let DataRenderType::RangeBars(RangeBarSeries { slot_width, .. }) = &candles.render_type else {
        panic!("range bars");
    };
    assert_eq!(*slot_width, Some(86_400.0));
    valid(&Config::default(), &candles);

    let category = SeriesConfig::category_bars("b", "i", "v")
        .color(up)
        .bar_labels(CategoryBarLabels::Inside, text.clone());
    let DataRenderType::CategoryBars(CategoryBarSeries { labels, .. }) = &category.render_type
    else {
        panic!("category bars");
    };
    assert_eq!(
        (labels.placement, &labels.text),
        (CategoryBarLabels::Inside, &text)
    );
    valid(&categorical(), &category);

    let boxes = SeriesConfig::box_plot("x", "i", "records").color(up);
    let same = boxes
        .clone()
        .range_colors(colors)
        .whiskers("low", "high", 1.5)
        .connector(grey, 1.5)
        .bar_labels(CategoryBarLabels::Outside, text)
        .style_range_bars(|parts| parts.slot_width = Some(1.0));
    assert_eq!(same, boxes);
}

// Box plots take a violin and outliers from modifiers, and raw points,
// overrides (moved in, not copied) and the box style from the parts
// closure; range bars take none.
#[test]
fn box_plot_modifiers() {
    let ink = Color::from_rgb8(60, 90, 140);
    let half = ViolinHalf {
        curve: "curve".into(),
        density: "density".into(),
        density_scale: 1.0,
        style: crate::defaults::violin_style(),
    };
    let violin = Violin::new(ViolinBody::Symmetric(half));
    let points = BoxPoints {
        positions: "pos".into(),
        values: "value".into(),
        style: crate::defaults::box_points_style(),
    };
    let mut first = BoxPlotOverride::new(BoxPlotTarget::new("c0", "b"));
    first.color = Some(Color::from_rgb8(200, 60, 50));
    let overrides = vec![first];
    let buffer = overrides.as_ptr();
    let boxes = SeriesConfig::box_plot("b", "i", "records")
        .color(ink)
        .violin(violin.clone())
        .outliers("out")
        .style_box_plot(|parts| {
            parts.points = Some(points.clone());
            parts.overrides = Some(overrides);
            parts.style.notched = true;
        });
    let DataRenderType::BoxPlot(BoxSeries {
        style,
        outliers,
        overrides,
        violin: v,
        points: p,
        ..
    }) = &boxes.render_type
    else {
        panic!("a box plot");
    };
    assert_eq!(v.as_ref(), Some(&violin));
    assert_eq!(p.as_ref(), Some(&points));
    let overrides = overrides.as_ref().unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides.as_ptr(), buffer);
    assert_eq!(outliers.as_deref(), Some("out"));
    assert!(style.notched);
    valid(&categorical(), &boxes);

    let bars = SeriesConfig::range_bars("r", "i", "start", "end").color(ink);
    let same = bars
        .clone()
        .violin(violin)
        .outliers("out")
        .style_box_plot(|parts| {
            parts.points = Some(points);
            parts.overrides = Some(Vec::new());
        });
    assert_eq!(same, bars);
}

// Fields and meshes with contours take a contour edit and levels; a filled
// one takes a surface look; a field without a fill takes no surface, and a
// vector series takes none of these.
#[test]
fn field_and_mesh_modifiers() {
    let config = with_colorbar();
    let contours = SeriesConfig::contours("c", "x", "y", grid("z"), vec![0.5])
        .style_contours(|contour| contour.line.line_width = 2.5)
        .contour_levels(vec![0.25, 0.5, 0.75]);
    let DataRenderType::Field(field) = &contours.render_type else {
        panic!("a field");
    };
    let contour = field.contour.as_ref().unwrap();
    assert_eq!(contour.line.line_width, 2.5);
    assert_eq!(contour.levels, [0.25, 0.5, 0.75]);
    valid(&config, &contours);
    // No fill: no surface.
    assert_eq!(
        contours
            .clone()
            .style_surface(|surface| surface.gloss = 0.5),
        contours
    );

    let heat = SeriesConfig::heatmap("h", "x", "y", grid("z"))
        .style_surface(|surface| surface.gloss = 0.5);
    let DataRenderType::Field(field) = &heat.render_type else {
        panic!("a field");
    };
    assert_eq!(field.surface.as_ref().map(|s| s.gloss), Some(0.5));
    valid(&config, &heat);

    let values = MeshValues {
        column: "value".into(),
        at: MeshValueLocation::Vertices,
    };
    let mut mesh =
        SeriesConfig::tri_mesh("m", "x", "y", "tri", values).shading(Shading::Interpolated);
    if let DataRenderType::TriMesh(series) = &mut mesh.render_type {
        series.contour = Some(ContourConfig {
            levels: vec![0.5],
            line: DataLineStyleConfig::default(),
            per_level_color: None,
            labels: None,
        });
    }
    let mesh = mesh
        .style_contours(|contour| contour.line.line_width = 1.5)
        .contour_levels(vec![0.1, 0.9])
        .style_surface(|surface| surface.gloss = 0.3);
    let DataRenderType::TriMesh(series) = &mesh.render_type else {
        panic!("a mesh");
    };
    let contour = series.contour.as_ref().unwrap();
    assert_eq!(
        (contour.line.line_width, contour.levels.as_slice()),
        (1.5, [0.1, 0.9].as_slice())
    );
    assert_eq!(series.surface.as_ref().map(|s| s.gloss), Some(0.3));
    valid(&config, &mesh);

    let arrows = SeriesConfig::vectors("v", "x", "y", "u", "w");
    let same = arrows
        .clone()
        .style_contours(|contour| contour.levels.clear())
        .contour_levels(vec![1.0])
        .style_surface(|surface| surface.gloss = 1.0);
    assert_eq!(same, arrows);
}

// Vectors take a scale and a colour column, and a minimum length through
// the glyph closure; a grid series keeps its colour (it takes a colour
// grid), and a field takes none.
#[test]
fn vector_modifiers() {
    let arrows = SeriesConfig::vectors("v", "x", "y", "u", "w")
        .vector_scale(VectorScale::Data { factor: 0.35 })
        .style_vectors(|style| style.min_length_px = 2.0)
        .color_by("speed");
    let DataRenderType::Vector(VectorSeries { style, .. }) = &arrows.render_type else {
        panic!("a vector series");
    };
    assert_eq!(
        (style.scale.clone(), style.min_length_px),
        (VectorScale::Data { factor: 0.35 }, 2.0)
    );
    assert_eq!(
        style.color_map,
        Some(VectorColorMap::Column("speed".into()))
    );
    valid(&with_colorbar(), &arrows);

    let gridded = SeriesConfig::vectors_grid("g", "x", "y", grid("u"), grid("w"));
    assert_eq!(gridded.clone().color_by("speed"), gridded);

    let heat = SeriesConfig::heatmap("h", "x", "y", grid("z"));
    let same = heat
        .clone()
        .vector_scale(VectorScale::Pixels { px_per_unit: 4.0 })
        .style_vectors(|style| style.min_length_px = 3.0);
    assert_eq!(same, heat);

    // Markers still take the colour column.
    let scatter = SeriesConfig::scatter("s", "x", "y").color_by("c");
    assert_eq!(
        xy(&scatter).scatter.as_ref().unwrap().point_color_map,
        Some(PointColorMap { column: "c".into() })
    );
}

// A horizontal histogram names its columns in their swapped roles: the
// counts on x, the edges on y.
#[test]
fn a_horizontal_histogram_takes_swapped_columns() {
    let bars = SeriesConfig::histogram("h", "counts", "edges").horizontal();
    assert_eq!(
        (bars.x_column.as_str(), bars.y_column.as_str()),
        ("counts", "edges")
    );
    valid(&Config::default(), &bars);
}

/// The timeline helper (#357): `range_bars` with one muted colour per row,
/// the first as the series colour, flat bars with 4 px corners, and labels
/// inside or beside their bar in contrast ink; every choice is a series
/// field. `with_progress` adds the host's progress column with the light
/// remaining tint. `range_bars` keeps its look.
#[test]
fn timeline_bars_colour_each_row_and_draw_flat() {
    use crate::data_config::{CategoryBarLabels, RangeProgress};
    use crate::default::{TIMELINE_COLORS, TIMELINE_LABEL_INK, TIMELINE_REMAINING_OPACITY};
    let bars = SeriesConfig::timeline_bars("t", "i", "start", "end").with_progress("done");
    let plain = SeriesConfig::range_bars("t", "i", "start", "end");
    let (DataRenderType::RangeBars(timeline), DataRenderType::RangeBars(range)) =
        (&bars.render_type, &plain.render_type)
    else {
        panic!("range bars");
    };
    assert_eq!(timeline.color, TIMELINE_COLORS[0]);
    assert_eq!(
        timeline.color_by,
        RangeColor::ByRow {
            colors: TIMELINE_COLORS.to_vec()
        }
    );
    assert_eq!(timeline.style.corner_radius, 4.0);
    assert_eq!(timeline.style.texture_strength, 0.0);
    assert_eq!(timeline.style.gloss, 0.0);
    assert_eq!(timeline.labels.placement, CategoryBarLabels::InsideOrBeside);
    assert_eq!(timeline.labels.contrast_ink, Some(TIMELINE_LABEL_INK));
    assert_eq!(
        timeline.progress,
        Some(RangeProgress {
            column: "done".into(),
            remaining_opacity: TIMELINE_REMAINING_OPACITY,
        })
    );
    assert_eq!(
        (bars.x_column.as_str(), bars.y_column.as_str()),
        ("i", "end")
    );
    assert_eq!(timeline.start, "start");
    assert_eq!(range.color_by, RangeColor::Single);
    assert_eq!(range.style, crate::defaults::category_bar_style());
    assert_eq!(range.labels, crate::defaults::category_bar_series_labels());
    assert_eq!(range.progress, None);
    let mut config = crate::default::default_timeline_config();
    config.left_y.categories = Some(vec![Category::new("a", "A"), Category::new("b", "B")].into());
    valid(&config, &bars);
}

/// The timeline chart (#357): horizontal bars over 0.6 of a row's band,
/// the first row at the top, a faint band behind every other row, no row
/// axis line, time gridlines only, no tick marks and a light top rule;
/// every choice is a stored field, and a saved timeline reads back the
/// same. The documented default is unchanged.
#[test]
fn the_timeline_config_puts_the_first_row_on_top_with_a_light_frame() {
    use crate::config::{CategoryBandEvery, CategoryBands, TickVisibility};
    use crate::data_config::{BarGroups, CategoryBarDirection};
    let config = crate::default::default_timeline_config();
    let plain = crate::default::default_config();
    assert_eq!(
        config.bar_groups,
        Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            group_width: 0.6,
            ..crate::defaults::bar_groups()
        })
    );
    assert!(config.left_y.inverted && !plain.left_y.inverted);
    assert_eq!(
        config.left_y.category_bands,
        Some(CategoryBands {
            fill: crate::Color::from_rgb8(247, 248, 250),
            every: CategoryBandEvery::Alternate,
        })
    );
    assert_eq!(plain.left_y.category_bands, None);
    // The row labels are the host's: the chart made from it fits them.
    assert_eq!(config.layout_state, crate::config::LayoutState::Fitted);
    assert!(config.grid.show_major_x && !config.grid.show_major_y);
    assert!(!config.grid.show_minor_x && !config.grid.show_minor_y);
    assert!(config.top_x.line_visible && !config.right_y.line_visible);
    assert!(!config.left_y.line_visible && !config.bottom_x.line_visible);
    for axis in [&config.bottom_x, &config.left_y] {
        assert_eq!(axis.tick, TickVisibility::None);
        assert_ne!(axis.label_style.color, plain.bottom_x.label_style.color);
    }
    assert_ne!(config.top_x.line_color, plain.top_x.line_color);
    // The helpers' default differs from the documented one by its tick
    // label format alone (#357).
    assert_eq!(
        plain,
        crate::default::config_with_default_label_format(crate::defaults::config())
    );
    #[cfg(feature = "serde")]
    {
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&json).unwrap(), config);
    }
}

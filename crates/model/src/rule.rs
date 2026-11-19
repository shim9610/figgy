//! Why a chart's configuration, a series or a renderer request is refused
//! (#215): one value per rule, which a host matches, and the sentence it
//! reads as that value's `Display`.
//!
//! [`RuleKind`] is the reason inside [`crate::ConfigError`] and inside the
//! renderer's `FiggyError::InvalidConfig` and `InvalidSeriesConfig`. It is
//! `#[non_exhaustive]`, so a release can add a rule without breaking a
//! host's `match`; [`RuleKind::code`] is the variant's name, the stable
//! string a host that cannot match an enum (the browser) branches on. The
//! `Display` sentences are the ones these errors carried as strings before
//! the enum, byte for byte.

/// A unit rule's doc: its sentence, or the Markdown its entry names.
macro_rules! rule_doc {
    ($sentence:literal) => {
        $sentence
    };
    ($sentence:literal, $doc:literal) => {
        $doc
    };
}

/// Writes [`RuleKind`] from one listing (#341). A unit rule is its name
/// and its sentence: the sentence is the variant's doc and its `Display`,
/// and the name is its [`RuleKind::code`]. A rule that carries ids, counts
/// or lengths lists its doc and fields; its sentence is written by hand in
/// `RuleKind::fmt_fielded`. A sentence that does not read as Markdown names
/// its doc after `doc`.
macro_rules! rule_kinds {
    (
        units {
            $($unit:ident => $sentence:literal $(; doc $doc:literal)?,)*
        }
        fielded {
            $($(#[$meta:meta])* $fielded:ident { $($field:ident: $ty:ty),* $(,)? },)*
        }
    ) => {
        /// One rule a chart, a series or a renderer request broke.
        #[derive(Clone, Debug, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum RuleKind {
            $(#[doc = rule_doc!($sentence $(, $doc)?)] $unit,)*
            $($(#[$meta])* $fielded { $($field: $ty),* },)*
        }

        impl RuleKind {
            /// The variant's name, a stable code a host can branch on without
            /// parsing the sentence (the browser's `code`).
            pub fn code(&self) -> &'static str {
                match self {
                    $(Self::$unit => stringify!($unit),)*
                    $(Self::$fielded { .. } => stringify!($fielded),)*
                }
            }
        }

        impl std::fmt::Display for RuleKind {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let sentence = match self {
                    $(Self::$unit => $sentence,)*
                    other => return other.fmt_fielded(f),
                };
                f.write_str(sentence)
            }
        }
    };
}

rule_kinds! {
    units {
        View3dAzimuthNotFinite => "view_3d.azimuth_degrees must be finite",
        View3dElevationOutOfRange => "view_3d.elevation_degrees must be in -90..=90",
        View3dDistanceOutOfRange => "view_3d.distance must be in 1.2..=100",
        View3dBoxAspectOutOfRange => "view_3d.box_aspect lengths must be in 0.05..=20",
        View3dCategoricalAxis => "3D chart axes (bottom_x, left_y, right_y) cannot be categorical",
        View3dAxisScale => "3D chart axes must be linear or logarithmic",
        View3dOverlay => "3D charts have no overlay layers",
        View3dLineOffsets => "3D charts have no line offsets",
        View3dDrawStyle => "3D charts are drawn in the Precise style only",
        View3dStem => "stems draw flat: not on a 3D chart",
        View3dPointsWithoutZColumn => "a scatter or line series on a 3D chart names its height column (z_column)",
        View3dStepLine => "a line on a 3D chart is linear: no step interpolation",
        View3dLineFill => "a line on a 3D chart has no area fill",
        View3dGridVector => "a grid vector draws flat: not on a 3D chart",
        View3dVectorWithoutZColumn => "a vector series on a 3D chart names its anchor height column (z_column)",
        View3dVectorWithoutW => "a vector series on a 3D chart names its z component (w)",
        View3dImage => "an image draws flat: not on a 3D chart",
        View3dParallel => "a parallel series draws flat: not on a 3D chart",
        View3dSeriesKind => "a 3D chart draws only heatmaps, contours, heatmap contours, and the scatter, line and point vector series that name a z_column",
        View3dHexagonalGrid => "a hexagonal grid draws flat: not on a 3D chart",
        View3dBandedSurface => "a surface's fill is continuous: banded fills are not drawn in 3D",
        View3dContourLabels => "contours on a 3D chart's floor have no labels",
        SurfaceColorMatrixShape => "a surface's color_matrix has the column count, orientation and layout of its matrix",
        SurfaceColorMatrixColumns => "a surface's color_matrix names its columns",
        SurfaceWireframeStyle => "a surface wireframe has a width in 0..=24 and a colour in 0..=1",
        SurfaceMaterialStyle => "a surface's gloss, texture_strength and emphasis_brightness are in 0..=1 and its texture_scale is positive",
        RadialSliceCount => "a radial chart needs 1..64 slices",
        RadialSliceValue => "slice values must be finite and nonnegative",
        RadialSliceColor => "slice colors must be finite opaque RGBA in 0..1",
        RadialSliceLabelColor => "invalid slice label color",
        RadialSliceExplode => "slice explode must be in 0..0.35",
        RadialSliceLabel => "slice labels must be single-line, at most 80 characters",
        RadialSliceLabelText => "slice label text must have at most two lines of at most 400 characters, without control characters",
        RadialSliceTotal => "slice total must be positive and finite",
        RadialStyle => "invalid radial material, light, or projection",
        RadialTitle => "invalid radial title: size 6..160, at most 512 characters / 8 lines, finite styles",
        RadialTitleOffsets => "a radial chart title keeps its position in decorations: its offsets and top_margin must be 0",
        RadialShape => "invalid radial shape or start angle",
        RadialTextOrBackground => "invalid radial text or background",
        RadialLabelFormatOrInteraction => "invalid radial label format or interaction",
        RadialInteractionTarget => "radial interaction target is out of range",
        RadialEmphasizedZeroSlice => "cannot emphasize a zero-valued slice",
        SunburstDetail => "a sunburst has no detail circle",
        RadialSplitIndex => "split slice index is out of range",
        RadialDetailSum => "detail values must sum to the parent slice value",
        RadialDetailShape => "invalid detail chart shape",
        SunburstRingCount => "a sunburst has at most 8 rings",
        SunburstNodeCount => "a sunburst ring has at most 64 nodes",
        SunburstTilt => "a sunburst draws flat: no tilt or depth",
        SunburstChildrenSum => "a node's children sum to at most its value",
        EnergyLevelsSourceWithRecords => "an energy-level series with a source holds no records of its own",
        EnergyLevelIds => "energy level ids must be non-empty and unique",
        IonizationLimitIds => "ionization limit ids must be non-empty and unique",
        TransitionIds => "transition ids must be non-empty and unique",
        LevelBandIds => "level band ids must be non-empty and unique",
        LevelBandEnds => "a level band's first and last name levels of its column",
        LevelBandLabelOffset => "a level band's label offset is out of range",
        FineStructureCentroid => "a fine-structure component must name a centroid level",
        TransitionSameLevel => "a transition joins two different levels",
        TransitionLevels => "a transition must name two levels of its series",
        EnergyLevelEnergy => "an energy level's energy must be finite",
        EnergyLevelUncertainty => "an energy level's uncertainty and g must be finite and not negative",
        EnergyLevelText => "an energy level's column must be set and its text fields one line of at most 160 characters",
        ComponentMultipletId => "a component's multiplet id is one line of at most 160 characters",
        EnergyLevelLabel => "invalid energy level label",
        IonizationLimitEnergy => "an ionization limit's energy must be finite and its uncertainty not negative",
        IonizationLimitLabel => "invalid ionization limit label or reference",
        TransitionValues => "a transition's values must be finite and its uncertainties not negative",
        TransitionXOffset => "a transition's x_offset must be in -0.5..=0.5 of the slot",
        TransitionText => "a transition's text fields are one line of at most 160 characters",
        TransitionLabel => "invalid transition label",
        EnergyLevelStroke => "invalid energy level stroke style",
        IonizationLimitLine => "invalid ionization limit line style",
        TransitionArrow => "invalid transition arrow style",
        EnergyLevelsPickedWidth => "picked_width_scale must be in 1..=4",
        LevelBandStyle => "invalid level band style",
        WavelengthLabels => "invalid wavelength labels: a text of size 6..160 and at most 17 decimals",
        WavelengthUnit => "a wavelength label unit is one line of at most 160 characters",
        EnergyLevelsColumns => "an energy-level series reads no columns: x_column and y_column stay empty",
        EnergyLevelsSelfSource => "an energy-level series cannot be its own source",
        EnergyLevelsChart => "an energy-level series is drawn on a rectangular chart",
        EnergyLevelsCategoricalX => "an energy-level series needs a categorical bottom_x: its term columns",
        BoxStatsOrder => "box statistics must be finite and low <= Q1 <= median <= Q3 <= high",
        BoxStatsMeanOrCount => "mean must be finite; sample count must be positive when supplied",
        BoxStatsMedianCi => "median CI must be finite and contain the median",
        BoxStatsOutliers => "outliers must be finite and strictly outside the whiskers",
        BoxPlotChartLimits => "boxplot limits: at least 1 category and 1 series",
        BoxPlotChartCategory => "invalid category ID/label",
        BoxPlotChartSeries => "invalid series ID/label, color or value count",
        BoxPlotChartStyle => "invalid boxplot layout, font or colors",
        BoxPlotChartTitle => "invalid title",
        BoxPlotChartValueRange => "invalid linear value range",
        BoxPlotChartOverride => "invalid or duplicate box override",
        BoxPlotChartUnresolvedStyle => "internal: a box with a value resolved no style",
        BoxPlotChartNotchWithoutCi => "notched boxes require an explicit median CI",
        BoxPlotChartInteractionTarget => "unknown or missing interaction target",
        PolarAngles => "polar angles must be finite",
        PolarInnerRadius => "polar inner_radius_fraction must be within 0..=0.9",
        PolarSector => "polar sector must be a finite [start, end] with start < end",
        PolarSectorTurn => "polar sector cannot exceed one turn",
        PolarArcPieces => "polar arc pieces must be within 1..=64",
        PolarAngleAxisScale => "the polar angle axis (bottom_x) must be linear",
        PolarAngleAxisInverted => "the polar angle axis (bottom_x) cannot be inverted; use clockwise",
        PolarPolygonGridAxis => "a polygon polar grid joins its rings at the categories: it needs a categorical angle axis (bottom_x.categories)",
        PolarRadiusCategorical => "the polar radius axis (left_y) cannot be categorical",
        PolarRadiusScale => "the polar radius axis (left_y) is linear or logarithmic",
        PolarOverlay => "polar charts have no overlay layers",
        PolarLineOffsets => "polar charts have no line offsets",
        DataAreaTooSmall => "data area must fit the measured plot region and be at least 80 by 60 logical pixels",
        DecorationOffsetCount => "too many decoration offsets: a chart holds at most 1024; drop the offsets of elements the chart no longer has",
        DecorationOffsets => "decoration offsets must be finite and unique",
        DecorationAxisMove => "axes only move perpendicular to their direction",
        RadarChartLimits => "radar limits: at least 3 categories and 1 series",
        RadarChartSeriesStyle => "invalid radar style",
        RadarChartStyle => "invalid radar layout, font or colors",
        RadarChartValueRange => "invalid value range",
        RadarChartRingSpacing => "ring spacing must be finite and positive",
        RadarChartInteractionTarget => "unknown interaction target",
        AxisCrossAt => "an axis crossing (`cross_at`) must be finite",
        CategoryWindow => "a category window needs finite bounds with from < to",
        AxisBoundsNotFinite => "axis bounds must be finite",
        AxisBoundsOrder => "axis max must be greater than min",
        LogAxisBounds => "logarithmic axis bounds must be positive",
        ProbabilityAxisBounds => "probability axis bounds must lie strictly between 0 and 1",
        AxisUnitScale => "an axis unit's scale must be finite and non-zero",
        AxisUnitOffset => "an axis unit's offset must be finite",
        AxisUnitLog => "a logarithmic axis takes a unit with offset 0 and a positive scale",
        AxisUnitProbability => "a probability axis cannot have a unit",
        AxisUnitTimestamp => "an axis with timestamp labels cannot have a unit",
        AxisUnitCategorical => "a categorical axis cannot have a unit",
        SeriesIdEmpty => "series_id must not be empty",
        SeriesIdDuplicate => "series_id must be unique within a chart",
        PolarErrorBars => "error bars are not drawn on polar axes",
        PolarStems => "stems are not drawn on polar axes",
        PolarHistogram => "histogram series are not drawn on polar axes",
        PolarHexagonalGrid => "a hexagonal grid is drawn on a rectangular chart",
        PolarHeatmapPolygonGrid => "a polar heatmap's cells follow circular rings: not on a polygon grid",
        PolarImage => "images are not drawn on polar axes",
        PolarVector => "vector series are not drawn on polar axes",
        PolarBoxPlot => "box plots are not drawn on polar axes",
        PolarRangeBars => "range bars are not drawn on polar axes",
        PolarStreamlines => "streamlines are not drawn on polar axes",
        PolarParallel => "parallel series are drawn on parallel charts, not polar axes",
        PolarEnergyLevels => "energy-level series are not drawn on polar axes",
        PolarTriMesh => "a triangle mesh is not drawn on polar axes",
        PolarFeynman => "Feynman series are not drawn on polar axes",
        PolarTernary => "ternary series are drawn on ternary charts, not polar axes",
        PolarArcStepLine => "a polar arc interpolation needs a linearly interpolated line; a step interpolation draws chords",
        CategoricalChartBarMaterial => "material not supported by bars",
        CategoricalChartLimits => "categorical limits: at least 1 category and 1 series",
        CategoricalChartCategory => "invalid or duplicate category ID/label",
        CategoricalChartSeries => "invalid series, value count, color or style",
        CategoricalChartValues => "values must be finite; percent stacks require nonnegative values",
        CategoricalChartStyle => "invalid categorical style or layout",
        CategoricalChartTitles => "titles must be single-line, at most 160 characters",
        CategoricalChartValueSuffix => "invalid value suffix",
        CategoricalChartOverride => "invalid or duplicate bar override",
        CategoricalChartInteractionTarget => "interaction target is missing or unknown",
        CategoricalChartStackOverflow => "stack total overflow",
        BoundedTitle => "invalid bounded title: size 6..160, at most 512 characters / 8 lines, finite styles",
        TriMeshTriangles => "a triangle mesh names its triangle index column",
        TriMeshValues => "a triangle mesh names its value column",
        TriMeshNothingDrawn => "a triangle mesh draws a fill, contour lines or both",
        TriMeshSurfaceWithoutFill => "a surface look needs the triangle mesh's fill",
        TriMeshVertexValuesFlat => "vertex values are shaded Interpolated: a flat triangle colour from vertex values is host work (give face values)",
        TriMeshFaceValuesInterpolated => "face values are shaded Flat: interpolating them needs vertex values, which are host work",
        TriMeshContourFaceValues => "contour lines need vertex values: a field constant on each triangle has no isolines",
        TriMeshContourLabels => "contour lines on a triangle mesh have no labels",
        TriMeshSurfaceColorMatrix => "a triangle mesh is coloured by its values: no surface color_matrix",
        TriMeshVertexColumns => "a triangle mesh names its vertex coordinate columns (x_column, y_column)",
        TriMeshTernary => "a triangle mesh is not drawn on a ternary chart",
        TriMeshParallel => "a triangle mesh is not drawn on a parallel chart",
        TriMeshWithoutZColumn => "a triangle mesh on a 3D chart names its vertex height column (z_column)",
        SubplotNested => "a subplot cannot have subplots of its own",
        SubplotAnnotations => "the document's annotation layer belongs to the parent, not a subplot",
        SubplotLink => "a subplot link must point at the parent or an earlier subplot",
        SeriesInManyLayers => "a series id may belong to at most one subplot or overlay layer",
        SeriesInManyOverlays => "a series id may belong to at most one overlay layer",
        LayerSeriesUnknown => "a layer names a series the chart has not registered",
        PointSizeFitNoMap => "the series has no point size map to fit",
        PointSizeFitExtent => "the size column has no finite, non-empty extent to fit the size domain to",
        VectorFitNotVector => "the series is not a vector field",
        VectorFitTarget => "a vector fit target must be finite and positive",
        VectorFitExtent => "the vector components have no finite, non-zero extent to fit the scale to",
        CategoryGroupCategories => "a category group names categories of its axis",
        CategoryGroupOrder => "a category group's first category comes before its last",
        CategoryGroupOverlap => "category groups run in axis order without overlapping",
        CategoryGroupLabel => "invalid category group label",
        CategoryGroupOffset => "invalid category group offset",
        BreakCategorical => "a categorical axis cannot be broken",
        BreakCount => "an axis can cut out at most three ranges",
        BreakRange => "a cut range needs finite bounds with from < to",
        BreakLogRange => "a cut range on a logarithmic axis must be positive",
        BreakProbabilityRange => "a cut range on a probability axis must lie strictly between 0 and 1",
        BreakOrder => "cut ranges must be sorted and must not touch",
        BreakGap => "a break gap must be between 0 and 200 px",
        BreakSizing => "only a Continuous broken axis sizes its pieces other than by value",
        BreakWeightCount => "piece weights need one weight per piece: one more than cut ranges",
        BreakWeights => "piece weights must be finite and positive",
        BreakMark => "a break mark needs a size in 0..=64, a width in 0..=16 and a colour in range",
        CategoriesEmpty => "a categorical axis needs at least one category",
        CategoryIds => "category ids must be unique, non-empty, single-line and at most 80 characters",
        CategoryLabels => "category labels must be single-line and at most 80 characters",
        CategoryRichLabel => "invalid category rich label",
        CategoricalLogAxis => "a categorical axis cannot be logarithmic",
        CategoricalProbabilityAxis => "a categorical axis cannot be a probability axis",
        StemBaseline => "a stem baseline must be finite",
        StemWidth => "a stem width must be finite and at least 0",
        XyNothingDrawn => "an xy series draws markers, a line or both",
        ErrorBarsWithoutMarkers => "error bars are drawn with markers",
        ErrorBarsDirection => "error bars run along x, y or both",
        StemsWithoutMarkers => "stems are drawn with markers",
        FieldNothingDrawn => "a field draws a fill, contour lines or both",
        FieldSurfaceWithoutFill => "a surface look needs the field's fill",
        XyWireNothingDrawn => "an xy series draws markers or a line",
        StreamlineArrows => "streamline arrows need a finite spacing_px > 0, finite non-negative head_length_px and head_width_px, and spacing_px >= head_length_px",
        StreamlineInterpolation => "streamlines are polylines: the line interpolation must be Linear",
        StreamlineFill => "streamlines have no fill",
        StreamlineColumns => "a streamline series names its x_column and y_column",
        StreamlineColorMapColumn => "a streamline colour map names its column",
        MonotonicGroupEmpty => "a monotonic series names its group",
        MonotonicInterpolation => "a monotonic series is a polyline: the line interpolation must be Linear",
        MonotonicFill => "a monotonic series has no fill",
        MonotonicColumns => "a monotonic series names two distinct envelope columns in x_column and y_column",
        MonotonicColumnsShared => "a monotonic series' envelope columns are its own: no other series of the chart reads them",
        MonotonicChartKind => "monotonic series are drawn on rectangular 2D charts, not ternary, parallel or 3D ones",
        MonotonicXAxis => "a monotonic series needs a linear, numeric, unbroken x axis",
        PolarMonotonic => "monotonic series are not drawn on polar axes",
        StreamedMonotonic => "monotonic series are fed by their own calls; streamed displays do not draw them",
        MonotonicGeneralChart => "a monotonic line series lives only in a chart registered as a monotonic chart",
        MonotonicMappingEdit => "a monotonic chart's x range, x scale, breaks, direction and plot area change only through its monotonic calls",
        MonotonicRange => "a monotonic series' x range must be finite and increasing",
        VectorPixels => "vector width_px, head_length_px, head_width_px and min_length_px must be finite and non-negative",
        VectorScale => "vector scale must be finite and non-zero",
        PointSizeMapDomain => "point size map domain must be finite with domain_min < domain_max",
        PointSizeMapPixels => "point size map min_px and max_px must be finite, non-negative and min_px <= max_px",
        ContourLabelSpacing => "contour label spacing_px must be finite and greater than zero",
        ContourLabelAnchors => "contour label anchors exceed the supported maximum of 1024",
        ValueLabelOffsetCount => "value label offsets need one entry per category",
        ValueLabelOffset => "a value label offset is finite and at most 4096 px along either axis",
        BarStyle => "invalid bar style",
        BarGroups => "bar groups need a group width in 0.1..=0.95 and a bar gap in 0..=24 px",
        LineOffsetSteps => "line offset steps must be finite",
        LineOffsetMember => "a line offset member is a non-empty series id, listed once",
        LineOffsetMemberLayer => "a line offset member must be drawn by the chart that lists it",
        LineOffsetMemberUnknown => "a line offset member names no series of the chart",
        LineOffsetMemberNotLine => "a line offset member must draw a line: Line, ScatterLine or a LineScatterErrorbar kind",
        LineOffsetFill => "a ToSeries fill cannot join series with different line offsets",
        BarColor => "a bar colour must be opaque with channels in 0..=1",
        BarLabelFormat => "bar labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
        BarLabelTextCount => "bar label text needs one string per category",
        BarLabel => "a bar label is one line of at most 160 characters",
        BarOverride => "a bar override names a category of the axis and its own series, once",
        HorizontalBarsOnOverlay => "horizontal category bars cannot sit on an overlay layer: its own axis is a value axis on y",
        RoseDirection => "a rose fans its bars around the centre: the group direction on a polar chart is vertical",
        CategoryBarsAxis => "category bars need a categorical axis: bottom_x for vertical bars, left_y for horizontal ones",
        CategoryBarsValueAxis => "category bars need a linear numeric value axis",
        CategoryBarsValueColumns => "the category bar series of a group need distinct value columns",
        BoxMaterial => "material not supported by boxes",
        BoxStyle => "invalid boxplot style",
        BoxColor => "a box colour must be opaque with channels in 0..=1",
        BoxLabelFormat => "box labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
        BoxLabelTextCount => "box label text needs one string per category",
        BoxLabel => "a box label is one line of at most 160 characters",
        BoxPlotColumns => "a box plot series names its category positions (x_column), its record (y_column) unless it draws a violin, and, when set, its outlier column",
        ViolinBoxWidth => "a violin's box width is a share of the slot in 0..=1",
        ViolinHalves => "a split violin draws at least one half",
        ViolinHalfColumns => "a violin half names a ragged curve and a distinct density column",
        ViolinDensityScale => "a violin half's density_scale is finite and positive",
        ViolinOutline => "a violin outline is 0..=8 px wide with channels in 0..=1, its fill opaque",
        BoxPointsColumns => "box points name distinct position and value columns",
        BoxPointsStyle => "box points are 1..=24 px, jittered by 0..=1 and offset by -1..=1 of the half slot, with colour channels in 0..=1",
        BoxOverride => "a box override names a category of the axis and its own series, once",
        BarsAndBoxesInLayer => "a layer draws either category bars or box plots, not both",
        BoxPlotsStacked => "box plots stand side by side: their bar_groups mode must be Grouped, not Stacked or PercentStacked",
        HorizontalBoxPlotsOnOverlay => "horizontal box plots cannot sit on an overlay layer: its own axis is a value axis on y",
        BoxPlotsAxis => "box plots need a categorical axis: bottom_x for vertical boxes, left_y for horizontal ones",
        BoxPlotsValueAxis => "box plots need a linear numeric value axis",
        RangeBarColor => "a range bar colour must be opaque with channels in 0..=1",
        RangeBarColumns => "a range bar series names its positions (x_column), its end values (y_column), its start values and every optional column",
        RangeBarColumnsDistinct => "the columns of a range bar series are distinct",
        RangeBarWhiskers => "range bar whiskers are 0..=24 px wide, in an opaque colour",
        RangeBarConnector => "a range bar connector is 0..=24 px wide, its colour channels in 0..=1",
        RangeBarStateColors => "range bar direction, state and row colours must be opaque",
        RangeBarRemainingOpacity => "a range bar's remaining opacity is in 0..=1",
        RangeBarSlotWidth => "a range bar slot width is finite and greater than 0",
        RangeBarLabelFormat => "range bar labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
        RangeBarLabel => "a range bar label is one line of at most 160 characters",
        RangeBarContrastInk => "a range bar's contrast ink colours must be opaque",
        ContrastInkSeries => "only range bar labels take contrast ink",
        CategoryBandsFill => "a category band fill has every channel in 0..=1",
        TimestampStepRequired => "relative, custom and period timestamp labels need a calendar step",
        TimestampRelativePrefix => "a relative timestamp label prefix is one line of at most 24 characters",
        TimestampCustomLabel => "a custom timestamp label is one line of at most 160 characters",
        RangeBarsWithOtherBars => "a layer draws either category bars, box plots or range bars",
        RangeBarsStacked => "range bars stand side by side: their bar_groups mode must be Grouped, not Stacked or PercentStacked",
        HorizontalRangeBarsOnOverlay => "horizontal range bars cannot sit on an overlay layer: its own axis is a value axis on y",
        RangeBarsValueAxis => "range bars need a numeric value axis",
        RangeBarsPositionAxis => "range bars need a categorical or linear numeric position axis: bottom_x for vertical bars, left_y for horizontal ones",
        RangeBarConnectorLayer => "a range bar connector needs its series alone in its layer",
        RangeBarSlotWidthAxis => "a range bar series on a categorical position axis has no slot_width; on a numeric one it needs one",
        RadarSeriesStyle => "invalid radar series style",
        RadarColor => "a radar colour needs channels in 0..=1",
        RadarLabelFormat => "radar labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
        RadarLabelTextCount => "radar label text needs one string per category",
        RadarLabel => "a radar label is one line of at most 160 characters",
        RadarColumns => "a radar series names its category index column (x_column) and its value column (y_column)",
        HexagonalGridVector => "a grid vector reads a rectangular lattice: not a hexagonal grid",
        HexagonalContours => "contours read a rectangular lattice: a hexagonal grid draws as a Heatmap only",
        HexagonalShading => "a hexagonal grid is shaded flat",
        HexagonalAxes => "a hexagonal grid needs linear x and y axes: its lattice is uniform",
        HexagonalBrokenAxis => "a hexagonal grid's lattice is uniform: not on a Continuous broken axis",
        ImageChannelColumns => "an image's channels share one lattice: the same number of columns",
        ImageChannelLattice => "an image's channels share one lattice: the same orientation and grid layout",
        ImageRange => "an image's range is finite with a smaller low end",
        ImageOpacity => "an image's opacity is 0..=1",
        RadarNotPolar => "radar series are drawn on a polar chart (Config.polar)",
        RadarAngleAxis => "radar series need a categorical angle axis: bottom_x.categories on the polar chart",
        RadarInterpolation => "radar polygons have straight edges: the polar interpolation must be Chord",
        SelectedMatrixGone => "selected matrix no longer exists",
        ParallelAxisRange => "a parallel axis's range is finite with a smaller min",
        ParallelLogAxisRange => "a logarithmic parallel axis's range is positive",
        ParallelAxisScale => "a parallel axis is linear or logarithmic: not a probability axis",
        ParallelAxisLabelOffset => "a parallel axis's label offset is finite",
        ParallelAxisCount => "a parallel chart has at least two axes",
        ParallelBottomXCategorical => "a parallel chart's bottom_x is categorical: one category per axis",
        ParallelBottomXCategories => "a parallel chart's bottom_x has one category per axis",
        ParallelLeftY => "a parallel chart's left_y spans the normalized band: linear 0..1, not inverted",
        ParallelOverlay => "parallel charts have no overlay layers",
        ParallelLineOffsets => "parallel charts have no line offsets",
        ParallelDrawStyle => "parallel charts are drawn in the Precise style only",
        ParallelSubplotLinks => "parallel charts take no subplot axis links",
        ParallelSeriesNotParallel => "a parallel series is drawn on a parallel chart (Config.parallel)",
        ParallelSeriesColumnCount => "a parallel series names one column per axis",
        ParallelSeriesTooFewColumns => "a parallel series names one column per axis, at least two",
        ParallelSeriesColumnNames => "a parallel series' axis columns are named",
        ParallelSeriesXY => "a parallel series' x_column and y_column are its first two axis columns",
        ParallelSeriesStepLine => "a parallel series' line is linear: no step interpolation",
        ParallelSeriesLineFill => "a parallel series' line has no area fill",
        ParallelSeriesDashed => "a parallel series' line is solid",
        ParallelSeriesColorColumn => "a parallel series' colour column is named",
        ParallelSeriesColorbar => "a parallel series coloured by a column needs Config.colorbar",
        ParallelChartSeriesKind => "a parallel chart draws only parallel series",
        TernaryAxisScale => "ternary axes (bottom_x, left_y, right_y) must be linear",
        TernaryAxisInverted => "ternary axes cannot be inverted: each increases clockwise",
        TernaryAxisCategorical => "ternary axes cannot be categorical",
        TernaryAxisSpan => "ternary axes span the whole triangle: every axis has min 0 and the same positive max (1 or 100)",
        TernaryOverlay => "ternary charts have no overlay layers",
        TernaryLineOffsets => "ternary charts have no line offsets",
        TernaryDrawStyle => "ternary charts are drawn in the Precise style only",
        TernarySubplotLinks => "ternary charts take no subplot axis links",
        TernarySeriesNotTernary => "a ternary series is drawn on a ternary chart (Config.ternary)",
        TernarySeriesColumns => "a ternary series names three distinct component columns (x_column, y_column, c_column)",
        TernaryNothingDrawn => "a ternary series draws markers, a line, a region fill or any of them",
        TernaryRegionStyle => "a ternary region fill's colour and opacity are finite and in 0..=1",
        TernaryStepLine => "a ternary line is linear: step modes are not drawn",
        TernaryLineFill => "a ternary line has no fill",
        TernaryChartSeriesKind => "a ternary chart draws only ternary series",
        AnnotationId => "annotation id must be at most 160 characters without control characters",
        AnnotationBox => "annotation box border width and padding must be 0..64",
        AnnotationArrowHead => "annotation arrow head must be 0..256 logical pixels",
        AnnotationPolygon => "annotation polygon needs at least three vertices",
        AnnotationColor => "annotation colors must be finite 0..1 RGBA",
        AnnotationLineWidth => "annotation line width must be 0..64 logical pixels",
        AnnotationText => "invalid annotation text: size 6..160, at most 512 characters / 8 lines, finite styles",
        AnnotationCount => "too many annotations: a chart holds at most 1024 (MAX_ANNOTATIONS); remove some or merge them",
        FeynmanLineStyle => "invalid Feynman line style",
        FeynmanWave => "invalid Feynman wave or coil",
        FeynmanSizes => "invalid Feynman marker, arrow, loop, fan or label size",
        FeynmanPickedStyle => "invalid Feynman picked style: a colour and an extra width in 0..=16",
        FeynmanVertexIds => "Feynman vertex ids must be unique",
        FeynmanPropagatorIds => "Feynman propagator ids must be unique",
        FeynmanVertexPosition => "a Feynman vertex position must be finite",
        FeynmanBlob => "invalid Feynman blob marker",
        FeynmanVertexLabel => "invalid Feynman vertex label",
        FeynmanPropagatorVertices => "a propagator must name two vertices of its series",
        FeynmanBend => "a propagator's bend must be in -170..=170 degrees",
        FeynmanPropagatorStyle => "invalid propagator colour or width",
        FeynmanPropagatorLabel => "invalid propagator label",
        FeynmanMomentumLabel => "invalid momentum label",
        FeynmanColumns => "a Feynman series reads no columns: x_column and y_column stay empty",
        FeynmanChart => "a Feynman series is drawn on a rectangular chart",
        FeynmanNumericX => "a Feynman series is placed on a numeric bottom_x",
        ColorbarCategorical => "a colourbar axis cannot be categorical",
        RgbaBufferLength => "the RGBA8 buffer length is not width * height * 4",
        VectorExportWholeDocument => "a vector export renders the whole document",
        FitPadding => "padding must be finite and non-negative",
        FitScopeLayer => "a fit scope names a layer the chart does not have",
        LayerOutsideChart => "a layer's area lies outside the chart area",
        NoFeasibleLayout => "the layered document has no feasible layout",
        SeriesPaintOrderOverflow => "series paint order exceeds the GPU u32 index space",
        HistogramBinCountOverflow => "histogram bin count exceeds the GPU u32 index space",
        ValueLabelPaintOrderOverflow => "value label paint order exceeds the GPU u32 index space",
        StreamedPointValueMap => "point value maps require GPU-resident columns; streamed sources are not supported",
        ContourLabelSpacingScaled => "automatic contour label spacing_px * scale must be finite and greater than zero",
        TriMeshColorbar => "a triangle mesh series needs Config.colorbar",
        GridColumnOffsetOverflow => "grid column offset exceeds a 32-bit lane index",
        FieldColorbarRequired => "a field series needs Config.colorbar",
        LineFillStyleMissing => "series style was created without its line fill; recreate the style for the current series config",
        ErrorBarStyleMapIndex => "errorbar style map expects an index column",
        ParallelValueMapMissing => "a colour-mapped parallel series needs its value map prepared; recreate the chart's preparation",
        RegionDispatchGrid => "the region's rows exceed the adapter's dispatch grid",
        StreamedPolar => "polar charts require GPU-resident columns; streamed sources are not supported",
        StreamedContoursOrStars => "stream pass does not yet support contours or arc-driven stars",
        StreamedCategoricalAxis => "categorical axes require GPU-resident columns; streamed sources are not supported",
        StreamedLineFillOrStep => "line fill and step interpolation require GPU-resident columns; streamed sources are not supported",
        Streamed3d => "3D chart series require GPU-resident columns; streamed sources are not supported",
        StreamedPointMaps => "point colour and size maps require GPU-resident columns; streamed sources are not supported",
        StreamedFieldExecutor => "streamed field requires the bounded tile executor",
        StreamedLineArcPrefix => "streamed line requires a prepared canonical arc prefix",
        StreamedPrimitiveColumns => "streamed primitive has no supplied columns",
        StreamedPrimitiveRows => "streamed primitive ranges must describe the same global rows",
        ScatterColorMapColorbar => "a colour-mapped scatter needs Config.colorbar: it owns the z range and colormap",
        ScatterColorMapNeedsColorbar => "a colour-mapped scatter needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw",
        FieldNeedsColorbar => "a field series needs Config.colorbar: it owns the z range and colormap, so without it there is no value to draw",
        ParallelColorMapNeedsColorbar => "a colour-mapped parallel series needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw",
        SeriesNotInChart => "no series with this id in the chart",
        LineOffsetMemberStreamed => "a line offset member must be resident: streamed sources are not drawn with line offsets",
        PlaceFeynmanVerticesNotFeynman => "place_feynman_vertices names a Feynman series of the chart",
        FeynmanLabelDrag => "a Feynman label moves through FeynmanSeries::drag_label, the value-label drag",
        DragFeynmanNotFeynman => "drag_feynman names a Feynman series of the chart",
        ViewStateOverlayRanges => "a view state must carry one Y and one X range per overlay layer",
        ViewStateSubplots => "a view state must carry one view per subplot, each with one Y and one X range per overlay layer",
        RebasedSelectionStyleMap => "rebased selection needs a global-index style map binding",
        ScatterStyleMapIndex => "scatter style map expects an index column",
        NotBoxPlot => "not a box plot series",
        NotRangeBars => "not a range bar series",
        StreamOperationCap => "must be nonzero and no larger than the initial operation cap",
        NonresidentOverlays => "nonresident streaming is not supported for a document with overlay layers",
        NonresidentSubplots => "nonresident streaming is not supported for a document with subplots",
        NonresidentLineOffsets => "nonresident streaming is not supported for a document with line offsets",
        StreamLimits => "limits must be non-zero and chunk bytes must fit the in-flight budget",
        NotFinitePositive => "must be finite and positive",
        StreamExecutionCap => "must be non-zero and no greater than the execution's initial cap",
        StreamedVector => "vector series require GPU-resident columns; streamed sources are not supported",
        StreamedCategoryBars => "category bar series require GPU-resident columns; streamed sources are not supported",
        StreamedBoxPlot => "box plot series require a GPU-resident record, outlier column, violin curves and raw point columns; streamed sources are not supported",
        StreamedRadar => "radar series require GPU-resident columns; streamed sources are not supported",
        StreamDrawCursorField => "stream draw cursor does not yet support field or contour passes",
        GlyphTooLarge => "a glyph larger than 4096 px (MAX_GLYPH_PX) is not rasterized",
        ChartAreaEmpty => "the chart area must have a non-zero width and height",
        ColorbarProbabilityAxis => "a colourbar axis is linear or logarithmic: not a probability axis",
        CategoryGroupsAxis => "category groups are drawn on a categorical top_x or bottom_x",
        BreaksAxis => "breaks go on bottom_x, left_y or an overlay's x_axis or y_axis",
        BreaksChart => "a broken axis needs a rectangular chart",
        LayoutDataAreaEmpty => "margins and visual dimensions must produce a non-empty data area",
        OverlayDataArea => "the overlay layers must share one non-empty data area",
        SubplotIndex => "subplot index out of range",
        FeynmanAllocation => "the Feynman series' working memory could not be allocated",
        FeynmanLayoutDidNotConverge => "the Feynman layout did not converge",
        RecordWithoutFields => "a record needs at least one field",
        RecordWithoutRows => "a record needs at least one row",
        SunburstRing => "sunburst has no such ring",
        RadialNoDetail => "radial chart has no detail circle",
        RadialDataAreaTooSmall => "radial data area is too small",
        RadialLabelCanvas => "could not allocate label canvas",
        RadialTargetFormat => "radial target must be RGBA8 or BGRA8",
        RadialLabelWithoutText => "a shown radial label has no text; call RadialChart::format_labels after editing slices, or normalize_labels on a loaded document",
        RadialCanvasSize => "radial canvas must be at least 160x160; scale must be at least 0.5",
        RadialTextureLimit => "radial output exceeds the device texture limit",
        RadialReadbackLimit => "radial readback exceeds the device buffer limit",
        TriMeshNeedsColorbar => "a triangle mesh series needs Config.colorbar: it owns the z range and colormap, so without it there is no value to draw",
        StreamlineColorMapNeedsColorbar => "colour-mapped streamlines need Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw",
        VectorGridLayout => "vector u and v grids must share orientation and grid layout",
        VectorColorGridLayout => "vector colour grid must share the u grid's orientation and grid layout",
        VectorColorMapForm => "vector colour map must take the components' form: a column for points, a grid for grids",
        VectorColorMapNeedsColorbar => "a colour-mapped vector needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw",
        LineFillBaseline => "line fill baseline must be finite",
        LineFillSelf => "line fill cannot reference the series itself",
        NonresidentOverlaySeries => "a nonresident streaming series cannot be scaled by an overlay layer",
        NonresidentSubplotSeries => "a nonresident streaming series cannot be drawn by a subplot",
        ParallelColumnAlignment => "parallel column byte offset is not f32-aligned",
        ParallelColumnLaneOverflow => "parallel column lane past u32",
        ParallelRowCountOverflow => "parallel row count past u32",
        ParallelAxisCountOverflow => "parallel axis count past u32",
        NotCategoryBars => "not a category bar series",
        NotInBarGroup => "the series is not in its bar group",
        NotInBoxGroup => "the series is not in its box group",
        BoxOutlierGroupTooLong => "a box plot outlier group is too long to draw",
        NoViolin => "the box plot series draws no violin",
        ViolinCurvesTooLong => "the violin curves are too long to draw",
        NoBoxPoints => "the box plot series draws no points",
        NotInRangeGroup => "the series is not in its range group",
        RegionColumnAlignment => "region column byte offset is not f32-aligned",
        RegionColumnLaneOverflow => "region column lies past the 32-bit lane range",
        RegionRowsOverflow => "region rows exceed the 32-bit row range",
        ColumnOffsetLaneOverflow => "column offset exceeds a 32-bit lane index",
        VectorGridCellsOverflow => "vector grid cell count exceeds the u32 instance space",
        VectorColorMapColorbar => "a colour-mapped vector needs Config.colorbar: it owns the z range and colormap",
        RaggedEmpty => "a ragged column needs at least one value; register none when every group is empty",
        RaggedOffsetsOrder => "the offsets source must write every offset once, in row order",
        RaggedOffsetsInteger => "ragged offsets must be finite non-negative integers",
        RaggedOffsetsDecreasing => "ragged offsets must be non-decreasing: group g is values[offsets[g]..offsets[g + 1]]";
            doc r"ragged offsets must be non-decreasing: group g is values\[offsets\[g\]..offsets\[g + 1\]\]",
        TrianglesEmpty => "a triangle index column needs at least one triangle (three indices)",
        TriangleIndicesOrder => "the triangle index source must write every index once, in row order",
        StreamedRangeBars => "range bar series require GPU-resident columns; streamed sources are not supported",
        StreamedHistogram => "histogram series require GPU-resident columns; streamed sources are not supported",
        StreamedStreamlines => "streamline series require GPU-resident columns; streamed sources are not supported",
        StreamedHexagonal => "hexagonal grids require GPU-resident columns; streamed sources are not supported",
        StreamedTernary => "ternary series require GPU-resident columns; streamed sources are not supported",
        StreamedBreak => "broken axes require GPU-resident columns; streamed sources are not supported",
        StreamedProbability => "probability axes require GPU-resident columns; streamed sources are not supported",
        StreamedImage => "image series require GPU-resident columns; streamed sources are not supported",
        StreamedParallel => "parallel coordinates series require GPU-resident columns; streamed sources are not supported",
        StreamedTriMesh => "triangle mesh series require GPU-resident columns; streamed sources are not supported",
        StreamedStem => "stem series require GPU-resident columns; streamed sources are not supported",
        StreamedEnergyLevels => "energy-level series are DataLess (SSOT only); streamed displays do not draw them",
        StreamedFeynman => "Feynman series are DataLess (SSOT only); streamed displays do not draw them",
    }
    fielded {
        /// A packed lane of a streamed view pick could not be read.
        PackedPickLane { detail: String },
        /// A pick chunk of a streamed source could not be read.
        PickChunk { detail: String },
        /// A packed lane of a streamed selection could not be read.
        PackedSelectionLane { detail: String },
        /// A style index column is shorter than a streamed pass.
        StreamStyleIndexShort { column: String, len: u64, need: u64 },
        /// A box plot record id is not a resident record.
        BoxPlotRecordNotResident { id: String },
        /// A box plot ragged column id is not a resident ragged column.
        BoxPlotRaggedNotResident { id: String },
        /// A box plot id is neither a resident record nor a ragged column.
        BoxPlotNotResident { id: String },
        /// A streamed column reuses the id of a record, ragged or triangle
        /// column (`kind` names which).
        StreamedCompositeId { kind: String },
        /// A record, ragged or triangle column (`kind`) names a streamed id.
        CompositeNeedsResident { kind: String },
        /// A radar series' columns do not hold one value per category.
        RadarValueCount {
            categories: u64,
            index_column: String,
            index_len: u64,
            value_column: String,
            value_len: u64,
        },
        /// A category bar column does not hold one value per category.
        CategoryBarColumnLength {
            column: String,
            len: u64,
            categories: u64,
        },
        /// A PercentStacked bar group's value column has a negative value.
        PercentStackedNegative { column: String, series_id: String },
        /// A monotonic series names a channel its group's source does not have
        /// (#271).
        MonotonicChannel { channel: usize, channels: usize },
        /// A monotonic series' envelope column is already a pool column the
        /// series does not own: a host column, or another series' or chart's
        /// envelope (#271).
        MonotonicColumnTaken { column: String },
        /// A range bar column does not hold one value per position.
        RangeBarColumnLength {
            column: String,
            len: u64,
            positions: u64,
            position_column: String,
        },
        /// A point value map column (`what`) is shorter than the scatter.
        PointMapColumnShort {
            what: String,
            column: String,
            len: u64,
            need: u64,
        },
        /// A scatter style index column is shorter than the scatter.
        ScatterStyleIndexShort { column: String, len: u64, need: u64 },
        /// An errorbar style index column is shorter than the errorbars.
        ErrorBarStyleIndexShort { column: String, len: u64, need: u64 },
        /// A hexagonal grid's coordinates (`role`: x or y) are not uniform.
        HexagonalCoordinatesNotUniform { role: String, column: String },
        /// A label could not be laid out.
        InvalidLabel { detail: String },
        /// Value labels could not be laid out.
        ValueLabels { detail: String },
        /// A contour label atlas could not be built.
        ContourLabelAtlas { detail: String },
        /// A line fill names a series that is not in the chart.
        LineFillTargetMissing { target: String },
        /// A line fill joins a series with a different x column.
        LineFillXColumn {
            target: String,
            x_column: String,
            target_x_column: String,
        },
        /// More contour levels than the renderer draws.
        ContourLevelCount { count: u64, max: u64 },
        /// A rich text's math segments are malformed.
        InvalidMath { error: crate::text::MathError },
        /// A streamed column's span could not be read.
        StreamedColumnSpan { detail: String },
        /// An id is already a column, record, ragged or triangle column (`other`).
        IdInUse { other: String },
        /// A record, ragged or triangle column (`kind`) id is empty.
        CompositeIdEmpty { kind: String },
        /// A record, ragged or triangle column (`kind`) id holds U+001F.
        CompositeIdSeparator { kind: String },
        /// A record, ragged or triangle column (`kind`) reuses a column's id.
        CompositeIdIsColumn { kind: String },
        /// A record schema has more fields than a record holds.
        RecordSchemaFields { fields: u64, max: u64 },
        /// A record source has more fields than a record holds.
        RecordSourceFields { fields: u64, max: u64 },
        /// A record source names a field its schema does not have.
        RecordUnknownField { name: String },
        /// A record source gives a field twice.
        RecordFieldTwice { name: String },
        /// A record source lacks a required field.
        RecordFieldMissing { name: String },
        /// A record's fields differ in length.
        RecordFieldLength {
            name: String,
            len: u64,
            first: String,
            rows: u64,
        },
        /// A ragged column has fewer than two offsets.
        RaggedOffsetCount { offsets: u64 },
        /// A ragged column's first offset is not 0 (`first` as written).
        RaggedOffsetsStart { first: String },
        /// A ragged column's last offset is not its value count.
        RaggedLastOffset { last: String, total: u64 },
        /// A triangle index column's length is not a multiple of 3.
        TriangleIndexCount { count: u64 },
        /// A triangle index is not an integer in the index space.
        TriangleIndexRange { max: u64 },
        /// A record has more rows than the GPU index space.
        RecordRowsOverflow { max: u64 },
        /// A ragged column has more values than its u32 offsets index.
        RaggedValuesOverflow { max: u64, total: u64 },
        /// A ragged column has more groups than the GPU index space.
        RaggedGroupsOverflow { max: u64 },
        /// A triangle index column has more indices than the GPU index space.
        TriangleIndicesOverflow { max: u64, count: u64 },
        /// A record id is not a box plot record.
        NotBoxPlotRecord { id: String },
        /// A box plot record does not hold one row per category.
        BoxPlotRecordRows {
            id: String,
            rows: u64,
            categories: u64,
        },
        /// A box plot outlier column is not a resident ragged column.
        BoxPlotOutliersNotResident { id: String },
        /// A box plot outlier column does not hold one group per category.
        BoxPlotOutlierGroups {
            id: String,
            groups: u64,
            categories: u64,
        },
        /// A box plot position column does not hold one value per category.
        BoxPlotPositions {
            column: String,
            len: u64,
            categories: u64,
        },
        /// A violin curve is not a resident ragged column.
        ViolinCurveNotResident { id: String },
        /// A violin curve does not hold one group per category.
        ViolinCurveGroups {
            id: String,
            groups: u64,
            categories: u64,
        },
        /// A violin density column does not hold one value per curve value.
        ViolinDensityLength {
            column: String,
            len: u64,
            values: u64,
            curve: String,
        },
        /// A triangle mesh's index column is not a resident triangle column.
        TrianglesNotResident { id: String },
        /// A triangle index names a vertex past the shortest vertex column.
        TriangleIndexPastVertices {
            index: u64,
            vertices: u64,
            column: String,
        },
        /// A face value column is shorter than the triangles.
        FaceValueColumnShort {
            column: String,
            len: u64,
            triangles: u64,
        },
    }
}

impl RuleKind {
    /// The sentences of the rules that carry ids, counts or lengths.
    fn fmt_fielded(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PackedPickLane { detail } => write!(f, "invalid packed view pick lane: {detail}"),
            Self::PickChunk { detail } => write!(f, "invalid pick chunk: {detail}"),
            Self::PackedSelectionLane { detail } => {
                write!(f, "invalid packed selection lane: {detail}")
            }
            Self::StreamStyleIndexShort { column, len, need } => write!(
                f,
                "style index column {column:?} has {len} values, but this pass uses {need}"
            ),
            Self::BoxPlotRecordNotResident { id } => write!(
                f,
                "box plot record {id:?} is not resident: upload it with `upsert_record`"
            ),
            Self::BoxPlotRaggedNotResident { id } => write!(
                f,
                "box plot ragged column {id:?} is not resident: upload it with `upsert_ragged`"
            ),
            Self::BoxPlotNotResident { id } => write!(
                f,
                "box plot {id:?} is not a resident record or ragged column: upload the record with `upsert_record` and the outliers and violin curves with `upsert_ragged`"
            ),
            Self::StreamedCompositeId { kind } => write!(
                f,
                "the id is {kind}; streaming is not extended to records, ragged and triangle columns, which are resident only"
            ),
            Self::CompositeNeedsResident { kind } => write!(
                f,
                "the id is a streamed column; streaming is not extended to records, ragged and triangle columns, so {kind} needs a resident id"
            ),
            Self::RadarValueCount {
                categories,
                index_column,
                index_len,
                value_column,
                value_len,
            } => write!(
                f,
                "a radar series holds one value per category: the angle axis has {categories} categories, the category index column {index_column:?} {index_len} values and the value column {value_column:?} {value_len}"
            ),
            Self::CategoryBarColumnLength {
                column,
                len,
                categories,
            } => write!(
                f,
                "category bar column {column:?} has {len} values for {categories} categories; it needs one per category"
            ),
            Self::MonotonicChannel { channel, channels } => write!(
                f,
                "a monotonic series draws channel {channel} of a source with {channels} channels"
            ),
            Self::MonotonicColumnTaken { column } => write!(
                f,
                "monotonic envelope column {column:?} is already a column the series does not own"
            ),
            Self::PercentStackedNegative { column, series_id } => write!(
                f,
                "a PercentStacked bar group needs non-negative values: the value column {column:?} of series {series_id:?} has a negative value; draw it Stacked or Grouped instead"
            ),
            Self::RangeBarColumnLength {
                column,
                len,
                positions,
                position_column,
            } => write!(
                f,
                "range bar column {column:?} has {len} values for the {positions} positions of {position_column:?}; it needs one per row"
            ),
            Self::PointMapColumnShort {
                what,
                column,
                len,
                need,
            } => write!(
                f,
                "{what} column {column:?} has {len} values, but scatter uses {need}"
            ),
            Self::ScatterStyleIndexShort { column, len, need } => write!(
                f,
                "style index column {column:?} has {len} values, but scatter uses {need}"
            ),
            Self::ErrorBarStyleIndexShort { column, len, need } => write!(
                f,
                "errorbar style index column {column:?} has {len} values, but errorbar uses {need}"
            ),
            Self::HexagonalCoordinatesNotUniform { role, column } => write!(
                f,
                "a hexagonal grid's {role} coordinates ({column:?}) are uniform: a resident column of at least two values, every step equal to the first within 1e-6"
            ),
            Self::InvalidLabel { detail } => write!(f, "label: {detail}"),
            Self::ValueLabels { detail } => write!(f, "value labels: {detail}"),
            Self::ContourLabelAtlas { detail } => f.write_str(detail),
            Self::LineFillTargetMissing { target } => write!(
                f,
                "line fill references series `{target}`, which is not in this chart"
            ),
            Self::LineFillXColumn {
                target,
                x_column,
                target_x_column,
            } => write!(
                f,
                "line fill to series `{target}` requires the same x column: `{x_column}` vs `{target_x_column}`"
            ),
            Self::ContourLevelCount { count, max } => write!(
                f,
                "contour level count {count} exceeds the supported maximum {max}"
            ),
            Self::InvalidMath { error } => f.write_str(error.as_str()),
            Self::StreamedColumnSpan { detail } => {
                write!(f, "invalid streamed column span: {detail}")
            }
            Self::IdInUse { other } => write!(
                f,
                "the id is {other}; columns, records, ragged and triangle columns share one id space"
            ),
            Self::CompositeIdEmpty { kind } => write!(f, "a {kind} id must not be empty"),
            Self::CompositeIdSeparator { kind } => write!(
                f,
                "a {kind} id must not contain U+001F, which separates member column ids"
            ),
            Self::CompositeIdIsColumn { kind } => write!(
                f,
                "the id is a column; columns, records, ragged and triangle columns share one id space, so a {kind} needs an id of its own"
            ),
            Self::RecordSchemaFields { fields, max } => write!(
                f,
                "the record schema has {fields} fields; a record holds at most MAX_RECORD_FIELDS ({max})"
            ),
            Self::RecordSourceFields { fields, max } => write!(
                f,
                "the record source has {fields} fields; a record holds at most MAX_RECORD_FIELDS ({max})"
            ),
            Self::RecordUnknownField { name } => {
                write!(f, "'{name}' is not a field of the record schema")
            }
            Self::RecordFieldTwice { name } => {
                write!(f, "the record field '{name}' is given twice")
            }
            Self::RecordFieldMissing { name } => {
                write!(f, "the required record field '{name}' is missing")
            }
            Self::RecordFieldLength {
                name,
                len,
                first,
                rows,
            } => write!(
                f,
                "the record field '{name}' has {len} rows but '{first}' has {rows}; every field of a record has the same length"
            ),
            Self::RaggedOffsetCount { offsets } => write!(
                f,
                "a ragged column needs one offset per group plus one, at least two, got {offsets}"
            ),
            Self::RaggedOffsetsStart { first } => {
                write!(f, "ragged offsets must start at 0, got {first}")
            }
            Self::RaggedLastOffset { last, total } => write!(
                f,
                "the last ragged offset is {last} but there are {total} values"
            ),
            Self::TriangleIndexCount { count } => write!(
                f,
                "a triangle index column holds three vertex indices per triangle; {count} indices is not a multiple of 3"
            ),
            Self::TriangleIndexRange { max } => {
                write!(f, "triangle indices must be finite integers in 0..={max}")
            }
            Self::RecordRowsOverflow { max } => write!(f, "a record holds at most {max} rows"),
            Self::RaggedValuesOverflow { max, total } => write!(
                f,
                "a ragged column indexes its values with u32 offsets, at most {max} values, got {total}"
            ),
            Self::RaggedGroupsOverflow { max } => {
                write!(f, "a ragged column holds at most {max} groups")
            }
            Self::TriangleIndicesOverflow { max, count } => write!(
                f,
                "a triangle index column holds at most {max} indices, got {count}"
            ),
            Self::NotBoxPlotRecord { id } => write!(
                f,
                "record {id:?} is not a box plot record (`BOX_PLOT_RECORD`)"
            ),
            Self::BoxPlotRecordRows {
                id,
                rows,
                categories,
            } => write!(
                f,
                "box plot record {id:?} has {rows} rows for {categories} categories; it needs one per category"
            ),
            Self::BoxPlotOutliersNotResident { id } => write!(
                f,
                "box plot outlier column {id:?} is not resident: upload it with `upsert_ragged`"
            ),
            Self::BoxPlotOutlierGroups {
                id,
                groups,
                categories,
            } => write!(
                f,
                "box plot outlier column {id:?} has {groups} groups for {categories} categories; it needs one per category"
            ),
            Self::BoxPlotPositions {
                column,
                len,
                categories,
            } => write!(
                f,
                "box plot position column {column:?} has {len} values for {categories} categories; it needs one per category"
            ),
            Self::ViolinCurveNotResident { id } => write!(
                f,
                "violin curve {id:?} is not resident: upload it with `upsert_ragged`"
            ),
            Self::ViolinCurveGroups {
                id,
                groups,
                categories,
            } => write!(
                f,
                "violin curve {id:?} has {groups} groups for {categories} categories; it needs one per category"
            ),
            Self::ViolinDensityLength {
                column,
                len,
                values,
                curve,
            } => write!(
                f,
                "violin density {column:?} has {len} values for the {values} values of curve {curve:?}; it needs one per curve value"
            ),
            Self::TrianglesNotResident { id } => write!(
                f,
                "triangle mesh {id:?} is not a resident triangle index column: upload it with `upsert_triangles`"
            ),
            Self::TriangleIndexPastVertices {
                index,
                vertices,
                column,
            } => write!(
                f,
                "triangle index {index} names a vertex past the {vertices} vertices of column {column:?}"
            ),
            Self::FaceValueColumnShort {
                column,
                len,
                triangles,
            } => write!(
                f,
                "face value column {column:?} has {len} values for {triangles} triangles"
            ),
            // Every unit rule is written by `Display` itself.
            unit => f.write_str(unit.code()),
        }
    }
}

impl std::error::Error for RuleKind {}

#[cfg(test)]
mod tests {
    use super::RuleKind;

    /// Every unit rule's sentence, pinned: a rewording is a change a host
    /// that logs or shows it can see, so it shows up here (#215).
    #[test]
    fn every_unit_rule_reads_as_its_sentence() {
        let pinned: &[(RuleKind, &str)] = &[
            (
                RuleKind::View3dAzimuthNotFinite,
                "view_3d.azimuth_degrees must be finite",
            ),
            (
                RuleKind::View3dElevationOutOfRange,
                "view_3d.elevation_degrees must be in -90..=90",
            ),
            (
                RuleKind::View3dDistanceOutOfRange,
                "view_3d.distance must be in 1.2..=100",
            ),
            (
                RuleKind::View3dBoxAspectOutOfRange,
                "view_3d.box_aspect lengths must be in 0.05..=20",
            ),
            (
                RuleKind::View3dCategoricalAxis,
                "3D chart axes (bottom_x, left_y, right_y) cannot be categorical",
            ),
            (
                RuleKind::View3dAxisScale,
                "3D chart axes must be linear or logarithmic",
            ),
            (RuleKind::View3dOverlay, "3D charts have no overlay layers"),
            (
                RuleKind::View3dLineOffsets,
                "3D charts have no line offsets",
            ),
            (
                RuleKind::View3dDrawStyle,
                "3D charts are drawn in the Precise style only",
            ),
            (RuleKind::View3dStem, "stems draw flat: not on a 3D chart"),
            (
                RuleKind::View3dPointsWithoutZColumn,
                "a scatter or line series on a 3D chart names its height column (z_column)",
            ),
            (
                RuleKind::View3dStepLine,
                "a line on a 3D chart is linear: no step interpolation",
            ),
            (
                RuleKind::View3dLineFill,
                "a line on a 3D chart has no area fill",
            ),
            (
                RuleKind::View3dGridVector,
                "a grid vector draws flat: not on a 3D chart",
            ),
            (
                RuleKind::View3dVectorWithoutZColumn,
                "a vector series on a 3D chart names its anchor height column (z_column)",
            ),
            (
                RuleKind::View3dVectorWithoutW,
                "a vector series on a 3D chart names its z component (w)",
            ),
            (
                RuleKind::View3dImage,
                "an image draws flat: not on a 3D chart",
            ),
            (
                RuleKind::View3dParallel,
                "a parallel series draws flat: not on a 3D chart",
            ),
            (
                RuleKind::View3dSeriesKind,
                "a 3D chart draws only heatmaps, contours, heatmap contours, and the scatter, line and point vector series that name a z_column",
            ),
            (
                RuleKind::View3dHexagonalGrid,
                "a hexagonal grid draws flat: not on a 3D chart",
            ),
            (
                RuleKind::View3dBandedSurface,
                "a surface's fill is continuous: banded fills are not drawn in 3D",
            ),
            (
                RuleKind::View3dContourLabels,
                "contours on a 3D chart's floor have no labels",
            ),
            (
                RuleKind::SurfaceColorMatrixShape,
                "a surface's color_matrix has the column count, orientation and layout of its matrix",
            ),
            (
                RuleKind::SurfaceColorMatrixColumns,
                "a surface's color_matrix names its columns",
            ),
            (
                RuleKind::SurfaceWireframeStyle,
                "a surface wireframe has a width in 0..=24 and a colour in 0..=1",
            ),
            (
                RuleKind::SurfaceMaterialStyle,
                "a surface's gloss, texture_strength and emphasis_brightness are in 0..=1 and its texture_scale is positive",
            ),
            (
                RuleKind::RadialSliceCount,
                "a radial chart needs 1..64 slices",
            ),
            (
                RuleKind::RadialSliceValue,
                "slice values must be finite and nonnegative",
            ),
            (
                RuleKind::RadialSliceColor,
                "slice colors must be finite opaque RGBA in 0..1",
            ),
            (RuleKind::RadialSliceLabelColor, "invalid slice label color"),
            (
                RuleKind::RadialSliceExplode,
                "slice explode must be in 0..0.35",
            ),
            (
                RuleKind::RadialSliceLabel,
                "slice labels must be single-line, at most 80 characters",
            ),
            (
                RuleKind::RadialSliceLabelText,
                "slice label text must have at most two lines of at most 400 characters, without control characters",
            ),
            (
                RuleKind::RadialSliceTotal,
                "slice total must be positive and finite",
            ),
            (
                RuleKind::RadialStyle,
                "invalid radial material, light, or projection",
            ),
            (
                RuleKind::RadialTitle,
                "invalid radial title: size 6..160, at most 512 characters / 8 lines, finite styles",
            ),
            (
                RuleKind::RadialTitleOffsets,
                "a radial chart title keeps its position in decorations: its offsets and top_margin must be 0",
            ),
            (RuleKind::RadialShape, "invalid radial shape or start angle"),
            (
                RuleKind::RadialTextOrBackground,
                "invalid radial text or background",
            ),
            (
                RuleKind::RadialLabelFormatOrInteraction,
                "invalid radial label format or interaction",
            ),
            (
                RuleKind::RadialInteractionTarget,
                "radial interaction target is out of range",
            ),
            (
                RuleKind::RadialEmphasizedZeroSlice,
                "cannot emphasize a zero-valued slice",
            ),
            (RuleKind::SunburstDetail, "a sunburst has no detail circle"),
            (
                RuleKind::RadialSplitIndex,
                "split slice index is out of range",
            ),
            (
                RuleKind::RadialDetailSum,
                "detail values must sum to the parent slice value",
            ),
            (RuleKind::RadialDetailShape, "invalid detail chart shape"),
            (
                RuleKind::SunburstRingCount,
                "a sunburst has at most 8 rings",
            ),
            (
                RuleKind::SunburstNodeCount,
                "a sunburst ring has at most 64 nodes",
            ),
            (
                RuleKind::SunburstTilt,
                "a sunburst draws flat: no tilt or depth",
            ),
            (
                RuleKind::SunburstChildrenSum,
                "a node's children sum to at most its value",
            ),
            (
                RuleKind::EnergyLevelsSourceWithRecords,
                "an energy-level series with a source holds no records of its own",
            ),
            (
                RuleKind::EnergyLevelIds,
                "energy level ids must be non-empty and unique",
            ),
            (
                RuleKind::IonizationLimitIds,
                "ionization limit ids must be non-empty and unique",
            ),
            (
                RuleKind::TransitionIds,
                "transition ids must be non-empty and unique",
            ),
            (
                RuleKind::LevelBandIds,
                "level band ids must be non-empty and unique",
            ),
            (
                RuleKind::LevelBandEnds,
                "a level band's first and last name levels of its column",
            ),
            (
                RuleKind::LevelBandLabelOffset,
                "a level band's label offset is out of range",
            ),
            (
                RuleKind::FineStructureCentroid,
                "a fine-structure component must name a centroid level",
            ),
            (
                RuleKind::TransitionSameLevel,
                "a transition joins two different levels",
            ),
            (
                RuleKind::TransitionLevels,
                "a transition must name two levels of its series",
            ),
            (
                RuleKind::EnergyLevelEnergy,
                "an energy level's energy must be finite",
            ),
            (
                RuleKind::EnergyLevelUncertainty,
                "an energy level's uncertainty and g must be finite and not negative",
            ),
            (
                RuleKind::EnergyLevelText,
                "an energy level's column must be set and its text fields one line of at most 160 characters",
            ),
            (
                RuleKind::ComponentMultipletId,
                "a component's multiplet id is one line of at most 160 characters",
            ),
            (RuleKind::EnergyLevelLabel, "invalid energy level label"),
            (
                RuleKind::IonizationLimitEnergy,
                "an ionization limit's energy must be finite and its uncertainty not negative",
            ),
            (
                RuleKind::IonizationLimitLabel,
                "invalid ionization limit label or reference",
            ),
            (
                RuleKind::TransitionValues,
                "a transition's values must be finite and its uncertainties not negative",
            ),
            (
                RuleKind::TransitionXOffset,
                "a transition's x_offset must be in -0.5..=0.5 of the slot",
            ),
            (
                RuleKind::TransitionText,
                "a transition's text fields are one line of at most 160 characters",
            ),
            (RuleKind::TransitionLabel, "invalid transition label"),
            (
                RuleKind::EnergyLevelStroke,
                "invalid energy level stroke style",
            ),
            (
                RuleKind::IonizationLimitLine,
                "invalid ionization limit line style",
            ),
            (RuleKind::TransitionArrow, "invalid transition arrow style"),
            (
                RuleKind::EnergyLevelsPickedWidth,
                "picked_width_scale must be in 1..=4",
            ),
            (RuleKind::LevelBandStyle, "invalid level band style"),
            (
                RuleKind::WavelengthLabels,
                "invalid wavelength labels: a text of size 6..160 and at most 17 decimals",
            ),
            (
                RuleKind::WavelengthUnit,
                "a wavelength label unit is one line of at most 160 characters",
            ),
            (
                RuleKind::EnergyLevelsColumns,
                "an energy-level series reads no columns: x_column and y_column stay empty",
            ),
            (
                RuleKind::EnergyLevelsSelfSource,
                "an energy-level series cannot be its own source",
            ),
            (
                RuleKind::EnergyLevelsChart,
                "an energy-level series is drawn on a rectangular chart",
            ),
            (
                RuleKind::EnergyLevelsCategoricalX,
                "an energy-level series needs a categorical bottom_x: its term columns",
            ),
            (
                RuleKind::BoxStatsOrder,
                "box statistics must be finite and low <= Q1 <= median <= Q3 <= high",
            ),
            (
                RuleKind::BoxStatsMeanOrCount,
                "mean must be finite; sample count must be positive when supplied",
            ),
            (
                RuleKind::BoxStatsMedianCi,
                "median CI must be finite and contain the median",
            ),
            (
                RuleKind::BoxStatsOutliers,
                "outliers must be finite and strictly outside the whiskers",
            ),
            (
                RuleKind::BoxPlotChartLimits,
                "boxplot limits: at least 1 category and 1 series",
            ),
            (RuleKind::BoxPlotChartCategory, "invalid category ID/label"),
            (
                RuleKind::BoxPlotChartSeries,
                "invalid series ID/label, color or value count",
            ),
            (
                RuleKind::BoxPlotChartStyle,
                "invalid boxplot layout, font or colors",
            ),
            (RuleKind::BoxPlotChartTitle, "invalid title"),
            (
                RuleKind::BoxPlotChartValueRange,
                "invalid linear value range",
            ),
            (
                RuleKind::BoxPlotChartOverride,
                "invalid or duplicate box override",
            ),
            (
                RuleKind::BoxPlotChartUnresolvedStyle,
                "internal: a box with a value resolved no style",
            ),
            (
                RuleKind::BoxPlotChartNotchWithoutCi,
                "notched boxes require an explicit median CI",
            ),
            (
                RuleKind::BoxPlotChartInteractionTarget,
                "unknown or missing interaction target",
            ),
            (RuleKind::PolarAngles, "polar angles must be finite"),
            (
                RuleKind::PolarInnerRadius,
                "polar inner_radius_fraction must be within 0..=0.9",
            ),
            (
                RuleKind::PolarSector,
                "polar sector must be a finite [start, end] with start < end",
            ),
            (
                RuleKind::PolarSectorTurn,
                "polar sector cannot exceed one turn",
            ),
            (
                RuleKind::PolarArcPieces,
                "polar arc pieces must be within 1..=64",
            ),
            (
                RuleKind::PolarAngleAxisScale,
                "the polar angle axis (bottom_x) must be linear",
            ),
            (
                RuleKind::PolarAngleAxisInverted,
                "the polar angle axis (bottom_x) cannot be inverted; use clockwise",
            ),
            (
                RuleKind::PolarPolygonGridAxis,
                "a polygon polar grid joins its rings at the categories: it needs a categorical angle axis (bottom_x.categories)",
            ),
            (
                RuleKind::PolarRadiusCategorical,
                "the polar radius axis (left_y) cannot be categorical",
            ),
            (
                RuleKind::PolarRadiusScale,
                "the polar radius axis (left_y) is linear or logarithmic",
            ),
            (
                RuleKind::PolarOverlay,
                "polar charts have no overlay layers",
            ),
            (
                RuleKind::PolarLineOffsets,
                "polar charts have no line offsets",
            ),
            (
                RuleKind::DataAreaTooSmall,
                "data area must fit the measured plot region and be at least 80 by 60 logical pixels",
            ),
            (
                RuleKind::DecorationOffsetCount,
                "too many decoration offsets: a chart holds at most 1024; drop the offsets of elements the chart no longer has",
            ),
            (
                RuleKind::DecorationOffsets,
                "decoration offsets must be finite and unique",
            ),
            (
                RuleKind::DecorationAxisMove,
                "axes only move perpendicular to their direction",
            ),
            (
                RuleKind::RadarChartLimits,
                "radar limits: at least 3 categories and 1 series",
            ),
            (RuleKind::RadarChartSeriesStyle, "invalid radar style"),
            (
                RuleKind::RadarChartStyle,
                "invalid radar layout, font or colors",
            ),
            (RuleKind::RadarChartValueRange, "invalid value range"),
            (
                RuleKind::RadarChartRingSpacing,
                "ring spacing must be finite and positive",
            ),
            (
                RuleKind::RadarChartInteractionTarget,
                "unknown interaction target",
            ),
            (
                RuleKind::AxisCrossAt,
                "an axis crossing (`cross_at`) must be finite",
            ),
            (
                RuleKind::CategoryWindow,
                "a category window needs finite bounds with from < to",
            ),
            (RuleKind::AxisBoundsNotFinite, "axis bounds must be finite"),
            (
                RuleKind::AxisBoundsOrder,
                "axis max must be greater than min",
            ),
            (
                RuleKind::LogAxisBounds,
                "logarithmic axis bounds must be positive",
            ),
            (
                RuleKind::ProbabilityAxisBounds,
                "probability axis bounds must lie strictly between 0 and 1",
            ),
            (
                RuleKind::AxisUnitScale,
                "an axis unit's scale must be finite and non-zero",
            ),
            (
                RuleKind::AxisUnitOffset,
                "an axis unit's offset must be finite",
            ),
            (
                RuleKind::AxisUnitLog,
                "a logarithmic axis takes a unit with offset 0 and a positive scale",
            ),
            (
                RuleKind::AxisUnitProbability,
                "a probability axis cannot have a unit",
            ),
            (
                RuleKind::AxisUnitTimestamp,
                "an axis with timestamp labels cannot have a unit",
            ),
            (
                RuleKind::AxisUnitCategorical,
                "a categorical axis cannot have a unit",
            ),
            (RuleKind::SeriesIdEmpty, "series_id must not be empty"),
            (
                RuleKind::SeriesIdDuplicate,
                "series_id must be unique within a chart",
            ),
            (
                RuleKind::PolarErrorBars,
                "error bars are not drawn on polar axes",
            ),
            (RuleKind::PolarStems, "stems are not drawn on polar axes"),
            (
                RuleKind::PolarHistogram,
                "histogram series are not drawn on polar axes",
            ),
            (
                RuleKind::PolarHexagonalGrid,
                "a hexagonal grid is drawn on a rectangular chart",
            ),
            (
                RuleKind::PolarHeatmapPolygonGrid,
                "a polar heatmap's cells follow circular rings: not on a polygon grid",
            ),
            (RuleKind::PolarImage, "images are not drawn on polar axes"),
            (
                RuleKind::PolarVector,
                "vector series are not drawn on polar axes",
            ),
            (
                RuleKind::PolarBoxPlot,
                "box plots are not drawn on polar axes",
            ),
            (
                RuleKind::PolarRangeBars,
                "range bars are not drawn on polar axes",
            ),
            (
                RuleKind::PolarStreamlines,
                "streamlines are not drawn on polar axes",
            ),
            (
                RuleKind::PolarParallel,
                "parallel series are drawn on parallel charts, not polar axes",
            ),
            (
                RuleKind::PolarEnergyLevels,
                "energy-level series are not drawn on polar axes",
            ),
            (
                RuleKind::PolarTriMesh,
                "a triangle mesh is not drawn on polar axes",
            ),
            (
                RuleKind::PolarFeynman,
                "Feynman series are not drawn on polar axes",
            ),
            (
                RuleKind::PolarTernary,
                "ternary series are drawn on ternary charts, not polar axes",
            ),
            (
                RuleKind::PolarArcStepLine,
                "a polar arc interpolation needs a linearly interpolated line; a step interpolation draws chords",
            ),
            (
                RuleKind::CategoricalChartBarMaterial,
                "material not supported by bars",
            ),
            (
                RuleKind::CategoricalChartLimits,
                "categorical limits: at least 1 category and 1 series",
            ),
            (
                RuleKind::CategoricalChartCategory,
                "invalid or duplicate category ID/label",
            ),
            (
                RuleKind::CategoricalChartSeries,
                "invalid series, value count, color or style",
            ),
            (
                RuleKind::CategoricalChartValues,
                "values must be finite; percent stacks require nonnegative values",
            ),
            (
                RuleKind::CategoricalChartStyle,
                "invalid categorical style or layout",
            ),
            (
                RuleKind::CategoricalChartTitles,
                "titles must be single-line, at most 160 characters",
            ),
            (
                RuleKind::CategoricalChartValueSuffix,
                "invalid value suffix",
            ),
            (
                RuleKind::CategoricalChartOverride,
                "invalid or duplicate bar override",
            ),
            (
                RuleKind::CategoricalChartInteractionTarget,
                "interaction target is missing or unknown",
            ),
            (
                RuleKind::CategoricalChartStackOverflow,
                "stack total overflow",
            ),
            (
                RuleKind::BoundedTitle,
                "invalid bounded title: size 6..160, at most 512 characters / 8 lines, finite styles",
            ),
            (
                RuleKind::TriMeshTriangles,
                "a triangle mesh names its triangle index column",
            ),
            (
                RuleKind::TriMeshValues,
                "a triangle mesh names its value column",
            ),
            (
                RuleKind::TriMeshNothingDrawn,
                "a triangle mesh draws a fill, contour lines or both",
            ),
            (
                RuleKind::TriMeshSurfaceWithoutFill,
                "a surface look needs the triangle mesh's fill",
            ),
            (
                RuleKind::TriMeshVertexValuesFlat,
                "vertex values are shaded Interpolated: a flat triangle colour from vertex values is host work (give face values)",
            ),
            (
                RuleKind::TriMeshFaceValuesInterpolated,
                "face values are shaded Flat: interpolating them needs vertex values, which are host work",
            ),
            (
                RuleKind::TriMeshContourFaceValues,
                "contour lines need vertex values: a field constant on each triangle has no isolines",
            ),
            (
                RuleKind::TriMeshContourLabels,
                "contour lines on a triangle mesh have no labels",
            ),
            (
                RuleKind::TriMeshSurfaceColorMatrix,
                "a triangle mesh is coloured by its values: no surface color_matrix",
            ),
            (
                RuleKind::TriMeshVertexColumns,
                "a triangle mesh names its vertex coordinate columns (x_column, y_column)",
            ),
            (
                RuleKind::TriMeshTernary,
                "a triangle mesh is not drawn on a ternary chart",
            ),
            (
                RuleKind::TriMeshParallel,
                "a triangle mesh is not drawn on a parallel chart",
            ),
            (
                RuleKind::TriMeshWithoutZColumn,
                "a triangle mesh on a 3D chart names its vertex height column (z_column)",
            ),
            (
                RuleKind::SubplotNested,
                "a subplot cannot have subplots of its own",
            ),
            (
                RuleKind::SubplotAnnotations,
                "the document's annotation layer belongs to the parent, not a subplot",
            ),
            (
                RuleKind::SubplotLink,
                "a subplot link must point at the parent or an earlier subplot",
            ),
            (
                RuleKind::SeriesInManyLayers,
                "a series id may belong to at most one subplot or overlay layer",
            ),
            (
                RuleKind::SeriesInManyOverlays,
                "a series id may belong to at most one overlay layer",
            ),
            (
                RuleKind::LayerSeriesUnknown,
                "a layer names a series the chart has not registered",
            ),
            (
                RuleKind::PointSizeFitNoMap,
                "the series has no point size map to fit",
            ),
            (
                RuleKind::PointSizeFitExtent,
                "the size column has no finite, non-empty extent to fit the size domain to",
            ),
            (
                RuleKind::VectorFitNotVector,
                "the series is not a vector field",
            ),
            (
                RuleKind::VectorFitTarget,
                "a vector fit target must be finite and positive",
            ),
            (
                RuleKind::VectorFitExtent,
                "the vector components have no finite, non-zero extent to fit the scale to",
            ),
            (
                RuleKind::CategoryGroupCategories,
                "a category group names categories of its axis",
            ),
            (
                RuleKind::CategoryGroupOrder,
                "a category group's first category comes before its last",
            ),
            (
                RuleKind::CategoryGroupOverlap,
                "category groups run in axis order without overlapping",
            ),
            (RuleKind::CategoryGroupLabel, "invalid category group label"),
            (
                RuleKind::CategoryGroupOffset,
                "invalid category group offset",
            ),
            (
                RuleKind::BreakCategorical,
                "a categorical axis cannot be broken",
            ),
            (
                RuleKind::BreakCount,
                "an axis can cut out at most three ranges",
            ),
            (
                RuleKind::BreakRange,
                "a cut range needs finite bounds with from < to",
            ),
            (
                RuleKind::BreakLogRange,
                "a cut range on a logarithmic axis must be positive",
            ),
            (
                RuleKind::BreakProbabilityRange,
                "a cut range on a probability axis must lie strictly between 0 and 1",
            ),
            (
                RuleKind::BreakOrder,
                "cut ranges must be sorted and must not touch",
            ),
            (
                RuleKind::BreakGap,
                "a break gap must be between 0 and 200 px",
            ),
            (
                RuleKind::BreakSizing,
                "only a Continuous broken axis sizes its pieces other than by value",
            ),
            (
                RuleKind::BreakWeightCount,
                "piece weights need one weight per piece: one more than cut ranges",
            ),
            (
                RuleKind::BreakWeights,
                "piece weights must be finite and positive",
            ),
            (
                RuleKind::BreakMark,
                "a break mark needs a size in 0..=64, a width in 0..=16 and a colour in range",
            ),
            (
                RuleKind::CategoriesEmpty,
                "a categorical axis needs at least one category",
            ),
            (
                RuleKind::CategoryIds,
                "category ids must be unique, non-empty, single-line and at most 80 characters",
            ),
            (
                RuleKind::CategoryLabels,
                "category labels must be single-line and at most 80 characters",
            ),
            (RuleKind::CategoryRichLabel, "invalid category rich label"),
            (
                RuleKind::CategoricalLogAxis,
                "a categorical axis cannot be logarithmic",
            ),
            (
                RuleKind::CategoricalProbabilityAxis,
                "a categorical axis cannot be a probability axis",
            ),
            (RuleKind::StemBaseline, "a stem baseline must be finite"),
            (
                RuleKind::StemWidth,
                "a stem width must be finite and at least 0",
            ),
            (
                RuleKind::XyNothingDrawn,
                "an xy series draws markers, a line or both",
            ),
            (
                RuleKind::ErrorBarsWithoutMarkers,
                "error bars are drawn with markers",
            ),
            (
                RuleKind::ErrorBarsDirection,
                "error bars run along x, y or both",
            ),
            (
                RuleKind::StemsWithoutMarkers,
                "stems are drawn with markers",
            ),
            (
                RuleKind::FieldNothingDrawn,
                "a field draws a fill, contour lines or both",
            ),
            (
                RuleKind::FieldSurfaceWithoutFill,
                "a surface look needs the field's fill",
            ),
            (
                RuleKind::XyWireNothingDrawn,
                "an xy series draws markers or a line",
            ),
            (
                RuleKind::StreamlineArrows,
                "streamline arrows need a finite spacing_px > 0, finite non-negative head_length_px and head_width_px, and spacing_px >= head_length_px",
            ),
            (
                RuleKind::StreamlineInterpolation,
                "streamlines are polylines: the line interpolation must be Linear",
            ),
            (RuleKind::StreamlineFill, "streamlines have no fill"),
            (
                RuleKind::StreamlineColumns,
                "a streamline series names its x_column and y_column",
            ),
            (
                RuleKind::StreamlineColorMapColumn,
                "a streamline colour map names its column",
            ),
            (
                RuleKind::MonotonicGroupEmpty,
                "a monotonic series names its group",
            ),
            (
                RuleKind::MonotonicInterpolation,
                "a monotonic series is a polyline: the line interpolation must be Linear",
            ),
            (RuleKind::MonotonicFill, "a monotonic series has no fill"),
            (
                RuleKind::MonotonicColumns,
                "a monotonic series names two distinct envelope columns in x_column and y_column",
            ),
            (
                RuleKind::MonotonicColumnsShared,
                "a monotonic series' envelope columns are its own: no other series of the chart reads them",
            ),
            (
                RuleKind::MonotonicChartKind,
                "monotonic series are drawn on rectangular 2D charts, not ternary, parallel or 3D ones",
            ),
            (
                RuleKind::MonotonicXAxis,
                "a monotonic series needs a linear, numeric, unbroken x axis",
            ),
            (
                RuleKind::PolarMonotonic,
                "monotonic series are not drawn on polar axes",
            ),
            (
                RuleKind::StreamedMonotonic,
                "monotonic series are fed by their own calls; streamed displays do not draw them",
            ),
            (
                RuleKind::MonotonicGeneralChart,
                "a monotonic line series lives only in a chart registered as a monotonic chart",
            ),
            (
                RuleKind::MonotonicMappingEdit,
                "a monotonic chart's x range, x scale, breaks, direction and plot area change only through its monotonic calls",
            ),
            (
                RuleKind::MonotonicRange,
                "a monotonic series' x range must be finite and increasing",
            ),
            (
                RuleKind::VectorPixels,
                "vector width_px, head_length_px, head_width_px and min_length_px must be finite and non-negative",
            ),
            (
                RuleKind::VectorScale,
                "vector scale must be finite and non-zero",
            ),
            (
                RuleKind::PointSizeMapDomain,
                "point size map domain must be finite with domain_min < domain_max",
            ),
            (
                RuleKind::PointSizeMapPixels,
                "point size map min_px and max_px must be finite, non-negative and min_px <= max_px",
            ),
            (
                RuleKind::ContourLabelSpacing,
                "contour label spacing_px must be finite and greater than zero",
            ),
            (
                RuleKind::ContourLabelAnchors,
                "contour label anchors exceed the supported maximum of 1024",
            ),
            (
                RuleKind::ValueLabelOffsetCount,
                "value label offsets need one entry per category",
            ),
            (
                RuleKind::ValueLabelOffset,
                "a value label offset is finite and at most 4096 px along either axis",
            ),
            (RuleKind::BarStyle, "invalid bar style"),
            (
                RuleKind::BarGroups,
                "bar groups need a group width in 0.1..=0.95 and a bar gap in 0..=24 px",
            ),
            (
                RuleKind::LineOffsetSteps,
                "line offset steps must be finite",
            ),
            (
                RuleKind::LineOffsetMember,
                "a line offset member is a non-empty series id, listed once",
            ),
            (
                RuleKind::LineOffsetMemberLayer,
                "a line offset member must be drawn by the chart that lists it",
            ),
            (
                RuleKind::LineOffsetMemberUnknown,
                "a line offset member names no series of the chart",
            ),
            (
                RuleKind::LineOffsetMemberNotLine,
                "a line offset member must draw a line: Line, ScatterLine or a LineScatterErrorbar kind",
            ),
            (
                RuleKind::LineOffsetFill,
                "a ToSeries fill cannot join series with different line offsets",
            ),
            (
                RuleKind::BarColor,
                "a bar colour must be opaque with channels in 0..=1",
            ),
            (
                RuleKind::BarLabelFormat,
                "bar labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
            ),
            (
                RuleKind::BarLabelTextCount,
                "bar label text needs one string per category",
            ),
            (
                RuleKind::BarLabel,
                "a bar label is one line of at most 160 characters",
            ),
            (
                RuleKind::BarOverride,
                "a bar override names a category of the axis and its own series, once",
            ),
            (
                RuleKind::HorizontalBarsOnOverlay,
                "horizontal category bars cannot sit on an overlay layer: its own axis is a value axis on y",
            ),
            (
                RuleKind::RoseDirection,
                "a rose fans its bars around the centre: the group direction on a polar chart is vertical",
            ),
            (
                RuleKind::CategoryBarsAxis,
                "category bars need a categorical axis: bottom_x for vertical bars, left_y for horizontal ones",
            ),
            (
                RuleKind::CategoryBarsValueAxis,
                "category bars need a linear numeric value axis",
            ),
            (
                RuleKind::CategoryBarsValueColumns,
                "the category bar series of a group need distinct value columns",
            ),
            (RuleKind::BoxMaterial, "material not supported by boxes"),
            (RuleKind::BoxStyle, "invalid boxplot style"),
            (
                RuleKind::BoxColor,
                "a box colour must be opaque with channels in 0..=1",
            ),
            (
                RuleKind::BoxLabelFormat,
                "box labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
            ),
            (
                RuleKind::BoxLabelTextCount,
                "box label text needs one string per category",
            ),
            (
                RuleKind::BoxLabel,
                "a box label is one line of at most 160 characters",
            ),
            (
                RuleKind::BoxPlotColumns,
                "a box plot series names its category positions (x_column), its record (y_column) unless it draws a violin, and, when set, its outlier column",
            ),
            (
                RuleKind::ViolinBoxWidth,
                "a violin's box width is a share of the slot in 0..=1",
            ),
            (
                RuleKind::ViolinHalves,
                "a split violin draws at least one half",
            ),
            (
                RuleKind::ViolinHalfColumns,
                "a violin half names a ragged curve and a distinct density column",
            ),
            (
                RuleKind::ViolinDensityScale,
                "a violin half's density_scale is finite and positive",
            ),
            (
                RuleKind::ViolinOutline,
                "a violin outline is 0..=8 px wide with channels in 0..=1, its fill opaque",
            ),
            (
                RuleKind::BoxPointsColumns,
                "box points name distinct position and value columns",
            ),
            (
                RuleKind::BoxPointsStyle,
                "box points are 1..=24 px, jittered by 0..=1 and offset by -1..=1 of the half slot, with colour channels in 0..=1",
            ),
            (
                RuleKind::BoxOverride,
                "a box override names a category of the axis and its own series, once",
            ),
            (
                RuleKind::BarsAndBoxesInLayer,
                "a layer draws either category bars or box plots, not both",
            ),
            (
                RuleKind::BoxPlotsStacked,
                "box plots stand side by side: their bar_groups mode must be Grouped, not Stacked or PercentStacked",
            ),
            (
                RuleKind::HorizontalBoxPlotsOnOverlay,
                "horizontal box plots cannot sit on an overlay layer: its own axis is a value axis on y",
            ),
            (
                RuleKind::BoxPlotsAxis,
                "box plots need a categorical axis: bottom_x for vertical boxes, left_y for horizontal ones",
            ),
            (
                RuleKind::BoxPlotsValueAxis,
                "box plots need a linear numeric value axis",
            ),
            (
                RuleKind::RangeBarColor,
                "a range bar colour must be opaque with channels in 0..=1",
            ),
            (
                RuleKind::RangeBarColumns,
                "a range bar series names its positions (x_column), its end values (y_column), its start values and every optional column",
            ),
            (
                RuleKind::RangeBarColumnsDistinct,
                "the columns of a range bar series are distinct",
            ),
            (
                RuleKind::RangeBarWhiskers,
                "range bar whiskers are 0..=24 px wide, in an opaque colour",
            ),
            (
                RuleKind::RangeBarConnector,
                "a range bar connector is 0..=24 px wide, its colour channels in 0..=1",
            ),
            (
                RuleKind::RangeBarStateColors,
                "range bar direction, state and row colours must be opaque",
            ),
            (
                RuleKind::RangeBarRemainingOpacity,
                "a range bar's remaining opacity is in 0..=1",
            ),
            (
                RuleKind::RangeBarSlotWidth,
                "a range bar slot width is finite and greater than 0",
            ),
            (
                RuleKind::RangeBarLabelFormat,
                "range bar labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
            ),
            (
                RuleKind::RangeBarLabel,
                "a range bar label is one line of at most 160 characters",
            ),
            (
                RuleKind::RangeBarContrastInk,
                "a range bar's contrast ink colours must be opaque",
            ),
            (
                RuleKind::ContrastInkSeries,
                "only range bar labels take contrast ink",
            ),
            (
                RuleKind::CategoryBandsFill,
                "a category band fill has every channel in 0..=1",
            ),
            (
                RuleKind::TimestampStepRequired,
                "relative, custom and period timestamp labels need a calendar step",
            ),
            (
                RuleKind::TimestampRelativePrefix,
                "a relative timestamp label prefix is one line of at most 24 characters",
            ),
            (
                RuleKind::TimestampCustomLabel,
                "a custom timestamp label is one line of at most 160 characters",
            ),
            (
                RuleKind::RangeBarsWithOtherBars,
                "a layer draws either category bars, box plots or range bars",
            ),
            (
                RuleKind::RangeBarsStacked,
                "range bars stand side by side: their bar_groups mode must be Grouped, not Stacked or PercentStacked",
            ),
            (
                RuleKind::HorizontalRangeBarsOnOverlay,
                "horizontal range bars cannot sit on an overlay layer: its own axis is a value axis on y",
            ),
            (
                RuleKind::RangeBarsValueAxis,
                "range bars need a numeric value axis",
            ),
            (
                RuleKind::RangeBarsPositionAxis,
                "range bars need a categorical or linear numeric position axis: bottom_x for vertical bars, left_y for horizontal ones",
            ),
            (
                RuleKind::RangeBarConnectorLayer,
                "a range bar connector needs its series alone in its layer",
            ),
            (
                RuleKind::RangeBarSlotWidthAxis,
                "a range bar series on a categorical position axis has no slot_width; on a numeric one it needs one",
            ),
            (RuleKind::RadarSeriesStyle, "invalid radar series style"),
            (
                RuleKind::RadarColor,
                "a radar colour needs channels in 0..=1",
            ),
            (
                RuleKind::RadarLabelFormat,
                "radar labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
            ),
            (
                RuleKind::RadarLabelTextCount,
                "radar label text needs one string per category",
            ),
            (
                RuleKind::RadarLabel,
                "a radar label is one line of at most 160 characters",
            ),
            (
                RuleKind::RadarColumns,
                "a radar series names its category index column (x_column) and its value column (y_column)",
            ),
            (
                RuleKind::HexagonalGridVector,
                "a grid vector reads a rectangular lattice: not a hexagonal grid",
            ),
            (
                RuleKind::HexagonalContours,
                "contours read a rectangular lattice: a hexagonal grid draws as a Heatmap only",
            ),
            (
                RuleKind::HexagonalShading,
                "a hexagonal grid is shaded flat",
            ),
            (
                RuleKind::HexagonalAxes,
                "a hexagonal grid needs linear x and y axes: its lattice is uniform",
            ),
            (
                RuleKind::HexagonalBrokenAxis,
                "a hexagonal grid's lattice is uniform: not on a Continuous broken axis",
            ),
            (
                RuleKind::ImageChannelColumns,
                "an image's channels share one lattice: the same number of columns",
            ),
            (
                RuleKind::ImageChannelLattice,
                "an image's channels share one lattice: the same orientation and grid layout",
            ),
            (
                RuleKind::ImageRange,
                "an image's range is finite with a smaller low end",
            ),
            (RuleKind::ImageOpacity, "an image's opacity is 0..=1"),
            (
                RuleKind::RadarNotPolar,
                "radar series are drawn on a polar chart (Config.polar)",
            ),
            (
                RuleKind::RadarAngleAxis,
                "radar series need a categorical angle axis: bottom_x.categories on the polar chart",
            ),
            (
                RuleKind::RadarInterpolation,
                "radar polygons have straight edges: the polar interpolation must be Chord",
            ),
            (
                RuleKind::SelectedMatrixGone,
                "selected matrix no longer exists",
            ),
            (
                RuleKind::ParallelAxisRange,
                "a parallel axis's range is finite with a smaller min",
            ),
            (
                RuleKind::ParallelLogAxisRange,
                "a logarithmic parallel axis's range is positive",
            ),
            (
                RuleKind::ParallelAxisScale,
                "a parallel axis is linear or logarithmic: not a probability axis",
            ),
            (
                RuleKind::ParallelAxisLabelOffset,
                "a parallel axis's label offset is finite",
            ),
            (
                RuleKind::ParallelAxisCount,
                "a parallel chart has at least two axes",
            ),
            (
                RuleKind::ParallelBottomXCategorical,
                "a parallel chart's bottom_x is categorical: one category per axis",
            ),
            (
                RuleKind::ParallelBottomXCategories,
                "a parallel chart's bottom_x has one category per axis",
            ),
            (
                RuleKind::ParallelLeftY,
                "a parallel chart's left_y spans the normalized band: linear 0..1, not inverted",
            ),
            (
                RuleKind::ParallelOverlay,
                "parallel charts have no overlay layers",
            ),
            (
                RuleKind::ParallelLineOffsets,
                "parallel charts have no line offsets",
            ),
            (
                RuleKind::ParallelDrawStyle,
                "parallel charts are drawn in the Precise style only",
            ),
            (
                RuleKind::ParallelSubplotLinks,
                "parallel charts take no subplot axis links",
            ),
            (
                RuleKind::ParallelSeriesNotParallel,
                "a parallel series is drawn on a parallel chart (Config.parallel)",
            ),
            (
                RuleKind::ParallelSeriesColumnCount,
                "a parallel series names one column per axis",
            ),
            (
                RuleKind::ParallelSeriesTooFewColumns,
                "a parallel series names one column per axis, at least two",
            ),
            (
                RuleKind::ParallelSeriesColumnNames,
                "a parallel series' axis columns are named",
            ),
            (
                RuleKind::ParallelSeriesXY,
                "a parallel series' x_column and y_column are its first two axis columns",
            ),
            (
                RuleKind::ParallelSeriesStepLine,
                "a parallel series' line is linear: no step interpolation",
            ),
            (
                RuleKind::ParallelSeriesLineFill,
                "a parallel series' line has no area fill",
            ),
            (
                RuleKind::ParallelSeriesDashed,
                "a parallel series' line is solid",
            ),
            (
                RuleKind::ParallelSeriesColorColumn,
                "a parallel series' colour column is named",
            ),
            (
                RuleKind::ParallelSeriesColorbar,
                "a parallel series coloured by a column needs Config.colorbar",
            ),
            (
                RuleKind::ParallelChartSeriesKind,
                "a parallel chart draws only parallel series",
            ),
            (
                RuleKind::TernaryAxisScale,
                "ternary axes (bottom_x, left_y, right_y) must be linear",
            ),
            (
                RuleKind::TernaryAxisInverted,
                "ternary axes cannot be inverted: each increases clockwise",
            ),
            (
                RuleKind::TernaryAxisCategorical,
                "ternary axes cannot be categorical",
            ),
            (
                RuleKind::TernaryAxisSpan,
                "ternary axes span the whole triangle: every axis has min 0 and the same positive max (1 or 100)",
            ),
            (
                RuleKind::TernaryOverlay,
                "ternary charts have no overlay layers",
            ),
            (
                RuleKind::TernaryLineOffsets,
                "ternary charts have no line offsets",
            ),
            (
                RuleKind::TernaryDrawStyle,
                "ternary charts are drawn in the Precise style only",
            ),
            (
                RuleKind::TernarySubplotLinks,
                "ternary charts take no subplot axis links",
            ),
            (
                RuleKind::TernarySeriesNotTernary,
                "a ternary series is drawn on a ternary chart (Config.ternary)",
            ),
            (
                RuleKind::TernarySeriesColumns,
                "a ternary series names three distinct component columns (x_column, y_column, c_column)",
            ),
            (
                RuleKind::TernaryNothingDrawn,
                "a ternary series draws markers, a line, a region fill or any of them",
            ),
            (
                RuleKind::TernaryRegionStyle,
                "a ternary region fill's colour and opacity are finite and in 0..=1",
            ),
            (
                RuleKind::TernaryStepLine,
                "a ternary line is linear: step modes are not drawn",
            ),
            (RuleKind::TernaryLineFill, "a ternary line has no fill"),
            (
                RuleKind::TernaryChartSeriesKind,
                "a ternary chart draws only ternary series",
            ),
            (
                RuleKind::AnnotationId,
                "annotation id must be at most 160 characters without control characters",
            ),
            (
                RuleKind::AnnotationBox,
                "annotation box border width and padding must be 0..64",
            ),
            (
                RuleKind::AnnotationArrowHead,
                "annotation arrow head must be 0..256 logical pixels",
            ),
            (
                RuleKind::AnnotationPolygon,
                "annotation polygon needs at least three vertices",
            ),
            (
                RuleKind::AnnotationColor,
                "annotation colors must be finite 0..1 RGBA",
            ),
            (
                RuleKind::AnnotationLineWidth,
                "annotation line width must be 0..64 logical pixels",
            ),
            (
                RuleKind::AnnotationText,
                "invalid annotation text: size 6..160, at most 512 characters / 8 lines, finite styles",
            ),
            (
                RuleKind::AnnotationCount,
                "too many annotations: a chart holds at most 1024 (MAX_ANNOTATIONS); remove some or merge them",
            ),
            (RuleKind::FeynmanLineStyle, "invalid Feynman line style"),
            (RuleKind::FeynmanWave, "invalid Feynman wave or coil"),
            (
                RuleKind::FeynmanSizes,
                "invalid Feynman marker, arrow, loop, fan or label size",
            ),
            (
                RuleKind::FeynmanPickedStyle,
                "invalid Feynman picked style: a colour and an extra width in 0..=16",
            ),
            (
                RuleKind::FeynmanVertexIds,
                "Feynman vertex ids must be unique",
            ),
            (
                RuleKind::FeynmanPropagatorIds,
                "Feynman propagator ids must be unique",
            ),
            (
                RuleKind::FeynmanVertexPosition,
                "a Feynman vertex position must be finite",
            ),
            (RuleKind::FeynmanBlob, "invalid Feynman blob marker"),
            (RuleKind::FeynmanVertexLabel, "invalid Feynman vertex label"),
            (
                RuleKind::FeynmanPropagatorVertices,
                "a propagator must name two vertices of its series",
            ),
            (
                RuleKind::FeynmanBend,
                "a propagator's bend must be in -170..=170 degrees",
            ),
            (
                RuleKind::FeynmanPropagatorStyle,
                "invalid propagator colour or width",
            ),
            (RuleKind::FeynmanPropagatorLabel, "invalid propagator label"),
            (RuleKind::FeynmanMomentumLabel, "invalid momentum label"),
            (
                RuleKind::FeynmanColumns,
                "a Feynman series reads no columns: x_column and y_column stay empty",
            ),
            (
                RuleKind::FeynmanChart,
                "a Feynman series is drawn on a rectangular chart",
            ),
            (
                RuleKind::FeynmanNumericX,
                "a Feynman series is placed on a numeric bottom_x",
            ),
            (
                RuleKind::ColorbarCategorical,
                "a colourbar axis cannot be categorical",
            ),
            (
                RuleKind::RgbaBufferLength,
                "the RGBA8 buffer length is not width * height * 4",
            ),
            (
                RuleKind::VectorExportWholeDocument,
                "a vector export renders the whole document",
            ),
            (
                RuleKind::FitPadding,
                "padding must be finite and non-negative",
            ),
            (
                RuleKind::FitScopeLayer,
                "a fit scope names a layer the chart does not have",
            ),
            (
                RuleKind::LayerOutsideChart,
                "a layer's area lies outside the chart area",
            ),
            (
                RuleKind::NoFeasibleLayout,
                "the layered document has no feasible layout",
            ),
            (
                RuleKind::SeriesPaintOrderOverflow,
                "series paint order exceeds the GPU u32 index space",
            ),
            (
                RuleKind::HistogramBinCountOverflow,
                "histogram bin count exceeds the GPU u32 index space",
            ),
            (
                RuleKind::ValueLabelPaintOrderOverflow,
                "value label paint order exceeds the GPU u32 index space",
            ),
            (
                RuleKind::StreamedPointValueMap,
                "point value maps require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::ContourLabelSpacingScaled,
                "automatic contour label spacing_px * scale must be finite and greater than zero",
            ),
            (
                RuleKind::TriMeshColorbar,
                "a triangle mesh series needs Config.colorbar",
            ),
            (
                RuleKind::GridColumnOffsetOverflow,
                "grid column offset exceeds a 32-bit lane index",
            ),
            (
                RuleKind::FieldColorbarRequired,
                "a field series needs Config.colorbar",
            ),
            (
                RuleKind::LineFillStyleMissing,
                "series style was created without its line fill; recreate the style for the current series config",
            ),
            (
                RuleKind::ErrorBarStyleMapIndex,
                "errorbar style map expects an index column",
            ),
            (
                RuleKind::ParallelValueMapMissing,
                "a colour-mapped parallel series needs its value map prepared; recreate the chart's preparation",
            ),
            (
                RuleKind::RegionDispatchGrid,
                "the region's rows exceed the adapter's dispatch grid",
            ),
            (
                RuleKind::StreamedPolar,
                "polar charts require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedContoursOrStars,
                "stream pass does not yet support contours or arc-driven stars",
            ),
            (
                RuleKind::StreamedCategoricalAxis,
                "categorical axes require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedLineFillOrStep,
                "line fill and step interpolation require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::Streamed3d,
                "3D chart series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedPointMaps,
                "point colour and size maps require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedFieldExecutor,
                "streamed field requires the bounded tile executor",
            ),
            (
                RuleKind::StreamedLineArcPrefix,
                "streamed line requires a prepared canonical arc prefix",
            ),
            (
                RuleKind::StreamedPrimitiveColumns,
                "streamed primitive has no supplied columns",
            ),
            (
                RuleKind::StreamedPrimitiveRows,
                "streamed primitive ranges must describe the same global rows",
            ),
            (
                RuleKind::ScatterColorMapColorbar,
                "a colour-mapped scatter needs Config.colorbar: it owns the z range and colormap",
            ),
            (
                RuleKind::ScatterColorMapNeedsColorbar,
                "a colour-mapped scatter needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw",
            ),
            (
                RuleKind::FieldNeedsColorbar,
                "a field series needs Config.colorbar: it owns the z range and colormap, so without it there is no value to draw",
            ),
            (
                RuleKind::ParallelColorMapNeedsColorbar,
                "a colour-mapped parallel series needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw",
            ),
            (
                RuleKind::SeriesNotInChart,
                "no series with this id in the chart",
            ),
            (
                RuleKind::LineOffsetMemberStreamed,
                "a line offset member must be resident: streamed sources are not drawn with line offsets",
            ),
            (
                RuleKind::PlaceFeynmanVerticesNotFeynman,
                "place_feynman_vertices names a Feynman series of the chart",
            ),
            (
                RuleKind::FeynmanLabelDrag,
                "a Feynman label moves through FeynmanSeries::drag_label, the value-label drag",
            ),
            (
                RuleKind::DragFeynmanNotFeynman,
                "drag_feynman names a Feynman series of the chart",
            ),
            (
                RuleKind::ViewStateOverlayRanges,
                "a view state must carry one Y and one X range per overlay layer",
            ),
            (
                RuleKind::ViewStateSubplots,
                "a view state must carry one view per subplot, each with one Y and one X range per overlay layer",
            ),
            (
                RuleKind::RebasedSelectionStyleMap,
                "rebased selection needs a global-index style map binding",
            ),
            (
                RuleKind::ScatterStyleMapIndex,
                "scatter style map expects an index column",
            ),
            (RuleKind::NotBoxPlot, "not a box plot series"),
            (RuleKind::NotRangeBars, "not a range bar series"),
            (
                RuleKind::StreamOperationCap,
                "must be nonzero and no larger than the initial operation cap",
            ),
            (
                RuleKind::NonresidentOverlays,
                "nonresident streaming is not supported for a document with overlay layers",
            ),
            (
                RuleKind::NonresidentSubplots,
                "nonresident streaming is not supported for a document with subplots",
            ),
            (
                RuleKind::NonresidentLineOffsets,
                "nonresident streaming is not supported for a document with line offsets",
            ),
            (
                RuleKind::StreamLimits,
                "limits must be non-zero and chunk bytes must fit the in-flight budget",
            ),
            (RuleKind::NotFinitePositive, "must be finite and positive"),
            (
                RuleKind::StreamExecutionCap,
                "must be non-zero and no greater than the execution's initial cap",
            ),
            (
                RuleKind::StreamedVector,
                "vector series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedCategoryBars,
                "category bar series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedBoxPlot,
                "box plot series require a GPU-resident record, outlier column, violin curves and raw point columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedRadar,
                "radar series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamDrawCursorField,
                "stream draw cursor does not yet support field or contour passes",
            ),
            (
                RuleKind::GlyphTooLarge,
                "a glyph larger than 4096 px (MAX_GLYPH_PX) is not rasterized",
            ),
            (
                RuleKind::ChartAreaEmpty,
                "the chart area must have a non-zero width and height",
            ),
            (
                RuleKind::ColorbarProbabilityAxis,
                "a colourbar axis is linear or logarithmic: not a probability axis",
            ),
            (
                RuleKind::CategoryGroupsAxis,
                "category groups are drawn on a categorical top_x or bottom_x",
            ),
            (
                RuleKind::BreaksAxis,
                "breaks go on bottom_x, left_y or an overlay's x_axis or y_axis",
            ),
            (
                RuleKind::BreaksChart,
                "a broken axis needs a rectangular chart",
            ),
            (
                RuleKind::LayoutDataAreaEmpty,
                "margins and visual dimensions must produce a non-empty data area",
            ),
            (
                RuleKind::OverlayDataArea,
                "the overlay layers must share one non-empty data area",
            ),
            (RuleKind::SubplotIndex, "subplot index out of range"),
            (
                RuleKind::FeynmanAllocation,
                "the Feynman series' working memory could not be allocated",
            ),
            (
                RuleKind::FeynmanLayoutDidNotConverge,
                "the Feynman layout did not converge",
            ),
            (
                RuleKind::RecordWithoutFields,
                "a record needs at least one field",
            ),
            (
                RuleKind::RecordWithoutRows,
                "a record needs at least one row",
            ),
            (RuleKind::SunburstRing, "sunburst has no such ring"),
            (
                RuleKind::RadialNoDetail,
                "radial chart has no detail circle",
            ),
            (
                RuleKind::RadialDataAreaTooSmall,
                "radial data area is too small",
            ),
            (
                RuleKind::RadialLabelCanvas,
                "could not allocate label canvas",
            ),
            (
                RuleKind::RadialTargetFormat,
                "radial target must be RGBA8 or BGRA8",
            ),
            (
                RuleKind::RadialLabelWithoutText,
                "a shown radial label has no text; call RadialChart::format_labels after editing slices, or normalize_labels on a loaded document",
            ),
            (
                RuleKind::RadialCanvasSize,
                "radial canvas must be at least 160x160; scale must be at least 0.5",
            ),
            (
                RuleKind::RadialTextureLimit,
                "radial output exceeds the device texture limit",
            ),
            (
                RuleKind::RadialReadbackLimit,
                "radial readback exceeds the device buffer limit",
            ),
            (
                RuleKind::TriMeshNeedsColorbar,
                "a triangle mesh series needs Config.colorbar: it owns the z range and colormap, so without it there is no value to draw",
            ),
            (
                RuleKind::StreamlineColorMapNeedsColorbar,
                "colour-mapped streamlines need Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw",
            ),
            (
                RuleKind::VectorGridLayout,
                "vector u and v grids must share orientation and grid layout",
            ),
            (
                RuleKind::VectorColorGridLayout,
                "vector colour grid must share the u grid's orientation and grid layout",
            ),
            (
                RuleKind::VectorColorMapForm,
                "vector colour map must take the components' form: a column for points, a grid for grids",
            ),
            (
                RuleKind::VectorColorMapNeedsColorbar,
                "a colour-mapped vector needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw",
            ),
            (
                RuleKind::LineFillBaseline,
                "line fill baseline must be finite",
            ),
            (
                RuleKind::LineFillSelf,
                "line fill cannot reference the series itself",
            ),
            (
                RuleKind::NonresidentOverlaySeries,
                "a nonresident streaming series cannot be scaled by an overlay layer",
            ),
            (
                RuleKind::NonresidentSubplotSeries,
                "a nonresident streaming series cannot be drawn by a subplot",
            ),
            (
                RuleKind::ParallelColumnAlignment,
                "parallel column byte offset is not f32-aligned",
            ),
            (
                RuleKind::ParallelColumnLaneOverflow,
                "parallel column lane past u32",
            ),
            (
                RuleKind::ParallelRowCountOverflow,
                "parallel row count past u32",
            ),
            (
                RuleKind::ParallelAxisCountOverflow,
                "parallel axis count past u32",
            ),
            (RuleKind::NotCategoryBars, "not a category bar series"),
            (
                RuleKind::NotInBarGroup,
                "the series is not in its bar group",
            ),
            (
                RuleKind::NotInBoxGroup,
                "the series is not in its box group",
            ),
            (
                RuleKind::BoxOutlierGroupTooLong,
                "a box plot outlier group is too long to draw",
            ),
            (RuleKind::NoViolin, "the box plot series draws no violin"),
            (
                RuleKind::ViolinCurvesTooLong,
                "the violin curves are too long to draw",
            ),
            (RuleKind::NoBoxPoints, "the box plot series draws no points"),
            (
                RuleKind::NotInRangeGroup,
                "the series is not in its range group",
            ),
            (
                RuleKind::RegionColumnAlignment,
                "region column byte offset is not f32-aligned",
            ),
            (
                RuleKind::RegionColumnLaneOverflow,
                "region column lies past the 32-bit lane range",
            ),
            (
                RuleKind::RegionRowsOverflow,
                "region rows exceed the 32-bit row range",
            ),
            (
                RuleKind::ColumnOffsetLaneOverflow,
                "column offset exceeds a 32-bit lane index",
            ),
            (
                RuleKind::VectorGridCellsOverflow,
                "vector grid cell count exceeds the u32 instance space",
            ),
            (
                RuleKind::VectorColorMapColorbar,
                "a colour-mapped vector needs Config.colorbar: it owns the z range and colormap",
            ),
            (
                RuleKind::RaggedEmpty,
                "a ragged column needs at least one value; register none when every group is empty",
            ),
            (
                RuleKind::RaggedOffsetsOrder,
                "the offsets source must write every offset once, in row order",
            ),
            (
                RuleKind::RaggedOffsetsInteger,
                "ragged offsets must be finite non-negative integers",
            ),
            (
                RuleKind::RaggedOffsetsDecreasing,
                "ragged offsets must be non-decreasing: group g is values[offsets[g]..offsets[g + 1]]",
            ),
            (
                RuleKind::TrianglesEmpty,
                "a triangle index column needs at least one triangle (three indices)",
            ),
            (
                RuleKind::TriangleIndicesOrder,
                "the triangle index source must write every index once, in row order",
            ),
            (
                RuleKind::StreamedRangeBars,
                "range bar series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedHistogram,
                "histogram series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedStreamlines,
                "streamline series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedHexagonal,
                "hexagonal grids require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedTernary,
                "ternary series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedBreak,
                "broken axes require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedProbability,
                "probability axes require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedImage,
                "image series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedParallel,
                "parallel coordinates series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedTriMesh,
                "triangle mesh series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedStem,
                "stem series require GPU-resident columns; streamed sources are not supported",
            ),
            (
                RuleKind::StreamedEnergyLevels,
                "energy-level series are DataLess (SSOT only); streamed displays do not draw them",
            ),
            (
                RuleKind::StreamedFeynman,
                "Feynman series are DataLess (SSOT only); streamed displays do not draw them",
            ),
        ];
        for (kind, sentence) in pinned {
            assert_eq!(kind.to_string(), *sentence, "{}", kind.code());
            assert_eq!(format!("{kind:?}"), kind.code());
        }
    }

    /// Every fielded rule's sentence with representative fields, and every
    /// code the variant's name as `Debug` spells it (#341): the sentences
    /// are hand-written in `fmt_fielded`, the codes come from `rule_kinds!`.
    #[test]
    fn every_fielded_rule_reads_with_its_fields() {
        let pinned: &[(RuleKind, &str)] = &[
            (
                RuleKind::PackedPickLane {
                    detail: "detail_v".into(),
                },
                "invalid packed view pick lane: detail_v",
            ),
            (
                RuleKind::PickChunk {
                    detail: "detail_v".into(),
                },
                "invalid pick chunk: detail_v",
            ),
            (
                RuleKind::PackedSelectionLane {
                    detail: "detail_v".into(),
                },
                "invalid packed selection lane: detail_v",
            ),
            (
                RuleKind::StreamStyleIndexShort {
                    column: "column_v".into(),
                    len: 3537,
                    need: 3538,
                },
                "style index column \"column_v\" has 3537 values, but this pass uses 3538",
            ),
            (
                RuleKind::BoxPlotRecordNotResident { id: "id_v".into() },
                "box plot record \"id_v\" is not resident: upload it with `upsert_record`",
            ),
            (
                RuleKind::BoxPlotRaggedNotResident { id: "id_v".into() },
                "box plot ragged column \"id_v\" is not resident: upload it with `upsert_ragged`",
            ),
            (
                RuleKind::BoxPlotNotResident { id: "id_v".into() },
                "box plot \"id_v\" is not a resident record or ragged column: upload the record with `upsert_record` and the outliers and violin curves with `upsert_ragged`",
            ),
            (
                RuleKind::StreamedCompositeId {
                    kind: "kind_v".into(),
                },
                "the id is kind_v; streaming is not extended to records, ragged and triangle columns, which are resident only",
            ),
            (
                RuleKind::CompositeNeedsResident {
                    kind: "kind_v".into(),
                },
                "the id is a streamed column; streaming is not extended to records, ragged and triangle columns, so kind_v needs a resident id",
            ),
            (
                RuleKind::RadarValueCount {
                    categories: 3578,
                    index_column: "index_column_v".into(),
                    index_len: 3580,
                    value_column: "value_column_v".into(),
                    value_len: 3582,
                },
                "a radar series holds one value per category: the angle axis has 3578 categories, the category index column \"index_column_v\" 3580 values and the value column \"value_column_v\" 3582",
            ),
            (
                RuleKind::CategoryBarColumnLength {
                    column: "column_v".into(),
                    len: 3586,
                    categories: 3587,
                },
                "category bar column \"column_v\" has 3586 values for 3587 categories; it needs one per category",
            ),
            (
                RuleKind::PercentStackedNegative {
                    column: "column_v".into(),
                    series_id: "series_id_v".into(),
                },
                "a PercentStacked bar group needs non-negative values: the value column \"column_v\" of series \"series_id_v\" has a negative value; draw it Stacked or Grouped instead",
            ),
            (
                RuleKind::MonotonicChannel {
                    channel: 3599,
                    channels: 3600,
                },
                "a monotonic series draws channel 3599 of a source with 3600 channels",
            ),
            (
                RuleKind::MonotonicColumnTaken {
                    column: "column_v".into(),
                },
                "monotonic envelope column \"column_v\" is already a column the series does not own",
            ),
            (
                RuleKind::RangeBarColumnLength {
                    column: "column_v".into(),
                    len: 3614,
                    positions: 3615,
                    position_column: "position_column_v".into(),
                },
                "range bar column \"column_v\" has 3614 values for the 3615 positions of \"position_column_v\"; it needs one per row",
            ),
            (
                RuleKind::PointMapColumnShort {
                    what: "what_v".into(),
                    column: "column_v".into(),
                    len: 3622,
                    need: 3623,
                },
                "what_v column \"column_v\" has 3622 values, but scatter uses 3623",
            ),
            (
                RuleKind::ScatterStyleIndexShort {
                    column: "column_v".into(),
                    len: 3628,
                    need: 3629,
                },
                "style index column \"column_v\" has 3628 values, but scatter uses 3629",
            ),
            (
                RuleKind::ErrorBarStyleIndexShort {
                    column: "column_v".into(),
                    len: 3635,
                    need: 3636,
                },
                "errorbar style index column \"column_v\" has 3635 values, but errorbar uses 3636",
            ),
            (
                RuleKind::HexagonalCoordinatesNotUniform {
                    role: "role_v".into(),
                    column: "column_v".into(),
                },
                "a hexagonal grid's role_v coordinates (\"column_v\") are uniform: a resident column of at least two values, every step equal to the first within 1e-6",
            ),
            (
                RuleKind::InvalidLabel {
                    detail: "detail_v".into(),
                },
                "label: detail_v",
            ),
            (
                RuleKind::ValueLabels {
                    detail: "detail_v".into(),
                },
                "value labels: detail_v",
            ),
            (
                RuleKind::ContourLabelAtlas {
                    detail: "detail_v".into(),
                },
                "detail_v",
            ),
            (
                RuleKind::LineFillTargetMissing {
                    target: "target_v".into(),
                },
                "line fill references series `target_v`, which is not in this chart",
            ),
            (
                RuleKind::LineFillXColumn {
                    target: "target_v".into(),
                    x_column: "x_column_v".into(),
                    target_x_column: "target_x_column_v".into(),
                },
                "line fill to series `target_v` requires the same x column: `x_column_v` vs `target_x_column_v`",
            ),
            (
                RuleKind::ContourLevelCount {
                    count: 3683,
                    max: 3684,
                },
                "contour level count 3683 exceeds the supported maximum 3684",
            ),
            (
                RuleKind::InvalidMath {
                    error: crate::text::MathError {
                        at: 3,
                        kind: crate::text::MathErrorKind::TooDeep,
                    },
                },
                "math nodes are nested deeper than 16",
            ),
            (
                RuleKind::StreamedColumnSpan {
                    detail: "detail_v".into(),
                },
                "invalid streamed column span: detail_v",
            ),
            (
                RuleKind::IdInUse {
                    other: "other_v".into(),
                },
                "the id is other_v; columns, records, ragged and triangle columns share one id space",
            ),
            (
                RuleKind::CompositeIdEmpty {
                    kind: "kind_v".into(),
                },
                "a kind_v id must not be empty",
            ),
            (
                RuleKind::CompositeIdSeparator {
                    kind: "kind_v".into(),
                },
                "a kind_v id must not contain U+001F, which separates member column ids",
            ),
            (
                RuleKind::CompositeIdIsColumn {
                    kind: "kind_v".into(),
                },
                "the id is a column; columns, records, ragged and triangle columns share one id space, so a kind_v needs an id of its own",
            ),
            (
                RuleKind::RecordSchemaFields {
                    fields: 3732,
                    max: 3733,
                },
                "the record schema has 3732 fields; a record holds at most MAX_RECORD_FIELDS (3733)",
            ),
            (
                RuleKind::RecordSourceFields {
                    fields: 3739,
                    max: 3740,
                },
                "the record source has 3739 fields; a record holds at most MAX_RECORD_FIELDS (3740)",
            ),
            (
                RuleKind::RecordUnknownField {
                    name: "name_v".into(),
                },
                "'name_v' is not a field of the record schema",
            ),
            (
                RuleKind::RecordFieldTwice {
                    name: "name_v".into(),
                },
                "the record field 'name_v' is given twice",
            ),
            (
                RuleKind::RecordFieldMissing {
                    name: "name_v".into(),
                },
                "the required record field 'name_v' is missing",
            ),
            (
                RuleKind::RecordFieldLength {
                    name: "name_v".into(),
                    len: 3768,
                    first: "first_v".into(),
                    rows: 3770,
                },
                "the record field 'name_v' has 3768 rows but 'first_v' has 3770; every field of a record has the same length",
            ),
            (
                RuleKind::RaggedOffsetCount { offsets: 3774 },
                "a ragged column needs one offset per group plus one, at least two, got 3774",
            ),
            (
                RuleKind::RaggedOffsetsStart {
                    first: "first_v".into(),
                },
                "ragged offsets must start at 0, got first_v",
            ),
            (
                RuleKind::RaggedLastOffset {
                    last: "last_v".into(),
                    total: 3789,
                },
                "the last ragged offset is last_v but there are 3789 values",
            ),
            (
                RuleKind::TriangleIndexCount { count: 3795 },
                "a triangle index column holds three vertex indices per triangle; 3795 indices is not a multiple of 3",
            ),
            (
                RuleKind::TriangleIndexRange { max: 3802 },
                "triangle indices must be finite integers in 0..=3802",
            ),
            (
                RuleKind::RecordRowsOverflow { max: 3809 },
                "a record holds at most 3809 rows",
            ),
            (
                RuleKind::RaggedValuesOverflow {
                    max: 3816,
                    total: 3817,
                },
                "a ragged column indexes its values with u32 offsets, at most 3816 values, got 3817",
            ),
            (
                RuleKind::RaggedGroupsOverflow { max: 3823 },
                "a ragged column holds at most 3823 groups",
            ),
            (
                RuleKind::TriangleIndicesOverflow {
                    max: 3830,
                    count: 3831,
                },
                "a triangle index column holds at most 3830 indices, got 3831",
            ),
            (
                RuleKind::NotBoxPlotRecord { id: "id_v".into() },
                "record \"id_v\" is not a box plot record (`BOX_PLOT_RECORD`)",
            ),
            (
                RuleKind::BoxPlotRecordRows {
                    id: "id_v".into(),
                    rows: 3845,
                    categories: 3846,
                },
                "box plot record \"id_v\" has 3845 rows for 3846 categories; it needs one per category",
            ),
            (
                RuleKind::BoxPlotOutliersNotResident { id: "id_v".into() },
                "box plot outlier column \"id_v\" is not resident: upload it with `upsert_ragged`",
            ),
            (
                RuleKind::BoxPlotOutlierGroups {
                    id: "id_v".into(),
                    groups: 3859,
                    categories: 3860,
                },
                "box plot outlier column \"id_v\" has 3859 groups for 3860 categories; it needs one per category",
            ),
            (
                RuleKind::BoxPlotPositions {
                    column: "column_v".into(),
                    len: 3866,
                    categories: 3867,
                },
                "box plot position column \"column_v\" has 3866 values for 3867 categories; it needs one per category",
            ),
            (
                RuleKind::ViolinCurveNotResident { id: "id_v".into() },
                "violin curve \"id_v\" is not resident: upload it with `upsert_ragged`",
            ),
            (
                RuleKind::ViolinCurveGroups {
                    id: "id_v".into(),
                    groups: 3880,
                    categories: 3881,
                },
                "violin curve \"id_v\" has 3880 groups for 3881 categories; it needs one per category",
            ),
            (
                RuleKind::ViolinDensityLength {
                    column: "column_v".into(),
                    len: 3887,
                    values: 3888,
                    curve: "curve_v".into(),
                },
                "violin density \"column_v\" has 3887 values for the 3888 values of curve \"curve_v\"; it needs one per curve value",
            ),
            (
                RuleKind::TrianglesNotResident { id: "id_v".into() },
                "triangle mesh \"id_v\" is not a resident triangle index column: upload it with `upsert_triangles`",
            ),
            (
                RuleKind::TriangleIndexPastVertices {
                    index: 3900,
                    vertices: 3901,
                    column: "column_v".into(),
                },
                "triangle index 3900 names a vertex past the 3901 vertices of column \"column_v\"",
            ),
            (
                RuleKind::FaceValueColumnShort {
                    column: "column_v".into(),
                    len: 3908,
                    triangles: 3909,
                },
                "face value column \"column_v\" has 3908 values for 3909 triangles",
            ),
        ];
        for (kind, sentence) in pinned {
            assert_eq!(kind.to_string(), *sentence, "{}", kind.code());
            let debug = format!("{kind:?}");
            assert_eq!(debug.split(' ').next(), Some(kind.code()));
        }
    }

    #[test]
    fn a_fielded_rule_reads_with_its_fields() {
        let kind = RuleKind::CategoryBarColumnLength {
            column: "sales".into(),
            len: 3,
            categories: 4,
        };
        assert_eq!(
            kind.to_string(),
            "category bar column \"sales\" has 3 values for 4 categories; it needs one per category"
        );
        assert_eq!(kind.code(), "CategoryBarColumnLength");
    }
}

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

/// One rule a chart, a series or a renderer request broke.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuleKind {
    /// view\_3d.azimuth\_degrees must be finite
    View3dAzimuthNotFinite,
    /// view\_3d.elevation\_degrees must be in -90..=90
    View3dElevationOutOfRange,
    /// view\_3d.distance must be in 1.2..=100
    View3dDistanceOutOfRange,
    /// view\_3d.box\_aspect lengths must be in 0.05..=20
    View3dBoxAspectOutOfRange,
    /// 3D chart axes (bottom\_x, left\_y, right\_y) cannot be categorical
    View3dCategoricalAxis,
    /// 3D chart axes must be linear or logarithmic
    View3dAxisScale,
    /// 3D charts have no overlay layers
    View3dOverlay,
    /// 3D charts have no line offsets
    View3dLineOffsets,
    /// 3D charts are drawn in the Precise style only
    View3dDrawStyle,
    /// stems draw flat: not on a 3D chart
    View3dStem,
    /// a scatter or line series on a 3D chart names its height column (z\_column)
    View3dPointsWithoutZColumn,
    /// a line on a 3D chart is linear: no step interpolation
    View3dStepLine,
    /// a line on a 3D chart has no area fill
    View3dLineFill,
    /// a grid vector draws flat: not on a 3D chart
    View3dGridVector,
    /// a vector series on a 3D chart names its anchor height column (z\_column)
    View3dVectorWithoutZColumn,
    /// a vector series on a 3D chart names its z component (w)
    View3dVectorWithoutW,
    /// an image draws flat: not on a 3D chart
    View3dImage,
    /// a parallel series draws flat: not on a 3D chart
    View3dParallel,
    /// a 3D chart draws only heatmaps, contours, heatmap contours, and the scatter, line and point vector series that name a z\_column
    View3dSeriesKind,
    /// a hexagonal grid draws flat: not on a 3D chart
    View3dHexagonalGrid,
    /// a surface's fill is continuous: banded fills are not drawn in 3D
    View3dBandedSurface,
    /// contours on a 3D chart's floor have no labels
    View3dContourLabels,
    /// a surface's color\_matrix has the column count, orientation and layout of its matrix
    SurfaceColorMatrixShape,
    /// a surface's color\_matrix names its columns
    SurfaceColorMatrixColumns,
    /// a surface wireframe has a width in 0..=24 and a colour in 0..=1
    SurfaceWireframeStyle,
    /// a surface's gloss, texture\_strength and emphasis\_brightness are in 0..=1 and its texture\_scale is positive
    SurfaceMaterialStyle,
    /// a radial chart needs 1..64 slices
    RadialSliceCount,
    /// slice values must be finite and nonnegative
    RadialSliceValue,
    /// slice colors must be finite opaque RGBA in 0..1
    RadialSliceColor,
    /// invalid slice label color
    RadialSliceLabelColor,
    /// slice explode must be in 0..0.35
    RadialSliceExplode,
    /// slice labels must be single-line, at most 80 characters
    RadialSliceLabel,
    /// slice label text must have at most two lines of at most 400 characters, without control characters
    RadialSliceLabelText,
    /// slice total must be positive and finite
    RadialSliceTotal,
    /// invalid radial material, light, or projection
    RadialStyle,
    /// invalid radial title: size 6..160, at most 512 characters / 8 lines, finite styles
    RadialTitle,
    /// a radial chart title keeps its position in decorations: its offsets and top\_margin must be 0
    RadialTitleOffsets,
    /// invalid radial shape or start angle
    RadialShape,
    /// invalid radial text or background
    RadialTextOrBackground,
    /// invalid radial label format or interaction
    RadialLabelFormatOrInteraction,
    /// radial interaction target is out of range
    RadialInteractionTarget,
    /// cannot emphasize a zero-valued slice
    RadialEmphasizedZeroSlice,
    /// a sunburst has no detail circle
    SunburstDetail,
    /// split slice index is out of range
    RadialSplitIndex,
    /// detail values must sum to the parent slice value
    RadialDetailSum,
    /// invalid detail chart shape
    RadialDetailShape,
    /// a sunburst has at most 8 rings
    SunburstRingCount,
    /// a sunburst ring has at most 64 nodes
    SunburstNodeCount,
    /// a sunburst draws flat: no tilt or depth
    SunburstTilt,
    /// a node's children sum to at most its value
    SunburstChildrenSum,
    /// an energy-level series with a source holds no records of its own
    EnergyLevelsSourceWithRecords,
    /// energy level ids must be non-empty and unique
    EnergyLevelIds,
    /// ionization limit ids must be non-empty and unique
    IonizationLimitIds,
    /// transition ids must be non-empty and unique
    TransitionIds,
    /// level band ids must be non-empty and unique
    LevelBandIds,
    /// a level band's first and last name levels of its column
    LevelBandEnds,
    /// a level band's label offset is out of range
    LevelBandLabelOffset,
    /// a fine-structure component must name a centroid level
    FineStructureCentroid,
    /// a transition joins two different levels
    TransitionSameLevel,
    /// a transition must name two levels of its series
    TransitionLevels,
    /// an energy level's energy must be finite
    EnergyLevelEnergy,
    /// an energy level's uncertainty and g must be finite and not negative
    EnergyLevelUncertainty,
    /// an energy level's column must be set and its text fields one line of at most 160 characters
    EnergyLevelText,
    /// a component's multiplet id is one line of at most 160 characters
    ComponentMultipletId,
    /// invalid energy level label
    EnergyLevelLabel,
    /// an ionization limit's energy must be finite and its uncertainty not negative
    IonizationLimitEnergy,
    /// invalid ionization limit label or reference
    IonizationLimitLabel,
    /// a transition's values must be finite and its uncertainties not negative
    TransitionValues,
    /// a transition's x\_offset must be in -0.5..=0.5 of the slot
    TransitionXOffset,
    /// a transition's text fields are one line of at most 160 characters
    TransitionText,
    /// invalid transition label
    TransitionLabel,
    /// invalid energy level stroke style
    EnergyLevelStroke,
    /// invalid ionization limit line style
    IonizationLimitLine,
    /// invalid transition arrow style
    TransitionArrow,
    /// picked\_width\_scale must be in 1..=4
    EnergyLevelsPickedWidth,
    /// invalid level band style
    LevelBandStyle,
    /// an energy-level series reads no columns: x\_column and y\_column stay empty
    EnergyLevelsColumns,
    /// an energy-level series cannot be its own source
    EnergyLevelsSelfSource,
    /// an energy-level series is drawn on a rectangular chart
    EnergyLevelsChart,
    /// an energy-level series needs a categorical bottom\_x: its term columns
    EnergyLevelsCategoricalX,
    /// box statistics must be finite and low \<= Q1 \<= median \<= Q3 \<= high
    BoxStatsOrder,
    /// mean must be finite; sample count must be positive when supplied
    BoxStatsMeanOrCount,
    /// median CI must be finite and contain the median
    BoxStatsMedianCi,
    /// outliers must be finite and strictly outside the whiskers
    BoxStatsOutliers,
    /// boxplot limits: at least 1 category and 1 series
    BoxPlotChartLimits,
    /// invalid category ID/label
    BoxPlotChartCategory,
    /// invalid series ID/label, color or value count
    BoxPlotChartSeries,
    /// invalid boxplot layout, font or colors
    BoxPlotChartStyle,
    /// invalid title
    BoxPlotChartTitle,
    /// invalid linear value range
    BoxPlotChartValueRange,
    /// invalid or duplicate box override
    BoxPlotChartOverride,
    /// internal: a box with a value resolved no style
    BoxPlotChartUnresolvedStyle,
    /// notched boxes require an explicit median CI
    BoxPlotChartNotchWithoutCi,
    /// unknown or missing interaction target
    BoxPlotChartInteractionTarget,
    /// polar angles must be finite
    PolarAngles,
    /// polar inner\_radius\_fraction must be within 0..=0.9
    PolarInnerRadius,
    /// polar sector must be a finite \[start, end\] with start \< end
    PolarSector,
    /// polar sector cannot exceed one turn
    PolarSectorTurn,
    /// polar arc pieces must be within 1..=64
    PolarArcPieces,
    /// the polar angle axis (bottom\_x) must be linear
    PolarAngleAxisScale,
    /// the polar angle axis (bottom\_x) cannot be inverted; use clockwise
    PolarAngleAxisInverted,
    /// a polygon polar grid joins its rings at the categories: it needs a categorical angle axis (bottom\_x.categories)
    PolarPolygonGridAxis,
    /// the polar radius axis (left\_y) cannot be categorical
    PolarRadiusCategorical,
    /// the polar radius axis (left\_y) is linear or logarithmic
    PolarRadiusScale,
    /// polar charts have no overlay layers
    PolarOverlay,
    /// polar charts have no line offsets
    PolarLineOffsets,
    /// data area must fit the measured plot region and be at least 80 by 60 logical pixels
    DataAreaTooSmall,
    /// too many decoration offsets
    DecorationOffsetCount,
    /// decoration offsets must be finite and unique
    DecorationOffsets,
    /// axes only move perpendicular to their direction
    DecorationAxisMove,
    /// radar limits: at least 3 categories and 1 series
    RadarChartLimits,
    /// invalid radar style
    RadarChartSeriesStyle,
    /// invalid radar layout, font or colors
    RadarChartStyle,
    /// invalid value range
    RadarChartValueRange,
    /// ring spacing must be finite and positive
    RadarChartRingSpacing,
    /// unknown interaction target
    RadarChartInteractionTarget,
    /// an axis crossing (`cross\_at`) must be finite
    AxisCrossAt,
    /// a category window needs finite bounds with from \< to
    CategoryWindow,
    /// axis bounds must be finite
    AxisBoundsNotFinite,
    /// axis max must be greater than min
    AxisBoundsOrder,
    /// logarithmic axis bounds must be positive
    LogAxisBounds,
    /// probability axis bounds must lie strictly between 0 and 1
    ProbabilityAxisBounds,
    /// series\_id must not be empty
    SeriesIdEmpty,
    /// series\_id must be unique within a chart
    SeriesIdDuplicate,
    /// error bars are not drawn on polar axes
    PolarErrorBars,
    /// stems are not drawn on polar axes
    PolarStems,
    /// histogram series are not drawn on polar axes
    PolarHistogram,
    /// a hexagonal grid is drawn on a rectangular chart
    PolarHexagonalGrid,
    /// a polar heatmap's cells follow circular rings: not on a polygon grid
    PolarHeatmapPolygonGrid,
    /// images are not drawn on polar axes
    PolarImage,
    /// vector series are not drawn on polar axes
    PolarVector,
    /// box plots are not drawn on polar axes
    PolarBoxPlot,
    /// range bars are not drawn on polar axes
    PolarRangeBars,
    /// streamlines are not drawn on polar axes
    PolarStreamlines,
    /// parallel series are drawn on parallel charts, not polar axes
    PolarParallel,
    /// energy-level series are not drawn on polar axes
    PolarEnergyLevels,
    /// a triangle mesh is not drawn on polar axes
    PolarTriMesh,
    /// Feynman series are not drawn on polar axes
    PolarFeynman,
    /// ternary series are drawn on ternary charts, not polar axes
    PolarTernary,
    /// a polar arc interpolation needs a linearly interpolated line; a step interpolation draws chords
    PolarArcStepLine,
    /// material not supported by bars
    CategoricalChartBarMaterial,
    /// categorical limits: at least 1 category and 1 series
    CategoricalChartLimits,
    /// invalid or duplicate category ID/label
    CategoricalChartCategory,
    /// invalid series, value count, color or style
    CategoricalChartSeries,
    /// values must be finite; percent stacks require nonnegative values
    CategoricalChartValues,
    /// invalid categorical style or layout
    CategoricalChartStyle,
    /// titles must be single-line, at most 160 characters
    CategoricalChartTitles,
    /// invalid value suffix
    CategoricalChartValueSuffix,
    /// invalid or duplicate bar override
    CategoricalChartOverride,
    /// interaction target is missing or unknown
    CategoricalChartInteractionTarget,
    /// stack total overflow
    CategoricalChartStackOverflow,
    /// invalid bounded title: size 6..160, at most 512 characters / 8 lines, finite styles
    BoundedTitle,
    /// a triangle mesh names its triangle index column
    TriMeshTriangles,
    /// a triangle mesh names its value column
    TriMeshValues,
    /// a triangle mesh draws a fill, contour lines or both
    TriMeshNothingDrawn,
    /// a surface look needs the triangle mesh's fill
    TriMeshSurfaceWithoutFill,
    /// vertex values are shaded Interpolated: a flat triangle colour from vertex values is host work (give face values)
    TriMeshVertexValuesFlat,
    /// face values are shaded Flat: interpolating them needs vertex values, which are host work
    TriMeshFaceValuesInterpolated,
    /// contour lines need vertex values: a field constant on each triangle has no isolines
    TriMeshContourFaceValues,
    /// contour lines on a triangle mesh have no labels
    TriMeshContourLabels,
    /// a triangle mesh is coloured by its values: no surface color\_matrix
    TriMeshSurfaceColorMatrix,
    /// a triangle mesh names its vertex coordinate columns (x\_column, y\_column)
    TriMeshVertexColumns,
    /// a triangle mesh is not drawn on a ternary chart
    TriMeshTernary,
    /// a triangle mesh is not drawn on a parallel chart
    TriMeshParallel,
    /// a triangle mesh on a 3D chart names its vertex height column (z\_column)
    TriMeshWithoutZColumn,
    /// a subplot cannot have subplots of its own
    SubplotNested,
    /// the document's annotation layer belongs to the parent, not a subplot
    SubplotAnnotations,
    /// a subplot link must point at the parent or an earlier subplot
    SubplotLink,
    /// a series id may belong to at most one subplot or overlay layer
    SeriesInManyLayers,
    /// a series id may belong to at most one overlay layer
    SeriesInManyOverlays,
    /// a category group names categories of its axis
    CategoryGroupCategories,
    /// a category group's first category comes before its last
    CategoryGroupOrder,
    /// category groups run in axis order without overlapping
    CategoryGroupOverlap,
    /// invalid category group label
    CategoryGroupLabel,
    /// invalid category group offset
    CategoryGroupOffset,
    /// a categorical axis cannot be broken
    BreakCategorical,
    /// an axis can cut out at most three ranges
    BreakCount,
    /// a cut range needs finite bounds with from \< to
    BreakRange,
    /// a cut range on a logarithmic axis must be positive
    BreakLogRange,
    /// a cut range on a probability axis must lie strictly between 0 and 1
    BreakProbabilityRange,
    /// cut ranges must be sorted and must not touch
    BreakOrder,
    /// a break gap must be between 0 and 200 px
    BreakGap,
    /// only a Continuous broken axis sizes its pieces other than by value
    BreakSizing,
    /// piece weights need one weight per piece: one more than cut ranges
    BreakWeightCount,
    /// piece weights must be finite and positive
    BreakWeights,
    /// a break mark needs a size in 0..=64, a width in 0..=16 and a colour in range
    BreakMark,
    /// a categorical axis needs at least one category
    CategoriesEmpty,
    /// category ids must be unique, non-empty, single-line and at most 80 characters
    CategoryIds,
    /// category labels must be single-line and at most 80 characters
    CategoryLabels,
    /// invalid category rich label
    CategoryRichLabel,
    /// a categorical axis cannot be logarithmic
    CategoricalLogAxis,
    /// a categorical axis cannot be a probability axis
    CategoricalProbabilityAxis,
    /// a stem baseline must be finite
    StemBaseline,
    /// a stem width must be finite and at least 0
    StemWidth,
    /// an xy series draws markers, a line or both
    XyNothingDrawn,
    /// error bars are drawn with markers
    ErrorBarsWithoutMarkers,
    /// error bars run along x, y or both
    ErrorBarsDirection,
    /// stems are drawn with markers
    StemsWithoutMarkers,
    /// a field draws a fill, contour lines or both
    FieldNothingDrawn,
    /// a surface look needs the field's fill
    FieldSurfaceWithoutFill,
    /// an xy series draws markers or a line
    XyWireNothingDrawn,
    /// streamline arrows need a finite spacing\_px \> 0, finite non-negative head\_length\_px and head\_width\_px, and spacing\_px \>= head\_length\_px
    StreamlineArrows,
    /// streamlines are polylines: the line interpolation must be Linear
    StreamlineInterpolation,
    /// streamlines have no fill
    StreamlineFill,
    /// a streamline series names its x\_column and y\_column
    StreamlineColumns,
    /// a streamline colour map names its column
    StreamlineColorMapColumn,
    /// vector width\_px, head\_length\_px, head\_width\_px and min\_length\_px must be finite and non-negative
    VectorPixels,
    /// vector scale must be finite and non-zero
    VectorScale,
    /// point size map domain must be finite with domain\_min \< domain\_max
    PointSizeMapDomain,
    /// point size map min\_px and max\_px must be finite, non-negative and min\_px \<= max\_px
    PointSizeMapPixels,
    /// contour label spacing\_px must be finite and greater than zero
    ContourLabelSpacing,
    /// contour label anchors exceed the supported maximum of 1024
    ContourLabelAnchors,
    /// value label offsets need one entry per category
    ValueLabelOffsetCount,
    /// a value label offset is finite and at most 4096 px along either axis
    ValueLabelOffset,
    /// invalid bar style
    BarStyle,
    /// bar groups need a group width in 0.1..=0.95 and a bar gap in 0..=24 px
    BarGroups,
    /// line offset steps must be finite
    LineOffsetSteps,
    /// a line offset member is a non-empty series id, listed once
    LineOffsetMember,
    /// a line offset member must be drawn by the chart that lists it
    LineOffsetMemberLayer,
    /// a line offset member names no series of the chart
    LineOffsetMemberUnknown,
    /// a line offset member must draw a line: Line, ScatterLine or a LineScatterErrorbar kind
    LineOffsetMemberNotLine,
    /// a ToSeries fill cannot join series with different line offsets
    LineOffsetFill,
    /// a bar colour must be opaque with channels in 0..=1
    BarColor,
    /// bar labels need 0..=6 decimals and a one-line suffix of at most 24 characters
    BarLabelFormat,
    /// bar label text needs one string per category
    BarLabelTextCount,
    /// a bar label is one line of at most 160 characters
    BarLabel,
    /// a bar override names a category of the axis and its own series, once
    BarOverride,
    /// horizontal category bars cannot sit on an overlay layer: its own axis is a value axis on y
    HorizontalBarsOnOverlay,
    /// a rose fans its bars around the centre: the group direction on a polar chart is vertical
    RoseDirection,
    /// category bars need a categorical axis: bottom\_x for vertical bars, left\_y for horizontal ones
    CategoryBarsAxis,
    /// category bars need a linear numeric value axis
    CategoryBarsValueAxis,
    /// the category bar series of a group need distinct value columns
    CategoryBarsValueColumns,
    /// material not supported by boxes
    BoxMaterial,
    /// invalid boxplot style
    BoxStyle,
    /// a box colour must be opaque with channels in 0..=1
    BoxColor,
    /// box labels need 0..=6 decimals and a one-line suffix of at most 24 characters
    BoxLabelFormat,
    /// box label text needs one string per category
    BoxLabelTextCount,
    /// a box label is one line of at most 160 characters
    BoxLabel,
    /// a box plot series names its category positions (x\_column), its record (y\_column) unless it draws a violin, and, when set, its outlier column
    BoxPlotColumns,
    /// a violin's box width is a share of the slot in 0..=1
    ViolinBoxWidth,
    /// a split violin draws at least one half
    ViolinHalves,
    /// a violin half names a ragged curve and a distinct density column
    ViolinHalfColumns,
    /// a violin half's density\_scale is finite and positive
    ViolinDensityScale,
    /// a violin outline is 0..=8 px wide with channels in 0..=1, its fill opaque
    ViolinOutline,
    /// box points name distinct position and value columns
    BoxPointsColumns,
    /// box points are 1..=24 px, jittered by 0..=1 and offset by -1..=1 of the half slot, with colour channels in 0..=1
    BoxPointsStyle,
    /// a box override names a category of the axis and its own series, once
    BoxOverride,
    /// a layer draws either category bars or box plots, not both
    BarsAndBoxesInLayer,
    /// box plots stand side by side: their bar\_groups mode must be Grouped, not Stacked or PercentStacked
    BoxPlotsStacked,
    /// horizontal box plots cannot sit on an overlay layer: its own axis is a value axis on y
    HorizontalBoxPlotsOnOverlay,
    /// box plots need a categorical axis: bottom\_x for vertical boxes, left\_y for horizontal ones
    BoxPlotsAxis,
    /// box plots need a linear numeric value axis
    BoxPlotsValueAxis,
    /// a range bar colour must be opaque with channels in 0..=1
    RangeBarColor,
    /// a range bar series names its positions (x\_column), its end values (y\_column), its start values and every optional column
    RangeBarColumns,
    /// the columns of a range bar series are distinct
    RangeBarColumnsDistinct,
    /// range bar whiskers are 0..=24 px wide, in an opaque colour
    RangeBarWhiskers,
    /// a range bar connector is 0..=24 px wide, its colour channels in 0..=1
    RangeBarConnector,
    /// range bar direction and state colours must be opaque
    RangeBarStateColors,
    /// a range bar's remaining opacity is in 0..=1
    RangeBarRemainingOpacity,
    /// a range bar slot width is finite and greater than 0
    RangeBarSlotWidth,
    /// range bar labels need 0..=6 decimals and a one-line suffix of at most 24 characters
    RangeBarLabelFormat,
    /// a range bar label is one line of at most 160 characters
    RangeBarLabel,
    /// a layer draws either category bars, box plots or range bars
    RangeBarsWithOtherBars,
    /// range bars stand side by side: their bar\_groups mode must be Grouped, not Stacked or PercentStacked
    RangeBarsStacked,
    /// horizontal range bars cannot sit on an overlay layer: its own axis is a value axis on y
    HorizontalRangeBarsOnOverlay,
    /// range bars need a numeric value axis
    RangeBarsValueAxis,
    /// range bars need a categorical or linear numeric position axis: bottom\_x for vertical bars, left\_y for horizontal ones
    RangeBarsPositionAxis,
    /// a range bar connector needs its series alone in its layer
    RangeBarConnectorLayer,
    /// a range bar series on a categorical position axis has no slot\_width; on a numeric one it needs one
    RangeBarSlotWidthAxis,
    /// invalid radar series style
    RadarSeriesStyle,
    /// a radar colour needs channels in 0..=1
    RadarColor,
    /// radar labels need 0..=6 decimals and a one-line suffix of at most 24 characters
    RadarLabelFormat,
    /// radar label text needs one string per category
    RadarLabelTextCount,
    /// a radar label is one line of at most 160 characters
    RadarLabel,
    /// a radar series names its category index column (x\_column) and its value column (y\_column)
    RadarColumns,
    /// a grid vector reads a rectangular lattice: not a hexagonal grid
    HexagonalGridVector,
    /// contours read a rectangular lattice: a hexagonal grid draws as a Heatmap only
    HexagonalContours,
    /// a hexagonal grid is shaded flat
    HexagonalShading,
    /// a hexagonal grid needs linear x and y axes: its lattice is uniform
    HexagonalAxes,
    /// a hexagonal grid's lattice is uniform: not on a Continuous broken axis
    HexagonalBrokenAxis,
    /// an image's channels share one lattice: the same number of columns
    ImageChannelColumns,
    /// an image's channels share one lattice: the same orientation and grid layout
    ImageChannelLattice,
    /// an image's range is finite with a smaller low end
    ImageRange,
    /// an image's opacity is 0..=1
    ImageOpacity,
    /// radar series are drawn on a polar chart (Config.polar)
    RadarNotPolar,
    /// radar series need a categorical angle axis: bottom\_x.categories on the polar chart
    RadarAngleAxis,
    /// radar polygons have straight edges: the polar interpolation must be Chord
    RadarInterpolation,
    /// selected matrix no longer exists
    SelectedMatrixGone,
    /// a parallel axis's range is finite with a smaller min
    ParallelAxisRange,
    /// a logarithmic parallel axis's range is positive
    ParallelLogAxisRange,
    /// a parallel axis is linear or logarithmic: not a probability axis
    ParallelAxisScale,
    /// a parallel axis's label offset is finite
    ParallelAxisLabelOffset,
    /// a parallel chart has at least two axes
    ParallelAxisCount,
    /// a parallel chart's bottom\_x is categorical: one category per axis
    ParallelBottomXCategorical,
    /// a parallel chart's bottom\_x has one category per axis
    ParallelBottomXCategories,
    /// a parallel chart's left\_y spans the normalized band: linear 0..1, not inverted
    ParallelLeftY,
    /// parallel charts have no overlay layers
    ParallelOverlay,
    /// parallel charts have no line offsets
    ParallelLineOffsets,
    /// parallel charts are drawn in the Precise style only
    ParallelDrawStyle,
    /// parallel charts take no subplot axis links
    ParallelSubplotLinks,
    /// a parallel series is drawn on a parallel chart (Config.parallel)
    ParallelSeriesNotParallel,
    /// a parallel series names one column per axis
    ParallelSeriesColumnCount,
    /// a parallel series names one column per axis, at least two
    ParallelSeriesTooFewColumns,
    /// a parallel series' axis columns are named
    ParallelSeriesColumnNames,
    /// a parallel series' x\_column and y\_column are its first two axis columns
    ParallelSeriesXY,
    /// a parallel series' line is linear: no step interpolation
    ParallelSeriesStepLine,
    /// a parallel series' line has no area fill
    ParallelSeriesLineFill,
    /// a parallel series' line is solid
    ParallelSeriesDashed,
    /// a parallel series' colour column is named
    ParallelSeriesColorColumn,
    /// a parallel series coloured by a column needs Config.colorbar
    ParallelSeriesColorbar,
    /// a parallel chart draws only parallel series
    ParallelChartSeriesKind,
    /// ternary axes (bottom\_x, left\_y, right\_y) must be linear
    TernaryAxisScale,
    /// ternary axes cannot be inverted: each increases clockwise
    TernaryAxisInverted,
    /// ternary axes cannot be categorical
    TernaryAxisCategorical,
    /// ternary axes span the whole triangle: every axis has min 0 and the same positive max (1 or 100)
    TernaryAxisSpan,
    /// ternary charts have no overlay layers
    TernaryOverlay,
    /// ternary charts have no line offsets
    TernaryLineOffsets,
    /// ternary charts are drawn in the Precise style only
    TernaryDrawStyle,
    /// ternary charts take no subplot axis links
    TernarySubplotLinks,
    /// a ternary series is drawn on a ternary chart (Config.ternary)
    TernarySeriesNotTernary,
    /// a ternary series names three distinct component columns (x\_column, y\_column, c\_column)
    TernarySeriesColumns,
    /// a ternary series draws markers, a line, a region fill or any of them
    TernaryNothingDrawn,
    /// a ternary region fill's colour and opacity are finite and in 0..=1
    TernaryRegionStyle,
    /// a ternary line is linear: step modes are not drawn
    TernaryStepLine,
    /// a ternary line has no fill
    TernaryLineFill,
    /// a ternary chart draws only ternary series
    TernaryChartSeriesKind,
    /// annotation id must be at most 160 characters without control characters
    AnnotationId,
    /// annotation box border width and padding must be 0..64
    AnnotationBox,
    /// annotation arrow head must be 0..256 logical pixels
    AnnotationArrowHead,
    /// annotation polygon needs at least three vertices
    AnnotationPolygon,
    /// annotation colors must be finite 0..1 RGBA
    AnnotationColor,
    /// annotation line width must be 0..64 logical pixels
    AnnotationLineWidth,
    /// invalid annotation text: size 6..160, at most 512 characters / 8 lines, finite styles
    AnnotationText,
    /// too many annotations
    AnnotationCount,
    /// invalid Feynman line style
    FeynmanLineStyle,
    /// invalid Feynman wave or coil
    FeynmanWave,
    /// invalid Feynman marker, arrow, loop or fan size
    FeynmanSizes,
    /// invalid Feynman picked style: a colour and an extra width in 0..=16
    FeynmanPickedStyle,
    /// Feynman vertex ids must be unique
    FeynmanVertexIds,
    /// Feynman propagator ids must be unique
    FeynmanPropagatorIds,
    /// a Feynman vertex position must be finite
    FeynmanVertexPosition,
    /// invalid Feynman blob marker
    FeynmanBlob,
    /// invalid Feynman vertex label
    FeynmanVertexLabel,
    /// a propagator must name two vertices of its series
    FeynmanPropagatorVertices,
    /// a propagator's bend must be in -170..=170 degrees
    FeynmanBend,
    /// invalid propagator colour or width
    FeynmanPropagatorStyle,
    /// invalid propagator label
    FeynmanPropagatorLabel,
    /// invalid momentum label
    FeynmanMomentumLabel,
    /// a Feynman series reads no columns: x\_column and y\_column stay empty
    FeynmanColumns,
    /// a Feynman series is drawn on a rectangular chart
    FeynmanChart,
    /// a Feynman series is placed on a numeric bottom\_x
    FeynmanNumericX,
    /// a colourbar axis cannot be categorical
    ColorbarCategorical,
    /// the RGBA8 buffer length is not width \* height \* 4
    RgbaBufferLength,
    /// a vector export renders the whole document
    VectorExportWholeDocument,
    /// padding must be finite and non-negative
    FitPadding,
    /// a layer's area lies outside the chart area
    LayerOutsideChart,
    /// the layered document has no feasible layout
    NoFeasibleLayout,
    /// series paint order exceeds the GPU u32 index space
    SeriesPaintOrderOverflow,
    /// histogram bin count exceeds the GPU u32 index space
    HistogramBinCountOverflow,
    /// value label paint order exceeds the GPU u32 index space
    ValueLabelPaintOrderOverflow,
    /// point value maps require GPU-resident columns; streamed sources are not supported
    StreamedPointValueMap,
    /// automatic contour label spacing\_px \* scale must be finite and greater than zero
    ContourLabelSpacingScaled,
    /// a triangle mesh series needs Config.colorbar
    TriMeshColorbar,
    /// grid column offset exceeds a 32-bit lane index
    GridColumnOffsetOverflow,
    /// a field series needs Config.colorbar
    FieldColorbarRequired,
    /// series style was created without its line fill; recreate the style for the current series config
    LineFillStyleMissing,
    /// errorbar style map expects an index column
    ErrorBarStyleMapIndex,
    /// a colour-mapped parallel series needs its value map prepared; recreate the chart's preparation
    ParallelValueMapMissing,
    /// the region's rows exceed the adapter's dispatch grid
    RegionDispatchGrid,
    /// polar charts require GPU-resident columns; streamed sources are not supported
    StreamedPolar,
    /// stream pass does not yet support contours or arc-driven stars
    StreamedContoursOrStars,
    /// categorical axes require GPU-resident columns; streamed sources are not supported
    StreamedCategoricalAxis,
    /// line fill and step interpolation require GPU-resident columns; streamed sources are not supported
    StreamedLineFillOrStep,
    /// 3D chart series require GPU-resident columns; streamed sources are not supported
    Streamed3d,
    /// point colour and size maps require GPU-resident columns; streamed sources are not supported
    StreamedPointMaps,
    /// streamed field requires the bounded tile executor
    StreamedFieldExecutor,
    /// streamed line requires a prepared canonical arc prefix
    StreamedLineArcPrefix,
    /// streamed primitive has no supplied columns
    StreamedPrimitiveColumns,
    /// streamed primitive ranges must describe the same global rows
    StreamedPrimitiveRows,
    /// a colour-mapped scatter needs Config.colorbar: it owns the z range and colormap
    ScatterColorMapColorbar,
    /// a colour-mapped scatter needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw
    ScatterColorMapNeedsColorbar,
    /// a field series needs Config.colorbar: it owns the z range and colormap, so without it there is no value to draw
    FieldNeedsColorbar,
    /// a colour-mapped parallel series needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw
    ParallelColorMapNeedsColorbar,
    /// no series with this id in the chart
    SeriesNotInChart,
    /// a line offset member must be resident: streamed sources are not drawn with line offsets
    LineOffsetMemberStreamed,
    /// auto\_layout names a Feynman series of the chart
    AutoLayoutNotFeynman,
    /// a Feynman label moves through FeynmanSeries::drag\_label, the value-label drag
    FeynmanLabelDrag,
    /// drag\_feynman names a Feynman series of the chart
    DragFeynmanNotFeynman,
    /// a view state must carry one Y range per overlay layer
    ViewStateOverlayRanges,
    /// a view state must carry one view per subplot, each with one Y range per overlay layer
    ViewStateSubplots,
    /// rebased selection needs a global-index style map binding
    RebasedSelectionStyleMap,
    /// scatter style map expects an index column
    ScatterStyleMapIndex,
    /// not a box plot series
    NotBoxPlot,
    /// not a range bar series
    NotRangeBars,
    /// must be nonzero and no larger than the initial operation cap
    StreamOperationCap,
    /// nonresident streaming is not supported for a document with overlay layers
    NonresidentOverlays,
    /// nonresident streaming is not supported for a document with subplots
    NonresidentSubplots,
    /// nonresident streaming is not supported for a document with line offsets
    NonresidentLineOffsets,
    /// limits must be non-zero and chunk bytes must fit the in-flight budget
    StreamLimits,
    /// must be finite and positive
    NotFinitePositive,
    /// must be non-zero and no greater than the execution's initial cap
    StreamExecutionCap,
    /// vector series require GPU-resident columns; streamed sources are not supported
    StreamedVector,
    /// category bar series require GPU-resident columns; streamed sources are not supported
    StreamedCategoryBars,
    /// box plot series require a GPU-resident record, outlier column, violin curves and raw point columns; streamed sources are not supported
    StreamedBoxPlot,
    /// radar series require GPU-resident columns; streamed sources are not supported
    StreamedRadar,
    /// stream draw cursor does not yet support field or contour passes
    StreamDrawCursorField,
    /// a glyph larger than 4096 px (MAX\_GLYPH\_PX) is not rasterized
    GlyphTooLarge,
    /// the chart area must have a non-zero width and height
    ChartAreaEmpty,
    /// a colourbar axis is linear or logarithmic: not a probability axis
    ColorbarProbabilityAxis,
    /// category groups are drawn on a categorical top\_x or bottom\_x
    CategoryGroupsAxis,
    /// breaks go on bottom\_x, left\_y or an overlay's y\_axis
    BreaksAxis,
    /// a broken axis needs a rectangular chart
    BreaksChart,
    /// margins and visual dimensions must produce a non-empty data area
    LayoutDataAreaEmpty,
    /// the overlay layers must share one non-empty data area
    OverlayDataArea,
    /// subplot index out of range
    SubplotIndex,
    /// the Feynman series' working memory could not be allocated
    FeynmanAllocation,
    /// the Feynman layout did not converge
    FeynmanLayoutDidNotConverge,
    /// a record needs at least one field
    RecordWithoutFields,
    /// a record needs at least one row
    RecordWithoutRows,
    /// sunburst has no such ring
    SunburstRing,
    /// radial chart has no detail circle
    RadialNoDetail,
    /// radial data area is too small
    RadialDataAreaTooSmall,
    /// could not allocate label canvas
    RadialLabelCanvas,
    /// radial target must be RGBA8 or BGRA8
    RadialTargetFormat,
    /// a shown radial label has no text; call RadialChart::format\_labels after editing slices, or normalize\_labels on a loaded document
    RadialLabelWithoutText,
    /// radial canvas must be at least 160x160; scale must be at least 0.5
    RadialCanvasSize,
    /// radial output exceeds the device texture limit
    RadialTextureLimit,
    /// radial readback exceeds the device buffer limit
    RadialReadbackLimit,
    /// a triangle mesh series needs Config.colorbar: it owns the z range and colormap, so without it there is no value to draw
    TriMeshNeedsColorbar,
    /// colour-mapped streamlines need Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw
    StreamlineColorMapNeedsColorbar,
    /// vector u and v grids must share orientation and grid layout
    VectorGridLayout,
    /// vector colour grid must share the u grid's orientation and grid layout
    VectorColorGridLayout,
    /// vector colour map must take the components' form: a column for points, a grid for grids
    VectorColorMapForm,
    /// a colour-mapped vector needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw
    VectorColorMapNeedsColorbar,
    /// line fill baseline must be finite
    LineFillBaseline,
    /// line fill cannot reference the series itself
    LineFillSelf,
    /// a nonresident streaming series cannot be scaled by an overlay layer
    NonresidentOverlaySeries,
    /// a nonresident streaming series cannot be drawn by a subplot
    NonresidentSubplotSeries,
    /// parallel column byte offset is not f32-aligned
    ParallelColumnAlignment,
    /// parallel column lane past u32
    ParallelColumnLaneOverflow,
    /// parallel row count past u32
    ParallelRowCountOverflow,
    /// parallel axis count past u32
    ParallelAxisCountOverflow,
    /// not a category bar series
    NotCategoryBars,
    /// the series is not in its bar group
    NotInBarGroup,
    /// the series is not in its box group
    NotInBoxGroup,
    /// a box plot outlier group is too long to draw
    BoxOutlierGroupTooLong,
    /// the box plot series draws no violin
    NoViolin,
    /// the violin curves are too long to draw
    ViolinCurvesTooLong,
    /// the box plot series draws no points
    NoBoxPoints,
    /// the series is not in its range group
    NotInRangeGroup,
    /// region column byte offset is not f32-aligned
    RegionColumnAlignment,
    /// region column lies past the 32-bit lane range
    RegionColumnLaneOverflow,
    /// region rows exceed the 32-bit row range
    RegionRowsOverflow,
    /// column offset exceeds a 32-bit lane index
    ColumnOffsetLaneOverflow,
    /// vector grid cell count exceeds the u32 instance space
    VectorGridCellsOverflow,
    /// a colour-mapped vector needs Config.colorbar: it owns the z range and colormap
    VectorColorMapColorbar,
    /// a ragged column needs at least one value; register none when every group is empty
    RaggedEmpty,
    /// the offsets source must write every offset once, in row order
    RaggedOffsetsOrder,
    /// ragged offsets must be finite non-negative integers
    RaggedOffsetsInteger,
    /// ragged offsets must be non-decreasing: group g is values\[offsets\[g\]..offsets\[g + 1\]\]
    RaggedOffsetsDecreasing,
    /// a triangle index column needs at least one triangle (three indices)
    TrianglesEmpty,
    /// the triangle index source must write every index once, in row order
    TriangleIndicesOrder,
    /// range bar series require GPU-resident columns; streamed sources are not supported
    StreamedRangeBars,
    /// streamline series require GPU-resident columns; streamed sources are not supported
    StreamedStreamlines,
    /// hexagonal grids require GPU-resident columns; streamed sources are not supported
    StreamedHexagonal,
    /// ternary series require GPU-resident columns; streamed sources are not supported
    StreamedTernary,
    /// broken axes require GPU-resident columns; streamed sources are not supported
    StreamedBreak,
    /// probability axes require GPU-resident columns; streamed sources are not supported
    StreamedProbability,
    /// image series require GPU-resident columns; streamed sources are not supported
    StreamedImage,
    /// parallel coordinates series require GPU-resident columns; streamed sources are not supported
    StreamedParallel,
    /// triangle mesh series require GPU-resident columns; streamed sources are not supported
    StreamedTriMesh,
    /// stem series require GPU-resident columns; streamed sources are not supported
    StreamedStem,
    /// energy-level series are DataLess (SSOT only); streamed displays do not draw them
    StreamedEnergyLevels,
    /// Feynman series are DataLess (SSOT only); streamed displays do not draw them
    StreamedFeynman,
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

impl RuleKind {
    /// The variant's name, a stable code a host can branch on without
    /// parsing the sentence (the browser's `code`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::View3dAzimuthNotFinite => "View3dAzimuthNotFinite",
            Self::View3dElevationOutOfRange => "View3dElevationOutOfRange",
            Self::View3dDistanceOutOfRange => "View3dDistanceOutOfRange",
            Self::View3dBoxAspectOutOfRange => "View3dBoxAspectOutOfRange",
            Self::View3dCategoricalAxis => "View3dCategoricalAxis",
            Self::View3dAxisScale => "View3dAxisScale",
            Self::View3dOverlay => "View3dOverlay",
            Self::View3dLineOffsets => "View3dLineOffsets",
            Self::View3dDrawStyle => "View3dDrawStyle",
            Self::View3dStem => "View3dStem",
            Self::View3dPointsWithoutZColumn => "View3dPointsWithoutZColumn",
            Self::View3dStepLine => "View3dStepLine",
            Self::View3dLineFill => "View3dLineFill",
            Self::View3dGridVector => "View3dGridVector",
            Self::View3dVectorWithoutZColumn => "View3dVectorWithoutZColumn",
            Self::View3dVectorWithoutW => "View3dVectorWithoutW",
            Self::View3dImage => "View3dImage",
            Self::View3dParallel => "View3dParallel",
            Self::View3dSeriesKind => "View3dSeriesKind",
            Self::View3dHexagonalGrid => "View3dHexagonalGrid",
            Self::View3dBandedSurface => "View3dBandedSurface",
            Self::View3dContourLabels => "View3dContourLabels",
            Self::SurfaceColorMatrixShape => "SurfaceColorMatrixShape",
            Self::SurfaceColorMatrixColumns => "SurfaceColorMatrixColumns",
            Self::SurfaceWireframeStyle => "SurfaceWireframeStyle",
            Self::SurfaceMaterialStyle => "SurfaceMaterialStyle",
            Self::RadialSliceCount => "RadialSliceCount",
            Self::RadialSliceValue => "RadialSliceValue",
            Self::RadialSliceColor => "RadialSliceColor",
            Self::RadialSliceLabelColor => "RadialSliceLabelColor",
            Self::RadialSliceExplode => "RadialSliceExplode",
            Self::RadialSliceLabel => "RadialSliceLabel",
            Self::RadialSliceLabelText => "RadialSliceLabelText",
            Self::RadialSliceTotal => "RadialSliceTotal",
            Self::RadialStyle => "RadialStyle",
            Self::RadialTitle => "RadialTitle",
            Self::RadialTitleOffsets => "RadialTitleOffsets",
            Self::RadialShape => "RadialShape",
            Self::RadialTextOrBackground => "RadialTextOrBackground",
            Self::RadialLabelFormatOrInteraction => "RadialLabelFormatOrInteraction",
            Self::RadialInteractionTarget => "RadialInteractionTarget",
            Self::RadialEmphasizedZeroSlice => "RadialEmphasizedZeroSlice",
            Self::SunburstDetail => "SunburstDetail",
            Self::RadialSplitIndex => "RadialSplitIndex",
            Self::RadialDetailSum => "RadialDetailSum",
            Self::RadialDetailShape => "RadialDetailShape",
            Self::SunburstRingCount => "SunburstRingCount",
            Self::SunburstNodeCount => "SunburstNodeCount",
            Self::SunburstTilt => "SunburstTilt",
            Self::SunburstChildrenSum => "SunburstChildrenSum",
            Self::EnergyLevelsSourceWithRecords => "EnergyLevelsSourceWithRecords",
            Self::EnergyLevelIds => "EnergyLevelIds",
            Self::IonizationLimitIds => "IonizationLimitIds",
            Self::TransitionIds => "TransitionIds",
            Self::LevelBandIds => "LevelBandIds",
            Self::LevelBandEnds => "LevelBandEnds",
            Self::LevelBandLabelOffset => "LevelBandLabelOffset",
            Self::FineStructureCentroid => "FineStructureCentroid",
            Self::TransitionSameLevel => "TransitionSameLevel",
            Self::TransitionLevels => "TransitionLevels",
            Self::EnergyLevelEnergy => "EnergyLevelEnergy",
            Self::EnergyLevelUncertainty => "EnergyLevelUncertainty",
            Self::EnergyLevelText => "EnergyLevelText",
            Self::ComponentMultipletId => "ComponentMultipletId",
            Self::EnergyLevelLabel => "EnergyLevelLabel",
            Self::IonizationLimitEnergy => "IonizationLimitEnergy",
            Self::IonizationLimitLabel => "IonizationLimitLabel",
            Self::TransitionValues => "TransitionValues",
            Self::TransitionXOffset => "TransitionXOffset",
            Self::TransitionText => "TransitionText",
            Self::TransitionLabel => "TransitionLabel",
            Self::EnergyLevelStroke => "EnergyLevelStroke",
            Self::IonizationLimitLine => "IonizationLimitLine",
            Self::TransitionArrow => "TransitionArrow",
            Self::EnergyLevelsPickedWidth => "EnergyLevelsPickedWidth",
            Self::LevelBandStyle => "LevelBandStyle",
            Self::EnergyLevelsColumns => "EnergyLevelsColumns",
            Self::EnergyLevelsSelfSource => "EnergyLevelsSelfSource",
            Self::EnergyLevelsChart => "EnergyLevelsChart",
            Self::EnergyLevelsCategoricalX => "EnergyLevelsCategoricalX",
            Self::BoxStatsOrder => "BoxStatsOrder",
            Self::BoxStatsMeanOrCount => "BoxStatsMeanOrCount",
            Self::BoxStatsMedianCi => "BoxStatsMedianCi",
            Self::BoxStatsOutliers => "BoxStatsOutliers",
            Self::BoxPlotChartLimits => "BoxPlotChartLimits",
            Self::BoxPlotChartCategory => "BoxPlotChartCategory",
            Self::BoxPlotChartSeries => "BoxPlotChartSeries",
            Self::BoxPlotChartStyle => "BoxPlotChartStyle",
            Self::BoxPlotChartTitle => "BoxPlotChartTitle",
            Self::BoxPlotChartValueRange => "BoxPlotChartValueRange",
            Self::BoxPlotChartOverride => "BoxPlotChartOverride",
            Self::BoxPlotChartUnresolvedStyle => "BoxPlotChartUnresolvedStyle",
            Self::BoxPlotChartNotchWithoutCi => "BoxPlotChartNotchWithoutCi",
            Self::BoxPlotChartInteractionTarget => "BoxPlotChartInteractionTarget",
            Self::PolarAngles => "PolarAngles",
            Self::PolarInnerRadius => "PolarInnerRadius",
            Self::PolarSector => "PolarSector",
            Self::PolarSectorTurn => "PolarSectorTurn",
            Self::PolarArcPieces => "PolarArcPieces",
            Self::PolarAngleAxisScale => "PolarAngleAxisScale",
            Self::PolarAngleAxisInverted => "PolarAngleAxisInverted",
            Self::PolarPolygonGridAxis => "PolarPolygonGridAxis",
            Self::PolarRadiusCategorical => "PolarRadiusCategorical",
            Self::PolarRadiusScale => "PolarRadiusScale",
            Self::PolarOverlay => "PolarOverlay",
            Self::PolarLineOffsets => "PolarLineOffsets",
            Self::DataAreaTooSmall => "DataAreaTooSmall",
            Self::DecorationOffsetCount => "DecorationOffsetCount",
            Self::DecorationOffsets => "DecorationOffsets",
            Self::DecorationAxisMove => "DecorationAxisMove",
            Self::RadarChartLimits => "RadarChartLimits",
            Self::RadarChartSeriesStyle => "RadarChartSeriesStyle",
            Self::RadarChartStyle => "RadarChartStyle",
            Self::RadarChartValueRange => "RadarChartValueRange",
            Self::RadarChartRingSpacing => "RadarChartRingSpacing",
            Self::RadarChartInteractionTarget => "RadarChartInteractionTarget",
            Self::AxisCrossAt => "AxisCrossAt",
            Self::CategoryWindow => "CategoryWindow",
            Self::AxisBoundsNotFinite => "AxisBoundsNotFinite",
            Self::AxisBoundsOrder => "AxisBoundsOrder",
            Self::LogAxisBounds => "LogAxisBounds",
            Self::ProbabilityAxisBounds => "ProbabilityAxisBounds",
            Self::SeriesIdEmpty => "SeriesIdEmpty",
            Self::SeriesIdDuplicate => "SeriesIdDuplicate",
            Self::PolarErrorBars => "PolarErrorBars",
            Self::PolarStems => "PolarStems",
            Self::PolarHistogram => "PolarHistogram",
            Self::PolarHexagonalGrid => "PolarHexagonalGrid",
            Self::PolarHeatmapPolygonGrid => "PolarHeatmapPolygonGrid",
            Self::PolarImage => "PolarImage",
            Self::PolarVector => "PolarVector",
            Self::PolarBoxPlot => "PolarBoxPlot",
            Self::PolarRangeBars => "PolarRangeBars",
            Self::PolarStreamlines => "PolarStreamlines",
            Self::PolarParallel => "PolarParallel",
            Self::PolarEnergyLevels => "PolarEnergyLevels",
            Self::PolarTriMesh => "PolarTriMesh",
            Self::PolarFeynman => "PolarFeynman",
            Self::PolarTernary => "PolarTernary",
            Self::PolarArcStepLine => "PolarArcStepLine",
            Self::CategoricalChartBarMaterial => "CategoricalChartBarMaterial",
            Self::CategoricalChartLimits => "CategoricalChartLimits",
            Self::CategoricalChartCategory => "CategoricalChartCategory",
            Self::CategoricalChartSeries => "CategoricalChartSeries",
            Self::CategoricalChartValues => "CategoricalChartValues",
            Self::CategoricalChartStyle => "CategoricalChartStyle",
            Self::CategoricalChartTitles => "CategoricalChartTitles",
            Self::CategoricalChartValueSuffix => "CategoricalChartValueSuffix",
            Self::CategoricalChartOverride => "CategoricalChartOverride",
            Self::CategoricalChartInteractionTarget => "CategoricalChartInteractionTarget",
            Self::CategoricalChartStackOverflow => "CategoricalChartStackOverflow",
            Self::BoundedTitle => "BoundedTitle",
            Self::TriMeshTriangles => "TriMeshTriangles",
            Self::TriMeshValues => "TriMeshValues",
            Self::TriMeshNothingDrawn => "TriMeshNothingDrawn",
            Self::TriMeshSurfaceWithoutFill => "TriMeshSurfaceWithoutFill",
            Self::TriMeshVertexValuesFlat => "TriMeshVertexValuesFlat",
            Self::TriMeshFaceValuesInterpolated => "TriMeshFaceValuesInterpolated",
            Self::TriMeshContourFaceValues => "TriMeshContourFaceValues",
            Self::TriMeshContourLabels => "TriMeshContourLabels",
            Self::TriMeshSurfaceColorMatrix => "TriMeshSurfaceColorMatrix",
            Self::TriMeshVertexColumns => "TriMeshVertexColumns",
            Self::TriMeshTernary => "TriMeshTernary",
            Self::TriMeshParallel => "TriMeshParallel",
            Self::TriMeshWithoutZColumn => "TriMeshWithoutZColumn",
            Self::SubplotNested => "SubplotNested",
            Self::SubplotAnnotations => "SubplotAnnotations",
            Self::SubplotLink => "SubplotLink",
            Self::SeriesInManyLayers => "SeriesInManyLayers",
            Self::SeriesInManyOverlays => "SeriesInManyOverlays",
            Self::CategoryGroupCategories => "CategoryGroupCategories",
            Self::CategoryGroupOrder => "CategoryGroupOrder",
            Self::CategoryGroupOverlap => "CategoryGroupOverlap",
            Self::CategoryGroupLabel => "CategoryGroupLabel",
            Self::CategoryGroupOffset => "CategoryGroupOffset",
            Self::BreakCategorical => "BreakCategorical",
            Self::BreakCount => "BreakCount",
            Self::BreakRange => "BreakRange",
            Self::BreakLogRange => "BreakLogRange",
            Self::BreakProbabilityRange => "BreakProbabilityRange",
            Self::BreakOrder => "BreakOrder",
            Self::BreakGap => "BreakGap",
            Self::BreakSizing => "BreakSizing",
            Self::BreakWeightCount => "BreakWeightCount",
            Self::BreakWeights => "BreakWeights",
            Self::BreakMark => "BreakMark",
            Self::CategoriesEmpty => "CategoriesEmpty",
            Self::CategoryIds => "CategoryIds",
            Self::CategoryLabels => "CategoryLabels",
            Self::CategoryRichLabel => "CategoryRichLabel",
            Self::CategoricalLogAxis => "CategoricalLogAxis",
            Self::CategoricalProbabilityAxis => "CategoricalProbabilityAxis",
            Self::StemBaseline => "StemBaseline",
            Self::StemWidth => "StemWidth",
            Self::XyNothingDrawn => "XyNothingDrawn",
            Self::ErrorBarsWithoutMarkers => "ErrorBarsWithoutMarkers",
            Self::ErrorBarsDirection => "ErrorBarsDirection",
            Self::StemsWithoutMarkers => "StemsWithoutMarkers",
            Self::FieldNothingDrawn => "FieldNothingDrawn",
            Self::FieldSurfaceWithoutFill => "FieldSurfaceWithoutFill",
            Self::XyWireNothingDrawn => "XyWireNothingDrawn",
            Self::StreamlineArrows => "StreamlineArrows",
            Self::StreamlineInterpolation => "StreamlineInterpolation",
            Self::StreamlineFill => "StreamlineFill",
            Self::StreamlineColumns => "StreamlineColumns",
            Self::StreamlineColorMapColumn => "StreamlineColorMapColumn",
            Self::VectorPixels => "VectorPixels",
            Self::VectorScale => "VectorScale",
            Self::PointSizeMapDomain => "PointSizeMapDomain",
            Self::PointSizeMapPixels => "PointSizeMapPixels",
            Self::ContourLabelSpacing => "ContourLabelSpacing",
            Self::ContourLabelAnchors => "ContourLabelAnchors",
            Self::ValueLabelOffsetCount => "ValueLabelOffsetCount",
            Self::ValueLabelOffset => "ValueLabelOffset",
            Self::BarStyle => "BarStyle",
            Self::BarGroups => "BarGroups",
            Self::LineOffsetSteps => "LineOffsetSteps",
            Self::LineOffsetMember => "LineOffsetMember",
            Self::LineOffsetMemberLayer => "LineOffsetMemberLayer",
            Self::LineOffsetMemberUnknown => "LineOffsetMemberUnknown",
            Self::LineOffsetMemberNotLine => "LineOffsetMemberNotLine",
            Self::LineOffsetFill => "LineOffsetFill",
            Self::BarColor => "BarColor",
            Self::BarLabelFormat => "BarLabelFormat",
            Self::BarLabelTextCount => "BarLabelTextCount",
            Self::BarLabel => "BarLabel",
            Self::BarOverride => "BarOverride",
            Self::HorizontalBarsOnOverlay => "HorizontalBarsOnOverlay",
            Self::RoseDirection => "RoseDirection",
            Self::CategoryBarsAxis => "CategoryBarsAxis",
            Self::CategoryBarsValueAxis => "CategoryBarsValueAxis",
            Self::CategoryBarsValueColumns => "CategoryBarsValueColumns",
            Self::BoxMaterial => "BoxMaterial",
            Self::BoxStyle => "BoxStyle",
            Self::BoxColor => "BoxColor",
            Self::BoxLabelFormat => "BoxLabelFormat",
            Self::BoxLabelTextCount => "BoxLabelTextCount",
            Self::BoxLabel => "BoxLabel",
            Self::BoxPlotColumns => "BoxPlotColumns",
            Self::ViolinBoxWidth => "ViolinBoxWidth",
            Self::ViolinHalves => "ViolinHalves",
            Self::ViolinHalfColumns => "ViolinHalfColumns",
            Self::ViolinDensityScale => "ViolinDensityScale",
            Self::ViolinOutline => "ViolinOutline",
            Self::BoxPointsColumns => "BoxPointsColumns",
            Self::BoxPointsStyle => "BoxPointsStyle",
            Self::BoxOverride => "BoxOverride",
            Self::BarsAndBoxesInLayer => "BarsAndBoxesInLayer",
            Self::BoxPlotsStacked => "BoxPlotsStacked",
            Self::HorizontalBoxPlotsOnOverlay => "HorizontalBoxPlotsOnOverlay",
            Self::BoxPlotsAxis => "BoxPlotsAxis",
            Self::BoxPlotsValueAxis => "BoxPlotsValueAxis",
            Self::RangeBarColor => "RangeBarColor",
            Self::RangeBarColumns => "RangeBarColumns",
            Self::RangeBarColumnsDistinct => "RangeBarColumnsDistinct",
            Self::RangeBarWhiskers => "RangeBarWhiskers",
            Self::RangeBarConnector => "RangeBarConnector",
            Self::RangeBarStateColors => "RangeBarStateColors",
            Self::RangeBarRemainingOpacity => "RangeBarRemainingOpacity",
            Self::RangeBarSlotWidth => "RangeBarSlotWidth",
            Self::RangeBarLabelFormat => "RangeBarLabelFormat",
            Self::RangeBarLabel => "RangeBarLabel",
            Self::RangeBarsWithOtherBars => "RangeBarsWithOtherBars",
            Self::RangeBarsStacked => "RangeBarsStacked",
            Self::HorizontalRangeBarsOnOverlay => "HorizontalRangeBarsOnOverlay",
            Self::RangeBarsValueAxis => "RangeBarsValueAxis",
            Self::RangeBarsPositionAxis => "RangeBarsPositionAxis",
            Self::RangeBarConnectorLayer => "RangeBarConnectorLayer",
            Self::RangeBarSlotWidthAxis => "RangeBarSlotWidthAxis",
            Self::RadarSeriesStyle => "RadarSeriesStyle",
            Self::RadarColor => "RadarColor",
            Self::RadarLabelFormat => "RadarLabelFormat",
            Self::RadarLabelTextCount => "RadarLabelTextCount",
            Self::RadarLabel => "RadarLabel",
            Self::RadarColumns => "RadarColumns",
            Self::HexagonalGridVector => "HexagonalGridVector",
            Self::HexagonalContours => "HexagonalContours",
            Self::HexagonalShading => "HexagonalShading",
            Self::HexagonalAxes => "HexagonalAxes",
            Self::HexagonalBrokenAxis => "HexagonalBrokenAxis",
            Self::ImageChannelColumns => "ImageChannelColumns",
            Self::ImageChannelLattice => "ImageChannelLattice",
            Self::ImageRange => "ImageRange",
            Self::ImageOpacity => "ImageOpacity",
            Self::RadarNotPolar => "RadarNotPolar",
            Self::RadarAngleAxis => "RadarAngleAxis",
            Self::RadarInterpolation => "RadarInterpolation",
            Self::SelectedMatrixGone => "SelectedMatrixGone",
            Self::ParallelAxisRange => "ParallelAxisRange",
            Self::ParallelLogAxisRange => "ParallelLogAxisRange",
            Self::ParallelAxisScale => "ParallelAxisScale",
            Self::ParallelAxisLabelOffset => "ParallelAxisLabelOffset",
            Self::ParallelAxisCount => "ParallelAxisCount",
            Self::ParallelBottomXCategorical => "ParallelBottomXCategorical",
            Self::ParallelBottomXCategories => "ParallelBottomXCategories",
            Self::ParallelLeftY => "ParallelLeftY",
            Self::ParallelOverlay => "ParallelOverlay",
            Self::ParallelLineOffsets => "ParallelLineOffsets",
            Self::ParallelDrawStyle => "ParallelDrawStyle",
            Self::ParallelSubplotLinks => "ParallelSubplotLinks",
            Self::ParallelSeriesNotParallel => "ParallelSeriesNotParallel",
            Self::ParallelSeriesColumnCount => "ParallelSeriesColumnCount",
            Self::ParallelSeriesTooFewColumns => "ParallelSeriesTooFewColumns",
            Self::ParallelSeriesColumnNames => "ParallelSeriesColumnNames",
            Self::ParallelSeriesXY => "ParallelSeriesXY",
            Self::ParallelSeriesStepLine => "ParallelSeriesStepLine",
            Self::ParallelSeriesLineFill => "ParallelSeriesLineFill",
            Self::ParallelSeriesDashed => "ParallelSeriesDashed",
            Self::ParallelSeriesColorColumn => "ParallelSeriesColorColumn",
            Self::ParallelSeriesColorbar => "ParallelSeriesColorbar",
            Self::ParallelChartSeriesKind => "ParallelChartSeriesKind",
            Self::TernaryAxisScale => "TernaryAxisScale",
            Self::TernaryAxisInverted => "TernaryAxisInverted",
            Self::TernaryAxisCategorical => "TernaryAxisCategorical",
            Self::TernaryAxisSpan => "TernaryAxisSpan",
            Self::TernaryOverlay => "TernaryOverlay",
            Self::TernaryLineOffsets => "TernaryLineOffsets",
            Self::TernaryDrawStyle => "TernaryDrawStyle",
            Self::TernarySubplotLinks => "TernarySubplotLinks",
            Self::TernarySeriesNotTernary => "TernarySeriesNotTernary",
            Self::TernarySeriesColumns => "TernarySeriesColumns",
            Self::TernaryNothingDrawn => "TernaryNothingDrawn",
            Self::TernaryRegionStyle => "TernaryRegionStyle",
            Self::TernaryStepLine => "TernaryStepLine",
            Self::TernaryLineFill => "TernaryLineFill",
            Self::TernaryChartSeriesKind => "TernaryChartSeriesKind",
            Self::AnnotationId => "AnnotationId",
            Self::AnnotationBox => "AnnotationBox",
            Self::AnnotationArrowHead => "AnnotationArrowHead",
            Self::AnnotationPolygon => "AnnotationPolygon",
            Self::AnnotationColor => "AnnotationColor",
            Self::AnnotationLineWidth => "AnnotationLineWidth",
            Self::AnnotationText => "AnnotationText",
            Self::AnnotationCount => "AnnotationCount",
            Self::FeynmanLineStyle => "FeynmanLineStyle",
            Self::FeynmanWave => "FeynmanWave",
            Self::FeynmanSizes => "FeynmanSizes",
            Self::FeynmanPickedStyle => "FeynmanPickedStyle",
            Self::FeynmanVertexIds => "FeynmanVertexIds",
            Self::FeynmanPropagatorIds => "FeynmanPropagatorIds",
            Self::FeynmanVertexPosition => "FeynmanVertexPosition",
            Self::FeynmanBlob => "FeynmanBlob",
            Self::FeynmanVertexLabel => "FeynmanVertexLabel",
            Self::FeynmanPropagatorVertices => "FeynmanPropagatorVertices",
            Self::FeynmanBend => "FeynmanBend",
            Self::FeynmanPropagatorStyle => "FeynmanPropagatorStyle",
            Self::FeynmanPropagatorLabel => "FeynmanPropagatorLabel",
            Self::FeynmanMomentumLabel => "FeynmanMomentumLabel",
            Self::FeynmanColumns => "FeynmanColumns",
            Self::FeynmanChart => "FeynmanChart",
            Self::FeynmanNumericX => "FeynmanNumericX",
            Self::ColorbarCategorical => "ColorbarCategorical",
            Self::RgbaBufferLength => "RgbaBufferLength",
            Self::VectorExportWholeDocument => "VectorExportWholeDocument",
            Self::FitPadding => "FitPadding",
            Self::LayerOutsideChart => "LayerOutsideChart",
            Self::NoFeasibleLayout => "NoFeasibleLayout",
            Self::SeriesPaintOrderOverflow => "SeriesPaintOrderOverflow",
            Self::HistogramBinCountOverflow => "HistogramBinCountOverflow",
            Self::ValueLabelPaintOrderOverflow => "ValueLabelPaintOrderOverflow",
            Self::StreamedPointValueMap => "StreamedPointValueMap",
            Self::ContourLabelSpacingScaled => "ContourLabelSpacingScaled",
            Self::TriMeshColorbar => "TriMeshColorbar",
            Self::GridColumnOffsetOverflow => "GridColumnOffsetOverflow",
            Self::FieldColorbarRequired => "FieldColorbarRequired",
            Self::LineFillStyleMissing => "LineFillStyleMissing",
            Self::ErrorBarStyleMapIndex => "ErrorBarStyleMapIndex",
            Self::ParallelValueMapMissing => "ParallelValueMapMissing",
            Self::RegionDispatchGrid => "RegionDispatchGrid",
            Self::StreamedPolar => "StreamedPolar",
            Self::StreamedContoursOrStars => "StreamedContoursOrStars",
            Self::StreamedCategoricalAxis => "StreamedCategoricalAxis",
            Self::StreamedLineFillOrStep => "StreamedLineFillOrStep",
            Self::Streamed3d => "Streamed3d",
            Self::StreamedPointMaps => "StreamedPointMaps",
            Self::StreamedFieldExecutor => "StreamedFieldExecutor",
            Self::StreamedLineArcPrefix => "StreamedLineArcPrefix",
            Self::StreamedPrimitiveColumns => "StreamedPrimitiveColumns",
            Self::StreamedPrimitiveRows => "StreamedPrimitiveRows",
            Self::ScatterColorMapColorbar => "ScatterColorMapColorbar",
            Self::ScatterColorMapNeedsColorbar => "ScatterColorMapNeedsColorbar",
            Self::FieldNeedsColorbar => "FieldNeedsColorbar",
            Self::ParallelColorMapNeedsColorbar => "ParallelColorMapNeedsColorbar",
            Self::SeriesNotInChart => "SeriesNotInChart",
            Self::LineOffsetMemberStreamed => "LineOffsetMemberStreamed",
            Self::AutoLayoutNotFeynman => "AutoLayoutNotFeynman",
            Self::FeynmanLabelDrag => "FeynmanLabelDrag",
            Self::DragFeynmanNotFeynman => "DragFeynmanNotFeynman",
            Self::ViewStateOverlayRanges => "ViewStateOverlayRanges",
            Self::ViewStateSubplots => "ViewStateSubplots",
            Self::RebasedSelectionStyleMap => "RebasedSelectionStyleMap",
            Self::ScatterStyleMapIndex => "ScatterStyleMapIndex",
            Self::NotBoxPlot => "NotBoxPlot",
            Self::NotRangeBars => "NotRangeBars",
            Self::StreamOperationCap => "StreamOperationCap",
            Self::NonresidentOverlays => "NonresidentOverlays",
            Self::NonresidentSubplots => "NonresidentSubplots",
            Self::NonresidentLineOffsets => "NonresidentLineOffsets",
            Self::StreamLimits => "StreamLimits",
            Self::NotFinitePositive => "NotFinitePositive",
            Self::StreamExecutionCap => "StreamExecutionCap",
            Self::StreamedVector => "StreamedVector",
            Self::StreamedCategoryBars => "StreamedCategoryBars",
            Self::StreamedBoxPlot => "StreamedBoxPlot",
            Self::StreamedRadar => "StreamedRadar",
            Self::StreamDrawCursorField => "StreamDrawCursorField",
            Self::GlyphTooLarge => "GlyphTooLarge",
            Self::ChartAreaEmpty => "ChartAreaEmpty",
            Self::ColorbarProbabilityAxis => "ColorbarProbabilityAxis",
            Self::CategoryGroupsAxis => "CategoryGroupsAxis",
            Self::BreaksAxis => "BreaksAxis",
            Self::BreaksChart => "BreaksChart",
            Self::LayoutDataAreaEmpty => "LayoutDataAreaEmpty",
            Self::OverlayDataArea => "OverlayDataArea",
            Self::SubplotIndex => "SubplotIndex",
            Self::FeynmanAllocation => "FeynmanAllocation",
            Self::FeynmanLayoutDidNotConverge => "FeynmanLayoutDidNotConverge",
            Self::RecordWithoutFields => "RecordWithoutFields",
            Self::RecordWithoutRows => "RecordWithoutRows",
            Self::SunburstRing => "SunburstRing",
            Self::RadialNoDetail => "RadialNoDetail",
            Self::RadialDataAreaTooSmall => "RadialDataAreaTooSmall",
            Self::RadialLabelCanvas => "RadialLabelCanvas",
            Self::RadialTargetFormat => "RadialTargetFormat",
            Self::RadialLabelWithoutText => "RadialLabelWithoutText",
            Self::RadialCanvasSize => "RadialCanvasSize",
            Self::RadialTextureLimit => "RadialTextureLimit",
            Self::RadialReadbackLimit => "RadialReadbackLimit",
            Self::TriMeshNeedsColorbar => "TriMeshNeedsColorbar",
            Self::StreamlineColorMapNeedsColorbar => "StreamlineColorMapNeedsColorbar",
            Self::VectorGridLayout => "VectorGridLayout",
            Self::VectorColorGridLayout => "VectorColorGridLayout",
            Self::VectorColorMapForm => "VectorColorMapForm",
            Self::VectorColorMapNeedsColorbar => "VectorColorMapNeedsColorbar",
            Self::LineFillBaseline => "LineFillBaseline",
            Self::LineFillSelf => "LineFillSelf",
            Self::NonresidentOverlaySeries => "NonresidentOverlaySeries",
            Self::NonresidentSubplotSeries => "NonresidentSubplotSeries",
            Self::ParallelColumnAlignment => "ParallelColumnAlignment",
            Self::ParallelColumnLaneOverflow => "ParallelColumnLaneOverflow",
            Self::ParallelRowCountOverflow => "ParallelRowCountOverflow",
            Self::ParallelAxisCountOverflow => "ParallelAxisCountOverflow",
            Self::NotCategoryBars => "NotCategoryBars",
            Self::NotInBarGroup => "NotInBarGroup",
            Self::NotInBoxGroup => "NotInBoxGroup",
            Self::BoxOutlierGroupTooLong => "BoxOutlierGroupTooLong",
            Self::NoViolin => "NoViolin",
            Self::ViolinCurvesTooLong => "ViolinCurvesTooLong",
            Self::NoBoxPoints => "NoBoxPoints",
            Self::NotInRangeGroup => "NotInRangeGroup",
            Self::RegionColumnAlignment => "RegionColumnAlignment",
            Self::RegionColumnLaneOverflow => "RegionColumnLaneOverflow",
            Self::RegionRowsOverflow => "RegionRowsOverflow",
            Self::ColumnOffsetLaneOverflow => "ColumnOffsetLaneOverflow",
            Self::VectorGridCellsOverflow => "VectorGridCellsOverflow",
            Self::VectorColorMapColorbar => "VectorColorMapColorbar",
            Self::RaggedEmpty => "RaggedEmpty",
            Self::RaggedOffsetsOrder => "RaggedOffsetsOrder",
            Self::RaggedOffsetsInteger => "RaggedOffsetsInteger",
            Self::RaggedOffsetsDecreasing => "RaggedOffsetsDecreasing",
            Self::TrianglesEmpty => "TrianglesEmpty",
            Self::TriangleIndicesOrder => "TriangleIndicesOrder",
            Self::StreamedRangeBars => "StreamedRangeBars",
            Self::StreamedStreamlines => "StreamedStreamlines",
            Self::StreamedHexagonal => "StreamedHexagonal",
            Self::StreamedTernary => "StreamedTernary",
            Self::StreamedBreak => "StreamedBreak",
            Self::StreamedProbability => "StreamedProbability",
            Self::StreamedImage => "StreamedImage",
            Self::StreamedParallel => "StreamedParallel",
            Self::StreamedTriMesh => "StreamedTriMesh",
            Self::StreamedStem => "StreamedStem",
            Self::StreamedEnergyLevels => "StreamedEnergyLevels",
            Self::StreamedFeynman => "StreamedFeynman",
            Self::PackedPickLane { .. } => "PackedPickLane",
            Self::PickChunk { .. } => "PickChunk",
            Self::PackedSelectionLane { .. } => "PackedSelectionLane",
            Self::StreamStyleIndexShort { .. } => "StreamStyleIndexShort",
            Self::BoxPlotRecordNotResident { .. } => "BoxPlotRecordNotResident",
            Self::BoxPlotRaggedNotResident { .. } => "BoxPlotRaggedNotResident",
            Self::BoxPlotNotResident { .. } => "BoxPlotNotResident",
            Self::StreamedCompositeId { .. } => "StreamedCompositeId",
            Self::CompositeNeedsResident { .. } => "CompositeNeedsResident",
            Self::RadarValueCount { .. } => "RadarValueCount",
            Self::CategoryBarColumnLength { .. } => "CategoryBarColumnLength",
            Self::PercentStackedNegative { .. } => "PercentStackedNegative",
            Self::RangeBarColumnLength { .. } => "RangeBarColumnLength",
            Self::PointMapColumnShort { .. } => "PointMapColumnShort",
            Self::ScatterStyleIndexShort { .. } => "ScatterStyleIndexShort",
            Self::ErrorBarStyleIndexShort { .. } => "ErrorBarStyleIndexShort",
            Self::HexagonalCoordinatesNotUniform { .. } => "HexagonalCoordinatesNotUniform",
            Self::InvalidLabel { .. } => "InvalidLabel",
            Self::ValueLabels { .. } => "ValueLabels",
            Self::ContourLabelAtlas { .. } => "ContourLabelAtlas",
            Self::LineFillTargetMissing { .. } => "LineFillTargetMissing",
            Self::LineFillXColumn { .. } => "LineFillXColumn",
            Self::ContourLevelCount { .. } => "ContourLevelCount",
            Self::InvalidMath { .. } => "InvalidMath",
            Self::StreamedColumnSpan { .. } => "StreamedColumnSpan",
            Self::IdInUse { .. } => "IdInUse",
            Self::CompositeIdEmpty { .. } => "CompositeIdEmpty",
            Self::CompositeIdSeparator { .. } => "CompositeIdSeparator",
            Self::CompositeIdIsColumn { .. } => "CompositeIdIsColumn",
            Self::RecordSchemaFields { .. } => "RecordSchemaFields",
            Self::RecordSourceFields { .. } => "RecordSourceFields",
            Self::RecordUnknownField { .. } => "RecordUnknownField",
            Self::RecordFieldTwice { .. } => "RecordFieldTwice",
            Self::RecordFieldMissing { .. } => "RecordFieldMissing",
            Self::RecordFieldLength { .. } => "RecordFieldLength",
            Self::RaggedOffsetCount { .. } => "RaggedOffsetCount",
            Self::RaggedOffsetsStart { .. } => "RaggedOffsetsStart",
            Self::RaggedLastOffset { .. } => "RaggedLastOffset",
            Self::TriangleIndexCount { .. } => "TriangleIndexCount",
            Self::TriangleIndexRange { .. } => "TriangleIndexRange",
            Self::RecordRowsOverflow { .. } => "RecordRowsOverflow",
            Self::RaggedValuesOverflow { .. } => "RaggedValuesOverflow",
            Self::RaggedGroupsOverflow { .. } => "RaggedGroupsOverflow",
            Self::TriangleIndicesOverflow { .. } => "TriangleIndicesOverflow",
            Self::NotBoxPlotRecord { .. } => "NotBoxPlotRecord",
            Self::BoxPlotRecordRows { .. } => "BoxPlotRecordRows",
            Self::BoxPlotOutliersNotResident { .. } => "BoxPlotOutliersNotResident",
            Self::BoxPlotOutlierGroups { .. } => "BoxPlotOutlierGroups",
            Self::BoxPlotPositions { .. } => "BoxPlotPositions",
            Self::ViolinCurveNotResident { .. } => "ViolinCurveNotResident",
            Self::ViolinCurveGroups { .. } => "ViolinCurveGroups",
            Self::ViolinDensityLength { .. } => "ViolinDensityLength",
            Self::TrianglesNotResident { .. } => "TrianglesNotResident",
            Self::TriangleIndexPastVertices { .. } => "TriangleIndexPastVertices",
            Self::FaceValueColumnShort { .. } => "FaceValueColumnShort",
        }
    }
}

impl std::fmt::Display for RuleKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sentence = match self {
            Self::View3dAzimuthNotFinite => "view_3d.azimuth_degrees must be finite",
            Self::View3dElevationOutOfRange => "view_3d.elevation_degrees must be in -90..=90",
            Self::View3dDistanceOutOfRange => "view_3d.distance must be in 1.2..=100",
            Self::View3dBoxAspectOutOfRange => "view_3d.box_aspect lengths must be in 0.05..=20",
            Self::View3dCategoricalAxis => {
                "3D chart axes (bottom_x, left_y, right_y) cannot be categorical"
            }
            Self::View3dAxisScale => "3D chart axes must be linear or logarithmic",
            Self::View3dOverlay => "3D charts have no overlay layers",
            Self::View3dLineOffsets => "3D charts have no line offsets",
            Self::View3dDrawStyle => "3D charts are drawn in the Precise style only",
            Self::View3dStem => "stems draw flat: not on a 3D chart",
            Self::View3dPointsWithoutZColumn => {
                "a scatter or line series on a 3D chart names its height column (z_column)"
            }
            Self::View3dStepLine => "a line on a 3D chart is linear: no step interpolation",
            Self::View3dLineFill => "a line on a 3D chart has no area fill",
            Self::View3dGridVector => "a grid vector draws flat: not on a 3D chart",
            Self::View3dVectorWithoutZColumn => {
                "a vector series on a 3D chart names its anchor height column (z_column)"
            }
            Self::View3dVectorWithoutW => "a vector series on a 3D chart names its z component (w)",
            Self::View3dImage => "an image draws flat: not on a 3D chart",
            Self::View3dParallel => "a parallel series draws flat: not on a 3D chart",
            Self::View3dSeriesKind => {
                "a 3D chart draws only heatmaps, contours, heatmap contours, and the scatter, line and point vector series that name a z_column"
            }
            Self::View3dHexagonalGrid => "a hexagonal grid draws flat: not on a 3D chart",
            Self::View3dBandedSurface => {
                "a surface's fill is continuous: banded fills are not drawn in 3D"
            }
            Self::View3dContourLabels => "contours on a 3D chart's floor have no labels",
            Self::SurfaceColorMatrixShape => {
                "a surface's color_matrix has the column count, orientation and layout of its matrix"
            }
            Self::SurfaceColorMatrixColumns => "a surface's color_matrix names its columns",
            Self::SurfaceWireframeStyle => {
                "a surface wireframe has a width in 0..=24 and a colour in 0..=1"
            }
            Self::SurfaceMaterialStyle => {
                "a surface's gloss, texture_strength and emphasis_brightness are in 0..=1 and its texture_scale is positive"
            }
            Self::RadialSliceCount => "a radial chart needs 1..64 slices",
            Self::RadialSliceValue => "slice values must be finite and nonnegative",
            Self::RadialSliceColor => "slice colors must be finite opaque RGBA in 0..1",
            Self::RadialSliceLabelColor => "invalid slice label color",
            Self::RadialSliceExplode => "slice explode must be in 0..0.35",
            Self::RadialSliceLabel => "slice labels must be single-line, at most 80 characters",
            Self::RadialSliceLabelText => {
                "slice label text must have at most two lines of at most 400 characters, without control characters"
            }
            Self::RadialSliceTotal => "slice total must be positive and finite",
            Self::RadialStyle => "invalid radial material, light, or projection",
            Self::RadialTitle => {
                "invalid radial title: size 6..160, at most 512 characters / 8 lines, finite styles"
            }
            Self::RadialTitleOffsets => {
                "a radial chart title keeps its position in decorations: its offsets and top_margin must be 0"
            }
            Self::RadialShape => "invalid radial shape or start angle",
            Self::RadialTextOrBackground => "invalid radial text or background",
            Self::RadialLabelFormatOrInteraction => "invalid radial label format or interaction",
            Self::RadialInteractionTarget => "radial interaction target is out of range",
            Self::RadialEmphasizedZeroSlice => "cannot emphasize a zero-valued slice",
            Self::SunburstDetail => "a sunburst has no detail circle",
            Self::RadialSplitIndex => "split slice index is out of range",
            Self::RadialDetailSum => "detail values must sum to the parent slice value",
            Self::RadialDetailShape => "invalid detail chart shape",
            Self::SunburstRingCount => "a sunburst has at most 8 rings",
            Self::SunburstNodeCount => "a sunburst ring has at most 64 nodes",
            Self::SunburstTilt => "a sunburst draws flat: no tilt or depth",
            Self::SunburstChildrenSum => "a node's children sum to at most its value",
            Self::EnergyLevelsSourceWithRecords => {
                "an energy-level series with a source holds no records of its own"
            }
            Self::EnergyLevelIds => "energy level ids must be non-empty and unique",
            Self::IonizationLimitIds => "ionization limit ids must be non-empty and unique",
            Self::TransitionIds => "transition ids must be non-empty and unique",
            Self::LevelBandIds => "level band ids must be non-empty and unique",
            Self::LevelBandEnds => "a level band's first and last name levels of its column",
            Self::LevelBandLabelOffset => "a level band's label offset is out of range",
            Self::FineStructureCentroid => "a fine-structure component must name a centroid level",
            Self::TransitionSameLevel => "a transition joins two different levels",
            Self::TransitionLevels => "a transition must name two levels of its series",
            Self::EnergyLevelEnergy => "an energy level's energy must be finite",
            Self::EnergyLevelUncertainty => {
                "an energy level's uncertainty and g must be finite and not negative"
            }
            Self::EnergyLevelText => {
                "an energy level's column must be set and its text fields one line of at most 160 characters"
            }
            Self::ComponentMultipletId => {
                "a component's multiplet id is one line of at most 160 characters"
            }
            Self::EnergyLevelLabel => "invalid energy level label",
            Self::IonizationLimitEnergy => {
                "an ionization limit's energy must be finite and its uncertainty not negative"
            }
            Self::IonizationLimitLabel => "invalid ionization limit label or reference",
            Self::TransitionValues => {
                "a transition's values must be finite and its uncertainties not negative"
            }
            Self::TransitionXOffset => "a transition's x_offset must be in -0.5..=0.5 of the slot",
            Self::TransitionText => {
                "a transition's text fields are one line of at most 160 characters"
            }
            Self::TransitionLabel => "invalid transition label",
            Self::EnergyLevelStroke => "invalid energy level stroke style",
            Self::IonizationLimitLine => "invalid ionization limit line style",
            Self::TransitionArrow => "invalid transition arrow style",
            Self::EnergyLevelsPickedWidth => "picked_width_scale must be in 1..=4",
            Self::LevelBandStyle => "invalid level band style",
            Self::EnergyLevelsColumns => {
                "an energy-level series reads no columns: x_column and y_column stay empty"
            }
            Self::EnergyLevelsSelfSource => "an energy-level series cannot be its own source",
            Self::EnergyLevelsChart => "an energy-level series is drawn on a rectangular chart",
            Self::EnergyLevelsCategoricalX => {
                "an energy-level series needs a categorical bottom_x: its term columns"
            }
            Self::BoxStatsOrder => {
                "box statistics must be finite and low <= Q1 <= median <= Q3 <= high"
            }
            Self::BoxStatsMeanOrCount => {
                "mean must be finite; sample count must be positive when supplied"
            }
            Self::BoxStatsMedianCi => "median CI must be finite and contain the median",
            Self::BoxStatsOutliers => "outliers must be finite and strictly outside the whiskers",
            Self::BoxPlotChartLimits => "boxplot limits: at least 1 category and 1 series",
            Self::BoxPlotChartCategory => "invalid category ID/label",
            Self::BoxPlotChartSeries => "invalid series ID/label, color or value count",
            Self::BoxPlotChartStyle => "invalid boxplot layout, font or colors",
            Self::BoxPlotChartTitle => "invalid title",
            Self::BoxPlotChartValueRange => "invalid linear value range",
            Self::BoxPlotChartOverride => "invalid or duplicate box override",
            Self::BoxPlotChartUnresolvedStyle => "internal: a box with a value resolved no style",
            Self::BoxPlotChartNotchWithoutCi => "notched boxes require an explicit median CI",
            Self::BoxPlotChartInteractionTarget => "unknown or missing interaction target",
            Self::PolarAngles => "polar angles must be finite",
            Self::PolarInnerRadius => "polar inner_radius_fraction must be within 0..=0.9",
            Self::PolarSector => "polar sector must be a finite [start, end] with start < end",
            Self::PolarSectorTurn => "polar sector cannot exceed one turn",
            Self::PolarArcPieces => "polar arc pieces must be within 1..=64",
            Self::PolarAngleAxisScale => "the polar angle axis (bottom_x) must be linear",
            Self::PolarAngleAxisInverted => {
                "the polar angle axis (bottom_x) cannot be inverted; use clockwise"
            }
            Self::PolarPolygonGridAxis => {
                "a polygon polar grid joins its rings at the categories: it needs a categorical angle axis (bottom_x.categories)"
            }
            Self::PolarRadiusCategorical => "the polar radius axis (left_y) cannot be categorical",
            Self::PolarRadiusScale => "the polar radius axis (left_y) is linear or logarithmic",
            Self::PolarOverlay => "polar charts have no overlay layers",
            Self::PolarLineOffsets => "polar charts have no line offsets",
            Self::DataAreaTooSmall => {
                "data area must fit the measured plot region and be at least 80 by 60 logical pixels"
            }
            Self::DecorationOffsetCount => "too many decoration offsets",
            Self::DecorationOffsets => "decoration offsets must be finite and unique",
            Self::DecorationAxisMove => "axes only move perpendicular to their direction",
            Self::RadarChartLimits => "radar limits: at least 3 categories and 1 series",
            Self::RadarChartSeriesStyle => "invalid radar style",
            Self::RadarChartStyle => "invalid radar layout, font or colors",
            Self::RadarChartValueRange => "invalid value range",
            Self::RadarChartRingSpacing => "ring spacing must be finite and positive",
            Self::RadarChartInteractionTarget => "unknown interaction target",
            Self::AxisCrossAt => "an axis crossing (`cross_at`) must be finite",
            Self::CategoryWindow => "a category window needs finite bounds with from < to",
            Self::AxisBoundsNotFinite => "axis bounds must be finite",
            Self::AxisBoundsOrder => "axis max must be greater than min",
            Self::LogAxisBounds => "logarithmic axis bounds must be positive",
            Self::ProbabilityAxisBounds => {
                "probability axis bounds must lie strictly between 0 and 1"
            }
            Self::SeriesIdEmpty => "series_id must not be empty",
            Self::SeriesIdDuplicate => "series_id must be unique within a chart",
            Self::PolarErrorBars => "error bars are not drawn on polar axes",
            Self::PolarStems => "stems are not drawn on polar axes",
            Self::PolarHistogram => "histogram series are not drawn on polar axes",
            Self::PolarHexagonalGrid => "a hexagonal grid is drawn on a rectangular chart",
            Self::PolarHeatmapPolygonGrid => {
                "a polar heatmap's cells follow circular rings: not on a polygon grid"
            }
            Self::PolarImage => "images are not drawn on polar axes",
            Self::PolarVector => "vector series are not drawn on polar axes",
            Self::PolarBoxPlot => "box plots are not drawn on polar axes",
            Self::PolarRangeBars => "range bars are not drawn on polar axes",
            Self::PolarStreamlines => "streamlines are not drawn on polar axes",
            Self::PolarParallel => "parallel series are drawn on parallel charts, not polar axes",
            Self::PolarEnergyLevels => "energy-level series are not drawn on polar axes",
            Self::PolarTriMesh => "a triangle mesh is not drawn on polar axes",
            Self::PolarFeynman => "Feynman series are not drawn on polar axes",
            Self::PolarTernary => "ternary series are drawn on ternary charts, not polar axes",
            Self::PolarArcStepLine => {
                "a polar arc interpolation needs a linearly interpolated line; a step interpolation draws chords"
            }
            Self::CategoricalChartBarMaterial => "material not supported by bars",
            Self::CategoricalChartLimits => "categorical limits: at least 1 category and 1 series",
            Self::CategoricalChartCategory => "invalid or duplicate category ID/label",
            Self::CategoricalChartSeries => "invalid series, value count, color or style",
            Self::CategoricalChartValues => {
                "values must be finite; percent stacks require nonnegative values"
            }
            Self::CategoricalChartStyle => "invalid categorical style or layout",
            Self::CategoricalChartTitles => "titles must be single-line, at most 160 characters",
            Self::CategoricalChartValueSuffix => "invalid value suffix",
            Self::CategoricalChartOverride => "invalid or duplicate bar override",
            Self::CategoricalChartInteractionTarget => "interaction target is missing or unknown",
            Self::CategoricalChartStackOverflow => "stack total overflow",
            Self::BoundedTitle => {
                "invalid bounded title: size 6..160, at most 512 characters / 8 lines, finite styles"
            }
            Self::TriMeshTriangles => "a triangle mesh names its triangle index column",
            Self::TriMeshValues => "a triangle mesh names its value column",
            Self::TriMeshNothingDrawn => "a triangle mesh draws a fill, contour lines or both",
            Self::TriMeshSurfaceWithoutFill => "a surface look needs the triangle mesh's fill",
            Self::TriMeshVertexValuesFlat => {
                "vertex values are shaded Interpolated: a flat triangle colour from vertex values is host work (give face values)"
            }
            Self::TriMeshFaceValuesInterpolated => {
                "face values are shaded Flat: interpolating them needs vertex values, which are host work"
            }
            Self::TriMeshContourFaceValues => {
                "contour lines need vertex values: a field constant on each triangle has no isolines"
            }
            Self::TriMeshContourLabels => "contour lines on a triangle mesh have no labels",
            Self::TriMeshSurfaceColorMatrix => {
                "a triangle mesh is coloured by its values: no surface color_matrix"
            }
            Self::TriMeshVertexColumns => {
                "a triangle mesh names its vertex coordinate columns (x_column, y_column)"
            }
            Self::TriMeshTernary => "a triangle mesh is not drawn on a ternary chart",
            Self::TriMeshParallel => "a triangle mesh is not drawn on a parallel chart",
            Self::TriMeshWithoutZColumn => {
                "a triangle mesh on a 3D chart names its vertex height column (z_column)"
            }
            Self::SubplotNested => "a subplot cannot have subplots of its own",
            Self::SubplotAnnotations => {
                "the document's annotation layer belongs to the parent, not a subplot"
            }
            Self::SubplotLink => "a subplot link must point at the parent or an earlier subplot",
            Self::SeriesInManyLayers => {
                "a series id may belong to at most one subplot or overlay layer"
            }
            Self::SeriesInManyOverlays => "a series id may belong to at most one overlay layer",
            Self::CategoryGroupCategories => "a category group names categories of its axis",
            Self::CategoryGroupOrder => "a category group's first category comes before its last",
            Self::CategoryGroupOverlap => "category groups run in axis order without overlapping",
            Self::CategoryGroupLabel => "invalid category group label",
            Self::CategoryGroupOffset => "invalid category group offset",
            Self::BreakCategorical => "a categorical axis cannot be broken",
            Self::BreakCount => "an axis can cut out at most three ranges",
            Self::BreakRange => "a cut range needs finite bounds with from < to",
            Self::BreakLogRange => "a cut range on a logarithmic axis must be positive",
            Self::BreakProbabilityRange => {
                "a cut range on a probability axis must lie strictly between 0 and 1"
            }
            Self::BreakOrder => "cut ranges must be sorted and must not touch",
            Self::BreakGap => "a break gap must be between 0 and 200 px",
            Self::BreakSizing => {
                "only a Continuous broken axis sizes its pieces other than by value"
            }
            Self::BreakWeightCount => {
                "piece weights need one weight per piece: one more than cut ranges"
            }
            Self::BreakWeights => "piece weights must be finite and positive",
            Self::BreakMark => {
                "a break mark needs a size in 0..=64, a width in 0..=16 and a colour in range"
            }
            Self::CategoriesEmpty => "a categorical axis needs at least one category",
            Self::CategoryIds => {
                "category ids must be unique, non-empty, single-line and at most 80 characters"
            }
            Self::CategoryLabels => "category labels must be single-line and at most 80 characters",
            Self::CategoryRichLabel => "invalid category rich label",
            Self::CategoricalLogAxis => "a categorical axis cannot be logarithmic",
            Self::CategoricalProbabilityAxis => "a categorical axis cannot be a probability axis",
            Self::StemBaseline => "a stem baseline must be finite",
            Self::StemWidth => "a stem width must be finite and at least 0",
            Self::XyNothingDrawn => "an xy series draws markers, a line or both",
            Self::ErrorBarsWithoutMarkers => "error bars are drawn with markers",
            Self::ErrorBarsDirection => "error bars run along x, y or both",
            Self::StemsWithoutMarkers => "stems are drawn with markers",
            Self::FieldNothingDrawn => "a field draws a fill, contour lines or both",
            Self::FieldSurfaceWithoutFill => "a surface look needs the field's fill",
            Self::XyWireNothingDrawn => "an xy series draws markers or a line",
            Self::StreamlineArrows => {
                "streamline arrows need a finite spacing_px > 0, finite non-negative head_length_px and head_width_px, and spacing_px >= head_length_px"
            }
            Self::StreamlineInterpolation => {
                "streamlines are polylines: the line interpolation must be Linear"
            }
            Self::StreamlineFill => "streamlines have no fill",
            Self::StreamlineColumns => "a streamline series names its x_column and y_column",
            Self::StreamlineColorMapColumn => "a streamline colour map names its column",
            Self::VectorPixels => {
                "vector width_px, head_length_px, head_width_px and min_length_px must be finite and non-negative"
            }
            Self::VectorScale => "vector scale must be finite and non-zero",
            Self::PointSizeMapDomain => {
                "point size map domain must be finite with domain_min < domain_max"
            }
            Self::PointSizeMapPixels => {
                "point size map min_px and max_px must be finite, non-negative and min_px <= max_px"
            }
            Self::ContourLabelSpacing => {
                "contour label spacing_px must be finite and greater than zero"
            }
            Self::ContourLabelAnchors => {
                "contour label anchors exceed the supported maximum of 1024"
            }
            Self::ValueLabelOffsetCount => "value label offsets need one entry per category",
            Self::ValueLabelOffset => {
                "a value label offset is finite and at most 4096 px along either axis"
            }
            Self::BarStyle => "invalid bar style",
            Self::BarGroups => {
                "bar groups need a group width in 0.1..=0.95 and a bar gap in 0..=24 px"
            }
            Self::LineOffsetSteps => "line offset steps must be finite",
            Self::LineOffsetMember => "a line offset member is a non-empty series id, listed once",
            Self::LineOffsetMemberLayer => {
                "a line offset member must be drawn by the chart that lists it"
            }
            Self::LineOffsetMemberUnknown => "a line offset member names no series of the chart",
            Self::LineOffsetMemberNotLine => {
                "a line offset member must draw a line: Line, ScatterLine or a LineScatterErrorbar kind"
            }
            Self::LineOffsetFill => {
                "a ToSeries fill cannot join series with different line offsets"
            }
            Self::BarColor => "a bar colour must be opaque with channels in 0..=1",
            Self::BarLabelFormat => {
                "bar labels need 0..=6 decimals and a one-line suffix of at most 24 characters"
            }
            Self::BarLabelTextCount => "bar label text needs one string per category",
            Self::BarLabel => "a bar label is one line of at most 160 characters",
            Self::BarOverride => {
                "a bar override names a category of the axis and its own series, once"
            }
            Self::HorizontalBarsOnOverlay => {
                "horizontal category bars cannot sit on an overlay layer: its own axis is a value axis on y"
            }
            Self::RoseDirection => {
                "a rose fans its bars around the centre: the group direction on a polar chart is vertical"
            }
            Self::CategoryBarsAxis => {
                "category bars need a categorical axis: bottom_x for vertical bars, left_y for horizontal ones"
            }
            Self::CategoryBarsValueAxis => "category bars need a linear numeric value axis",
            Self::CategoryBarsValueColumns => {
                "the category bar series of a group need distinct value columns"
            }
            Self::BoxMaterial => "material not supported by boxes",
            Self::BoxStyle => "invalid boxplot style",
            Self::BoxColor => "a box colour must be opaque with channels in 0..=1",
            Self::BoxLabelFormat => {
                "box labels need 0..=6 decimals and a one-line suffix of at most 24 characters"
            }
            Self::BoxLabelTextCount => "box label text needs one string per category",
            Self::BoxLabel => "a box label is one line of at most 160 characters",
            Self::BoxPlotColumns => {
                "a box plot series names its category positions (x_column), its record (y_column) unless it draws a violin, and, when set, its outlier column"
            }
            Self::ViolinBoxWidth => "a violin's box width is a share of the slot in 0..=1",
            Self::ViolinHalves => "a split violin draws at least one half",
            Self::ViolinHalfColumns => {
                "a violin half names a ragged curve and a distinct density column"
            }
            Self::ViolinDensityScale => "a violin half's density_scale is finite and positive",
            Self::ViolinOutline => {
                "a violin outline is 0..=8 px wide with channels in 0..=1, its fill opaque"
            }
            Self::BoxPointsColumns => "box points name distinct position and value columns",
            Self::BoxPointsStyle => {
                "box points are 1..=24 px, jittered by 0..=1 and offset by -1..=1 of the half slot, with colour channels in 0..=1"
            }
            Self::BoxOverride => {
                "a box override names a category of the axis and its own series, once"
            }
            Self::BarsAndBoxesInLayer => {
                "a layer draws either category bars or box plots, not both"
            }
            Self::BoxPlotsStacked => {
                "box plots stand side by side: their bar_groups mode must be Grouped, not Stacked or PercentStacked"
            }
            Self::HorizontalBoxPlotsOnOverlay => {
                "horizontal box plots cannot sit on an overlay layer: its own axis is a value axis on y"
            }
            Self::BoxPlotsAxis => {
                "box plots need a categorical axis: bottom_x for vertical boxes, left_y for horizontal ones"
            }
            Self::BoxPlotsValueAxis => "box plots need a linear numeric value axis",
            Self::RangeBarColor => "a range bar colour must be opaque with channels in 0..=1",
            Self::RangeBarColumns => {
                "a range bar series names its positions (x_column), its end values (y_column), its start values and every optional column"
            }
            Self::RangeBarColumnsDistinct => "the columns of a range bar series are distinct",
            Self::RangeBarWhiskers => "range bar whiskers are 0..=24 px wide, in an opaque colour",
            Self::RangeBarConnector => {
                "a range bar connector is 0..=24 px wide, its colour channels in 0..=1"
            }
            Self::RangeBarStateColors => "range bar direction and state colours must be opaque",
            Self::RangeBarRemainingOpacity => "a range bar's remaining opacity is in 0..=1",
            Self::RangeBarSlotWidth => "a range bar slot width is finite and greater than 0",
            Self::RangeBarLabelFormat => {
                "range bar labels need 0..=6 decimals and a one-line suffix of at most 24 characters"
            }
            Self::RangeBarLabel => "a range bar label is one line of at most 160 characters",
            Self::RangeBarsWithOtherBars => {
                "a layer draws either category bars, box plots or range bars"
            }
            Self::RangeBarsStacked => {
                "range bars stand side by side: their bar_groups mode must be Grouped, not Stacked or PercentStacked"
            }
            Self::HorizontalRangeBarsOnOverlay => {
                "horizontal range bars cannot sit on an overlay layer: its own axis is a value axis on y"
            }
            Self::RangeBarsValueAxis => "range bars need a numeric value axis",
            Self::RangeBarsPositionAxis => {
                "range bars need a categorical or linear numeric position axis: bottom_x for vertical bars, left_y for horizontal ones"
            }
            Self::RangeBarConnectorLayer => {
                "a range bar connector needs its series alone in its layer"
            }
            Self::RangeBarSlotWidthAxis => {
                "a range bar series on a categorical position axis has no slot_width; on a numeric one it needs one"
            }
            Self::RadarSeriesStyle => "invalid radar series style",
            Self::RadarColor => "a radar colour needs channels in 0..=1",
            Self::RadarLabelFormat => {
                "radar labels need 0..=6 decimals and a one-line suffix of at most 24 characters"
            }
            Self::RadarLabelTextCount => "radar label text needs one string per category",
            Self::RadarLabel => "a radar label is one line of at most 160 characters",
            Self::RadarColumns => {
                "a radar series names its category index column (x_column) and its value column (y_column)"
            }
            Self::HexagonalGridVector => {
                "a grid vector reads a rectangular lattice: not a hexagonal grid"
            }
            Self::HexagonalContours => {
                "contours read a rectangular lattice: a hexagonal grid draws as a Heatmap only"
            }
            Self::HexagonalShading => "a hexagonal grid is shaded flat",
            Self::HexagonalAxes => {
                "a hexagonal grid needs linear x and y axes: its lattice is uniform"
            }
            Self::HexagonalBrokenAxis => {
                "a hexagonal grid's lattice is uniform: not on a Continuous broken axis"
            }
            Self::ImageChannelColumns => {
                "an image's channels share one lattice: the same number of columns"
            }
            Self::ImageChannelLattice => {
                "an image's channels share one lattice: the same orientation and grid layout"
            }
            Self::ImageRange => "an image's range is finite with a smaller low end",
            Self::ImageOpacity => "an image's opacity is 0..=1",
            Self::RadarNotPolar => "radar series are drawn on a polar chart (Config.polar)",
            Self::RadarAngleAxis => {
                "radar series need a categorical angle axis: bottom_x.categories on the polar chart"
            }
            Self::RadarInterpolation => {
                "radar polygons have straight edges: the polar interpolation must be Chord"
            }
            Self::SelectedMatrixGone => "selected matrix no longer exists",
            Self::ParallelAxisRange => "a parallel axis's range is finite with a smaller min",
            Self::ParallelLogAxisRange => "a logarithmic parallel axis's range is positive",
            Self::ParallelAxisScale => {
                "a parallel axis is linear or logarithmic: not a probability axis"
            }
            Self::ParallelAxisLabelOffset => "a parallel axis's label offset is finite",
            Self::ParallelAxisCount => "a parallel chart has at least two axes",
            Self::ParallelBottomXCategorical => {
                "a parallel chart's bottom_x is categorical: one category per axis"
            }
            Self::ParallelBottomXCategories => {
                "a parallel chart's bottom_x has one category per axis"
            }
            Self::ParallelLeftY => {
                "a parallel chart's left_y spans the normalized band: linear 0..1, not inverted"
            }
            Self::ParallelOverlay => "parallel charts have no overlay layers",
            Self::ParallelLineOffsets => "parallel charts have no line offsets",
            Self::ParallelDrawStyle => "parallel charts are drawn in the Precise style only",
            Self::ParallelSubplotLinks => "parallel charts take no subplot axis links",
            Self::ParallelSeriesNotParallel => {
                "a parallel series is drawn on a parallel chart (Config.parallel)"
            }
            Self::ParallelSeriesColumnCount => "a parallel series names one column per axis",
            Self::ParallelSeriesTooFewColumns => {
                "a parallel series names one column per axis, at least two"
            }
            Self::ParallelSeriesColumnNames => "a parallel series' axis columns are named",
            Self::ParallelSeriesXY => {
                "a parallel series' x_column and y_column are its first two axis columns"
            }
            Self::ParallelSeriesStepLine => {
                "a parallel series' line is linear: no step interpolation"
            }
            Self::ParallelSeriesLineFill => "a parallel series' line has no area fill",
            Self::ParallelSeriesDashed => "a parallel series' line is solid",
            Self::ParallelSeriesColorColumn => "a parallel series' colour column is named",
            Self::ParallelSeriesColorbar => {
                "a parallel series coloured by a column needs Config.colorbar"
            }
            Self::ParallelChartSeriesKind => "a parallel chart draws only parallel series",
            Self::TernaryAxisScale => "ternary axes (bottom_x, left_y, right_y) must be linear",
            Self::TernaryAxisInverted => {
                "ternary axes cannot be inverted: each increases clockwise"
            }
            Self::TernaryAxisCategorical => "ternary axes cannot be categorical",
            Self::TernaryAxisSpan => {
                "ternary axes span the whole triangle: every axis has min 0 and the same positive max (1 or 100)"
            }
            Self::TernaryOverlay => "ternary charts have no overlay layers",
            Self::TernaryLineOffsets => "ternary charts have no line offsets",
            Self::TernaryDrawStyle => "ternary charts are drawn in the Precise style only",
            Self::TernarySubplotLinks => "ternary charts take no subplot axis links",
            Self::TernarySeriesNotTernary => {
                "a ternary series is drawn on a ternary chart (Config.ternary)"
            }
            Self::TernarySeriesColumns => {
                "a ternary series names three distinct component columns (x_column, y_column, c_column)"
            }
            Self::TernaryNothingDrawn => {
                "a ternary series draws markers, a line, a region fill or any of them"
            }
            Self::TernaryRegionStyle => {
                "a ternary region fill's colour and opacity are finite and in 0..=1"
            }
            Self::TernaryStepLine => "a ternary line is linear: step modes are not drawn",
            Self::TernaryLineFill => "a ternary line has no fill",
            Self::TernaryChartSeriesKind => "a ternary chart draws only ternary series",
            Self::AnnotationId => {
                "annotation id must be at most 160 characters without control characters"
            }
            Self::AnnotationBox => "annotation box border width and padding must be 0..64",
            Self::AnnotationArrowHead => "annotation arrow head must be 0..256 logical pixels",
            Self::AnnotationPolygon => "annotation polygon needs at least three vertices",
            Self::AnnotationColor => "annotation colors must be finite 0..1 RGBA",
            Self::AnnotationLineWidth => "annotation line width must be 0..64 logical pixels",
            Self::AnnotationText => {
                "invalid annotation text: size 6..160, at most 512 characters / 8 lines, finite styles"
            }
            Self::AnnotationCount => "too many annotations",
            Self::FeynmanLineStyle => "invalid Feynman line style",
            Self::FeynmanWave => "invalid Feynman wave or coil",
            Self::FeynmanSizes => "invalid Feynman marker, arrow, loop or fan size",
            Self::FeynmanPickedStyle => {
                "invalid Feynman picked style: a colour and an extra width in 0..=16"
            }
            Self::FeynmanVertexIds => "Feynman vertex ids must be unique",
            Self::FeynmanPropagatorIds => "Feynman propagator ids must be unique",
            Self::FeynmanVertexPosition => "a Feynman vertex position must be finite",
            Self::FeynmanBlob => "invalid Feynman blob marker",
            Self::FeynmanVertexLabel => "invalid Feynman vertex label",
            Self::FeynmanPropagatorVertices => "a propagator must name two vertices of its series",
            Self::FeynmanBend => "a propagator's bend must be in -170..=170 degrees",
            Self::FeynmanPropagatorStyle => "invalid propagator colour or width",
            Self::FeynmanPropagatorLabel => "invalid propagator label",
            Self::FeynmanMomentumLabel => "invalid momentum label",
            Self::FeynmanColumns => {
                "a Feynman series reads no columns: x_column and y_column stay empty"
            }
            Self::FeynmanChart => "a Feynman series is drawn on a rectangular chart",
            Self::FeynmanNumericX => "a Feynman series is placed on a numeric bottom_x",
            Self::ColorbarCategorical => "a colourbar axis cannot be categorical",
            Self::RgbaBufferLength => "the RGBA8 buffer length is not width * height * 4",
            Self::VectorExportWholeDocument => "a vector export renders the whole document",
            Self::FitPadding => "padding must be finite and non-negative",
            Self::LayerOutsideChart => "a layer's area lies outside the chart area",
            Self::NoFeasibleLayout => "the layered document has no feasible layout",
            Self::SeriesPaintOrderOverflow => "series paint order exceeds the GPU u32 index space",
            Self::HistogramBinCountOverflow => {
                "histogram bin count exceeds the GPU u32 index space"
            }
            Self::ValueLabelPaintOrderOverflow => {
                "value label paint order exceeds the GPU u32 index space"
            }
            Self::StreamedPointValueMap => {
                "point value maps require GPU-resident columns; streamed sources are not supported"
            }
            Self::ContourLabelSpacingScaled => {
                "automatic contour label spacing_px * scale must be finite and greater than zero"
            }
            Self::TriMeshColorbar => "a triangle mesh series needs Config.colorbar",
            Self::GridColumnOffsetOverflow => "grid column offset exceeds a 32-bit lane index",
            Self::FieldColorbarRequired => "a field series needs Config.colorbar",
            Self::LineFillStyleMissing => {
                "series style was created without its line fill; recreate the style for the current series config"
            }
            Self::ErrorBarStyleMapIndex => "errorbar style map expects an index column",
            Self::ParallelValueMapMissing => {
                "a colour-mapped parallel series needs its value map prepared; recreate the chart's preparation"
            }
            Self::RegionDispatchGrid => "the region's rows exceed the adapter's dispatch grid",
            Self::StreamedPolar => {
                "polar charts require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedContoursOrStars => {
                "stream pass does not yet support contours or arc-driven stars"
            }
            Self::StreamedCategoricalAxis => {
                "categorical axes require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedLineFillOrStep => {
                "line fill and step interpolation require GPU-resident columns; streamed sources are not supported"
            }
            Self::Streamed3d => {
                "3D chart series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedPointMaps => {
                "point colour and size maps require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedFieldExecutor => "streamed field requires the bounded tile executor",
            Self::StreamedLineArcPrefix => "streamed line requires a prepared canonical arc prefix",
            Self::StreamedPrimitiveColumns => "streamed primitive has no supplied columns",
            Self::StreamedPrimitiveRows => {
                "streamed primitive ranges must describe the same global rows"
            }
            Self::ScatterColorMapColorbar => {
                "a colour-mapped scatter needs Config.colorbar: it owns the z range and colormap"
            }
            Self::ScatterColorMapNeedsColorbar => {
                "a colour-mapped scatter needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw"
            }
            Self::FieldNeedsColorbar => {
                "a field series needs Config.colorbar: it owns the z range and colormap, so without it there is no value to draw"
            }
            Self::ParallelColorMapNeedsColorbar => {
                "a colour-mapped parallel series needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw"
            }
            Self::SeriesNotInChart => "no series with this id in the chart",
            Self::LineOffsetMemberStreamed => {
                "a line offset member must be resident: streamed sources are not drawn with line offsets"
            }
            Self::AutoLayoutNotFeynman => "auto_layout names a Feynman series of the chart",
            Self::FeynmanLabelDrag => {
                "a Feynman label moves through FeynmanSeries::drag_label, the value-label drag"
            }
            Self::DragFeynmanNotFeynman => "drag_feynman names a Feynman series of the chart",
            Self::ViewStateOverlayRanges => "a view state must carry one Y range per overlay layer",
            Self::ViewStateSubplots => {
                "a view state must carry one view per subplot, each with one Y range per overlay layer"
            }
            Self::RebasedSelectionStyleMap => {
                "rebased selection needs a global-index style map binding"
            }
            Self::ScatterStyleMapIndex => "scatter style map expects an index column",
            Self::NotBoxPlot => "not a box plot series",
            Self::NotRangeBars => "not a range bar series",
            Self::StreamOperationCap => {
                "must be nonzero and no larger than the initial operation cap"
            }
            Self::NonresidentOverlays => {
                "nonresident streaming is not supported for a document with overlay layers"
            }
            Self::NonresidentSubplots => {
                "nonresident streaming is not supported for a document with subplots"
            }
            Self::NonresidentLineOffsets => {
                "nonresident streaming is not supported for a document with line offsets"
            }
            Self::StreamLimits => {
                "limits must be non-zero and chunk bytes must fit the in-flight budget"
            }
            Self::NotFinitePositive => "must be finite and positive",
            Self::StreamExecutionCap => {
                "must be non-zero and no greater than the execution's initial cap"
            }
            Self::StreamedVector => {
                "vector series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedCategoryBars => {
                "category bar series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedBoxPlot => {
                "box plot series require a GPU-resident record, outlier column, violin curves and raw point columns; streamed sources are not supported"
            }
            Self::StreamedRadar => {
                "radar series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamDrawCursorField => {
                "stream draw cursor does not yet support field or contour passes"
            }
            Self::GlyphTooLarge => "a glyph larger than 4096 px (MAX_GLYPH_PX) is not rasterized",
            Self::ChartAreaEmpty => "the chart area must have a non-zero width and height",
            Self::ColorbarProbabilityAxis => {
                "a colourbar axis is linear or logarithmic: not a probability axis"
            }
            Self::CategoryGroupsAxis => {
                "category groups are drawn on a categorical top_x or bottom_x"
            }
            Self::BreaksAxis => "breaks go on bottom_x, left_y or an overlay's y_axis",
            Self::BreaksChart => "a broken axis needs a rectangular chart",
            Self::LayoutDataAreaEmpty => {
                "margins and visual dimensions must produce a non-empty data area"
            }
            Self::OverlayDataArea => "the overlay layers must share one non-empty data area",
            Self::SubplotIndex => "subplot index out of range",
            Self::FeynmanAllocation => "the Feynman series' working memory could not be allocated",
            Self::FeynmanLayoutDidNotConverge => "the Feynman layout did not converge",
            Self::RecordWithoutFields => "a record needs at least one field",
            Self::RecordWithoutRows => "a record needs at least one row",
            Self::SunburstRing => "sunburst has no such ring",
            Self::RadialNoDetail => "radial chart has no detail circle",
            Self::RadialDataAreaTooSmall => "radial data area is too small",
            Self::RadialLabelCanvas => "could not allocate label canvas",
            Self::RadialTargetFormat => "radial target must be RGBA8 or BGRA8",
            Self::RadialLabelWithoutText => {
                "a shown radial label has no text; call RadialChart::format_labels after editing slices, or normalize_labels on a loaded document"
            }
            Self::RadialCanvasSize => {
                "radial canvas must be at least 160x160; scale must be at least 0.5"
            }
            Self::RadialTextureLimit => "radial output exceeds the device texture limit",
            Self::RadialReadbackLimit => "radial readback exceeds the device buffer limit",
            Self::TriMeshNeedsColorbar => {
                "a triangle mesh series needs Config.colorbar: it owns the z range and colormap, so without it there is no value to draw"
            }
            Self::StreamlineColorMapNeedsColorbar => {
                "colour-mapped streamlines need Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw"
            }
            Self::VectorGridLayout => "vector u and v grids must share orientation and grid layout",
            Self::VectorColorGridLayout => {
                "vector colour grid must share the u grid's orientation and grid layout"
            }
            Self::VectorColorMapForm => {
                "vector colour map must take the components' form: a column for points, a grid for grids"
            }
            Self::VectorColorMapNeedsColorbar => {
                "a colour-mapped vector needs Config.colorbar: it owns the z range and colormap, so without it there is no colour to draw"
            }
            Self::LineFillBaseline => "line fill baseline must be finite",
            Self::LineFillSelf => "line fill cannot reference the series itself",
            Self::NonresidentOverlaySeries => {
                "a nonresident streaming series cannot be scaled by an overlay layer"
            }
            Self::NonresidentSubplotSeries => {
                "a nonresident streaming series cannot be drawn by a subplot"
            }
            Self::ParallelColumnAlignment => "parallel column byte offset is not f32-aligned",
            Self::ParallelColumnLaneOverflow => "parallel column lane past u32",
            Self::ParallelRowCountOverflow => "parallel row count past u32",
            Self::ParallelAxisCountOverflow => "parallel axis count past u32",
            Self::NotCategoryBars => "not a category bar series",
            Self::NotInBarGroup => "the series is not in its bar group",
            Self::NotInBoxGroup => "the series is not in its box group",
            Self::BoxOutlierGroupTooLong => "a box plot outlier group is too long to draw",
            Self::NoViolin => "the box plot series draws no violin",
            Self::ViolinCurvesTooLong => "the violin curves are too long to draw",
            Self::NoBoxPoints => "the box plot series draws no points",
            Self::NotInRangeGroup => "the series is not in its range group",
            Self::RegionColumnAlignment => "region column byte offset is not f32-aligned",
            Self::RegionColumnLaneOverflow => "region column lies past the 32-bit lane range",
            Self::RegionRowsOverflow => "region rows exceed the 32-bit row range",
            Self::ColumnOffsetLaneOverflow => "column offset exceeds a 32-bit lane index",
            Self::VectorGridCellsOverflow => {
                "vector grid cell count exceeds the u32 instance space"
            }
            Self::VectorColorMapColorbar => {
                "a colour-mapped vector needs Config.colorbar: it owns the z range and colormap"
            }
            Self::RaggedEmpty => {
                "a ragged column needs at least one value; register none when every group is empty"
            }
            Self::RaggedOffsetsOrder => {
                "the offsets source must write every offset once, in row order"
            }
            Self::RaggedOffsetsInteger => "ragged offsets must be finite non-negative integers",
            Self::RaggedOffsetsDecreasing => {
                "ragged offsets must be non-decreasing: group g is values[offsets[g]..offsets[g + 1]]"
            }
            Self::TrianglesEmpty => {
                "a triangle index column needs at least one triangle (three indices)"
            }
            Self::TriangleIndicesOrder => {
                "the triangle index source must write every index once, in row order"
            }
            Self::StreamedRangeBars => {
                "range bar series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedStreamlines => {
                "streamline series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedHexagonal => {
                "hexagonal grids require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedTernary => {
                "ternary series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedBreak => {
                "broken axes require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedProbability => {
                "probability axes require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedImage => {
                "image series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedParallel => {
                "parallel coordinates series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedTriMesh => {
                "triangle mesh series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedStem => {
                "stem series require GPU-resident columns; streamed sources are not supported"
            }
            Self::StreamedEnergyLevels => {
                "energy-level series are DataLess (SSOT only); streamed displays do not draw them"
            }
            Self::StreamedFeynman => {
                "Feynman series are DataLess (SSOT only); streamed displays do not draw them"
            }
            other => return other.fmt_fielded(f),
        };
        f.write_str(sentence)
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
                "too many decoration offsets",
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
                "range bar direction and state colours must be opaque",
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
            (RuleKind::AnnotationCount, "too many annotations"),
            (RuleKind::FeynmanLineStyle, "invalid Feynman line style"),
            (RuleKind::FeynmanWave, "invalid Feynman wave or coil"),
            (
                RuleKind::FeynmanSizes,
                "invalid Feynman marker, arrow, loop or fan size",
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
                RuleKind::AutoLayoutNotFeynman,
                "auto_layout names a Feynman series of the chart",
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
                "a view state must carry one Y range per overlay layer",
            ),
            (
                RuleKind::ViewStateSubplots,
                "a view state must carry one view per subplot, each with one Y range per overlay layer",
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
                "breaks go on bottom_x, left_y or an overlay's y_axis",
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

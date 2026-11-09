//! Triangle mesh fields: [`DataRenderType::TriMesh`] (#210).
//!
//! A mesh series draws a field over triangles the host gives: the output of
//! a finite element or finite volume solver, or scattered measurements the
//! host triangulated. It reads
//!
//! - `x_column` / `y_column`: the vertex coordinates, one row per vertex;
//! - [`TriMeshSeries::triangles`]: a triangle index column (the pool's
//!   fourth source kind), three vertex rows per triangle, in triangle
//!   order;
//! - [`TriMeshSeries::values`]: one value per vertex or one per triangle,
//!   as [`MeshValues::at`] says (never inferred from the lengths);
//! - on a 3D chart (`Config.view_3d`), `z_column`: the vertex heights on
//!   `right_y`.
//!
//! It draws a fill, contour lines or both, coloured through
//! `Config.colorbar`, and on a 3D chart a surface with the look of
//! [`TriMeshSeries::surface`].
//!
//! The host computes every value the reader reads as data (rule 1): the
//! triangulation, the coordinates, the values and the heights. The renderer
//! computes appearance only: the projection, the colour mapping, the linear
//! interpolation of the given vertex values across each triangle, the
//! level set of that interpolant as contour strokes, shading normals and
//! the picked highlight, on every draw and never stored (rule 2).
//!
//! [`DataRenderType::TriMesh`]: crate::data_config::DataRenderType::TriMesh

use crate::config::Config;
use crate::data::ColumnId;
use crate::data_config::{
    ContourConfig, DataRenderType, FieldFillConfig, FillMode, SeriesConfig, Shading,
};
use crate::view3d::SurfaceStyle;

/// A field over host-given triangles. See the [module documentation](self).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TriMeshSeries {
    /// The triangle index column (`Renderer::upsert_triangles`): three
    /// vertex rows per triangle.
    pub triangles: ColumnId,
    /// The values the mesh is coloured by, and whether they belong to the
    /// vertices or the triangles.
    pub values: MeshValues,
    /// The fill, or none (omitted from JSON).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub fill: Option<FieldFillConfig>,
    /// The contour lines, or none (omitted from JSON). Drawn over the fill,
    /// or on the box floor of a 3D chart.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub contour: Option<ContourConfig>,
    /// The look of the fill drawn as a surface on a 3D chart; kept and
    /// unused on a flat chart. `None` (omitted from JSON) draws the surface
    /// with [`SurfaceStyle::default`].
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub surface: Option<SurfaceStyle>,
}

/// The value column of a mesh and where its values sit.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MeshValues {
    /// The value column.
    pub column: ColumnId,
    /// Whether row `i` is the value of vertex `i` or of triangle `i`.
    pub at: MeshValueLocation,
}

/// Where the values of a mesh sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MeshValueLocation {
    /// One value per vertex, interpolated linearly across each triangle
    /// (`Shading::Interpolated`); the contour lines are the level sets of
    /// that interpolant.
    Vertices,
    /// One value per triangle, painted flat (`Shading::Flat`). A field
    /// constant on each triangle has cell boundaries, not isolines, so it
    /// draws no contour lines.
    Faces,
}

impl TriMeshSeries {
    /// A mesh's own rules:
    ///
    /// - it names its triangle and value columns;
    /// - it draws a fill, contour lines or both, and a surface look only
    ///   with a fill;
    /// - vertex values are shaded `Interpolated` and face values `Flat`
    ///   (a flat colour from vertex values would be a mean the colourbar
    ///   shows, and interpolating face values would need values at the
    ///   vertices: both are host work);
    /// - contour lines need vertex values and carry no labels;
    /// - the surface look takes no `color_matrix` (the values colour the
    ///   mesh) and its wireframe, gloss and texture are in range.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        if self.triangles.is_empty() {
            return Err(crate::RuleKind::TriMeshTriangles);
        }
        if self.values.column.is_empty() {
            return Err(crate::RuleKind::TriMeshValues);
        }
        if self.fill.is_none() && self.contour.is_none() {
            return Err(crate::RuleKind::TriMeshNothingDrawn);
        }
        if self.fill.is_none() && self.surface.is_some() {
            return Err(crate::RuleKind::TriMeshSurfaceWithoutFill);
        }
        if let Some(fill) = &self.fill {
            match (self.values.at, &fill.shading) {
                (MeshValueLocation::Vertices, Shading::Flat) => {
                    return Err(crate::RuleKind::TriMeshVertexValuesFlat);
                }
                (MeshValueLocation::Faces, Shading::Interpolated) => {
                    return Err(crate::RuleKind::TriMeshFaceValuesInterpolated);
                }
                _ => {}
            }
        }
        if let Some(contour) = &self.contour {
            if self.values.at == MeshValueLocation::Faces {
                return Err(crate::RuleKind::TriMeshContourFaceValues);
            }
            if contour.labels.is_some() {
                return Err(crate::RuleKind::TriMeshContourLabels);
            }
        }
        if let Some(surface) = &self.surface {
            if surface.color_matrix.is_some() {
                return Err(crate::RuleKind::TriMeshSurfaceColorMatrix);
            }
            crate::view3d::validate_surface_look(surface)?;
        }
        Ok(())
    }

    /// The triangles a mesh draws, given the length of its triangle index
    /// column: three indices each, a trailing partial triangle not drawn.
    pub fn triangle_count(index_len: usize) -> usize {
        index_len / 3
    }
}

impl SeriesConfig {
    /// The mesh fields of this series, or `None` for every other kind.
    pub fn tri_mesh(&self) -> Option<&TriMeshSeries> {
        match &self.render_type {
            DataRenderType::TriMesh(mesh) => Some(mesh),
            _ => None,
        }
    }
}

impl Config {
    /// The chart rules of triangle mesh series (#210): each is valid on its
    /// own ([`TriMeshSeries::validate`]), names its vertex coordinate
    /// columns, and is drawn on a rectangular or 3D chart, not on polar,
    /// ternary or parallel axes. On a 3D chart it names its height column
    /// (`z_column`) and its fill is continuous, not banded.
    pub fn validate_tri_mesh_series(&self, series: &[SeriesConfig]) -> Result<(), crate::RuleKind> {
        for item in series {
            let Some(mesh) = item.tri_mesh() else {
                continue;
            };
            mesh.validate()?;
            if item.x_column.is_empty() || item.y_column.is_empty() {
                return Err(crate::RuleKind::TriMeshVertexColumns);
            }
            if self.polar().is_some() {
                return Err(crate::RuleKind::PolarTriMesh);
            }
            if self.ternary().is_some() {
                return Err(crate::RuleKind::TriMeshTernary);
            }
            if self.parallel().is_some() {
                return Err(crate::RuleKind::TriMeshParallel);
            }
            if self.view_3d().is_some() {
                if item.z_column.as_deref().is_none_or(str::is_empty) {
                    return Err(crate::RuleKind::TriMeshWithoutZColumn);
                }
                if mesh
                    .fill
                    .as_ref()
                    .is_some_and(|f| f.mode == FillMode::Bands)
                {
                    return Err(crate::RuleKind::View3dBandedSurface);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::data_config::{ContourLabelConfig, DataLineStyleConfig, LineInterpolation};
    use crate::format::LabelFormat;
    use crate::line::LineStylePreset;

    fn fill(shading: Shading) -> FieldFillConfig {
        FieldFillConfig {
            mode: FillMode::Continuous,
            shading,
            opacity: 1.0,
        }
    }

    fn contour() -> ContourConfig {
        ContourConfig {
            levels: vec![0.25, 0.5, 0.75],
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

    fn mesh(at: MeshValueLocation) -> TriMeshSeries {
        TriMeshSeries {
            triangles: "tris".into(),
            values: MeshValues {
                column: "cp".into(),
                at,
            },
            fill: Some(fill(match at {
                MeshValueLocation::Vertices => Shading::Interpolated,
                MeshValueLocation::Faces => Shading::Flat,
            })),
            contour: None,
            surface: None,
        }
    }

    fn series(mesh: TriMeshSeries) -> SeriesConfig {
        SeriesConfig {
            series_id: "mesh".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::TriMesh(mesh),
        }
    }

    #[test]
    fn a_mesh_draws_a_fill_contours_or_both_with_columns_named() {
        assert_eq!(mesh(MeshValueLocation::Vertices).validate(), Ok(()));
        assert_eq!(mesh(MeshValueLocation::Faces).validate(), Ok(()));
        let mut both = mesh(MeshValueLocation::Vertices);
        both.contour = Some(contour());
        assert_eq!(both.validate(), Ok(()));
        let mut lines = both.clone();
        lines.fill = None;
        assert_eq!(lines.validate(), Ok(()));

        let mut neither = lines.clone();
        neither.contour = None;
        assert_eq!(
            neither.validate(),
            Err(crate::RuleKind::TriMeshNothingDrawn)
        );
        let mut look = lines.clone();
        look.surface = Some(SurfaceStyle::default());
        assert_eq!(
            look.validate(),
            Err(crate::RuleKind::TriMeshSurfaceWithoutFill)
        );
        let mut unnamed = both.clone();
        unnamed.triangles.clear();
        assert_eq!(unnamed.validate(), Err(crate::RuleKind::TriMeshTriangles));
        let mut unnamed = both;
        unnamed.values.column.clear();
        assert_eq!(unnamed.validate(), Err(crate::RuleKind::TriMeshValues));
    }

    #[test]
    fn the_shading_follows_where_the_values_sit() {
        let mut flat_vertices = mesh(MeshValueLocation::Vertices);
        flat_vertices.fill = Some(fill(Shading::Flat));
        assert!(matches!(
            flat_vertices.validate().unwrap_err(),
            crate::RuleKind::TriMeshVertexValuesFlat
        ));
        let mut smooth_faces = mesh(MeshValueLocation::Faces);
        smooth_faces.fill = Some(fill(Shading::Interpolated));
        assert!(matches!(
            smooth_faces.validate().unwrap_err(),
            crate::RuleKind::TriMeshFaceValuesInterpolated
        ));
    }

    #[test]
    fn contours_need_vertex_values_and_carry_no_labels() {
        let mut faces = mesh(MeshValueLocation::Faces);
        faces.contour = Some(contour());
        assert!(matches!(
            faces.validate().unwrap_err(),
            crate::RuleKind::TriMeshContourFaceValues
        ));
        let mut labelled = mesh(MeshValueLocation::Vertices);
        let mut lines = contour();
        lines.labels = Some(ContourLabelConfig {
            visible: true,
            font_size: 11.0,
            color: Color::BLACK,
            format: LabelFormat::Decimal,
            significant_digits: 3,
            spacing_px: 120.0,
            anchors: Vec::new(),
            bg_color: None,
            bg_padding_px: 0.0,
        });
        labelled.contour = Some(lines);
        assert_eq!(
            labelled.validate(),
            Err(crate::RuleKind::TriMeshContourLabels)
        );
    }

    #[test]
    fn the_surface_look_takes_no_colour_matrix_and_stays_in_range() {
        let mut coloured = mesh(MeshValueLocation::Vertices);
        coloured.surface = Some(SurfaceStyle {
            color_matrix: Some(crate::data_config::MatrixRef {
                columns: vec!["c".into()],
                orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
                grid_layout: crate::data_config::GridLayout::Centers,
            }),
            ..SurfaceStyle::default()
        });
        assert_eq!(
            coloured.validate(),
            Err(crate::RuleKind::TriMeshSurfaceColorMatrix)
        );
        let mut glossy = mesh(MeshValueLocation::Vertices);
        glossy.surface = Some(SurfaceStyle {
            gloss: 2.0,
            ..SurfaceStyle::default()
        });
        assert!(matches!(
            glossy.validate().unwrap_err(),
            crate::RuleKind::SurfaceMaterialStyle
        ));
        let mut plain = mesh(MeshValueLocation::Vertices);
        plain.surface = Some(SurfaceStyle::default());
        assert_eq!(plain.validate(), Ok(()));
    }

    #[test]
    fn the_chart_rules_place_a_mesh_on_rectangular_and_3d_charts() {
        let config = crate::defaults::config();
        let item = series(mesh(MeshValueLocation::Vertices));
        assert_eq!(
            config.validate_tri_mesh_series(std::slice::from_ref(&item)),
            Ok(())
        );
        let mut unnamed = item.clone();
        unnamed.y_column.clear();
        assert!(matches!(
            config.validate_tri_mesh_series(&[unnamed]).unwrap_err(),
            crate::RuleKind::TriMeshVertexColumns
        ));

        let mut polar = config.clone();
        polar.set_polar(Some(crate::polar::PolarOptions::default()));
        assert_eq!(
            polar.validate_tri_mesh_series(std::slice::from_ref(&item)),
            Err(crate::RuleKind::PolarTriMesh)
        );
        let mut ternary = config.clone();
        ternary.set_ternary(Some(crate::ternary::TernaryOptions::default()));
        assert_eq!(
            ternary.validate_tri_mesh_series(std::slice::from_ref(&item)),
            Err(crate::RuleKind::TriMeshTernary)
        );
        let mut parallel = config.clone();
        parallel.set_parallel(Some(crate::parallel::ParallelOptions::default()));
        assert_eq!(
            parallel.validate_tri_mesh_series(std::slice::from_ref(&item)),
            Err(crate::RuleKind::TriMeshParallel)
        );

        let mut three_d = config.clone();
        three_d.set_view_3d(Some(crate::view3d::View3d::default()));
        assert!(
            three_d
                .validate_tri_mesh_series(std::slice::from_ref(&item))
                .unwrap_err()
                .to_string()
                .contains("(z_column)")
        );
        let mut lifted = item.clone();
        lifted.z_column = Some("z".into());
        assert_eq!(
            three_d.validate_tri_mesh_series(std::slice::from_ref(&lifted)),
            Ok(())
        );
        if let DataRenderType::TriMesh(mesh) = &mut lifted.render_type {
            mesh.fill.as_mut().unwrap().mode = FillMode::Bands;
        }
        assert_eq!(
            three_d.validate_tri_mesh_series(&[lifted.clone()]),
            Err(crate::RuleKind::View3dBandedSurface)
        );
        // The same banded mesh draws on a flat chart; the height is kept.
        assert_eq!(config.validate_tri_mesh_series(&[lifted]), Ok(()));
    }

    #[test]
    fn step_selection_walks_triangles_and_mesh_series() {
        use crate::config::{DataSelectionsConfig, PickedDataRef};
        let named = |id: &str, triangles: &str| {
            let mut item = series(mesh(MeshValueLocation::Vertices));
            item.series_id = id.into();
            if let DataRenderType::TriMesh(mesh) = &mut item.render_type {
                mesh.triangles = triangles.into();
            }
            item
        };
        let mut other = series(mesh(MeshValueLocation::Vertices));
        other.series_id = "points".into();
        other.render_type = DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(contour().line),
            errors: None,
            stem: None,
        });
        let list = [named("a", "a.tris"), other, named("b", "b.tris")];
        let mut config = crate::defaults::config();
        let select = |config: &mut Config, series_id: &str, triangle_index| {
            config.picked_data = Some(DataSelectionsConfig {
                refs: vec![PickedDataRef::MeshTriangle {
                    source_id: None,
                    series_id: series_id.into(),
                    triangle_index,
                }],
                ..Default::default()
            });
        };
        // Index counts, three per triangle; a trailing partial triangle is
        // not drawn and not stepped to.
        let len = |id: &str| match id {
            "a.tris" => Some(3 * 10 + 2),
            "b.tris" => Some(3 * 4),
            _ => Some(100),
        };
        let at = |config: &Config| match &config.picked_data.as_ref().unwrap().refs[0] {
            PickedDataRef::MeshTriangle {
                series_id,
                triangle_index,
                ..
            } => (series_id.clone(), *triangle_index),
            other => panic!("{other:?}"),
        };
        select(&mut config, "a", 8);
        assert_eq!(config.step_selection(&list, 1, 0, len), Ok(true));
        assert_eq!(at(&config), ("a".into(), 9));
        // Clamped at the last whole triangle, never wrapped: no move.
        assert_eq!(config.step_selection(&list, 5, 0, len), Ok(false));
        assert_eq!(at(&config), ("a".into(), 9));
        // `dy` moves across the mesh series only, the index clamped there.
        assert_eq!(config.step_selection(&list, 0, 1, len), Ok(true));
        assert_eq!(at(&config), ("b".into(), 3));
        assert_eq!(config.step_selection(&list, -2, -1, len), Ok(true));
        assert_eq!(at(&config), ("a".into(), 1));
        // An unregistered index column draws nothing to step to.
        select(&mut config, "a", 0);
        assert_eq!(config.step_selection(&list, 1, 0, |_| None), Ok(false));
        assert_eq!(at(&config), ("a".into(), 0));
        // A contour level of a mesh does not step (the existing rule).
        config.picked_data = Some(DataSelectionsConfig {
            refs: vec![PickedDataRef::ContourLevel {
                source_id: None,
                series_id: "a".into(),
                level_index: 1,
                x_index: 4,
                y_index: 0,
            }],
            ..Default::default()
        });
        assert_eq!(config.step_selection(&list, 1, 1, len), Ok(false));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn a_mesh_triangle_selection_has_its_own_tag() {
        use crate::config::PickedDataRef;
        let selection = PickedDataRef::MeshTriangle {
            source_id: None,
            series_id: "mesh".into(),
            triangle_index: 7,
        };
        let text = serde_json::to_string(&selection).unwrap();
        assert_eq!(
            text,
            "{\"kind\":\"mesh_triangle\",\"series_id\":\"mesh\",\"triangle_index\":7}"
        );
        assert_eq!(
            serde_json::from_str::<PickedDataRef>(&text).unwrap(),
            selection
        );
        assert_eq!(selection.series_id(), "mesh");
        assert_eq!(selection.source_id(), None);
    }

    #[test]
    fn validate_series_names_the_mesh_rule_and_the_series() {
        let config = crate::defaults::config();
        let mut item = series(mesh(MeshValueLocation::Faces));
        assert_eq!(config.validate_series(std::slice::from_ref(&item)), Ok(()));
        if let DataRenderType::TriMesh(mesh) = &mut item.render_type {
            mesh.contour = Some(contour());
        }
        let error = config.validate_series(&[item]).unwrap_err();
        assert_eq!(error.field, "tri_mesh");
        assert_eq!(error.series_id.as_deref(), Some("mesh"));
        assert!(matches!(
            error.kind,
            crate::RuleKind::TriMeshContourFaceValues
        ));
        // On polar axes the chart's polar rule names it too.
        let mut polar = config.clone();
        polar.set_polar(Some(crate::polar::PolarOptions::default()));
        let item = series(mesh(MeshValueLocation::Vertices));
        assert_eq!(
            polar.validate_series_on_polar(&item),
            Err(crate::RuleKind::PolarTriMesh)
        );
    }

    #[test]
    fn a_trailing_partial_triangle_is_not_counted() {
        assert_eq!(TriMeshSeries::triangle_count(0), 0);
        assert_eq!(TriMeshSeries::triangle_count(5), 1);
        assert_eq!(TriMeshSeries::triangle_count(6), 2);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn a_mesh_round_trips_with_absent_parts_omitted() {
        let mut item = series(mesh(MeshValueLocation::Vertices));
        let text = serde_json::to_string(&item.render_type).unwrap();
        assert_eq!(
            text,
            "{\"TriMesh\":{\"triangles\":\"tris\",\"values\":{\"column\":\"cp\",\"at\":\
             \"Vertices\"},\"fill\":{\"mode\":\"Continuous\",\"shading\":\"Interpolated\",\
             \"opacity\":1.0}}}"
        );
        let back: DataRenderType = serde_json::from_str(&text).unwrap();
        assert_eq!(back, item.render_type);

        if let DataRenderType::TriMesh(mesh) = &mut item.render_type {
            mesh.contour = Some(contour());
            mesh.surface = Some(SurfaceStyle::default());
        }
        item.z_column = Some("z".into());
        let text = serde_json::to_string(&item).unwrap();
        let back: SeriesConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(back, item);
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
    }
}

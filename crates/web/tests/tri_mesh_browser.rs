#![cfg(target_arch = "wasm32")]
//! A triangle mesh (#210) through the WASM wrapper: `register_triangles`,
//! the typed pick, `selection_details` and `step_selection`.

use figgy::FiggyChart;
use renderer::{
    Color, ColorMap, Config, DataRenderType, SeriesConfig,
    config::{DataSelectionsConfig, PickedDataRef},
    data_config::{FieldFillConfig, FillMode, Shading},
    tri_mesh::{MeshValueLocation, MeshValues, TriMeshSeries},
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

fn canvas() -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(480);
    canvas.set_height(320);
    canvas
}

fn mesh(at: MeshValueLocation, values: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: "mesh".into(),
        source_id: Some("solver".into()),
        label: None,
        x_column: "mx".into(),
        y_column: "my".into(),
        z_column: None,
        render_type: DataRenderType::TriMesh(TriMeshSeries {
            triangles: "tris".into(),
            values: MeshValues {
                column: values.into(),
                at,
            },
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: match at {
                    MeshValueLocation::Vertices => Shading::Interpolated,
                    MeshValueLocation::Faces => Shading::Flat,
                },
                opacity: 1.0,
            }),
            contour: None,
            surface: None,
        }),
    }
}

/// Where the chrome of `config` places `(x, y)`, in canvas pixels.
fn at(config: &Config, x: f64, y: f64) -> (f32, f32) {
    let area = config.data_area().unwrap().0;
    let fx = renderer::layout::axis_fraction(x, &config.bottom_x);
    let fy = renderer::layout::axis_fraction(y, &config.left_y);
    (
        area.x as f32 + area.width as f32 * fx,
        area.y as f32 + area.height as f32 * (1.0 - fy),
    )
}

async fn details(chart: &FiggyChart) -> serde_json::Value {
    serde_json::from_str(&chart.selection_details().await.unwrap()).unwrap()
}

#[wasm_bindgen_test(async)]
async fn a_mesh_picks_reports_and_steps_its_triangles() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart
        .register_column_f64("mx", &[0.0, 2.0, 2.0, 0.0])
        .unwrap();
    chart
        .register_column_f64("my", &[0.0, 0.0, 2.0, 2.0])
        .unwrap();
    chart
        .register_column_f64("mv", &[0.0, 2.0, 6.0, 4.0])
        .unwrap();
    chart.register_column_f64("mf", &[1.5, 4.5]).unwrap();
    chart
        .register_triangles("tris", &[0, 1, 2, 0, 2, 3])
        .unwrap();
    let mut config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config.bottom_x.min = -1.0;
    config.bottom_x.max = 3.0;
    config.left_y.min = -1.0;
    config.left_y.max = 3.0;
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = ColorMap::Custom {
        stops: vec![Color::from_rgb8(255, 0, 0), Color::from_rgb8(0, 255, 0)],
    };
    bar.axis.min = 0.0;
    bar.axis.max = 6.0;
    config.colorbar = Some(bar);
    config.picked_data = None;
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .unwrap();
    chart
        .set_series(&serde_json::to_string(&vec![mesh(MeshValueLocation::Vertices, "mv")]).unwrap())
        .unwrap();
    chart.frame().unwrap();

    // The typed pick answers each triangle and its vertex nearest the
    // cursor; off the mesh, nothing.
    let config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    for ((x, y), triangle, vertex) in [((1.5, 0.25), 0, 1), ((0.25, 1.5), 1, 3)] {
        let (px, py) = at(&config, x, y);
        let picked = chart.pick_data(px, py, 4.0).await.unwrap();
        let picked: serde_json::Value =
            serde_json::from_str(&picked.as_string().expect("a triangle is picked")).unwrap();
        assert_eq!(picked["kind"], "mesh_triangle", "{picked}");
        assert_eq!(picked["series_id"], "mesh");
        assert_eq!(picked["source_id"], "solver");
        assert_eq!(picked["triangle_index"], triangle, "{picked}");
        assert_eq!(picked["vertex_index"], vertex, "{picked}");
    }
    let (px, py) = at(&config, -0.5, 2.5);
    assert!(chart.pick_data(px, py, 4.0).await.unwrap().is_undefined());

    // The details read the triangle's indices and their samples back from
    // the GPU columns the draw used.
    let mut selected = config.clone();
    selected.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::MeshTriangle {
            source_id: Some("solver".into()),
            series_id: "mesh".into(),
            triangle_index: 0,
        }],
        ..Default::default()
    });
    chart
        .set_config(&serde_json::to_string(&selected).unwrap())
        .unwrap();
    chart.frame().unwrap();
    let selected = details(&chart).await;
    assert_eq!(selected["identity"]["kind"], "mesh_triangle", "{selected}");
    assert_eq!(selected["index"], 0);
    assert_eq!(selected["vertices"], serde_json::json!([0, 1, 2]));
    assert_eq!(selected["x"], serde_json::json!([0.0, 2.0, 2.0]));
    assert_eq!(selected["y"], serde_json::json!([0.0, 0.0, 2.0]));
    assert_eq!(selected["values"], serde_json::json!([0.0, 2.0, 6.0]));

    // `dx` walks the triangles and stops at the last; there is no other
    // mesh series for `dy`.
    assert!(chart.step_selection(1, 0).unwrap());
    let stepped = details(&chart).await;
    assert_eq!(stepped["index"], 1);
    assert_eq!(stepped["vertices"], serde_json::json!([0, 2, 3]));
    assert!(!chart.step_selection(1, 0).unwrap());
    assert!(!chart.step_selection(0, 1).unwrap());

    // Face values: one value per triangle.
    chart
        .set_series(&serde_json::to_string(&vec![mesh(MeshValueLocation::Faces, "mf")]).unwrap())
        .unwrap();
    let faces = details(&chart).await;
    assert_eq!(faces["index"], 1);
    assert_eq!(faces["value"], 4.5);
    assert_eq!(faces["at"], "Faces");

    // On a 3D chart the mesh draws as a surface over its height column
    // (#210), through the browser's own WGSL compiler, and the details
    // report the triangle's heights too.
    let mut lifted = mesh(MeshValueLocation::Vertices, "mv");
    lifted.z_column = Some("mv".into());
    chart
        .set_series(&serde_json::to_string(&vec![lifted]).unwrap())
        .unwrap();
    let mut three_d: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    three_d.set_view_3d(Some(renderer::view3d::View3d::default()));
    three_d.right_y.min = 0.0;
    three_d.right_y.max = 6.0;
    chart
        .set_config(&serde_json::to_string(&three_d).unwrap())
        .unwrap();
    chart.frame().unwrap();
    let surface = details(&chart).await;
    assert_eq!(surface["index"], 1, "{surface}");
    assert_eq!(surface["vertices"], serde_json::json!([0, 2, 3]));
    assert_eq!(surface["values"], serde_json::json!([0.0, 6.0, 4.0]));
    assert_eq!(surface["z"], serde_json::json!([0.0, 6.0, 4.0]));
}

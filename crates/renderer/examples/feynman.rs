//! Feynman diagrams: DataLess series whose topology is the data. The host
//! lists the vertices and propagators and pins the external legs; the
//! layout places every internal vertex.
//! - `render` writes `gallery-feynman.png`: six processes on cards in a
//!   3 x 2 grid (electron-positron annihilation into muons, Compton
//!   scattering, Higgs production by gluon fusion through a top loop, beta
//!   decay through a W, the electron's one-loop vertex correction and a
//!   three-jet event). The warm-paper family's Feynman member gives the look:
//!   each line's colour by its particle, the strokes and the label ink.
//!
//! cargo run --locked -p figgy-renderer --example feynman -- target/feynman
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use ParticleLine::{ChargedBoson, Fermion, Gluon, Photon, Scalar};
use VertexRole::{Incoming, Internal, Outgoing};
use gallery::Result;
use renderer::theme::ThemeFamily;
use renderer::{
    Anchor, Annotation, AnnotationKind, AnnotationLineStyle, Color, Renderer, SeriesConfig,
    ShapeStyle,
    feynman::{
        FeynmanSeries, FeynmanVertex, MomentumArrow, MomentumSide, ParticleLine, Propagator,
        VertexRole,
    },
    text::RichText,
    theme::ChartKind,
};
use std::path::Path;
use styled::{background, export, save, scene_chart, themed};

/// A label: its content, in the look the theme gives it.
fn tex(text: &str) -> RichText {
    RichText::from_tex(text, Color::BLACK, 16., "").expect("valid TeX")
}

/// One diagram on its card: the card's lower-left corner in data units, so
/// every pinned leg is given in the card's own unit square.
struct Card {
    origin: [f64; 2],
}

impl Card {
    fn leg(&self, id: &str, role: VertexRole, [x, y]: [f64; 2], label: &str) -> FeynmanVertex {
        FeynmanVertex {
            at: Some([self.origin[0] + x, self.origin[1] + y]),
            label: (!label.is_empty()).then(|| tex(label)),
            ..FeynmanVertex::new(id, role)
        }
    }

    /// An internal vertex pinned where the free layout would draw it too
    /// small or too short.
    fn pinned(&self, id: &str, [x, y]: [f64; 2]) -> FeynmanVertex {
        FeynmanVertex {
            at: Some([self.origin[0] + x, self.origin[1] + y]),
            ..FeynmanVertex::new(id, Internal)
        }
    }
}

fn vertex(id: &str) -> FeynmanVertex {
    FeynmanVertex::new(id, Internal)
}

/// A propagator; the theme colours it by its particle.
fn line(id: &str, from: &str, to: &str, particle: ParticleLine, label: &str) -> Propagator {
    Propagator {
        label: (!label.is_empty()).then(|| tex(label)),
        ..Propagator::new(id, from, to, particle)
    }
}

fn diagram(vertices: Vec<FeynmanVertex>, propagators: Vec<Propagator>) -> FeynmanSeries {
    FeynmanSeries {
        vertices,
        propagators,
        ..FeynmanSeries::default()
    }
}

/// The six diagrams with their captions, top row first.
fn diagrams() -> [(&'static str, FeynmanSeries); 6] {
    // e+ e- -> mu+ mu- through a virtual photon.
    let c = Card { origin: [0., 1.] };
    let muons = diagram(
        vec![
            c.leg("e-", Incoming, [0.14, 0.7], "$e^-$"),
            c.leg("e+", Incoming, [0.14, 0.12], "$e^+$"),
            vertex("v1"),
            vertex("v2"),
            c.leg("mu-", Outgoing, [0.86, 0.7], r"$\mu^-$"),
            c.leg("mu+", Outgoing, [0.86, 0.12], r"$\mu^+$"),
        ],
        vec![
            line("a", "e-", "v1", Fermion, ""),
            line("b", "v1", "e+", Fermion, ""),
            line("g", "v1", "v2", Photon, r"$\gamma$"),
            line("c", "v2", "mu-", Fermion, ""),
            line("d", "mu+", "v2", Fermion, ""),
        ],
    );
    // Compton scattering, the s-channel diagram.
    let c = Card { origin: [1., 1.] };
    let compton = diagram(
        vec![
            c.leg("e-in", Incoming, [0.14, 0.12], "$e^-$"),
            c.leg("g-in", Incoming, [0.14, 0.7], r"$\gamma$"),
            vertex("a"),
            vertex("b"),
            c.leg("e-out", Outgoing, [0.86, 0.12], "$e^-$"),
            c.leg("g-out", Outgoing, [0.86, 0.7], r"$\gamma$"),
        ],
        vec![
            line("e1", "e-in", "a", Fermion, ""),
            line("e2", "a", "b", Fermion, "$e^-$"),
            line("e3", "b", "e-out", Fermion, ""),
            line("p1", "g-in", "a", Photon, ""),
            line("p2", "b", "g-out", Photon, ""),
        ],
    );
    // Higgs production by gluon fusion through a top-quark loop.
    let c = Card { origin: [2., 1.] };
    let higgs = diagram(
        vec![
            c.leg("g1", Incoming, [0.14, 0.7], "$g$"),
            c.leg("g2", Incoming, [0.14, 0.12], "$g$"),
            c.pinned("t1", [0.4, 0.62]),
            c.pinned("t2", [0.4, 0.2]),
            c.pinned("t3", [0.6, 0.41]),
            c.leg("H", Outgoing, [0.86, 0.41], "$H$"),
        ],
        vec![
            line("ga", "g1", "t1", Gluon, ""),
            line("gb", "g2", "t2", Gluon, ""),
            line("ta", "t1", "t2", Fermion, "$t$"),
            line("tb", "t2", "t3", Fermion, ""),
            line("tc", "t3", "t1", Fermion, ""),
            line("h", "t3", "H", Scalar, ""),
        ],
    );
    // Beta decay at the quark level: d -> u W-, W- -> e- anti-nu.
    let c = Card { origin: [0., 0.] };
    let mut beta = diagram(
        vec![
            c.leg("d", Incoming, [0.14, 0.12], "$d$"),
            c.pinned("w1", [0.4, 0.2]),
            c.pinned("w2", [0.64, 0.5]),
            c.leg("u", Outgoing, [0.86, 0.12], "$u$"),
            c.leg("nu", Outgoing, [0.86, 0.4], r"$\bar\nu_e$"),
            c.leg("e", Outgoing, [0.8, 0.76], "$e^-$"),
        ],
        vec![
            line("q1", "d", "w1", Fermion, ""),
            line("q2", "w1", "u", Fermion, ""),
            line("w", "w1", "w2", ChargedBoson, "$W^-$"),
            line("l1", "w2", "e", Fermion, ""),
            line("l2", "nu", "w2", Fermion, ""),
        ],
    );
    // Clear of the wave's arrowhead.
    beta.propagators[2].label_offset = [-8., -6.];
    // The electron's one-loop vertex correction (Schwinger's g - 2).
    let c = Card { origin: [1., 0.] };
    let mut vertex_loop = diagram(
        vec![
            c.leg("in", Incoming, [0.14, 0.14], "$e^-$"),
            vertex("a"),
            vertex("c"),
            vertex("b"),
            c.leg("out", Outgoing, [0.86, 0.14], "$e^-$"),
            c.leg("field", Incoming, [0.5, 0.76], r"$\gamma$"),
        ],
        vec![
            line("e1", "in", "a", Fermion, ""),
            line("e2", "a", "c", Fermion, ""),
            line("e3", "c", "b", Fermion, ""),
            line("e4", "b", "out", Fermion, ""),
            line("loop", "a", "b", Photon, ""),
            line("ext", "field", "c", Photon, ""),
        ],
    );
    vertex_loop.propagators[4].bend = Some(-50.);
    vertex_loop.propagators[4].momentum = Some(MomentumArrow {
        label: tex("$k$"),
        side: MomentumSide::Right,
        reversed: false,
        label_offset: [0.; 2],
    });
    // A three-jet event: e+ e- -> q qbar g, the gluon radiated by the quark.
    let c = Card { origin: [2., 0.] };
    let jets = diagram(
        vec![
            c.leg("e-", Incoming, [0.14, 0.68], "$e^-$"),
            c.leg("e+", Incoming, [0.14, 0.12], "$e^+$"),
            vertex("v1"),
            vertex("v2"),
            c.pinned("v3", [0.64, 0.62]),
            c.leg("q", Outgoing, [0.8, 0.78], "$q$"),
            c.leg("g", Outgoing, [0.9, 0.44], "$g$"),
            c.leg("qb", Outgoing, [0.86, 0.1], r"$\bar q$"),
        ],
        vec![
            line("a", "e-", "v1", Fermion, ""),
            line("b", "v1", "e+", Fermion, ""),
            line("gamma", "v1", "v2", Photon, r"$\gamma^*$"),
            line("c", "v2", "v3", Fermion, ""),
            line("d", "v3", "q", Fermion, ""),
            line("e", "qb", "v2", Fermion, ""),
            line("glue", "v3", "g", Gluon, ""),
        ],
    );
    [
        (r"Annihilation: $e^+e^- \to \mu^+\mu^-$", muons),
        (r"Compton scattering: $e^-\gamma \to e^-\gamma$", compton),
        (r"Gluon fusion: $gg \to H$", higgs),
        (r"Beta decay: $d \to u\, e^-\, \bar\nu_e$", beta),
        (r"Electron $g-2$ at one loop", vertex_loop),
        (r"Three jets: $e^+e^- \to q\bar q g$", jets),
    ]
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    let theme = ThemeFamily::WarmPaper.theme(ChartKind::Feynman);
    let bg = background(&theme);
    let chrome = &theme.chrome;
    let mut chart = scene_chart("Feynman diagrams / six processes", "", "");
    chart.set_x_range(0., 3.);
    chart.set_y_range(0., 2.);
    let config = chart.config_mut();
    config.legend.visible = false;
    let mut specs = Vec::new();
    for (k, (caption, diagram)) in diagrams().into_iter().enumerate() {
        let (x, y) = ((k % 3) as f64, if k < 3 { 1. } else { 0. });
        // A card under each diagram in the theme's box colours, its
        // caption at the top left in the theme's ink.
        config
            .annotations
            .items
            .push(Annotation::new(AnnotationKind::Rect {
                from: Anchor::Data {
                    x: x + 0.025,
                    y: y + 0.03,
                },
                to: Anchor::Data {
                    x: x + 0.975,
                    y: y + 0.97,
                },
                style: ShapeStyle {
                    fill: Some(chrome.legend_fill),
                    border: AnnotationLineStyle {
                        color: chrome.legend_border,
                        width: 1.,
                        ..Default::default()
                    },
                    below: true,
                },
            }));
        config
            .annotations
            .items
            .push(Annotation::new(AnnotationKind::Text {
                anchor: Anchor::Data {
                    x: x + 0.07,
                    y: y + 0.93,
                },
                text: RichText {
                    color: chrome.ink,
                    font_size: chrome.legend_size,
                    ..tex(caption)
                },
                box_style: None,
                align: Default::default(),
            }));
        specs.push(SeriesConfig::feynman_diagram(
            format!("diagram-{k}"),
            diagram,
        ));
    }
    themed(&mut chart, &theme, &mut specs)?;
    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-feynman.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/feynman")?;
    gallery::with_renderer(|r| render(r, &out))
}

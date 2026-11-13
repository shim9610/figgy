# Science diagrams

[← Gallery](../../../README.md#gallery) · [Lines, areas and stems](lines.md) · [Bars and distributions](distributions.md) · [Ranges and timelines](ranges.md) · [Fields and maps](fields.md) · [3D](3d.md) · [Polar and radial](polar.md) · **Science diagrams** · [Layouts and annotations](layouts.md) · [Styles and visual concepts](styles.md)

Click an image to open the example code that draws it. All data is synthetic;
the images are native renderer output.

## Energy levels

<a href="../examples/nist_grotrian_gallery.rs"><img src="../assets/gallery-grotrian.png" alt="Grotrian diagram of He I" width="760"></a>

A Grotrian diagram of He I from NIST data: singlet and triplet term columns, transitions labelled with their wavelengths to one decimal, the unit once in a corner note, the ground state below a broken energy axis.

**Code:** [nist_grotrian_gallery.rs](../examples/nist_grotrian_gallery.rs), case `he-i-1x`

**Run:** `cargo run --locked -p figgy-renderer --example nist_grotrian_gallery -- target/nist-grotrian-gallery`

## Feynman diagrams

<a href="../examples/feynman.rs"><img src="../assets/gallery-feynman.png" alt="Six Feynman diagrams" width="760"></a>

Six processes on cards: annihilation, Compton scattering, gluon fusion through a top loop, beta decay through a W, the electron's one-loop vertex and a three-jet event.

**Code:** [feynman.rs](../examples/feynman.rs), `render`

**Run:** `cargo run --locked -p figgy-renderer --example feynman -- target/feynman`

## Ternary plot

<a href="../examples/ternary.rs"><img src="../assets/gallery-ternary.png" alt="Soil texture triangle with three cores" width="760"></a>

The USDA soil texture triangle: twelve texture classes as filled region polygons named by text annotations, and three soil cores sampled every 10 cm as lines whose markers grow with depth (`point_size_map`).

**Code:** [ternary.rs](../examples/ternary.rs), `render`

**Run:** `cargo run --locked -p figgy-renderer --example ternary -- target/ternary`

## Probability paper

<a href="../examples/probability_paper.rs"><img src="../assets/gallery-probability.png" alt="Probability paper" width="760"></a>

Normal and Weibull probability axes (#209), the Weibull panel over a log time axis.

**Code:** [probability_paper.rs](../examples/probability_paper.rs), `probability_paper`

**Run:** `cargo run --locked -p figgy-renderer --example probability_paper -- target/probability`

## Parallel coordinates

<a href="../examples/parallel.rs"><img src="../assets/gallery-parallel.png" alt="Parallel coordinates of a hyperparameter sweep" width="760"></a>

A 120-run hyperparameter sweep across six axes (two of them logarithmic), each run coloured by its validation accuracy through the colourbar, the best drawn last.

**Code:** [parallel.rs](../examples/parallel.rs), `render`

**Run:** `cargo run --locked -p figgy-renderer --example parallel -- target/parallel`

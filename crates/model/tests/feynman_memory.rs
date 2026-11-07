//! The working memory of a Feynman series (#234, rule 3): every operation
//! takes its buffers from the allocator a fixed number of times, whatever
//! the size of the diagram, and no loop allocates. A counting allocator
//! (a per-thread counter, no lock: rule 4) checks each count exactly.
//!
//! `cargo test --release -p figgy-model --test feynman_memory -- --ignored
//! --nocapture` runs the benchmark: the time of each operation and the
//! share its allocations take.

use model::annotation::{AxisMap, AxisSpace, PixelSpace};
use model::color::Color;
use model::feynman::{
    FeynmanHit, FeynmanSeries, FeynmanVertex, ParticleLine, Propagator, VertexRole,
};
use model::layout::{BreakMap, RectF};
use model::sink::{Border, PathSegment, PlacedText, Sink, Slot, StrokeStyle};
use model::text::{MeasureText, RichText, TextExtents};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

struct Counting;

thread_local! {
    // `const` and without a destructor: reading it never allocates.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: every call is forwarded to the system allocator unchanged; the
// counter only reads and writes a thread-local `Cell`.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The allocations `f` makes on this thread.
fn allocations<R>(f: impl FnOnce() -> R) -> (usize, R) {
    let before = ALLOCATIONS.with(Cell::get);
    let result = f();
    (ALLOCATIONS.with(Cell::get) - before, result)
}

/// Text measured without allocating.
struct Fixed;
impl MeasureText for Fixed {
    fn measure_rich(&self, rt: &RichText) -> TextExtents {
        TextExtents {
            width: rt.segments.len() as f32 * 8.0,
            ascent: 10.0,
            descent: 3.0,
        }
    }
}

/// A sink that counts the parts and keeps nothing.
#[derive(Default)]
struct Count(usize);
impl Sink for Count {
    fn text(&mut self, _: Slot, _: &PlacedText<'_>) {
        self.0 += 1;
    }
    fn stroke(&mut self, _: Slot, _: [f32; 2], _: [f32; 2], _: &StrokeStyle<'_>) {
        self.0 += 1;
    }
    fn rect(&mut self, _: Slot, _: RectF, _: Option<Color>, _: Option<Border<'_>>) {
        self.0 += 1;
    }
    fn triangle(&mut self, _: Slot, _: [[f32; 2]; 3], _: Color) {
        self.0 += 1;
    }
    fn path(&mut self, _: Slot, segments: &[PathSegment], _: &StrokeStyle<'_>) {
        self.0 += segments.len();
    }
    fn polygon(&mut self, _: Slot, _: &[[f32; 2]], _: Option<Color>, _: Option<&StrokeStyle<'_>>) {
        self.0 += 1;
    }
    fn ramp(&mut self, _: Slot, _: RectF, _: &model::config::ColorBarOptions) {
        self.0 += 1;
    }
}

/// x and y over 0..1 on a 1200 × 800 px plot, y up.
fn space() -> PixelSpace {
    let axis = AxisSpace {
        map: AxisMap::Linear { min: 0.0, max: 1.0 },
        inverted: false,
        breaks: BreakMap::IDENTITY,
    };
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: 1200.0,
        height: 800.0,
    };
    PixelSpace {
        panel: rect,
        plot: rect,
        x: axis,
        y: axis,
    }
}

fn vertex(id: String, role: VertexRole) -> FeynmanVertex {
    FeynmanVertex::new(id, role)
}

/// A ladder of `rungs` rungs between two rails, legs at both ends; every
/// tenth rung a photon, the rails fermions; labels on the legs.
fn ladder(rungs: usize) -> FeynmanSeries {
    let mut d = FeynmanSeries::default();
    for (id, role) in [
        ("in_a", VertexRole::Incoming),
        ("in_b", VertexRole::Incoming),
        ("out_a", VertexRole::Outgoing),
        ("out_b", VertexRole::Outgoing),
    ] {
        let mut v = vertex(id.into(), role);
        v.label = Some(RichText::plain(id, Color::BLACK, 14.0, ""));
        d.vertices.push(v);
    }
    for k in 0..rungs {
        for rail in ["a", "b"] {
            d.vertices
                .push(vertex(format!("{rail}{k}"), VertexRole::Internal));
        }
        let particle = if k % 10 == 0 {
            ParticleLine::Photon
        } else {
            ParticleLine::Plain
        };
        d.propagators.push(Propagator::new(
            format!("r{k}"),
            format!("a{k}"),
            format!("b{k}"),
            particle,
        ));
        if k > 0 {
            for rail in ["a", "b"] {
                d.propagators.push(Propagator::new(
                    format!("{rail}{k}-"),
                    format!("{rail}{}", k - 1),
                    format!("{rail}{k}"),
                    ParticleLine::Fermion,
                ));
            }
        }
    }
    let last = rungs - 1;
    for (id, from, to) in [
        ("la", "in_a".to_string(), "a0".to_string()),
        ("lb", "in_b".to_string(), "b0".to_string()),
        ("ra", format!("a{last}"), "out_a".to_string()),
        ("rb", format!("b{last}"), "out_b".to_string()),
    ] {
        d.propagators
            .push(Propagator::new(id, from, to, ParticleLine::Fermion));
    }
    d
}

/// The ladder with its layout committed, as the WASM wrapper registers it.
fn committed(rungs: usize) -> FeynmanSeries {
    let mut d = ladder(rungs);
    d.auto_layout().unwrap();
    d
}

/// The most times a doubling buffer grows to hold `n` items: once from
/// empty, then once per doubling.
fn growths(n: usize) -> usize {
    1 + (usize::BITS - n.max(1).leading_zeros()) as usize
}

#[test]
fn each_operation_allocates_a_fixed_number_of_times_at_every_size() {
    let space = space();
    for rungs in [1usize, 50, 5_000] {
        let diagram = committed(rungs);
        let mut unplaced = diagram.clone();
        unplaced.vertices[5].at = None;
        let cursor = [600.0f32, 400.0];

        // Validation: one list of rows.
        assert_eq!(allocations(|| diagram.validate()).0, 1, "{rungs}: validate");
        // A committed diagram: the topology and the screen points.
        let (n, hit) = allocations(|| diagram.pick(&space, &Fixed, cursor, 4.0).unwrap());
        assert_eq!(n, 2, "{rungs}: pick");
        let _ = hit;
        // An unplaced vertex adds the solver's work buffer.
        assert_eq!(
            allocations(|| unplaced.pick(&space, &Fixed, cursor, 4.0).unwrap()).0,
            3,
            "{rungs}: pick, unplaced"
        );
        // The emit adds its one path buffer, grown (doubling) before it is
        // written; the longest path bounds the growths.
        let mut sink = Count::default();
        let (n, ()) = allocations(|| {
            diagram
                .emit(&space, Some(FeynmanHit::Vertex(0)), &Fixed, &mut sink)
                .unwrap()
        });
        assert!(
            (3..=2 + growths(4096)).contains(&n),
            "{rungs}: emit allocated {n} times"
        );
        // Reads of a committed diagram allocate nothing.
        assert_eq!(
            allocations(|| diagram.extent().unwrap()).0,
            0,
            "{rungs}: extent"
        );
        let mut moved = diagram.clone();
        assert_eq!(
            allocations(|| moved.drag_vertex(4, 1.0, 1.0, &space)).0,
            0,
            "{rungs}: vertex drag"
        );
        assert_eq!(
            allocations(|| moved.auto_layout().unwrap()).0,
            0,
            "{rungs}: nothing to lay out"
        );
        // A bend drag places the diagram once.
        assert_eq!(
            allocations(|| moved.drag_bend(0, 0.0, -5.0, &space)).0,
            2,
            "{rungs}: bend drag"
        );
        // The layout: the list returned, the topology and the work buffer.
        assert_eq!(
            allocations(|| unplaced.positions().unwrap()).0,
            3,
            "{rungs}: layout"
        );
        assert_eq!(
            allocations(|| diagram.positions().unwrap()).0,
            1,
            "{rungs}: positions, committed"
        );
    }
}

/// The median of `runs` timings of `f`.
fn median(runs: usize, mut f: impl FnMut()) -> Duration {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    times.sort();
    times[runs / 2]
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

#[test]
#[ignore = "a benchmark: run with --release -- --ignored --nocapture"]
fn benchmark() {
    let space = space();
    println!(
        "{:>7} {:>7} {:>7} | {:>12} {:>12} {:>12} {:>12} | {:>12} {:>7}",
        "rungs", "V", "E", "pick µs", "pick+lay µs", "emit µs", "layout µs", "alloc µs", "share"
    );
    for rungs in [1usize, 10, 100, 1_000, 5_000] {
        let diagram = committed(rungs);
        let unplaced = ladder(rungs);
        let (v, e) = (diagram.vertices.len(), diagram.propagators.len());
        let runs = if rungs >= 1_000 { 21 } else { 201 };
        let cursor = [600.0f32, 400.0];
        let pick = median(runs, || {
            std::hint::black_box(diagram.pick(&space, &Fixed, cursor, 4.0).unwrap());
        });
        let pick_unplaced = median(if rungs >= 1_000 { 5 } else { runs }, || {
            std::hint::black_box(unplaced.pick(&space, &Fixed, cursor, 4.0).unwrap());
        });
        let emit = median(runs, || {
            let mut sink = Count::default();
            diagram.emit(&space, None, &Fixed, &mut sink).unwrap();
            std::hint::black_box(sink.0);
        });
        let layout = median(if rungs >= 1_000 { 5 } else { runs }, || {
            std::hint::black_box(unplaced.positions().unwrap());
        });
        // What a pick spends in the allocator: the same two buffers
        // (topology words and screen points) taken, touched and dropped.
        let words = 3 * v + 1 + 6 * e;
        let alloc = median(runs, || {
            let mut a: Vec<usize> = Vec::new();
            a.try_reserve_exact(words).unwrap();
            a.resize(words, 0);
            let mut b: Vec<[f64; 2]> = Vec::new();
            b.try_reserve_exact(v).unwrap();
            b.resize(v, [f64::NAN; 2]);
            std::hint::black_box((&a, &b));
        });
        // Only the allocator calls, without the writes a pick makes anyway.
        let calls = median(runs, || {
            let a: Vec<usize> = Vec::with_capacity(words);
            let b: Vec<[f64; 2]> = Vec::with_capacity(v);
            std::hint::black_box((&a, &b));
        });
        println!(
            "{rungs:>7} {v:>7} {e:>7} | {:>12.2} {:>12.2} {:>12.2} {:>12.2} | {:>5.2}/{:<6.2} {:>6.2}%",
            micros(pick),
            micros(pick_unplaced),
            micros(emit),
            micros(layout),
            micros(calls),
            micros(alloc),
            100.0 * calls.as_secs_f64() / pick.as_secs_f64()
        );
    }
}

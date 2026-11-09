//! Host conveniences, behind the `helpers` feature (on by default; #266).
//!
//! Everything here is for hosts and examples: `Default` on the document
//! types, the [`default`] builders and the series constructors
//! (`SeriesConfig::line`, `scatter`, ...). The model and the renderer build
//! without this feature (`ci/verify.py static` checks it), so their own code
//! takes every default from its one crate-private source and never falls back
//! on `Default`.

pub mod default;
mod defaults;
mod series;
mod series_style;
pub mod theme;

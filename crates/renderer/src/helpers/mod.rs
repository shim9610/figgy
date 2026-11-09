//! Host conveniences, behind the `helpers` feature (on by default; #265,
//! #266).
//!
//! The model's helpers (`Default` on the document types, the [`default`]
//! builders and the series constructors) and the renderer's own: a headless
//! renderer in one call ([`crate::Renderer::headless`], or
//! `headless_blocking` on native) and one export call by format
//! ([`ExportFormat`], `save_blocking`). Each step of the pipeline stays
//! explicit: create the renderer, upload the columns, set the document, then
//! draw or save. The renderer's own code builds without this feature
//! (`ci/verify.py static`), so it cannot reach for any of it.

pub use ::model::helpers::*;

mod chart;
mod defaults;
mod headless;
mod save;

pub use headless::HeadlessOptions;
pub use save::{ExportFormat, SaveError};

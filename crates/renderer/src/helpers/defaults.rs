//! `Default` on the renderer's option types, for hosts and examples (#266).

use crate::{ExportOptions, ExportScope};

/// The whole document, the scope a host gets by default.
impl Default for ExportScope {
    fn default() -> Self {
        Self::Document
    }
}

/// The whole document at scale 1 on a transparent background
/// ([`ExportOptions::document`]`(1.0)`).
impl Default for ExportOptions {
    fn default() -> Self {
        Self::document(1.0)
    }
}

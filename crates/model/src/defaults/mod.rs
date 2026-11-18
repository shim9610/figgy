//! The documented defaults of the serialized model types (#266): the one
//! source of the value a field takes when a document omits it. Serde reads
//! them (`#[serde(default = "crate::defaults::name")]`) and the model uses
//! them where it builds a document. Crate-private: hosts get them through
//! `Default` and `crate::default`, behind the `helpers` feature
//! (`crate::helpers`).

// Without `helpers` and `serde` some areas are read by nothing.
#![cfg_attr(
    not(any(feature = "helpers", feature = "serde")),
    allow(unused_imports)
)]

mod frame;
pub(crate) use frame::*;
mod series;
pub(crate) use series::*;
mod annotations;
pub(crate) use annotations::*;
mod charts;
pub(crate) use charts::*;
mod science;
pub(crate) use science::*;
mod text;
pub(crate) use text::*;

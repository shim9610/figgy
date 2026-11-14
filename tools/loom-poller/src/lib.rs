//! The renderer's poll thread protocol, compiled against loom's atomics and
//! threads. `cargo test --release` here runs its `loom_model` tests.

#[path = "../../../crates/renderer/src/gpu_poller.rs"]
#[allow(dead_code)]
mod gpu_poller;

//! # vexus-core
//!
//! The Vexus engine: everything that decides *what* may be cleaned and *how*, with no UI and no
//! Tauri dependency. The desktop app and the CLI are thin front ends over this crate.
//!
//! The pipeline is deliberately linear and every stage is independently safe:
//!
//! ```text
//! rules ──► resolve + safety guard ──► scan (read-only plan) ──► preview / exclusions
//!                                                                      │
//!                         recovery store (undo) ◄── clean (re-verify each item) ◄─┘
//! ```
//!
//! Windows only.

pub mod clean;
mod engine;
pub mod env;
pub mod error;
pub mod fsops;
pub mod history;
pub mod model;
pub mod paths;
pub mod progress;
pub mod recovery;
pub mod recyclebin;
pub mod rules;
pub mod safety;
pub mod sandbox;
pub mod scan;
pub mod settings;
pub mod system;
pub mod util;

pub use engine::Engine;
pub use error::{Error, Result};

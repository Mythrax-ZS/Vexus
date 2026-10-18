//! The safety layer: what Vexus is allowed to touch, and what it must never touch.
//!
//! The guard is **default-deny**. A path is only touchable if it lies strictly inside a
//! directory root that a built-in rule declared *and* that passed [`PathGuard::validate_root`].
//! On top of that, a hard deny-list (Windows core, Program Files, personal folders, Vexus's own
//! data) always wins, even if a rule root were somehow registered inside it.

mod guard;
mod path;

pub use guard::{
    PathGuard, Protection, RootRejection, is_ambiguous_name, is_protected_file_name,
    looks_like_temp,
};
pub use path::{is_strictly_within, is_within, normalize, strip_verbatim};

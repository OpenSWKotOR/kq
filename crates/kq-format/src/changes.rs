//! Generate a TSLPatcher `changes.ini` from a pair of resources.
//!
//! This moved to `kotor-diff`, shared with the instruction-file editor, which
//! needs to answer the same question from the other end: an editor generating
//! instructions and a query tool reporting a difference are the same comparison
//! over the same formats, and a second copy of it would drift.
//!
//! The module stays here as a re-export so every `kq_format::changes::…` path
//! keeps working.
//!
//! See [`kotor_diff::changes`] for the documentation and the tests.

pub use kotor_diff::changes::*;

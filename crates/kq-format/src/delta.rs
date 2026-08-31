//! Structural compare, patch, and three-way merge of JSON values.
//!
//! This moved to `kotor-diff`, shared with the instruction-file editor that
//! needs the same comparisons over the same formats. The module stays here as a
//! re-export so every `kq_format::delta::…` path keeps working and there is
//! still one obvious place to look for it.
//!
//! See [`kotor_diff::json`] for the documentation and the tests.

pub use kotor_diff::json::*;

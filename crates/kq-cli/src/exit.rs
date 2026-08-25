//! Exit codes.
//!
//! A script or agent should be able to branch on the outcome without parsing
//! the message, so each failure mode gets its own code and they never move.

/// The command did what was asked.
pub const OK: i32 = 0;
/// Something went wrong at runtime: unreadable file, corrupt archive, I/O.
pub const FAILURE: i32 = 1;
/// The arguments did not make sense.
pub const USAGE: i32 = 2;
/// The query was valid and matched nothing.
pub const NO_MATCH: i32 = 3;
/// No KotOR installation was found at the given or inferred path.
pub const NO_INSTALL: i32 = 4;

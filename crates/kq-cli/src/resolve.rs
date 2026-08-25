//! Turning "where do I look" into an answer.
//!
//! In order: an explicit `--install`, then `KQ_INSTALL`, then an upward walk
//! from the working directory. The upward walk is what makes `kq` usable from
//! inside `Override/` without repeating the path every time — but it only
//! ever finds a real installation, never a bare file or folder, since `cd`ing
//! somewhere and running `kq ls` should not surprise-index the whole tree.
//!
//! An explicit `--install`/`KQ_INSTALL`, by contrast, may point at anything
//! named in the tool's stated scope: an installation, a standalone capsule, a
//! folder of loose files, or a single resource file. `kq -i some.mod ls`
//! reads that one archive the same way every other command reads an index.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// No installation could be resolved. Its own type so `main` can map it to
/// [`crate::exit::NO_INSTALL`] instead of the generic failure code.
#[derive(Debug)]
pub struct NoInstall(pub String);

impl fmt::Display for NoInstall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NoInstall {}

/// What `-i`/`KQ_INSTALL`/the working directory resolved to.
pub enum Target {
    /// A full installation, rooted at a `chitin.key`.
    Install(PathBuf),
    /// A capsule, folder, or single file named directly by the user.
    Standalone(PathBuf),
}

pub fn resolve_target(explicit: Option<&PathBuf>) -> Result<Target> {
    if let Some(p) = explicit {
        return resolve_explicit(p);
    }
    if let Some(env) = std::env::var_os("KQ_INSTALL") {
        return resolve_explicit(&PathBuf::from(env));
    }
    let cwd = std::env::current_dir()?;
    match kq_index::discover::find_upward(&cwd) {
        Some(p) => Ok(Target::Install(p)),
        None => Err(NoInstall(
            "no KotOR installation found. Pass --install <path>, set KQ_INSTALL, \
             or run kq from inside an install."
                .to_string(),
        )
        .into()),
    }
}

fn resolve_explicit(p: &Path) -> Result<Target> {
    if kq_index::discover::is_install_root(p) {
        return Ok(Target::Install(p.to_path_buf()));
    }
    // A directory inside an install (`-i .` from Override/) means the whole
    // install, matching how the cwd fallback behaves. A named *file* never
    // gets this treatment — pointing at `modules/danm13.mod` means that one
    // capsule even when it happens to sit inside an install.
    if p.is_dir() {
        if let Some(found) = kq_index::discover::find_upward(p) {
            return Ok(Target::Install(found));
        }
    }
    if p.exists() {
        return Ok(Target::Standalone(p.to_path_buf()));
    }
    Err(NoInstall(format!("{} does not exist", p.display())).into())
}

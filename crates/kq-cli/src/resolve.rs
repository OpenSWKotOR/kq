//! Turning "where is the install" into an answer.
//!
//! In order: an explicit `--install`, then `KQ_INSTALL`, then an upward walk
//! from the working directory. The upward walk is what makes `kq` usable from
//! inside `Override/` without repeating the path every time.

use std::fmt;
use std::path::PathBuf;

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

pub fn resolve_install(explicit: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if !kq_index::discover::is_install_root(p) {
            // Accept a path inside the install too — it is the same intent.
            if let Some(found) = kq_index::discover::find_upward(p) {
                return Ok(found);
            }
            return Err(NoInstall(format!(
                "{} is not a KotOR installation (no chitin.key)",
                p.display()
            ))
            .into());
        }
        return Ok(p.clone());
    }
    if let Some(env) = std::env::var_os("KQ_INSTALL") {
        let p = PathBuf::from(env);
        if kq_index::discover::is_install_root(&p) {
            return Ok(p);
        }
        return Err(NoInstall(format!(
            "KQ_INSTALL={} is not a KotOR installation (no chitin.key)",
            p.display()
        ))
        .into());
    }
    let cwd = std::env::current_dir()?;
    match kq_index::discover::find_upward(&cwd) {
        Some(p) => Ok(p),
        None => Err(NoInstall(
            "no KotOR installation found. Pass --install <path>, set KQ_INSTALL, \
             or run kq from inside an install."
                .to_string(),
        )
        .into()),
    }
}

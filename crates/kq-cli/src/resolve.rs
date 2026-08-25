//! Turning "where is the install" into an answer.
//!
//! In order: an explicit `--install`, then `KQ_INSTALL`, then an upward walk
//! from the working directory. The upward walk is what makes `kq` usable from
//! inside `Override/` without repeating the path every time.

use std::path::PathBuf;

use anyhow::{bail, Result};

pub fn resolve_install(explicit: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if !kq_index::discover::is_install_root(p) {
            // Accept a path inside the install too — it is the same intent.
            if let Some(found) = kq_index::discover::find_upward(p) {
                return Ok(found);
            }
            bail!("{} is not a KotOR installation (no chitin.key)", p.display());
        }
        return Ok(p.clone());
    }
    if let Some(env) = std::env::var_os("KQ_INSTALL") {
        let p = PathBuf::from(env);
        if kq_index::discover::is_install_root(&p) {
            return Ok(p);
        }
        bail!("KQ_INSTALL={} is not a KotOR installation (no chitin.key)", p.display());
    }
    let cwd = std::env::current_dir()?;
    match kq_index::discover::find_upward(&cwd) {
        Some(p) => Ok(p),
        None => bail!(
            "no KotOR installation found. Pass --install <path>, set KQ_INSTALL, \
             or run kq from inside an install."
        ),
    }
}

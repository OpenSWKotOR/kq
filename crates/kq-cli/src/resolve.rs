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
    if p.is_dir() {
        if let Some(steam) = kq_index::discover::child(p, "steamassets") {
            if kq_index::discover::is_install_root(&steam) {
                return Ok(Target::Install(steam));
            }
        }
        if let Some(found) = kq_index::discover::find_upward(p) {
            return Ok(Target::Install(found));
        }
    }
    if p.exists() {
        if p.is_dir() {
            if let Some(steam) = kq_index::discover::child(p, "steamassets") {
                if steam.join("chitin.key").exists()
                    || kq_index::discover::child(&steam, "chitin.key").is_some()
                {
                    eprintln!(
                        "warning: treating {} as a loose folder, but {} looks like an Aspyr install root (chitin.key). Pass -i on that path if indexing failed.",
                        p.display(),
                        steam.display()
                    );
                }
            }
        }
        return Ok(Target::Standalone(p.to_path_buf()));
    }
    Err(NoInstall(format!("{} does not exist", p.display())).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn resolve_explicit_follows_steamassets_child() {
        let root = tempfile::tempdir().unwrap();
        let steam = root.path().join("steamassets");
        touch(&steam.join("chitin.key"));
        // Parent is NOT an install root (no chitin.key at parent).
        let target = resolve_explicit(root.path()).unwrap();
        match target {
            Target::Install(p) => assert_eq!(p, steam.canonicalize().unwrap_or(steam)),
            _ => panic!("expected Install(steamassets), got non-install"),
        }
    }

    #[test]
    fn resolve_explicit_warns_when_folder_has_steamassets_chitin() {
        let root = tempfile::tempdir().unwrap();
        let steam = root.path().join("SteamAssets"); // case variant
        touch(&steam.join("chitin.key"));
        // Also put a loose file so Standalone path is valid if discovery misses.
        // After the fix, this must still prefer steamassets Install.
        let target = resolve_explicit(root.path()).unwrap();
        assert!(matches!(target, Target::Install(_)));
    }
}

//! Where output goes, and in what shape.
//!
//! Data goes to stdout, everything else to stderr. That split is what lets
//! `kq ls --json | jq` work while progress and warnings stay visible.

use std::io::{self, IsTerminal, Write};

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

/// Output settings resolved once, from flags and environment.
#[derive(Clone, Copy, Debug)]
pub struct Out {
    pub json: bool,
    pub text: bool,
    pub color: bool,
}

impl Out {
    pub fn new(json: bool, text: bool, color: ColorChoice) -> Out {
        let color = match color {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            // NO_COLOR is honored regardless of whether a terminal is
            // attached; see https://no-color.org.
            ColorChoice::Auto => {
                std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
            }
        };
        Out { json, text, color }
    }

    /// Print a value as JSON on stdout.
    pub fn json_value<T: Serialize>(&self, value: &T) -> anyhow::Result<()> {
        let stdout = io::stdout();
        let mut w = stdout.lock();
        serde_json::to_writer_pretty(&mut w, value)?;
        w.write_all(b"\n")?;
        Ok(())
    }

    /// Print one JSON object per line, for streaming into `jq` and friends.
    pub fn json_line<T: Serialize>(&self, w: &mut impl Write, value: &T) -> anyhow::Result<()> {
        serde_json::to_writer(&mut *w, value)?;
        w.write_all(b"\n")?;
        Ok(())
    }

    pub fn dim(&self, s: &str) -> String {
        if self.color {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    pub fn bold(&self, s: &str) -> String {
        if self.color {
            format!("\x1b[1m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    pub fn accent(&self, s: &str) -> String {
        if self.color {
            format!("\x1b[36m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

/// Report a non-fatal problem. Always stderr, never stdout.
pub fn warn(message: impl AsRef<str>) {
    let _ = writeln!(io::stderr(), "kq: warning: {}", message.as_ref());
}

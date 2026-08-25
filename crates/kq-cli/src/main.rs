//! `kq` — query a KotOR installation like it was plain text.

mod exit;
mod filter;
mod glob;
mod output;
mod read;
mod render;
mod resolve;

mod live;

mod cmd {
    pub mod cache;
    pub mod cat;
    pub mod grep;
    pub mod info;
    pub mod graph;
    pub mod leftovers;
    pub mod ls;
    pub mod unused;
    pub mod which;
}

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use output::{ColorChoice, Out};

/// Shared context every subcommand gets: where the install is, how to print,
/// and whether the cache may be used.
pub struct Ctx {
    pub install: Option<PathBuf>,
    pub out: Out,
    pub use_cache: bool,
    pub refresh: bool,
}

impl Ctx {
    /// Open the target — an installation, or a standalone capsule/folder/file
    /// — and get its index.
    pub fn index(&self) -> anyhow::Result<kq_index::Index> {
        let (index, freshness) = match resolve::resolve_target(self.install.as_ref())? {
            resolve::Target::Install(root) => {
                // --refresh skips the read but still writes, so the rebuilt
                // index replaces the stale cache instead of being discarded
                // after use.
                let read_cache = self.use_cache && !self.refresh;
                let write_cache = self.use_cache;
                let (_install, index, freshness) = kq_index::open(&root, read_cache, write_cache)?;
                (index, freshness)
            }
            resolve::Target::Standalone(path) => (
                kq_index::open_standalone(&path)?,
                kq_index::Freshness::Built,
            ),
        };
        if freshness == kq_index::Freshness::Built && self.refresh {
            // Nothing to say on a plain cold run; only confirm an explicit
            // --refresh actually rebuilt.
            output::warn(format!("rebuilt index for {}", index.root.display()));
        }
        for w in &index.warnings {
            output::warn(w);
        }
        Ok(index)
    }
}

#[derive(Parser)]
#[command(
    name = "kq",
    version,
    about = "Query a KotOR installation like it was plain text.",
    long_about = "kq reads a KotOR installation — its archives, modules and \
loose files — and answers questions about it.\n\n\
Point it at an install with --install, set KQ_INSTALL, or run it from inside \
one. Every command takes --json.",
    disable_help_subcommand = true,
    propagate_version = true
)]
struct Cli {
    /// Path to the KotOR installation.
    #[arg(
        short = 'i',
        long,
        global = true,
        value_name = "PATH",
        env = "KQ_INSTALL"
    )]
    install: Option<PathBuf>,

    /// Emit JSON instead of text.
    #[arg(long, global = true)]
    json: bool,

    /// When to colorize output.
    #[arg(long, global = true, value_name = "WHEN", default_value = "auto")]
    color: ColorChoice,

    /// Ignore any cached index and do not write one.
    #[arg(long, global = true)]
    no_cache: bool,

    /// Rebuild the index even if a valid cache exists.
    #[arg(long, global = true)]
    refresh: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Summarize the installation.
    Info(cmd::info::Args),
    /// List resources.
    #[command(visible_alias = "list")]
    Ls(cmd::ls::Args),
    /// Show every copy of a resource, in the order the game resolves them.
    Which(cmd::which::Args),
    /// Print a resource.
    Cat(cmd::cat::Args),
    /// Search resource contents as text.
    Grep(cmd::grep::Args),
    /// Inspect or clear the index cache.
    Cache(cmd::cache::Args),
    /// List leftover resources the live graph never reaches.
    Unused(cmd::unused::Args),
    /// Live mention hierarchy and leftovers in one report.
    Graph(cmd::graph::Args),
    /// Catalog every ResRef and talk-table row, then list what the live graph never reaches.
    #[command(visible_alias = "leftover")]
    Leftovers(cmd::leftovers::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let ctx = Ctx {
        install: cli.install,
        out: Out::new(cli.json, cli.color),
        use_cache: !cli.no_cache,
        refresh: cli.refresh,
    };

    let result = match cli.command {
        Command::Info(a) => cmd::info::run(&ctx, a),
        Command::Ls(a) => cmd::ls::run(&ctx, a),
        Command::Which(a) => cmd::which::run(&ctx, a),
        Command::Cat(a) => cmd::cat::run(&ctx, a),
        Command::Grep(a) => cmd::grep::run(&ctx, a),
        Command::Cache(a) => cmd::cache::run(&ctx, a),
        Command::Unused(a) => cmd::unused::run(&ctx, a),
        Command::Graph(a) => cmd::graph::run(&ctx, a),
        Command::Leftovers(a) => cmd::leftovers::run(&ctx, a),
    };

    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            // A broken pipe means the reader went away — `kq ls | head` is a
            // normal thing to do and should not print an error.
            if is_broken_pipe(&e) {
                return ExitCode::from(exit::OK as u8);
            }
            eprintln!("kq: {e:#}");
            let code = if e.downcast_ref::<resolve::NoInstall>().is_some() {
                exit::NO_INSTALL
            } else {
                exit::FAILURE
            };
            ExitCode::from(code as u8)
        }
    }
}

/// Split `name.ext` into a ResRef and an optional type.
///
/// A trailing component is only treated as an extension when it names a real
/// resource type, so a ResRef that legitimately contains a dot is not
/// truncated.
pub fn parse_ref(
    input: &str,
    explicit: Option<&str>,
) -> anyhow::Result<(String, Option<kq_format::ResType>)> {
    let (name, from_name) = match input.rsplit_once('.') {
        Some((base, ext)) if kq_format::ResType::from_extension(ext).is_some() => {
            (base.to_string(), Some(ext.to_string()))
        }
        _ => (input.to_string(), None),
    };
    let ext = explicit.map(str::to_string).or(from_name);
    let want = match ext.as_deref() {
        Some(e) => match kq_format::ResType::from_extension(e) {
            Some(t) => Some(t),
            None => anyhow::bail!("unknown resource type: {e}"),
        },
        None => None,
    };
    Ok((name.to_ascii_lowercase(), want))
}

fn is_broken_pipe(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
    })
}

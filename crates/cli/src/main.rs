//! # archguard
//!
//! CLI entry point. Future subcommands: `check`, `ack`, `upgrade`, `merge`.
//! Phase 0: only `--version` / `--help`.

use clap::Parser;

/// Guard Arch Linux upgrades against unread, relevant Arch news.
#[derive(Debug, Parser)]
#[command(name = "archguard", version, about)]
pub struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!("archguard {} (phase 0 skeleton)", archguard_core::version());
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn version_matches_cargo_pkg_version() {
        let cmd = Cli::command();
        assert_eq!(cmd.get_version(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(archguard_core::version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn cli_debug_assert() {
        Cli::command().debug_assert();
    }
}

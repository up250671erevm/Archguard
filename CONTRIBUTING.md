# Contributing / Contribuir

1. Fork, create a branch (`fN-short-topic`).
2. Before pushing, run:
   ```sh
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace --locked
   ```
3. Keep `Cargo.lock` committed and updated.
4. Open a PR against `main`; CI (incl. an `archlinux:latest` job) must pass.
5. Never make archguard require running entirely as root; elevate single commands only.

Contributions are licensed under GPL-3.0-or-later. Issues and PRs in Spanish or English are welcome.

# ZTOP — assistant instructions

- **Versioning:** use monotonically increasing `MAJOR.MINOR.PATCH` release
  numbers with UTC `+YYYYMMDD.HHMM` build metadata. Metadata identifies a
  build and does not affect ordering. Prepare a new release with
  `scripts/version.sh VERSION "change summary"`; run
  `scripts/version.sh --check` before shipping. See README §Versioning.
- **Ownership (MANDATORY):** the entire folder — every directory and file,
  including `.git/`, docs, scripts, tests and anything a session creates — must
  always be owned by `spabam:spabam`. Sessions may run as root, but the project
  belongs to user `spabam`, who runs `./installer.sh` at any moment — even
  mid-turn. Three rules:
  1. **NEVER build into the repo's `target/` as root.** Export
     `CARGO_TARGET_DIR=/root/.cache/ztop-target` before every cargo command
     so build artifacts land outside the repo.
  2. After any commit/write, run
     `chown -R spabam:spabam /home/spabam/Dropbox/Public/VANTAGE/AI/ZTOP`
     (background git operations rewrite `.git/index` as root, so do this at
     the END of the turn, not only after explicit git commands).
  3. Before ending the turn verify with
     `find /home/spabam/Dropbox/Public/VANTAGE/AI/ZTOP -not -user spabam`
     (it must print nothing). Never leave a root-owned file behind, not even
     inside `.git/`. `./installer.sh` restores ownership of the whole tree at
     the start and end of every mutating run, but that is a safety net, not a
     licence to leave root-owned files around.
- **Toolchain:** rustc is pinned to 1.88 (`rust-toolchain.toml`); `cargo` may
  need `export PATH="$HOME/.cargo/bin:$PATH"`. Keep
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo test` green — CI enforces all three. sysinfo must stay ≤ 0.38
  (0.39+ needs rustc 1.95).
- **Perf guard:** process refresh must stay minimal
  (`ProcessRefreshKind::nothing().without_tasks()` + cpu/memory/user-once/cmd-once);
  sysinfo defaults cost ~26% of a core vs ~2% (see CHANGELOG 0.4.0).
- Headless verification without a TTY:
  `$CARGO_TARGET_DIR/release/ztop --json --top 5`.

# Sisu

Sisu is a small, statically typed language compiled to native x86-64 Linux code through LLVM 22. This Cargo workspace holds two crates:

- `crates/sisuc`: the compiler (`sisuc`).
- `crates/runtime`: `sisu-runtime`, a `staticlib` with a C ABI that is linked into every compiled program.

## Commands

| Task | Command |
|---|---|
| Build | `cargo build --workspace` |
| Test | `cargo nextest run --workspace` (not `cargo test`: a nextest setup script builds the runtime) |
| Fill the expected `.out`/`.err`/`.json` files of the end-to-end cases (see `docs/development.md`) | `SISU_BLESS=1 cargo nextest run -E 'binary(cases)'` |
| Lint and format check (the same hooks CI runs) | `prek run --all-files` |
| Dependency audit | `cargo deny check` |
| Mutation test a branch's changes (on a stacked branch, pass the stage's base) | `scripts/mutants.sh [base]` |
| Coverage of a branch's changes (on a stacked branch, pass the stage's base) | `scripts/coverage.sh [base]` |
| Count the instructions of a compiler run under callgrind, which CodSpeed's simulation mode also uses | `scripts/codspeed-prep.sh`, then `valgrind --tool=callgrind --callgrind-out-file=/dev/null target/release/sisuc --check target/codspeed/large/main.sisu` |

Once per clone, run `scripts/install-llvm.sh`, `prek install`, `cargo install --locked cargo-mutants` and `cargo install --locked cargo-llvm-cov`. The script installs LLVM 22 into `.llvm/22`; `.cargo/config.toml` sets `LLVM_SYS_221_PREFIX` to it. Do not build against a system LLVM. The `prek install` step enables the Git pre-commit hook. The toolchain is pinned in `rust-toolchain.toml`.

## Code rules

- Lint policy lives in the root `Cargo.toml` under `[workspace.lints]`. Every crate opts in with `[lints] workspace = true`. CI fails on any warning.
- Clippy enforces the code rules (`#[expect]` with a reason, no `unwrap()` outside tests, `// SAFETY:` comments). Fix the code to satisfy a lint; the lint table changes only with the user's agreement.
- Reviews: apply `CODING_STANDARDS.md`.
- Write code to the rules in `CODING_STANDARDS.md` as well as reviewing against them; its Performance section covers allocation and measurement.

## Open-source facts: use GitHits, do not guess

Check every fact about an open-source dependency, tool or GitHub Action (`inkwell`, `llvm-sys`, LLVM, `cargo-nextest`, `cargo-deny`, `prek`, `.github/workflows/`) with the GitHits tools, at the version in `Cargo.lock` or the pinned SHA; the `githits-mcp` skill says which tool to use. Cite what you used and name what you could not verify. If GitHits is unavailable, say so and read the pinned source in `~/.cargo/registry/src/`.

## Agent skills

### Issue tracker

Issues live as local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-role vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`), recorded as `Status:` lines. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Docs

Before you open a pull request, run the `update-docs` skill to bring the docs in line with the code. After a merge, run it only when the user asks or the pull request skipped it. Skills live in `.agents/skills/`, where Codex finds them; `.claude/skills/` symlinks each one for Claude Code. An agent without skill support follows `.agents/skills/update-docs/SKILL.md` directly.

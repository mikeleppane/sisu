# Sisu

Sisu is a small, statically typed language compiled to native x86-64 Linux code through LLVM 22. This Cargo workspace holds two crates:

- `crates/sisuc`: the compiler (`sisuc`).
- `crates/runtime`: `sisu-runtime`, a `staticlib` with a C ABI that is linked into every compiled program.

## Commands

| Task | Command |
|---|---|
| Build | `cargo build --workspace` |
| Test | `cargo nextest run --workspace` |
| Lint and format check (the same hooks CI runs) | `prek run --all-files` |
| Dependency audit | `cargo deny check` |

Once per clone, run `scripts/install-llvm.sh` and `prek install`. The script installs LLVM 22 into `.llvm/22`; `.cargo/config.toml` sets `LLVM_SYS_221_PREFIX` to it. Do not build against a system LLVM. The `prek install` step enables the Git pre-commit hook. The toolchain is pinned in `rust-toolchain.toml`.

## Code rules

- Lint policy lives in the root `Cargo.toml` under `[workspace.lints]`. Every crate opts in with `[lints] workspace = true`. CI fails on any warning.
- Clippy enforces the code rules (`#[expect]` with a reason, no `unwrap()` outside tests, `// SAFETY:` comments). Fix the code to satisfy a lint; the lint table changes only with the user's agreement.
- Reviews: apply `CODING_STANDARDS.md`.

## Open-source facts: use GitHits, do not guess

Before you use, change or explain anything about an open-source library, crate, tool or GitHub Action, check it with the GitHits MCP tools. Do not rely on memory, and do not rely on documentation that may describe a different version. This applies to APIs, behavior, configuration keys, CLI flags, versions, licenses and advisories. It covers `inkwell`, `llvm-sys`, LLVM itself, `cargo-nextest`, `cargo-deny`, `prek` and every GitHub Action in `.github/workflows/`.

- Code, at the version in `Cargo.lock` or the pinned SHA: use `search`, `grep` and `read` to inspect source, symbols and docs. Read the source before you explain a behavior or debug an error that comes from a dependency.
- Package intelligence: use `pkg_info`, `pkg_vulns`, `pkg_deps` and `pkg_changelog` before you add a dependency. Use `pkg_upgrade_review` before you bump one.
- Examples: use `get_example` to find implementation patterns in other projects. Then check each API it uses against our pinned versions.
- Cite what you used. Name any assumption you could not verify.

If GitHits is unavailable, say so. Then check the source of the exact pinned version, for example in `~/.cargo/registry/src/`, before you continue.

## Agent skills

### Issue tracker

Issues live as local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-role vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`), recorded as `Status:` lines. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Docs

Before you open a pull request, run the `update-docs` skill to bring the docs in line with the code. After a merge, run it only when the user asks or the pull request skipped it. An agent without skill support follows `.claude/skills/update-docs/SKILL.md` directly.

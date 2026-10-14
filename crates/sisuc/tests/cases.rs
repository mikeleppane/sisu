//! One test per file in `tests/run`, `tests/run-fail` and `tests/diagnostics`. A `run` case exits
//! 0 and prints its `.out`, and its `.err` on stderr when it has one, else nothing there; a
//! `run_fail` case panics with its `.out` on stdout and its `.err` on stderr; a `diagnostics` case makes `sisuc --check` report its `.err` on stderr, and
//! `sisuc --format json --check` its `.json`. Every
//! file in these directories must belong to a case, as a file its mode reads, and none may be
//! hidden.
//!
//! `SISU_BLESS=1` rewrites the `.out`, `.err` and `.json` files of passing cases. It never creates one.

use std::path::Path;

mod support;

/// Whether `SISU_BLESS=1` is set.
fn bless() -> bool {
    std::env::var("SISU_BLESS").as_deref() == Ok("1")
}

// datatest-stable names each test `<function>::<path under root>`, so these function names are
// the `run::` and `run_fail::` prefixes that nextest filters match.

#[expect(
    clippy::unnecessary_wraps,
    reason = "datatest-stable accepts only test functions that return a `Result`"
)]
fn run(path: &Path) -> datatest_stable::Result<()> {
    support::run(path, bless());
    Ok(())
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "datatest-stable accepts only test functions that return a `Result`"
)]
fn run_fail(path: &Path) -> datatest_stable::Result<()> {
    support::run_fail(path, bless());
    Ok(())
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "datatest-stable accepts only test functions that return a `Result`"
)]
fn run_orphans(path: &Path) -> datatest_stable::Result<()> {
    support::orphan_check(path, &["out", "err"]);
    Ok(())
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "datatest-stable accepts only test functions that return a `Result`"
)]
fn run_fail_orphans(path: &Path) -> datatest_stable::Result<()> {
    support::orphan_check(path, &["out", "err"]);
    Ok(())
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "datatest-stable accepts only test functions that return a `Result`"
)]
fn diagnostics(path: &Path) -> datatest_stable::Result<()> {
    support::diagnostics(path, bless());
    Ok(())
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "datatest-stable accepts only test functions that return a `Result`"
)]
fn diagnostics_orphans(path: &Path) -> datatest_stable::Result<()> {
    support::orphan_check(path, &["err", "json"]);
    Ok(())
}

datatest_stable::harness! {
    { test = run, root = "tests/run", pattern = r"\.sisu$" },
    { test = run_fail, root = "tests/run-fail", pattern = r"\.sisu$" },
    { test = run_orphans, root = "tests/run" },
    { test = run_fail_orphans, root = "tests/run-fail" },
    { test = diagnostics, root = "tests/diagnostics", pattern = r"\.sisu$" },
    { test = diagnostics_orphans, root = "tests/diagnostics" },
}

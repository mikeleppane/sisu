//! One test per file in `tests/run` and `tests/run-fail`. A `run` case exits 0 and prints its
//! `.out`; a `run_fail` case panics with its `.out` on stdout and its `.err` on stderr. Every
//! file in either directory must belong to a case.
//!
//! `SISU_BLESS=1` rewrites the `.out` and `.err` files of passing builds. It never creates one.

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
    support::orphan_check(path);
    Ok(())
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "datatest-stable accepts only test functions that return a `Result`"
)]
fn run_fail_orphans(path: &Path) -> datatest_stable::Result<()> {
    support::orphan_check(path);
    Ok(())
}

datatest_stable::harness! {
    { test = run, root = "tests/run", pattern = r"\.sisu$" },
    { test = run_fail, root = "tests/run-fail", pattern = r"\.sisu$" },
    { test = run_orphans, root = "tests/run" },
    { test = run_fail_orphans, root = "tests/run-fail" },
}

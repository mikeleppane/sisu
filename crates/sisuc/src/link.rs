//! Links an object file with the Sisu runtime into an executable. The system C
//! compiler drives the linker, so it also adds the C startup code and libc.

use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn link(object: &Path, output: &Path) -> Result<(), String> {
    let runtime = runtime_library()?;
    // `output`, not `status`: cc's messages go into the tool error, so stderr stays one format.
    let out = Command::new("cc")
        .arg(object)
        .arg(&runtime)
        // libm, after the runtime: `frem` lowers to a call to `fmod`.
        .arg("-lm")
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|e| format!("cannot run cc: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let message = format!("cc failed: {}", out.status);
    match String::from_utf8_lossy(&out.stderr).trim() {
        "" => Err(message),
        detail => Err(format!("{message}\n{detail}")),
    }
}

/// Cargo builds `libsisu_runtime.a` into the same directory as `sisuc`.
// ponytail: only works from a Cargo target directory; add a search path once sisuc is installed elsewhere.
fn runtime_library() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate sisuc: {e}"))?;
    let path = exe.with_file_name("libsisu_runtime.a");
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "runtime library not found at {}; build it with `cargo build --workspace`",
            path.display()
        ))
    }
}

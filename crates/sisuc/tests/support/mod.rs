//! Helpers shared by the end-to-end test binaries.

use std::path::Path;
use std::process::{Command, Output};

/// `sisuc [-O2] <input> <output>`, run from `dir`, after asserting the runtime archive exists.
pub(crate) fn sisuc(dir: &Path, input: &str, output: &Path, o2: bool) -> Command {
    let sisuc = Path::new(env!("CARGO_BIN_EXE_sisuc"));
    assert!(
        sisuc.with_file_name("libsisu_runtime.a").exists(),
        "libsisu_runtime.a is missing: run the tests with `cargo nextest run`, whose setup \
         script builds it, or run `cargo build -p sisu-runtime`"
    );
    let mut command = Command::new(sisuc);
    command.current_dir(dir);
    if o2 {
        command.arg("-O2");
    }
    command.arg(input).arg(output);
    command
}

/// Compiles `<dir>/<name>.sisu`, run from `dir`, into `exe`, at `-O2` if `o2`.
pub(crate) fn compile(dir: &Path, name: &str, exe: &Path, o2: bool) {
    let out = sisuc(dir, &format!("{name}.sisu"), exe, o2)
        .output()
        .expect("sisuc starts");
    assert_eq!(
        out.status.code(),
        Some(0),
        "sisuc failed on {name}.sisu:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Runs `exe` under Valgrind; any leak or memory error makes it exit 1.
pub(crate) fn valgrind(exe: &Path) -> Output {
    Command::new("valgrind")
        .args([
            "--leak-check=full",
            "--errors-for-leak-kinds=definite,indirect,possible",
            "--error-exitcode=1",
        ])
        .arg(exe)
        .output()
        .expect("valgrind is required: install it with `apt install valgrind`")
}

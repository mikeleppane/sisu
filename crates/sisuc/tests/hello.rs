//! End to end: `sisuc` compiles a program, and the program prints the right output.

use std::path::Path;
use std::process::Command;

#[test]
fn print_int_42() {
    // `cargo nextest run` does not build the runtime archive that `sisuc` links
    // against, so build it here. This is a no-op when it is up to date.
    let built = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "-p", "sisu-runtime"])
        .status()
        .expect("cargo starts");
    assert!(built.success(), "building sisu-runtime failed");

    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join("print_int_42");
    let compiled = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .arg(&exe)
        .status()
        .expect("sisuc starts");
    assert!(compiled.success(), "sisuc failed");

    let run = Command::new(&exe).output().expect("the program starts");
    assert!(run.status.success());
    assert_eq!(String::from_utf8_lossy(&run.stdout), "42\n");
}

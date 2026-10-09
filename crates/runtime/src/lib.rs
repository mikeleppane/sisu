//! The Sisu runtime: functions that compiled Sisu programs call over the C ABI.

use std::io::Write;

/// Prints `value` and a newline to standard output. Sisu's `print` calls this for an `i64`
/// argument.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_print_int(value: i64) {
    println!("{value}");
}

/// Prints `true` or `false` and a newline to standard output. Sisu's `print` calls this
/// for a `bool` argument.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_print_bool(value: bool) {
    println!("{value}");
}

/// Prints `sisu: panic at <msg>` to standard error and exits with code 101.
///
/// Standard output is a line-buffered `LineWriter` and `exit` flushes it, so lines
/// printed before the panic are not lost.
///
/// # Safety
///
/// - `msg` is non-null, even when `len` is 0.
/// - The `len` bytes starting at `msg` are readable and lie within one allocation.
/// - Nothing writes to those bytes for the duration of the call.
/// - `len` is at most `isize::MAX`.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name and passes a raw pointer"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_panic(msg: *const u8, len: usize) -> ! {
    // SAFETY: the caller upholds the `# Safety` contract above, which is exactly what
    // `from_raw_parts` requires for a `u8` slice.
    let bytes = unsafe { std::slice::from_raw_parts(msg, len) };
    // `eprintln!` panics on a failed write, which would abort before exit code 101.
    let _ = writeln!(
        std::io::stderr(),
        "sisu: panic at {}",
        String::from_utf8_lossy(bytes)
    );
    std::process::exit(101)
}

//! The Sisu runtime: functions that compiled Sisu programs call over the C ABI.

/// Prints `value` and a newline to standard output. Sisu's `print_int` calls this.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_print_int(value: i64) {
    println!("{value}");
}

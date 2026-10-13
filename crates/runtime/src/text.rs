//! Printing text: `char` now, strings with their own functions later.

/// Prints the `char` with scalar value `c` and a newline to standard output. Sisu's `print`
/// calls this for a `char` argument; generated code passes only scalar values, so the
/// `from_u32` arm that aborts is never reached.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_print_char(c: u32) {
    match char::from_u32(c) {
        Some(c) => crate::print_line(format_args!("{c}")),
        None => std::process::abort(),
    }
}

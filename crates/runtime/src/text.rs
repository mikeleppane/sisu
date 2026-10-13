//! Printing and comparing text: `char`s, and strings as bytes with a length.

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

/// Prints the `len` bytes at `bytes` and a newline to standard output. Sisu's `print` calls
/// this for a `string` argument. It writes the bytes as they are, zero bytes included.
///
/// # Safety
///
/// `len` bytes at `bytes` are readable; valid UTF-8 is not required.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_print_str(bytes: *const u8, len: usize) {
    // SAFETY: the caller promises `len` readable bytes at `bytes`; a string's bytes pointer
    // is never null, even for the empty string.
    crate::print_bytes(unsafe { std::slice::from_raw_parts(bytes, len) });
}

/// Compares the `a_len` bytes at `a` with the `b_len` bytes at `b`, as byte slices' `Ord`
/// does: -1 when `a` sorts first, 0 when they are equal, 1 when `b` sorts first. For valid
/// UTF-8 this is code-point order.
///
/// # Safety
///
/// `a_len` bytes at `a` and `b_len` bytes at `b` are readable.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_str_compare(
    a: *const u8,
    a_len: usize,
    b: *const u8,
    b_len: usize,
) -> i64 {
    // SAFETY: the caller promises that both ranges are readable; neither pointer is null.
    let (a, b) = unsafe {
        (
            std::slice::from_raw_parts(a, a_len),
            std::slice::from_raw_parts(b, b_len),
        )
    };
    a.cmp(b) as i64
}

#[cfg(test)]
mod tests {
    use super::sisu_str_compare;

    fn compare(a: &[u8], b: &[u8]) -> i64 {
        // SAFETY: both slices are readable for their lengths.
        #[expect(unsafe_code, reason = "calls the runtime's C ABI function")]
        unsafe {
            sisu_str_compare(a.as_ptr(), a.len(), b.as_ptr(), b.len())
        }
    }

    /// The result is -1, 0 or 1, by the bytes, zero bytes included, a prefix sorting first.
    #[test]
    fn str_compare_orders_bytes() {
        let rows: [(&[u8], &[u8], i64); 6] = [
            (b"a", b"b", -1),
            (b"b", b"a", 1),
            (b"abc", b"abc", 0),
            (b"", b"a", -1),
            (b"a\0b", b"a\0c", -1),
            (b"a\0b", b"a", 1),
        ];
        for (a, b, want) in rows {
            assert_eq!(compare(a, b), want, "{a:?} against {b:?}");
        }
    }
}

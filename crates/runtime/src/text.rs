//! Printing, writing and comparing text: `char`s, and strings as bytes with a length. Each
//! type has one formatter, which both `print` and interpolation use.

use std::io::Write;

/// The longest text of an `i64`: `-9223372036854775808`.
const INT_BYTES: usize = 20;

/// `value` in decimal, in `buffer`.
pub(crate) fn int_text(value: i64, buffer: &mut [u8; INT_BYTES]) -> &[u8] {
    let mut rest = &mut buffer[..];
    write!(rest, "{value}").expect("20 bytes should hold every `i64`");
    let len = INT_BYTES - rest.len();
    &buffer[..len]
}

/// `true` or `false`.
pub(crate) fn bool_text(value: bool) -> &'static [u8] {
    if value { b"true" } else { b"false" }
}

/// The UTF-8 encoding of the `char` with scalar value `c`, in `buffer`. Generated code passes
/// only scalar values, so the arm that aborts is never reached.
fn char_text(c: u32, buffer: &mut [u8; 4]) -> &[u8] {
    match char::from_u32(c) {
        Some(c) => c.encode_utf8(buffer).as_bytes(),
        None => std::process::abort(),
    }
}

/// Prints the `char` with scalar value `c` and a newline to standard output. Sisu's `print`
/// calls this for a `char` argument.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_print_char(c: u32) {
    crate::print_bytes(char_text(c, &mut [0; 4]));
}

/// Copies `text` to `out` and returns its length.
///
/// # Safety
///
/// `out` has room for `text.len()` bytes, which do not overlap `text`.
#[expect(
    unsafe_code,
    reason = "writes through the pointer generated code passes"
)]
unsafe fn write(text: &[u8], out: *mut u8) -> usize {
    // SAFETY: the caller promises room for `text.len()` bytes at `out`, apart from `text`.
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), out, text.len()) };
    text.len()
}

/// Writes `value` in decimal, as `print` does, to `out`, and returns how many bytes it wrote:
/// at most 20. Interpolation calls this for an `i64` or a widened `u8`.
///
/// # Safety
///
/// `out` has room for the decimal text of `value`, at most 20 bytes.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_write_int(value: i64, out: *mut u8) -> usize {
    // SAFETY: the caller promises room for the text, at most the 20 bytes `int_text` gives.
    unsafe { write(int_text(value, &mut [0; INT_BYTES]), out) }
}

/// Writes `true` or `false`, as `print` does, to `out`, and returns how many bytes it wrote:
/// at most 5. Interpolation calls this for a `bool`.
///
/// # Safety
///
/// `out` has room for 5 bytes.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_write_bool(value: bool, out: *mut u8) -> usize {
    // SAFETY: the caller promises room for 5 bytes, the length of `false`.
    unsafe { write(bool_text(value), out) }
}

/// Writes the UTF-8 encoding of the `char` with scalar value `c`, as `print` does, to `out`,
/// and returns how many bytes it wrote: at most 4. Interpolation calls this for a `char`.
///
/// # Safety
///
/// `out` has room for 4 bytes.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_write_char(c: u32, out: *mut u8) -> usize {
    // SAFETY: the caller promises room for 4 bytes, the longest UTF-8 encoding.
    unsafe { write(char_text(c, &mut [0; 4]), out) }
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
    use super::{sisu_str_compare, sisu_write_bool, sisu_write_char, sisu_write_int};

    /// `i64::MIN` fills the 20 bytes that an `i64` may take; the others take fewer.
    #[test]
    fn write_int_extremes() {
        for (value, want) in [
            (i64::MIN, "-9223372036854775808"),
            (0, "0"),
            (i64::MAX, "9223372036854775807"),
        ] {
            let mut out = [0; 20];
            // SAFETY: `out` has room for 20 bytes.
            #[expect(unsafe_code, reason = "calls the runtime's C ABI function")]
            let len = unsafe { sisu_write_int(value, out.as_mut_ptr()) };
            assert_eq!(&out[..len], want.as_bytes());
        }
    }

    /// `false` fills the 5 bytes that a `bool` may take.
    #[test]
    fn write_bool_words() {
        for (value, want) in [(true, "true"), (false, "false")] {
            let mut out = [0; 5];
            // SAFETY: `out` has room for 5 bytes.
            #[expect(unsafe_code, reason = "calls the runtime's C ABI function")]
            let len = unsafe { sisu_write_bool(value, out.as_mut_ptr()) };
            assert_eq!(&out[..len], want.as_bytes());
        }
    }

    /// A `char` is written as its UTF-8 encoding, 1 to 4 bytes.
    #[test]
    fn write_char_widths() {
        for c in ['a', 'ä', '€', '😀'] {
            let mut out = [0; 4];
            // SAFETY: `out` has room for 4 bytes.
            #[expect(unsafe_code, reason = "calls the runtime's C ABI function")]
            let len = unsafe { sisu_write_char(u32::from(c), out.as_mut_ptr()) };
            assert_eq!(&out[..len], c.to_string().as_bytes());
        }
    }

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

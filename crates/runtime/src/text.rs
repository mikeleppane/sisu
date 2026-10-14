//! Printing, writing and comparing text: `char`s, and strings as bytes with a length. Each
//! type has one formatter, which both `print` and interpolation use.

use std::borrow::Cow;
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

/// `bytes` as valid UTF-8: borrowed when it is already, else with each maximal subpart of an
/// ill-formed sequence replaced by one `U+FFFD`, as `String::from_utf8_lossy` does.
pub(crate) fn repair(bytes: &[u8]) -> Cow<'_, [u8]> {
    match String::from_utf8_lossy(bytes) {
        Cow::Borrowed(_) => Cow::Borrowed(bytes),
        Cow::Owned(text) => Cow::Owned(text.into_bytes()),
    }
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

/// Writes the `char` with scalar value `c` and a newline to standard error. Sisu's `eprint`
/// calls this for a `char` argument.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_eprint_char(c: u32) {
    crate::eprint_bytes(char_text(c, &mut [0; 4]));
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

/// The `len` bytes at `bytes` as text. A Sisu string is valid UTF-8, so the arm that aborts is
/// never reached unless the compiler broke that invariant, which stops the program instead of
/// being undefined behavior.
///
/// # Safety
///
/// `bytes` is not null and `len` bytes at it are readable.
#[expect(
    unsafe_code,
    reason = "reads through the pointer generated code passes"
)]
unsafe fn utf8<'a>(bytes: *const u8, len: usize) -> &'a str {
    // SAFETY: the caller promises `len` readable bytes at `bytes`; a string's bytes pointer is
    // never null, even for the empty string.
    match core::str::from_utf8(unsafe { std::slice::from_raw_parts(bytes, len) }) {
        Ok(text) => text,
        Err(_) => std::process::abort(),
    }
}

/// The scalar values at `chars` as text, which are the `n` values of a `[char]`. Generated code
/// holds only scalar values there, so the arm that aborts is never reached.
///
/// # Safety
///
/// `chars` is not null and aligned, and `n` `u32`s at it are readable.
#[expect(
    unsafe_code,
    reason = "reads through the pointer generated code passes"
)]
unsafe fn scalars<'a>(chars: *const u32, n: usize) -> impl Iterator<Item = char> + 'a {
    // SAFETY: the caller promises `n` readable `u32`s at `chars`; an array's elements pointer
    // is never null, even for the empty array.
    let values = unsafe { std::slice::from_raw_parts(chars, n) };
    values
        .iter()
        .map(|&c| char::from_u32(c).unwrap_or_else(|| std::process::abort()))
}

/// How many code points the `len` bytes at `bytes` hold, which is the length of the `[char]`
/// that `chars()` returns.
///
/// # Safety
///
/// `bytes` is not null and `len` bytes at it are readable. They are valid UTF-8; if they are
/// not, the program aborts.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_utf8_count(bytes: *const u8, len: usize) -> usize {
    // SAFETY: the caller promises `len` readable bytes at `bytes`.
    unsafe { utf8(bytes, len) }.chars().count()
}

/// Writes the code points of the `len` bytes at `bytes` to `out`, as many as `sisu_utf8_count`
/// counts for them. `chars()` calls this to fill the array it has sized.
///
/// # Safety
///
/// `bytes` is not null and `len` bytes at it are readable and valid UTF-8; if they are not,
/// the program aborts. `out` is aligned and has room for `sisu_utf8_count(bytes, len)` `u32`s,
/// apart from `bytes`.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_utf8_decode(bytes: *const u8, len: usize, out: *mut u32) {
    // SAFETY: the caller promises `len` readable bytes at `bytes`.
    let text = unsafe { utf8(bytes, len) };
    for (i, c) in text.chars().enumerate() {
        // SAFETY: the caller promises room for one `u32` per code point, and `i` is below
        // their count.
        unsafe { out.add(i).write(u32::from(c)) };
    }
}

/// How many bytes the `n` scalar values at `chars` encode to, which is the length of the string
/// that `string(cs)` returns.
///
/// # Safety
///
/// `chars` is not null and aligned, and `n` `u32`s at it are readable. They are scalar values,
/// which a `[char]` holds; if one is not, the program aborts.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_utf8_len(chars: *const u32, n: usize) -> usize {
    // SAFETY: the caller promises `n` readable `u32`s at `chars`.
    unsafe { scalars(chars, n) }.map(char::len_utf8).sum()
}

/// Writes the UTF-8 encoding of the `n` scalar values at `chars` to `out`, as many bytes as
/// `sisu_utf8_len` gives for them. `string(cs)` calls this to fill the string it has sized.
///
/// # Safety
///
/// `chars` is not null and aligned, and `n` `u32`s at it are readable and scalar values; if
/// one is not, the program aborts. `out` has room for `sisu_utf8_len(chars, n)` bytes, which do
/// not overlap `chars`.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_utf8_encode(chars: *const u32, n: usize, out: *mut u8) {
    let mut written = 0;
    // SAFETY: the caller promises `n` readable `u32`s at `chars`.
    for c in unsafe { scalars(chars, n) } {
        // SAFETY: the caller promises room for every encoded byte, and `written` plus this
        // `char`'s bytes does not pass their total.
        written += unsafe { write(c.encode_utf8(&mut [0; 4]).as_bytes(), out.add(written)) };
    }
}

/// Prints the `len` bytes at `bytes` and a newline to standard output. Sisu's `print` calls
/// this for a `string` argument. It writes the bytes as they are, zero bytes included.
///
/// # Safety
///
/// `bytes` is not null and `len` bytes at it are readable; valid UTF-8 is not required.
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

/// Writes the `len` bytes at `bytes` and a newline to standard error. Sisu's `eprint` calls this
/// for a `string` argument. It writes the bytes as they are, zero bytes included.
///
/// # Safety
///
/// `bytes` is not null and `len` bytes at it are readable; valid UTF-8 is not required.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_eprint_str(bytes: *const u8, len: usize) {
    // SAFETY: the caller promises `len` readable bytes at `bytes`; a string's bytes pointer
    // is never null, even for the empty string.
    crate::eprint_bytes(unsafe { std::slice::from_raw_parts(bytes, len) });
}

/// Compares the `a_len` bytes at `a` with the `b_len` bytes at `b`, as byte slices' `Ord`
/// does: -1 when `a` sorts first, 0 when they are equal, 1 when `b` sorts first. For valid
/// UTF-8 this is code-point order.
///
/// # Safety
///
/// `a` and `b` are not null, and `a_len` bytes at `a` and `b_len` bytes at `b` are readable.
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
    use std::borrow::Cow;

    use super::{
        repair, sisu_str_compare, sisu_utf8_count, sisu_utf8_decode, sisu_utf8_encode,
        sisu_utf8_len, sisu_write_bool, sisu_write_char, sisu_write_int,
    };

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

    /// Each maximal subpart of an ill-formed sequence becomes one `U+FFFD`, and valid UTF-8 is
    /// borrowed as it is.
    #[test]
    fn repair_maximal_subparts() {
        let rows: [(&[u8], &str); 4] = [
            (b"a\xFF\xFFb", "a\u{FFFD}\u{FFFD}b"),
            (b"\xE2\x82", "\u{FFFD}"),
            (b"\xE2\x82A", "\u{FFFD}A"),
            ("häst".as_bytes(), "häst"),
        ];
        for (bytes, want) in rows {
            assert_eq!(*repair(bytes), *want.as_bytes(), "{bytes:?}");
        }
        assert!(matches!(repair("häst".as_bytes()), Cow::Borrowed(_)));
    }

    /// `"aä€😀"`: one code point of each UTF-8 width.
    const TEXT: &str = "aä€😀";

    /// The scalar values of `text`, which `sisu_utf8_count` counts and `sisu_utf8_decode` writes.
    #[test]
    fn utf8_decode_values() {
        // SAFETY: `TEXT` is valid UTF-8 and readable for its length.
        #[expect(unsafe_code, reason = "calls the runtime's C ABI function")]
        let count = unsafe { sisu_utf8_count(TEXT.as_ptr(), TEXT.len()) };
        assert_eq!(count, 4);
        let mut out = vec![0; count];
        // SAFETY: `TEXT` is valid UTF-8, and `out` has room for its `count` code points.
        #[expect(unsafe_code, reason = "calls the runtime's C ABI function")]
        unsafe {
            sisu_utf8_decode(TEXT.as_ptr(), TEXT.len(), out.as_mut_ptr());
        }
        assert_eq!(out, [97, 228, 8364, 128_512]);
    }

    /// The bytes of `TEXT`, which `sisu_utf8_len` sizes and `sisu_utf8_encode` writes.
    #[test]
    fn utf8_encode_bytes() {
        let chars = [97, 228, 8364, 128_512];
        // SAFETY: `chars` holds four readable scalar values.
        #[expect(unsafe_code, reason = "calls the runtime's C ABI function")]
        let len = unsafe { sisu_utf8_len(chars.as_ptr(), chars.len()) };
        assert_eq!(len, 10);
        let mut out = vec![0; len];
        // SAFETY: `chars` holds four readable scalar values, and `out` has room for their bytes.
        #[expect(unsafe_code, reason = "calls the runtime's C ABI function")]
        unsafe {
            sisu_utf8_encode(chars.as_ptr(), chars.len(), out.as_mut_ptr());
        }
        assert_eq!(out, [97, 195, 164, 226, 130, 172, 240, 159, 152, 128]);
    }

    /// Zero code points make zero bytes, and back.
    #[test]
    fn utf8_count_of_empty() {
        // SAFETY: no bytes are read, and a dangling pointer is the empty slice's.
        #[expect(unsafe_code, reason = "calls the runtime's C ABI function")]
        let (count, len) = unsafe {
            (
                sisu_utf8_count(b"".as_ptr(), 0),
                sisu_utf8_len([].as_ptr(), 0),
            )
        };
        assert_eq!((count, len), (0, 0));
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

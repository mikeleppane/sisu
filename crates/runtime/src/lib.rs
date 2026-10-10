//! The Sisu runtime: functions that compiled Sisu programs call over the C ABI.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
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

/// Allocates `size` bytes aligned to `align` for a Sisu object. Generated code calls this
/// to construct a class instance.
///
/// Aborts when `size` is 0 or `align` is not a power of two, or when the layout overflows
/// `isize`; codegen never asks for any of these. Calls `handle_alloc_error` when the
/// allocator returns null, so the result is never null.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_alloc(size: usize, align: usize) -> *mut u8 {
    let Ok(layout) = Layout::from_size_align(size, align) else {
        std::process::abort()
    };
    if size == 0 {
        std::process::abort()
    }
    // SAFETY: `layout` has a non-zero size, which `alloc` requires.
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        handle_alloc_error(layout)
    }
    ptr
}

/// Frees an object that `sisu_alloc` returned. Generated code calls this when an object's
/// reference count reaches zero.
///
/// # Safety
///
/// - `ptr` came from `sisu_alloc(size, align)` with these `size` and `align`.
/// - `ptr` is not used after this call.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name and passes a raw pointer"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_free(ptr: *mut u8, size: usize, align: usize) {
    // SAFETY: `sisu_alloc` accepted `size` and `align` as a valid layout, and the caller
    // passes the same two values.
    let layout = unsafe { Layout::from_size_align_unchecked(size, align) };
    // SAFETY: the caller upholds the `# Safety` contract above: `ptr` was allocated with
    // `layout` and is not used again.
    unsafe { dealloc(ptr, layout) };
}

#[cfg(test)]
mod tests {
    use super::{sisu_alloc, sisu_free};

    #[expect(
        unsafe_code,
        reason = "the test writes through the raw pointer and frees it"
    )]
    #[test]
    fn alloc_and_free_round_trip() {
        let ptr = sisu_alloc(24, 8);
        assert!(!ptr.is_null());
        assert_eq!(ptr.addr() % 8, 0, "pointer must be 8-aligned");
        for i in 0..24 {
            // SAFETY: `ptr` points to 24 allocated bytes and `i` is below 24.
            unsafe { ptr.add(i).write(u8::try_from(i).expect("i is below 24")) };
        }
        for i in 0..24 {
            // SAFETY: the same 24 bytes were written above.
            let byte = unsafe { ptr.add(i).read() };
            assert_eq!(usize::from(byte), i);
        }
        // SAFETY: `ptr` came from `sisu_alloc(24, 8)` and is not used afterwards.
        unsafe { sisu_free(ptr, 24, 8) };
    }

    #[expect(unsafe_code, reason = "the test frees the pointer")]
    #[test]
    fn alloc_honours_an_alignment_above_the_allocator_default() {
        let ptr = sisu_alloc(24, 4096);
        assert_eq!(ptr.addr() % 4096, 0, "pointer must be 4096-aligned");
        // SAFETY: `ptr` came from `sisu_alloc(24, 4096)` and is not used afterwards.
        unsafe { sisu_free(ptr, 24, 4096) };
    }
}

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
    exit_with_panic(format_args!(
        "sisu: panic at {}",
        String::from_utf8_lossy(bytes)
    ))
}

/// Prints `sisu: panic at <msg>: index <index> out of bounds for length <length>` to
/// standard error and exits with code 101. Generated code calls this for a failed array
/// index check; `msg` is the source location of the index expression.
///
/// # Safety
///
/// `msg` and `len` meet the same contract as for [`sisu_panic`].
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name and passes a raw pointer"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_panic_bounds(
    msg: *const u8,
    len: usize,
    index: i64,
    length: i64,
) -> ! {
    // SAFETY: the caller upholds the `# Safety` contract above, which is exactly what
    // `from_raw_parts` requires for a `u8` slice.
    let bytes = unsafe { std::slice::from_raw_parts(msg, len) };
    exit_with_panic(format_args!(
        "sisu: panic at {}: index {index} out of bounds for length {length}",
        String::from_utf8_lossy(bytes)
    ))
}

fn exit_with_panic(message: std::fmt::Arguments<'_>) -> ! {
    // `eprintln!` panics on a failed write, which would abort before exit code 101.
    let _ = writeln!(std::io::stderr(), "{message}");
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
    use super::{sisu_alloc, sisu_free, sisu_panic_bounds};
    use std::os::unix::process::ExitStatusExt;
    use std::process::Command;

    const ABORT_CASE_VAR: &str = "SISU_RUNTIME_ABORT_CASE";
    const PANIC_BOUNDS_VAR: &str = "SISU_RUNTIME_PANIC_BOUNDS";
    const SIGABRT: i32 = 6;

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
        assert!(!ptr.is_null());
        assert_eq!(ptr.addr() % 4096, 0, "pointer must be 4096-aligned");
        // SAFETY: `ptr` came from `sisu_alloc(24, 4096)` and is not used afterwards.
        unsafe { sisu_free(ptr, 24, 4096) };
    }

    /// Child entry point for the `alloc_aborts_*` tests. It returns at once
    /// unless the parent set the env var, so a normal test run ignores it.
    #[test]
    fn abort_child() {
        let Ok(case) = std::env::var(ABORT_CASE_VAR) else {
            return;
        };
        let (size, align) = match case.as_str() {
            "zero" => (0, 8),
            "invalid_align" => (8, 3),
            "overflow" => (usize::MAX - 8, 4096),
            other => panic!("unknown abort case {other}"),
        };
        sisu_alloc(size, align);
        // Reached only if `sisu_alloc` returned: exit normally so the parent sees no signal.
    }

    /// Child entry point for `panic_bounds_exits_101_when_stderr_fails`; a no-op unless the
    /// parent set the env var.
    #[expect(unsafe_code, reason = "the test calls an unsafe extern function")]
    #[test]
    fn panic_bounds_child() {
        if std::env::var(PANIC_BOUNDS_VAR).is_err() {
            return;
        }
        let msg = "f.sisu:3:7";
        // SAFETY: `msg` is non-null, readable for `msg.len()` bytes, and not written to.
        unsafe { sisu_panic_bounds(msg.as_ptr(), msg.len(), 7, 5) };
    }

    #[test]
    fn panic_bounds_exits_101_when_stderr_fails() {
        let exe = std::env::current_exe().expect("test binary path");
        let full = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .expect("/dev/full should exist on Linux");
        let out = Command::new(exe)
            .args(["--exact", "tests::panic_bounds_child", "--nocapture"])
            .env(PANIC_BOUNDS_VAR, "1")
            .stderr(full)
            .output()
            .expect("spawn child test");
        assert_eq!(
            out.status.code(),
            Some(101),
            "child exited with {:?}",
            out.status
        );
        // libtest also exits 101 for a failed test, but then it prints its summary; the
        // runtime's own exit leaves before the test finishes.
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.contains("test result:"),
            "libtest finished the child test, so the runtime did not exit: {stdout}"
        );
    }

    fn assert_child_aborts(case: &str) {
        let exe = std::env::current_exe().expect("test binary path");
        let status = Command::new(exe)
            .args(["--exact", "tests::abort_child", "--nocapture"])
            .env(ABORT_CASE_VAR, case)
            .status()
            .expect("spawn child test");
        assert_eq!(
            status.signal(),
            Some(SIGABRT),
            "child exited with {status:?}"
        );
    }

    #[test]
    fn alloc_aborts_on_zero_size() {
        assert_child_aborts("zero");
    }

    #[test]
    fn alloc_aborts_on_invalid_alignment() {
        assert_child_aborts("invalid_align");
    }

    #[test]
    fn alloc_aborts_when_layout_overflows() {
        assert_child_aborts("overflow");
    }
}

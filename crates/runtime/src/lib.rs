//! The Sisu runtime: functions that compiled Sisu programs call over the C ABI.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::fs::File;
use std::io::{IsTerminal, Write};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, PoisonError};

/// Number of objects `sisu_alloc` returned that `sisu_free` has not yet freed.
static LIVE: AtomicU64 = AtomicU64::new(0);

/// Prints `value` and a newline to standard output. Sisu's `print` calls this for an `i64`
/// argument.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_print_int(value: i64) {
    print_line(format_args!("{value}"));
}

/// Prints `true` or `false` and a newline to standard output. Sisu's `print` calls this
/// for a `bool` argument.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_print_bool(value: bool) {
    print_line(format_args!("{value}"));
}

/// Bytes `print` collects before it writes them out, when standard output is not a terminal.
const BUFFER_CAPACITY: usize = 8192;

/// The longest line `print` writes: `-9223372036854775808` and a newline.
const LONGEST_LINE: usize = 21;

/// The buffer's size: a line that starts below `BUFFER_CAPACITY` still fits.
const RESERVED: usize = BUFFER_CAPACITY + LONGEST_LINE;

/// Where `print` output goes, fixed at the first `print`.
struct Sink {
    /// A duplicate of file descriptor 1, written without std's own line buffering. `None`
    /// when standard output is closed: output is then dropped, as Rust's `println!` does.
    file: Option<File>,
    /// Whether standard output is a terminal, which gets a write per line.
    line_buffered: bool,
}

/// What `print` has collected and not yet written. Like C stdio, it writes at once to a
/// terminal and in blocks otherwise.
struct Stdout {
    buffer: Vec<u8>,
    sink: Option<Sink>,
}

impl Stdout {
    fn append(&mut self, line: std::fmt::Arguments<'_>) -> std::io::Result<()> {
        let sink = self.sink.get_or_insert_with(|| {
            // One allocation, once: the buffer never outgrows this. A flush follows any
            // line that takes it to `BUFFER_CAPACITY`.
            self.buffer.reserve_exact(RESERVED);
            let stdout = std::io::stdout();
            Sink {
                line_buffered: stdout.is_terminal(),
                file: stdout.as_fd().try_clone_to_owned().ok().map(File::from),
            }
        });
        let line_buffered = sink.line_buffered;
        // Writing to a `Vec` cannot fail.
        let _ = writeln!(self.buffer, "{line}");
        if line_buffered || self.buffer.len() >= BUFFER_CAPACITY {
            self.flush()
        } else {
            Ok(())
        }
    }

    /// Writes out the buffer. The buffer is emptied even when the write fails, so nothing
    /// tries the same bytes again, and the panic that a failure causes is the only error.
    fn flush(&mut self) -> std::io::Result<()> {
        let result = match self.sink.as_mut().and_then(|sink| sink.file.as_mut()) {
            Some(file) => file.write_all(&self.buffer),
            None => Ok(()),
        };
        self.buffer.clear();
        result
    }
}

static STDOUT: Mutex<Stdout> = Mutex::new(Stdout {
    buffer: Vec::new(),
    sink: None,
});

/// Runs `f` on the output buffer. Nothing panics while it holds the lock (`panic = "abort"`),
/// so a poisoned lock cannot happen; the error arm only avoids an `unwrap`.
fn with_stdout<R>(f: impl FnOnce(&mut Stdout) -> R) -> R {
    f(&mut STDOUT.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Adds `line` and a newline to the output. A failed write is a Sisu panic: `println!`
/// would panic and, with `panic = "abort"`, kill the program with SIGABRT. A pipe with no
/// reader still kills the program with SIGPIPE, as it does a C program.
fn print_line(line: std::fmt::Arguments<'_>) {
    // The lock is released here: the panic below flushes the buffer through it again.
    let result = with_stdout(|out| out.append(line));
    if let Err(error) = result {
        stdout_panic(&error);
    }
}

fn stdout_panic(error: &std::io::Error) -> ! {
    exit_with_panic(format_args!(
        "sisu: panic: standard output could not be written: {error}"
    ))
}

/// Writes out what `print` has collected; a failed write is the panic of `print_line`.
fn flush_or_panic() {
    let result = with_stdout(Stdout::flush);
    if let Err(error) = result {
        stdout_panic(&error);
    }
}

/// Writes out what `print` has collected. The generated C `main` calls this when `sisu.main`
/// returns, so a program that ends normally delivers all its output.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_flush_stdout() {
    flush_or_panic();
}

/// Prints `sisu: panic at <msg>` to standard error and exits with code 101.
///
/// Lines printed before the panic are written to standard output first, so they come out
/// before the panic line.
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

// `signal(2)`, declared here to avoid a dependency. `handler` is a function address or
// `SIG_IGN`; the previous handler is not needed.
#[expect(
    unsafe_code,
    reason = "declares a libc function; the runtime is linked against libc"
)]
unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

// shortcut: Linux x86-64 values, the only target; upgrade if another target is added.
const SIGPIPE: i32 = 13;
const SIG_IGN: usize = 1;

fn exit_with_panic(message: std::fmt::Arguments<'_>) -> ! {
    // A Sisu program keeps the default SIGPIPE action for standard output, but a panic must
    // exit 101 even when nothing reads standard error.
    // SAFETY: `signal` gets a valid signal number and `SIG_IGN`; it only changes the
    // process's signal disposition.
    #[expect(unsafe_code, reason = "calls libc `signal`")]
    unsafe {
        signal(SIGPIPE, SIG_IGN);
    }
    // Earlier lines come out before the panic line. A failure here, or one that led to this
    // panic, is not reported twice: a failed flush has emptied the buffer.
    let _ = with_stdout(Stdout::flush);
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
    LIVE.fetch_add(1, Relaxed);
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
    LIVE.fetch_sub(1, Relaxed);
}

/// Exit code for a reference-count check failure under `--rc-checks`.
const RC_CHECK_EXIT: i32 = 102;

/// Exits with code 102 when any object is still live. Programs compiled with
/// `--rc-checks` call this on a normal exit to detect leaks.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_rc_check_exit() {
    let live = LIVE.load(Relaxed);
    if live != 0 {
        // Write errors are ignored as in `exit_with_panic`: `eprintln!` would panic.
        let _ = writeln!(
            std::io::stderr(),
            "sisu: rc check: live objects at exit: {live}"
        );
        std::process::exit(RC_CHECK_EXIT);
    }
}

/// Reports a reference-count fault and exits with code 102. Kind 0 is a retain of an
/// object whose count is 0, kind 1 a release of one.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn sisu_rc_fault(kind: u32) -> ! {
    flush_or_panic();
    let _ = match kind {
        0 => writeln!(
            std::io::stderr(),
            "sisu: rc check: retain of an object whose count is 0"
        ),
        1 => writeln!(
            std::io::stderr(),
            "sisu: rc check: release of an object whose count is 0"
        ),
        other => writeln!(std::io::stderr(), "sisu: rc check: fault {other}"),
    };
    std::process::exit(RC_CHECK_EXIT)
}

#[cfg(test)]
mod tests {
    use super::{
        sisu_alloc, sisu_free, sisu_panic_bounds, sisu_print_int, sisu_rc_check_exit, sisu_rc_fault,
    };
    use std::os::unix::process::ExitStatusExt;
    use std::process::Command;

    const ABORT_CASE_VAR: &str = "SISU_RUNTIME_ABORT_CASE";
    const PANIC_BOUNDS_VAR: &str = "SISU_RUNTIME_PANIC_BOUNDS";
    const RC_CASE_VAR: &str = "SISU_RUNTIME_RC_CASE";
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

    /// Child entry point for the `check_exit_*` and `rc_fault_*` tests. `LIVE` is
    /// process-global, so each case runs in its own process; a no-op unless the parent set
    /// the env var.
    #[expect(unsafe_code, reason = "the test frees raw pointers")]
    #[test]
    fn rc_child() {
        let Ok(case) = std::env::var(RC_CASE_VAR) else {
            return;
        };
        let a = sisu_alloc(8, 8);
        match case.as_str() {
            "balanced" => {
                // SAFETY: `a` came from `sisu_alloc(8, 8)` and is not used afterwards.
                unsafe { sisu_free(a, 8, 8) };
                sisu_rc_check_exit();
            }
            "leak" => {
                let _leaked = sisu_alloc(8, 8);
                // SAFETY: `a` came from `sisu_alloc(8, 8)` and is not used afterwards.
                unsafe { sisu_free(a, 8, 8) };
                sisu_rc_check_exit();
            }
            "fault1" => sisu_rc_fault(1),
            "fault0" => sisu_rc_fault(0),
            "fault7" => sisu_rc_fault(7),
            "print_then_fault" => {
                sisu_print_int(5);
                sisu_rc_fault(1);
            }
            other => panic!("unknown rc case {other}"),
        }
    }

    fn run_rc_child(case: &str) -> std::process::Output {
        let exe = std::env::current_exe().expect("test binary path");
        Command::new(exe)
            .args(["--exact", "tests::rc_child", "--nocapture"])
            .env(RC_CASE_VAR, case)
            .output()
            .expect("spawn child test")
    }

    fn assert_rc_exit(case: &str, code: Option<i32>, stderr: &str) {
        let out = run_rc_child(case);
        assert_eq!(
            out.status.code(),
            code,
            "child exited with {:?}",
            out.status
        );
        assert_eq!(String::from_utf8_lossy(&out.stderr), stderr);
    }

    /// An `rc_fault` ends the program, so the lines printed before it must come out first.
    #[test]
    fn rc_fault_flushes_buffered_output() {
        let out = run_rc_child("print_then_fault");
        assert_eq!(out.status.code(), Some(102), "{:?}", out.status);
        // The libtest child also prints to stdout; the line it buffered is among it.
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.lines().any(|line| line == "5"), "{stdout}");
    }

    #[test]
    fn check_exit_passes_when_balanced() {
        assert_rc_exit("balanced", Some(0), "");
    }

    #[test]
    fn check_exit_reports_live_objects() {
        assert_rc_exit(
            "leak",
            Some(102),
            "sisu: rc check: live objects at exit: 1\n",
        );
    }

    #[test]
    fn rc_fault_exits_102() {
        assert_rc_exit(
            "fault1",
            Some(102),
            "sisu: rc check: release of an object whose count is 0\n",
        );
        assert_rc_exit(
            "fault0",
            Some(102),
            "sisu: rc check: retain of an object whose count is 0\n",
        );
        assert_rc_exit("fault7", Some(102), "sisu: rc check: fault 7\n");
    }
}

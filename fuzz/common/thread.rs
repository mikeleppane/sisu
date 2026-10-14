//! Runs one fuzz input on its own thread, where a stack overflow reaches libFuzzer's crash
//! handler and leaves a `crash-` artifact.
//!
//! The artifact is best-effort: libFuzzer's crash path is not async-signal-safe, so the handler
//! can still fail to write it.

use std::sync::Once;

use sisuc::STACK_SIZE;

/// The stack libFuzzer's SIGSEGV handler runs on after an overflow; far above `SIGSTKSZ`, since
/// the handler prints and writes the artifact on it.
const ALT_STACK_SIZE: usize = 1 << 20;

/// Runs `body` on a fresh `STACK_SIZE` thread with an alternate signal stack.
pub fn run_on_thread<F: FnOnce() + Send + 'static>(body: F) {
    // libFuzzer installs its handlers after `LLVMFuzzerInitialize`, so this waits for the first
    // input instead of going in the `fuzz_target!` init block.
    static ON_STACK: Once = Once::new();
    ON_STACK.call_once(|| {
        add_on_stack(libc::SIGSEGV);
        add_on_stack(libc::SIGBUS);
    });
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let _alt = AltStack::install();
            body();
        })
        .expect("the OS should start a thread with a 64 MiB stack")
        .join()
        .unwrap_or_else(|p| std::panic::resume_unwind(p));
}

/// Adds `SA_ONSTACK` to the handler libFuzzer installed for `signal`, keeping the rest.
fn add_on_stack(signal: libc::c_int) {
    // SAFETY: `sigaction` is plain old data; all zeroes is a valid value, overwritten below.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    // SAFETY: a null new action makes `sigaction` only read the current one into `action`.
    let read = unsafe { libc::sigaction(signal, std::ptr::null(), &raw mut action) };
    assert_eq!(read, 0, "sigaction should read the current handler");
    action.sa_flags |= libc::SA_ONSTACK;
    // SAFETY: `action` is libFuzzer's own handler, mask and flags as just read, plus SA_ONSTACK.
    let write = unsafe { libc::sigaction(signal, &raw const action, std::ptr::null_mut()) };
    assert_eq!(write, 0, "sigaction should install the handler");
}

/// This thread's alternate signal stack, registered until dropped.
struct AltStack {
    /// Only kept alive: the kernel writes to it, Rust never reads it.
    _memory: Vec<u8>,
}

impl AltStack {
    fn install() -> Self {
        let mut memory = vec![0u8; ALT_STACK_SIZE];
        let stack = libc::stack_t {
            ss_sp: memory.as_mut_ptr().cast(),
            ss_flags: 0,
            ss_size: memory.len(),
        };
        // SAFETY: `memory` lives in the returned value, whose `drop` unregisters the stack
        // before freeing it, so the kernel never runs a handler on freed memory.
        let set = unsafe { libc::sigaltstack(&raw const stack, std::ptr::null_mut()) };
        assert_eq!(set, 0, "sigaltstack should register the alternate stack");
        Self { _memory: memory }
    }
}

impl Drop for AltStack {
    fn drop(&mut self) {
        let off = libc::stack_t {
            ss_sp: std::ptr::null_mut(),
            ss_flags: libc::SS_DISABLE,
            ss_size: 0,
        };
        // SAFETY: disabling takes no memory; it only stops the kernel from using `self._memory`.
        let unset = unsafe { libc::sigaltstack(&raw const off, std::ptr::null_mut()) };
        assert_eq!(
            unset, 0,
            "sigaltstack should unregister the alternate stack"
        );
    }
}

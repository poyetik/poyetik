//! Runs native engine calls in the default floating-point mode of the processor, whatever
//! mode the host set (`spec/engine-design.md`, decision 11).
//!
//! A plugin host may turn on flush-to-zero or denormals-are-zero, or another rounding
//! direction, on the thread that calls the engine; WebAssembly has none of these. So that
//! a native render gives the same bits as WebAssembly, every native engine call runs inside
//! [`with_default_mode`], which sets the mode fields of the thread's control register to
//! the IEEE default (round to nearest, no flush, no trap, IEEE NaN propagation) and restores
//! the host's fields when it returns, whether the call returns, exits early or unwinds.
//!
//! A function that takes a [`DefaultMode`], a proof that only [`with_default_mode`] creates
//! and that lives no longer than its call, does not compile outside it: `engine-cli`'s
//! preparation of a case (parsing and building its plan) and its render take one.
//! `engine-core`'s entry points (`Plan::build`, `Plan::process`, `Renderer`) take none,
//! since `engine-core` also builds for WebAssembly, whose floats have no mode: a native
//! caller calls them inside [`with_default_mode`], a precondition stated in prose until the
//! native host API makes it a type (roadmap #32). The proof cannot be built anywhere else:
//!
//! ```compile_fail,E0451
//! let forged = engine_fpenv::DefaultMode {
//!     scope: core::marker::PhantomData,
//!     thread: core::marker::PhantomData,
//! };
//! ```
//!
//! nor kept after its call:
//!
//! ```compile_fail,E0521
//! let mut kept = None;
//! engine_fpenv::with_default_mode(|mode| kept = Some(mode));
//! ```
//!
//! nor sent to another thread, whose mode is its own:
//!
//! ```compile_fail,E0277
//! engine_fpenv::with_default_mode(|mode| std::thread::spawn(move || drop(mode)).join());
//! ```
//!
//! while within its call it can be passed on:
//!
//! ```
//! fn engine_call(_mode: engine_fpenv::DefaultMode<'_>) -> u32 { 7 }
//! assert_eq!(engine_fpenv::with_default_mode(|mode| engine_call(mode)), 7);
//! ```
//!
//! Native only: `x86_64` (MXCSR) and aarch64 (FPCR); any other target refuses to build it.
//! Its one `unsafe` file, `word.rs`, reads and writes the control register with inline
//! assembly, which Miri cannot run: it is proven by this crate's tests on the machine that
//! runs them (`x86_64` in CI, aarch64 on a Mac) and by the review written there.

#![no_std]

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!(
    "engine-fpenv supports x86_64 and aarch64 only: set the floating-point mode of another target before adding it"
);

mod word;

use core::marker::PhantomData;

/// The proof that the current thread runs in the default floating-point mode, for as long
/// as it lives: only [`with_default_mode`] creates one, for the duration of its call.
///
/// Neither `Send` nor `Sync`: the mode belongs to one thread.
#[derive(Debug, Clone, Copy)]
pub struct DefaultMode<'scope> {
    /// Ties the proof to the call of [`with_default_mode`] that made it.
    scope: PhantomData<&'scope ()>,
    /// Keeps the proof on its thread.
    thread: PhantomData<*const ()>,
}

/// Restores the host's mode fields when dropped: at the end of [`with_default_mode`], on
/// return, early exit or unwinding alike.
#[derive(Debug)]
struct Restore {
    /// The host's mode fields, as they were on entry.
    host: word::Word,
}

impl Drop for Restore {
    fn drop(&mut self) {
        word::set_control(self.host);
    }
}

/// Runs `body` in the default floating-point mode of the processor, then restores the
/// host's mode.
///
/// Only the mode fields change: on `x86_64` the exception flags keep accumulating, as IEEE
/// 754 defines them. The host's mode fields are always written back on exit, so a mode that
/// foreign code sets inside `body` is undone too; when the host's mode already is the
/// default, the entry write is skipped, so a nested call costs one write, on exit.
///
/// Review only: foreign code called inside `body` (assembly, a C function) must not change
/// the mode, since the proof `body` holds would be false from then on; safe Rust cannot
/// change it. The two writes break the letter of Rust's floating-point rule (RFC 3514), on
/// purpose and for the reasons the audit of the control-register file gives. Detecting such a
/// change and reporting it is roadmap #24's, with the first plugin shell.
///
/// Rust has no strict floating-point mode: the compiler orders the register's writes with
/// memory and `core::hint::black_box`, not with float arithmetic on values in registers.
/// So `body` passes through `black_box` after the entry write (its captures are read after
/// it) and its result through `black_box` before the restore (it is computed before): float
/// work inside `body` that reads its inputs from its captures or from memory, and returns
/// or stores its results, runs in the default mode.
pub fn with_default_mode<R>(body: impl for<'scope> FnOnce(DefaultMode<'scope>) -> R) -> R {
    let host = word::read() & word::CONTROL;
    if host != word::DEFAULT {
        word::set_control(word::DEFAULT);
    }
    let _restore = Restore { host };
    let body = core::hint::black_box(body);
    core::hint::black_box(body(DefaultMode {
        scope: PhantomData,
        thread: PhantomData,
    }))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

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
//! Engine calls that need the default mode take a [`DefaultMode`], a proof that only
//! [`with_default_mode`] creates and that lives no longer than its call: a call made without
//! it does not compile. The proof cannot be built anywhere else:
//!
//! ```compile_fail,E0451
//! let forged = engine_fpenv::DefaultMode { scope: core::marker::PhantomData, thread: core::marker::PhantomData };
//! ```
//!
//! nor kept after its call:
//!
//! ```compile_fail
//! let kept = engine_fpenv::with_default_mode(|mode| mode);
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

#[cfg(test)]
extern crate alloc;
#[cfg(test)]
extern crate std;

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
/// 754 defines them. When the host's mode already is the default, the register is not
/// written at all; so a nested call costs one read.
pub fn with_default_mode<R>(body: impl for<'scope> FnOnce(DefaultMode<'scope>) -> R) -> R {
    let host = word::read() & word::CONTROL;
    let _restore = (host != word::DEFAULT).then(|| {
        word::set_control(word::DEFAULT);
        Restore { host }
    });
    body(DefaultMode {
        scope: PhantomData,
        thread: PhantomData,
    })
}

#[cfg(test)]
mod tests;

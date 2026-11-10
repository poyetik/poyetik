//! The floating-point control word of the current thread: the only `unsafe` code of this
//! crate, one instruction to read it and one to write it.
//!
//! Audit (what review covers and the tests cannot):
//!
//! - **The instructions.** `x86_64`: `stmxcsr` stores MXCSR to memory, `ldmxcsr` loads it;
//!   aarch64: `mrs` reads FPCR into a general register, `msr` writes it back. Each is
//!   allowed in user mode, touches only the register named and the operand given, and is
//!   the instruction the architecture manuals define for that register.
//! - **The reserved bits.** Writing a reserved bit of MXCSR set faults, and FPCR's reserved
//!   bits must be kept as read. [`write()`] is private, and its only caller,
//!   [`set_control`], writes back every bit outside [`CONTROL`] exactly as [`read()`] gave it:
//!   no reserved bit is ever changed.
//! - **Ordering.** No `asm!` here is `pure` or `nomem`: the compiler treats each one as
//!   reading and writing memory, so it keeps them in program order with the loads and stores
//!   around them, and with `core::hint::black_box`. It does **not** order float arithmetic:
//!   Rust has no strict floating-point mode, and the compiler assumes the default mode
//!   everywhere, so it may move a computation on values held in registers across the
//!   register's write. A float computation runs in the mode set here only if its inputs are
//!   loaded after the write and its results stored before the restoring one (or pass
//!   through `black_box`). `with_default_mode` does that for its body: the closure passes
//!   through `black_box` after the entry write, so its captures are read after it, and the
//!   result through `black_box` before the restore. `black_box` is a best-effort barrier of
//!   the compiler, documented as such, which this ordering relies on.
//! - **The language rule this file breaks, on purpose.** Rust's floating-point semantics
//!   (RFC 3514) make it undefined behavior for inline assembly to alter the behavior of
//!   floating-point operations, and require the control bits to be, on leaving an `asm!`
//!   block, exactly as they were on entering it. Both writes break that letter, which
//!   decision 11 of `spec/engine-design.md` requires: the entry write moves the thread to
//!   the environment Rust assumes (it repairs a host already outside the rule), and the
//!   restoring write gives the host its own mode back, in which the rest of
//!   `with_default_mode`'s return path runs (dropping the guard, returning the result: no
//!   float work of the engine) before the host's code, outside Rust's assumption by the
//!   host's choice. The crate's tests go further: their `as_host` runs Rust code (renders,
//!   parsing) under forced modes on purpose, to show what the default mode protects, which
//!   the rule calls unsupported; their bodies pass through `black_box` as
//!   `with_default_mode`'s do, and each forced mode is first shown in force, so a body the
//!   compiler moved into the default mode would fail loudly, never pass. Accepted, review
//!   only: no other way gives a native engine call the default mode a host may have
//!   changed.
//! - **AMD's MM bit.** Some AMD processors define MXCSR bit 17 (misaligned exception mask);
//!   it is outside [`CONTROL`], so it is kept as the host set it.
//! - **The fields.** [`CONTROL`] and [`DEFAULT`] follow the architecture manuals: on
//!   `x86_64`, DAZ (bit 6), the exception masks (7 to 12), the rounding control (13, 14) and
//!   FTZ (15); on aarch64, FIZ (0), AH (1), NEP (2), the trap enables (8 to 12, 15), EBF
//!   (13), FZ16 (19), `RMode` (22, 23), FZ (24), DN (25) and AHP (26). The x86 x87 control
//!   word is not touched: Rust's floating point on `x86_64` runs on SSE only.

#![expect(
    unsafe_code,
    reason = "reading and writing the floating-point control register needs inline assembly"
)]

/// The control register's value: MXCSR (32 bits) on `x86_64`, FPCR (64 bits) on aarch64.
#[cfg(target_arch = "x86_64")]
pub(crate) type Word = u32;

/// The control register's value: MXCSR (32 bits) on `x86_64`, FPCR (64 bits) on aarch64.
#[cfg(target_arch = "aarch64")]
pub(crate) type Word = u64;

/// The bits of MXCSR that set the mode: DAZ, the six exception masks, the rounding control
/// and FTZ. The others are the sticky exception flags (bits 0 to 5), which record what
/// happened rather than set what happens, and reserved bits.
#[cfg(target_arch = "x86_64")]
pub(crate) const CONTROL: Word = 0xffc0;

/// The default mode of MXCSR: every exception masked, rounding to nearest, no DAZ, no FTZ.
#[cfg(target_arch = "x86_64")]
pub(crate) const DEFAULT: Word = 0x1f80;

/// The bits of FPCR that set the mode: FIZ, AH, NEP, the trap enables, EBF, FZ16, `RMode`,
/// FZ, DN and AHP. The others are reserved.
#[cfg(target_arch = "aarch64")]
pub(crate) const CONTROL: Word = 0x07c8_bf07;

/// The default mode of FPCR: every field zero (round to nearest, no flush, no trap, IEEE
/// NaN propagation and half precision).
#[cfg(target_arch = "aarch64")]
pub(crate) const DEFAULT: Word = 0;

/// The control register of the current thread.
pub(crate) fn read() -> Word {
    let word: Word;
    #[cfg(target_arch = "aarch64")]
    // SAFETY: `mrs` from FPCR is allowed in user mode and only writes the output register.
    unsafe {
        core::arch::asm!("mrs {word}, fpcr", word = out(reg) word, options(nostack, preserves_flags));
    }
    #[cfg(target_arch = "x86_64")]
    {
        let mut stored: Word = 0;
        // SAFETY: `stmxcsr` writes 4 bytes at its operand, the address of `stored`, a live,
        // aligned `u32` of this frame.
        unsafe {
            core::arch::asm!("stmxcsr [{at}]", at = in(reg) &raw mut stored, options(nostack, preserves_flags));
        }
        word = stored;
    }
    word
}

/// Writes the control register of the current thread. Private: [`set_control`] is the only
/// caller, so every reserved bit is written back as it was read.
fn write(word: Word) {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: `msr` to FPCR is allowed in user mode; `word` keeps the reserved bits as read
    // (its only caller, `set_control`).
    unsafe {
        core::arch::asm!("msr fpcr, {word}", word = in(reg) word, options(nostack, preserves_flags));
    }
    #[cfg(target_arch = "x86_64")]
    // SAFETY: `ldmxcsr` reads 4 bytes at its operand, the address of `word`, a live, aligned
    // `u32` argument; its reserved bits are as read (the only caller, `set_control`), so
    // the load cannot fault.
    unsafe {
        core::arch::asm!("ldmxcsr [{at}]", at = in(reg) &raw const word, options(nostack, preserves_flags));
    }
}

/// Sets the mode fields ([`CONTROL`]) of the control register to those of `control`, and
/// keeps every other bit (flags, reserved bits) as it is.
pub(crate) fn set_control(control: Word) {
    let current = read();
    // A masked merge: the bits of `control` under `CONTROL`, those of `current` elsewhere.
    write(current ^ ((current ^ control) & CONTROL));
}

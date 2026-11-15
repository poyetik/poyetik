//! The Poyetik engine compiled to WebAssembly, for browsers (in an `AudioWorklet`) and
//! Node.
//!
//! The module exports a minimal C interface ([`ffi`]) with no imports and no generated
//! glue: JavaScript writes a patch into the module memory, loads it, then pulls one
//! block of planar samples at a time. All the logic lives in the safe [`Engine`] and
//! [`Instance`]; [`ffi`] uses `unsafe` (for `#[unsafe(no_mangle)]` and in its own tests),
//! as one of the files allowed to, listed in xtask's `UNSAFE_FILES` and run under Miri.

#![doc(test(attr(forbid(unsafe_code))))]

extern crate alloc;

mod engine;
pub mod ffi;
mod instance;

pub use engine::{Blocks, Engine, Output, OutputMode};
pub use instance::Instance;

/// The engine block this module is built for (`spec/engine-design.md`, decision 2), in
/// frames.
///
/// The size the variable `POYETIK_ENGINE_BLOCK` names when the module is compiled (the
/// build of each module of `pnpm build:wasm` sets it), [`audio_core::BLOCK_SIZE`] when it
/// names none. Always one of [`audio_core::ENGINE_BLOCKS`]: a name of any other value does
/// not compile.
pub const ENGINE_BLOCK: usize = match option_env!("POYETIK_ENGINE_BLOCK") {
    Some(name) => named(name.as_bytes(), &NAMED),
    None => audio_core::BLOCK_SIZE,
};

const _: () = assert!(
    ENGINE_BLOCK != 0,
    "POYETIK_ENGINE_BLOCK must name one of audio_core::ENGINE_BLOCKS in decimal"
);

/// Each engine block a build offers, by its decimal name: [`audio_core::ENGINE_BLOCKS`], held
/// equal by a test.
const NAMED: [(&str, usize); 6] = [
    ("32", 32),
    ("64", 64),
    ("128", 128),
    ("256", 256),
    ("512", 512),
    ("1024", 1024),
];

/// The engine block of `named` called `name`, 0 when none is.
const fn named(name: &[u8], named_sizes: &[(&str, usize)]) -> usize {
    match named_sizes.split_first() {
        Some(((known, frames), rest)) => {
            if same(name, known.as_bytes()) {
                *frames
            } else {
                named(name, rest)
            }
        }
        None => 0,
    }
}

/// Whether `first` and `second` are the same bytes.
const fn same(first: &[u8], second: &[u8]) -> bool {
    match (first.split_first(), second.split_first()) {
        (Some((one, first_rest)), Some((other, second_rest))) => {
            *one == *other && same(first_rest, second_rest)
        }
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

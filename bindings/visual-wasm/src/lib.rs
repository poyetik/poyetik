//! The Poyetik visual library compiled to WebAssembly, for browsers.
//!
//! The module exports a minimal C interface ([`ffi`]) with no imports and no generated
//! glue (the playground's `src/square.ts` drives it): JavaScript sets the square up, then asks for one image at a time and reads its
//! linear-light values from the module memory. All the logic lives in the safe [`View`];
//! [`ffi`] uses `unsafe` only for `#[unsafe(no_mangle)]`.

pub mod ffi;
mod view;

pub use view::View;

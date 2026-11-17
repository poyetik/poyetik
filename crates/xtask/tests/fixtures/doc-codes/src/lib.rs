//! A crate whose one doc test names an error code that its code does not fail with: it
//! fails to compile as it should, with E0425 (a name not found), not E0599.
//!
//! ```compile_fail,E0599
//! let _unused = not_a_name;
//! ```

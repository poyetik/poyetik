//! A library whose tests load a module through one of its macros.

/// Declares the module `$name`, read from the directory of the module that calls it.
#[cfg(test)]
macro_rules! load {
    ($name:ident) => {
        mod $name;
    };
}

/// A function of the library.
#[must_use]
pub const fn one() -> u8 {
    1
}

#[cfg(test)]
mod lib_tests;

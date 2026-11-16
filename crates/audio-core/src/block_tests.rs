//! Tests of the engine blocks a build offers and names.

#![forbid(unsafe_code)]

use alloc::format;

use super::{MAX_ENGINE_BLOCK, NAMED, NAMED_ENGINE_BLOCK, held, holds, named, same};
use crate::ENGINE_BLOCKS;

#[test]
fn names_every_engine_block_offered() {
    assert_eq!(
        NAMED.map(|(_, frames)| frames),
        ENGINE_BLOCKS,
        "the sizes named"
    );
    for (name, frames) in NAMED {
        assert_eq!(name, format!("{frames}"), "{frames} named in decimal");
        assert_eq!(named(name.as_bytes(), &NAMED), frames, "{name} found");
    }
    for unknown in ["", "6", "640", "6x", "064", "1025", "32 "] {
        assert_eq!(named(unknown.as_bytes(), &NAMED), 0, "{unknown:?} refused");
    }
    assert!(same(b"", b""), "empty names");
}

#[test]
fn a_test_build_holds_every_engine_block() {
    // The tests build with no engine block named: every size is built, as the desktop
    // patcher's is; a size named in the environment would fail here first.
    assert_eq!(NAMED_ENGINE_BLOCK, None, "no engine block named");
    for frames in ENGINE_BLOCKS {
        assert!(holds(NAMED_ENGINE_BLOCK, frames), "{frames} built");
    }
}

#[test]
fn a_build_naming_a_size_holds_that_size_only() {
    assert!(holds(Some(128), 128), "the size named");
    assert!(!holds(Some(128), 64), "another size");
    assert!(holds(None, 64), "a build naming none holds every size");
    assert_eq!(held(true, 7_u8), Some(7), "held");
    assert_eq!(held(false, 7_u8), None, "not held");
}

#[test]
fn caps_the_engine_block_at_two_to_the_twentieth() {
    assert_eq!(MAX_ENGINE_BLOCK, 2_usize.pow(20), "2^20 frames");
    assert!(
        ENGINE_BLOCKS
            .iter()
            .all(|frames| *frames <= MAX_ENGINE_BLOCK),
        "every offered block within the cap"
    );
}

//! Tests of the engine blocks a build offers and names.

#![forbid(unsafe_code)]

use alloc::format;

use super::{ENGINE_BLOCKS, NAMED, NAMED_ENGINE_BLOCK, built_for, named, same};

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
        assert!(built_for(frames), "{frames} built");
    }
}

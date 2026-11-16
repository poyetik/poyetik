//! The global allocator of this crate's test binary.

#![forbid(unsafe_code)]

/// The test binary counts heap operations, so that the C interface can be checked not to
/// use the heap while rendering.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;

/// The package's one TypeScript list of the engine blocks it ships a module for, and its
/// default one, which the build script (`scripts/module.ts`) and the TypeScript tests read,
/// and these tests too.
const BLOCKS_SOURCE: &str = include_str!("../ts/blocks.ts");

/// The value of the exported TypeScript constant `name` of [`BLOCKS_SOURCE`], as written on
/// its one line `export const <name> = <value>;`, `as const` dropped; none when that line is
/// not there exactly once.
fn script_constant(name: &str) -> Option<&'static str> {
    let start = format!("export const {name} = ");
    let mut lines = BLOCKS_SOURCE
        .lines()
        .filter_map(|line| line.strip_prefix(start.as_str()));
    let value = lines.next()?.strip_suffix(';')?;
    lines
        .next()
        .is_none()
        .then(|| value.strip_suffix(" as const").unwrap_or(value))
}

/// `text` read as a decimal size of frames, written as the script writes them.
fn frames(text: &str) -> Option<usize> {
    text.parse()
        .ok()
        .filter(|frames: &usize| frames.to_string() == text)
}

#[test]
fn builds_a_module_for_every_engine_block_and_no_other() {
    // One list of sizes, `audio_core::ENGINE_BLOCKS`: the package writes it in TypeScript
    // (`ts/blocks.ts`, which the script building its modules reads), held equal here, so
    // that a size added to the engine ships its module.
    let list = script_constant("ENGINE_BLOCKS")
        .and_then(|value| value.strip_prefix('['))
        .and_then(|value| value.strip_suffix(']'))
        .expect("the package's list of engine blocks, on one line");
    let sizes: Vec<Option<usize>> = list.split(", ").map(frames).collect();
    assert_eq!(
        sizes,
        audio_core::ENGINE_BLOCKS.map(Some),
        "ts/blocks.ts lists a module for every engine block of audio_core::ENGINE_BLOCKS"
    );
    assert_eq!(
        script_constant("DEFAULT_ENGINE_BLOCK").and_then(frames),
        Some(audio_core::BLOCK_SIZE),
        "the module copied to engine.wasm is the default engine block's"
    );
}

#[test]
fn reads_the_script_constants_as_written_only() {
    // The reader of the test above, on what it refuses: a size not written in decimal as
    // the package writes it, or a constant missing.
    assert_eq!(frames("64"), Some(64), "a size");
    assert_eq!(frames("064"), None, "a leading zero");
    assert_eq!(frames("6_4"), None, "a separator");
    assert_eq!(frames(" 64"), None, "a space");
    assert_eq!(
        script_constant("NO_SUCH_CONSTANT"),
        None,
        "a constant missing"
    );
}

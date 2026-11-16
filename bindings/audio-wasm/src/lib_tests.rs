//! The global allocator of this crate's test binary.

#![forbid(unsafe_code)]

/// The test binary counts heap operations, so that the C interface can be checked not to
/// use the heap while rendering.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;

/// The package's one TypeScript list of the engine blocks it ships a module for, and its
/// default one, which the build script (`scripts/module.ts`) and the TypeScript tests read,
/// and these tests too: loaded when the tests build, a file of this crate declared as its
/// test data, so that Miri, which runs these tests with no access to files, needs none.
fn blocks_source() -> &'static str {
    include_str!("../ts/blocks.ts")
}

/// The value of the exported TypeScript constant `name` of `source`, as written on its one
/// line `export const <name> = <value>;`, `as const` dropped; none when that line is not
/// there exactly once.
fn script_constant<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let start = format!("export const {name} = ");
    let mut lines = source
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
    let source = blocks_source();
    let list = script_constant(source, "ENGINE_BLOCKS")
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
        script_constant(source, "DEFAULT_ENGINE_BLOCK").and_then(frames),
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
        script_constant(blocks_source(), "NO_SUCH_CONSTANT"),
        None,
        "a constant missing"
    );
}

/// The larger of two numbers, the first when they compare equal: `f64::max` is banned
/// (`clippy.toml`), since it may return either zero when they are `+0` and `-0`, a sign that
/// would differ between targets. A copy of audio-core's test helper (this test target cannot
/// reach that one), with its own test.
pub(crate) fn larger(first: f64, second: f64) -> f64 {
    if second > first { second } else { first }
}

#[test]
fn takes_the_larger_number_and_the_first_of_equal_ones() {
    for (first, second, larger_one) in [
        (1.0_f64, 2.0_f64, 2.0_f64),
        (2.0_f64, 1.0_f64, 2.0_f64),
        (0.0_f64, -0.0_f64, 0.0_f64),
        (-0.0_f64, 0.0_f64, -0.0_f64),
    ] {
        assert_eq!(
            larger(first, second).to_bits(),
            larger_one.to_bits(),
            "{first} and {second}"
        );
    }
}

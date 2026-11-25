//! Tests of the box contract's ABI: versions and buffer runs.

#![forbid(unsafe_code)]

use alloc::string::ToString;

use alloc::vec;
use alloc::vec::Vec;

use super::{
    BufferRun, CONTRACT_VERSION, ContractVersion, CopyError, Header, HeaderError, RunError,
    SUPPORTED_VERSIONS, VersionError,
};
use crate::host::Raw as _;
use crate::{Binary32, Binary64};

/// The majors an engine declares in these tests, each with its last minor, out of order and
/// with a gap, so that a rule reading only the first, the last or the range fails.
const DECLARED: [ContractVersion; 3] = [
    ContractVersion::new(7, 0),
    ContractVersion::new(2, 5),
    ContractVersion::new(4, 65_535),
];

#[test]
fn a_version_crosses_as_one_integer() {
    for (major, minor, packed) in [
        (0, 0, 0),
        (0, 1, 1),
        (1, 0, 65_536),
        (2, 5, 131_077),
        (0, 65_535, 65_535),
        (65_535, 0, 4_294_901_760),
        (65_535, 65_535, 4_294_967_295),
    ] {
        let version = ContractVersion::new(major, minor);
        assert_eq!(version.packed(), packed, "{major}.{minor}");
        assert_eq!(ContractVersion::from_packed(packed), version, "{packed}");
        assert_eq!(version.major(), major, "major of {packed}");
        assert_eq!(version.minor(), minor, "minor of {packed}");
    }
    assert_eq!(ContractVersion::new(12, 345).to_string(), "12.345", "shown");
}

#[test]
fn implements_the_version_its_specification_states_and_declares_it_alone() {
    const SPEC: &str = include_str!("../../../spec/box-contract.md");
    assert_eq!(
        (CONTRACT_VERSION.major(), CONTRACT_VERSION.minor()),
        (0, 0),
        "0.0"
    );
    assert!(
        SPEC.contains(
            "**This version of the contract is 0.0**, the one version the engine declares"
        ),
        "the specification states the version implemented"
    );
    assert_eq!(SUPPORTED_VERSIONS, [ContractVersion::new(0, 0)], "it alone");
    assert_eq!(
        CONTRACT_VERSION.check(&SUPPORTED_VERSIONS),
        Ok(()),
        "accepted"
    );
}

#[test]
fn accepts_a_declared_major_up_to_its_last_minor() {
    for (major, minor) in [(7, 0), (2, 0), (2, 4), (2, 5), (4, 0), (4, 65_535)] {
        assert_eq!(
            ContractVersion::new(major, minor).check(&DECLARED),
            Ok(()),
            "{major}.{minor}"
        );
    }
}

#[test]
fn refuses_a_later_minor_naming_the_last_known() {
    for (minor, major, last) in [(1, 7, 0), (6, 2, 5), (65_535, 2, 5)] {
        let version = ContractVersion::new(major, minor);
        assert_eq!(
            version.check(&DECLARED),
            Err(VersionError::LaterMinor { version, last }),
            "{major}.{minor}"
        );
    }
}

#[test]
fn refuses_a_major_not_declared() {
    for major in [0, 1, 3, 5, 6, 8, 65_535] {
        let version = ContractVersion::new(major, 0);
        let refused = Err(VersionError::UnsupportedMajor { version });
        assert_eq!(version.check(&DECLARED), refused, "{major}.0");
        assert_eq!(version.check(&[]), refused, "{major}.0, none declared");
    }
}

#[test]
fn accepts_major_0_only_at_the_minor_declared() {
    let declared = [ContractVersion::new(0, 3), ContractVersion::new(1, 2)];
    for (minor, accepted) in [
        (0, false),
        (2, false),
        (3, true),
        (4, false),
        (65_535, false),
    ] {
        let version = ContractVersion::new(0, minor);
        let expected = if accepted {
            Ok(())
        } else {
            Err(VersionError::Unfrozen { version, known: 3 })
        };
        assert_eq!(version.check(&declared), expected, "0.{minor}");
    }
    assert_eq!(ContractVersion::new(1, 0).check(&declared), Ok(()), "1.0");
}

#[test]
fn says_why_a_version_is_refused() {
    assert_eq!(
        VersionError::Unfrozen {
            version: ContractVersion::new(0, 2),
            known: 3
        }
        .to_string(),
        "contract version 0.2: before 1.0 this engine knows 0.3 alone",
        "not frozen"
    );
    let version = ContractVersion::new(2, 6);
    assert_eq!(
        VersionError::LaterMinor { version, last: 5 }.to_string(),
        "contract version 2.6 is later than 2.5, the last minor version this engine knows",
        "later minor"
    );
    assert_eq!(
        VersionError::UnsupportedMajor { version }.to_string(),
        "contract version 2.6: this engine does not support major version 2",
        "unsupported major"
    );
}

/// The stride of a span of `frames` samples of `bytes` bytes, counted 64 bytes at a time:
/// the first multiple of 64 that holds them.
fn counted_stride(frames: u32, bytes: u64) -> u64 {
    let span = u64::from(frames).saturating_mul(bytes);
    (0_u64..)
        .step_by(64)
        .take(131_074)
        .find(|&stride| stride >= span)
        .expect("a span of 2^20 samples of 8 bytes is 131,072 times 64 bytes")
}

#[test]
fn pads_each_buffer_to_a_multiple_of_64_bytes() {
    for frames in [
        1, 7, 8, 9, 15, 16, 17, 63, 64, 65, 441, 1_048_575, 1_048_576,
    ] {
        let wide = BufferRun::<Binary64>::new(0, frames, 3, u64::MAX).expect("binary64 run");
        let narrow = BufferRun::<Binary32>::new(0, frames, 3, u64::MAX).expect("binary32 run");
        assert_eq!(
            wide.stride(),
            counted_stride(frames, 8),
            "{frames}, binary64"
        );
        assert_eq!(
            narrow.stride(),
            counted_stride(frames, 4),
            "{frames}, binary32"
        );
        assert_eq!(wide.frames(), frames, "frames");
        assert_eq!(wide.buffers(), 3, "buffers");
    }
    for (frames, stride) in [(1, 64), (8, 64), (9, 128), (64, 512)] {
        let run = BufferRun::<Binary64>::new(0, frames, 1, 512).expect("a run");
        assert_eq!(run.stride(), stride, "{frames} frames, as a literal");
    }
}

#[test]
fn starts_each_buffer_one_stride_after_the_last() {
    let run = BufferRun::<Binary64>::new(192, 9, 3, 576).expect("a run ending with its memory");
    for (index, start) in [(0, Some(192)), (1, Some(320)), (2, Some(448)), (3, None)] {
        assert_eq!(run.start(index), start, "buffer {index}");
    }
    assert_eq!(run.start(u32::MAX), None, "no such buffer");
    assert_eq!(run.end(), 576, "end");
}

#[test]
fn accepts_a_run_that_ends_with_its_memory_and_refuses_one_byte_less() {
    let new = |memory| BufferRun::<Binary32>::new(64, 17, 2, memory).map(BufferRun::end);
    assert_eq!(new(320), Ok(320), "64 + 2 × 128");
    assert_eq!(
        new(319),
        Err(RunError::OutsideMemory {
            end: 320,
            memory: 319
        }),
        "one byte short"
    );
}

#[test]
fn counts_the_end_of_the_largest_run_without_wrapping() {
    let offset = u32::MAX - 63;
    let end = 36_028_801_305_542_592;
    assert_eq!(
        BufferRun::<Binary64>::new(offset, 1_048_576, u32::MAX, 4_294_967_296).map(BufferRun::end),
        Err(RunError::OutsideMemory {
            end,
            memory: 4_294_967_296
        }),
        "2^20 frames of 8 bytes in every buffer"
    );
    assert_eq!(
        BufferRun::<Binary64>::new(offset, 1, 1, 4_294_967_296).map(BufferRun::end),
        Ok(4_294_967_296),
        "the last 64 bytes of the largest memory"
    );
}

#[test]
fn a_run_of_no_buffer_takes_no_byte() {
    let run = BufferRun::<Binary64>::new(128, 64, 0, 128).expect("at the end of its memory");
    assert_eq!(run.end(), 128, "no byte");
    assert_eq!(run.start(0), None, "no buffer");
    assert_eq!(
        BufferRun::<Binary64>::new(192, 64, 0, 128).map(BufferRun::end),
        Err(RunError::OutsideMemory {
            end: 192,
            memory: 128
        }),
        "beyond its memory"
    );
}

#[test]
fn refuses_an_offset_off_the_alignment() {
    for offset in [1, 8, 32, 63, 65, 96, u32::MAX] {
        assert_eq!(
            BufferRun::<Binary64>::new(offset, 64, 1, u64::MAX).map(BufferRun::end),
            Err(RunError::Misaligned { offset }),
            "{offset}"
        );
    }
}

#[test]
fn refuses_a_span_no_engine_block_has() {
    for frames in [0, 1_048_577, u32::MAX] {
        assert_eq!(
            BufferRun::<Binary32>::new(0, frames, 1, u64::MAX).map(BufferRun::end),
            Err(RunError::Span { frames }),
            "{frames}"
        );
    }
}

/// Each fault of a run stacked on the later ones: the first that holds is told.
#[test]
fn tells_the_first_fault_of_a_run_that_holds() {
    for (offset, frames, refusal) in [
        (8, 0, RunError::Misaligned { offset: 8 }),
        (8, 2, RunError::Misaligned { offset: 8 }),
        (0, 0, RunError::Span { frames: 0 }),
        (64, 1_048_577, RunError::Span { frames: 1_048_577 }),
    ] {
        assert_eq!(
            BufferRun::<Binary64>::new(offset, frames, 4, 64).map(BufferRun::end),
            Err(refusal),
            "at {offset}, {frames} frames, beyond a memory of 64 bytes"
        );
    }
}

#[test]
fn says_why_a_run_is_refused() {
    assert_eq!(
        RunError::Misaligned { offset: 8 }.to_string(),
        "a run of buffers starts at byte 8, not a multiple of 64",
        "misaligned"
    );
    assert_eq!(
        RunError::Span { frames: 0 }.to_string(),
        "a run of buffers has a span of 0 frames, outside 1 to 1048576",
        "span"
    );
    assert_eq!(
        RunError::OutsideMemory {
            end: 65,
            memory: 64
        }
        .to_string(),
        "a run of buffers ends before byte 65, beyond a memory of 64 bytes",
        "outside memory"
    );
}

/// The byte a memory holds before a copy: no padding nor sample below is made of it.
const UNTOUCHED: u8 = 0xa5;

#[test]
fn writes_a_binary32_span_little_endian_then_zero_padding_and_nothing_else() {
    let run = BufferRun::<Binary32>::new(64, 2, 2, 192).expect("a run");
    let mut memory = vec![UNTOUCHED; 192];
    let samples = [0x0403_0201, 0x8000_0000].map(|bits| Binary32::from_raw(f32::from_bits(bits)));
    assert_eq!(run.write(1, &samples, &mut memory), Ok(()), "written");
    let mut expected = vec![UNTOUCHED; 128];
    expected.extend([1, 2, 3, 4, 0, 0, 0, 0x80]);
    expected.extend([0; 56]);
    assert_eq!(memory, expected, "bytes");
}

#[test]
fn writes_a_binary64_span_little_endian_then_zero_padding_and_nothing_else() {
    let run = BufferRun::<Binary64>::new(0, 9, 1, 192).expect("a run");
    let mut memory = vec![UNTOUCHED; 192];
    let samples = [Binary64::from_raw(f64::from_bits(0x0807_0605_0403_0201)); 9];
    assert_eq!(run.write(0, &samples, &mut memory), Ok(()), "written");
    let mut expected = [1, 2, 3, 4, 5, 6, 7, 8].repeat(9);
    expected.extend([0; 56]);
    expected.extend([UNTOUCHED; 64]);
    assert_eq!(memory, expected, "bytes");
}

#[test]
fn writes_a_nan_with_its_bits() {
    let run = BufferRun::<Binary64>::new(0, 1, 1, 64).expect("a run");
    let mut memory = vec![UNTOUCHED; 64];
    let nan = Binary64::from_raw(f64::from_bits(0xfff8_0000_dead_beef));
    assert_eq!(run.write(0, &[nan], &mut memory), Ok(()), "written");
    assert_eq!(
        memory.first_chunk::<8>(),
        Some(&0xfff8_0000_dead_beef_u64.to_le_bytes()),
        "the engine's own sample is not changed"
    );
}

#[test]
fn reads_a_binary64_span_with_its_nans_canonical_its_subnormals_flushed_never_the_padding() {
    let run = BufferRun::<Binary64>::new(64, 7, 1, 128).expect("a run");
    let mut memory = vec![UNTOUCHED; 64];
    for bits in [
        0xfff0_0000_0000_0001_u64,
        0x8000_0000_0000_0000,
        1,
        0x8000_0000_0000_0001,
        0x000f_ffff_ffff_ffff,
        0x0010_0000_0000_0000,
        0xfff0_0000_0000_0000,
    ] {
        memory.extend(bits.to_le_bytes());
    }
    memory.extend([0xff; 8]);
    let mut out = [Binary64::new(0.25); 7];
    assert_eq!(run.read(0, &memory, &mut out), Ok(()), "read");
    assert_eq!(
        out.map(Binary64::bits),
        [
            0x7ff8_0000_0000_0000,
            0x8000_0000_0000_0000,
            0,
            0x8000_0000_0000_0000,
            0,
            0x0010_0000_0000_0000,
            0xfff0_0000_0000_0000,
        ],
        "a NaN canonical, a subnormal a zero of its sign, the smallest normal, -0 and an \
         infinity kept"
    );
}

#[test]
fn reads_a_binary32_span_with_its_nans_canonical_its_subnormals_flushed_never_the_padding() {
    let run = BufferRun::<Binary32>::new(0, 7, 1, 64).expect("a run");
    let mut memory = Vec::new();
    for bits in [
        0xff80_0001_u32,
        0x8000_0000,
        1,
        0x8000_0001,
        0x007f_ffff,
        0x0080_0000,
        0xff80_0000,
    ] {
        memory.extend(bits.to_le_bytes());
    }
    memory.extend([0xff; 36]);
    let mut out = [Binary32::from_raw(0.25); 7];
    assert_eq!(run.read(0, &memory, &mut out), Ok(()), "read");
    assert_eq!(
        out.map(Binary32::bits),
        [
            0x7fc0_0000,
            0x8000_0000,
            0,
            0x8000_0000,
            0,
            0x0080_0000,
            0xff80_0000
        ],
        "a NaN canonical, a subnormal a zero of its sign, the smallest normal, -0 and an \
         infinity kept"
    );
}

/// A call processes 1 frame to a span (`spec/box-contract.md`, section 5, step 4): the
/// engine writes the first frames of an input, then zeros to the end of its stride, and reads
/// the first frames of an output alone.
#[test]
fn copies_the_first_frames_of_a_span_zeros_after_them() {
    let run = BufferRun::<Binary64>::new(64, 7, 2, 256).expect("a run");
    for given in 1..=7_u8 {
        let mut memory = vec![UNTOUCHED; 256];
        let samples: Vec<_> = (1..=given)
            .map(|value| Binary64::new(f64::from(value)))
            .collect();
        assert_eq!(
            run.write(1, &samples, &mut memory),
            Ok(()),
            "{given} written"
        );
        let mut expected = vec![UNTOUCHED; 128];
        for value in 1..=given {
            expected.extend(f64::from(value).to_bits().to_le_bytes());
        }
        expected.resize(192, 0);
        expected.extend([UNTOUCHED; 64]);
        assert_eq!(
            memory, expected,
            "{given} samples, then zeros to the stride's end"
        );
        let mut out = vec![Binary64::new(-1.0); usize::from(given)];
        assert_eq!(run.read(1, &memory, &mut out), Ok(()), "{given} read");
        assert_eq!(out, samples, "{given} read back");
    }
    let narrow = BufferRun::<Binary32>::new(0, 5, 1, 64).expect("a run");
    let mut memory = vec![UNTOUCHED; 64];
    let samples = [Binary32::from_raw(f32::from_bits(0x0403_0201)); 2];
    assert_eq!(
        narrow.write(0, &samples, &mut memory),
        Ok(()),
        "binary32 written"
    );
    let mut expected = [1, 2, 3, 4].repeat(2);
    expected.resize(64, 0);
    assert_eq!(memory, expected, "binary32, then zeros");
    let mut held = (1..=5_u16)
        .flat_map(|value| f32::from(value).to_bits().to_le_bytes())
        .collect::<Vec<u8>>();
    held.resize(64, UNTOUCHED);
    let mut out = [Binary32::from_raw(-1.0); 3];
    assert_eq!(narrow.read(0, &held, &mut out), Ok(()), "binary32 read");
    assert_eq!(
        out.map(Binary32::bits),
        [1.0_f32, 2.0_f32, 3.0_f32].map(|value| u64::from(value.to_bits())),
        "the first three alone"
    );
}

#[test]
fn reads_back_every_buffer_it_writes() {
    for frames in [1_u32, 7, 8, 9, 64, 441] {
        let run = BufferRun::<Binary64>::new(128, frames, 3, 65_536).expect("a run");
        let mut memory = vec![UNTOUCHED; 65_536];
        // A value of its own for each frame of each buffer.
        let spans = [1.0_f64, 1_000.0_f64, 1_000_000.0_f64].map(|buffer| {
            (0..frames)
                .map(|frame| Binary64::new(buffer) + Binary64::new(f64::from(frame)))
                .collect::<Vec<Binary64>>()
        });
        for (buffer, span) in [2_u32, 1, 0].into_iter().zip(spans.iter().rev()) {
            assert_eq!(run.write(buffer, span, &mut memory), Ok(()), "write");
        }
        for (buffer, span) in (0_u32..).zip(&spans) {
            let mut out = vec![Binary64::new(-1.0); span.len()];
            assert_eq!(run.read(buffer, &memory, &mut out), Ok(()), "read");
            assert_eq!(&out, span, "buffer {buffer} of {frames} frames");
        }
    }
}

#[test]
fn refuses_a_copy_that_is_not_the_runs_leaving_both_sides() {
    let run = BufferRun::<Binary64>::new(64, 2, 2, 192).expect("a run");
    let full = [Binary64::new(0.5); 3];
    for (index, frames, bytes, refusal) in [
        (2, 2, 192, CopyError::NoBuffer { index: 2 }),
        (u32::MAX, 2, 192, CopyError::NoBuffer { index: u32::MAX }),
        (0, 0, 192, CopyError::Frames { given: 0, span: 2 }),
        (0, 3, 192, CopyError::Frames { given: 3, span: 2 }),
        (
            1,
            2,
            191,
            CopyError::ShorterMemory {
                bytes: 191,
                end: 192,
            },
        ),
        (
            0,
            2,
            64,
            CopyError::ShorterMemory {
                bytes: 64,
                end: 128,
            },
        ),
    ] {
        let given = full.get(..frames).expect("at most three");
        let mut memory = vec![UNTOUCHED; bytes];
        assert_eq!(run.write(index, given, &mut memory), Err(refusal), "write");
        assert_eq!(memory, vec![UNTOUCHED; bytes], "memory left, {refusal}");
        let mut out = given.to_vec();
        assert_eq!(run.read(index, &memory, &mut out), Err(refusal), "read");
        assert_eq!(out, given, "samples left, {refusal}");
    }
    let mut memory = vec![UNTOUCHED; 128];
    assert_eq!(
        run.write(0, full.first_chunk::<2>().expect("two"), &mut memory),
        Ok(()),
        "buffer 0 lies in a memory that ends with it"
    );
}

/// Each fault of a copy stacked on the later ones: the first that holds is told, both sides
/// left as they are.
#[test]
fn tells_the_first_fault_of_a_copy_that_holds() {
    let run = BufferRun::<Binary64>::new(64, 2, 2, 192).expect("a run");
    let many = [Binary64::new(0.5); 3];
    for (index, frames, refusal) in [
        (2, 3, CopyError::NoBuffer { index: 2 }),
        (2, 0, CopyError::NoBuffer { index: 2 }),
        (1, 3, CopyError::Frames { given: 3, span: 2 }),
        (1, 0, CopyError::Frames { given: 0, span: 2 }),
    ] {
        let given = many.get(..frames).expect("at most three");
        let mut memory = vec![UNTOUCHED; 8];
        assert_eq!(run.write(index, given, &mut memory), Err(refusal), "write");
        assert_eq!(memory, vec![UNTOUCHED; 8], "memory left, {refusal}");
        let mut out = given.to_vec();
        assert_eq!(run.read(index, &memory, &mut out), Err(refusal), "read");
        assert_eq!(out, given, "samples left, {refusal}");
    }
}

#[test]
fn says_why_a_copy_is_refused() {
    for (refusal, said) in [
        (
            CopyError::NoBuffer { index: 2 },
            "a run of buffers has no buffer 2",
        ),
        (
            CopyError::Frames { given: 3, span: 2 },
            "a copy of 3 samples, outside 1 to the 2 frames of its run of buffers",
        ),
        (
            CopyError::ShorterMemory { bytes: 9, end: 128 },
            "a memory of 9 bytes does not hold the buffer, which ends before byte 128",
        ),
    ] {
        assert_eq!(refusal.to_string(), said, "{refusal:?}");
    }
}

/// A memory holding, from byte `offset`, a header of these fields, then `description`.
fn memory_with(offset: usize, version: u32, length: u32, description: &[u8]) -> Vec<u8> {
    let mut memory = vec![UNTOUCHED; offset];
    memory.extend(version.to_le_bytes());
    memory.extend(length.to_le_bytes());
    memory.extend(description);
    memory
}

#[test]
fn finds_the_version_and_the_description_of_a_header() {
    for offset in [0, 1, 64, 1_021] {
        let mut memory = memory_with(offset, 131_077, 3, b"abc");
        memory.extend(b"after");
        let header = Header::read(u32::try_from(offset).expect("a small offset"), &memory);
        assert_eq!(
            header.map(|header| (header.version(), header.description())),
            Ok((ContractVersion::new(2, 5), &b"abc"[..])),
            "at {offset}"
        );
    }
}

#[test]
fn reads_a_header_whose_description_ends_with_the_memory() {
    let memory = memory_with(8, 1, 4, b"last");
    let header = Header::read(8, &memory).expect("inside its memory");
    assert_eq!(header.description(), b"last", "description");
    let empty = memory_with(0, 1, 0, b"");
    assert_eq!(
        Header::read(0, &empty).map(Header::description),
        Ok(&b""[..]),
        "none: refused later, by their reader"
    );
}

#[test]
fn refuses_a_header_that_ends_beyond_the_memory() {
    let memory = memory_with(0, 1, 0, b"");
    for offset in [1, 7, 8, 9, 1 << 31_u32, u32::MAX] {
        assert_eq!(
            Header::read(offset, &memory).map(Header::version),
            Err(HeaderError::Header { offset, memory: 8 }),
            "at {offset}"
        );
    }
    assert_eq!(
        Header::read(0, &[]).map(Header::version),
        Err(HeaderError::Header {
            offset: 0,
            memory: 0
        }),
        "no memory"
    );
}

#[test]
fn refuses_a_description_that_ends_beyond_the_memory() {
    for length in [4, 5, 1 << 31_u32, u32::MAX] {
        let memory = memory_with(16, 1, length, b"abc");
        assert_eq!(
            Header::read(16, &memory).map(Header::version),
            Err(HeaderError::Description {
                offset: 24,
                length,
                memory: 27
            }),
            "{length}"
        );
    }
}

#[test]
fn says_why_a_header_is_refused() {
    assert_eq!(
        HeaderError::Header {
            offset: 9,
            memory: 12
        }
        .to_string(),
        "the module's header, 8 bytes at byte 9, ends beyond a memory of 12 bytes",
        "header"
    );
    assert_eq!(
        HeaderError::Description {
            offset: 24,
            length: 4,
            memory: 27
        }
        .to_string(),
        "the module's description, 4 bytes at byte 24, ends beyond a memory of 27 bytes",
        "description"
    );
}

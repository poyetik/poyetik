//! Tests of kernels chained on buffers that stay on the GPU: passes and copies in one
//! submission, buffers rewritten and read between submissions, dispatches over two
//! dimensions, what each buffer's values are made of, and every refusal before the GPU.

use super::{Buffers, Step};
use crate::compare::{Mismatch, Tolerance, Values};
use crate::run::{Data, Gpu, Kernel, Limits, RunError, Workgroups};

/// Squares each input, one invocation per value.
const SQUARE: &str = "@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read_write> squares: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < arrayLength(&input) {
        squares[id.x] = input[id.x] * input[id.x];
    }
}
";

/// Adds a uniform offset to each square.
const OFFSET: &str = "@group(0) @binding(0) var<uniform> offset: vec4<f32>;
@group(0) @binding(1) var<storage, read> squares: array<f32>;
@group(0) @binding(2) var<storage, read_write> shifted: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < arrayLength(&squares) {
        shifted[id.x] = squares[id.x] + offset.x;
    }
}
";

/// Writes `100 y + x` to each cell of a 16 by 4 grid, over workgroups of 8 by 8.
const GRID: &str = "@group(0) @binding(0) var<storage, read_write> cells: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < 16u && id.y < 4u {
        cells[id.y * 16u + id.x] = id.y * 100u + id.x;
    }
}
";

/// Reads two buffers of different scalars, and a structure of mixed members.
const READERS: &str = "struct Mixed { count: u32, scale: f32 }
@group(0) @binding(0) var<storage, read> words: array<u32>;
@group(0) @binding(1) var<storage, read> floats: array<f32>;
@group(0) @binding(2) var<storage, read_write> mixed: Mixed;

@compute @workgroup_size(1)
fn main() {
    mixed.count = words[0];
    mixed.scale = floats[0];
}
";

/// The shared device, which the machine running the tests must have.
fn gpu() -> &'static Gpu {
    Gpu::shared().expect("a GPU: the tests of the harness need one")
}

/// The kernel `main` of `source` on the shared device.
fn kernel(source: &str) -> Kernel<'static> {
    Kernel::new(gpu(), source, "main").expect("a kernel")
}

/// A uniform offset of `by`.
fn offset(by: f32) -> [f32; 4] {
    [by, 0.0, 0.0, 0.0]
}

#[test]
fn chains_passes_in_one_submission_and_copies_between_them() {
    let (square, shift) = (kernel(SQUARE), kernel(OFFSET));
    let mut buffers = Buffers::new(gpu());
    let input = buffers
        .add(Data::F32(&[1.0, 2.0, 3.0, -4.0]))
        .expect("a buffer");
    let squares = buffers.add(Data::Zeroed(16)).expect("a buffer");
    let by = buffers.add(Data::F32(&offset(0.5))).expect("a buffer");
    let shifted = buffers.add(Data::Zeroed(16)).expect("a buffer");
    let copied = buffers.add(Data::Zeroed(16)).expect("a buffer");
    let first = buffers
        .bind(&square, &[(0, 0, input), (0, 1, squares)])
        .expect("bound");
    let second = buffers
        .bind(&shift, &[(0, 1, squares), (0, 0, by), (0, 2, shifted)])
        .expect("bound, in any order");
    buffers
        .dispatch(&[
            Step::Pass(&first, Workgroups(1, 1, 1)),
            Step::Pass(&second, Workgroups(1, 1, 1)),
            Step::Copy {
                from: shifted,
                to: copied,
            },
        ])
        .expect("one submission");
    let exact = Tolerance::Exact;
    assert_eq!(
        buffers.check(squares, Values::F32(&[1.0, 4.0, 9.0, 16.0]), &exact, None),
        Ok(()),
        "the first pass's output, read between steps"
    );
    assert_eq!(
        buffers.check(shifted, Values::F32(&[1.5, 4.5, 9.5, 16.5]), &exact, None),
        Ok(()),
        "the second pass read the first's output on the GPU"
    );
    assert_eq!(
        buffers.check(copied, Values::F32(&[1.5, 4.5, 9.5, 16.5]), &exact, None),
        Ok(()),
        "copied after both passes, in the same submission"
    );
    assert_eq!(
        buffers.check(input, Values::F32(&[1.0, 2.0, 3.0, -4.0]), &exact, None),
        Ok(()),
        "an input, unchanged"
    );
}

#[test]
fn rewrites_buffers_between_submissions() {
    let (square, shift) = (kernel(SQUARE), kernel(OFFSET));
    let mut buffers = Buffers::new(gpu());
    let input = buffers.add(Data::F32(&[3.0, 5.0])).expect("a buffer");
    let squares = buffers.add(Data::Zeroed(8)).expect("a buffer");
    let by = buffers.add(Data::F32(&offset(1.0))).expect("a buffer");
    let shifted = buffers.add(Data::Zeroed(8)).expect("a buffer");
    let first = buffers
        .bind(&square, &[(0, 0, input), (0, 1, squares)])
        .expect("bound");
    let second = buffers
        .bind(&shift, &[(0, 0, by), (0, 1, squares), (0, 2, shifted)])
        .expect("bound");
    let both = [
        Step::Pass(&first, Workgroups(1, 1, 1)),
        Step::Pass(&second, Workgroups(1, 1, 1)),
    ];
    buffers.dispatch(&both).expect("a submission");
    let exact = Tolerance::Exact;
    assert_eq!(
        buffers.check(shifted, Values::F32(&[10.0, 26.0]), &exact, None),
        Ok(()),
        "the first submission"
    );
    buffers
        .write(by, Data::F32(&offset(-2.0)))
        .expect("a new offset");
    buffers
        .dispatch(&[Step::Pass(&second, Workgroups(1, 1, 1))])
        .expect("the second pass alone");
    assert_eq!(
        buffers.check(shifted, Values::F32(&[7.0, 23.0]), &exact, None),
        Ok(()),
        "the new offset, the squares kept on the GPU"
    );
    buffers
        .write(
            input,
            Data::Bytes(&7.0_f32.to_bits().to_le_bytes().repeat(2)),
        )
        .expect("new inputs, as bytes");
    buffers.write(shifted, Data::Zeroed(8)).expect("cleared");
    assert_eq!(
        buffers.bytes(shifted),
        Ok(vec![0; 8]),
        "zeros written over the whole buffer"
    );
    buffers.dispatch(&both).expect("both passes again");
    assert_eq!(
        buffers.check(shifted, Values::F32(&[47.0, 47.0]), &exact, None),
        Ok(()),
        "the new inputs through both passes"
    );
}

#[test]
fn dispatches_over_two_dimensions() {
    let grid = kernel(GRID);
    let mut buffers = Buffers::new(gpu());
    let cells = buffers.add(Data::Zeroed(256)).expect("a buffer");
    let bound = buffers.bind(&grid, &[(0, 0, cells)]).expect("bound");
    buffers
        .dispatch(&[Step::Pass(&bound, Workgroups(2, 1, 1))])
        .expect("16 by 8 invocations");
    let mut expected = [0_u32; 64];
    for (index, cell) in (0_u32..).zip(expected.iter_mut()) {
        let (row, column) = (index / 16, index % 16);
        *cell = row * 100 + column;
    }
    assert_eq!(
        buffers.check(cells, Values::U32(&expected), &Tolerance::Exact, None),
        Ok(()),
        "each cell of the 16 by 4 grid written by its invocation"
    );
}

#[test]
fn binds_a_kernel_that_binds_nothing() {
    let empty = kernel("@compute @workgroup_size(1)\nfn main() {}\n");
    let mut buffers = Buffers::new(gpu());
    let kept = buffers.add(Data::U32(&[9])).expect("a buffer");
    let bound = buffers.bind(&empty, &[]).expect("bound to nothing");
    buffers
        .dispatch(&[Step::Pass(&bound, Workgroups(1, 1, 1))])
        .expect("a pass");
    assert_eq!(
        buffers.check(kept, Values::U32(&[9]), &Tolerance::Exact, None),
        Ok(()),
        "a buffer it does not bind, untouched"
    );
}

#[test]
fn reads_a_buffer_as_what_was_last_put_in_it() {
    let readers = kernel(READERS);
    let mut buffers = Buffers::new(gpu());
    let words = buffers.add(Data::Zeroed(4)).expect("a buffer");
    let floats = buffers.add(Data::Zeroed(4)).expect("a buffer");
    let mixed = buffers.add(Data::Zeroed(8)).expect("a buffer");
    let exact = Tolerance::Exact;
    let _bound = buffers
        .bind(&readers, &[(0, 0, words), (0, 1, floats), (0, 2, mixed)])
        .expect("bound");
    assert_eq!(
        buffers.check(words, Values::F32(&[0.0]), &exact, None),
        Ok(()),
        "only read by the kernel: still read as the reference's kind"
    );
    assert_eq!(
        buffers.check(mixed, Values::U32(&[0, 0]), &exact, None),
        Err(Mismatch::Kinds {
            actual: "mixed or padded",
            expected: "u32"
        }),
        "written through mixed members"
    );
    assert_eq!(
        buffers.check(mixed, Values::Bytes(&[0; 8]), &exact, None),
        Ok(()),
        "mixed members as bytes"
    );
    assert_eq!(
        buffers.write(mixed, Data::U32(&[1, 2])),
        Ok(()),
        "values written over mixed members, which they replace"
    );
    assert_eq!(
        buffers.check(mixed, Values::U32(&[1, 2]), &exact, None),
        Ok(()),
        "now the u32 values written"
    );
    assert_eq!(
        buffers.write(floats, Data::U32(&[1])),
        Ok(()),
        "u32 values over zeros"
    );
    assert_eq!(
        buffers.check(floats, Values::F32(&[0.0]), &exact, None),
        Err(Mismatch::Kinds {
            actual: "u32",
            expected: "f32"
        }),
        "read as the values written"
    );
    assert_eq!(
        buffers.write(floats, Data::Bytes(&[0, 0, 0xc0, 0x3f])),
        Ok(()),
        "bytes over values"
    );
    assert_eq!(
        buffers.check(floats, Values::F32(&[1.5]), &exact, None),
        Ok(()),
        "bytes are read as the reference's kind"
    );
    let typed = buffers.add(Data::I32(&[-1_i32])).expect("a buffer");
    assert_eq!(
        buffers.check(typed, Values::U32(&[u32::MAX]), &exact, None),
        Err(Mismatch::Kinds {
            actual: "i32",
            expected: "u32"
        }),
        "added as i32"
    );
}

#[test]
fn lets_a_pass_read_what_another_wrote_as_another_scalar() {
    let writer = kernel(
        "@group(0) @binding(0) var<storage, read_write> bits: array<u32>;
@compute @workgroup_size(1)
fn main() { bits[0] = 0x3fc00000u; }
",
    );
    let reader = kernel(
        "@group(0) @binding(0) var<storage, read> floats: array<f32>;
@group(0) @binding(1) var<storage, read_write> doubled: array<f32>;
@compute @workgroup_size(1)
fn main() { doubled[0] = floats[0] * 2.0; }
",
    );
    let mut buffers = Buffers::new(gpu());
    let shared = buffers.add(Data::Zeroed(4)).expect("a buffer");
    let doubled = buffers.add(Data::Zeroed(4)).expect("a buffer");
    let copied = buffers.add(Data::F32(&[9.0])).expect("a buffer");
    let first = buffers.bind(&writer, &[(0, 0, shared)]).expect("bound");
    let second = buffers
        .bind(&reader, &[(0, 0, shared), (0, 1, doubled)])
        .expect("the u32 a kernel wrote, bound as f32, as WebGPU allows");
    buffers
        .dispatch(&[
            Step::Pass(&first, Workgroups(1, 1, 1)),
            Step::Pass(&second, Workgroups(1, 1, 1)),
            Step::Copy {
                from: shared,
                to: copied,
            },
        ])
        .expect("a submission");
    let exact = Tolerance::Exact;
    assert_eq!(
        buffers.check(doubled, Values::F32(&[3.0]), &exact, None),
        Ok(()),
        "1.5, written as its bits, doubled"
    );
    assert_eq!(
        buffers.check(shared, Values::U32(&[0x3fc0_0000]), &exact, None),
        Ok(()),
        "read back as the u32 its writer declares"
    );
    assert_eq!(
        buffers.check(copied, Values::U32(&[0x3fc0_0000]), &exact, None),
        Ok(()),
        "a copy holds what its source held, not the f32 first given"
    );
    let given = buffers.add(Data::U32(&[1])).expect("a buffer");
    assert_eq!(
        buffers
            .bind(&reader, &[(0, 0, given), (0, 1, doubled)])
            .map(drop),
        Err(RunError::BadLayout {
            group: 0,
            binding: 0,
            why: "u32 values given, the shader declares f32".to_owned()
        }),
        "values the test gave are still checked against each binding"
    );
}

#[test]
fn copies_carry_what_each_buffer_held_in_their_order() {
    let mut buffers = Buffers::new(gpu());
    let words = buffers.add(Data::U32(&[7])).expect("a buffer");
    let middle = buffers.add(Data::F32(&[0.0])).expect("a buffer");
    let last = buffers.add(Data::I32(&[0_i32])).expect("a buffer");
    buffers
        .dispatch(&[
            Step::Copy {
                from: words,
                to: middle,
            },
            Step::Copy {
                from: middle,
                to: last,
            },
        ])
        .expect("two copies");
    assert_eq!(
        buffers.check(last, Values::U32(&[7]), &Tolerance::Exact, None),
        Ok(()),
        "the u32 values, through the buffer copied in the same submission"
    );
}

#[test]
fn reads_a_buffer_of_no_kind_yet_as_the_reference_s_until_values_are_written() {
    let mut buffers = Buffers::new(gpu());
    let never = buffers.add(Data::Zeroed(4)).expect("a buffer");
    let exact = Tolerance::Exact;
    for expected in [
        Values::U32(&[0]),
        Values::I32(&[0_i32]),
        Values::F32(&[0.0]),
        Values::Bytes(&[0, 0, 0, 0]),
    ] {
        assert_eq!(
            buffers.check(never, expected, &exact, None),
            Ok(()),
            "zeros never bound, read as the reference's kind: {expected:?}"
        );
    }
    assert_eq!(
        buffers.write(never, Data::I32(&[-3_i32])),
        Ok(()),
        "values into a buffer of no kind yet"
    );
    assert_eq!(
        buffers.check(never, Values::I32(&[-3_i32]), &exact, None),
        Ok(()),
        "now i32"
    );
    assert_eq!(
        buffers.check(never, Values::F32(&[0.0]), &exact, None),
        Err(Mismatch::Kinds {
            actual: "i32",
            expected: "f32"
        }),
        "no longer read as another kind"
    );
}

#[test]
fn refuses_a_binding_that_does_not_fit_before_the_gpu() {
    let (square, shift, readers) = (kernel(SQUARE), kernel(OFFSET), kernel(READERS));
    let mut buffers = Buffers::new(gpu());
    let input = buffers.add(Data::F32(&[1.0])).expect("a buffer");
    let words = buffers.add(Data::U32(&[1])).expect("a buffer");
    let out = buffers.add(Data::Zeroed(4)).expect("a buffer");
    let odd = buffers.add(Data::Zeroed(12)).expect("a buffer");
    let large = buffers.add(Data::Zeroed(65_552)).expect("a buffer");
    let bad = |group: u32, binding: u32, why: &str| {
        Err::<(), _>(RunError::BadLayout {
            group,
            binding,
            why: why.to_owned(),
        })
    };
    let cases = [
        (
            buffers
                .bind(&square, &[(0, 0, input), (0, 1, out), (1, 0, out)])
                .map(drop),
            bad(1, 0, "the shader binds no buffer there"),
        ),
        (
            buffers.bind(&square, &[(0, 0, input)]).map(drop),
            bad(0, 1, "no buffer given for it"),
        ),
        (
            buffers
                .bind(&square, &[(0, 0, input), (0, 1, out), (0, 1, odd)])
                .map(drop),
            bad(0, 1, "given more than once"),
        ),
        (
            buffers
                .bind(&square, &[(0, 0, words), (0, 1, out)])
                .map(drop),
            bad(0, 0, "u32 values given, the shader declares f32"),
        ),
        (
            buffers
                .bind(&shift, &[(0, 0, odd), (0, 1, input), (0, 2, out)])
                .map(drop),
            bad(0, 0, "12 bytes given, 16 expected"),
        ),
        (
            buffers
                .bind(&square, &[(0, 0, input), (0, 1, input)])
                .map(drop),
            bad(
                0,
                1,
                "its buffer is also bound at group 0 binding 0, and the kernel writes one of them",
            ),
        ),
        (
            buffers
                .bind(&readers, &[(0, 0, words), (0, 1, words), (0, 2, odd)])
                .map(drop),
            bad(0, 1, "u32 values given, the shader declares f32"),
        ),
    ];
    for (found, expected) in cases {
        assert_eq!(found, expected, "{expected:?}");
    }
    let writer_first = kernel(
        "@group(0) @binding(0) var<storage, read_write> written: array<u32>;
@group(0) @binding(1) var<storage, read> source: array<u32>;
@compute @workgroup_size(1)
fn main() { written[0] = source[0]; }
",
    );
    assert_eq!(
        buffers
            .bind(&writer_first, &[(0, 0, words), (0, 1, words)])
            .map(drop),
        bad(
            0,
            1,
            "its buffer is also bound at group 0 binding 0, and the kernel writes one of them",
        ),
        "written at the first binding, read at the second"
    );
    let wide = kernel(
        "@group(0) @binding(0) var<uniform> wide: array<vec4<u32>, 4097>;
@group(0) @binding(1) var<storage, read_write> first: array<u32>;
@compute @workgroup_size(1)
fn main() { first[0] = wide[4096].x; }
",
    );
    assert_eq!(
        buffers.bind(&wide, &[(0, 0, large), (0, 1, out)]).map(drop),
        Err(RunError::TooLarge {
            group: 0,
            binding: 0,
            bytes: 65_552,
            limit: 65_536
        }),
        "a uniform beyond WebGPU's 64 KiB"
    );
}

#[test]
fn binds_one_buffer_twice_where_the_kernel_only_reads_it() {
    let mut buffers = Buffers::new(gpu());
    let twice = buffers.add(Data::Zeroed(8)).expect("a buffer");
    let both = kernel(
        "@group(0) @binding(0) var<storage, read> first: array<u32>;
@group(0) @binding(1) var<storage, read> second: array<u32>;
@group(0) @binding(2) var<storage, read_write> sum: array<u32>;
@compute @workgroup_size(1)
fn main() { sum[0] = first[0] + second[1]; }
",
    );
    let sum = buffers.add(Data::Zeroed(4)).expect("a buffer");
    buffers
        .write(twice, Data::U32(&[20, 22]))
        .expect("two words");
    let bound = buffers
        .bind(&both, &[(0, 0, twice), (0, 1, twice), (0, 2, sum)])
        .expect("one buffer read twice");
    buffers
        .dispatch(&[Step::Pass(&bound, Workgroups(1, 1, 1))])
        .expect("a pass");
    assert_eq!(
        buffers.check(sum, Values::U32(&[42]), &Tolerance::Exact, None),
        Ok(()),
        "read through both bindings"
    );
}

#[test]
fn refuses_buffers_bindings_and_kernels_of_another_set_or_device() {
    let square = kernel(SQUARE);
    let mut buffers = Buffers::new(gpu());
    let mut others = Buffers::new(gpu());
    let mine = buffers.add(Data::F32(&[2.0])).expect("a buffer");
    let out = buffers.add(Data::Zeroed(4)).expect("a buffer");
    let theirs = others.add(Data::F32(&[2.0])).expect("a buffer");
    let their_out = others.add(Data::Zeroed(4)).expect("a buffer");
    let foreign = RunError::BadBuffer("a buffer of another set of buffers".to_owned());
    assert_eq!(
        buffers
            .bind(&square, &[(0, 0, theirs), (0, 1, out)])
            .map(drop),
        Err(foreign.clone()),
        "bound"
    );
    assert_eq!(
        buffers.write(theirs, Data::F32(&[1.0])),
        Err(foreign.clone()),
        "written"
    );
    assert_eq!(buffers.bytes(theirs), Err(foreign.clone()), "read");
    assert_eq!(
        buffers.check(theirs, Values::F32(&[2.0]), &Tolerance::Exact, None),
        Err(Mismatch::Unread(foreign.clone())),
        "checked"
    );
    for (from, to) in [(theirs, out), (mine, their_out)] {
        assert_eq!(
            buffers.dispatch(&[Step::Copy { from, to }]),
            Err(foreign.clone()),
            "copied"
        );
    }
    let their_pass = others
        .bind(&square, &[(0, 0, theirs), (0, 1, their_out)])
        .expect("bound to their buffers");
    assert_eq!(
        buffers.dispatch(&[Step::Pass(&their_pass, Workgroups(1, 1, 1))]),
        Err(RunError::BadBuffer(
            "a kernel bound to other buffers".to_owned()
        )),
        "a pass of another set"
    );
    let own = Gpu::open(wgpu::Backends::all(), Limits::WebGpu).expect("a GPU of its own");
    let elsewhere = Kernel::new(&own, SQUARE, "main").expect("a kernel");
    assert_eq!(
        buffers
            .bind(&elsewhere, &[(0, 0, mine), (0, 1, out)])
            .map(drop),
        Err(RunError::BadBuffer(
            "a kernel of another device than the buffers'".to_owned()
        )),
        "a kernel of another device"
    );
}

#[test]
fn refuses_buffers_and_steps_that_do_not_fit() {
    let square = kernel(SQUARE);
    let mut buffers = Buffers::new(gpu());
    assert_eq!(
        gpu().limits().max_buffer_size,
        268_435_456,
        "WebGPU's default largest buffer"
    );
    assert_eq!(
        buffers.add(Data::Bytes(&[1, 2, 3])),
        Err(RunError::BadBuffer(
            "3 bytes: a buffer holds whole 4-byte words".to_owned()
        )),
        "a part of a word"
    );
    assert_eq!(
        buffers.add(Data::Zeroed(268_435_460)),
        Err(RunError::BadBuffer(
            "268435460 bytes: at most 268435456 in one buffer of this device".to_owned()
        )),
        "one word past the largest buffer"
    );
    let largest = Buffers::new(gpu()).add(Data::Zeroed(268_435_456)).map(drop);
    assert_eq!(largest, Ok(()), "the largest buffer itself");
    let input = buffers.add(Data::F32(&[1.0, 2.0])).expect("a buffer");
    let out = buffers.add(Data::Zeroed(8)).expect("a buffer");
    let short = buffers.add(Data::Zeroed(4)).expect("a buffer");
    assert_eq!(
        buffers.write(input, Data::F32(&[1.0])),
        Err(RunError::BadBuffer(
            "4 bytes written to a buffer of 8".to_owned()
        )),
        "fewer bytes"
    );
    assert_eq!(
        buffers.write(input, Data::Zeroed(12)),
        Err(RunError::BadBuffer(
            "12 bytes written to a buffer of 8".to_owned()
        )),
        "more bytes"
    );
    assert_eq!(
        buffers.dispatch(&[Step::Copy {
            from: input,
            to: input
        }]),
        Err(RunError::BadBuffer(
            "a buffer copied onto itself".to_owned()
        )),
        "onto itself"
    );
    assert_eq!(
        buffers.dispatch(&[Step::Copy {
            from: input,
            to: short
        }]),
        Err(RunError::BadBuffer(
            "a buffer of 8 bytes copied onto one of 4".to_owned()
        )),
        "onto a smaller buffer"
    );
    let bound = buffers
        .bind(&square, &[(0, 0, input), (0, 1, out)])
        .expect("bound");
    assert_eq!(
        buffers.dispatch(&[
            Step::Pass(&bound, Workgroups(1, 1, 1)),
            Step::Pass(&bound, Workgroups(1, 65_536, 1)),
        ]),
        Err(RunError::TooManyWorkgroups {
            asked: Workgroups(1, 65_536, 1),
            limit: 65_535
        }),
        "too many workgroups in the second pass"
    );
    assert_eq!(
        buffers.bytes(out),
        Ok(vec![0; 8]),
        "nothing dispatched: every step is checked before any is encoded"
    );
}

#[test]
fn says_the_device_s_refusal_of_each_call_on_a_destroyed_buffer() {
    let square = kernel(SQUARE);
    let mut buffers = Buffers::new(gpu());
    let gone = buffers.add(Data::F32(&[1.0])).expect("a buffer");
    let kept = buffers.add(Data::Zeroed(4)).expect("a buffer");
    let bound = buffers
        .bind(&square, &[(0, 0, gone), (0, 1, kept)])
        .expect("bound");
    buffers.held.first().expect("the first buffer").0.destroy();
    let refused = |error: Result<(), RunError>, call: &str| {
        assert!(
            matches!(
                &error,
                Err(RunError::Device { adapter, why })
                    if why.contains("destroyed") && adapter == gpu().adapter()
            ),
            "{call}: {error:?}"
        );
    };
    refused(buffers.write(gone, Data::F32(&[2.0])), "values written");
    refused(buffers.write(gone, Data::Zeroed(4)), "zeros written");
    refused(
        buffers
            .bind(&square, &[(0, 0, gone), (0, 1, kept)])
            .map(drop),
        "bound",
    );
    refused(
        buffers.dispatch(&[Step::Pass(&bound, Workgroups(1, 1, 1))]),
        "a pass",
    );
    refused(
        buffers.dispatch(&[Step::Copy {
            from: gone,
            to: kept,
        }]),
        "a copy",
    );
    refused(buffers.bytes(gone).map(drop), "read back");
    assert_eq!(
        buffers.check(kept, Values::F32(&[0.0]), &Tolerance::Exact, None),
        Ok(()),
        "the other buffer, untouched by every refused call"
    );
}

#[test]
fn says_the_device_s_refusal_of_a_buffer_once_it_is_destroyed() {
    let own = Gpu::open(wgpu::Backends::all(), Limits::WebGpu).expect("a GPU of its own");
    let mut buffers = Buffers::new(&own);
    own.device.destroy();
    assert!(
        matches!(
            buffers.add(Data::F32(&[1.0])),
            Err(RunError::Device { adapter, why }) if !why.is_empty() && adapter == own.adapter()
        ),
        "a buffer added on a destroyed device"
    );
}

// A regression test of a race, so probabilistic: before the fix (a callback taken as
// fired once the poll returned), it failed about one run in two on an Apple M2 Pro
// (2026-11-19); a thread's poll may fire another thread's mapping callback late.
#[test]
fn reads_back_from_many_threads_sharing_the_device() {
    std::thread::scope(|scope| {
        for thread in 0..16_u32 {
            let _reader = scope.spawn(move || {
                let mut buffers = Buffers::new(gpu());
                let values = [thread, thread.wrapping_add(1)];
                let held = buffers.add(Data::U32(&values)).expect("a buffer");
                for _ in 0..60_u32 {
                    assert_eq!(
                        buffers.check(held, Values::U32(&values), &Tolerance::Exact, None),
                        Ok(()),
                        "thread {thread}: each read gets its own callback"
                    );
                }
            });
        }
    });
}

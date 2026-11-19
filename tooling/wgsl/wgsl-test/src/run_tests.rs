//! Tests of running kernels: values in, one dispatch, written buffers out, bit for bit; and
//! each failure as its typed error, on the shared device and on devices of their own.

use super::{Data, Gpu, Kernel, Limits, Outputs, RunError, Workgroups, block_on};
use crate::compare::{Mismatch, Tolerance, Value, Values};
use crate::layout::Scalar;
use crate::validation::validate;

/// Adds one to each value, scaled by a uniform, one invocation per value.
const ADD: &str = "struct Params { scale: f32, count: u32 }
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < params.count {
        output[id.x] = input[id.x] * params.scale + 1.0;
    }
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

/// The bytes of a uniform `Params { scale, count }`.
fn params(scale: f32, count: u32) -> Vec<u8> {
    [scale.to_bits(), count]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect()
}

/// The refusal of running `kernel` on `given` over one workgroup.
fn refused(kernel: &Kernel<'_>, given: &[super::Given<'_>]) -> RunError {
    kernel
        .run(given, Workgroups(1, 1, 1))
        .expect_err("a refused run")
}

#[test]
fn runs_a_kernel_and_reads_back_what_it_wrote() {
    let kernel = kernel(ADD);
    assert_eq!(
        kernel
            .bindings()
            .iter()
            .map(|binding| (binding.group(), binding.binding()))
            .collect::<Vec<_>>(),
        [(0, 0), (0, 1), (0, 2)],
        "the buffers its shader binds"
    );
    let input: Vec<f32> = (0..100_u16).map(|value| f32::from(value) * 0.5).collect();
    let uniform = params(3.0, 100);
    let outputs = kernel
        .run(
            &[
                (0, 0, Data::Bytes(&uniform)),
                (0, 1, Data::F32(&input)),
                (0, 2, Data::Zeroed(400)),
            ],
            Workgroups(2, 1, 1),
        )
        .expect("a run");
    let expected: Vec<f32> = (0..100_u16)
        .map(|value| f32::from(value) * 1.5 + 1.0)
        .collect();
    assert_eq!(
        outputs.f32s(0, 2).map(|values| values
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()),
        Ok(expected.iter().map(|value| value.to_bits()).collect()),
        "each value scaled, plus one, exactly (every value and product is exact in f32)"
    );
    assert_eq!(
        outputs.bytes(0, 1),
        Err(RunError::NotWritten {
            group: 0,
            binding: 1
        }),
        "a buffer only read is not read back"
    );
    assert_eq!(
        outputs.bytes(0, 0),
        Err(RunError::NotWritten {
            group: 0,
            binding: 0
        }),
        "nor a uniform"
    );
}

#[test]
fn values_the_kernel_does_not_reach_keep_their_bits() {
    let kernel = kernel(ADD);
    let uniform = params(2.0, 2);
    let start = [7.0_f32, 8.0, 9.0, 10.0];
    let outputs = kernel
        .run(
            &[
                (0, 0, Data::Bytes(&uniform)),
                (0, 1, Data::F32(&[1.0, 2.0, 3.0, 4.0])),
                (0, 2, Data::F32(&start)),
            ],
            Workgroups(1, 1, 1),
        )
        .expect("a run");
    assert_eq!(
        outputs.u32s(0, 2),
        Ok([3.0_f32, 5.0, 9.0, 10.0].map(f32::to_bits).to_vec()),
        "the first two computed, the last two as given"
    );
}

#[test]
fn runs_integers_of_both_signs_across_groups() {
    let kernel = kernel(
        "@group(0) @binding(0) var<storage, read> signed: array<i32>;
@group(2) @binding(1) var<storage, read_write> unsigned: array<u32>;
@group(2) @binding(0) var<storage, read_write> negated: array<i32>;

@compute @workgroup_size(4)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    unsigned[id.x] = bitcast<u32>(signed[id.x]);
    negated[id.x] = -signed[id.x];
}
",
    );
    let signed = [0_i32, -1_i32, i32::MAX, i32::MIN + 1_i32];
    let outputs = kernel
        .run(
            &[
                (2, 1, Data::Zeroed(16)),
                (0, 0, Data::I32(&signed)),
                (2, 0, Data::I32(&[5_i32; 4])),
            ],
            Workgroups(1, 1, 1),
        )
        .expect("a run, an empty group 1 between the two");
    assert_eq!(
        outputs.u32s(2, 1),
        Ok(vec![0, u32::MAX, 0x7fff_ffff, 0x8000_0001]),
        "the bits of each value"
    );
    assert_eq!(
        outputs.i32s(2, 0),
        Ok(vec![0_i32, 1_i32, -i32::MAX, i32::MAX]),
        "each value negated"
    );
}

#[test]
fn dispatches_along_every_dimension() {
    let kernel = kernel(
        "@group(0) @binding(0) var<storage, read_write> seen: array<atomic<u32>>;

@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    atomicAdd(&seen[id.x + 3u * (id.y + 2u * id.z)], 1u);
}
",
    );
    let outputs = kernel
        .run(&[(0, 0, Data::Zeroed(4 * 24))], Workgroups(3, 2, 4))
        .expect("a run");
    assert_eq!(
        outputs.u32s(0, 0),
        Ok(vec![1; 24]),
        "each of the 3 x 2 x 4 invocations once"
    );
}

#[test]
fn a_kernel_binding_nothing_runs() {
    let kernel = kernel("@compute @workgroup_size(1)\nfn main() {}\n");
    assert_eq!(kernel.bindings(), [], "no binding");
    assert_eq!(
        kernel.run(&[], Workgroups(1, 1, 1)),
        Ok(Outputs {
            adapter: gpu().adapter().to_owned(),
            buffers: Vec::new()
        }),
        "nothing written"
    );
}

#[test]
fn dispatches_no_workgroup_and_up_to_the_limit() {
    let kernel = kernel(ADD);
    let uniform = params(1.0, 0);
    let given = [
        (0, 0, Data::Bytes(&uniform)),
        (0, 1, Data::F32(&[1.0])),
        (0, 2, Data::F32(&[6.0])),
    ];
    let limit = gpu().limits().max_compute_workgroups_per_dimension;
    assert_eq!(limit, 65_535, "WebGPU's default limit");
    for workgroups in [Workgroups(0, 0, 0), Workgroups(limit, 1, 1)] {
        assert_eq!(
            kernel
                .run(&given, workgroups)
                .map(|outputs| outputs.f32s(0, 2)),
            Ok(Ok(vec![6.0])),
            "{workgroups:?}: no invocation writes, count being 0"
        );
    }
}

#[test]
fn refuses_more_workgroups_than_the_device_dispatches() {
    let kernel = kernel(ADD);
    let limit = gpu().limits().max_compute_workgroups_per_dimension;
    let over = limit.checked_add(1).expect("a limit below u32::MAX");
    for asked in [
        Workgroups(over, 1, 1),
        Workgroups(1, over, 1),
        Workgroups(1, 1, over),
    ] {
        assert_eq!(
            kernel.run(&[], asked),
            Err(RunError::TooManyWorkgroups { asked, limit }),
            "{asked:?}, checked before the buffers"
        );
    }
}

#[test]
fn refuses_a_buffer_the_shader_does_not_bind() {
    let kernel = kernel(ADD);
    for (group, binding) in [(0, 3), (1, 0)] {
        assert_eq!(
            refused(&kernel, &[(group, binding, Data::Zeroed(4))]),
            RunError::BadLayout {
                group,
                binding,
                why: "the shader binds no buffer there".to_owned()
            },
            "group {group} binding {binding}"
        );
    }
}

#[test]
fn refuses_a_binding_given_no_buffer_or_two() {
    let kernel = kernel(ADD);
    let uniform = params(1.0, 1);
    assert_eq!(
        refused(
            &kernel,
            &[(0, 0, Data::Bytes(&uniform)), (0, 2, Data::Zeroed(4))]
        ),
        RunError::BadLayout {
            group: 0,
            binding: 1,
            why: "no buffer given for it".to_owned()
        },
        "the input left out"
    );
    assert_eq!(
        refused(
            &kernel,
            &[
                (0, 0, Data::Bytes(&uniform)),
                (0, 1, Data::F32(&[1.0])),
                (0, 1, Data::F32(&[2.0])),
                (0, 2, Data::Zeroed(4))
            ]
        ),
        RunError::BadLayout {
            group: 0,
            binding: 1,
            why: "given more than once".to_owned()
        },
        "the input given twice"
    );
}

#[test]
fn refuses_a_buffer_that_does_not_fit_its_binding() {
    let kernel = kernel(ADD);
    let uniform = params(1.0, 1);
    let cases = [
        (
            [
                Data::Bytes(&uniform[..4]),
                Data::F32(&[1.0]),
                Data::Zeroed(4),
            ],
            (0, "4 bytes given, 8 expected"),
        ),
        (
            [Data::Bytes(&uniform), Data::U32(&[1]), Data::Zeroed(4)],
            (1, "u32 values given, the shader declares f32"),
        ),
        (
            [Data::Bytes(&uniform), Data::F32(&[]), Data::Zeroed(4)],
            (1, "0 bytes given, 0 then a positive multiple of 4 expected"),
        ),
        (
            [Data::Bytes(&uniform), Data::F32(&[1.0]), Data::Zeroed(6)],
            (2, "6 bytes given, 0 then a positive multiple of 4 expected"),
        ),
        (
            [Data::U32(&[1, 1]), Data::F32(&[1.0]), Data::Zeroed(4)],
            (
                0,
                "u32 values given, the shader declares values of several kinds: give bytes",
            ),
        ),
    ];
    for ([first, second, third], (binding, why)) in cases {
        assert_eq!(
            refused(&kernel, &[(0, 0, first), (0, 1, second), (0, 2, third)]),
            RunError::BadLayout {
                group: 0,
                binding,
                why: why.to_owned()
            },
            "{why}"
        );
    }
}

#[test]
fn refuses_an_invalid_shader_with_the_validation_s_refusal() {
    let source = "@compute @workgroup_size(1)\nfn main() { let x: u32 = 1.5; }\n";
    let invalid = validate(source).expect_err("an invalid shader");
    assert_eq!(
        Kernel::new(gpu(), source, "main").map(|_| ()),
        Err(RunError::InvalidShader(invalid)),
        "the validation's refusal, whole"
    );
}

#[test]
fn refuses_a_kernel_with_no_such_entry_point() {
    assert_eq!(
        Kernel::new(gpu(), ADD, "other").map(|_| ()),
        Err(RunError::BadKernel(
            "no compute entry point named other".to_owned()
        )),
        "an entry point named other"
    );
}

#[test]
fn says_when_the_device_refuses_a_pipeline() {
    // Valid WGSL, but more invocations per workgroup than any device allows.
    let refusal = Kernel::new(
        gpu(),
        "@compute @workgroup_size(1024, 1024, 64)\nfn main() {}\n",
        "main",
    )
    .map(|_| ())
    .expect_err("a refused pipeline");
    assert!(
        matches!(&refusal, RunError::Device { adapter, why } if !why.is_empty() && adapter == gpu().adapter()),
        "the device's reason, naming its adapter: {refusal}"
    );
}

#[test]
fn refuses_a_buffer_larger_than_the_device_binds() {
    let add = kernel(ADD);
    let uniform = params(1.0, 1);
    let storage_limit = 128 << 20_u32;
    for (bytes, refused) in [
        (storage_limit, None),
        (
            storage_limit + 4,
            Some(RunError::TooLarge {
                group: 0,
                binding: 1,
                bytes: storage_limit + 4,
                limit: storage_limit,
            }),
        ),
    ] {
        let outcome = add.run(
            &[
                (0, 0, Data::Bytes(&uniform)),
                (0, 1, Data::Zeroed(bytes)),
                (0, 2, Data::Zeroed(4)),
            ],
            Workgroups(0, 0, 0),
        );
        assert_eq!(
            outcome.err(),
            refused,
            "{bytes} bytes of input, never read back, WebGPU's default storage binding limit being 128 MiB"
        );
    }
    let wide = kernel(
        "@group(0) @binding(0) var<uniform> table: array<vec4<u32>, 4097>;
@group(0) @binding(1) var<storage, read_write> output: u32;
@compute @workgroup_size(1)
fn main() { output = table[4096].x; }
",
    );
    assert_eq!(
        wide.run(
            &[(0, 0, Data::Zeroed(65_552)), (0, 1, Data::Zeroed(4))],
            Workgroups(1, 1, 1)
        )
        .err(),
        Some(RunError::TooLarge {
            group: 0,
            binding: 0,
            bytes: 65_552,
            limit: 65_536,
        }),
        "WebGPU's default uniform binding limit, 64 KiB"
    );
}

#[test]
fn opens_a_device_under_the_limits_asked() {
    let web = Gpu::open(wgpu::Backends::all(), Limits::WebGpu).expect("a GPU");
    assert_eq!(web.limits(), &wgpu::Limits::default(), "WebGPU's defaults");
    assert_eq!(
        web.limits().max_storage_buffer_binding_size,
        128 << 20_u32,
        "128 MiB"
    );
    let own = Gpu::open(wgpu::Backends::all(), Limits::Adapter).expect("a GPU");
    assert_ne!(
        own.limits(),
        web.limits(),
        "the adapter's own, beyond WebGPU's defaults"
    );
    assert!(
        own.limits().max_compute_workgroups_per_dimension >= 65_535,
        "at least WebGPU's defaults"
    );
}

#[test]
fn says_when_the_written_buffers_cannot_be_read_back() {
    let unmappable = gpu().device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let mut read = Ok(Vec::new());
    let refused = gpu().scoped(&mut || read = gpu().read_back(core::slice::from_ref(&unmappable)));
    assert!(refused.is_some(), "the device refuses to map it");
    assert!(
        matches!(&read, Err(RunError::ReadBack { adapter, why }) if !why.is_empty() && adapter == gpu().adapter()),
        "the mapping's reason: {read:?}"
    );
}

#[test]
fn says_the_device_s_refusal_once_it_is_destroyed() {
    let gpu = Gpu::open(wgpu::Backends::all(), Limits::WebGpu).expect("a GPU of its own");
    let kernel = Kernel::new(&gpu, ADD, "main").expect("a kernel");
    gpu.device.destroy();
    let uniform = params(1.0, 1);
    let refusal = kernel
        .run(
            &[
                (0, 0, Data::Bytes(&uniform)),
                (0, 1, Data::F32(&[1.0])),
                (0, 2, Data::Zeroed(4)),
            ],
            Workgroups(1, 1, 1),
        )
        .expect_err("a destroyed device");
    assert!(
        matches!(&refusal, RunError::Device { adapter, why } if !why.is_empty() && adapter == gpu.adapter()),
        "the device's reason, never a panic, naming its adapter: {refusal}"
    );
}

#[test]
fn says_when_no_adapter_answers() {
    assert!(
        matches!(
            Gpu::open(wgpu::Backends::empty(), Limits::WebGpu),
            Err(RunError::NoAdapter(why)) if !why.is_empty()
        ),
        "no API asked for"
    );
}

#[test]
fn says_when_the_adapter_refuses_a_device() {
    let refusal = Gpu::opening(wgpu::Backends::all(), |limits| wgpu::Limits {
        max_bind_groups: u32::MAX,
        ..limits
    })
    .expect_err("limits beyond the adapter's");
    assert!(
        matches!(&refusal, RunError::NoDevice { adapter, why } if !why.is_empty() && adapter == gpu().adapter()),
        "the adapter's reason, naming it (the shared device's): {refusal}"
    );
}

#[test]
fn shares_one_device() {
    let first = Gpu::shared().expect("a GPU");
    let second = Gpu::shared().expect("a GPU");
    assert!(core::ptr::eq(first, second), "the same device");
    assert!(!first.adapter().is_empty(), "an adapter named");
    assert!(
        first.adapter().ends_with(')'),
        "its API in parentheses: {}",
        first.adapter()
    );
}

#[test]
fn transports_the_bits_of_every_value() {
    let floats = [
        f32::from_bits(0x7fc0_1234),
        -0.0,
        f32::INFINITY,
        f32::from_bits(1),
    ];
    assert_eq!(
        Data::F32(&floats).bytes(),
        Some(vec![
            0x34, 0x12, 0xc0, 0x7f, 0, 0, 0, 0x80, 0, 0, 0x80, 0x7f, 1, 0, 0, 0
        ]),
        "a payload, a negative zero, an infinity, the least subnormal, little-endian"
    );
    assert_eq!(
        Data::U32(&[0x0102_0304]).bytes(),
        Some(vec![4, 3, 2, 1]),
        "little-endian"
    );
    assert_eq!(
        Data::I32(&[-2_i32]).bytes(),
        Some(vec![0xfe, 0xff, 0xff, 0xff]),
        "two's complement"
    );
    assert_eq!(Data::Bytes(&[9, 8]).bytes(), Some(vec![9, 8]), "as given");
    assert_eq!(Data::Zeroed(1 << 60_u32).bytes(), None, "nothing to copy");
    let lengths = [
        Data::F32(&floats).len(),
        Data::U32(&[1, 2]).len(),
        Data::I32(&[1_i32, 2_i32, 3_i32]).len(),
        Data::Bytes(&[1]).len(),
        Data::Zeroed(1 << 60_u32).len(),
    ];
    assert_eq!(lengths, [16, 8, 12, 1, 1 << 60_u32], "the bytes of each");
    let outputs = Outputs {
        adapter: "Test GPU (Vulkan)".to_owned(),
        buffers: vec![(
            1,
            2,
            Some(Scalar::F32),
            vec![0x34, 0x12, 0xc0, 0x7f, 0xfe, 0xff, 0xff, 0xff, 7],
        )],
    };
    assert_eq!(
        outputs.f32s(1, 2).map(|values| values
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()),
        Ok(vec![0x7fc0_1234, 0xffff_fffe]),
        "the bits back, payloads included; a trailing byte is no value"
    );
    assert_eq!(
        outputs.i32s(1, 2),
        Ok(vec![0x7fc0_1234_i32, -2_i32]),
        "as i32"
    );
    assert_eq!(
        outputs.u32s(1, 2),
        Ok(vec![0x7fc0_1234, 0xffff_fffe]),
        "as u32"
    );
    assert_eq!(outputs.bytes(1, 2).map(<[u8]>::len), Ok(9), "every byte");
    for (group, binding) in [(1, 1), (0, 2), (2, 2)] {
        let missing = RunError::NotWritten { group, binding };
        assert_eq!(
            outputs.bytes(group, binding),
            Err(missing.clone()),
            "{missing}"
        );
        assert_eq!(
            outputs.u32s(group, binding),
            Err(missing.clone()),
            "{missing}"
        );
        assert_eq!(
            outputs.i32s(group, binding),
            Err(missing.clone()),
            "{missing}"
        );
        assert_eq!(
            outputs.f32s(group, binding),
            Err(missing.clone()),
            "{missing}"
        );
    }
}

#[test]
fn says_each_error_on_one_safe_line() {
    let invalid = validate("fn f(").expect_err("a parse error");
    let cases = [
        (
            RunError::NoAdapter("none\nhere".to_owned()),
            "no GPU adapter: none\\u{a}here",
        ),
        (
            RunError::NoDevice {
                adapter: "GPU\u{1b}".to_owned(),
                why: "limits".to_owned(),
            },
            "no GPU device on GPU\\u{1b}: limits",
        ),
        (
            RunError::InvalidShader(invalid.clone()),
            &format!("invalid shader: {invalid}"),
        ),
        (
            RunError::BadKernel("no \u{1b}entry".to_owned()),
            "bad kernel: no \\u{1b}entry",
        ),
        (
            RunError::BadLayout {
                group: 3,
                binding: 4,
                why: "too\rshort".to_owned(),
            },
            "bad layout: group 3 binding 4: too\\u{d}short",
        ),
        (
            RunError::TooManyWorkgroups {
                asked: Workgroups(70_000, 2, 3),
                limit: 65_535,
            },
            "too many workgroups: 70000x2x3 asked, at most 65535 per dimension",
        ),
        (
            RunError::TooLarge {
                group: 0,
                binding: 2,
                bytes: 134_217_732,
                limit: 134_217_728,
            },
            "too large: group 0 binding 2: 134217732 bytes given, at most 134217728 bound",
        ),
        (
            RunError::NotWritten {
                group: 1,
                binding: 0,
            },
            "not written: group 1 binding 0 is no read_write buffer of the kernel",
        ),
        (
            RunError::Device {
                adapter: "Test GPU (Metal)".to_owned(),
                why: "out\nof memory".to_owned(),
            },
            "the device refused on Test GPU (Metal): out\\u{a}of memory",
        ),
        (
            RunError::BadBuffer("too\nlarge".to_owned()),
            "bad buffer: too\\u{a}large",
        ),
        (
            RunError::ReadBack {
                adapter: "Test\rGPU".to_owned(),
                why: "lost".to_owned(),
            },
            "read-back failed on Test\\u{d}GPU: lost",
        ),
    ];
    for (error, line) in cases {
        assert_eq!(error.to_string(), line, "{error:?}");
    }
}

#[test]
fn waits_for_a_future_that_is_pending_at_first() {
    let mut polls = 0_u32;
    let mut yielded = false;
    block_on(core::pin::pin!(core::future::poll_fn(|_| {
        polls = polls.saturating_add(1);
        if yielded {
            core::task::Poll::Ready(())
        } else {
            yielded = true;
            core::task::Poll::Pending
        }
    })));
    assert_eq!(polls, 2, "polled again after it was pending");
}

#[test]
fn checks_a_buffer_written_against_a_reference() {
    let kernel = kernel(ADD);
    let input = [0.5_f32, 1.0_f32, 2.0_f32];
    let uniform = params(3.0, 3);
    let outputs = kernel
        .run(
            &[
                (0, 0, Data::Bytes(&uniform)),
                (0, 1, Data::F32(&input)),
                (0, 2, Data::Zeroed(12)),
            ],
            Workgroups(1, 1, 1),
        )
        .expect("a run");
    let exact = Tolerance::Exact;
    assert_eq!(
        outputs.check(0, 2, Values::F32(&[2.5, 4.0, 7.0]), &exact, None),
        Ok(()),
        "each value exact (every value and product is exact in f32)"
    );
    let describe = |index: usize| format!("input {:?}", input.get(index));
    assert_eq!(
        outputs
            .check(0, 2, Values::F32(&[2.5, 4.0, 7.5]), &exact, Some(&describe))
            .map_err(|mismatch| mismatch.to_string()),
        Err(format!(
            "first difference at index 2 on {}: expected 7.5 (0x40f00000), actual 7.0 \
             (0x40e00000), bit for bit expected; inputs: input Some(2.0)",
            gpu().adapter()
        )),
        "the first difference on the shared device's adapter, its inputs described by the test"
    );
    assert_eq!(
        outputs.check(0, 1, Values::F32(&input), &exact, None),
        Err(Mismatch::NotWritten {
            group: 0,
            binding: 1
        }),
        "an input is not read back"
    );
    assert_eq!(
        outputs.check(0, 2, Values::U32(&[0, 0, 0]), &exact, None),
        Err(Mismatch::Kinds {
            actual: "f32",
            expected: "u32"
        }),
        "the shader declares f32"
    );
}

#[test]
fn checks_each_scalar_and_bytes_against_what_the_shader_declares() {
    let bytes = vec![1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
    let outputs = Outputs {
        adapter: "Test\nGPU (Vulkan)".to_owned(),
        buffers: vec![
            (0, 0, Some(Scalar::U32), bytes.clone()),
            (0, 1, Some(Scalar::I32), bytes.clone()),
            (0, 2, Some(Scalar::F32), bytes.clone()),
            (0, 3, None, bytes.clone()),
        ],
    };
    let exact = Tolerance::Exact;
    assert_eq!(
        outputs.check(0, 0, Values::U32(&[1, u32::MAX]), &exact, None),
        Ok(()),
        "as u32"
    );
    assert_eq!(
        outputs
            .check(0, 0, Values::U32(&[1, 0]), &exact, None)
            .map_err(|mismatch| mismatch.to_string()),
        Err(
            "first difference at index 1 on Test\\u{a}GPU (Vulkan): expected 0 (0x00000000), \
             actual 4294967295 (0xffffffff), bit for bit expected"
                .to_owned()
        ),
        "the adapter named, on one line"
    );
    assert_eq!(
        outputs.check(0, 1, Values::I32(&[1_i32, -1_i32]), &exact, None),
        Ok(()),
        "as i32"
    );
    assert_eq!(
        outputs.check(0, 1, Values::I32(&[1_i32, 1_i32]), &exact, None),
        Err(Mismatch::Differs {
            index: 1,
            expected: Value::I32(1),
            actual: Value::I32(-1),
            apart: None,
            inputs: None,
            adapter: Some("Test\nGPU (Vulkan)".to_owned())
        }),
        "as i32, the second differing, on the adapter that computed it"
    );
    assert_eq!(
        outputs
            .check(
                0,
                2,
                Values::F32(&[f32::from_bits(1), f32::from_bits(u32::MAX)]),
                &exact,
                None
            )
            .map_err(|mismatch| mismatch.to_string()),
        Ok(()),
        "as f32, a subnormal and a not-a-number, bit for bit"
    );
    for at in 0..4_u32 {
        assert_eq!(
            outputs.check(0, at, Values::Bytes(&bytes), &exact, None),
            Ok(()),
            "bytes compare with binding {at}"
        );
    }
    assert_eq!(
        outputs.check(0, 3, Values::U32(&[1, u32::MAX]), &exact, None),
        Err(Mismatch::Kinds {
            actual: "mixed",
            expected: "u32"
        }),
        "mixed members compare as bytes only"
    );
    for (expected, kind) in [
        (Values::I32(&[1_i32, -1_i32]), "i32"),
        (Values::F32(&[1.0, 2.0]), "f32"),
    ] {
        assert_eq!(
            outputs.check(0, 3, expected, &exact, None),
            Err(Mismatch::Kinds {
                actual: "mixed",
                expected: kind
            }),
            "{kind} against mixed members"
        );
    }
}

#[test]
fn runs_the_same_kernels_on_every_adapter_of_the_machine() {
    let every = Gpu::every(
        wgpu::Backends::from_env().unwrap_or_default(),
        Limits::WebGpu,
    );
    let mut adapters = Vec::new();
    for opened in &every {
        let device = opened.as_ref().expect("a device on each adapter");
        assert_eq!(
            device.limits(),
            &wgpu::Limits::default(),
            "under WebGPU's limits: {}",
            device.adapter()
        );
        let add = Kernel::new(device, ADD, "main").expect("a kernel");
        let uniform = params(3.0, 3);
        let outputs = add
            .run(
                &[
                    (0, 0, Data::Bytes(&uniform)),
                    (0, 1, Data::F32(&[0.5, 1.0, 2.0])),
                    (0, 2, Data::Zeroed(12)),
                ],
                Workgroups(1, 1, 1),
            )
            .expect("a run");
        assert_eq!(
            outputs.check(0, 2, Values::F32(&[2.5, 4.0, 7.0]), &Tolerance::Exact, None),
            Ok(()),
            "the same bits on {} (every value and product is exact in f32)",
            device.adapter()
        );
        adapters.push(device.adapter().to_owned());
    }
    assert!(
        adapters.iter().any(|adapter| adapter == gpu().adapter()),
        "the shared device's adapter among them: {adapters:?}"
    );
    for device in Gpu::every(
        wgpu::Backends::from_env().unwrap_or_default(),
        Limits::Adapter,
    )
    .iter()
    .map(|opened| opened.as_ref().expect("a device on each adapter"))
    {
        assert_ne!(
            device.limits(),
            &wgpu::Limits::default(),
            "the adapter's own limits: {}",
            device.adapter()
        );
    }
}

#[test]
fn opens_no_device_where_no_adapter_answers() {
    assert!(
        Gpu::every(wgpu::Backends::empty(), Limits::WebGpu).is_empty(),
        "no API asked for"
    );
}

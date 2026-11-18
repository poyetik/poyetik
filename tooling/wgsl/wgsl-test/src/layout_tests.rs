//! Tests of the bindings read from a shader: each kind of buffer, its size and its scalar,
//! the globals an entry point does not use left out, and each refusal of a buffer given.

use super::{Access, Binding, Scalar, Size, bindings, scalar_of, size_of};
use naga::ScalarKind;

use crate::validation::validated;

/// A shader binding one buffer of each kind, declared out of order, with globals of every
/// size shape, one the entry point does not use, and globals that are no binding.
const SHADER: &str = "struct Params { count: u32, scale: f32 }
struct Words { header: vec4<u32>, items: array<vec4<u32>> }
struct Pair { low: i32, high: i32 }

@group(1) @binding(3) var<storage, read_write> words: Words;
@group(0) @binding(2) var<storage, read> values: array<f32>;
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> pairs: array<Pair, 4>;
@group(0) @binding(5) var<storage, read_write> counter: atomic<u32>;
@group(0) @binding(6) var<uniform> matrix: mat2x2<f32>;
@group(2) @binding(0) var<storage, read_write> unused: array<u32>;
var<workgroup> shared_count: u32;
var<private> step: f32 = 1.0;

@compute @workgroup_size(1)
fn main() {
    shared_count = 1u;
    let scaled = values[0] * params.scale + f32(pairs[0].low) + matrix[0][0] + step;
    words.items[0] = vec4<u32>(u32(scaled), params.count, words.header.x, 0u);
    atomicAdd(&counter, 1u);
}

@compute @workgroup_size(1)
fn other() {
    unused[0] = 1u;
}

@vertex
fn shade() -> @builtin(position) vec4<f32> {
    return vec4<f32>(0.0);
}
";

/// The module of `source` and what naga's validator learned of it; `source` must be valid.
fn parsed(source: &str) -> (naga::Module, naga::valid::ModuleInfo) {
    validated(source).expect("a valid shader")
}

/// The bindings of `entry` in `source`, which must be valid.
fn bindings_of(source: &str, entry: &str) -> Result<Vec<Binding>, String> {
    let (module, info) = parsed(source);
    bindings(&module, &info, entry)
}

/// The one binding of `main` in `source`.
fn only_binding(source: &str) -> Binding {
    let found = bindings_of(source, "main").expect("a kernel");
    assert_eq!(found.len(), 1, "one binding: {found:?}");
    found.into_iter().next().expect("one binding")
}

#[test]
fn reads_each_used_buffer_in_group_then_binding_order() {
    let found = bindings_of(SHADER, "main").expect("a kernel");
    let summary: Vec<(u32, u32, Access, Size, Option<Scalar>)> = found
        .iter()
        .map(|binding| {
            (
                binding.group(),
                binding.binding(),
                binding.access(),
                binding.size(),
                binding.scalar(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (0, 0, Access::Uniform, Size::Fixed(8), None),
            (0, 1, Access::Read, Size::Fixed(32), Some(Scalar::I32)),
            (
                0,
                2,
                Access::Read,
                Size::Runtime { base: 0, stride: 4 },
                Some(Scalar::F32)
            ),
            (0, 5, Access::ReadWrite, Size::Fixed(4), Some(Scalar::U32)),
            (0, 6, Access::Uniform, Size::Fixed(16), Some(Scalar::F32)),
            (
                1,
                3,
                Access::ReadWrite,
                Size::Runtime {
                    base: 16,
                    stride: 16
                },
                Some(Scalar::U32)
            ),
        ],
        "the buffers main binds, the unused one left out"
    );
    let other = bindings_of(SHADER, "other").expect("a kernel");
    assert_eq!(
        other
            .iter()
            .map(|binding| (binding.group(), binding.binding()))
            .collect::<Vec<_>>(),
        [(2, 0)],
        "each entry point binds what it uses"
    );
}

#[test]
fn refuses_an_entry_point_that_is_not_a_compute_one() {
    assert_eq!(
        bindings_of(SHADER, "missing"),
        Err("no compute entry point named missing".to_owned()),
        "no entry point of that name"
    );
    assert_eq!(
        bindings_of(SHADER, "shade"),
        Err("no compute entry point named shade".to_owned()),
        "a vertex entry point"
    );
}

#[test]
fn refuses_a_binding_that_is_not_a_buffer() {
    let source = "@group(0) @binding(4) var image: texture_storage_2d<rgba8unorm, write>;
@compute @workgroup_size(1)
fn main() {
    textureStore(image, vec2<i32>(0, 0), vec4<f32>(0.0));
}
";
    assert_eq!(
        bindings_of(source, "main"),
        Err("binding 4 of group 0 is not a buffer: the harness binds buffers only".to_owned()),
        "a storage texture"
    );
}

#[test]
fn a_kernel_may_bind_nothing() {
    assert_eq!(
        bindings_of("@compute @workgroup_size(1)\nfn main() {}\n", "main"),
        Ok(Vec::new()),
        "no binding"
    );
}

#[test]
fn a_fixed_size_takes_exactly_its_bytes() {
    let binding = only_binding(
        "@group(0) @binding(0) var<storage, read_write> data: array<u32, 3>;
@compute @workgroup_size(1)
fn main() { data[0] = 1u; }
",
    );
    assert_eq!(binding.size(), Size::Fixed(12), "three words");
    assert_eq!(
        binding.refuses(12, Some(Scalar::U32)),
        None,
        "exactly its bytes"
    );
    assert_eq!(binding.refuses(12, None), None, "as bytes too");
    for wrong in [0, 8, 11, 13, 16] {
        assert_eq!(
            binding.refuses(wrong, None),
            Some(format!("{wrong} bytes given, 12 expected")),
            "{wrong} bytes"
        );
    }
}

#[test]
fn a_runtime_size_takes_its_base_then_whole_elements() {
    let binding = only_binding(
        "struct Data { first: vec4<f32>, rest: array<vec4<f32>> }
@group(0) @binding(0) var<storage, read_write> data: Data;
@compute @workgroup_size(1)
fn main() { data.rest[0] = data.first; }
",
    );
    assert_eq!(
        binding.size(),
        Size::Runtime {
            base: 16,
            stride: 16
        },
        "a header, then elements"
    );
    for right in [32, 48, 16 + 16 * 1000] {
        assert_eq!(
            binding.refuses(right, Some(Scalar::F32)),
            None,
            "{right} bytes"
        );
    }
    for wrong in [0, 15, 16, 20, 33, 40] {
        assert_eq!(
            binding.refuses(wrong, None),
            Some(format!(
                "{wrong} bytes given, 16 then a positive multiple of 16 expected"
            )),
            "{wrong} bytes: below the header, the header alone, or a part of an element"
        );
    }
}

#[test]
fn a_size_with_no_stride_takes_nothing() {
    let size = Size::Runtime { base: 0, stride: 0 };
    assert_eq!(
        size.refuses(4),
        Some("4 bytes given, 0 then a positive multiple of 0 expected".to_owned()),
        "a stride of zero divides nothing"
    );
}

#[test]
fn the_values_given_must_be_of_the_shader_s_scalar() {
    let binding = only_binding(
        "@group(0) @binding(0) var<storage, read_write> data: array<i32>;
@compute @workgroup_size(1)
fn main() { data[0] = 1; }
",
    );
    assert_eq!(binding.refuses(4, Some(Scalar::I32)), None, "its scalar");
    for (given, name) in [(Scalar::U32, "u32"), (Scalar::F32, "f32")] {
        assert_eq!(
            binding.refuses(4, Some(given)),
            Some(format!("{name} values given, the shader declares i32")),
            "{name} values"
        );
    }
    assert_eq!(
        binding.refuses(6, Some(Scalar::I32)),
        Some("6 bytes given, 0 then a positive multiple of 4 expected".to_owned()),
        "the size is checked once the scalar fits"
    );
}

#[test]
fn a_structure_of_mixed_members_takes_bytes_only() {
    let binding = only_binding(
        "struct Mixed { count: u32, scale: f32 }
@group(0) @binding(0) var<storage, read_write> data: Mixed;
@compute @workgroup_size(1)
fn main() { data.count = 1u; }
",
    );
    assert_eq!(binding.scalar(), None, "several kinds");
    assert_eq!(binding.refuses(8, None), None, "bytes");
    assert_eq!(
        binding.refuses(8, Some(Scalar::U32)),
        Some(
            "u32 values given, the shader declares values of several kinds: give bytes".to_owned()
        ),
        "values of one kind"
    );
}

#[test]
fn a_structure_of_one_scalar_takes_its_values() {
    let binding = only_binding(
        "struct Same { low: u32, high: vec2<u32> }
@group(0) @binding(0) var<storage, read_write> data: Same;
@compute @workgroup_size(1)
fn main() { data.low = 1u; }
",
    );
    assert_eq!(binding.scalar(), Some(Scalar::U32), "one kind");
    assert_eq!(
        binding.size(),
        Size::Fixed(16),
        "the structure's span, padding included"
    );
}

#[test]
fn a_scalar_other_than_the_three_has_none() {
    for (kind, width) in [
        (ScalarKind::Float, 2),
        (ScalarKind::Float, 8),
        (ScalarKind::Uint, 8),
        (ScalarKind::Sint, 8),
        (ScalarKind::Bool, 1),
    ] {
        assert_eq!(Scalar::of(kind, width), None, "{kind:?} of {width} bytes");
    }
}

#[test]
fn names_each_scalar() {
    assert_eq!(
        [Scalar::U32, Scalar::I32, Scalar::F32].map(|scalar| scalar.to_string()),
        ["u32", "i32", "f32"],
        "the names WGSL gives them"
    );
}

#[test]
fn a_type_of_another_module_has_no_size_nor_scalar() {
    let (module, _) = validated(
        "@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@compute @workgroup_size(1)
fn main() { data[0] = 1u; }
",
    )
    .expect("a valid shader");
    let (_, global) = module.global_variables.iter().next().expect("a global");
    let empty = naga::Module::default();
    assert_eq!(size_of(&empty, global.ty), Size::Fixed(0), "no such type");
    assert_eq!(scalar_of(&empty, global.ty), None, "no such type");
}

#[test]
fn a_type_that_is_no_buffer_has_no_scalar() {
    let (module, _) = parsed(
        "@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var pick: sampler;
@compute @workgroup_size(1)
fn main() {}
",
    );
    for (_, global) in module.global_variables.iter() {
        assert_eq!(scalar_of(&module, global.ty), None, "{:?}", global.name);
    }
}

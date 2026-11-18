//! The bindings of a compute kernel, read from the shader itself, and the check of the
//! buffers a test gives it against them, before anything reaches the GPU.
//!
//! Read from naga's module rather than declared by hand: what the shader says is what the
//! test must give, and a test that disagrees with its shader is refused with the binding
//! named, not run.

use core::fmt;

use naga::valid::ModuleInfo;
use naga::{AddressSpace, ArraySize, Module, ScalarKind, StorageAccess, TypeInner};

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;

/// How a kernel reaches a buffer it binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// A uniform buffer (`var<uniform>`): read only, a fixed size.
    Uniform,
    /// A storage buffer the kernel only reads (`var<storage, read>`).
    Read,
    /// A storage buffer the kernel may write (`var<storage, read_write>`): read back after
    /// the dispatch.
    ReadWrite,
}

/// The bytes a binding holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// Exactly this many bytes.
    Fixed(u64),
    /// A runtime-sized array, alone or as the last member of a structure: `base` bytes
    /// before it (0 for a bare array), then one or more elements of `stride` bytes.
    Runtime {
        /// The bytes before the array.
        base: u64,
        /// The bytes of one element, its padding included.
        stride: u64,
    },
}

impl Size {
    /// Why `bytes` bytes do not fit this size, if they do not.
    fn refuses(self, bytes: u64) -> Option<String> {
        match self {
            Self::Fixed(size) => {
                (bytes != size).then(|| format!("{bytes} bytes given, {size} expected"))
            }
            Self::Runtime { base, stride } => {
                let elements = bytes.checked_sub(base).filter(|after| *after > 0);
                match elements {
                    Some(after) if after.checked_rem(stride) == Some(0) => None,
                    _ => Some(format!(
                        "{bytes} bytes given, {base} then a positive multiple of {stride} expected"
                    )),
                }
            }
        }
    }
}

/// The 32-bit scalar every value of a binding is made of, when it is made of one kind only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    /// `u32`.
    U32,
    /// `i32`.
    I32,
    /// `f32`.
    F32,
}

impl Scalar {
    /// The scalar of naga's `kind` of `width` bytes, if it is one of the three.
    const fn of(kind: ScalarKind, width: u8) -> Option<Self> {
        match (kind, width) {
            (ScalarKind::Uint, 4) => Some(Self::U32),
            (ScalarKind::Sint, 4) => Some(Self::I32),
            (ScalarKind::Float, 4) => Some(Self::F32),
            _ => None,
        }
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::U32 => "u32",
            Self::I32 => "i32",
            Self::F32 => "f32",
        })
    }
}

/// One buffer a kernel binds, as its shader declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// Its bind group (`@group`).
    group: u32,
    /// Its binding in the group (`@binding`).
    slot: u32,
    /// How the kernel reaches it.
    access: Access,
    /// The bytes it holds.
    size: Size,
    /// The one scalar its values are made of, if they are made of one of `u32`, `i32` and
    /// `f32` only (a structure of mixed members has none: it is given as bytes).
    scalar: Option<Scalar>,
}

impl Binding {
    /// Its bind group (`@group`).
    #[must_use]
    pub const fn group(&self) -> u32 {
        self.group
    }

    /// Its binding in the group (`@binding`).
    #[must_use]
    pub const fn binding(&self) -> u32 {
        self.slot
    }

    /// How the kernel reaches it.
    #[must_use]
    pub const fn access(&self) -> Access {
        self.access
    }

    /// The bytes it holds.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// The one scalar its values are made of, if any.
    #[must_use]
    pub const fn scalar(&self) -> Option<Scalar> {
        self.scalar
    }

    /// Why a buffer of `bytes` bytes, of values of `scalar` (none for raw bytes), does not fit
    /// this binding, if it does not.
    pub(crate) fn refuses(&self, bytes: u64, scalar: Option<Scalar>) -> Option<String> {
        match (scalar, self.scalar) {
            (Some(given), Some(declared)) if given != declared => Some(format!(
                "{given} values given, the shader declares {declared}"
            )),
            (Some(given), None) => Some(format!(
                "{given} values given, the shader declares values of several kinds: give bytes"
            )),
            _ => self.size.refuses(bytes),
        }
    }
}

/// The scalar every value of the type `ty` of `module` is made of, if one of the three.
fn scalar_of(module: &Module, ty: naga::Handle<naga::Type>) -> Option<Scalar> {
    match module.types.get_handle(ty).ok()?.inner {
        TypeInner::Scalar(scalar)
        | TypeInner::Atomic(scalar)
        | TypeInner::Vector { scalar, .. }
        | TypeInner::Matrix { scalar, .. } => Scalar::of(scalar.kind, scalar.width),
        TypeInner::Array { base, .. } => scalar_of(module, base),
        TypeInner::Struct { ref members, .. } => {
            let mut kinds = members.iter().map(|member| scalar_of(module, member.ty));
            let first = kinds.next()??;
            kinds.all(|kind| kind == Some(first)).then_some(first)
        }
        TypeInner::CooperativeMatrix { .. }
        | TypeInner::Pointer { .. }
        | TypeInner::ValuePointer { .. }
        | TypeInner::Image { .. }
        | TypeInner::Sampler { .. }
        | TypeInner::AccelerationStructure { .. }
        | TypeInner::RayQuery { .. }
        | TypeInner::BindingArray { .. } => None,
    }
}

/// The bytes the type `ty` of `module` holds as a buffer.
fn size_of(module: &Module, ty: naga::Handle<naga::Type>) -> Size {
    let Ok(declared) = module.types.get_handle(ty) else {
        return Size::Fixed(0);
    };
    match declared.inner {
        TypeInner::Array {
            size: ArraySize::Dynamic,
            stride,
            ..
        } => Size::Runtime {
            base: 0,
            stride: u64::from(stride),
        },
        TypeInner::Struct { ref members, span } => members
            .last()
            .and_then(|last| match size_of(module, last.ty) {
                Size::Runtime { stride, .. } => Some(Size::Runtime {
                    base: u64::from(last.offset),
                    stride,
                }),
                Size::Fixed(_) => None,
            })
            .unwrap_or_else(|| Size::Fixed(u64::from(span))),
        ref inner @ (TypeInner::Array { .. }
        | TypeInner::Scalar(_)
        | TypeInner::Vector { .. }
        | TypeInner::Matrix { .. }
        | TypeInner::CooperativeMatrix { .. }
        | TypeInner::Atomic(_)
        | TypeInner::Pointer { .. }
        | TypeInner::ValuePointer { .. }
        | TypeInner::Image { .. }
        | TypeInner::Sampler { .. }
        | TypeInner::AccelerationStructure { .. }
        | TypeInner::RayQuery { .. }
        | TypeInner::BindingArray { .. }) => {
            Size::Fixed(u64::from(inner.try_size(module.to_ctx()).unwrap_or(0)))
        }
    }
}

/// The buffers the entry point `entry` of a validated `module` binds, in group then binding
/// order: every uniform and storage global it uses.
///
/// # Errors
///
/// Returns why the entry point cannot be run as a compute kernel: no compute entry point of
/// that name, or a binding other than a buffer (a texture, a sampler), which the harness does
/// not bind yet.
pub(crate) fn bindings(
    module: &Module,
    info: &ModuleInfo,
    entry: &str,
) -> Result<Vec<Binding>, String> {
    let (index, _) = module
        .entry_points
        .iter()
        .enumerate()
        .find(|(_, point)| point.name == entry && point.stage == naga::ShaderStage::Compute)
        .ok_or_else(|| format!("no compute entry point named {entry}"))?;
    let used = info.get_entry_point(index);
    let mut found = Vec::new();
    for (handle, global) in module.global_variables.iter() {
        let Some(resource) = global.binding.as_ref() else {
            continue;
        };
        if used[handle].is_empty() {
            continue;
        }
        let access = match global.space {
            AddressSpace::Uniform => Access::Uniform,
            AddressSpace::Storage { access } if access.contains(StorageAccess::STORE) => {
                Access::ReadWrite
            }
            AddressSpace::Storage { .. } => Access::Read,
            AddressSpace::Function
            | AddressSpace::Private
            | AddressSpace::WorkGroup
            | AddressSpace::Handle
            | AddressSpace::Immediate
            | AddressSpace::TaskPayload
            | AddressSpace::RayPayload
            | AddressSpace::IncomingRayPayload => {
                return Err(format!(
                    "binding {} of group {} is not a buffer: the harness binds buffers only",
                    resource.binding, resource.group
                ));
            }
        };
        found.push(Binding {
            group: resource.group,
            slot: resource.binding,
            access,
            size: size_of(module, global.ty),
            scalar: scalar_of(module, global.ty),
        });
    }
    found.sort_by_key(|binding| (binding.group, binding.slot));
    Ok(found)
}

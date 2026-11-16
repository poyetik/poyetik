// The square's pixels on the GPU, one source for every GPU interface (wgpu natively, WebGPU
// in browsers). Where the square is comes from the CPU, exactly (`Square::placement`); each
// pixel's value is the exact area the square covers, computed in 32-bit integers as the CPU
// reference does, then converted to a float without rounding (a coverage of at most 2^24,
// scaled by 2^-24, a power of two): the CPU reference's bits, on any GPU that follows WGSL's
// integer and conversion rules.

struct Placement {
    // The square, in subpixel units (4096 per pixel side): [left, right) x [top, bottom).
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
    // The image's width, in pixels.
    width: u32,
    // The rows this dispatch renders: from `first_row`, `rows` of them.
    first_row: u32,
    rows: u32,
}

@group(0) @binding(0) var<uniform> placement: Placement;
@group(0) @binding(1) var<storage, read_write> values: array<vec4<f32>>;

// Subpixel units per pixel side.
const SUBPIXELS: u32 = 4096u;

// 2^-24: the intensity of one subpixel unit of area, a power of two.
const UNIT_AREA: f32 = 5.9604644775390625e-8;

// The length of the overlap of [start, end) with the cell [cell, cell + SUBPIXELS).
fn overlap(start: u32, end: u32, cell: u32) -> u32 {
    let low = max(start, cell);
    let high = min(end, cell + SUBPIXELS);
    return select(0u, high - low, high > low);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= placement.width || id.y >= placement.rows) {
        return;
    }
    let row = placement.first_row + id.y;
    let coverage = overlap(placement.left, placement.right, id.x * SUBPIXELS)
        * overlap(placement.top, placement.bottom, row * SUBPIXELS);
    let value = f32(coverage) * UNIT_AREA;
    values[id.y * placement.width + id.x] = vec4<f32>(value, value, value, 1.0);
}

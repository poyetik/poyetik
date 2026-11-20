// The display encoding of an image on the GPU (spec/visual-design.md, section 9, "Display encoding"),
// one source for every GPU interface (wgpu natively, WebGPU in browsers): the bytes
// `visual_core::pixel8` gives, exactly, on any GPU that follows WGSL's integer rules.
//
// Every value is read as its bits and every step is in 32-bit integers: no float is loaded,
// compared, multiplied or divided, so that neither a GPU's division (which WGSL lets err by
// 2.5 units in the last place), nor its flushing of subnormals, nor a compiler's fast math
// can change a code. The thresholds come from the library (`SRGB8_THRESHOLDS` then
// `ALPHA8_THRESHOLDS`, as bits), never retyped.

struct Params {
    // The pixels to encode.
    count: u32,
    // The pixels of one row of the dispatch: an image larger than one dimension of
    // workgroups allows is dispatched in rows of workgroups.
    stride: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
// The image's values, four a pixel (red, green, blue, alpha), as bits.
@group(0) @binding(1) var<storage, read> values: array<vec4<u32>>;
// The 255 sRGB thresholds, then the 255 alpha thresholds, as bits.
@group(0) @binding(2) var<storage, read> thresholds: array<u32, 510>;
// Each pixel's four codes: red in the low byte, then green, blue and alpha.
@group(0) @binding(3) var<storage, read_write> pixels: array<u32>;

// The sign bit.
const SIGN: u32 = 0x80000000u;
// The bits of positive infinity: above them, NaNs.
const INFINITY: u32 = 0x7f800000u;
// The bits of 1: an opaque alpha.
const ONE: u32 = 0x3f800000u;
// Where the alpha thresholds start.
const ALPHA: u32 = 255u;

// Whether `bits` are a NaN's.
fn is_nan(bits: u32) -> bool {
    return (bits & 0x7fffffffu) > INFINITY;
}

// How many of the 255 thresholds from `first` (0 for sRGB, ALPHA for alpha) are at or below
// the float of `bits`, of positive sign and not NaN: for such floats, the order of their bits
// is their order, so that a binary search on the bits counts them.
fn at_or_below(bits: u32, first: u32) -> u32 {
    var low = 0u;
    var high = 255u;
    while (low < high) {
        let middle = (low + high) / 2u;
        if (thresholds[first + middle] <= bits) {
            low = middle + 1u;
        } else {
            high = middle;
        }
    }
    return low;
}

// The 8-bit code of the alpha `bits`: how many alpha thresholds are at or below it.
fn alpha_code(bits: u32) -> u32 {
    if ((bits & SIGN) != 0u || is_nan(bits)) {
        return 0u;
    }
    return at_or_below(bits, ALPHA);
}

// A positive finite float as an integer times a power of two, exactly.
struct Exact {
    significand: u32,
    exponent: i32,
}

// The float of `bits` (positive, finite) as `significand × 2^exponent`, subnormals included.
fn exact(bits: u32) -> Exact {
    let field = (bits >> 23u) & 0xffu;
    let fraction = bits & 0x7fffffu;
    if (field == 0u) {
        return Exact(fraction, -149);
    }
    return Exact(fraction | 0x800000u, i32(field) - 150);
}

// `x × y` for `x` and `y` below 2^24, as its high and low 32-bit halves, by 16-bit parts:
// the high parts are below 2^8, so no partial product nor their sum overflows.
fn wide(x: u32, y: u32) -> vec2<u32> {
    let x0 = x & 0xffffu;
    let x1 = x >> 16u;
    let y0 = y & 0xffffu;
    let y1 = y >> 16u;
    let middle = x0 * y1 + x1 * y0;
    let low = x0 * y0 + (middle << 16u);
    let carry = select(0u, 1u, low < x0 * y0);
    return vec2<u32>(x1 * y1 + (middle >> 16u) + carry, low);
}

// Whether `threshold × alpha ≤ color` exactly, for a threshold, an alpha whose code is not
// 0 and a positive finite nonzero color. The threshold and the alpha are normal:
// the smallest threshold is above 2^-13, the smallest such alpha above 2^-9; so their
// significands' product is from 2^46 to below 2^48, in units of 2^(t + a), their exponents'
// sum, at least -68.
fn reaches(threshold: u32, alpha: u32, color: u32) -> bool {
    let t = exact(threshold);
    let a = exact(alpha);
    let c = exact(color);
    let product = wide(t.significand, a.significand);
    let shift = c.exponent - (t.exponent + a.exponent);
    // The color is below 2^24 units: below the product.
    if (shift < 0) {
        return false;
    }
    // Its exponent at least -68, the color is normal (its significand from 2^23): from
    // 2^48 units on, above the product.
    let up = u32(shift);
    if (up >= 25u) {
        return true;
    }
    // Below 2^48 units: its significand times 2^up, as two 32-bit halves.
    let high = select(0u, c.significand >> (32u - max(up, 1u)), up > 0u);
    let low = c.significand << up;
    return product.x < high || (product.x == high && product.y <= low);
}

// The 8-bit sRGB code of `color / alpha`, the largest float at or below the real quotient
// (`visual_core::straight8`): how many thresholds `t` have `t × alpha ≤ color`, exactly, for
// an alpha whose code is not 0: positive, at least the first alpha threshold, possibly infinite.
fn color_code(color: u32, alpha: u32) -> u32 {
    if ((color & SIGN) != 0u || is_nan(color) || color == 0u) {
        return 0u;
    }
    if (color == INFINITY) {
        return 255u;
    }
    if (alpha == INFINITY) {
        return 0u;
    }
    // An opaque color is its own straight value (`visual_core::straight8` does the same): a
    // threshold times 1 is at most the color exactly when the threshold is, which the bits
    // decide, with no product.
    if (alpha == ONE) {
        return at_or_below(color, 0u);
    }
    var low = 0u;
    var high = 255u;
    while (low < high) {
        let middle = (low + high) / 2u;
        if (reaches(thresholds[middle], alpha, color)) {
            low = middle + 1u;
        } else {
            high = middle;
        }
    }
    return low;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.y * params.stride + id.x;
    if (pixel >= params.count) {
        return;
    }
    let value = values[pixel];
    let opacity = alpha_code(value.w);
    var packed = 0u;
    if (opacity != 0u) {
        packed = color_code(value.x, value.w)
            | (color_code(value.y, value.w) << 8u)
            | (color_code(value.z, value.w) << 16u)
            | (opacity << 24u);
    }
    pixels[pixel] = packed;
}

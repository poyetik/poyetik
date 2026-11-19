//! The display corpus: values chosen to reach every path of the display encoding, and the
//! bytes the native encoding gives them, which `bless` writes to `display.json` beside the
//! references and `check` compares, so that a target encoding them otherwise (WebAssembly,
//! `bindings/visual-wasm`'s conformance tests) is caught.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use visual_core::{ALPHA8_THRESHOLDS, SRGB8_THRESHOLDS, pixel8};

/// The file of the display corpus in a conformance directory.
pub const DISPLAY_FILE: &str = "display.json";

/// Special values, as bits: both zeros, the smallest and largest subnormals and the
/// smallest normal (each of both signs), a half, 1 and the float above it, 2, the largest
/// float, both infinities, a quiet and a signaling NaN, a NaN of negative sign, and -1.
const SPECIALS: [u32; 19] = [
    0x0000_0000,
    0x8000_0000,
    0x0000_0001,
    0x8000_0001,
    0x007f_ffff,
    0x807f_ffff,
    0x0080_0000,
    0x8080_0000,
    0x3f00_0000,
    0x3f80_0000,
    0x3f80_0001,
    0x4000_0000,
    0x7f7f_ffff,
    0x7f80_0000,
    0xff80_0000,
    0x7fc0_0000,
    0x7f80_0001,
    0xffc0_0000,
    0xbf80_0000,
];

/// `value` and the floats just below and just above it.
const fn around(value: f32) -> [f32; 3] {
    [value.next_down(), value, value.next_up()]
}

/// The pixels of the corpus, four values each (red, green, blue, alpha), in its order.
///
/// - Opaque: every sRGB threshold, the float below it and the float above it, one in each
///   color channel.
/// - Translucent: every alpha threshold and its two neighbors as the alpha, each with a
///   color at the exact product of that alpha and an sRGB threshold (a code that rounds
///   there) and its neighbors, the sRGB threshold taken in turn.
/// - Special: each special value in each channel of a translucent pixel, and as all four.
#[must_use]
pub fn display_corpus() -> Vec<[f32; 4]> {
    let mut pixels = Vec::new();
    for threshold in SRGB8_THRESHOLDS {
        let [below, at, above] = around(threshold);
        pixels.push([below, at, above, 1.0]);
    }
    for (alpha_threshold, color_threshold) in ALPHA8_THRESHOLDS.iter().zip(SRGB8_THRESHOLDS) {
        for alpha in around(*alpha_threshold) {
            let [below, at, above] = around(alpha * color_threshold);
            pixels.push([below, at, above, alpha]);
        }
    }
    for special in SPECIALS.map(f32::from_bits) {
        pixels.push([special, 0.25, 0.125, 0.5]);
        pixels.push([0.25, special, 0.125, 0.5]);
        pixels.push([0.25, 0.125, special, 0.5]);
        pixels.push([0.25, 0.125, 0.0625, special]);
        pixels.push([special; 4]);
    }
    pixels
}

/// The display corpus file of `pixels`, exactly as `visual-cli bless` writes it.
///
/// A JSON object whose `pixels` hold one line per pixel, its four values' bits then its four
/// bytes encoded for display ([`pixel8`]), each as lowercase hexadecimal.
#[must_use]
pub fn display_text(pixels: &[[f32; 4]]) -> String {
    let mut text = String::from("{\n  \"pixels\": [\n");
    let last = pixels.len().saturating_sub(1);
    for (at, &[red, green, blue, alpha]) in pixels.iter().enumerate() {
        let bytes = u32::from_be_bytes(pixel8(red, green, blue, alpha));
        let comma = if at == last { "" } else { "," };
        // Writing to a string cannot fail.
        let _: core::fmt::Result = writeln!(
            text,
            "    [\"{:08x}\", \"{:08x}\", \"{:08x}\", \"{:08x}\", \"{bytes:08x}\"]{comma}",
            red.to_bits(),
            green.to_bits(),
            blue.to_bits(),
            alpha.to_bits(),
        );
    }
    text.push_str("  ]\n}\n");
    text
}

/// Where `held` first differs from the display corpus file `expected`: the pixel, from 0,
/// whose line differs, or the line past the pixels; `None` when they are the same.
#[must_use]
pub fn first_difference(held: &str, expected: &str) -> Option<String> {
    let mut held_lines = held.lines();
    for (at, line) in expected.lines().enumerate() {
        if held_lines.next() != Some(line) {
            // The two lines before the pixels: the opening brace and the list's key.
            return Some(match at.checked_sub(2) {
                Some(pixel) if line.starts_with("    [") => format!("pixel {pixel}"),
                _ => format!("line {}", at.saturating_add(1)),
            });
        }
    }
    held_lines
        .next()
        .map(|_| format!("line {}", expected.lines().count().saturating_add(1)))
}

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;

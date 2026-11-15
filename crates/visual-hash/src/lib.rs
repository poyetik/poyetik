//! Canonical hash of a sequence of images.
//!
//! The hash covers the exact bit pattern of every value of every image, so two sequences
//! have the same hash only if they are bit-for-bit identical. It is how the visual
//! library proves conformance across targets, as the render hash does for sound.
//!
//! The scheme is specified in `spec/image-hash.md`, with known-answer vectors in
//! `spec/image-hash-vectors.json`. In short, the hashed bytes, all little-endian: the
//! ASCII tag `image-hash/1`, the image rate's numerator and denominator, the width, the
//! height and the color encoding code (`u32` each), every value's `f32` bit pattern
//! (`u32`) image after image, then the number of images (`u64`).

#![no_std]
#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

use core::fmt;

use sha2::{Digest, Sha256};
use visual_core::{
    ColorEncoding, Image, ImageRange, ImageRate, ImageSize, VisualBox, render_sequence,
};

/// The name and version of the hashing scheme, `image-hash/1`: a new version whenever the
/// hashed bytes change, the first bytes hashed, so that hashes of two schemes never match.
pub const SCHEME: &str = "image-hash/1";

/// The SHA-256 hash of a sequence of images, shown as 64 lowercase hexadecimal digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageHash([u8; 32]);

impl ImageHash {
    /// The 32 bytes of the SHA-256, the first byte of the digest first.
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for ImageHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// An image whose size or encoding differs from the sequence's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mismatch {
    /// The index of the image in the sequence, from 0.
    pub image: u64,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "image {} differs from the sequence in size or color encoding",
            self.image
        )
    }
}

/// Its message names the image refused.
impl core::error::Error for Mismatch {}

/// Hashes a sequence of images as it is rendered, without holding them: start it with
/// the sequence's rate, size and encoding, add each image in order, then finish.
#[derive(Debug, Clone)]
pub struct ImageHasher {
    /// The hash of the bytes so far.
    hasher: Sha256,
    /// The size every image must have.
    size: ImageSize,
    /// The encoding every image must have.
    encoding: ColorEncoding,
    /// Images added so far.
    images: u64,
}

impl ImageHasher {
    /// Starts the hash of a sequence of images of `size` and `encoding` at `rate`.
    #[must_use]
    pub fn new(rate: ImageRate, size: ImageSize, encoding: ColorEncoding) -> Self {
        let code: u32 = match encoding {
            ColorEncoding::LinearRec709Rgba32F => 1,
        };
        let mut hasher = Sha256::new();
        hasher.update(SCHEME.as_bytes());
        hasher.update(rate.numerator().to_le_bytes());
        hasher.update(rate.denominator().to_le_bytes());
        hasher.update(size.width().to_le_bytes());
        hasher.update(size.height().to_le_bytes());
        hasher.update(code.to_le_bytes());
        Self {
            hasher,
            size,
            encoding,
            images: 0,
        }
    }

    /// Adds the next image of the sequence.
    ///
    /// # Errors
    ///
    /// Returns [`Mismatch`], and adds nothing, when the image's size or encoding differs
    /// from the sequence's.
    pub fn add(&mut self, image: &Image) -> Result<(), Mismatch> {
        // One encoding exists, so an image always has the sequence's: this exhaustive match
        // stops compiling when a second one is added, and the check must then be written.
        match (image.encoding(), self.encoding) {
            (ColorEncoding::LinearRec709Rgba32F, ColorEncoding::LinearRec709Rgba32F) => {}
        }
        if image.size() != self.size {
            return Err(Mismatch { image: self.images });
        }
        self.update(image);
        Ok(())
    }

    /// Adds the values of `image`, whose size and encoding the caller has made the
    /// sequence's.
    fn update(&mut self, image: &Image) {
        for value in image.values() {
            self.hasher.update(value.to_bits().to_le_bytes());
        }
        self.images = self.images.saturating_add(1);
    }

    /// The hash of the sequence: the bytes so far, then the number of images.
    #[must_use]
    pub fn finish(self) -> ImageHash {
        let mut hasher = self.hasher;
        hasher.update(self.images.to_le_bytes());
        ImageHash(hasher.finalize().into())
    }
}

/// Renders the images of `range` of `visual` into `image` and returns the image hash of the
/// sequence (`spec/image-hash.md`), whose header records `rate`.
///
/// The box renders at the rate it was made for: `rate` is only what the hash records, and
/// the caller passes that same rate (a hash of images recorded at another rate than they
/// were rendered at is a hash of other images). Whether the box's own rate can be read
/// back, making this argument unnecessary, is a question of the visual engine's design
/// (#35 step 2).
///
/// Every image of the sequence is `image` itself, rendered again, so each has the size and
/// encoding the hash starts with: no image can be refused, by construction, where
/// [`ImageHasher::add`] must check images that come from elsewhere. The same hash as
/// rendering each image and adding it in order.
#[must_use]
pub fn hash_sequence(
    visual: &mut dyn VisualBox,
    rate: ImageRate,
    range: ImageRange,
    image: &mut Image,
) -> ImageHash {
    let mut hasher = ImageHasher::new(rate, image.size(), image.encoding());
    render_sequence(visual, range, image, |_, rendered| {
        hasher.update(rendered);
    });
    hasher.finish()
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

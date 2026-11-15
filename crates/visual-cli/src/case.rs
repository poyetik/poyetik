//! Visual conformance cases: what to render, read from JSON once, at the boundary, into a
//! checked case, and its render to a hash.

use core::fmt;
use core::marker::PhantomData;

use serde::Deserialize;
use serde::de::{Deserializer, Error as _, MapAccess, Visitor, value::MapAccessDeserializer};
use visual_boxes::{BoxError, Square};
use visual_core::{Image, ImageIndex, ImageRange, ImageRate, ImageSize, Seconds, VisualError};
use visual_hash::{ImageHash, hash_sequence};

/// The largest case file read, in bytes.
pub const MAX_CASE_BYTES: usize = 64 * 1024;

/// The most images a case renders, 10 000: ten seconds at the fastest rate, longer than a
/// whole cycle of any square of the corpus, and, with [`MAX_RENDERED_PIXELS`], a bound on
/// the work of a case.
pub const MAX_IMAGES: u64 = 10_000;

/// The most pixels a case renders in all, its images times the pixels of each: 2²⁸.
///
/// So that rendering and hashing a case takes seconds at most in a release build (about
/// 20 ns a pixel), a few minutes unoptimized, whatever its file says.
pub const MAX_RENDERED_PIXELS: u64 = 1 << 28;

/// Reads a JSON object as `T`, and nothing else: serde's derived structs also accept a
/// JSON array of their fields in order, which a case file must not be.
fn object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<T, D::Error> {
    /// Visits a map only.
    struct Object<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for Object<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a JSON object")
        }

        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_map(Object(PhantomData))
}

/// Readers of one field each, of type `$kind`, whose refusal (a value of another type, or
/// out of the type's range) names the field `$name`, as every other refusal of a case does.
macro_rules! named_fields {
    ($($reader:ident: $kind:ty = $name:literal;)*) => {$(
        #[doc = concat!("Reads `", $name, "`, naming it when it is refused.")]
        fn $reader<'de, D: Deserializer<'de>>(deserializer: D) -> Result<$kind, D::Error> {
            <$kind>::deserialize(deserializer)
                .map_err(|error| D::Error::custom(format_args!(concat!($name, ": {}"), error)))
        }
    )*};
}

named_fields! {
    rate_field: [u32; 2] = "rate";
    width_field: u32 = "width";
    height_field: u32 = "height";
    first_field: u64 = "first";
    images_field: u64 = "images";
    side_field: u32 = "side";
    cycle_field: [u32; 2] = "cycle";
}

/// Reads `square`, a JSON object only, naming it when it is refused.
fn square_field<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SquareFile, D::Error> {
    object(deserializer).map_err(|error| D::Error::custom(format_args!("square: {error}")))
}

/// The square of a case file, as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SquareFile {
    /// The side, in 65 536ths of the height.
    #[serde(deserialize_with = "side_field")]
    side: u32,
    /// One sweep lasts `cycle[0] / cycle[1]` seconds.
    #[serde(deserialize_with = "cycle_field")]
    cycle: [u32; 2],
}

/// A case file, as written: a square rendered at a rate and a size, from an image index,
/// for a number of images.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFile {
    /// Images per second, `rate[0] / rate[1]`.
    #[serde(deserialize_with = "rate_field")]
    rate: [u32; 2],
    /// Width of the images, in pixels.
    #[serde(deserialize_with = "width_field")]
    width: u32,
    /// Height of the images, in pixels.
    #[serde(deserialize_with = "height_field")]
    height: u32,
    /// The index of the first image rendered.
    #[serde(deserialize_with = "first_field")]
    first: u64,
    /// How many images are rendered.
    #[serde(deserialize_with = "images_field")]
    images: u64,
    /// The square.
    #[serde(deserialize_with = "square_field")]
    square: SquareFile,
}

/// Why the text of a case file is not a case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseError {
    /// The file is longer than [`MAX_CASE_BYTES`]: its length in bytes.
    TooLong(usize),
    /// Not a JSON object of exactly the fields of a case, each of its type: the reader's
    /// explanation, with its line and column.
    Json(String),
    /// The count of images is not from 1 to [`MAX_IMAGES`].
    Images(u64),
    /// The last image is past the largest index.
    Range(VisualError),
    /// The image rate.
    Rate(VisualError),
    /// The width and height.
    Size(VisualError),
    /// The cycle of the square.
    Cycle(VisualError),
    /// The side of the square.
    Side(BoxError),
    /// More than [`MAX_RENDERED_PIXELS`] pixels to render: how many.
    TooMuchWork(u64),
}

impl fmt::Display for CaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong(bytes) => write!(
                f,
                "a case file holds at most {MAX_CASE_BYTES} bytes, this one {bytes}"
            ),
            Self::Json(reason) => f.write_str(reason),
            Self::Images(images) => {
                write!(f, "images: from 1 to {MAX_IMAGES}, not {images}")
            }
            Self::Range(error) => write!(f, "first and images: {error}"),
            Self::Rate(error) => write!(f, "rate: {error}"),
            Self::Size(error) => write!(f, "width and height: {error}"),
            Self::Cycle(error) => write!(f, "square: cycle: {error}"),
            Self::Side(error) => write!(f, "square: side: {error}"),
            Self::TooMuchWork(pixels) => write!(
                f,
                "images, width and height: a case renders at most {MAX_RENDERED_PIXELS} pixels in all, this one {pixels}"
            ),
        }
    }
}

/// Its message names the field refused and why; its source, for a value the library refused,
/// is that value's error.
impl core::error::Error for CaseError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Range(error) | Self::Rate(error) | Self::Size(error) | Self::Cycle(error) => {
                Some(error)
            }
            Self::Side(error) => Some(error),
            Self::TooLong(_) | Self::Json(_) | Self::Images(_) | Self::TooMuchWork(_) => None,
        }
    }
}

/// A visual conformance case, every value checked: what is rendered and hashed.
#[derive(Debug, Clone, Copy)]
pub struct PreparedCase {
    /// The images rendered.
    range: ImageRange,
    /// The image size.
    size: ImageSize,
    /// The square, which holds the image rate.
    square: Square,
}

impl PreparedCase {
    /// Renders the case with the exact CPU reference renderer and returns the image hash
    /// of its images.
    #[must_use]
    pub fn hash(&self) -> ImageHash {
        let mut square = self.square;
        let mut image = Image::new(self.size);
        let rate = square.rate();
        hash_sequence(&mut square, rate, self.range, &mut image)
    }
}

/// Reads a case from the text of its file.
///
/// # Errors
///
/// Returns why the text is not a valid case: too long, not a JSON object of exactly the
/// fields of a case (one missing, unknown or written twice, or of another type), or a
/// value outside its range, naming the field.
pub fn parse_case(text: &str) -> Result<PreparedCase, CaseError> {
    if text.len() > MAX_CASE_BYTES {
        return Err(CaseError::TooLong(text.len()));
    }
    let json = |error: serde_json::Error| CaseError::Json(error.to_string());
    let mut reader = serde_json::Deserializer::from_str(text);
    let file: CaseFile = object(&mut reader).map_err(json)?;
    reader.end().map_err(json)?;
    if !(1..=MAX_IMAGES).contains(&file.images) {
        return Err(CaseError::Images(file.images));
    }
    let range = ImageRange::new(ImageIndex(file.first), file.images).map_err(CaseError::Range)?;
    let [numerator, denominator] = file.rate;
    let rate = ImageRate::new(numerator, denominator).map_err(CaseError::Rate)?;
    let size = ImageSize::new(file.width, file.height).map_err(CaseError::Size)?;
    // At most 2¹⁴ images of 2²⁵ pixels: the product never saturates.
    let pixels = file.images.saturating_mul(size.pixels());
    if pixels > MAX_RENDERED_PIXELS {
        return Err(CaseError::TooMuchWork(pixels));
    }
    let [seconds, divisor] = file.square.cycle;
    let cycle = Seconds::new(seconds, divisor).map_err(CaseError::Cycle)?;
    let square = Square::new(rate, file.square.side, cycle).map_err(CaseError::Side)?;
    Ok(PreparedCase {
        range,
        size,
        square,
    })
}

/// Reads a case from the text of its file and renders it to its image hash: what the
/// commands do with a case file.
///
/// # Errors
///
/// Returns why the text is not a valid case (see [`parse_case`]).
pub fn hash_case(text: &str) -> Result<ImageHash, CaseError> {
    parse_case(text).map(|case| case.hash())
}

#[cfg(test)]
#[path = "case_tests.rs"]
mod tests;

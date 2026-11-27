use core::sync::atomic::{AtomicU32, Ordering};

use visual_core::{Image, ImageIndex, ImageRate, SubpixelPoint, Trace, VisualBox};

use super::{check_laws, from_both_ends};
use crate::matrix::{IMAGE_INDICES, IMAGE_RATES, IMAGE_SIZES};

/// A correct box: every value derived from the image index and the rate, alpha 1.
#[derive(Debug)]
struct Good {
    /// The rate, which the value depends on.
    rate: ImageRate,
}

impl VisualBox for Good {
    fn render(&mut self, index: ImageIndex, output: &mut Image) {
        let value = f32::from(u8::try_from(index.0 % 7).unwrap_or(0)) / 8.0
            + f32::from(u8::try_from(self.rate.denominator() % 3).unwrap_or(0)) / 32.0;
        for row in output.rows_mut() {
            for pixel in row.as_chunks_mut::<{ Image::CHANNELS }>().0 {
                *pixel = [value, value, value, 1.0];
            }
        }
    }

    fn trace(&self, index: ImageIndex, point: SubpixelPoint) -> Trace {
        if point.x() > point.y() {
            Trace::Shape(u32::try_from(index.0 % 5).unwrap_or(0))
        } else {
            Trace::Background
        }
    }
}

/// Creates a [`Good`] box.
fn good(rate: ImageRate) -> Box<dyn VisualBox> {
    Box::new(Good { rate })
}

/// What a faulty box gets wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// Its images depend on how many it rendered before.
    Stateful,
    /// Each instance renders differently.
    PerInstance,
    /// It allocates while rendering, from image 2 on.
    Allocates,
    /// It renders a NaN at image 59, in the channel given (0 red, 1 green, 2 blue, 3
    /// alpha) of the first pixel.
    NotANumber(usize),
    /// It renders an infinity at image 59.
    Infinite,
    /// Its alpha is above 1.
    AlphaAbove,
    /// Its alpha is negative.
    AlphaBelow,
    /// It leaves the first value of every image as it was.
    KeepsAValue,
    /// Its trace depends on how many times it traced.
    ImpureTrace,
    /// Its trace depends on how many times it traced, at the exact center of the image only.
    ImpureTraceAtCenter,
    /// Its trace changes once it has rendered an image.
    TraceAfterRendering,
    /// Its trace changes once it has rendered six images.
    TraceAfterSixImages,
    /// It allocates when it renders an image before the last one it rendered.
    AllocatesSeekingBack,
    /// It writes nothing when asked for the image it rendered last.
    SkipsARepeat,
    /// It renders a NaN at the highest rate of the matrix only.
    NotANumberAtTheHighestRate,
    /// It renders a NaN in the widest images only.
    NotANumberWhenWidest,
    /// It renders a NaN in the tallest images only.
    NotANumberWhenTallest,
    /// Its trace depends on how many times it traced, at the last subpixel of the image only.
    ImpureTraceAtTheLastSubpixel,
    /// It renders a NaN at a rate the matrix does not hold only.
    NotANumberOffTheMatrixRates,
    /// It renders a NaN in images of a size the matrix does not hold only.
    NotANumberOffTheMatrixSizes,
    /// It renders a NaN at an index the matrix does not hold only.
    NotANumberOffTheMatrixIndices,
    /// It renders a NaN when it moves forward after having moved back: only the pass
    /// jumping between both ends of the indices does, in its first rendering of an image.
    NotANumberForwardAfterGoingBack,
    /// It renders a NaN once it has seen a size other than the first it rendered.
    NotANumberAfterAnotherSize,
    /// Its trace changes for a size other than the first it rendered: geometry cached by
    /// rendering, read back by tracing.
    TraceAfterAnotherSize,
    /// Its trace answers from the size it rendered last, whatever the point's size.
    TraceFromTheLastSize,
    /// It renders the smallest subnormal value at image 59, in the first value.
    Subnormal,
    /// It renders -0 at image 59, in the first value.
    NegativeZero,
    /// It allocates while rendering at a size other than the first it rendered.
    AllocatesAfterAnotherSize,
    /// It allocates while rendering in the second instance its factory made.
    AllocatesInItsSecondInstance,
    /// It allocates while rendering into an image whose first value is NaN: an image of
    /// other values.
    AllocatesOverOtherValues,
    /// It allocates while rendering forward after having rendered an image before the last.
    AllocatesForwardAfterGoingBack,
    /// It allocates while rendering the image it rendered last again.
    AllocatesRepeatingAnImage,
    /// Its first value is 1 once it has rendered an image, at an index of the matrix neither
    /// its first nor its last: only an instance rendering such an image alone shows it.
    ChangesInTheMiddleAfterRendering,
}

/// A box with one fault, otherwise like [`Good`].
#[derive(Debug)]
struct Faulty {
    /// Its fault.
    fault: Fault,
    /// The correct box it otherwise behaves like.
    good: Good,
    /// Images rendered so far.
    rendered: u32,
    /// The number of this instance.
    instance: u32,
    /// Traces so far.
    traced: AtomicU32,
    /// The last image rendered, if any.
    last: Option<ImageIndex>,
    /// Whether it ever rendered an image before the last one it rendered.
    went_back: bool,
    /// The size of the first image it rendered.
    first_size: Option<(u32, u32)>,
    /// The size of the last image it rendered.
    last_size: Option<(u32, u32)>,
}

impl Faulty {
    /// What its fault does before rendering image `index` into `output`: `None` when it
    /// writes nothing at all, else whether it then writes a NaN.
    fn before(&mut self, index: ImageIndex, output: &Image) -> Option<bool> {
        let last = self.last.replace(index);
        let forward = last.is_some_and(|before| index > before);
        let came_back = self.went_back;
        self.went_back |= last.is_some_and(|before| index < before);
        if self.fault == Fault::SkipsARepeat && last == Some(index) {
            return None;
        }
        let size = output.size();
        let first_size = *self
            .first_size
            .get_or_insert_with(|| (size.width(), size.height()));
        let allocates = match self.fault {
            Fault::AllocatesSeekingBack => last.is_some_and(|before| index < before),
            Fault::AllocatesAfterAnotherSize => first_size != (size.width(), size.height()),
            Fault::AllocatesInItsSecondInstance => self.instance == 1,
            Fault::AllocatesOverOtherValues => {
                output.values().first().is_some_and(|value| value.is_nan())
            }
            Fault::AllocatesForwardAfterGoingBack => came_back && forward,
            Fault::AllocatesRepeatingAnImage => last == Some(index),
            Fault::Stateful
            | Fault::PerInstance
            | Fault::Allocates
            | Fault::NotANumber(_)
            | Fault::Infinite
            | Fault::AlphaAbove
            | Fault::AlphaBelow
            | Fault::KeepsAValue
            | Fault::ImpureTrace
            | Fault::ImpureTraceAtCenter
            | Fault::TraceAfterRendering
            | Fault::TraceAfterSixImages
            | Fault::SkipsARepeat
            | Fault::NotANumberAtTheHighestRate
            | Fault::NotANumberWhenWidest
            | Fault::NotANumberWhenTallest
            | Fault::ImpureTraceAtTheLastSubpixel
            | Fault::NotANumberOffTheMatrixRates
            | Fault::NotANumberOffTheMatrixSizes
            | Fault::NotANumberOffTheMatrixIndices
            | Fault::NotANumberForwardAfterGoingBack
            | Fault::NotANumberAfterAnotherSize
            | Fault::TraceAfterAnotherSize
            | Fault::TraceFromTheLastSize
            | Fault::Subnormal
            | Fault::NegativeZero
            | Fault::ChangesInTheMiddleAfterRendering => false,
        };
        if allocates {
            drop(core::hint::black_box(vec![0_u8; 16]));
        }
        self.last_size = Some((size.width(), size.height()));
        let odd = match self.fault {
            Fault::NotANumberAtTheHighestRate => self.good.rate.numerator() == 1000,
            Fault::NotANumberWhenWidest => size.width() == visual_core::ImageSize::MAX_SIDE,
            Fault::NotANumberWhenTallest => size.height() == visual_core::ImageSize::MAX_SIDE,
            Fault::NotANumberOffTheMatrixRates => {
                !IMAGE_RATES.contains(&(self.good.rate.numerator(), self.good.rate.denominator()))
            }
            Fault::NotANumberOffTheMatrixSizes => {
                !IMAGE_SIZES.contains(&(size.width(), size.height()))
            }
            Fault::NotANumberOffTheMatrixIndices => !IMAGE_INDICES.contains(&index.0),
            Fault::NotANumberForwardAfterGoingBack => came_back && forward,
            Fault::NotANumberAfterAnotherSize => first_size != (size.width(), size.height()),
            Fault::Stateful
            | Fault::PerInstance
            | Fault::Allocates
            | Fault::NotANumber(_)
            | Fault::Infinite
            | Fault::AlphaAbove
            | Fault::AlphaBelow
            | Fault::KeepsAValue
            | Fault::ImpureTrace
            | Fault::ImpureTraceAtCenter
            | Fault::TraceAfterRendering
            | Fault::TraceAfterSixImages
            | Fault::AllocatesSeekingBack
            | Fault::AllocatesAfterAnotherSize
            | Fault::AllocatesInItsSecondInstance
            | Fault::AllocatesOverOtherValues
            | Fault::AllocatesForwardAfterGoingBack
            | Fault::AllocatesRepeatingAnImage
            | Fault::SkipsARepeat
            | Fault::ImpureTraceAtTheLastSubpixel
            | Fault::TraceAfterAnotherSize
            | Fault::TraceFromTheLastSize
            | Fault::Subnormal
            | Fault::NegativeZero
            | Fault::ChangesInTheMiddleAfterRendering => false,
        };
        Some(odd)
    }
}

impl Faulty {
    /// The alpha its fault writes in the last value of every image, if any.
    const fn alpha(&self) -> Option<f32> {
        match self.fault {
            Fault::AlphaAbove => Some(1.5),
            Fault::AlphaBelow => Some(-0.25),
            Fault::Stateful
            | Fault::PerInstance
            | Fault::Allocates
            | Fault::NotANumber(_)
            | Fault::Infinite
            | Fault::KeepsAValue
            | Fault::ImpureTrace
            | Fault::ImpureTraceAtCenter
            | Fault::TraceAfterRendering
            | Fault::TraceAfterSixImages
            | Fault::AllocatesSeekingBack
            | Fault::AllocatesAfterAnotherSize
            | Fault::AllocatesInItsSecondInstance
            | Fault::AllocatesOverOtherValues
            | Fault::AllocatesForwardAfterGoingBack
            | Fault::AllocatesRepeatingAnImage
            | Fault::SkipsARepeat
            | Fault::NotANumberAtTheHighestRate
            | Fault::NotANumberWhenWidest
            | Fault::NotANumberWhenTallest
            | Fault::ImpureTraceAtTheLastSubpixel
            | Fault::NotANumberOffTheMatrixRates
            | Fault::NotANumberOffTheMatrixSizes
            | Fault::NotANumberOffTheMatrixIndices
            | Fault::NotANumberForwardAfterGoingBack
            | Fault::NotANumberAfterAnotherSize
            | Fault::TraceAfterAnotherSize
            | Fault::TraceFromTheLastSize
            | Fault::Subnormal
            | Fault::NegativeZero
            | Fault::ChangesInTheMiddleAfterRendering => None,
        }
    }
}

impl VisualBox for Faulty {
    fn render(&mut self, index: ImageIndex, output: &mut Image) {
        let Some(odd) = self.before(index, output) else {
            return;
        };
        let kept = output.values().first().copied();
        self.good.render(index, output);
        if odd && let Some(slot) = output.rows_mut().next().and_then(|row| row.first_mut()) {
            *slot = f32::NAN;
        }
        self.rendered = self.rendered.saturating_add(1);
        // The channel a NaN of `NotANumber` goes to; any other fault changes the first.
        let channel = if let Fault::NotANumber(channel) = self.fault {
            channel
        } else {
            0
        };
        let first = output
            .rows_mut()
            .next()
            .and_then(|row| row.get_mut(channel));
        let changed = match self.fault {
            Fault::Stateful => Some(f32::from(u16::try_from(self.rendered).unwrap_or(0))),
            Fault::PerInstance => Some(f32::from(u16::try_from(self.instance).unwrap_or(0))),
            Fault::Allocates => {
                if index.0 >= 2 {
                    drop(core::hint::black_box(vec![0_u8; 16]));
                }
                None
            }
            Fault::NotANumber(_) => (index.0 == 59).then_some(f32::NAN),
            Fault::Infinite => (index.0 == 59).then_some(f32::INFINITY),
            Fault::Subnormal => (index.0 == 59).then_some(f32::from_bits(1)),
            Fault::NegativeZero => (index.0 == 59).then_some(-0.0),
            Fault::KeepsAValue => kept,
            Fault::AlphaAbove
            | Fault::AlphaBelow
            | Fault::ImpureTrace
            | Fault::ImpureTraceAtCenter
            | Fault::TraceAfterRendering
            | Fault::TraceAfterSixImages
            | Fault::AllocatesSeekingBack
            | Fault::AllocatesAfterAnotherSize
            | Fault::AllocatesInItsSecondInstance
            | Fault::AllocatesOverOtherValues
            | Fault::AllocatesForwardAfterGoingBack
            | Fault::AllocatesRepeatingAnImage
            | Fault::SkipsARepeat
            | Fault::NotANumberAtTheHighestRate
            | Fault::NotANumberWhenWidest
            | Fault::NotANumberWhenTallest
            | Fault::ImpureTraceAtTheLastSubpixel
            | Fault::NotANumberOffTheMatrixRates
            | Fault::NotANumberOffTheMatrixSizes
            | Fault::NotANumberOffTheMatrixIndices
            | Fault::NotANumberForwardAfterGoingBack
            | Fault::NotANumberAfterAnotherSize
            | Fault::TraceAfterAnotherSize
            | Fault::TraceFromTheLastSize => None,
            Fault::ChangesInTheMiddleAfterRendering => (self.rendered > 1
                && IMAGE_INDICES.first() != Some(&index.0)
                && IMAGE_INDICES.last() != Some(&index.0))
            .then_some(1.0),
        };
        if let (Some(value), Some(slot)) = (changed, first) {
            *slot = value;
        }
        let alpha = self.alpha();
        if let Some(value) = alpha
            && let Some(slot) = output.rows_mut().last().and_then(|row| row.last_mut())
        {
            *slot = value;
        }
    }

    fn trace(&self, index: ImageIndex, point: SubpixelPoint) -> Trace {
        let count = self.traced.fetch_add(1, Ordering::Relaxed);
        match self.fault {
            Fault::ImpureTrace => Trace::Shape(count),
            Fault::ImpureTraceAtCenter => {
                let unit = u64::from(visual_core::SUBPIXELS);
                let size = point.size();
                let center = size.point(
                    u64::from(size.width()).saturating_mul(unit) / 2,
                    u64::from(size.height()).saturating_mul(unit) / 2,
                );
                if Some(point) == center {
                    Trace::Shape(count)
                } else {
                    self.good.trace(index, point)
                }
            }
            Fault::ImpureTraceAtTheLastSubpixel => {
                let unit = u64::from(visual_core::SUBPIXELS);
                let size = point.size();
                let right = u64::from(size.width()).saturating_mul(unit);
                let bottom = u64::from(size.height()).saturating_mul(unit);
                if Some(point) == size.point(right.saturating_sub(1), bottom.saturating_sub(1)) {
                    Trace::Shape(count)
                } else {
                    self.good.trace(index, point)
                }
            }
            Fault::TraceAfterRendering => Trace::Shape(u32::from(self.rendered > 0)),
            Fault::TraceAfterSixImages => Trace::Shape(u32::from(self.rendered >= 6)),
            Fault::TraceFromTheLastSize => {
                let size = point.size();
                if self
                    .last_size
                    .is_some_and(|last| last != (size.width(), size.height()))
                {
                    Trace::Shape(9)
                } else {
                    self.good.trace(index, point)
                }
            }
            Fault::TraceAfterAnotherSize => {
                let size = point.size();
                if self
                    .first_size
                    .is_some_and(|first| first != (size.width(), size.height()))
                {
                    Trace::Shape(9)
                } else {
                    self.good.trace(index, point)
                }
            }
            Fault::Stateful
            | Fault::PerInstance
            | Fault::Allocates
            | Fault::NotANumber(_)
            | Fault::Infinite
            | Fault::AlphaAbove
            | Fault::AlphaBelow
            | Fault::KeepsAValue
            | Fault::AllocatesSeekingBack
            | Fault::AllocatesAfterAnotherSize
            | Fault::AllocatesInItsSecondInstance
            | Fault::AllocatesOverOtherValues
            | Fault::AllocatesForwardAfterGoingBack
            | Fault::AllocatesRepeatingAnImage
            | Fault::SkipsARepeat
            | Fault::NotANumberAtTheHighestRate
            | Fault::NotANumberWhenWidest
            | Fault::NotANumberWhenTallest
            | Fault::NotANumberOffTheMatrixRates
            | Fault::NotANumberOffTheMatrixSizes
            | Fault::NotANumberOffTheMatrixIndices
            | Fault::NotANumberForwardAfterGoingBack
            | Fault::NotANumberAfterAnotherSize
            | Fault::Subnormal
            | Fault::NegativeZero
            | Fault::ChangesInTheMiddleAfterRendering => self.good.trace(index, point),
        }
    }
}

/// The laws' verdict on a box with `fault`.
fn verdict(fault: Fault) -> Result<(), String> {
    // The instances of this verdict's factory, numbered from 0 in the order the laws make
    // them.
    let made = AtomicU32::new(0);
    check_laws(&|rate| -> Box<dyn VisualBox> {
        Box::new(Faulty {
            fault,
            good: Good { rate },
            rendered: 0,
            instance: made.fetch_add(1, Ordering::Relaxed),
            traced: AtomicU32::new(0),
            last: None,
            went_back: false,
            first_size: None,
            last_size: None,
        })
    })
}

#[test]
fn a_correct_box_passes_every_law() {
    assert_eq!(check_laws(&good), Ok(()), "every law holds");
}

#[test]
fn catches_a_box_that_changes_with_the_size() {
    for (fault, law) in [
        (
            Fault::NotANumberAfterAnotherSize,
            "size change: rendering at another size first changes the bits at rate 1/1, size 2x3, image 0",
        ),
        (
            Fault::TraceFromTheLastSize,
            "size change: (0, 0) of size 2x3 traces differently after size 1x1 at rate 1/1, size 1x1, image 0",
        ),
        (
            Fault::TraceAfterAnotherSize,
            "size change: (0, 0) of size 2x3 traces differently after size 1x1 at rate 1/1, size 1x1, image 0",
        ),
    ] {
        assert_eq!(verdict(fault), Err(law.to_owned()), "{fault:?}");
    }
}

#[test]
fn catches_a_box_whose_bits_change_once_it_has_rendered() {
    // At an index neither end of the matrix: only an instance rendering it alone differs.
    assert_eq!(
        verdict(Fault::ChangesInTheMiddleAfterRendering),
        Err(
            "history: rendering the images before it changes the bits at rate 1/1, size 1x1, image 1"
                .to_owned()
        )
    );
}

#[test]
fn catches_each_broken_law() {
    for (fault, law) in [
        (
            Fault::Stateful,
            "random access: rendering other images first changes the bits at rate 1/1, size 1x1, image 18446744073709551615",
        ),
        (
            Fault::PerInstance,
            "determinism: two fresh instances differ at rate 1/1, size 1x1, image 0",
        ),
        (
            Fault::Allocates,
            "no heap operation: 2 while rendering at rate 1/1, size 1x1, image 2",
        ),
        (
            Fault::NotANumber(0),
            "finite values: a value is NaN or infinite at rate 1/1, size 1x1, image 59",
        ),
        (
            Fault::NotANumber(1),
            "finite values: a value is NaN or infinite at rate 1/1, size 1x1, image 59",
        ),
        (
            Fault::NotANumber(2),
            "finite values: a value is NaN or infinite at rate 1/1, size 1x1, image 59",
        ),
        (
            Fault::NotANumber(3),
            "finite values: a value is NaN or infinite at rate 1/1, size 1x1, image 59",
        ),
        (
            Fault::Subnormal,
            "finite values: a subnormal value at rate 1/1, size 1x1, image 59",
        ),
        (
            Fault::NegativeZero,
            "finite values: a negative zero at rate 1/1, size 1x1, image 59",
        ),
        (
            Fault::Infinite,
            "finite values: a value is NaN or infinite at rate 1/1, size 1x1, image 59",
        ),
        (
            Fault::AlphaAbove,
            "finite values: alpha 1.5 outside [0, 1] at rate 1/1, size 1x1, image 0",
        ),
        (
            Fault::AlphaBelow,
            "finite values: alpha -0.25 outside [0, 1] at rate 1/1, size 1x1, image 0",
        ),
        (
            Fault::KeepsAValue,
            "overwrite: a value kept from before rendering at rate 1/1, size 1x1, image 0",
        ),
        (
            Fault::ImpureTrace,
            "pure trace: (0, 0) traces differently at rate 1/1, size 1x1, image 18446744073709551615",
        ),
        (
            Fault::ImpureTraceAtCenter,
            "pure trace: (2048, 2048) traces differently at rate 1/1, size 1x1, image 18446744073709551615",
        ),
        (
            Fault::TraceAfterRendering,
            "pure trace: (0, 0) traces differently at rate 1/1, size 1x1, image 18446744073709551615",
        ),
        (
            Fault::TraceAfterSixImages,
            "pure trace: (0, 0) traces differently at rate 1/1, size 1x1, image 18446744073709551615",
        ),
        (
            Fault::AllocatesSeekingBack,
            "no heap operation: 2 while rendering, other images first, at rate 1/1, size 1x1, image 1000000",
        ),
        (
            Fault::SkipsARepeat,
            "random access: rendering the images in another order, each twice, changes the bits at rate 1/1, size 1x1, image 0",
        ),
        (
            Fault::NotANumberAtTheHighestRate,
            "finite values: a value is NaN or infinite at rate 1000/1, size 1x1, image 0",
        ),
        (
            Fault::NotANumberWhenWidest,
            "finite values: a value is NaN or infinite at rate 1/1, size 16384x1, image 0",
        ),
        (
            Fault::NotANumberWhenTallest,
            "finite values: a value is NaN or infinite at rate 1/1, size 1x16384, image 0",
        ),
        (
            Fault::ImpureTraceAtTheLastSubpixel,
            "pure trace: (4095, 4095) traces differently at rate 1/1, size 1x1, image 18446744073709551615",
        ),
        (
            Fault::NotANumberForwardAfterGoingBack,
            "random access: rendering the images in another order, each twice, changes the bits at rate 1/1, size 1x1, image 1000000",
        ),
    ] {
        assert_eq!(verdict(fault), Err(law.to_owned()), "{fault:?}");
    }
}

/// The inputs the visual box contract makes an image and a trace depend on
/// (`VisualBox::render` and `VisualBox::trace`, in `visual-core`'s contract): the index of the
/// image, the rate the box is made for, the size of its output and the size a traced point
/// belongs to, each with a fault the laws must refuse: a box keeping state across a change of
/// index or of size, or right only at some rates. A new input of the contract comes with its
/// fault here (a convention, reviewed: no tool lists the contract's inputs).
const CONTRACT_INPUTS: [(&str, Fault, &str); 4] = [
    (
        "index",
        Fault::NotANumberForwardAfterGoingBack,
        "random access: ",
    ),
    (
        "rate",
        Fault::NotANumberOffTheMatrixRates,
        "Test failed: finite values: ",
    ),
    (
        "size",
        Fault::NotANumberAfterAnotherSize,
        "size change: rendering ",
    ),
    (
        "traced point's size",
        Fault::TraceFromTheLastSize,
        "size change: (0, 0) of size ",
    ),
];

#[test]
fn refuses_a_box_wrong_along_any_input_of_the_contract() {
    for (input, fault, law) in CONTRACT_INPUTS {
        let refused = verdict(fault);
        assert!(
            refused.as_ref().is_err_and(|said| said.starts_with(law)),
            "{input}: {fault:?} refused by the law it breaks, {law}: {refused:?}"
        );
    }
}

#[test]
fn catches_a_box_right_only_at_the_values_of_the_matrix() {
    // Each fault appears only at a rate, a size or an index the matrix does not hold: the
    // random cases find it, and report the simplest case that shows it.
    for (fault, law) in [
        (
            Fault::NotANumberOffTheMatrixRates,
            "Test failed: finite values: a value is NaN or infinite at rate 2/1, size 1x1, image 0, images drawn [0, 0].",
        ),
        (
            Fault::NotANumberOffTheMatrixSizes,
            "Test failed: finite values: a value is NaN or infinite at rate 1/1, size 1x2, image 0, images drawn [0, 0].",
        ),
        (
            Fault::NotANumberOffTheMatrixIndices,
            "Test failed: finite values: a value is NaN or infinite at rate 1/1, size 1x1, image 4294967295, images drawn [4294967295, 0].",
        ),
    ] {
        assert_eq!(verdict(fault), Err(law.to_owned()), "{fault:?}");
    }
}

#[test]
fn orders_images_from_both_ends_in_turn() {
    for (indices, order) in [
        (&[][..], &[][..]),
        (&[0_u64][..], &[0_u64][..]),
        (&[0, 1][..], &[0, 1][..]),
        (&[0, 1, 2][..], &[0, 2, 1][..]),
        (&[0, 1, 2, 3, 4, 5][..], &[0, 5, 1, 4, 2, 3][..]),
    ] {
        assert_eq!(from_both_ends(indices), order, "{indices:?}");
    }
}

#[test]
fn catches_a_heap_operation_at_each_pass_of_the_laws() {
    // Each fault allocates in one pass only: the no-heap check of that pass catches it.
    for (fault, law) in [
        (
            Fault::AllocatesInItsSecondInstance,
            "no heap operation: 2 while rendering, a second instance, at rate 1/1, size 1x1, image 0",
        ),
        (
            Fault::AllocatesOverOtherValues,
            "no heap operation: 2 while rendering over other values at rate 1/1, size 1x1, image 0",
        ),
        (
            Fault::AllocatesForwardAfterGoingBack,
            "no heap operation: 2 while rendering, jumping between images, at rate 1/1, size 1x1, image 1000000",
        ),
        (
            Fault::AllocatesRepeatingAnImage,
            "no heap operation: 2 while rendering, the same image again, at rate 1/1, size 1x1, image 0",
        ),
        (
            Fault::AllocatesAfterAnotherSize,
            "no heap operation: 2 while rendering, after another size, at rate 1/1, size 2x3, image 0",
        ),
    ] {
        assert_eq!(verdict(fault), Err(law.to_owned()), "{fault:?}");
    }
}

/// An image refused as the machine's memory would refuse it.
fn no_room(size: visual_core::ImageSize) -> Result<Image, visual_core::VisualError> {
    Err(visual_core::VisualError::OutOfMemory {
        width: size.width(),
        height: size.height(),
        of: visual_core::Holding::Image,
    })
}

/// Allocations made so far by [`image_but_the`].
static ALLOCATIONS: AtomicU32 = AtomicU32::new(0);

/// The allocation [`refuses_each_allocation_and_count_where_it_comes`] refuses, one in each
/// pass that allocates: the third (the first pass's image of other values), one far into
/// the pass jumping between images, and the first of the pass rendering each image alone.
static REFUSED: AtomicU32 = AtomicU32::new(u32::MAX);

/// Allocates every image but the one numbered [`REFUSED`], counted by [`ALLOCATIONS`].
fn image_but_the(size: visual_core::ImageSize) -> Result<Image, visual_core::VisualError> {
    if ALLOCATIONS.fetch_add(1, Ordering::Relaxed) == REFUSED.load(Ordering::Relaxed) {
        no_room(size)
    } else {
        Image::try_new(size)
    }
}

/// Counts made so far by [`count_once`].
static COUNTS: AtomicU32 = AtomicU32::new(0);

/// Counts the first action as the counting allocator does, then refuses every one after.
fn count_once(action: &mut dyn FnMut()) -> Result<usize, String> {
    if COUNTS.fetch_add(1, Ordering::Relaxed) == 0 {
        super::counted(action)
    } else {
        Err("not counted".to_owned())
    }
}

#[test]
fn refuses_each_allocation_and_count_where_it_comes() {
    use super::{Seams, check_laws_with};
    let real = Seams::REAL;
    let room = "image 1x1 does not fit in this machine's memory: 16 bytes are needed";
    // Every refusal below comes at the first rate, size and index of the matrix.
    let first = super::place(
        *crate::matrix::image_rates().first().expect("a rate"),
        *crate::matrix::image_sizes().first().expect("a size"),
        *crate::matrix::image_indices().first().expect("an index"),
    );
    // No count at all: refused before any law, as without the counting allocator.
    assert_eq!(
        check_laws_with(
            &good,
            Seams {
                count: |_| Err("not counted".to_owned()),
                ..real
            }
        ),
        Err("not counted".to_owned()),
        "the count checked first"
    );
    // The first count taken, the second refused: the first image's render.
    assert_eq!(
        check_laws_with(
            &good,
            Seams {
                count: count_once,
                ..real
            }
        ),
        Err(format!("not counted while rendering at {first}")),
        "the first render's count"
    );
    assert_eq!(
        COUNTS.load(Ordering::Relaxed),
        2,
        "stopped at the second count"
    );
    // No image at all: the first render's.
    assert_eq!(
        check_laws_with(
            &good,
            Seams {
                image: no_room,
                ..real
            }
        ),
        Err(format!("{room} while rendering at {first}")),
        "the first image"
    );
    // The first pass allocates three images an index (the first instance's, the second's,
    // and the image of other values), then the pass of other images first one an index,
    // then the pass jumping between images two an image (once, then an image of other values
    // for twice), then the pass rendering each image alone one an index: the third refused
    // is the first pass's image of other values, the allocation 4n + 1 the jumping pass's,
    // and the allocation 6n the first image rendered alone.
    let indices = u32::try_from(crate::matrix::image_indices().len()).expect("a count");
    for (refused, how) in [
        (2, " over other values"),
        (4 * indices + 1, ", the same image again,"),
        (6 * indices, ", alone,"),
    ] {
        ALLOCATIONS.store(0, Ordering::Relaxed);
        REFUSED.store(refused, Ordering::Relaxed);
        assert_eq!(
            check_laws_with(
                &good,
                Seams {
                    image: image_but_the,
                    ..real
                }
            ),
            Err(format!("{room} while rendering{how} at {first}")),
            "allocation {refused}"
        );
        assert_eq!(
            ALLOCATIONS.load(Ordering::Relaxed),
            refused + 1,
            "stopped at allocation {refused}"
        );
    }
}

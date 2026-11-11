//! Laws of reading patches, over random and mutated documents: the deterministic start
//! of fuzzing (coverage-guided fuzzing comes with roadmap #3).
//!
//! Whatever the bytes, `parse_patch` then `Plan::build` (with the real registry, its boxes
//! called directly as the engine calls them) return `Ok` or `Err`, never panic; every
//! plan they accept renders finite samples without a heap operation. The documents mix arbitrary text with patches built from valid parts and
//! hostile ones: wrong types, extreme ids, ports and numbers, unknown and repeated keys,
//! deep nesting, every box type with random parameters and cables.

#![forbid(unsafe_code)]

/// Counts heap operations, so that rendering an accepted plan can be checked not to use
/// the heap.
#[global_allocator]
static ALLOCATOR: engine_testkit::CountingAllocator = engine_testkit::CountingAllocator;

#[cfg(test)]
mod tests {
    extern crate alloc;

    use engine_core::host::{Delivered, Raw};
    use engine_core::{Binary32, Binary64, Plan, Precision, Renderer, SampleRate};
    use engine_objects::{BuiltIn, REGISTRY};
    use engine_testkit::heap_operations_during;
    use engine_testkit::matrix::PRECISIONS;
    use engine_testkit::runner::runner;
    use proptest::collection::vec;
    use proptest::option::weighted;
    use proptest::prelude::{Just, any, prop_oneof};
    use proptest::strategy::Strategy;
    use proptest::test_runner::TestCaseError;

    /// Documents tried per law.
    const CASES: u32 = 2_000;

    /// Frames rendered from each accepted plan.
    const FRAMES: usize = 200;

    /// A number as JSON text: ordinary, extreme, special or not a number at all.
    fn number() -> impl Strategy<Value = String> {
        prop_oneof![
            (-30_000.0_f64..30_000.0).prop_map(|value| format!("{value}")),
            any::<u32>().prop_map(|value| value.to_string()),
            any::<i64>().prop_map(|value| value.to_string()),
            any::<f64>().prop_map(|value| format!("{value:e}")),
            prop_oneof![
                Just("0"),
                Just("-0"),
                Just("1e-45"),
                Just("1e-400"),
                Just("3.4028235e38"),
                Just("3.5e38"),
                Just("1e308"),
                Just("1e999"),
                Just("65535"),
                Just("65536"),
                Just("4294967295"),
                Just("4294967296"),
                Just("18446744073709551616"),
                Just("-1"),
                Just("0.5"),
                Just("\"1\""),
                Just("null"),
                Just("true"),
                Just("[]"),
                Just("{}"),
            ]
            .prop_map(str::to_owned),
        ]
    }

    /// A string as JSON text: a box name, an escape, a control character, or anything.
    fn text() -> impl Strategy<Value = String> {
        prop_oneof![
            prop_oneof![
                Just("osc~"),
                Just("dac~"),
                Just("frequency"),
                Just(""),
                Just("x\\u001b[31m\\n"),
                Just("\\ud800"),
            ]
            .prop_map(str::to_owned),
            "[a-z~]{0,8}",
            any::<String>().prop_map(|value| format!("{value:?}").trim_matches('"').to_owned()),
        ]
        .prop_map(|value| format!("\"{value}\""))
    }

    /// A box as generated: whether it is an `osc~` (else a `dac~`), its frequency, and
    /// rarely a hostile id, box type, parameter or field in place of the valid ones.
    type BoxParts = (
        bool,
        String,
        Option<String>,
        Option<String>,
        Option<(String, String)>,
        &'static str,
    );

    /// A box: mostly valid, sometimes with a hostile part.
    fn node() -> impl Strategy<Value = BoxParts> {
        (
            any::<bool>(),
            prop_oneof![
                20 => (0.0_f32..=20_000.0).prop_map(|value| format!("{value}")),
                1 => number(),
            ],
            weighted(0.02, number()),
            weighted(0.02, text()),
            weighted(0.03, (text(), number())),
            prop_oneof![80 => Just(""), 1 => Just(",\"x\":1"), 1 => Just(",\"id\":2")],
        )
    }

    /// A patch document: mostly valid boxes (ids by position) and cables from an `osc~`
    /// to a `dac~` inlet, with rare hostile parts anywhere.
    fn document() -> impl Strategy<Value = String> {
        let hostile_cable = prop_oneof![
            (number(), number(), number(), number()).prop_map(|(source, outlet, target, inlet)| {
                format!("{{\"from\":[{source},{outlet}],\"to\":[{target},{inlet}]}}")
            }),
            number().prop_map(|value| format!("{{\"from\":[{value}],\"to\":[0,0]}}")),
            Just("{\"from\":[1,0,0],\"to\":[0,0]}".to_owned()),
            Just("{\"from\":[0,0],\"to\":[1,0],\"gain\":1}".to_owned()),
        ];
        (
            prop_oneof![80 => Just("1".to_owned()), 1 => number()],
            vec(node(), 0..6),
            vec(
                (
                    any::<u8>(),
                    any::<u8>(),
                    any::<u8>(),
                    weighted(0.03, hostile_cable),
                ),
                0..8,
            ),
            prop_oneof![80 => Just(""), 1 => Just(",\"extra\":0"), 1 => Just(",\"format\":1")],
        )
            .prop_map(|(format, nodes, cables, extra)| {
                let (mut oscillators, mut outputs) = (Vec::new(), Vec::new());
                let boxes: Vec<String> = nodes
                    .into_iter()
                    .zip(0_u32..)
                    .map(|((is_osc, frequency, id, kind, param, field), index)| {
                        if is_osc {
                            oscillators.push(index);
                        } else {
                            outputs.push(index);
                        }
                        let kind = kind.unwrap_or_else(|| {
                            (if is_osc { "\"osc~\"" } else { "\"dac~\"" }).to_owned()
                        });
                        let mut params: Vec<String> = Vec::new();
                        if is_osc {
                            params.push(format!("\"frequency\":{frequency}"));
                        }
                        params.extend(param.map(|(name, value)| format!("{name}:{value}")));
                        format!(
                            "{{\"id\":{},\"box\":{kind},\"params\":{{{}}}{field}}}",
                            id.unwrap_or_else(|| index.to_string()),
                            params.join(",")
                        )
                    })
                    .collect();
                let mut wires = alloc::collections::BTreeSet::new();
                for (from, to, inlet, hostile) in cables {
                    let pick = |list: &[u32], at: u8| {
                        usize::from(at)
                            .checked_rem(list.len())
                            .and_then(|position| list.get(position).copied())
                    };
                    let valid =
                        pick(&oscillators, from)
                            .zip(pick(&outputs, to))
                            .map(|(source, target)| {
                                format!("{{\"from\":[{source},0],\"to\":[{target},{}]}}", inlet % 2)
                            });
                    let _: bool = wires.insert(hostile.or(valid).unwrap_or_default());
                }
                let wires: Vec<String> =
                    wires.into_iter().filter(|wire| !wire.is_empty()).collect();
                format!(
                    "{{\"format\":{format},\"nodes\":[{}],\"connections\":[{}]{extra}}}",
                    boxes.join(","),
                    wires.join(",")
                )
            })
    }

    /// Builds and, when accepted, renders `patch` (read from `text`), computing in `S`: any
    /// failure but a refusal.
    fn play<S: Raw<Float: Delivered<S>>>(
        patch: &engine_core::Patch,
        text: &str,
        rate: SampleRate,
    ) -> Result<(), String> {
        let Ok(plan) = Plan::<S, BuiltIn<S>>::build(patch, &REGISTRY, rate) else {
            return Ok(());
        };
        let mut renderer = Renderer::new(plan);
        let mut samples = vec![S::ZERO.raw(); FRAMES.saturating_mul(renderer.channels())];
        let ((), operations) = heap_operations_during(|| renderer.render_into(&mut samples))?;
        if operations > 0 {
            return Err(format!("{operations} heap operations rendering {text}"));
        }
        if samples
            .iter()
            .any(|&sample| !S::from_raw(sample).to_binary64().raw().is_finite())
        {
            return Err(format!("non-finite samples from {text}"));
        }
        Ok(())
    }

    /// Reads, builds and, when accepted, renders `text` in every precision: any failure but
    /// a refusal.
    fn check(text: &str, rate: SampleRate) -> Result<(), String> {
        let Ok(patch) = engine_patch::parse_patch(text) else {
            return Ok(());
        };
        for precision in PRECISIONS {
            match precision {
                Precision::Binary64 => play::<Binary64>(&patch, text, rate),
                Precision::Binary32 => play::<Binary32>(&patch, text, rate),
            }
            .map_err(|problem| format!("binary{}: {problem}", precision.bits()))?;
        }
        Ok(())
    }

    /// Runs `check` on every document of `documents`, at the lowest rate (where `osc~`
    /// turns fastest).
    fn never_panics(documents: impl Strategy<Value = String>) -> Result<(), String> {
        runner(CASES)
            .run(&documents, |text| {
                check(&text, SampleRate::new(SampleRate::MIN).expect("supported"))
                    .map_err(TestCaseError::fail)
            })
            .map_err(|error| error.to_string())
    }

    #[test]
    fn arbitrary_text_is_refused_or_read_never_panics() {
        assert_eq!(
            never_panics(prop_oneof![
                any::<Vec<u8>>().prop_map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                any::<String>(),
                "[\\[\\]{}:,\"0-9a-z ]{0,64}",
            ]),
            Ok(()),
            "arbitrary text"
        );
    }

    #[test]
    fn hostile_patches_are_refused_or_played_never_panic() {
        assert_eq!(never_panics(document()), Ok(()), "patch documents");
    }

    #[test]
    fn deep_nesting_is_refused_never_overflows() {
        let deep = (1_usize..400).prop_map(|depth| {
            format!(
                "{{\"format\":1,\"nodes\":[{}{}],\"connections\":[]}}",
                "[".repeat(depth),
                "]".repeat(depth)
            )
        });
        assert_eq!(never_panics(deep), Ok(()), "nested arrays");
    }

    #[test]
    fn the_documents_reach_every_outcome() {
        // The generator is useful only if it reaches every stage: refused while reading,
        // refused by the plan, and played.
        let (mut unread, mut unbuilt, mut played) = (0_u32, 0_u32, 0_u32);
        let rate = SampleRate::new(SampleRate::MIN).expect("supported");
        let mut runner = runner(CASES);
        for _ in 0..CASES {
            let text = proptest::strategy::ValueTree::current(
                &document().new_tree(&mut runner).expect("a document"),
            );
            match engine_patch::parse_patch(&text) {
                Err(_) => unread += 1,
                Ok(patch) => match Plan::<Binary64>::build(&patch, &REGISTRY, rate) {
                    Err(_) => unbuilt += 1,
                    Ok(plan) => played += u32::from(plan.channels() > 0),
                },
            }
        }
        assert!(unread >= 100, "{unread} refused while reading");
        assert!(unbuilt >= 100, "{unbuilt} refused by the plan");
        assert!(played >= 100, "{played} played");
    }
}

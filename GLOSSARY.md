# Glossary

The words of the project, with the name each one takes in the patch format and in the code. One concept, one word: when a document or a message needs one of these concepts, it uses the word of this list. The names in the code that still differ (`Object`, `kind`, `node`) are kept until the public engine API takes its final names (roadmap ticket #5); the patch format's own naming conventions are roadmap ticket #3. This table maps them meanwhile.

## Patches and the engine

| Word | Meaning | Patch format | Code |
|---|---|---|---|
| **patch** | A graph of boxes connected by cables: what a user builds and the engine plays | the JSON document (`spec/patch-format.md`) | `Patch` (`engine-core`), parsed by `engine-patch` |
| **box** | One processing unit of a patch (an oscillator, an output) | an entry of `nodes` | `Node` (a box placed in a patch, `engine-core`); its behavior implements the `Object` trait (`engine-core`), registered in `engine-objects` |
| **box type** | What a box is, by name (`osc~`, `dac~`) | `nodes[].box` | `Node::kind`; `ObjectSpec` describes it |
| **box id** | The identifier of a box, unique in its patch; also orders execution among boxes ready at the same time | `nodes[].id` | `NodeId` |
| **parameter** | A named number set on a box (`frequency`); unset ones take their default | `nodes[].params` | `ParamSpec`, `Node::params` |
| **inlet**, **outlet** | The inputs and outputs of a box, numbered from 0 | `connections[].to`, `connections[].from` | `PortSpec` |
| **cable** | A connection from an outlet to an inlet; cables into one inlet are summed in a fixed order | an entry of `connections` | `Connection` |
| **plan** | The patch compiled for execution: boxes in a deterministic order, buffers allocated once | — | `Plan` (`engine-core`) |
| **block** | The fixed number of frames the engine processes at once, whatever the host buffer: each box processes one block per call | — | `Block`, `BLOCK_SIZE` (`engine-core`), `Plan::process`, `Object::process`, `engine_block_size` (`engine-wasm`) |
| **host buffer** | The span of frames a host asks the engine for at once (an audio callback, a call of an offline render); any length, and the result never depends on how a render is split into host buffers | — | `Plan::render`, `Plan::render_into`, `WasmEngine.render`; the sizes tested are `host_buffer_sizes` of the test matrix |
| **render quantum** | The frames a browser's audio thread processes at once (Web Audio): the host buffer of the engine in a browser, and what a tap delivers | — | `tap` and its `onBlock` callback, `parseTapBlock` (`engine-wasm`; names kept until roadmap ticket #5) |
| **frame**, **sample** | A frame is one instant of every channel; a sample is one channel's value at one frame (32-bit float) | — | `f32`; planar (one buffer per channel, mono `Block`s) inside the engine and in `WasmEngine.render`; interleaved by channel in `Plan::render`, `Plan::render_into` and the render hash |
| **sample rate** | Frames per second, 8 000 to 384 000 Hz | — | `SampleRate` |
| **render** | The output of a patch over a duration at a sample rate, offline | — | `engine-cli`, `renderOffline` (browser) |
| **render hash** | The SHA-256 of a render in canonical form: rate, channels and every sample's bits; equal hashes mean identical renders (`spec/render-hash.md`) | — | `render_hash` (`engine-hash`), `renderHash` (`@poyetik/engine-wasm/hash`, and the oracle's own) |

## Proof

| Word | Meaning | Where |
|---|---|---|
| **conformance case** | A patch, a rate and a duration whose render hash is recorded and must be reproduced on every target | `conformance/cases/` |
| **reference hash** | The recorded hash of a case; it changes only through `cargo xtask bless`, and the commit says why | `conformance/references.json` |
| **oracle** | The independent checker: rereads the native renders and checks them against the mathematical definition of their signal, sharing no code with the engine | `tooling/oracle/` |
| **test matrix** | The shared values every dimension is tested across (rates, host buffer sizes, …), never retyped | `conformance/test-matrix.json` |
| **box laws** | The properties every registered box must respect, each box run in isolation, listed in the crate documentation of `engine-testkit` | `engine-testkit` (`check_laws`) |
| **plan laws** | The properties every plan must respect, checked over random patches, among them that a render never depends on how it is split into host buffers; listed at the top of their test file | `crates/engine-testkit/tests/plan_laws.rs` |
| **check** | `cargo xtask check`: every quality check, as the pre-push hook and CI run it | `crates/xtask/` |

## Roadmap

| Word | Meaning | Data |
|---|---|---|
| **card** | One piece of work on the roadmap, in one column | an entry of `cards` |
| **column** | Where a card stands: Later, Next, In progress, Done | `column`: `later`, `next`, `doing`, `done` |
| **step** | One ordered part of a card, done or not; in a card in progress, the first step not done is the one in progress | an entry of `tasks` (the data keeps that key) |
| **milestone** | The stage of the project a card belongs to ("M1 Engine") | `milestone` |

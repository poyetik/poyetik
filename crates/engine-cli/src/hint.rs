//! What a refusal of `check` or `bless` says to do: every hint `engine-cli` gives, one
//! variant per state that gives it.
//!
//! A hint names the exact commands that succeed in its state (`CLAUDE.md`, "A hint is a
//! command"). Its alternatives are separated by `, or ` or `; `, the steps of one by
//! `, then `; a step is a `cargo xtask` command, a file to remove or restore from git, or a
//! change of the code or a review, said in words. The tests follow every alternative of
//! every variant from a state that gives it, and a variant without such a state does not
//! compile: they enumerate the variants through `HintKind::ALL`, which they declare from
//! the list that declares the enum (`with_hints`), and build each state in an exhaustive
//! `match`.

use std::path::Path;

/// Gives the list of every hint, its doc and its fields, to the macro `$then`: here
/// `declare_hint`, which declares [`Hint`]; and in the tests, the macro that declares
/// their kinds, `HintKind`, with every one of them in `HintKind::ALL`. One list, so that no
/// kind is left out.
macro_rules! with_hints {
    ($then:ident) => {
        $then! {
            /// `check`: a case with references and no versions recorded.
            CaseChangedOrRestore {
                /// The case.
                case: &'a str,
                /// The record, `algorithms.json`.
                record: &'a Path,
            },
            /// `check`: a case with neither references nor versions, a new case.
            NewCase,
            /// `check`: a box type at another algorithm version than its case's references.
            NewVersion,
            /// `check`: a box type a case uses without a version recorded.
            UnversionedBox,
            /// `check`: a render format of a case without a reference hash.
            NoReference,
            /// `bless`: one of the two files missing beside the other.
            RestoreBeside {
                /// The file missing.
                file: &'a Path,
            },
            /// `bless`: both files missing, without `--from-nothing`.
            FirstBless,
            /// `bless --from-nothing` with both files there.
            WithoutFromNothing,
            /// `bless --from-nothing` that wrote the references and not the record.
            RemoveThenFirstBless {
                /// The references, written.
                written: &'a Path,
            },
            /// `bless` that wrote the references and not the record, with its options.
            BlessAgain {
                /// Its cases of `--case-changed`.
                changed: &'a [String],
                /// Whether it was given `--battery-changed`.
                battery_changed: bool,
            },
            /// `check`: a file only bless writes missing.
            RestoreOrFirstBless {
                /// The file missing.
                file: &'a Path,
            },
            /// `check`: a file only bless writes that is not valid.
            RestoreInvalid {
                /// The file.
                file: &'a Path,
            },
            /// `check`: a file only bless writes, valid, but not as bless writes it.
            Rewrite,
            /// `bless`: a file it replaces that is not valid.
            RestoreCompared {
                /// The file.
                file: &'a Path,
            },
            /// The spent record (`--spent`) missing or not valid.
            WriteSpent {
                /// The command of `cargo xtask` that writes it.
                writer: &'a str,
            },
            /// `check`: references hashed with another render hash.
            Hash,
            /// `check`: references rendered under other render rules.
            Rules,
            /// `check`: no fingerprint of a box type at its current version.
            NoFingerprint,
            /// `check`: fingerprints of another battery, and nothing else changed.
            Battery,
            /// A battery change with another change, which no one bless accepts: `check`'s line of
            /// the battery, `bless`'s without `--battery-changed`, and each refusal of `bless
            /// --battery-changed`.
            BatteryAlone {
                /// The cases of `--case-changed` given, which the last bless needs again.
                changed: &'a [String],
            },
            /// `check`: other bits than the record on `main` at a version spent there.
            SpentBits {
                /// The box type.
                name: &'a str,
            },
            /// `check`: a box type's outputs changed at its recorded version.
            BitsChanged {
                /// The box type.
                name: &'a str,
            },
            /// `bless`: a box type's outputs changed at a spent version.
            BitsKept {
                /// The box type.
                name: &'a str,
            },
            /// `bless`: a case with references and no versions in the record it replaces.
            NoVersionsAtBless {
                /// The case.
                case: &'a str,
                /// The record, `algorithms.json`.
                record: &'a Path,
            },
            /// `bless`: a reference changed under the same versions, its case not named.
            ChangedReference {
                /// The case.
                case: &'a str,
            },
        }
    };
}

/// Declares [`Hint`] from the list `with_hints` gives.
macro_rules! declare_hint {
    ($(
        $(#[doc = $doc:literal])+
        $name:ident $({ $($(#[doc = $field_doc:literal])+ $field:ident: $type:ty),+ $(,)? })?
    ),+ $(,)?) => {
        /// What a refusal says to do, written by its [`Display`](core::fmt::Display).
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum Hint<'a> {
            $(
                $(#[doc = $doc])+
                $name $({ $($(#[doc = $field_doc])+ $field: $type),+ })?,
            )+
        }
    };
}

with_hints!(declare_hint);

/// The command that blesses again with `changed` named and the battery flag if
/// `battery_changed`.
fn bless_command(changed: &[String], battery_changed: bool) -> String {
    let mut command = "cargo xtask bless".to_owned();
    for case in changed {
        command.push_str(" --case-changed ");
        command.push_str(case);
    }
    if battery_changed {
        command.push_str(" --battery-changed");
    }
    command
}

impl core::fmt::Display for Hint<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CaseChangedOrRestore { case, record }
            | Self::NoVersionsAtBless { case, record } => write!(
                f,
                "`cargo xtask bless --case-changed {case}`, or restore {} from git",
                record.display()
            ),
            Self::NewCase
            | Self::UnversionedBox
            | Self::NoReference
            | Self::Rewrite
            | Self::Hash
            | Self::Rules
            | Self::NoFingerprint => f.write_str("run `cargo xtask bless`"),
            Self::NewVersion => {
                f.write_str("check the renders of the new version, then run `cargo xtask bless`")
            }
            Self::RestoreBeside { file } => write!(
                f,
                "restore {} from git: bless compares with both",
                file.display()
            ),
            Self::FirstBless => f.write_str("a first bless: `cargo xtask bless --from-nothing`"),
            Self::WithoutFromNothing => f.write_str("run `cargo xtask bless` without it"),
            Self::RemoveThenFirstBless { written } => write!(
                f,
                "remove {}, then run `cargo xtask bless --from-nothing`",
                written.display()
            ),
            Self::BlessAgain {
                changed,
                battery_changed,
            } => write!(
                f,
                "run `{}` again",
                bless_command(changed, *battery_changed)
            ),
            Self::RestoreOrFirstBless { file } => write!(
                f,
                "restore {} from git; a new directory: `cargo xtask bless --from-nothing`",
                file.display()
            ),
            Self::RestoreInvalid { file } => write!(f, "restore {} from git", file.display()),
            Self::RestoreCompared { file } => write!(
                f,
                "bless compares with it: restore {} from git",
                file.display()
            ),
            Self::WriteSpent { writer } => write!(f, "`{writer}` writes it: run that"),
            Self::Battery => f.write_str("run `cargo xtask bless --battery-changed`"),
            Self::BatteryAlone { changed } => write!(
                f,
                "bless the battery change alone first: set the other changes aside, then run \
                 `cargo xtask bless --battery-changed`, then commit, then bring the other \
                 changes back, then run `{}`",
                bless_command(changed, false)
            ),
            Self::SpentBits { name } | Self::BitsChanged { name } | Self::BitsKept { name } => {
                write!(f, "a new version of {name}, then run `cargo xtask bless`")
            }
            Self::ChangedReference { case } => write!(
                f,
                "a new version, then run `cargo xtask bless`, or `cargo xtask bless \
                 --case-changed {case}`"
            ),
        }
    }
}

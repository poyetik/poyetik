//! What a refusal of `check` or `bless` says to do: every hint `audio-cli` gives, one
//! variant per state that gives it.
//!
//! A hint names the exact commands that succeed in its state (`CLAUDE.md`, "A hint is a
//! command"). Its alternatives are separated by `, or ` or `; `, the steps of one by
//! `, then `; a step is a `cargo xtask` command as a runnable hint, a file to remove, a file
//! to restore with the git command that restores it ([`Restore`]), or a change of the code,
//! a review, or what stands in a file's way, said in words. The tests follow every alternative of every variant from a
//! state that gives it, and a variant without such a state does not compile: they enumerate
//! the variants through `HintKind::ALL`, which they declare from the list that declares the
//! enum (`with_hints`), and build each state in an exhaustive `match`.

use std::io::ErrorKind;
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
            /// `bless --battery-changed` with the battery unchanged.
            WithoutBatteryChanged,
            /// `bless --from-nothing` that wrote the references and not the record.
            RemoveThenFirstBless {
                /// The references, written.
                written: &'a Path,
            },
            /// `bless` that wrote the references and not the record, with its options.
            BlessAgain {
                /// Its cases of `--case-changed`.
                changed: &'a [String],
                /// Whether it was given `--live-changed`.
                live_changed: bool,
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
            /// A file that cannot be read since a directory stands in its place: never a restore
            /// beside it, which deletes the untracked files it holds; then the check again, as
            /// every tool says.
            MoveDirectory,
            /// A file its permissions keep unread: then the check again, as every tool says.
            MakeReadable,
            /// `check`: a file only bless writes, valid, but not as bless writes it. Bless
            /// alone: it compares the hashes it reads, so that no render that changed is taken
            /// in by rewriting the file.
            Rewrite,
            /// `check`: something recorded that no case, box type or render format has, which
            /// a bless drops.
            Stale,
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
            /// A battery change with another change, which no one bless accepts: the line of the
            /// battery of `check` and of `bless`, with or without `--battery-changed`.
            BatteryAlone {
                /// The cases the last bless names: those of `--case-changed` given, and those
                /// whose reference changed under the same algorithm versions.
                changed: &'a [String],
                /// Whether the last bless names the live vector: `--live-changed` given with
                /// a live vector, or a hash of it changed.
                live_changed: bool,
            },
            /// `check`: other bits than the spent record at a version spent there.
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
            /// `check` and `bless`: the live vector's hashes beside no live vector.
            RemoveLiveReferences {
                /// The hashes, `live-references.json`.
                file: &'a Path,
            },
            /// `bless`: a hash of the live vector changed, without `--live-changed`, with the
            /// options it was given that its command needs again.
            ChangedLive {
                /// Its cases of `--case-changed`.
                changed: &'a [String],
                /// Whether it was given `--from-nothing`.
                from_nothing: bool,
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

/// The command that blesses again with `changed` named, the live vector's flag if
/// `live_changed` and the battery's if `battery_changed`.
fn bless_command(changed: &[String], live_changed: bool, battery_changed: bool) -> String {
    let mut command = "cargo xtask bless".to_owned();
    for case in changed {
        command.push_str(" --case-changed ");
        command.push_str(case);
    }
    if live_changed {
        command.push_str(" --live-changed");
    }
    if battery_changed {
        command.push_str(" --battery-changed");
    }
    command
}

/// A hint after the problem it answers, ` (hint)`, or nothing while the command it names
/// would be refused: `check` gives none while the battery differs, since every bless but
/// the battery's is refused then, and the battery's line gives the procedure ([`said`]).
pub(crate) struct Said<'a>(Option<Hint<'a>>);

/// `hint` said after its problem when `shown`, else nothing ([`Said`]).
pub(crate) const fn said(hint: Hint<'_>, shown: bool) -> Said<'_> {
    Said(if shown { Some(hint) } else { None })
}

/// What a file that cannot be read for an error of `kind` asks, said after it: `damaged` when
/// its content is the reason (missing, too large, not text), a directory moved away, a
/// permission given; nothing for another failure (a disk's), which no command fixes.
pub(crate) fn read_hint(kind: ErrorKind, damaged: Hint<'_>) -> Said<'_> {
    let damage = [
        ErrorKind::NotFound,
        ErrorKind::FileTooLarge,
        ErrorKind::InvalidData,
    ];
    Said(if damage.contains(&kind) {
        Some(damaged)
    } else if kind == ErrorKind::IsADirectory {
        Some(Hint::MoveDirectory)
    } else if kind == ErrorKind::PermissionDenied {
        Some(Hint::MakeReadable)
    } else {
        None
    })
}

impl core::fmt::Display for Said<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0
            .as_ref()
            .map_or(Ok(()), |hint| write!(f, " ({hint})"))
    }
}

/// The step restoring `file` to its committed contents, as a runnable hint: `restore`, the
/// file as given, then the hint running `git checkout HEAD --` with it, from the last
/// commit, never from the index, which may hold the damage staged; the path absolute
/// ([`crate::files::absolute`]) and quoted for a shell ([`shell_word`]), so that the command
/// runs from any directory of the repository (`CLAUDE.md`, "A hint is a command").
struct Restore<'a>(&'a Path);

impl core::fmt::Display for Restore<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "restore {} (see: git checkout HEAD -- {})",
            self.0.display(),
            shell_word(&crate::files::absolute(self.0))
        )
    }
}

/// `path` as one word of a shell command: quoted for a shell when it holds anything but
/// letters, digits and `_./-` (a copy of `visual-cli`'s, until one crate holds the hint
/// builders of every tool, #24 step 109).
pub(crate) fn shell_word(path: &Path) -> String {
    let text = path.display().to_string();
    let plain = !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./-".contains(&byte));
    if plain {
        text
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

impl core::fmt::Display for Hint<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CaseChangedOrRestore { case, record }
            | Self::NoVersionsAtBless { case, record } => write!(
                f,
                "bless (see: cargo xtask bless --case-changed {case}), or {}",
                Restore(record)
            ),
            Self::NewCase
            | Self::UnversionedBox
            | Self::NoReference
            | Self::Rewrite
            | Self::Stale
            | Self::Hash
            | Self::Rules
            | Self::NoFingerprint => f.write_str("bless (see: cargo xtask bless)"),
            Self::NewVersion => f.write_str(
                "check the renders of the new version, then bless (see: cargo xtask bless)",
            ),
            Self::RestoreBeside { file } => {
                write!(f, "{}: bless compares with both", Restore(file))
            }
            Self::FirstBless => {
                f.write_str("a first bless (see: cargo xtask bless --from-nothing)")
            }
            Self::WithoutFromNothing | Self::WithoutBatteryChanged => {
                f.write_str("bless without it (see: cargo xtask bless)")
            }
            Self::RemoveThenFirstBless { written } => write!(
                f,
                "remove {}, then bless (see: cargo xtask bless --from-nothing)",
                written.display()
            ),
            Self::BlessAgain {
                changed,
                live_changed,
                battery_changed,
            } => write!(
                f,
                "bless again (see: {})",
                bless_command(changed, *live_changed, *battery_changed)
            ),
            Self::RestoreOrFirstBless { file } => write!(
                f,
                "{}; a new directory: bless (see: cargo xtask bless --from-nothing)",
                Restore(file)
            ),
            Self::RestoreInvalid { file } => write!(f, "{}", Restore(file)),
            Self::MoveDirectory => f.write_str(
                "a directory stands where the file should be: move it away, then check again \
                 (see: cargo xtask check)",
            ),
            Self::MakeReadable => {
                f.write_str("make it readable, then check again (see: cargo xtask check)")
            }
            Self::RestoreCompared { file } => {
                write!(f, "bless compares with it: {}", Restore(file))
            }
            Self::WriteSpent { writer } => write!(f, "write it (see: {writer})"),
            Self::Battery => f.write_str("bless (see: cargo xtask bless --battery-changed)"),
            Self::BatteryAlone {
                changed,
                live_changed,
            } => write!(
                f,
                "bless the battery change alone first: set the other changes aside, then bless \
                 (see: cargo xtask bless --battery-changed), then commit, then bring the other \
                 changes back, then bless (see: {})",
                bless_command(changed, *live_changed, false)
            ),
            Self::SpentBits { name } | Self::BitsChanged { name } | Self::BitsKept { name } => {
                write!(
                    f,
                    "a new version of {name}, then bless (see: cargo xtask bless)"
                )
            }
            Self::ChangedReference { case } => write!(
                f,
                "a new version, then bless (see: cargo xtask bless), or bless (see: cargo xtask \
                 bless --case-changed {case})"
            ),
            Self::RemoveLiveReferences { file } => write!(f, "remove {}", file.display()),
            Self::ChangedLive {
                changed,
                from_nothing,
            } => write!(
                f,
                "bless (see: {}{})",
                bless_command(changed, true, false),
                if *from_nothing { " --from-nothing" } else { "" }
            ),
        }
    }
}

//! How the check runs commands and reads files: the runner, its failures and its real implementation.

use core::num::NonZeroUsize;
use core::time::Duration;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Instant;

use crate::{
    changes::nul_separated,
    preflight::{Environment, cargo_config_candidates},
    steps::{Cmd, Dir},
};

/// How a command that ran ended without success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// It exited with this code (never 0).
    Code(i32),
    /// A signal stopped it: its number.
    Signal(i32),
}

/// Why a command did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// The program could not be started (missing, not executable): the system's reason.
    NotStarted(String),
    /// It ran and ended unsuccessfully, with the line of its error output that says why
    /// when that output was captured ([`error_reason`]).
    Ended {
        /// How it ended.
        exit: Exit,
        /// What it said about it.
        reason: Option<String>,
    },
    /// It succeeded but wrote something other than UTF-8 text.
    NotText,
}

/// A command that did not succeed: its line and why. Displayed as one line naming the
/// command once, with its exit code or signal and its own reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFailure {
    /// The command line ([`Cmd::line`]).
    pub line: String,
    /// Why it failed.
    pub failed: Failed,
}

impl CommandFailure {
    /// A failure of `cmd`.
    #[must_use]
    pub fn of(cmd: &Cmd, failed: Failed) -> Self {
        Self {
            line: cmd.line(),
            failed,
        }
    }
}

/// The usual names of the signals whose numbers POSIX fixes, the same on Linux and macOS.
pub(crate) const SIGNAL_NAMES: [(i32, &str); 8] = [
    (1, "SIGHUP"),
    (2, "SIGINT"),
    (3, "SIGQUIT"),
    (6, "SIGABRT"),
    (9, "SIGKILL"),
    (11, "SIGSEGV"),
    (13, "SIGPIPE"),
    (15, "SIGTERM"),
];

impl core::fmt::Display for CommandFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let line = &self.line;
        match &self.failed {
            Failed::NotStarted(error) => write!(f, "cannot run `{line}`: {error}"),
            Failed::NotText => write!(f, "`{line}` wrote output that is not UTF-8 text"),
            Failed::Ended { exit, reason } => {
                match exit {
                    Exit::Code(code) => write!(f, "`{line}` exited with code {code}")?,
                    Exit::Signal(signal) => {
                        write!(f, "`{line}` was stopped by signal {signal}")?;
                        if let Some((_, name)) =
                            SIGNAL_NAMES.iter().find(|(number, _)| number == signal)
                        {
                            write!(f, " ({name})")?;
                        }
                    }
                }
                reason
                    .as_ref()
                    .map_or(Ok(()), |reason| write!(f, ": {reason}"))
            }
        }
    }
}

/// The line of a command's error output that says why it failed.
///
/// The first line starting with `error` or `fatal` (cargo writes `help:` lines after its
/// error, rustup `info:` lines before it, git `fatal:`), otherwise the first non-empty
/// line; `None` when the command wrote nothing.
#[must_use]
pub fn error_reason(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = lines.clone().next();
    lines
        .find(|line| line.starts_with("error") || line.starts_with("fatal"))
        .or(first)
        .map(str::to_owned)
}

/// How a process that did not succeed ended.
pub(crate) fn exit_of(status: std::process::ExitStatus) -> Exit {
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt as _;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    // On Unix a process that has no exit code was stopped by a signal.
    status
        .code()
        .map_or_else(|| Exit::Signal(signal.unwrap_or_default()), Exit::Code)
}

/// What a command [`Runner::run_for_stdout`] ran wrote, and how it ended.
#[derive(Debug)]
pub struct Ran {
    /// What it wrote on stdout, its result: empty when the command did not start or wrote
    /// what is not text.
    pub stdout: String,
    /// What it wrote on stderr, bytes that are not UTF-8 replaced: empty when it did not
    /// start.
    pub stderr: String,
    /// Why the command could not be started, did not succeed (with the reason it wrote on
    /// stderr, [`error_reason`]), or wrote on stdout what is not text.
    pub ended: Result<(), CommandFailure>,
}

/// Executes commands and reads files on behalf of [`run_steps`](crate::actions::run_steps).
pub trait Runner {
    /// Runs a command, its output going to the terminal.
    ///
    /// # Errors
    ///
    /// Returns why the command could not be started or did not succeed.
    fn run(&mut self, cmd: &Cmd) -> Result<(), CommandFailure>;

    /// Reads a text file below the repository root.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read.
    fn read(&mut self, path: &str) -> io::Result<String>;

    /// Empties a directory below the repository root, creating it if needed.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be removed or created.
    fn fresh_dir(&mut self, path: &str) -> io::Result<()>;

    /// The files tracked by git matching any of `patterns` (git pathspecs such as
    /// `*.ts`), relative to the root, but those deleted from the work tree and not yet from
    /// the index: no gate can read them, and cargo builds the work tree.
    ///
    /// # Errors
    ///
    /// Returns why git could not list them.
    fn tracked(&mut self, patterns: &[&str]) -> Result<Vec<String>, CommandFailure>;

    /// Runs a command and returns what it wrote on stdout.
    ///
    /// # Errors
    ///
    /// Returns why the command could not be started or did not succeed, with the reason
    /// it wrote on stderr.
    fn capture(&mut self, cmd: &Cmd) -> Result<String, CommandFailure>;

    /// Runs a command whose stdout is its result: returns what it wrote there and on stderr,
    /// whether it succeeded or not, and how it ended, the reason it wrote on stderr
    /// included ([`Ran`]). Both are read whole once it ends, so that a caller can show them
    /// in the order the command wrote its result and its summary: nothing shows while it
    /// runs.
    fn run_for_stdout(&mut self, cmd: &Cmd) -> Ran;

    /// Writes `text` to a file below the repository root, creating its directory if needed.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory or the file cannot be written.
    fn write(&mut self, path: &str, text: &str) -> io::Result<()>;

    /// How many threads the machine can run at once (at least one).
    fn parallelism(&mut self) -> NonZeroUsize;

    /// What the environment holds that could change cargo from outside the repository.
    fn environment(&mut self) -> Environment;

    /// A reading of a monotonic clock, from an origin fixed for the life of the process:
    /// only the difference of two readings means anything, the time between them. The
    /// checks print it, and never decide anything on it.
    fn clock(&mut self) -> Duration;
}

/// The origin of [`SystemRunner`]'s clock: its first reading in the process.
pub(crate) static CLOCK_ORIGIN: OnceLock<Instant> = OnceLock::new();

/// Runs real commands below a repository root.
#[derive(Debug, Clone)]
pub struct SystemRunner {
    /// The repository root.
    pub root: PathBuf,
    /// Variables set for every command it starts, on top of xtask's own: none for the
    /// checks, which keep the user's git configuration (`safe.directory`, for one); a
    /// test's runner sets those that keep git from reading the machine's configuration.
    pub env: &'static [(&'static str, &'static str)],
}

/// The variables under which a test runs git without the machine's configuration.
///
/// Neither the system's configuration nor the user's (`GIT_CONFIG_NOSYSTEM`, `GIT_CONFIG_GLOBAL`:
/// their aliases, hooks path, `core.attributesFile`, `core.excludesFile`), nor the system's
/// attributes (`GIT_ATTR_NOSYSTEM`), nor the user's attributes, ignore file and
/// configuration that git reads below `XDG_CONFIG_HOME`, or `$HOME/.config` when it is not
/// set (`XDG_CONFIG_HOME` pointed at `/dev/null`, below which no file can be). The checks
/// keep the user's configuration ([`SystemRunner::env`]); xtask's tests, the hooks' ones
/// included, set every one of these.
pub const HERMETIC_GIT: [(&str, &str); 4] = [
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_ATTR_NOSYSTEM", "1"),
    ("XDG_CONFIG_HOME", "/dev/null"),
];

/// The variables git sets for a hook that name one repository.
///
/// They are listed by `git rev-parse --local-env-vars`. A command inheriting them acts on
/// that repository whatever its working directory, so every command xtask runs is started
/// without them: its working directory alone says which repository it works on. The list's
/// `GIT_NO_REPLACE_OBJECTS` is not among them: xtask sets it for git instead.
pub const LOCAL_GIT_VARIABLES: [&str; 14] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// The variables that change how git reads a pathspec: literal, glob or case-insensitive.
///
/// xtask's pathspecs (`*.rs`, `:(exclude)<dir>/`) are its own, written for git's default
/// reading: under `GIT_LITERAL_PATHSPECS` they match nothing, under `GIT_GLOB_PATHSPECS`
/// `*.rs` matches the files at the root only, and under `GIT_NOGLOB_PATHSPECS` nothing; a
/// gate given no file may pass vacuously. `GIT_ICASE_PATHSPECS` only widens a match, and is removed
/// with them. Git does not list them among its local variables ([`LOCAL_GIT_VARIABLES`]).
pub const PATHSPEC_GIT_VARIABLES: [&str; 4] = [
    "GIT_LITERAL_PATHSPECS",
    "GIT_GLOB_PATHSPECS",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
];

/// The variable whose diff options git applies after those of its command line:
/// `GIT_DIFF_OPTS=-u0`, a profile's wish for terse diffs, would drop the context lines
/// cargo-mutants reads to find the changed functions, whatever [`diff_since`](crate::changes::diff_since) asks for.
///
/// `GIT_EXTERNAL_DIFF` is overridden by `--no-ext-diff`; `GIT_ATTR_SOURCE`, which reads the
/// attributes from another tree, is out of scope: no honest change sets it for a check.
pub(crate) const DIFF_GIT_VARIABLES: [&str; 1] = ["GIT_DIFF_OPTS"];

/// A command for `program` in `dir`, with the variables of `env`, without the variables of
/// [`LOCAL_GIT_VARIABLES`], [`PATHSPEC_GIT_VARIABLES`] and [`DIFF_GIT_VARIABLES`], nor
/// `RUSTUP_TOOLCHAIN`, reading no input
/// (a command waiting for an answer fails instead of blocking the check), and for git with
/// `LC_ALL=C` (its messages untranslated, as [`error_reason`] reads them) and
/// `GIT_NO_REPLACE_OBJECTS=1`: git reads each commit as it is, never a replacement a local
/// ref gives it, so a replace ref cannot give the merge base or `HEAD` another tree. Other
/// ways of showing git another history (a grafts file, a shallow boundary edited by hand)
/// are out of scope: no honest change rewrites the history it is compared with.
///
/// Rustup sets that variable for xtask itself, to the toolchain it chose for it (`cargo
/// +beta xtask`, `RUSTUP_TOOLCHAIN=stable`): inherited, it would replace the toolchain of
/// [`RUST_TOOLCHAIN_FILE`](crate::preflight::RUST_TOOLCHAIN_FILE) for every command. Without it, each command runs with the
/// toolchain of the file (or of an override, which the preflight refuses).
pub(crate) fn command(program: &str, dir: &Path, env: &[(&str, &str)]) -> Command {
    let mut command = Command::new(program);
    let _in_dir = command
        .current_dir(dir)
        .stdin(Stdio::null())
        .envs(env.iter().copied());
    if program == "git" {
        let _untranslated = command
            .env("LC_ALL", "C")
            .env("GIT_NO_REPLACE_OBJECTS", "1");
    }
    let _pinned = command.env_remove("RUSTUP_TOOLCHAIN");
    for variable in LOCAL_GIT_VARIABLES
        .into_iter()
        .chain(PATHSPEC_GIT_VARIABLES)
        .chain(DIFF_GIT_VARIABLES)
    {
        let _without = command.env_remove(variable);
    }
    command
}

impl SystemRunner {
    /// The process of `cmd`, ready to start: [`command`] in its directory below the root,
    /// with its arguments and its own variables ([`Cmd::env`]).
    fn started(&self, cmd: &Cmd) -> Command {
        let mut started = command(cmd.program, &cmd.dir.under(&self.root), self.env);
        let _set = started
            .args(&cmd.args)
            .envs(cmd.env.iter().map(|(name, value)| (*name, value.as_str())));
        started
    }
}

impl Runner for SystemRunner {
    /// Runs `cmd`, its output written to xtask's stderr, both streams: a closed stdout
    /// (`cargo xtask check | head`) then stops none of the commands xtask starts (CLAUDE.md,
    /// command-line tools), while each keeps a terminal to write to, colors included.
    fn run(&mut self, cmd: &Cmd) -> Result<(), CommandFailure> {
        let status = self
            .started(cmd)
            .stdout(io::stderr())
            .status()
            .map_err(|error| CommandFailure::of(cmd, Failed::NotStarted(error.to_string())))?;
        if status.success() {
            Ok(())
        } else {
            Err(CommandFailure::of(
                cmd,
                Failed::Ended {
                    exit: exit_of(status),
                    reason: None,
                },
            ))
        }
    }

    fn read(&mut self, path: &str) -> io::Result<String> {
        std::fs::read_to_string(self.root.join(path))
    }

    fn fresh_dir(&mut self, path: &str) -> io::Result<()> {
        let dir = self.root.join(path);
        match std::fs::remove_dir_all(&dir) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        std::fs::create_dir_all(dir)
    }

    fn capture(&mut self, cmd: &Cmd) -> Result<String, CommandFailure> {
        let output = self
            .started(cmd)
            .output()
            .map_err(|error| CommandFailure::of(cmd, Failed::NotStarted(error.to_string())))?;
        if output.status.success() {
            String::from_utf8(output.stdout).map_err(|_| CommandFailure::of(cmd, Failed::NotText))
        } else {
            Err(CommandFailure::of(
                cmd,
                Failed::Ended {
                    exit: exit_of(output.status),
                    reason: error_reason(&output.stderr),
                },
            ))
        }
    }

    fn run_for_stdout(&mut self, cmd: &Cmd) -> Ran {
        let output = match self.started(cmd).output() {
            Ok(output) => output,
            Err(error) => {
                return Ran {
                    stdout: String::new(),
                    stderr: String::new(),
                    ended: Err(CommandFailure::of(
                        cmd,
                        Failed::NotStarted(error.to_string()),
                    )),
                };
            }
        };
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let Ok(stdout) = String::from_utf8(output.stdout) else {
            return Ran {
                stdout: String::new(),
                stderr,
                ended: Err(CommandFailure::of(cmd, Failed::NotText)),
            };
        };
        let ended = if output.status.success() {
            Ok(())
        } else {
            Err(CommandFailure::of(
                cmd,
                Failed::Ended {
                    exit: exit_of(output.status),
                    reason: error_reason(&output.stderr),
                },
            ))
        };
        Ran {
            stdout,
            stderr,
            ended,
        }
    }

    fn tracked(&mut self, patterns: &[&str]) -> Result<Vec<String>, CommandFailure> {
        // Separated by NUL, every name as it is: git quotes a name out of ASCII otherwise.
        let mut list = |listed: &str| {
            let mut args = vec!["ls-files", "-z", listed, "--"];
            args.extend(patterns);
            self.capture(&Cmd::new("git", &args, Dir::Root))
                .map(|text| nul_separated(&text))
        };
        let deleted = list("--deleted")?;
        list("--cached").map(|cached| {
            cached
                .into_iter()
                .filter(|path| !deleted.contains(path))
                .collect()
        })
    }

    fn write(&mut self, path: &str, text: &str) -> io::Result<()> {
        let file = self.root.join(path);
        file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
        std::fs::write(file, text)
    }

    fn parallelism(&mut self) -> NonZeroUsize {
        std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN)
    }

    /// The time since the first reading of the process (`CLOCK_ORIGIN`).
    #[expect(
        clippy::disallowed_methods,
        reason = "the one reading of the system clock of the checks: the elapsed time they print, which decides nothing (the ban keeps the engine counting time in samples)"
    )]
    fn clock(&mut self) -> Duration {
        CLOCK_ORIGIN.get_or_init(Instant::now).elapsed()
    }

    /// The names of xtask's own variables, which every command inherits, and the Cargo
    /// configuration files that exist above the root, resolved (a root that cannot be
    /// resolved is walked up as it is).
    fn environment(&mut self) -> Environment {
        let root = std::fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
        Environment {
            variables: std::env::vars_os()
                .map(|(name, _)| name.to_string_lossy().into_owned())
                .collect(),
            cargo_configs: cargo_config_candidates(
                &root,
                std::env::var_os("CARGO_HOME"),
                std::env::var_os("HOME"),
            )
            .into_iter()
            .filter(|path| path.is_file())
            .map(|path| path.display().to_string())
            .collect(),
        }
    }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;

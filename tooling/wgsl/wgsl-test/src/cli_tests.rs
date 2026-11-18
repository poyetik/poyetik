//! Tests of the command line: every command, every usage error, each problem said on one
//! line, and stdout closed or refusing.

use std::io::{self, Write};

use std::ffi::OsString;

use super::{CliError, MAX_SOURCE_BYTES, USAGE, arguments, read_at_most, read_source, run};

/// A valid module.
const VALID: &str = "fn f() -> u32 { return 1u; }\n";

/// A module refused at line 2, column 12 (a u32 returned for an f32).
const INVALID: &str = "fn f() -> f32 {\n    return 1u;\n}\n";

/// Runs `args` over the files `valid.wgsl` and `invalid.wgsl` (and no other), and returns
/// the outcome and what was written on stdout.
fn ran(args: &[&str]) -> (Result<(), CliError>, String) {
    let args: Vec<String> = args.iter().map(|&arg| arg.to_owned()).collect();
    let mut read = |path: &str| match path {
        "valid.wgsl" => Ok(VALID.to_owned()),
        "invalid.wgsl" => Ok(INVALID.to_owned()),
        _ => Err(io::Error::new(io::ErrorKind::NotFound, "no such file")),
    };
    let mut out = Vec::new();
    let outcome = run(&args, &mut read, &mut out);
    (outcome, String::from_utf8(out).expect("UTF-8"))
}

/// The usage error of `reason`.
fn usage(reason: &str) -> Result<(), CliError> {
    Err(CliError {
        code: 2,
        line: format!("{reason} (see: wgsl-test help)"),
    })
}

/// The failure of the work saying `line`.
fn failed(line: &str) -> Result<(), CliError> {
    Err(CliError {
        code: 1,
        line: line.to_owned(),
    })
}

#[test]
fn prints_its_usage_for_help_alone() {
    assert_eq!(
        ran(&["help"]),
        (Ok(()), USAGE.to_owned()),
        "the usage, whole"
    );
    assert!(USAGE.starts_with("usage: wgsl-test <command>\n"), "{USAGE}");
}

#[test]
fn refuses_each_usage_error_with_code_2() {
    for (args, reason) in [
        (&[][..], "no command"),
        (&["validate"][..], "validate names no file"),
        (&["help", "validate"][..], "help takes no argument"),
        (&["check"][..], "unknown command \"check\""),
        (&["check", "x.wgsl"][..], "unknown command \"check\""),
        (
            &["a\u{1b}[2J\nerror: forged"][..],
            "unknown command \"a\\u{1b}[2J\\u{a}error: forged\"",
        ),
    ] {
        assert_eq!(ran(args), (usage(reason), String::new()), "{args:?}");
    }
    let refused = usage("no command").expect_err("refused");
    assert_eq!(
        (refused.code(), refused.line()),
        (2, "no command (see: wgsl-test help)")
    );
}

#[test]
fn accepts_valid_files_silently() {
    assert_eq!(
        ran(&["validate", "valid.wgsl", "valid.wgsl"]),
        (Ok(()), String::new()),
        "nothing to say"
    );
}

#[test]
fn says_each_refused_file_on_one_line_then_counts_them() {
    let (outcome, out) = ran(&["validate", "invalid.wgsl", "valid.wgsl", "gone.wgsl"]);
    assert_eq!(
        outcome,
        failed("2 of 3 files refused, listed above"),
        "the refused counted among the files"
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "one line each, in order: {out}");
    assert!(
        lines
            .first()
            .is_some_and(|line| line.starts_with("invalid.wgsl:2:12: invalid: Function [0]")),
        "the file, the position, the stage and naga's message: {out}"
    );
    assert_eq!(
        lines.get(1).copied(),
        Some("gone.wgsl: cannot read: no such file"),
        "a file that cannot be read: {out}"
    );
}

#[test]
fn escapes_a_file_name_that_would_drive_the_terminal() {
    let (_, out) = ran(&["validate", "a\nerror: forged.wgsl"]);
    assert_eq!(
        out, "a\\u{a}error: forged.wgsl: cannot read: no such file\n",
        "one line, the name escaped"
    );
}

/// A stdout that refuses every write with `kind`.
struct Refusing(io::ErrorKind);

impl Write for Refusing {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(self.0, "refused"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn keeps_its_outcome_when_stdout_is_closed() {
    let mut read = |_: &str| Ok(INVALID.to_owned());
    for (args, outcome) in [
        (vec!["help"], Ok(())),
        (
            vec!["validate", "a.wgsl"],
            failed("1 of 1 files refused, listed above"),
        ),
    ] {
        let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
        assert_eq!(
            run(&args, &mut read, &mut Refusing(io::ErrorKind::BrokenPipe)),
            outcome,
            "{args:?}"
        );
    }
}

#[test]
fn fails_when_stdout_refuses_a_line_for_another_reason() {
    let mut read = |_: &str| Ok(INVALID.to_owned());
    for args in [vec!["help"], vec!["validate", "a.wgsl"]] {
        let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
        assert_eq!(
            run(&args, &mut read, &mut Refusing(io::ErrorKind::Other)),
            failed("cannot write to stdout: refused"),
            "{args:?}"
        );
    }
}

#[test]
fn takes_utf_8_arguments_and_refuses_others_as_a_usage_error() {
    assert_eq!(
        arguments(vec![OsString::from("validate"), OsString::from("é.wgsl")]),
        Ok(vec!["validate".to_owned(), "é.wgsl".to_owned()]),
        "UTF-8 arguments, kept in order"
    );
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        assert_eq!(
            arguments(vec![
                OsString::from("validate"),
                OsString::from_vec(vec![b'a', 0xff]),
                OsString::from_vec(vec![0xfe]),
            ]),
            Err(usage("argument 2 is not valid UTF-8").expect_err("a usage error")),
            "the first argument that is not UTF-8, by its position, and no other file read"
        );
    }
}

#[test]
fn reads_a_source_up_to_naga_s_32_bit_positions() {
    assert_eq!(
        MAX_SOURCE_BYTES,
        u64::from(u32::MAX),
        "naga's largest offset"
    );
    assert_eq!(
        read_source(&mut VALID.as_bytes()).ok().as_deref(),
        Some(VALID),
        "a source whole"
    );
    assert_eq!(
        read_at_most(&mut b"abc".as_slice(), 3).ok().as_deref(),
        Some("abc"),
        "kept at the bound"
    );
    let past = read_at_most(&mut b"abcd".as_slice(), 3).expect_err("one byte past it");
    assert_eq!(
        (past.kind(), past.to_string()),
        (
            io::ErrorKind::FileTooLarge,
            "larger than 3 bytes, more than naga's 32-bit positions address".to_owned()
        ),
        "refused one byte past the bound"
    );
    let mut endless = io::repeat(b'x');
    assert!(
        read_at_most(&mut endless, 3).is_err(),
        "an endless input refused once past the bound, never held whole"
    );
    let not_utf_8 = read_at_most(&mut [0xff_u8].as_slice(), 3).expect_err("not UTF-8");
    assert_eq!(not_utf_8.kind(), io::ErrorKind::InvalidData, "{not_utf_8}");
}

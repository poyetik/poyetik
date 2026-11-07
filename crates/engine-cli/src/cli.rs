//! The `engine-cli` commands.

use alloc::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Files, prepare_case, render_case, wav_file};

/// Usage of the `engine-cli` command.
pub const USAGE: &str = "usage: engine-cli <command>
  render <case.json> [--wav <file.wav>]  render a case, print its hash, optionally write a WAV file
  check <dir>                            compare every case of <dir>/cases with <dir>/references.json
  bless <dir>                            rewrite <dir>/references.json from the current renders
  export <dir> <out>                     write the WAV file of every case of <dir>/cases to <out>/<case>.wav";

/// Reads a file, naming it on failure.
fn read(files: &mut dyn Files, path: &Path) -> Result<String, String> {
    files
        .read(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// Writes a line of output.
fn say(out: &mut dyn Write, line: &str) -> Result<(), String> {
    writeln!(out, "{line}").map_err(|error| error.to_string())
}

/// Reads every case of `<dir>/cases`, sorted by name: `(name, JSON text)`.
fn cases(files: &mut dyn Files, dir: &Path) -> Result<Vec<(String, String)>, String> {
    let cases = dir.join("cases");
    let mut entries = files
        .list(&cases)
        .map_err(|error| format!("cannot list {}: {error}", cases.display()))?;
    entries.sort();
    let mut read_cases = Vec::new();
    for path in entries
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
    {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("invalid case file name: {}", path.display()))?;
        read_cases.push((name.to_owned(), read(files, path)?));
    }
    Ok(read_cases)
}

/// Renders every case of `<dir>/cases`, sorted by name: `(name, hash)`.
fn hashes(files: &mut dyn Files, dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut hashes = BTreeMap::new();
    for (name, json) in cases(files, dir)? {
        let rendered = render_case(&json).map_err(|e| format!("{name}: {e}"))?;
        // Case names are file names of one directory, hence unique.
        drop(hashes.insert(name, rendered.hash().to_string()));
    }
    Ok(hashes)
}

/// Where the reference hashes of a conformance directory live.
fn references_path(dir: &Path) -> PathBuf {
    dir.join("references.json")
}

/// `render <case> [--wav <file>]`.
fn render(files: &mut dyn Files, out: &mut dyn Write, args: &[String]) -> Result<(), String> {
    let (case, wav) = match args {
        [case] => (case, None),
        [case, flag, wav] if flag == "--wav" => (case, Some(wav)),
        _ => return Err(USAGE.to_owned()),
    };
    let prepared = prepare_case(&read(files, Path::new(case))?)?;
    let header = *prepared.wav_header();
    let rendered = prepared.render();
    if let Some(path) = wav {
        files
            .write(Path::new(path), &wav_file(&header, &rendered.samples))
            .map_err(|error| format!("cannot write {path}: {error}"))?;
    }
    say(out, &rendered.hash().to_string())
}

/// `check <dir>`.
fn check(files: &mut dyn Files, out: &mut dyn Write, dir: &Path) -> Result<(), String> {
    let actual = hashes(files, dir)?;
    let path = references_path(dir);
    let expected: BTreeMap<String, String> = serde_json::from_str(&read(files, &path)?)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    let mut failures = 0_usize;
    for (name, hash) in &actual {
        match expected.get(name) {
            Some(reference) if reference == hash => say(out, &format!("ok       {name} {hash}"))?,
            Some(reference) => {
                failures = failures.saturating_add(1);
                say(
                    out,
                    &format!("MISMATCH {name}: expected {reference}, got {hash}"),
                )?;
            }
            None => {
                failures = failures.saturating_add(1);
                say(
                    out,
                    &format!("MISSING  {name}: no reference hash (run bless)"),
                )?;
            }
        }
    }
    for name in expected.keys().filter(|name| !actual.contains_key(*name)) {
        failures = failures.saturating_add(1);
        say(out, &format!("STALE    {name}: reference without a case"))?;
    }
    if failures == 0 {
        Ok(())
    } else {
        Err(format!("conformance failed: {failures} problem(s)"))
    }
}

/// `bless <dir>`.
fn bless(files: &mut dyn Files, out: &mut dyn Write, dir: &Path) -> Result<(), String> {
    let actual = hashes(files, dir)?;
    let path = references_path(dir);
    let entries = actual
        .iter()
        .map(|(name, hash)| (name.clone(), serde_json::Value::from(hash.as_str())));
    let json = format!("{:#}\n", serde_json::Value::Object(entries.collect()));
    files
        .write(&path, json.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    say(
        out,
        &format!("blessed {} case(s) in {}", actual.len(), path.display()),
    )
}

/// `export <dir> <out>`: the WAV file of every case, for tools that check renders
/// independently of this engine.
fn export(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    target: &Path,
) -> Result<(), String> {
    files
        .create_dir_all(target)
        .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
    for (name, json) in cases(files, dir)? {
        let prepared = prepare_case(&json).map_err(|e| format!("{name}: {e}"))?;
        let header = *prepared.wav_header();
        let rendered = prepared.render();
        let path = target.join(format!("{name}.wav"));
        files
            .write(&path, &wav_file(&header, &rendered.samples))
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        say(
            out,
            &format!("wrote    {} {}", path.display(), rendered.hash()),
        )?;
    }
    Ok(())
}

/// Runs the command named by `args`.
///
/// # Errors
///
/// Returns a message for an unknown command, invalid arguments, an unreadable or
/// invalid case, or a conformance failure.
pub fn run(args: &[String], files: &mut dyn Files, out: &mut dyn Write) -> Result<(), String> {
    match args {
        [command, rest @ ..] if command == "render" => render(files, out, rest),
        [command, dir] if command == "check" => check(files, out, Path::new(dir)),
        [command, dir] if command == "bless" => bless(files, out, Path::new(dir)),
        [command, dir, target] if command == "export" => {
            export(files, out, Path::new(dir), Path::new(target))
        }
        _ => Err(USAGE.to_owned()),
    }
}

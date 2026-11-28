//! Tests of the widening of a diff to the functions it touches.

use alloc::collections::BTreeSet;

use super::{affected, function_spans, partly_changed, self_section, widen, widened};
use crate::steps::Failure;

/// A file of nine lines: a function moved into `checked` (lines 4 to 7) and another one,
/// `other` (lines 8 to 9), untouched.
const SOURCE: &str = "impl Square {\n    pub fn new() -> Self {\n        Self::checked()\n    }\n    fn checked() -> Self {\n        Self\n    }\n    fn other() {}\n}\n";

/// The diff of moving `new`'s body into `checked`: lines 3 to 5 added, the body kept as
/// context, as git shows such a move.
const MOVED: &str = "diff --git a/src/square.rs b/src/square.rs\n--- a/src/square.rs\n+++ b/src/square.rs\n@@ -2,4 +2,7 @@ impl Square {\n     pub fn new() -> Self {\n+        Self::checked()\n+    }\n+    fn checked() -> Self {\n         Self\n     }\n     fn other() {}\n";

/// cargo-mutants' listing of `SOURCE`'s mutants, its functions' spans with them, and one of a
/// file the diff does not change.
const LISTING: &str = r#"[
  {"file": "src/square.rs", "function": {"function_name": "Square::checked", "span": {"start": {"line": 5, "column": 5}, "end": {"line": 7, "column": 6}}}},
  {"file": "src/square.rs", "function": {"function_name": "Square::other", "span": {"start": {"line": 8, "column": 5}, "end": {"line": 8, "column": 18}}}},
  {"file": "src/square.rs"},
  {"file": "src/other.rs", "function": {"function_name": "f", "span": {"start": {"line": 1, "column": 1}, "end": {"line": 3, "column": 2}}}},
  {"file": "src/new.rs", "function": {"function_name": "f", "span": {"start": {"line": 1, "column": 1}, "end": {"line": 1, "column": 10}}}}
]"#;

#[test]
fn lists_the_rust_files_a_diff_changes_in_part() {
    let diff = format!(
        "{MOVED}diff --git a/src/new.rs b/src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1 @@\n+fn f() {{}}\ndiff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/src/gone.rs b/src/gone.rs\ndeleted file mode 100644\n--- a/src/gone.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-fn g() {{}}\n"
    );
    assert_eq!(partly_changed(&diff), ["src/square.rs"]);
}

#[test]
fn reads_the_lines_a_diff_affects_as_cargo_mutants_does() {
    // Each line added; the line before a deletion and the line after it, as cargo-mutants 27
    // counts them; context alone, nothing.
    assert_eq!(
        affected(MOVED),
        BTreeSet::from([3, 4, 5]),
        "added lines only"
    );
    let deleted = "--- a/f.rs\n+++ b/f.rs\n@@ -4,3 +4,2 @@\n a\n-b\n c\n";
    assert_eq!(
        affected(deleted),
        BTreeSet::from([4, 5]),
        "around a deletion"
    );
    let first = "--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,1 @@\n-a\n b\n";
    assert_eq!(
        affected(first),
        BTreeSet::from([1]),
        "a deletion at the first line"
    );
    let replaced = "--- a/f.rs\n+++ b/f.rs\n@@ -7,1 +7,1 @@\n-a\n+b\n\\ No newline at end of file\n@@ -20 +20,2 @@\n x\n+y\n";
    assert_eq!(
        affected(replaced),
        BTreeSet::from([6, 7, 21]),
        "two hunks, the second numbered afresh"
    );
}

#[test]
fn reads_the_spans_of_the_functions_cargo_mutants_lists() {
    let spans = function_spans(LISTING);
    assert_eq!(
        spans
            .as_ref()
            .map(|spans| spans.get("src/square.rs").cloned()),
        Ok(Some(BTreeSet::from([(5, 7), (8, 8)]))),
        "a mutant outside every function adds no span"
    );
    assert_eq!(
        spans.map(|spans| spans.get("src/other.rs").cloned()),
        Ok(Some(BTreeSet::from([(1, 3)])))
    );
    assert_eq!(
        function_spans("nothing").map(|_| ()),
        Err("cargo-mutants listed no JSON: expected ident at line 1 column 2".to_owned())
    );
    assert_eq!(
        function_spans("{}").map(|_| ()),
        Err("cargo-mutants listed no array of mutants".to_owned())
    );
    assert_eq!(
        function_spans("[{\"name\": \"x\"}]").map(|_| ()),
        Err("cargo-mutants listed a mutant without its file: {\"name\":\"x\"}".to_owned())
    );
}

#[test]
fn widens_the_lines_to_each_function_holding_one() {
    let spans = BTreeSet::from([(5, 7), (8, 8), (20, 30)]);
    assert_eq!(
        widened(&BTreeSet::from([3, 4, 5]), &spans),
        (BTreeSet::from([3, 4, 5, 6, 7]), 1),
        "the function whose first line is affected, whole; the others not"
    );
    assert_eq!(
        widened(&BTreeSet::from([30]), &spans),
        ((20..=30).collect(), 1),
        "a function whose last line is affected"
    );
    assert_eq!(
        widened(&BTreeSet::from([9]), &spans),
        (BTreeSet::from([9]), 0),
        "a line outside every function"
    );
}

#[test]
fn marks_each_line_added_as_itself_in_one_hunk_per_run() {
    assert_eq!(
        self_section("src/square.rs", SOURCE, &BTreeSet::from([2, 3, 5, 9, 10])),
        "diff --git a/src/square.rs b/src/square.rs\n--- a/src/square.rs\n+++ b/src/square.rs\n@@ -2,2 +2,2 @@\n-    pub fn new() -> Self {\n-        Self::checked()\n+    pub fn new() -> Self {\n+        Self::checked()\n@@ -5,1 +5,1 @@\n-    fn checked() -> Self {\n+    fn checked() -> Self {\n@@ -9,1 +9,1 @@\n-}\n+}\n",
        "line 10, past the end, left out"
    );
}

#[test]
fn marks_a_moved_body_added_whole_and_nothing_else() {
    let new_file = "diff --git a/src/new.rs b/src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1 @@\n+fn f() {}\n";
    let other = "diff --git a/src/other.rs b/src/other.rs\n--- a/src/other.rs\n+++ b/src/other.rs\n@@ -9 +9 @@\n-a\n+b\n";
    let diff = format!("{MOVED}{new_file}{other}");
    let mut read = |path: &str| {
        if path == "src/square.rs" {
            Ok(SOURCE.to_owned())
        } else {
            Err(Failure::from(format!("not read: {path}")))
        }
    };
    assert_eq!(
        widen(&diff, LISTING, &mut read),
        Ok((
            format!(
                "diff --git a/src/square.rs b/src/square.rs\n--- a/src/square.rs\n+++ b/src/square.rs\n@@ -3,5 +3,5 @@\n-        Self::checked()\n-    }}\n-    fn checked() -> Self {{\n-        Self\n-    }}\n+        Self::checked()\n+    }}\n+    fn checked() -> Self {{\n+        Self\n+    }}\n{new_file}{other}"
            ),
            1
        )),
        "the moved body whole; a new file, its function listed, and a section touching no listed \
         function as they were"
    );
    let mut unread = |path: &str| Err(Failure::from(format!("cannot read {path}")));
    assert_eq!(
        widen(MOVED, LISTING, &mut unread),
        Err(Failure::from("cannot read src/square.rs".to_owned())),
        "a file it cannot read"
    );
    assert_eq!(
        widen(MOVED, "{}", &mut read),
        Err(Failure::from(
            "cargo-mutants listed no array of mutants".to_owned()
        )),
        "a listing that is not cargo-mutants'"
    );
}

#[test]
fn selects_every_mutant_of_a_moved_body_with_the_real_cargo_mutants() {
    // The fixture's `new` delegates to `checked`, its body moved there: a diff adding the
    // signature of `checked` alone, the body kept as context, as git shows such a move.
    use crate::runner::Runner as _;
    let root = crate::tests::fixture("moved-body");
    let source = std::fs::read_to_string(root.join("src/lib.rs")).expect("the fixture's source");
    let signature = source
        .lines()
        .position(|line| line.trim_start().starts_with("const fn checked("))
        .and_then(|index| index.checked_add(1))
        .expect("the signature of `checked`");
    let line = source.lines().nth(signature - 1).unwrap_or_default();
    let moved = format!(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -{at},0 +{signature},1 @@\n+{line}\n",
        at = signature - 1
    );
    let mut system = crate::tests::system(root.clone());
    let listing = system
        .capture(&crate::steps::functions_list_command(&[
            "src/lib.rs".to_owned()
        ]))
        .expect("cargo-mutants lists the fixture's mutants");
    let mutants: Vec<serde_json::Value> = serde_json::from_str(&listing).expect("its JSON");
    let of_checked: BTreeSet<String> = mutants
        .iter()
        .filter(|mutant| {
            mutant
                .pointer("/function/function_name")
                .and_then(serde_json::Value::as_str)
                == Some("Side::checked")
        })
        .filter_map(|mutant| mutant.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        of_checked,
        [
            "src/lib.rs:15:18: replace > with < in Side::checked",
            "src/lib.rs:15:18: replace > with == in Side::checked",
            "src/lib.rs:15:18: replace > with >= in Side::checked",
            "src/lib.rs:15:9: replace Side::checked -> Option<Self> with None",
            "src/lib.rs:15:9: replace Side::checked -> Option<Self> with Some(Default::default())",
        ]
        .map(str::to_owned)
        .into(),
        "the body's mutants"
    );
    let scratch = crate::tests::scratch("moved-body");
    let mut selected = |diff: &str| {
        let path = scratch.0.join("changes.diff");
        std::fs::write(&path, diff).expect("the diff written");
        let listed = system
            .capture(&crate::steps::mutants_command(&[
                "--workspace",
                "--in-diff",
                &path.display().to_string(),
                "--list",
            ]))
            .expect("cargo-mutants filters by the diff");
        listed
            .lines()
            .map(str::to_owned)
            .collect::<BTreeSet<String>>()
    };
    assert_eq!(
        selected(&moved),
        BTreeSet::new(),
        "the diff as git shows the move: no mutant of the body kept"
    );
    let mut read = |path: &str| {
        std::fs::read_to_string(root.join(path)).map_err(|error| Failure::from(error.to_string()))
    };
    let (widened, functions) = widen(&moved, &listing, &mut read).expect("widened");
    assert_eq!(functions, 1, "`checked` alone");
    assert_eq!(
        selected(&widened),
        of_checked,
        "widened: every mutant of the body moved, and none of `units`"
    );
}

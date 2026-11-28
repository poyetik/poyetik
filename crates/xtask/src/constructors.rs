//! The delegating constructor gate: every `fn new` of a crate's sources is one call to a
//! function of another name, its parameters passed on.
//!
//! cargo-mutants never mutates a function named `new`, so whatever a `new` holds goes
//! unmutated: a check, an encoding, a field set. A `new` that only passes its parameters on to
//! a function of another name leaves nothing unmutated, and that function's body is mutated.

use crate::{
    rust_source::{RustSource, Token, TokenKind, close_of},
    steps::{Failure, step_verdict},
};

/// Whether `path`, a tracked file of the repository, is a source of a crate's code mutation
/// testing reaches: `crates/<crate>/src/`, a Rust file, not a unit test file (`*_tests.rs`).
fn crate_source(path: &str) -> bool {
    let mut parts = path.split('/');
    let rooted =
        parts.next() == Some("crates") && parts.next().is_some() && parts.next() == Some("src");
    rooted
        && std::path::Path::new(path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
        && !path.ends_with("_tests.rs")
}

/// The index of the body of the function whose `fn` keyword is at `at`: the first `{` after
/// the group of its parameters, or none for a function with no body (a trait's declaration).
fn body_of(tokens: &[Token], at: usize) -> Option<usize> {
    let parameters = (at..tokens.len())
        .find(|&index| tokens.get(index).is_some_and(|token| token.opens('(')))?;
    let after = close_of(tokens, parameters).saturating_add(1);
    let end = (after..tokens.len()).find(|&index| {
        tokens
            .get(index)
            .is_some_and(|token| token.opens('{') || token.is_punct(';'))
    })?;
    tokens
        .get(end)
        .filter(|token| token.opens('{'))
        .map(|_| end)
}

/// Whether `body`, the tokens inside a `new`'s braces, is one call to a function of another
/// name, every argument one word (a parameter passed on), never a type or a variant built:
/// `Self::checked(width, height)`,
/// `Self::made::<T, K>()`, `Factory::of(spec, instance)`.
fn delegates(body: &[Token]) -> bool {
    // The call's group: the first of the body, which closes at its end, so that a group
    // among the arguments (no parameter passed on) or anything after the call is refused.
    let Some(open) = body
        .iter()
        .position(|token| token.kind == TokenKind::Open)
        .filter(|&open| body.get(open).is_some_and(|token| token.opens('(')))
        .filter(|&open| close_of(body, open).saturating_add(1) == body.len())
    else {
        return false;
    };
    let path = body.get(..open).unwrap_or_default();
    let arguments = body
        .get(open.saturating_add(1)..body.len().saturating_sub(1))
        .unwrap_or_default();
    // The path: words and `::`, a turbofish's `<…>` aside, the last word outside it the callee.
    let mut depth = 0_usize;
    let mut callee = None;
    for token in path {
        match token.kind {
            TokenKind::Ident if depth == 0 => callee = Some(token.text.as_str()),
            TokenKind::Ident => {}
            TokenKind::Punct { .. } if token.is_punct('<') => depth = depth.saturating_add(1),
            TokenKind::Punct { .. } if token.is_punct('>') => depth = depth.saturating_sub(1),
            TokenKind::Punct { .. } if token.is_punct(':') || token.is_punct(',') => {}
            TokenKind::Punct { .. }
            | TokenKind::Literal
            | TokenKind::Open
            | TokenKind::Close
            | TokenKind::Doc { .. } => return false,
        }
    }
    let passed_on = arguments
        .iter()
        .all(|token| token.kind == TokenKind::Ident || token.is_punct(','));
    // A function's name is snake case (the compiler's lint): a word opening with a capital is
    // a type or a variant, built, not called (`Self(a)`, `Some(a)`).
    let function = |name: &str| {
        name != "new"
            && name
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
    };
    depth == 0 && passed_on && callee.is_some_and(function)
}

/// The `fn new` of `source` whose body is anything but one call to a function of another
/// name, each named by its line.
fn holding(source: &RustSource) -> Vec<String> {
    let tokens = &source.tokens;
    tokens
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| matches!(pair, [keyword, name] if keyword.is_ident("fn") && name.is_ident("new")))
        .filter_map(|(at, _)| {
            let open = body_of(tokens, at)?;
            let body = tokens.get(open.saturating_add(1)..close_of(tokens, open))?;
            let line = tokens.get(at).map_or(0, |token| token.line);
            (!delegates(body)).then(|| {
                format!(
                    "{}:{line}: a `new` holding more than one call to a function of another \
                     name, its parameters passed on: cargo-mutants never mutates a function named \
                     `new`, so move its body into one it mutates",
                    source.path
                )
            })
        })
        .collect()
}

/// Checks that every `fn new` of a crate's sources is one call to a function of another name.
///
/// The sources are `crates/<crate>/src/`, unit test files apart; every argument is a parameter
/// passed on (one word each), so that no code goes unmutated in a function cargo-mutants never
/// mutates. A function with no body (a trait's declaration) holds nothing.
///
/// Not seen: a constructor of another name, which cargo-mutants mutates, and a `new` outside
/// a crate's sources (a fixture's, a build script's), which no mutation run reaches.
///
/// # Errors
///
/// Returns each `new` holding more, with its file and line, or why a source does not lex.
pub fn delegating_new_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for (path, text) in sources.iter().filter(|(path, _)| crate_source(path)) {
        if text.contains("fn new") {
            problems.extend(holding(&RustSource::parse(path, text)?));
        }
    }
    step_verdict(problems)
}

#[cfg(test)]
#[path = "constructors_tests.rs"]
mod tests;

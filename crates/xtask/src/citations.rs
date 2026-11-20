//! The citation gate: every roadmap step a versioned file cites exists on the board.

use alloc::collections::BTreeMap;

use crate::{
    release::{CHANGELOG, released_heading},
    steps::{Failure, step_verdict},
};

/// The roadmap board, written only by `pnpm roadmap` (`apps/roadmap/`).
pub const ROADMAP_BOARD: &str = "apps/roadmap/board.json";

/// The tracked files the citation gate reads: every kind of text file that can cite a
/// roadmap step, the board included.
pub const CITATION_SOURCES: [&str; 13] = [
    "*.md", "*.rs", "*.ts", "*.tsx", "*.mts", "*.js", "*.mjs", "*.astro", "*.json", "*.toml",
    "*.txt", "*.yml", "*.yaml",
];

/// Whether each step of each card of `board` ([`ROADMAP_BOARD`]) is done, in order, by card
/// number: a step without `"done": true` is not.
///
/// # Errors
///
/// Returns why the board cannot be read.
pub(crate) fn board_steps(board: &str) -> Result<BTreeMap<u64, Vec<bool>>, Failure> {
    let json: serde_json::Value = serde_json::from_str(board).map_err(|error| {
        Failure::from(format!(
            "cannot read the roadmap board {ROADMAP_BOARD}: {error}"
        ))
    })?;
    json.get("cards")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            Failure::from(format!(
                "{ROADMAP_BOARD} lists no cards (see: pnpm roadmap validate)"
            ))
        })?
        .iter()
        .map(|card| {
            let id = card.get("id").and_then(serde_json::Value::as_u64);
            let steps = card
                .get("tasks")
                .and_then(serde_json::Value::as_array)
                .map(|steps| {
                    steps
                        .iter()
                        .map(|step| step.get("done") == Some(&serde_json::Value::Bool(true)))
                        .collect::<Vec<bool>>()
                });
            id.zip(steps).ok_or_else(|| {
                Failure::from(format!(
                    "{ROADMAP_BOARD} holds a card without its number or its steps (see: pnpm \
                     roadmap validate)"
                ))
            })
        })
        .collect()
}

/// The number `text` opens with, and the text after it.
pub(crate) fn leading_number(text: &str) -> Option<(u64, &str)> {
    let rest = text.trim_start_matches(|digit: char| digit.is_ascii_digit());
    let digits = text.strip_suffix(rest)?;
    digits.parse().ok().map(|number| (number, rest))
}

/// The ways one step number follows another in a list of steps: the last is a range from
/// the number before it.
pub(crate) const STEP_SEPARATORS: [&str; 5] = [", and ", ", ", " and ", " or ", " to "];

/// The steps the list `text` opens with cites (`M`, then any of [`STEP_SEPARATORS`] and a
/// number), and the text after it.
///
/// # Errors
///
/// Returns why the list cannot be read: a range that does not rise, or written with a dash.
pub(crate) fn step_list(text: &str) -> Result<(Vec<u64>, &str), String> {
    let (first, mut rest) =
        leading_number(text).ok_or_else(|| "a step without its number".to_owned())?;
    let mut steps = vec![first];
    let mut previous = first;
    while let Some((separator, (number, after))) = STEP_SEPARATORS.iter().find_map(|separator| {
        rest.strip_prefix(separator)
            .and_then(leading_number)
            .map(|found| (*separator, found))
    }) {
        if separator == " to " {
            if number <= previous {
                return Err(format!(
                    "the range of steps {previous} to {number} does not rise"
                ));
            }
            steps.extend(previous.saturating_add(1)..=number);
        } else {
            steps.push(number);
        }
        previous = number;
        rest = after;
    }
    if rest
        .strip_prefix(['-', '–'])
        .is_some_and(|after| after.starts_with(|digit: char| digit.is_ascii_digit()))
    {
        return Err(format!(
            "a range of steps from {previous} written with a dash: write `{previous} to N`"
        ));
    }
    Ok((steps, rest))
}

/// The roadmap steps `line` cites, each (card, step), and what in it the gate cannot read.
///
/// A step is cited as `#N step M` or `#N steps <list>` ([`step_list`]), with a comma,
/// spaces and an opening parenthesis allowed between them (`#N, step M`, `#N (step M`). On a line
/// of a spec opening with `**Step.**`, which must cite one, a list after `step` or `steps`
/// alone cites the card named last before it on the line. A card followed the same way by
/// `item`, `items`, `task` or `tasks` and a number is refused: a step is cited as a step.
/// Unless `released`, a card named as roadmap work ([`WORK_WORDS`] before it) and followed
/// by no step is refused too: a "not yet" names the step that ends it.
pub(crate) fn line_citations(line: &str, released: bool) -> (Vec<(u64, u64)>, Vec<String>) {
    let step_line = line.trim_start().starts_with("**Step.**");
    let mut cited = Vec::new();
    let mut problems = Vec::new();
    let mut card = None;
    let mut rest = line;
    let mut before = ' ';
    while let Some(first) = rest.chars().next() {
        let after_first = rest.get(first.len_utf8()..).unwrap_or_default();
        let ticket = (first == '#' && !before.is_alphanumeric() && before != '&')
            .then(|| leading_number(after_first))
            .flatten();
        let (named, list) = if let Some((number, after)) = ticket {
            card = Some(number);
            let follower = after.strip_prefix(',').unwrap_or(after);
            let follower = follower.trim_start_matches(' ');
            let follower = follower.strip_prefix('(').unwrap_or(follower);
            let Some(list) = ["steps ", "step "]
                .iter()
                .find_map(|word| follower.strip_prefix(word))
            else {
                if let Some((word, (step, _))) = ["items ", "item ", "tasks ", "task "]
                    .iter()
                    .find_map(|word| Some((word, leading_number(follower.strip_prefix(word)?)?)))
                {
                    problems.push(format!(
                        "#{number} {word}{step} cites no step: write `#{number} step {step}`"
                    ));
                } else if !released && named_as_work(line, rest) {
                    problems.push(format!(
                        "cites #{number} without the step that ends it: write `#{number} step M`"
                    ));
                }
                before = '0';
                rest = after;
                continue;
            };
            (Some(number), list)
        } else if (before == ' ' || before == '(')
            && let Some(list) = ["steps ", "step "]
                .iter()
                .find_map(|word| rest.strip_prefix(word))
                .filter(|list| list.starts_with(|digit: char| digit.is_ascii_digit()))
        {
            if !step_line {
                before = first;
                rest = after_first;
                continue;
            }
            if card.is_none() {
                problems.push(format!(
                    "a step cited without its card: write `#N step {}`",
                    leading_number(list).map_or(0, |(number, _)| number)
                ));
            }
            (card, list)
        } else {
            before = first;
            rest = after_first;
            continue;
        };
        match step_list(list) {
            Ok((steps, after)) => {
                if let Some(number) = named {
                    cited.extend(steps.into_iter().map(|step| (number, step)));
                }
                before = '0';
                rest = after;
            }
            Err(problem) => {
                problems.push(problem);
                before = '0';
                rest = list;
            }
        }
    }
    if step_line && cited.is_empty() && problems.is_empty() {
        problems.push("a **Step.** line citing no roadmap step: write `#N step M`".to_owned());
    }
    (cited, problems)
}

/// The words that name a card as roadmap work, each followed by a space: such a card is
/// cited with the step that ends the work ([`line_citations`]).
pub(crate) const WORK_WORDS: [&str; 2] = ["roadmap ", "ticket "];

/// The words that say a sentence's work is still to come, each matched as whole words in
/// any case: a step the board has done is never cited in such a sentence
/// ([`citation_gate`]).
pub(crate) const FUTURE_WORDS: [&str; 5] = ["not yet", "to come", "until", "later", "will"];

/// The first of [`FUTURE_WORDS`] `sentence` holds as whole words, in any case.
pub(crate) fn future_word(sentence: &str) -> Option<&'static str> {
    let lower = sentence.to_ascii_lowercase();
    FUTURE_WORDS.into_iter().find(|word| {
        lower.match_indices(word).any(|(at, _)| {
            let before = lower.get(..at).and_then(|text| text.chars().next_back());
            let after = lower
                .get(at.saturating_add(word.len())..)
                .and_then(|text| text.chars().next());
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    })
}

/// The sentences of `line`: its text split after each `.` or `;` that a space follows.
pub(crate) fn sentences(line: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0_usize;
    for (at, _) in line.match_indices([';', '.']) {
        let end = at.saturating_add(1);
        if line.get(end..).is_some_and(|rest| rest.starts_with(' ')) {
            sentences.extend(line.get(start..end));
            start = end;
        }
    }
    sentences.extend(line.get(start..));
    sentences
}

/// Whether the card at `rest`, a suffix of `line`, follows one of [`WORK_WORDS`] as a
/// whole word, in any case.
pub(crate) fn named_as_work(line: &str, rest: &str) -> bool {
    let lead = line
        .get(..line.len().saturating_sub(rest.len()))
        .unwrap_or_default();
    WORK_WORDS.iter().any(|word| {
        lead.len()
            .checked_sub(word.len())
            .and_then(|start| Some((lead.get(start..)?, lead.get(..start)?)))
            .is_some_and(|(tail, before)| {
                tail.eq_ignore_ascii_case(word)
                    && !before.ends_with(|letter: char| letter.is_alphanumeric())
            })
    })
}

/// Checks that every roadmap step a tracked file cites exists on the board.
///
/// `sources` are the tracked files of [`CITATION_SOURCES`] (path and text), the board
/// ([`ROADMAP_BOARD`]) among them. A line cites a step as `#N step M` or `#N steps` and a
/// list (`M, M and M`, `M or M`, `M to M`), a comma, spaces or an opening parenthesis
/// allowed after the card; on a spec's `**Step.**` line, which must cite one, `step M`
/// alone cites the card named last before it; `#N item M` (or `items`, `task`, `tasks`)
/// is refused, never skipped; so is a card named as roadmap work (`roadmap #N`, `ticket
/// #N`, the word and the card on one line) with no step, but in the released sections of
/// [`CHANGELOG`] (from its first heading `## [X.Y.Z] - YYYY-MM-DD` on, as
/// [`vocabulary_gate`](crate::vocabulary::vocabulary_gate) reads it), kept as released; and, outside those sections, a
/// step the board has done cited in a sentence ([`sentences`]) saying its work is to come
/// ([`FUTURE_WORDS`]). A step is cited by its number, so a
/// step or a card the board does not have means the citation is stale or wrong. A card
/// written `#N` alone names no work and is not read: it is how tests and the board name
/// cards.
///
/// # Errors
///
/// Returns every citation of a step or a card the board does not have, of a done step as
/// work to come, every citation the gate cannot read, or why the board cannot be read.
pub fn citation_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    let board = sources
        .iter()
        .find(|(path, _)| path == ROADMAP_BOARD)
        .map(|(_, text)| text.as_str())
        .ok_or_else(|| Failure::from(format!("cannot read the roadmap board {ROADMAP_BOARD}")))?;
    let steps = board_steps(board)?;
    let mut problems = Vec::new();
    for (path, text) in sources {
        let mut released = false;
        for (index, line) in text.lines().enumerate() {
            released |= path == CHANGELOG && released_heading(line).is_some();
            let at = format!("{path}:{}", index.saturating_add(1));
            let (cited, unread) = line_citations(line, released);
            problems.extend(unread.into_iter().map(|problem| format!("{at}: {problem}")));
            for (card, step) in cited {
                match steps.get(&card) {
                    None => problems.push(format!(
                        "{at}: cites #{card} step {step}, but the board has no card #{card}"
                    )),
                    Some(done)
                        if usize::try_from(step)
                            .map_or(true, |step| step == 0 || step > done.len()) =>
                    {
                        problems.push(format!(
                            "{at}: cites #{card} step {step}, but #{card} has {} steps: \
                             cite one of them, or add it (`pnpm roadmap step add {card} …`)",
                            done.len()
                        ));
                    }
                    Some(_) => {}
                }
            }
            // A step the board has done, cited in a sentence saying its work is to come, is
            // stale; a released section of the changelog says what was to come then.
            if !released {
                for sentence in sentences(line) {
                    let Some(word) = future_word(sentence) else {
                        continue;
                    };
                    for (card, step) in line_citations(sentence, released).0 {
                        let finished = usize::try_from(step)
                            .ok()
                            .and_then(|step| step.checked_sub(1))
                            .and_then(|position| steps.get(&card)?.get(position))
                            .copied()
                            .unwrap_or(false);
                        if finished {
                            problems.push(format!(
                                "{at}: cites #{card} step {step}, done on the board, as work to \
                                 come (`{word}`): cite the step that remains, or say it is done"
                            ));
                        }
                    }
                }
            }
        }
    }
    step_verdict(problems)
}

#[cfg(test)]
#[path = "citations_tests.rs"]
mod tests;

//! Local spelling preferences learned from edits to dictated text.
use crate::expansion_commands::DictionarySuggestion;
use crate::settings::{AppSettings, DictionaryTerm};

/// Align short transcripts so spacing changes do not shift every subsequent word.
/// Bound the alignment and only learn small spelling edits, never inserted prose.
pub fn correction_suggestions(before: &str, after: &str) -> Vec<DictionarySuggestion> {
    let tokenize = |text: &str| {
        text.split_whitespace()
            .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric() && c != '-'))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    if before.len() > 16_384 || after.len() > 16_384 {
        return vec![];
    }
    let a = tokenize(before);
    let b = tokenize(after);
    if a.len() > 256 || b.len() > 256 || a.iter().chain(&b).any(|s| s.len() > 512) {
        return vec![];
    }
    let mut common = vec![vec![0u16; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            common[i][j] = if a[i] == b[j] {
                1 + common[i + 1][j + 1]
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut changed) = (0, 0, 0);
    let mut suggestions = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        let (start_a, start_b) = (i, j);
        while (i < a.len() || j < b.len()) && !(i < a.len() && j < b.len() && a[i] == b[j]) {
            if j == b.len() || (i < a.len() && common[i + 1][j] >= common[i][j + 1]) {
                i += 1;
            } else {
                j += 1;
            }
        }
        changed += (i - start_a).max(j - start_b);
        if i == start_a || j == start_b || changed > 2 {
            return vec![];
        }
        let from = a[start_a..i].join(" ");
        let to = b[start_b..j].join(" ");
        let normalized = |s: &str| {
            s.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        };
        let ac = normalized(&from);
        let bc = normalized(&to);
        if from.len() > 512 || to.len() > 512 || ac.is_empty() || bc.is_empty() {
            return vec![];
        }
        let close = ac.chars().count() >= 4
            && bc.chars().count() >= 4
            && (edit_distance(&ac, &bc) <= 1 || adjacent_transposition(&ac, &bc));
        if ac != bc && !close {
            return vec![];
        }
        suggestions.push(DictionarySuggestion {
            before: from,
            after: to,
            frequency: 1,
        });
    }
    suggestions
}

fn adjacent_transposition(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len() != b.len() {
        return false;
    }
    let differences: Vec<usize> = a
        .iter()
        .zip(&b)
        .enumerate()
        .filter_map(|(i, (x, y))| (x != y).then_some(i))
        .collect();
    differences.len() == 2
        && differences[1] == differences[0] + 1
        && a[differences[0]] == b[differences[1]]
        && a[differences[1]] == b[differences[0]]
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j] + 1)
                .min(old + 1)
                .min(previous + usize::from(x != *y));
            previous = old;
        }
    }
    row[b.len()]
}

/// The latest spelling correction wins for this user's dictionary. No model training.
pub fn learn_into_settings(settings: &mut AppSettings, before: &str, after: &str) -> usize {
    if !settings.auto_learn_dictionary {
        return 0;
    }
    let mut learned = 0;
    for correction in correction_suggestions(before, after) {
        if settings.dismissed_corrections.iter().any(|pair| {
            pair.split_once('\n').is_some_and(|(a, b)| {
                a.eq_ignore_ascii_case(&correction.before) && b == correction.after
            })
        }) {
            continue;
        }
        if let Some(term) = settings
            .dictionary_terms
            .iter_mut()
            .find(|t| t.term.eq_ignore_ascii_case(&correction.before))
        {
            if term.preferred_spelling == correction.after {
                continue;
            }
            term.preferred_spelling = correction.after.clone();
        } else if settings.dictionary_terms.len() < 1000 {
            settings.dictionary_terms.insert(
                0,
                DictionaryTerm {
                    id: uuid::Uuid::new_v4().to_string(),
                    term: correction.before.clone(),
                    preferred_spelling: correction.after.clone(),
                    category: "Learned".into(),
                },
            );
        } else {
            continue;
        }
        // Keep existing aliases pointing at the user's newest spelling.
        for term in &mut settings.dictionary_terms {
            if term
                .preferred_spelling
                .eq_ignore_ascii_case(&correction.before)
            {
                term.preferred_spelling = correction.after.clone();
            }
        }
        settings
            .dictionary_suggestions
            .retain(|s| !s.before.eq_ignore_ascii_case(&correction.before));
        learned += 1;
    }
    learned
}

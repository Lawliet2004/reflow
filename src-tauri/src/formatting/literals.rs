use std::ops::Range;

/// Quote and code spans are content, including when a closing delimiter is
/// missing. Apostrophes in words and possessive endings are not delimiters.
pub(super) fn literal_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut quote = None;
    for (at, ch) in text.char_indices() {
        let previous = text[..at].chars().next_back();
        let next = text[at + ch.len_utf8()..].chars().next();
        if matches!(ch, '\'' | '’')
            && previous.is_some_and(char::is_alphanumeric)
            && next.is_some_and(char::is_alphanumeric)
        {
            continue;
        }
        if let Some((start, closing)) = quote {
            if ch == closing {
                ranges.push(start..at + ch.len_utf8());
                quote = None;
            }
        } else if matches!(ch, '"' | '“' | '‘' | '`')
            || ch == '\'' && !previous.is_some_and(char::is_alphanumeric)
        {
            quote = Some((
                at,
                match ch {
                    '“' => '”',
                    '‘' => '’',
                    _ => ch,
                },
            ));
        }
    }
    if let Some((start, _)) = quote {
        ranges.push(start..text.len());
    }
    ranges
}

pub(super) fn overlaps_literal(text: &str, span: Range<usize>) -> bool {
    literal_ranges(text)
        .iter()
        .any(|literal| span.start < literal.end && span.end > literal.start)
}

pub(crate) fn literal_contents(text: &str) -> Vec<&str> {
    literal_ranges(text)
        .into_iter()
        .map(|span| {
            let literal = &text[span];
            let opening = literal.chars().next().expect("literal delimiter");
            let closing = match opening {
                '“' => '”',
                '‘' => '’',
                _ => opening,
            };
            let start = opening.len_utf8();
            let end = if literal[start..].ends_with(closing) {
                literal.len() - closing.len_utf8()
            } else {
                literal.len()
            };
            &literal[start..end]
        })
        .collect()
}

/// Apply a text transform outside literal spans, retaining their exact bytes
/// and the whitespace that separates them from surrounding prose.
pub(super) fn map_unquoted(text: &str, transform: impl Fn(&str, bool, bool) -> String) -> String {
    let mut out = String::new();
    let mut from = 0;
    let append = |out: &mut String, span: Range<usize>| {
        let segment = &text[span.clone()];
        let core = segment.trim();
        if core.is_empty() {
            out.push_str(segment);
        } else {
            let start = segment.len() - segment.trim_start().len();
            let end = segment.trim_end().len();
            out.push_str(&segment[..start]);
            out.push_str(&transform(core, span.start == 0, span.end == text.len()));
            out.push_str(&segment[end..]);
        }
    };
    for literal in literal_ranges(text) {
        append(&mut out, from..literal.start);
        out.push_str(&text[literal.clone()]);
        from = literal.end;
    }
    append(&mut out, from..text.len());
    out
}

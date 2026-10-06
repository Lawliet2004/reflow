use regex::Regex;

/// Weekday and month names, which are always capitalised in English.
///
/// Stage 1 has to do this itself. Sentence capitalisation only fixes the first
/// word, so ASR output like "ship this on thursday" kept a lowercase proper noun
/// — and leaving it to the polish model means it is simply never fixed in Fast
/// mode, where there is no polish model. It is a closed, unambiguous list, so
/// there is no reason to spend an LLM call on it.
const PROPER_NOUNS: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "april",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Capitalise the closed set of always-capitalised words.
///
/// "may" and "march" are deliberately absent from the pattern as bare words:
/// they are common verbs ("may I", "march on") and capitalising them would be
/// wrong more often than right. Only unambiguous names are corrected.
fn capitalize_proper_nouns(text: &str) -> String {
    let mut out = text.to_string();
    for name in PROPER_NOUNS {
        let Ok(re) = Regex::new(&format!(r"(?i)\b{name}\b")) else {
            continue;
        };
        let mut capitalized = String::with_capacity(name.len());
        let mut chars = name.chars();
        if let Some(first) = chars.next() {
            capitalized.extend(first.to_uppercase());
            capitalized.push_str(chars.as_str());
        }
        out = re
            .replace_all(&out, |captures: &regex::Captures<'_>| {
                let found = captures.get(0).unwrap();
                if super::replacements::is_embedded_identifier(&out, found.start(), found.end()) {
                    found.as_str().to_owned()
                } else {
                    capitalized.clone()
                }
            })
            .into_owned();
    }
    out
}

/// Conservative spoken-list cleanup. Does not rewrite prose number-words.
pub fn apply_normalizers(text: &str, dictation_mode: &str) -> String {
    let mut out = text.trim().to_string();
    let mode = dictation_mode.to_lowercase();
    if matches!(
        mode.as_str(),
        "normal" | "notes" | "email" | "developer_prompt"
    ) {
        if let Some(list) = try_list(&out, mode == "normal") {
            out = list;
        }
    }
    // Not applied in coding mode: identifiers like `march` or `friday` are
    // ordinary lowercase symbols there, and rewriting them would break code.
    if !matches!(mode.as_str(), "coding" | "developer_prompt") {
        out = capitalize_proper_nouns(&out);
    }
    let spaces = Regex::new(r"[ \t]+\n").expect("trail space");
    out = spaces.replace_all(&out, "\n").to_string();
    let multi = Regex::new(r" {2,}").expect("multi space");
    multi.replace_all(&out, " ").trim().to_string()
}

fn word_key(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn marker_num(key: &str) -> Option<u32> {
    match key {
        "one" | "1" | "first" | "firstly" => Some(1),
        "two" | "2" | "second" | "secondly" => Some(2),
        "three" | "3" | "third" | "thirdly" => Some(3),
        "four" | "4" | "fourth" | "fourthly" => Some(4),
        "five" | "5" | "fifth" | "fifthly" => Some(5),
        "six" | "6" | "sixth" => Some(6),
        "seven" | "7" | "seventh" => Some(7),
        "eight" | "8" | "eighth" => Some(8),
        "nine" | "9" | "ninth" => Some(9),
        "ten" | "10" | "tenth" => Some(10),
        "eleven" | "11" | "eleventh" => Some(11),
        "twelve" | "12" | "twelfth" => Some(12),
        "twenty" | "20" => Some(20),
        "thirty" | "30" => Some(30),
        _ => None,
    }
}

/// Ordinals announce an enumeration on their own ("first …, second …"), while
/// bare cardinals ("one", "2") are common in prose and need a separator or an
/// explicit label like "point 2" to count as a list marker.
fn is_ordinal(key: &str) -> bool {
    matches!(
        key,
        "first"
            | "second"
            | "third"
            | "fourth"
            | "fifth"
            | "sixth"
            | "seventh"
            | "eighth"
            | "ninth"
            | "tenth"
            | "eleventh"
            | "twelfth"
            | "firstly"
            | "secondly"
            | "thirdly"
            | "fourthly"
            | "fifthly"
    )
}

/// Turns a spoken enumeration into a list: an optional lead-in line ending in
/// a colon, a blank line, then one item per line.
///
/// Normal dictation numbers explicit inventories; other short enumerations
/// stay in prose. Writing modes use "• " bullets;
/// an explicit, unnegated "numbered list" in the lead-in uses "1. ".
/// Anything that does not parse as a
/// strictly sequential enumeration returns `None` so prose like "I have one
/// cat and two dogs" passes through untouched.
fn try_list(text: &str, prose: bool) -> Option<String> {
    try_grouped_list(text, prose)
        .or_else(|| try_spoken_list(text, prose))
        .or_else(|| try_inventory_list(text, prose))
}

fn try_grouped_list(text: &str, numbered: bool) -> Option<String> {
    if text.contains('\n') {
        return None;
    }
    let tokens = Regex::new(r"\S+").ok()?.find_iter(text).collect::<Vec<_>>();
    let words = tokens
        .iter()
        .map(|token| token.as_str())
        .collect::<Vec<_>>();
    let protected = protected_list_words(&words)?;
    let mut ordinals = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let key = word_key(word);
        if protected[index]
            || !is_ordinal(&key)
            || words
                .get(index + 1)
                .is_some_and(|word| ordinal_qualifier(&word_key(word)))
            || index > 0 && is_determiner(&word_key(words[index - 1]))
        {
            continue;
        }
        ordinals.push((index, marker_num(&key)?));
    }
    if ordinals.first()?.1 != 1 {
        return None;
    }
    let mut boundaries = vec![0];
    for pair in ordinals.windows(2).filter(|pair| pair[1].1 == 1) {
        if pair[0].1 < 2 {
            return None;
        }
        let boundary = (pair[0].0 + 2..pair[1].0).find(|&index| {
            index > 0
                && !protected[index]
                && !protected[index + 1]
                && words[index - 1].ends_with([',', ';', '.', '!', '?'])
                && matches!(word_key(words[index]).as_str(), "and" | "then")
                && words.get(index + 1).is_some_and(|word| {
                    matches!(word_key(word).as_str(), "from" | "for" | "in" | "at" | "on")
                })
        })?;
        boundaries.push(tokens[boundary].start());
    }
    if boundaries.len() < 2 {
        return None;
    }
    boundaries.push(text.len());
    let mut groups = Vec::new();
    for bounds in boundaries.windows(2) {
        let first = ordinals
            .iter()
            .find(|&&(index, _)| tokens[index].start() >= bounds[0])?
            .0;
        if tokens[first].start() >= bounds[1] {
            return None;
        }
        let lead = text[bounds[0]..tokens[first].start()]
            .trim()
            .trim_end_matches([',', ';', '.', ':']);
        if lead.is_empty() {
            return None;
        }
        let lead = if looks_technical(lead.split_whitespace().next().unwrap_or_default()) {
            lead.to_string()
        } else {
            capitalize_first(lead)
        };
        let contents = text[tokens[first].start()..bounds[1]]
            .trim()
            .trim_end_matches([',', ';']);
        let prepared = format!("{lead}: {contents}");
        let rendered = try_spoken_list(&prepared, false).or_else(|| {
            // A comma inventory may follow a lone "first". Failed explicit
            // ordinal sequences must not be relabelled as comma inventories.
            if ordinals
                .iter()
                .filter(|&&(index, _)| {
                    tokens[index].start() >= bounds[0] && tokens[index].start() < bounds[1]
                })
                .count()
                != 1
            {
                return None;
            }
            let contents = text[tokens[first].end()..bounds[1]]
                .trim()
                .trim_end_matches([',', ';']);
            try_inventory_list_with_lead(&format!("{lead}: {contents}"), numbered, true)
        })?;
        if use_numbered_layout(&lead, numbered)? {
            let mut item = 0;
            groups.push(
                rendered
                    .lines()
                    .map(|line| {
                        if let Some(body) = line.strip_prefix("• ") {
                            item += 1;
                            format!("{item}. {body}")
                        } else {
                            line.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        } else {
            groups.push(rendered);
        }
    }
    Some(groups.join("\n\n"))
}

/// Quotes and bracketed expressions contain literal labels/arguments, not
/// enumeration markers or group boundaries. Unbalanced syntax stays intact.
fn protected_list_words(words: &[&str]) -> Option<Vec<bool>> {
    let mut quote = None;
    let mut brackets = Vec::new();
    let mut protected = Vec::with_capacity(words.len());
    for word in words {
        let mut literal = quote.is_some() || !brackets.is_empty();
        for (at, ch) in word.char_indices() {
            if let Some(closing) = quote {
                literal = true;
                if ch == closing {
                    quote = None;
                }
                continue;
            }
            match ch {
                '"' | '“' | '‘' => {
                    literal = true;
                    quote = Some(match ch {
                        '“' => '”',
                        '‘' => '’',
                        _ => '"',
                    });
                }
                '\'' if at == 0 => {
                    literal = true;
                    quote = Some('\'');
                }
                '(' | '[' | '{' => {
                    literal = true;
                    brackets.push(ch);
                }
                ')' | ']' | '}' if !brackets.is_empty() => {
                    literal = true;
                    if !matches!((brackets.pop()?, ch), ('(', ')') | ('[', ']') | ('{', '}')) {
                        return None;
                    }
                }
                ')' if word.strip_suffix(')').is_some_and(|body| {
                    !body.is_empty() && body.bytes().all(|byte| byte.is_ascii_digit())
                }) => {}
                ')' | ']' | '}' => return None,
                _ => {}
            }
        }
        protected.push(literal);
    }
    (quote.is_none() && brackets.is_empty()).then_some(protected)
}

fn ordinal_qualifier(word: &str) -> bool {
    matches!(
        word,
        "of" | "class"
            | "aid"
            | "hand"
            | "person"
            | "world"
            | "degree"
            | "floor"
            | "grade"
            | "edition"
            | "version"
            | "place"
            | "generation"
            | "time"
            | "year"
            | "day"
    )
}

fn is_determiner(word: &str) -> bool {
    matches!(
        word,
        "the"
            | "a"
            | "an"
            | "my"
            | "your"
            | "his"
            | "her"
            | "our"
            | "their"
            | "its"
            | "this"
            | "that"
    )
}

fn try_spoken_list(text: &str, prose: bool) -> Option<String> {
    let normal = prose;
    // A list must not consume words from an intentionally separate paragraph.
    if text.contains('\n') {
        return None;
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }

    // (word index, marker value, whether a "number"/"point"/"step" label word
    // immediately before the marker belongs to the marker rather than the text)
    let mut markers: Vec<(usize, u32, bool)> = Vec::new();
    let protected = protected_list_words(&words)?;
    for (i, w) in words.iter().enumerate() {
        if protected[i] {
            continue;
        }
        let key = word_key(w);
        let Some(n) = marker_num(&key) else {
            continue;
        };
        // Ordinal qualifiers belong to the item itself: dropping "first" in
        // "first class" or "first aid" would change what the speaker asked for.
        if is_ordinal(&key)
            && words
                .get(i + 1)
                .is_some_and(|next| ordinal_qualifier(&word_key(next)))
        {
            continue;
        }
        let prev_key = if i > 0 {
            word_key(words[i - 1])
        } else {
            String::new()
        };
        let labelled = matches!(prev_key.as_str(), "number" | "point" | "step");
        let spoken_lead = markers.is_empty() && has_list_lead_in(&words[..i]);
        let ordinal_continuation =
            !markers.is_empty() && is_ordinal(&key) && !is_determiner(&prev_key);
        let boundary = i == 0
            || (i > 0 && matches!(words[i - 1].chars().last(), Some(c) if ",.:;".contains(c)))
            || matches!(
                prev_key.as_str(),
                "and" | "then" | "number" | "point" | "step"
            )
            || spoken_lead
            || ordinal_continuation;
        if !boundary {
            continue;
        }
        if !is_ordinal(&key)
            && !labelled
            && !spoken_lead
            && !matches!(w.chars().last(), Some(c) if ",.:)".contains(c))
        {
            continue;
        }
        markers.push((i, n, labelled));
    }

    if markers.len() < 2
        || !markers
            .iter()
            .enumerate()
            .all(|(i, (_, n, _))| *n == i as u32 + 1)
    {
        return None;
    }

    let mut lead_words: Vec<&str> = words[..markers[0].0].to_vec();
    if markers[0].2 {
        lead_words.pop();
    }
    // Bare numeric text keeps its existing form in normal dictation. Spoken
    // ordinals or a counted lead-in provide the intent needed to edit prose.
    if prose && !is_ordinal(&word_key(words[markers[0].0])) && !has_list_lead_in(&lead_words) {
        return None;
    }
    if lead_words.len() >= 2
        && word_key(lead_words[lead_words.len() - 2]) == "that"
        && word_key(lead_words[lead_words.len() - 1]) == "is"
    {
        lead_words.truncate(lead_words.len() - 2);
    }
    let counted_lead = has_inventory_lead_in(&lead_words);
    let layout = requested_list_layout(&lead_words.join(" ")).ok()?;
    let prose = prose && !counted_lead && layout.is_none();

    let mut items: Vec<Vec<&str>> = Vec::new();
    for (index, &(marker_at, _, _)) in markers.iter().enumerate() {
        let end = markers.get(index + 1).map(|m| m.0).unwrap_or(words.len());
        let mut item: Vec<&str> = words[marker_at + 1..end].to_vec();
        if markers.get(index + 1).map(|m| m.2).unwrap_or(false) {
            item.pop();
        }
        while item.last().is_some_and(|w| word_key(w) == "and") {
            item.pop();
        }
        if item.is_empty() {
            return None;
        }
        let head = word_key(item[0]);
        if matches!(
            head.as_str(),
            "of" | "or" | "and" | "hundred" | "thousand" | "million"
        ) {
            return None;
        }
        items.push(item);
    }

    // Dictated lists often run straight into the next sentence ("3, add tests.
    // Let me know"). When every earlier item is a single sentence, a boundary
    // inside the last one is where the list ended: keep the first sentence as
    // the last bullet and give the remainder its own paragraph.
    let mut tail: Option<Vec<&str>> = None;
    if items[..items.len() - 1]
        .iter()
        .all(|item| !has_sentence_boundary(item))
    {
        if let Some(cut) = sentence_boundary(items.last().expect("nonempty")) {
            let last = items.pop().expect("nonempty");
            tail = Some(last[cut..].to_vec());
            items.push(last[..cut].to_vec());
        }
    }

    let items: Vec<String> = items
        .iter()
        .map(|item| {
            // A bullet is a fragment: drop trailing separators and capitalise,
            // unless the first word is an identifier or path whose casing the
            // speaker dictated on purpose.
            let mut item = item.clone();
            while let Some(last) = item.last_mut() {
                // Shell path arguments are content, even when they look like
                // terminal punctuation ("git add .", "cd ..").
                if matches!(*last, "." | "..") {
                    break;
                }
                let stripped = last.trim_end_matches([',', ';', '.']);
                if stripped.is_empty() {
                    item.pop();
                } else {
                    *last = stripped;
                    break;
                }
            }
            let mut text = item.join(" ");
            if !prose && !looks_technical(item.first().copied().unwrap_or_default()) {
                text = capitalize_first(&text);
            }
            text
        })
        .collect();

    let lead_in = lead_words.join(" ");
    let use_numbers = use_numbered_layout(&lead_in, normal && counted_lead)?;
    let lead_in = lead_in
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches([' ', '\t', ',', '.', ';', ':'])
        .to_string();
    let lead_in = if lead_in.is_empty() {
        String::new()
    } else {
        format!("{lead_in}:")
    };

    if prose {
        let body = if items.len() == 2 {
            format!("{} and {}", items[0], items[1])
        } else {
            format!(
                "{}, and {}",
                items[..items.len() - 1].join(", "),
                items.last()?
            )
        };
        let terminal = text.trim().chars().last().filter(|c| ".!?".contains(*c));
        let mut output = if lead_in.is_empty() {
            body
        } else {
            format!("{lead_in} {body}")
        };
        if let Some(tail) = tail {
            if !output.ends_with(['.', '?', '!']) {
                output.push('.');
            }
            output.push(' ');
            output.push_str(&tail.join(" "));
        } else if let Some(terminal) = terminal {
            if !output.ends_with(terminal) {
                output.push(terminal);
            }
        }
        return Some(output);
    }

    Some(render_inventory(
        &lead_in,
        &items,
        use_numbers,
        tail.as_ref().map(|words| words.join(" ")).as_deref(),
    ))
}

fn render_inventory(lead_in: &str, items: &[String], numbered: bool, tail: Option<&str>) -> String {
    let mut body = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            if numbered {
                format!("{}. {item}", i + 1)
            } else {
                format!("• {item}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(tail) = tail {
        let mut text = tail.to_string();
        if !looks_technical(tail.split_whitespace().next().unwrap_or_default()) {
            text = capitalize_first(&text);
        }
        body.push_str("\n\n");
        body.push_str(&text);
    }
    if lead_in.is_empty() {
        body
    } else {
        format!("{lead_in}\n\n{body}")
    }
}

fn try_inventory_list(text: &str, numbered: bool) -> Option<String> {
    try_inventory_list_with_lead(text, numbered, false)
}

fn try_inventory_list_with_lead(
    text: &str,
    numbered: bool,
    confirmed_group: bool,
) -> Option<String> {
    if text.contains('\n') {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    let (lead, contents) = text.split_once(':').or_else(|| {
        let at = lower.find(" that is ")?;
        Some((&text[..at], &text[at + " that is ".len()..]))
    })?;
    if !confirmed_group && !has_inventory_lead_in(&lead.split_whitespace().collect::<Vec<_>>()) {
        return None;
    }
    // Delimit only at the inventory's top level: commas inside a quoted label
    // or function arguments belong to one complete item.
    let mut quote = None;
    let mut brackets = Vec::new();
    let mut start = 0;
    let mut parts = Vec::new();
    for (at, ch) in contents.char_indices() {
        if let Some(closing) = quote {
            if ch == closing {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' => quote = Some('"'),
            '“' => quote = Some('”'),
            '‘' => quote = Some('’'),
            '\'' if at == 0 || contents[..at].ends_with(char::is_whitespace) => quote = Some('\''),
            '(' | '[' | '{' => brackets.push(ch),
            ')' | ']' | '}' => {
                if !matches!((brackets.pop()?, ch), ('(', ')') | ('[', ']') | ('{', '}')) {
                    return None;
                }
            }
            ',' if brackets.is_empty()
                && !(contents[..at].ends_with(char::is_numeric)
                    && contents[at + ch.len_utf8()..].starts_with(char::is_numeric)) =>
            {
                parts.push(contents[start..at].trim());
                start = at + ch.len_utf8();
            }
            _ => {}
        }
    }
    if quote.is_some() || !brackets.is_empty() || parts.is_empty() {
        return None;
    }
    parts.push(contents[start..].trim());
    if let Some(last) = parts.last_mut() {
        *last = last.strip_prefix("and ").unwrap_or(last).trim();
    }
    if parts.iter().any(|part| {
        part.is_empty()
            || matches!(
                word_key(part.split_whitespace().next().unwrap_or_default()).as_str(),
                "and" | "or" | "but" | "which" | "who" | "because"
            )
    }) {
        return None;
    }
    if parts[..parts.len() - 1]
        .iter()
        .any(|part| has_sentence_boundary(&part.split_whitespace().collect::<Vec<_>>()))
    {
        return None;
    }
    let mut parts = parts.into_iter().map(str::to_string).collect::<Vec<_>>();
    let mut tail = None;
    let last_words = parts.last()?.split_whitespace().collect::<Vec<_>>();
    if let Some(cut) = sentence_boundary(&last_words) {
        tail = Some(last_words[cut..].join(" "));
        let item = last_words[..cut].join(" ");
        *parts.last_mut()? = item;
    }
    let items = parts
        .iter()
        .map(|part| {
            let stripped = if matches!(part.as_str(), "." | "..")
                || part.ends_with(" .")
                || part.ends_with(" ..")
            {
                part.as_str()
            } else {
                part.trim_end_matches([',', ';', '.'])
            };
            if looks_technical(stripped.split_whitespace().next().unwrap_or_default()) {
                stripped.to_string()
            } else {
                capitalize_first(stripped)
            }
        })
        .collect::<Vec<_>>();
    if items.iter().any(|item| item.trim().is_empty()) {
        return None;
    }
    Some(render_inventory(
        &format!("{}:", lead.trim()),
        &items,
        use_numbered_layout(lead, numbered)?,
        tail.as_deref(),
    ))
}

fn use_numbered_layout(lead: &str, requested: bool) -> Option<bool> {
    requested_list_layout(lead)
        .ok()
        .map(|layout| layout.unwrap_or(requested))
}

/// An explicit layout wins over the mode's default. Quoted labels are content;
/// negation applies to the nearby layout directive, not an unrelated clause.
fn requested_list_layout(lead: &str) -> Result<Option<bool>, ()> {
    let words = lead.split_whitespace().collect::<Vec<_>>();
    let protected = protected_list_words(&words).ok_or(())?;
    let keys = words.iter().map(|word| word_key(word)).collect::<Vec<_>>();
    let mut specified = [false; 2];
    let mut forbidden = [false; 2];
    for index in 0..words.len().saturating_sub(1) {
        if protected[index] || protected[index + 1] {
            continue;
        }
        let numbered = match (keys[index].as_str(), keys[index + 1].as_str()) {
            ("numbered", "list" | "lists" | "points" | "steps") => true,
            ("bullet" | "bulleted", "list" | "lists" | "points") => false,
            _ => continue,
        };
        let start = (0..index)
            .rfind(|&at| {
                !protected[at]
                    && (words[at].ends_with([',', ';', '.', ':', '!', '?'])
                        || matches!(keys[at].as_str(), "but" | "instead")
                        || keys[at] == "and"
                            && keys.get(at + 1).is_some_and(|word| {
                                matches!(
                                    word.as_str(),
                                    "use" | "make" | "create" | "write" | "render" | "format"
                                )
                            }))
            })
            .map_or(0, |at| at + 1);
        let negated = (start..index)
            .rfind(|&at| {
                !protected[at]
                    && matches!(
                        keys[at].as_str(),
                        "not" | "dont" | "never" | "no" | "without" | "avoid"
                    )
            })
            .is_some_and(|at| {
                keys[at + 1..index].iter().all(|word| {
                    matches!(
                        word.as_str(),
                        "a" | "an"
                            | "any"
                            | "use"
                            | "using"
                            | "make"
                            | "create"
                            | "write"
                            | "render"
                            | "format"
                            | "as"
                            | "into"
                            | "please"
                            | "or"
                            | "and"
                            | "bullet"
                            | "bulleted"
                            | "numbered"
                            | "points"
                            | "steps"
                            | "list"
                            | "lists"
                    )
                })
            });
        if negated {
            forbidden[usize::from(numbered)] = true;
        } else {
            specified[usize::from(numbered)] = true;
        }
    }
    match (specified, forbidden) {
        ([false, false], [false, false]) => Ok(None),
        ([false, false], [true, false]) | ([false, true], [_, false]) => Ok(Some(true)),
        ([false, false], [false, true]) | ([true, false], [false, _]) => Ok(Some(false)),
        _ => Err(()),
    }
}

/// Content of a confirmed enumeration, with structural markers removed.
/// Used by rewrite validation to distinguish item quantities from item indices.
pub(crate) fn spoken_list_content(text: &str) -> Option<String> {
    try_list(text, false)
}

/// A confirmed list's scope, complete items and following prose. Keeping the
/// scope alongside its items prevents validation from moving items between
/// independently enumerated groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnumerationGroup {
    pub header: String,
    pub items: Vec<String>,
    pub tail: String,
}

pub(crate) fn enumeration_groups(text: &str) -> Option<Vec<EnumerationGroup>> {
    let normalized;
    let source = if text.contains('\n') {
        text
    } else {
        normalized = try_list(text, false)?;
        &normalized
    };
    let lines = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let mut groups = Vec::new();
    let mut current: Option<EnumerationGroup> = None;
    let mut header = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some(item) = list_item(line) {
            if item.is_empty() {
                return None;
            }
            let group = current.get_or_insert_with(|| EnumerationGroup {
                header: header.join("\n"),
                items: Vec::new(),
                tail: String::new(),
            });
            header.clear();
            // Item rows after prose require a fresh, explicit group header.
            if !group.tail.is_empty() {
                return None;
            }
            group.items.push(item.to_string());
        } else if current.is_some()
            && line.ends_with(':')
            && lines
                .get(index + 1)
                .is_some_and(|next| list_item(next).is_some())
        {
            groups.push(current.take()?);
            header.push(*line);
        } else if let Some(group) = current.as_mut() {
            if !group.tail.is_empty() {
                group.tail.push('\n');
            }
            group.tail.push_str(line);
        } else {
            header.push(*line);
        }
    }
    if let Some(group) = current {
        groups.push(group);
    }
    (groups.iter().map(|group| group.items.len()).sum::<usize>() >= 2).then_some(groups)
}

fn list_item(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix(['•', '-', '*']) {
        return rest.starts_with(char::is_whitespace).then(|| rest.trim());
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && matches!(line.as_bytes().get(digits), Some(b'.') | Some(b')')) {
        let rest = &line[digits + 1..];
        return rest.starts_with(char::is_whitespace).then(|| rest.trim());
    }
    None
}

/// Complete item text in order, shared by formatting and rewrite validation.
pub(crate) fn enumeration_items(text: &str) -> Option<Vec<String>> {
    Some(
        enumeration_groups(text)?
            .into_iter()
            .flat_map(|group| group.items)
            .collect(),
    )
}

fn has_inventory_lead_in(words: &[&str]) -> bool {
    has_list_lead_in(words)
        || (words.len() >= 2
            && matches!(
                word_key(words[words.len() - 1]).as_str(),
                "things" | "items" | "steps" | "points" | "tasks" | "reasons" | "options"
            )
            && matches!(
                word_key(words[words.len() - 2]).as_str(),
                "these" | "following"
            ))
}

fn has_list_lead_in(words: &[&str]) -> bool {
    let mut end = words.len();
    if end >= 2 && word_key(words[end - 2]) == "that" && word_key(words[end - 1]) == "is" {
        end -= 2;
    }
    end >= 2
        && matches!(
            word_key(words[end - 1]).as_str(),
            "things" | "items" | "steps" | "points" | "tasks" | "reasons" | "options"
        )
        && marker_num(&word_key(words[end - 2])).is_some()
        && !is_ordinal(&word_key(words[end - 2]))
}

/// `true` when a token sequence contains ". "/"? "/"! " — a full stop followed
/// by more words — i.e. the item holds more than one sentence.
fn has_sentence_boundary(words: &[&str]) -> bool {
    sentence_boundary(words).is_some()
}

/// Index just past the first token ending in `.`, `?` or `!` that is followed
/// by more words.
fn sentence_boundary(words: &[&str]) -> Option<usize> {
    words[..words.len().saturating_sub(1)]
        .iter()
        .position(|w| matches!(w.chars().last(), Some('.') | Some('?') | Some('!')))
        .map(|i| i + 1)
}

/// Identifiers and paths (`src/api.ts`, `getUserById`, `BASE_URL`) keep the
/// casing they were dictated with.
fn looks_technical(word: &str) -> bool {
    matches!(
        word,
        "cd" | "pwd"
            | "ls"
            | "git"
            | "npm"
            | "pnpm"
            | "yarn"
            | "cargo"
            | "python"
            | "python3"
            | "node"
            | "rustc"
            | "rg"
    ) || word
        .chars()
        .any(|c| matches!(c, '/' | '.' | '_' | '-' | '\\' | '(') || c.is_ascii_digit())
        || word.chars().skip(1).any(|c| c.is_uppercase())
}

fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_mode_builds_a_list() {
        let out = apply_normalizers("1. apples, 2. bananas", "notes");
        assert_eq!(out, "• Apples\n• Bananas");
    }

    #[test]
    fn email_mode_builds_a_list() {
        let out = apply_normalizers("1. apples, 2. bananas", "email");
        assert_eq!(out, "• Apples\n• Bananas");
    }

    #[test]
    fn first_second_list() {
        let out = apply_normalizers("first apples, second bananas", "notes");
        assert_eq!(out, "• Apples\n• Bananas");
    }

    #[test]
    fn lead_in_gets_a_colon_and_a_blank_line() {
        let out = apply_normalizers(
            "These are the four things I want: 1, look at this, and 2, fix the login, 3, add tests",
            "notes",
        );
        assert_eq!(
            out,
            "These are the four things I want:\n\n• Look at this\n• Fix the login\n• Add tests"
        );
    }

    #[test]
    fn ordinal_markers_need_no_punctuation() {
        let out = apply_normalizers("first update the readme, second fix the bug", "notes");
        assert_eq!(out, "• Update the readme\n• Fix the bug");
    }

    #[test]
    fn explicit_numbered_list_keeps_number_prefixes() {
        let out = apply_normalizers("here's a numbered list: 1. apples, 2. bananas", "notes");
        assert_eq!(out, "here's a numbered list:\n\n1. Apples\n2. Bananas");
    }

    #[test]
    fn negated_numbered_list_instruction_does_not_request_number_markers() {
        assert_eq!(
            apply_normalizers(
                "Do not use a numbered list: first apples second bananas",
                "notes"
            ),
            "Do not use a numbered list:\n\n• Apples\n• Bananas"
        );
    }

    #[test]
    fn command_arguments_survive_spoken_list_formatting() {
        assert_eq!(
            apply_normalizers(
                "first run git add . second run git status",
                "developer_prompt"
            ),
            "• Run git add .\n• Run git status"
        );
        assert_eq!(
            apply_normalizers("first cd .. second pwd", "developer_prompt"),
            "• cd ..\n• pwd"
        );
    }

    #[test]
    fn prose_lists_keep_single_question_and_exclamation_marks() {
        for terminal in ['?', '!'] {
            assert_eq!(
                apply_normalizers(&format!("first apples second bananas{terminal}"), "normal"),
                format!("apples and bananas{terminal}")
            );
            assert_eq!(
                apply_normalizers(
                    &format!("first apples second bananas{terminal} Let me know."),
                    "normal"
                ),
                format!("apples and bananas{terminal} Let me know.")
            );
        }
    }

    #[test]
    fn cardinals_without_markers_stay_prose() {
        let out = apply_normalizers("I have one cat and two dogs", "notes");
        assert_eq!(out, "I have one cat and two dogs");
    }

    #[test]
    fn trailing_sentence_after_the_last_item_becomes_a_paragraph() {
        let out = apply_normalizers(
            "I need three things: 1, update the readme, 2, fix login, 3, add tests. Let me know when it's done.",
            "notes",
        );
        assert_eq!(
            out,
            "I need three things:\n\n• Update the readme\n• Fix login\n• Add tests\n\nLet me know when it's done."
        );
    }

    #[test]
    fn developer_prompt_mode_builds_a_list() {
        let out = apply_normalizers("step 1 run the tests, step 2 deploy", "developer_prompt");
        assert_eq!(out, "• Run the tests\n• Deploy");
    }

    #[test]
    fn non_sequential_markers_are_left_alone() {
        let out = apply_normalizers("1. apples, 3. bananas", "notes");
        assert_eq!(out, "1. apples, 3. bananas");
    }

    #[test]
    fn normal_mode_leaves_prose() {
        let out = apply_normalizers("1. apples, 2. bananas", "normal");
        assert_eq!(out, "1. apples, 2. bananas");
    }

    #[test]
    fn does_not_rewrite_one_of_two() {
        let out = apply_normalizers("one of the two options", "notes");
        assert_eq!(out, "one of the two options");
    }

    #[test]
    fn mixed_spoken_inventory_becomes_a_numbered_list() {
        assert_eq!(
            apply_normalizers(
                "I want you to bring me three things that is one the mobile, second the power bank, third the earbuds.",
                "normal",
            ),
            "I want you to bring me three things:\n\n1. The mobile\n2. The power bank\n3. The earbuds"
        );
    }

    #[test]
    fn mixed_spoken_enumeration_becomes_bullets_in_writing_modes() {
        for mode in ["notes", "email", "developer_prompt"] {
            assert_eq!(
                apply_normalizers(
                    "I need three things that is one the mobile, second the power bank, third the earbuds.",
                    mode,
                ),
                "I need three things:\n\n• The mobile\n• The power bank\n• The earbuds",
                "{mode}",
            );
        }
    }

    #[test]
    fn unpunctuated_ordinals_keep_item_quantities() {
        assert_eq!(
            apply_normalizers(
                "first two chargers second three cables third one adapter",
                "notes"
            ),
            "• Two chargers\n• Three cables\n• One adapter"
        );
        assert_eq!(
            apply_normalizers(
                "I need three things first two chargers second three cables third one adapter",
                "normal"
            ),
            "I need three things:\n\n1. Two chargers\n2. Three cables\n3. One adapter"
        );
    }

    #[test]
    fn enumeration_preserves_prose_counts_quotes_and_paragraphs() {
        for mode in [
            "normal",
            "notes",
            "email",
            "developer_prompt",
            "coding",
            "unknown",
        ] {
            for text in [
                "I have one cat and two dogs",
                "My first car was red and my second car was blue",
                "First I need the second version",
                "First use the second option",
                "First class is expensive, second class is cheaper",
                "first-class tickets and second-class tickets",
                "First aid and second aid",
                "Say \"first\" apples and \"second\" bananas",
                "first apples\n\nsecond bananas",
                "first apples third bananas second pears",
            ] {
                assert_eq!(apply_normalizers(text, mode), text, "{mode}: {text}");
            }
        }
        assert_eq!(
            apply_normalizers(
                "first getUserById second src/api.ts third BASE_URL",
                "developer_prompt"
            ),
            "• getUserById\n• src/api.ts\n• BASE_URL"
        );
        assert_eq!(
            apply_normalizers(
                "First use the \"second\" label second use the \"first\" label",
                "notes"
            ),
            "• Use the \"second\" label\n• Use the \"first\" label"
        );
    }

    #[test]
    fn shared_list_content_keeps_quantities_and_the_counted_lead_in() {
        assert_eq!(
            spoken_list_content(
                "I need three things first two chargers second three cables third one adapter"
            ),
            Some("I need three things:\n\n• Two chargers\n• Three cables\n• One adapter".into())
        );
        assert!(spoken_list_content("I want one of the two options").is_none());
    }

    #[test]
    fn counted_inventory_uses_numbered_items_without_inventing_the_stated_count() {
        let expected = "I want you to bring me these four things:\n\n1. My mobile\n2. My earbuds\n3. My power bank";
        for input in [
            "I want you to bring me these four things: my mobile, my earbuds, and my power bank.",
            "I want you to bring me these four things first my mobile second my earbuds third my power bank.",
            "I want you to bring me these four things that is one my mobile, second my earbuds, third my power bank.",
        ] {
            assert_eq!(apply_normalizers(input, "normal"), expected, "{input}");
        }
    }

    #[test]
    fn counted_comma_inventory_keeps_complete_multiword_items_and_quantities() {
        assert_eq!(
            apply_normalizers("Please bring these three items: two phones, three power banks, and four pairs of earbuds.", "normal"),
            "Please bring these three items:\n\n1. Two phones\n2. Three power banks\n3. Four pairs of earbuds"
        );
        for mode in ["notes", "email", "developer_prompt"] {
            assert_eq!(
                apply_normalizers(
                    "I need these three things: my mobile, my earbuds, and my power bank.",
                    mode
                ),
                "I need these three things:\n\n• My mobile\n• My earbuds\n• My power bank",
                "{mode}"
            );
        }
    }

    #[test]
    fn inventory_does_not_split_quoted_or_parenthesized_commas() {
        assert_eq!(
            apply_normalizers(
                "I need three items: the label \"mobile, earbuds\", my cable, and my charger.",
                "normal"
            ),
            "I need three items:\n\n1. The label \"mobile, earbuds\"\n2. My cable\n3. My charger"
        );
        assert_eq!(
            apply_normalizers(
                "I need three items: getUserById(user, role), src/api.ts, and BASE_URL.",
                "developer_prompt"
            ),
            "I need three items:\n\n• getUserById(user, role)\n• src/api.ts\n• BASE_URL"
        );
    }

    #[test]
    fn ambiguous_commas_and_connected_quantity_prose_keep_their_shape() {
        for text in [
            "Please bring my mobile, my earbuds, and my power bank.",
            "I have four things to do, and three days available.",
            "We discussed the design, the timeline, and the budget.",
            "I need three things: polish the draft and review the outline.",
        ] {
            assert_eq!(apply_normalizers(text, "normal"), text);
        }
    }

    #[test]
    fn shared_inventory_items_preserve_complete_words_and_order() {
        for input in [
            "I need these four things: my mobile, my earbuds, and my power bank.",
            "I need these four things that is my mobile, my earbuds, and my power bank.",
            "I need these four things first my mobile second my earbuds third my power bank.",
            "I need these four things:\n\n1. My mobile\n2. My earbuds\n3. My power bank",
            "I need these four things:\n\n• My mobile\n• My earbuds\n• My power bank",
        ] {
            assert_eq!(
                enumeration_items(input),
                Some(vec![
                    "My mobile".into(),
                    "My earbuds".into(),
                    "My power bank".into()
                ])
            );
        }
        assert!(enumeration_items("My mobile, my earbuds, and my power bank.").is_none());
    }

    #[test]
    fn inventory_numbering_obeys_explicit_and_negated_instructions() {
        assert_eq!(
            apply_normalizers(
                "Use a numbered list for these three items: my phone, my charger, and my cable.",
                "notes"
            ),
            "Use a numbered list for these three items:\n\n1. My phone\n2. My charger\n3. My cable"
        );
        assert_eq!(
            apply_normalizers("Do not use a numbered list for these three items: my phone, my charger, and my cable.", "normal"),
            "Do not use a numbered list for these three items:\n\n• My phone\n• My charger\n• My cable"
        );
    }

    #[test]
    fn explicit_list_layout_overrides_mode_defaults() {
        for mode in ["normal", "notes", "email"] {
            for cue in ["bullet points", "bulleted list", "bullet list"] {
                let input = format!("Use {cue} for these two items: apples, and bananas.");
                assert_eq!(
                    apply_normalizers(&input, mode),
                    format!("Use {cue} for these two items:\n\n• Apples\n• Bananas"),
                    "{mode}: {cue}"
                );
                assert_eq!(
                    apply_normalizers(&format!("Use {cue}: first apples second bananas."), mode),
                    format!("Use {cue}:\n\n• Apples\n• Bananas")
                );
            }
            for cue in ["numbered points", "numbered list", "numbered steps"] {
                let input = format!("Use {cue} for these two items: apples, and bananas.");
                assert_eq!(
                    apply_normalizers(&input, mode),
                    format!("Use {cue} for these two items:\n\n1. Apples\n2. Bananas"),
                    "{mode}: {cue}"
                );
                assert_eq!(
                    apply_normalizers(&format!("Use {cue}: first apples second bananas."), mode),
                    format!("Use {cue}:\n\n1. Apples\n2. Bananas")
                );
            }
        }
    }

    #[test]
    fn list_layout_negation_belongs_to_its_directive() {
        for mode in ["normal", "notes", "email"] {
            for lead in [
                "Do not use numbered points for these two items",
                "Use bullet points, not a numbered list, for these two items",
                "Do not use a numbered list; use bullet points for these two items",
            ] {
                assert_eq!(
                    apply_normalizers(&format!("{lead}: apples, and bananas."), mode),
                    format!("{lead}:\n\n• Apples\n• Bananas"),
                    "{mode}: {lead}"
                );
            }
            for lead in [
                "Do not use bullet points for these two items",
                "Use numbered steps, not bullet points, for these two items",
                "Do not use bullet points; use numbered points for these two items",
                "Do not forget the folder; use numbered steps for these two items",
            ] {
                assert_eq!(
                    apply_normalizers(&format!("{lead}: apples, and bananas."), mode),
                    format!("{lead}:\n\n1. Apples\n2. Bananas"),
                    "{mode}: {lead}"
                );
            }
        }
    }

    #[test]
    fn quoted_layout_words_do_not_override_mode_defaults() {
        for lead in [
            "Title this \"numbered list\" for these two items",
            "Title this 'numbered points' for these two items",
            "Title this “numbered steps” for these two items",
        ] {
            assert_eq!(
                apply_normalizers(&format!("{lead}: apples, and bananas."), "notes"),
                format!("{lead}:\n\n• Apples\n• Bananas")
            );
        }
        for lead in [
            "Title this \"bullet points\" for these two items",
            "Title this 'bullet list' for these two items",
            "Title this “bulleted list” for these two items",
        ] {
            assert_eq!(
                apply_normalizers(&format!("{lead}: apples, and bananas."), "normal"),
                format!("{lead}:\n\n1. Apples\n2. Bananas")
            );
        }
    }

    #[test]
    fn conflicting_or_fully_negated_layout_instructions_stay_intact() {
        for mode in ["normal", "notes", "email"] {
            for input in [
                "Do not use bullet points or a numbered list for these two items: apples, and bananas.",
                "Do not use numbered steps or a bulleted list for these two items: apples, and bananas.",
                "Use bullet points and numbered points for these two items: apples, and bananas.",
                "Use bullet points; do not use bullet points for these two items: apples, and bananas.",
            ] {
                assert_eq!(apply_normalizers(input, mode), input, "{mode}: {input}");
            }
            let lead = "Do not use bullet points and use numbered points for these two items";
            assert_eq!(
                apply_normalizers(&format!("{lead}: apples, and bananas."), mode),
                format!("{lead}:\n\n1. Apples\n2. Bananas")
            );
        }
    }

    #[test]
    fn inventory_keeps_a_repeated_trailing_sentence_outside_the_items() {
        assert_eq!(
            apply_normalizers(
                "I need three items: my mobile, my earbuds, and my charger. my charger.",
                "normal"
            ),
            "I need three items:\n\n1. My mobile\n2. My earbuds\n3. My charger\n\nMy charger."
        );
        for text in [
            "I need three items: my mobile, which is broken, and my charger.",
            "I need three items: my mobile, my earbuds, and my charger. Let me know, thanks.",
            "I need three items: getUserById(user, role], src/api.ts, and BASE_URL.",
        ] {
            assert_eq!(apply_normalizers(text, "normal"), text);
        }
    }

    #[test]
    fn inventory_preserves_commas_inside_numeric_quantities() {
        assert_eq!(
            apply_normalizers(
                "I need three items: a 1,000 mAh power bank, two cables, and three adapters.",
                "normal"
            ),
            "I need three items:\n\n1. A 1,000 mAh power bank\n2. Two cables\n3. Three adapters"
        );
    }

    #[test]
    fn repeated_ordinal_groups_preserve_each_scope_and_item() {
        let input = "But first of all from the bakery shop you will bring me first the chocolate cake, second the strawberry cake, and from the electronics shop you will be bringing me first one charger, one power bank, one new smartphone and laptop.";
        assert_eq!(
            apply_normalizers(input, "normal"),
            "But first of all from the bakery shop you will bring me:\n\n1. The chocolate cake\n2. The strawberry cake\n\nAnd from the electronics shop you will be bringing me:\n\n1. One charger\n2. One power bank\n3. One new smartphone and laptop"
        );
    }

    #[test]
    fn repeated_ordinal_groups_work_with_unrelated_scopes_and_objects() {
        let input = "For the design team bring first the printed map, second two markers, and for the support team bring first three notebooks, four pens, and the radio.";
        assert_eq!(
            apply_normalizers(input, "normal"),
            "For the design team bring:\n\n1. The printed map\n2. Two markers\n\nAnd for the support team bring:\n\n1. Three notebooks\n2. Four pens\n3. The radio"
        );
        assert_eq!(
            apply_normalizers(input, "notes"),
            "For the design team bring:\n\n• The printed map\n• Two markers\n\nAnd for the support team bring:\n\n• Three notebooks\n• Four pens\n• The radio"
        );
    }

    #[test]
    fn ambiguous_resets_and_ordinary_ordinals_keep_their_words() {
        for text in [
            "first apples second pears first oranges",
            "First I need the second version, and at the first desk I will ask.",
            "First class is expensive, second class is cheaper, and for the first class ticket ask Sarah.",
            "First use the \"second first\" label second use another label, and from here first consider the answer.",
        ] {
            assert_eq!(apply_normalizers(text, "normal"), text, "{text}");
        }
    }

    #[test]
    fn shared_enumeration_groups_preserve_headers_and_assignment() {
        let input = "For the design team bring first the printed map, second two markers, and for the support team bring first three notebooks, four pens, and the radio.";
        let expected = vec![
            EnumerationGroup {
                header: "For the design team bring:".into(),
                items: vec!["The printed map".into(), "Two markers".into()],
                tail: String::new(),
            },
            EnumerationGroup {
                header: "And for the support team bring:".into(),
                items: vec![
                    "Three notebooks".into(),
                    "Four pens".into(),
                    "The radio".into(),
                ],
                tail: String::new(),
            },
        ];
        assert_eq!(enumeration_groups(input), Some(expected.clone()));
        assert_eq!(
            enumeration_groups(&apply_normalizers(input, "normal")),
            Some(expected)
        );
        assert_eq!(enumeration_items(input).unwrap().len(), 5);
    }

    #[test]
    fn shared_enumeration_groups_preserve_each_tail() {
        let input = "For the first team:\n\n1. The map\n2. Two markers\n\nCall Sarah.\n\nFor the other team:\n\n1. Three notebooks\n2. The radio\n\nEmail Pat.";
        let groups = enumeration_groups(input).expect("two explicit groups");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].header, "For the first team:");
        assert_eq!(groups[0].tail, "Call Sarah.");
        assert_eq!(groups[1].header, "For the other team:");
        assert_eq!(groups[1].tail, "Email Pat.");
        assert!(enumeration_groups("First I need the second version").is_none());
        assert!(enumeration_groups("• The map\nCall Sarah.\n• Two markers").is_none());
    }

    #[test]
    fn grouped_lists_ignore_connectors_and_ordinals_inside_labels() {
        let input = "For the team bring first the folder, second the label \"x, and from y\", and for the other team bring first the map, the pen, and the ruler.";
        assert_eq!(
            apply_normalizers(input, "normal"),
            "For the team bring:\n\n1. The folder\n2. The label \"x, and from y\"\n\nAnd for the other team bring:\n\n1. The map\n2. The pen\n3. The ruler"
        );
        assert_eq!(
            apply_normalizers(
                "For the code run first choose(first, second), second src/api.ts, and for the tools run first npm test, cargo check, and git status.",
                "developer_prompt"
            ),
            "For the code run:\n\n• choose(first, second)\n• src/api.ts\n\nAnd for the tools run:\n\n• npm test\n• cargo check\n• git status"
        );
    }

    #[test]
    fn grouped_lists_handle_more_than_two_scopes_and_each_tail() {
        assert_eq!(
            apply_normalizers(
                "For A bring first two folders, second the map. Call Sarah. Then for B bring first three pens, second the notebook, and for C bring first one ruler, one eraser, and the tape.",
                "normal"
            ),
            "For A bring:\n\n1. Two folders\n2. The map\n\nCall Sarah.\n\nThen for B bring:\n\n1. Three pens\n2. The notebook\n\nAnd for C bring:\n\n1. One ruler\n2. One eraser\n3. The tape"
        );
    }

    #[test]
    fn nonsequential_groups_and_unbalanced_labels_are_preserved() {
        for input in [
            "For A bring first the folder, third the map, and for B bring first one pen, one notebook.",
            "For A bring first the folder, second the label \"x, and for B bring first one pen, one notebook.",
            "first choose(first, second] second print the result",
        ] {
            assert_eq!(apply_normalizers(input, "normal"), input, "{input}");
        }
        assert_eq!(
            apply_normalizers("1) apples, 2) bananas", "notes"),
            "• Apples\n• Bananas"
        );
    }
}

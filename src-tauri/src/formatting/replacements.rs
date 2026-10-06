use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplacementRule {
    pub id: String,
    pub before: String,
    pub after: String,
    pub enabled: bool,
}

pub struct CustomReplacements {
    rules: Vec<ReplacementRule>,
}

impl CustomReplacements {
    pub fn new(rules: Vec<ReplacementRule>) -> Self {
        Self { rules }
    }

    pub fn default_rules() -> Vec<ReplacementRule> {
        vec![
            ReplacementRule {
                id: "1".into(),
                before: "git hub".into(),
                after: "GitHub".into(),
                enabled: true,
            },
            ReplacementRule {
                id: "2".into(),
                before: "vs code".into(),
                after: "VS Code".into(),
                enabled: true,
            },
            ReplacementRule {
                id: "3".into(),
                before: "type script".into(),
                after: "TypeScript".into(),
                enabled: true,
            },
            ReplacementRule {
                id: "4".into(),
                before: "tauri".into(),
                after: "Tauri".into(),
                enabled: true,
            },
            ReplacementRule {
                id: "5".into(),
                before: "qwen".into(),
                after: "Qwen".into(),
                enabled: true,
            },
            ReplacementRule {
                id: "6".into(),
                before: "postgres ql".into(),
                after: "PostgreSQL".into(),
                enabled: true,
            },
            ReplacementRule {
                id: "7".into(),
                before: "supabase".into(),
                after: "Supabase".into(),
                enabled: true,
            },
            ReplacementRule {
                id: "8".into(),
                before: "lang graph".into(),
                after: "LangGraph".into(),
                enabled: true,
            },
        ]
    }

    pub fn apply(&self, text: &str) -> String {
        let rules = self
            .rules
            .iter()
            .filter(|r| r.enabled)
            .map(|r| (r.before.clone(), r.after.clone()))
            .collect::<Vec<_>>();
        apply_literal_rules(text, &rules)
    }

    /// Restore configured output casing after grammar editing without applying
    /// aliases twice (a second pass could turn one mapping into another).
    pub(crate) fn restore_spellings(&self, text: &str) -> String {
        let rules = self
            .rules
            .iter()
            .filter(|rule| rule.enabled)
            .map(|rule| (rule.after.clone(), rule.after.clone()))
            .collect::<Vec<_>>();
        apply_literal_rules(text, &rules)
    }
}

/// Standard spellings for technical terms ASR routinely mishears or
/// lowercases. Applied only in `developer_prompt` mode, where the listener is
/// a coding tool and the exact spelling is the point.
const TECHNICAL_TERMS: &[(&str, &str)] = &[
    ("java script", "JavaScript"),
    ("javascript", "JavaScript"),
    ("type script", "TypeScript"),
    ("typescript", "TypeScript"),
    ("node js", "Node.js"),
    ("nodejs", "Node.js"),
    ("next js", "Next.js"),
    ("nextjs", "Next.js"),
    ("vue js", "Vue.js"),
    ("git hub", "GitHub"),
    ("github", "GitHub"),
    ("git lab", "GitLab"),
    ("gitlab", "GitLab"),
    ("postgre sql", "PostgreSQL"),
    ("postgres ql", "PostgreSQL"),
    ("postgresql", "PostgreSQL"),
    ("my sql", "MySQL"),
    ("mysql", "MySQL"),
    ("mongo db", "MongoDB"),
    ("mongodb", "MongoDB"),
    ("graph ql", "GraphQL"),
    ("graphql", "GraphQL"),
    ("fast api", "FastAPI"),
    ("fastapi", "FastAPI"),
    ("py torch", "PyTorch"),
    ("pytorch", "PyTorch"),
    ("dev ops", "DevOps"),
    ("devops", "DevOps"),
    ("ci cd", "CI/CD"),
    ("vs code", "VS Code"),
    ("vscode", "VS Code"),
    ("use effect", "useEffect"),
    ("use state", "useState"),
    ("local host", "localhost"),
    ("read me", "README"),
    ("readme", "README"),
    ("apis", "APIs"),
    ("api", "API"),
    ("json", "JSON"),
    ("yaml", "YAML"),
    ("html", "HTML"),
    ("css", "CSS"),
    ("sql", "SQL"),
    ("urls", "URLs"),
    ("url", "URL"),
    ("ui", "UI"),
    ("ux", "UX"),
    ("cli", "CLI"),
    ("sdk", "SDK"),
    ("jwt", "JWT"),
    ("oauth", "OAuth"),
    ("https", "HTTPS"),
    ("http", "HTTP"),
    ("prs", "PRs"),
    ("pr", "PR"),
    ("kubernetes", "Kubernetes"),
];

/// Characters that make a neighbouring "term" part of an identifier or path
/// instead: `my_api`, `src/api.ts` and `web-ui` must survive untouched. `\b`
/// alone cannot see this because it treats `_` as a word character and `/`
/// as a boundary.
fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '\\' | '-' | '@')
}

pub(crate) fn is_embedded_identifier(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start]
        .chars()
        .next_back()
        .is_some_and(is_identifier_char);
    let mut after = text[end..].chars();
    let next = after.next();
    let after = next.is_some_and(|ch| {
        if ch == '.' {
            // A sentence's final full stop is punctuation; a dotted suffix is
            // part of a path, address, domain or identifier.
            after.next().is_some_and(is_identifier_char)
        } else {
            is_identifier_char(ch)
        }
    });
    before || after
}

/// Rewrites misheard technical terms to their canonical spelling, in one
/// pass, longest term first. A match is only replaced when it is not glued to
/// identifier characters on either side.
pub fn apply_technical_terms(text: &str) -> String {
    let mut terms: Vec<&str> = TECHNICAL_TERMS.iter().map(|(before, _)| *before).collect();
    terms.sort_by_key(|before| std::cmp::Reverse(before.len()));
    let pattern = terms
        .iter()
        .map(|before| regex::escape(before))
        .collect::<Vec<_>>()
        .join("|");
    let Ok(re) = Regex::new(&format!(r"(?i)(?:{pattern})")) else {
        return text.into();
    };
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for found in re.find_iter(text) {
        if is_embedded_identifier(text, found.start(), found.end()) {
            continue;
        }
        let replacement = TECHNICAL_TERMS
            .iter()
            .find(|(before, _)| before.eq_ignore_ascii_case(found.as_str()))
            .map(|(_, after)| *after)
            .unwrap_or(found.as_str());
        out.push_str(&text[last..found.start()]);
        out.push_str(replacement);
        last = found.end();
    }
    out.push_str(&text[last..]);
    out
}

/// One pass prevents cascades; longest matching phrases take priority. A closure
/// keeps user text literal (including `$1`), rather than regex replacement syntax.
pub fn apply_literal_rules(text: &str, rules: &[(String, String)]) -> String {
    let mut rules: Vec<_> = rules
        .iter()
        .filter(|(a, b)| !a.trim().is_empty() && !b.trim().is_empty())
        .collect();
    rules.sort_by_key(|(a, _)| std::cmp::Reverse(a.len()));
    let pattern = rules
        .iter()
        .map(|(a, _)| regex::escape(a.trim()))
        .collect::<Vec<_>>()
        .join("|");
    if pattern.is_empty() {
        return text.into();
    }
    let Ok(re) = Regex::new(&format!(r"(?i)\b(?:{pattern})\b")) else {
        return text.into();
    };
    re.replace_all(text, |captures: &regex::Captures<'_>| {
        let found = captures.get(0).unwrap();
        if is_embedded_identifier(text, found.start(), found.end()) {
            return found.as_str().to_owned();
        }
        let tail = &text[found.end()..];
        if found.as_str().to_lowercase().ends_with('n')
            && ["'t", "’t"]
                .iter()
                .any(|s| tail.to_lowercase().starts_with(s))
        {
            return found.as_str().to_owned();
        }
        rules
            .iter()
            .find(|(a, _)| a.trim().to_lowercase() == found.as_str().to_lowercase())
            .map(|(_, b)| b.trim().to_owned())
            .unwrap_or_else(|| found.as_str().to_owned())
    })
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_replacements() {
        let manager = CustomReplacements::new(CustomReplacements::default_rules());
        let input = "I pushed the commit to git hub using vs code and type script.";
        let result = manager.apply(input);
        assert_eq!(
            result,
            "I pushed the commit to GitHub using VS Code and TypeScript."
        );
    }

    #[test]
    fn technical_terms_fix_mishearings() {
        assert_eq!(
            apply_technical_terms("update the api in src/api.ts using type script"),
            "update the API in src/api.ts using TypeScript"
        );
        assert_eq!(
            apply_technical_terms("deploy to git hub with ci cd and docker on kubernetes"),
            "deploy to GitHub with CI/CD and docker on Kubernetes"
        );
    }

    #[test]
    fn technical_terms_never_touch_identifiers_or_paths() {
        for text in ["my_api", "src/api.ts", "web-ui", "API-V2", "configs\\api"] {
            assert_eq!(apply_technical_terms(text), text, "{text:?}");
        }
    }
}

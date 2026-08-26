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
        let mut out = text.to_string();

        for rule in &self.rules {
            if !rule.enabled || rule.before.trim().is_empty() {
                continue;
            }

            // Word-boundary case-insensitive replacement
            let pattern = format!(r"(?i)\b{}\b", regex::escape(&rule.before));
            if let Ok(re) = Regex::new(&pattern) {
                out = re.replace_all(&out, rule.after.as_str()).to_string();
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_replacements() {
        let manager = CustomReplacements::new(CustomReplacements::default_rules());
        let input = "I pushed the commit to git hub using vs code and type script.";
        let result = manager.apply(input);
        assert_eq!(result, "I pushed the commit to GitHub using VS Code and TypeScript.");
    }
}

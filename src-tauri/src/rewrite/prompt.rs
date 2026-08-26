use super::client::RewriteRequest;

const BASE_SYSTEM: &str = "You are Reflow's on-device real-time dictation cleaner. \
Your task: Transform raw spoken speech into clean, polished, accurate written text. \
Strict Rules: \
Fix grammar, punctuation, and capitalization. \
Remove filler words (um, uh, ah, er, you know, like, I mean). \
Resolve self-repairs and speech restarts: when the speaker corrects themselves \
(e.g., 'meet at 2, wait 3 PM'), output ONLY the final corrected intention ('meet at 3:00 PM'). \
Preserve the speaker's exact meaning, technical terms, names, and numbers. \
NEVER add facts, answer questions, or output meta-commentary \
(do NOT say 'Here is the cleaned text:'). \
NEVER wrap the output in markdown code blocks or quotes. \
Output ONLY the final cleaned text.";

pub fn build_prompts(req: &RewriteRequest) -> (String, String) {
    let style = req.style.trim().to_ascii_lowercase();
    let mut system = String::from(BASE_SYSTEM);

    if style == "decisive" {
        system.push_str(
            " Use a decisive tone: eliminate hedges ('I think', 'maybe', 'sort of').",
        );
    } else if style == "email" || style == "professional" {
        system.push_str(" Use a clear, professional email register.");
    } else if style == "chat" || style == "casual" {
        system.push_str(" Use a natural, conversational chat style.");
    }

    let mut user = String::from("/no_think\n");
    if !req.app_process.trim().is_empty() {
        user.push_str("Application: ");
        user.push_str(req.app_process.trim());
        user.push('\n');
    }
    if !req.vocabulary.is_empty() {
        user.push_str("Vocabulary: ");
        user.push_str(&req.vocabulary.join(", "));
        user.push('\n');
    }
    user.push_str("Transcript:\n");
    user.push_str(req.text.trim());

    (system, user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rewrite::RewriteRequest;

    fn req(level: &str, style: &str) -> RewriteRequest {
        RewriteRequest {
            text: "hello world".into(),
            cleanup_level: level.into(),
            style: style.into(),
            dictation_mode: "normal".into(),
            vocabulary: vec!["Tauri".into()],
            app_process: "Code.exe".into(),
            model_id: "lfm2.5-1.2b".into(),
        }
    }

    #[test]
    fn user_message_includes_no_think_vocab_and_app() {
        let (system, user) = build_prompts(&req("medium", "neutral"));
        assert!(system.contains("on-device real-time dictation cleaner"));
        assert!(user.contains("/no_think"));
        assert!(user.contains("Tauri"));
        assert!(user.contains("Code.exe"));
        assert!(user.contains("hello world"));
    }

    #[test]
    fn decisive_prompt_asks_for_stronger_cleanup() {
        let (system, _) = build_prompts(&req("high", "decisive"));
        assert!(system.to_lowercase().contains("decisive"));
    }

    #[test]
    fn email_prompt_adds_email_register() {
        let (system, _) = build_prompts(&req("medium", "email"));
        assert!(system.to_lowercase().contains("email register"));
    }
}

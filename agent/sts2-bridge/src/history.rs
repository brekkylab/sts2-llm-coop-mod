//! History manipulation as pure functions, testable without calling a model.

use ailoy::message::{Message, Part, Role};

pub fn text_of(m: &Message) -> String {
    m.contents
        .iter()
        .filter_map(|p| match p {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

pub fn user(text: impl Into<String>) -> Message {
    Message::new(Role::User).with_contents([Part::text(text.into())])
}

/// Replaces the last user message with one line, right after a decision. Only the
/// newest briefing stays in full, so an old hand can't lead to a card that's gone.
pub fn shrink_last_query(history: &mut [Message], line: &str) {
    if let Some(m) = history.iter_mut().rev().find(|m| m.role == Role::User) {
        m.contents = vec![Part::text(line)];
    }
}

/// ailoy skips the instruction if the history contains any system message, so a
/// recovered history must not keep one.
pub fn strip_system(history: &mut Vec<Message>) {
    history.retain(|m| m.role != Role::System);
}

/// Rebuilds history from `said/`, merging consecutive same-speaker lines (roles
/// must alternate). Drops a leading assistant message and the trailing user run;
/// the latter was never seen by the agent, and `DecisionLoop::begin_combat` makes
/// the first decision carry it.
pub fn from_said(said: &[(String, String)]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    for (who, text) in said {
        let role = if who == "human" { Role::User } else { Role::Assistant };
        let line = if role == Role::User {
            format!("동료: {text}")
        } else {
            text.clone()
        };

        match out.last_mut() {
            Some(m) if m.role == role => {
                let merged = format!("{}\n{line}", text_of(m));
                m.contents = vec![Part::text(merged)];
            }
            _ => out.push(Message::new(role).with_contents([Part::text(line)])),
        }
    }

    if out.first().is_some_and(|m| m.role == Role::Assistant) {
        out.remove(0);
    }
    if out.last().is_some_and(|m| m.role == Role::User) {
        out.pop();
    }
    out
}

/// The partner's words for the query body. Put in the query rather than pushed as
/// a separate user message, which would make two consecutive user messages (ailoy
/// doesn't merge them) and leave a pair in the history every time the partner
/// spoke. Partner lines only, so no speaker labels.
pub fn join_said(said: &[(String, String)]) -> String {
    said.iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_for_log(history: &[Message], query: &Message) -> String {
    let mut out = String::new();
    for m in history.iter().chain(std::iter::once(query)) {
        let head = match m.role {
            Role::System => "system",
            Role::Assistant => "assistant",
            _ => "user",
        };
        out.push_str(&format!("### {head}\n{}\n\n", text_of(m)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(text: &str) -> Message {
        Message::new(Role::Assistant).with_contents([Part::text(text)])
    }

    #[test]
    fn shrinks_only_the_last_query() {
        let mut h = vec![
            user("턴 1 — 적 슬라임 30"),
            assistant("CHOSEN: 1"),
            user("[턴 2 브리핑 아주 긴 글]"),
            assistant("CHOSEN: 2"),
        ];
        shrink_last_query(&mut h, "턴 2 — 적 슬라임 18");

        assert_eq!(text_of(&h[0]), "턴 1 — 적 슬라임 30");
        assert_eq!(text_of(&h[2]), "턴 2 — 적 슬라임 18");
        assert_eq!(text_of(&h[3]), "CHOSEN: 2");
    }

    #[test]
    fn shrinking_an_empty_history_is_a_no_op() {
        let mut h: Vec<Message> = vec![];
        shrink_last_query(&mut h, "턴 1");
        assert!(h.is_empty());
    }

    #[test]
    fn strips_the_system_message() {
        let mut h = vec![
            Message::new(Role::System).with_contents([Part::text("RULES")]),
            user("턴 1"),
        ];
        strip_system(&mut h);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].role, Role::User);
    }

    #[test]
    fn rebuilds_turns_from_said() {
        let said = vec![
            ("human".to_string(), "방어 챙겨".to_string()),
            ("agent".to_string(), "그럴게".to_string()),
        ];
        let h = from_said(&said);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].role, Role::User);
        assert!(text_of(&h[0]).starts_with("동료:"));
        assert_eq!(h[1].role, Role::Assistant);
    }

    #[test]
    fn merges_consecutive_speakers() {
        let said = vec![
            ("human".to_string(), "저거 먼저".to_string()),
            ("human".to_string(), "아니 방어부터".to_string()),
            ("agent".to_string(), "알겠어".to_string()),
        ];
        let h = from_said(&said);

        assert_eq!(h.len(), 2, "둘로 합쳐야 한다");
        assert!(text_of(&h[0]).contains("저거 먼저"));
        assert!(text_of(&h[0]).contains("아니 방어부터"));
        assert_eq!(h[1].role, Role::Assistant);
    }

    #[test]
    fn drops_a_leading_assistant() {
        let said = vec![
            ("agent".to_string(), "먼저 말했다".to_string()),
            ("human".to_string(), "응".to_string()),
            ("agent".to_string(), "알겠어".to_string()),
        ];
        let h = from_said(&said);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].role, Role::User);
        assert_eq!(h[1].role, Role::Assistant);
    }

    /// Both rules together leave nothing, which is right: the first decision
    /// carries that line.
    #[test]
    fn an_unanswered_opening_leaves_nothing() {
        let said = vec![
            ("agent".to_string(), "먼저 말했다".to_string()),
            ("human".to_string(), "응".to_string()),
        ];
        assert!(from_said(&said).is_empty());
    }

    #[test]
    fn drops_a_trailing_user() {
        let said = vec![
            ("human".to_string(), "방어 챙겨".to_string()),
            ("agent".to_string(), "그럴게".to_string()),
            ("human".to_string(), "아직 안 끝났어".to_string()),
        ];
        let h = from_said(&said);
        assert_eq!(h.len(), 2, "마지막 User 를 빼야 한다");
        assert_eq!(h[1].role, Role::Assistant);
    }

    #[test]
    fn joins_what_the_partner_said() {
        let said = [
            ("human".to_string(), "저거 먼저".to_string()),
            ("human".to_string(), "방어도 챙겨".to_string()),
        ];
        assert_eq!(join_said(&said), "저거 먼저\n방어도 챙겨");
        assert_eq!(join_said(&[]), "");
    }
}

use anyhow::{Context as _, Result};
use serde_json::json;

use crate::dto::{CombatState, CoopConfig, PendingDecision};

/// A rejected action (the board moved) is distinct from an unreachable server (no
/// game).
#[derive(Debug, thiserror::Error)]
pub enum SpireError {
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Unreachable(#[from] anyhow::Error),
}

pub struct Sts2Client {
    base: String,
    http: reqwest::Client,
}

impl Sts2Client {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }

    /// The mod's port by default; overridable to run two games side by side.
    pub fn from_env() -> Self {
        Self::new(
            &std::env::var("STS2_ADDR").unwrap_or_else(|_| "http://127.0.0.1:15527".into()),
        )
    }

    /// Defaults on 404 (an older mod without the endpoint).
    pub async fn config(&self) -> CoopConfig {
        let url = format!("{}/config", self.base);
        match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
            _ => CoopConfig::default(),
        }
    }

    /// Asks the mod to drop its held plan; carries no answer. The mod re-asks
    /// through the normal path. Returns whether anything was dropped.
    pub async fn replan(&self) -> Result<bool, SpireError> {
        let url = format!("{}/replan", self.base);
        let res: serde_json::Value = self
            .http
            .post(&url)
            .json(&json!({}))
            .send()
            .await
            .with_context(|| format!("POST {url}"))
            .map_err(SpireError::Unreachable)?
            .json()
            .await
            .context("replan reply did not parse")
            .map_err(SpireError::Unreachable)?;

        Ok(dropped_from(&res))
    }

    /// Best effort; a failure doesn't affect decisions.
    pub async fn push_status(&self, state: &str, bubble: Option<&str>) -> Result<(), SpireError> {
        let url = format!("{}/agent/status", self.base);
        self.http
            .post(&url)
            .json(&json!({"state": state, "bubble": bubble}))
            .send()
            .await
            .with_context(|| format!("POST {url}"))
            .map_err(SpireError::Unreachable)?;
        Ok(())
    }

    pub async fn combat_state(&self) -> Result<CombatState> {
        let url = format!("{}/state/combat", self.base);
        self.http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url} — 게임이 켜져 있고 모드가 올라와 있습니까"))?
            .json()
            .await
            .context("combat state did not parse")
    }

    /// Returns after the action has settled, so the next state read is current.
    pub async fn act(&self, action_id: &str, snapshot_id: &str) -> Result<(), SpireError> {
        let url = format!("{}/action", self.base);
        let res: serde_json::Value = self
            .http
            .post(&url)
            .json(&json!({"actionId": action_id, "snapshotId": snapshot_id}))
            .send()
            .await
            .with_context(|| format!("POST {url}"))
            .map_err(SpireError::Unreachable)?
            .json()
            .await
            .context("action response did not parse")
            .map_err(SpireError::Unreachable)?;

        if res.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            return Ok(());
        }
        Err(SpireError::Refused(
            res.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("action refused")
                .to_string(),
        ))
    }

    /// When nothing is waiting the reply is just `pending: false`.
    pub async fn pending_decision(&self) -> Result<PendingDecision> {
        let url = format!("{}/decision/pending", self.base);
        self.http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .json()
            .await
            .context("pending decision did not parse")
    }

    /// Answers the waiting question with the turn's plan (at least one action).
    /// The rejection reason is passed through, so "too late" (`stale requestId`,
    /// `nothing is waiting`) can be told from "illegal choice". `actionId` is also
    /// sent for older mods.
    pub async fn answer(
        &self,
        request_id: &str,
        action_ids: &[String],
        say: Option<&str>,
        act_now: bool,
    ) -> Result<(), SpireError> {
        let url = format!("{}/decision", self.base);
        let action_id = action_ids.first().map(String::as_str).unwrap_or("");
        let res: serde_json::Value = self
            .http
            .post(&url)
            .json(&json!({
                "requestId": request_id,
                "actionId": action_id,
                "actionIds": action_ids,
                "say": say,
                "now": act_now,
            }))
            .send()
            .await
            .with_context(|| format!("POST {url}"))
            .map_err(SpireError::Unreachable)?
            .json()
            .await
            .context("answer response did not parse")
            .map_err(SpireError::Unreachable)?;

        if res.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            return Ok(());
        }
        Err(SpireError::Refused(
            res.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("refused")
                .to_string(),
        ))
    }
}

fn dropped_from(v: &serde_json::Value) -> bool {
    v.get("dropped").and_then(|d| d.as_bool()).unwrap_or(false)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A real server response, so renamed fields fail here. Shared with render tests.
    pub(crate) const SAMPLE_JSON: &str = include_str!("sample_state.json");

    pub(crate) fn sample() -> CombatState {
        serde_json::from_str(SAMPLE_JSON).expect("표본이 파싱되지 않는다")
    }

    #[test]
    fn a_replan_reply_says_whether_it_dropped_anything() {
        assert!(dropped_from(&serde_json::json!({"ok": true, "dropped": true})));
        assert!(!dropped_from(&serde_json::json!({"ok": true, "dropped": false})));
        // Older mods omit `dropped`.
        assert!(!dropped_from(&serde_json::json!({"ok": true})));
    }

    #[test]
    fn parses_a_real_response() {
        let s = sample();
        assert!(s.in_combat);
        assert!(s.can_act);
        assert_eq!(s.enemies.len(), 1);
        assert_eq!(s.enemies[0].hp, 80);
        assert_eq!(s.hand.len(), 2);
        assert_eq!(s.hand[0].estimated_damage, 6);
        assert_eq!(s.me.as_ref().unwrap().energy, 3);
        assert_eq!(s.legal_actions.len(), 3);
    }

    #[test]
    fn finds_an_action_by_card_name() {
        let s = sample();
        let a = s.find_action("타격", Some("압축벌레")).expect("타격을 못 찾았다");
        assert_eq!(a.action_id, "play_card_combat_14_target_creature_2");
    }

    #[test]
    fn finds_a_targetless_action() {
        let s = sample();
        let a = s.find_action("수비", None).expect("수비를 못 찾았다");
        assert_eq!(a.action_id, "play_card_combat_15_target_none");
    }

    #[test]
    fn finds_end_turn() {
        assert!(sample().end_turn_action().is_some());
    }

    /// A real question; `snapshotId` must match /state/combat's character for
    /// character.
    #[test]
    fn parses_a_pending_decision() {
        let json = r#"{"pending":true,"requestId":"6d9d01d8",
                       "snapshotId":"play_card_combat_14_target_creature_2|end_turn_player_9",
                       "deadlineMs":14188}"#;
        let p: PendingDecision = serde_json::from_str(json).unwrap();
        assert!(p.pending);
        assert_eq!(p.request_id.as_deref(), Some("6d9d01d8"));
        assert_eq!(p.deadline_ms, 14188);
        assert!(p.snapshot_id.unwrap().contains("end_turn_player_9"));
    }

    #[test]
    fn parses_an_idle_decision() {
        let p: PendingDecision = serde_json::from_str(r#"{"pending":false}"#).unwrap();
        assert!(!p.pending);
        assert!(p.request_id.is_none());
        assert_eq!(p.deadline_ms, 0);
    }
}

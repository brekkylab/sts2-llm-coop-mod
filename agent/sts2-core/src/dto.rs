use std::collections::HashMap;

use serde::Deserialize;

/// Combat facts from the mod, no interpretation. `Default` is an out-of-combat
/// state for tests.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombatState {
    pub in_combat: bool,

    /// Echo back with an action; rejected if the legal actions changed since.
    #[serde(default)]
    pub snapshot_id: String,
    /// Tells a new run from a bridge restart. Empty if unreadable.
    #[serde(default)]
    pub run_seed: String,

    /// False after ending the turn, when `legal_actions` is empty by design.
    #[serde(default)]
    pub can_act: bool,

    #[serde(default)]
    pub turn: i32,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub me: Option<Actor>,
    #[serde(default)]
    pub ally: Option<Actor>,
    #[serde(default)]
    pub enemies: Vec<Enemy>,
    #[serde(default)]
    pub hand: Vec<Card>,
    #[serde(default)]
    pub draw_count: usize,
    #[serde(default)]
    pub discard_count: usize,
    #[serde(default)]
    pub legal_actions: Vec<LegalAction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegalAction {
    pub action_id: String,
    #[serde(default)]
    pub card_instance_id: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub energy_cost: i32,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    pub net_id: u64,
    pub hp: i32,
    pub max_hp: i32,
    pub block: i32,
    pub energy: i32,
    #[serde(default)]
    pub powers: HashMap<String, i32>,

    /// Informational; the mod decides when to play.
    #[serde(default)]
    pub ended_turn: bool,

    /// The agent holds a plan; avoids pointless `/replan` calls.
    #[serde(default)]
    pub holding: bool,

    #[serde(default)]
    pub relics: Vec<Relic>,

    /// Also in the action list, but only here with capacity.
    #[serde(default)]
    pub potions: Vec<Potion>,

    #[serde(default)]
    pub max_potion_count: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Intent {
    /// Text over the enemy's head; empty for icon-only intents such as debuffs.
    #[serde(default)]
    pub label: Option<String>,

    /// Attack, DebuffStrong, ...
    #[serde(default)]
    pub kind: Option<String>,

    /// Tooltip; the only content for intents without a label.
    #[serde(default)]
    pub tip: Option<String>,
}

impl Intent {
    /// Label, else tooltip, else kind.
    pub fn text(&self) -> String {
        match (&self.label, &self.tip, &self.kind) {
            (Some(l), _, _) if !l.is_empty() => l.clone(),
            (_, Some(t), _) if !t.is_empty() => t.clone(),
            (_, _, Some(k)) => k.clone(),
            _ => "-".into(),
        }
    }
}

/// Id only, e.g. `BOTTLED_LIGHTNING`; the game has no display name to send.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Relic {
    pub id: String,
}

/// `slot_index` matches the number in potion action ids.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Potion {
    pub id: String,
    #[serde(default)]
    pub slot_index: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Enemy {
    pub id: String,
    pub name: String,
    pub hp: i32,
    pub max_hp: i32,
    pub block: i32,

    /// All of them: attack and debuff can come together.
    #[serde(default)]
    pub intents: Vec<Intent>,

    /// Damage *if* it targets me. Targets are unknown; don't sum as incoming.
    #[serde(default)]
    pub damage_if_it_hits_me: i32,

    #[serde(default)]
    pub powers: HashMap<String, i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub instance_id: String,
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub cost: i32,
    pub estimated_damage: i32,
    pub estimated_block: i32,
    #[serde(default)]
    pub description: Option<String>,

    pub playable: bool,
    #[serde(default)]
    pub unplayable_reason: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
}

impl Enemy {
    /// Damage needed to kill it this turn.
    pub fn effective_hp(&self) -> i32 {
        self.hp + self.block
    }
}

impl CombatState {
    /// Maps a card name (and optional target name) to an `action_id`. With
    /// duplicates in hand, the first copy.
    pub fn find_action(&self, card: &str, target: Option<&str>) -> Option<&LegalAction> {
        let target_id = target.and_then(|t| {
            self.enemies
                .iter()
                .find(|e| e.name == t || e.id == t)
                .map(|e| e.id.as_str())
        });

        // No card match means failure, never "any action on that target".
        let instance_ids: Vec<&str> = self
            .hand
            .iter()
            .filter(|c| c.name == card || c.id == card)
            .map(|c| c.instance_id.as_str())
            .collect();
        if instance_ids.is_empty() {
            return None;
        }

        self.legal_actions.iter().find(|a| {
            a.card_instance_id
                .as_deref()
                .is_some_and(|id| instance_ids.contains(&id))
                && match (target_id, a.target_id.as_deref()) {
                    (None, _) => true,
                    (Some(want), Some(got)) => want == got,
                    (Some(_), None) => false,
                }
        })
    }

    pub fn end_turn_action(&self) -> Option<&LegalAction> {
        self.legal_actions
            .iter()
            .find(|a| a.action_id.starts_with("end_turn"))
    }
}

/// Whether the game is waiting for an answer. `snapshot_id` is the action-set
/// fingerprint, comparable with `CombatState::snapshot_id`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDecision {
    pub pending: bool,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub snapshot_id: Option<String>,
    /// Miss it and the game uses its heuristic for the rest of the round.
    #[serde(default)]
    pub deadline_ms: i64,
}

/// In-game settings.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoopConfig {
    pub language: String,
    pub decision_budget_seconds: u32,
    pub calls_per_combat: u32,
}

impl Default for CoopConfig {
    /// When the game can't provide settings (e.g. an older mod).
    fn default() -> Self {
        Self {
            language: "한국어".into(),
            decision_budget_seconds: 20,
            calls_per_combat: 30,
        }
    }
}

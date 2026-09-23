use crate::dto::{CombatState, Intent};
use crate::josa;

/// Everything one decision needs, on one page. Anything missing would cost a tool
/// round trip, which resends the whole context.
pub fn brief(s: &CombatState) -> String {
    if !s.in_combat {
        return "전투 중이 아니다.\n".into();
    }

    let mut out = String::new();
    out.push_str(&format!("# 턴 {}\n\n", s.turn));

    if let Some(m) = &s.me {
        out.push_str(&format!(
            "나 HP {}/{} 방어 {} 에너지 {}\n",
            m.hp, m.max_hp, m.block, m.energy
        ));
    }
    if let Some(a) = &s.ally {
        out.push_str(&format!("동료 HP {}/{} 방어 {}\n", a.hp, a.max_hp, a.block));
    }
    if !s.can_act {
        out.push_str("\n지금은 내 차례가 아니다. 이미 턴을 끝냈다.\n");
    }
    out.push('\n');

    out.push_str(&enemies_table(s));
    out.push('\n');
    out.push_str(&hand_table(s));
    out.push_str(&inventory(s));

    // Interpretation happens here, not in the mod.
    let energy = s.me.as_ref().map_or(0, |m| m.energy);
    let reach = reachable_damage(s, energy);
    out.push_str(&format!(
        "\n에너지 {energy} 안에서 낼 수 있는 최대 피해 {reach}.\n"
    ));

    if let Some(weakest) = s.enemies.iter().min_by_key(|e| e.effective_hp()) {
        let need = weakest.effective_hp();
        let name = &weakest.name;
        if reach >= need {
            out.push_str(&format!(
                "{name}{} 내 손만으로 처치할 수 있다(필요 {need}).\n",
                josa::eun(name)
            ));
        } else {
            out.push_str(&format!(
                "{name}{} 처치하려면 {need}인데 내 손은 {reach}{} 모자란다.\n",
                josa::eul(name),
                josa::ira(&reach.to_string()),
            ));
            // The partner's hand is hidden; say so, so the agent asks.
            out.push_str("동료의 손패는 볼 수 없다. 함께 잡을 수 있는지는 물어봐야 안다.\n");
        }
    }
    out
}

/// One history line standing in for a past turn's briefing: enough to compare
/// turns, without an old hand the agent might pick from. Includes block, since HP
/// deltas alone misread absorbed damage. Powers and intents are left out to keep
/// it short.
pub fn turn_line(s: &CombatState) -> String {
    if !s.in_combat {
        return "전투 밖".into();
    }

    let enemies = s
        .enemies
        .iter()
        .map(|e| format!("{} {}", e.name, e.hp))
        .collect::<Vec<_>>()
        .join(", ");

    let mut out = format!("턴 {}", s.turn);
    if !enemies.is_empty() {
        out.push_str(&format!(" — 적 {enemies}"));
    }
    if let Some(m) = &s.me {
        out.push_str(&format!(" | 나 HP {}", m.hp));
        if m.block > 0 {
            out.push_str(&format!(" 방어 {}", m.block));
        }
        out.push_str(&format!(" 에너지 {}", m.energy));
    }
    if let Some(a) = &s.ally {
        out.push_str(&format!(" | 동료 HP {}", a.hp));
        if a.block > 0 {
            out.push_str(&format!(" 방어 {}", a.block));
        }
    }
    out
}

/// Best damage within the energy budget. Summing every playable card overstates
/// it (two 2-cost cards on 3 energy), and this number decides "can we kill it".
/// Small inputs, so a plain 0/1 knapsack.
fn reachable_damage(s: &CombatState, energy: i32) -> i32 {
    if energy < 0 {
        return 0;
    }
    let cap = energy as usize;
    let mut best = vec![0i32; cap + 1];

    for c in s.hand.iter().filter(|c| c.playable && c.estimated_damage > 0) {
        let cost = c.cost.max(0) as usize;
        if cost > cap {
            continue;
        }
        // Each card at most once, 0-cost included.
        for e in (cost..=cap).rev() {
            best[e] = best[e].max(best[e - cost] + c.estimated_damage);
        }
    }
    best[cap]
}

pub fn enemies_table(s: &CombatState) -> String {
    // Intents as the game draws them. Summing numbers would claim a target the
    // game never told us.
    let mut out = String::from("## 적\n\n| 이름 | HP | 방어 | 의도 |\n|---|---:|---:|---|\n");
    for e in &s.enemies {
        // An enemy can have several intents.
        let intent = if e.intents.is_empty() {
            "-".to_string()
        } else {
            e.intents
                .iter()
                .map(Intent::text)
                .collect::<Vec<_>>()
                .join(" + ")
        };
        out.push_str(&format!(
            "| {} | {}/{} | {} | {} |\n",
            e.name, e.hp, e.max_hp, e.block, intent
        ));
    }
    out
}

/// Relics and potions; relics change how the same board should be read.
pub fn inventory(s: &CombatState) -> String {
    let Some(me) = &s.me else {
        return String::new();
    };
    if me.relics.is_empty() && me.potions.is_empty() {
        return String::new();
    }

    let mut out = String::from("\n## 내가 들고 있는 것\n\n");
    if !me.relics.is_empty() {
        let ids: Vec<&str> = me.relics.iter().map(|r| r.id.as_str()).collect();
        out.push_str(&format!("유물 {}\n", ids.join(", ")));
    }
    if me.potions.is_empty() {
        out.push_str(&format!("포션 없음 (최대 {})\n", me.max_potion_count));
    } else {
        let ids: Vec<&str> = me.potions.iter().map(|p| p.id.as_str()).collect();
        out.push_str(&format!(
            "포션 {} ({}칸 중 {}개)\n",
            ids.join(", "),
            me.max_potion_count,
            me.potions.len()
        ));
    }
    out
}

pub fn hand_table(s: &CombatState) -> String {
    // No position column: the agent would use positions, which shift as cards
    // are played.
    let mut out = String::from(
        "## 손패\n\n| 이름 | 비용 | 낼 수 있나 | 하는 일 |\n|---|---:|---|---|\n",
    );
    for c in &s.hand {
        let ok = if c.playable {
            "예".to_string()
        } else {
            c.unplayable_reason.clone().unwrap_or_else(|| "아니오".into())
        };
        // The description already contains damage and block.
        let what = c.description.clone().unwrap_or_else(|| {
            match (c.estimated_damage, c.estimated_block) {
                (d, 0) if d > 0 => format!("피해 {d}"),
                (0, b) if b > 0 => format!("방어 {b}"),
                (d, b) if d > 0 && b > 0 => format!("피해 {d}, 방어 {b}"),
                _ => "-".into(),
            }
        });
        out.push_str(&format!("| {} | {} | {} | {} |\n", c.name, c.cost, ok, what));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_line_fits_on_one_line() {
        let s: CombatState =
            serde_json::from_str(include_str!("sample_state.json")).unwrap();
        let line = turn_line(&s);

        assert!(line.starts_with("턴 "), "{line}");
        assert_eq!(line.lines().count(), 1, "한 줄이어야 한다: {line}");
        assert!(line.contains(&s.enemies[0].name), "{line}");
        assert!(line.contains("에너지"), "{line}");
    }

    #[test]
    fn turn_line_carries_block() {
        let mut s: CombatState =
            serde_json::from_str(include_str!("sample_state.json")).unwrap();
        if let Some(m) = s.me.as_mut() {
            m.block = 7;
        }
        let line = turn_line(&s);
        assert!(line.contains("방어 7"), "{line}");
    }

    #[test]
    fn turn_line_says_so_when_out_of_combat() {
        assert_eq!(turn_line(&CombatState::default()), "전투 밖");
    }

    use crate::client::tests::sample;

    #[test]
    fn brief_names_what_it_takes_to_kill_the_weakest() {
        let s = brief(&sample());
        assert!(s.contains("80"), "적의 유효 체력이 없다:\n{s}");
        assert!(s.contains("모자란다"), "부족하다는 판단이 없다:\n{s}");
    }

    /// The game doesn't say whom an intent targets.
    #[test]
    fn brief_never_claims_the_enemy_targets_me() {
        let s = brief(&sample());
        assert!(!s.contains("나에게 올 피해"), "대상을 단정하고 있다:\n{s}");
    }

    /// Falls back to the tooltip, then the kind.
    #[test]
    fn an_intent_without_a_label_still_says_something() {
        let s = enemies_table(&sample());
        assert!(!s.contains("| - |"), "의도 칸이 비었다:\n{s}");
    }

    #[test]
    fn every_intent_reaches_the_table() {
        let mut s = sample();
        s.enemies[0].intents = vec![
            Intent { label: Some("공격 12".into()), kind: None, tip: None },
            Intent { label: None, kind: Some("DebuffStrong".into()), tip: Some("취약을 겁니다".into()) },
        ];

        let t = enemies_table(&s);
        assert!(t.contains("공격 12"), "{t}");
        assert!(t.contains("취약을 겁니다"), "둘째 의도가 빠졌다:\n{t}");
    }

    #[test]
    fn what_i_carry_reaches_the_brief() {
        let mut s = sample();
        if let Some(me) = s.me.as_mut() {
            me.relics = vec![crate::dto::Relic { id: "BURNING_BLOOD".into() }];
            me.potions = vec![crate::dto::Potion { id: "FIRE_POTION".into(), slot_index: 0 }];
            me.max_potion_count = 3;
        }

        let b = brief(&s);
        assert!(b.contains("BURNING_BLOOD"), "유물이 없다:\n{b}");
        assert!(b.contains("FIRE_POTION"), "포션이 없다:\n{b}");
        assert!(b.contains("3칸"), "몇 칸인지가 없다 — 아껴야 하는지 알 수 없다:\n{b}");
    }

    /// An empty section would read as if something were there.
    #[test]
    fn an_empty_inventory_adds_no_section() {
        let s = sample();
        assert!(!brief(&s).contains("내가 들고 있는 것"));
    }

    #[test]
    fn the_hand_table_has_no_positions() {
        let s = hand_table(&sample());
        assert!(!s.contains("| # |"), "번호 칸이 있다:\n{s}");
    }

    #[test]
    fn reach_respects_the_energy_budget() {
        let mut s = sample();
        // 3 energy, two 2-cost 8-damage cards: only one fits
        let mut big = s.hand[0].clone();
        big.name = "강타".into();
        big.cost = 2;
        big.estimated_damage = 8;
        big.playable = true;
        s.hand = vec![big.clone(), big];
        s.me.as_mut().unwrap().energy = 3;

        assert_eq!(reachable_damage(&s, 3), 8, "둘을 다 낸 것으로 셌다");
    }

    #[test]
    fn reach_picks_the_better_combination() {
        let mut s = sample();
        let mut big = s.hand[0].clone();
        big.cost = 3;
        big.estimated_damage = 9;
        big.playable = true;
        let mut small = s.hand[0].clone();
        small.cost = 1;
        small.estimated_damage = 6;
        small.playable = true;
        s.hand = vec![big, small.clone(), small];
        s.me.as_mut().unwrap().energy = 3;

        // two 6s beat one 9
        assert_eq!(reachable_damage(&s, 3), 12);
    }

    /// The design budgets ~3,200 tokens per turn; 1,600 chars of brief is the line.
    #[test]
    fn brief_stays_small() {
        let n = brief(&sample()).chars().count();
        assert!(n < 1600, "브리핑이 {n}자다");
    }
}

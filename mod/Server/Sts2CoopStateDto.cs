using System.Collections.Generic;

namespace Sts2LlmCoop;

/// Facts the game can compute. Turning them into prose is the bridge's job.
internal sealed class CombatStateDto
{
    public bool InCombat { get; set; }

    /// Echoed back with an action; rejected if the legal actions changed since.
    public string SnapshotId { get; set; } = "";

    /// False after ending the turn; LegalActions is then empty by design.
    public bool CanAct { get; set; }

    /// Lets the bridge tell a new run (clear the conversation) from its own restart
    /// (keep it). A continued run keeps its seed.
    public string RunSeed { get; set; } = "";

    public int Turn { get; set; }
    public string? Phase { get; set; }
    public ActorDto? Me { get; set; }
    public ActorDto? Ally { get; set; }
    public List<EnemyDto> Enemies { get; set; } = [];
    public List<CardDto> Hand { get; set; } = [];
    public int DrawCount { get; set; }
    public int DiscardCount { get; set; }

    /// Referenced by ActionId, never by position.
    public List<LegalActionDto> LegalActions { get; set; } = [];
}

internal sealed class LegalActionDto
{
    public string ActionId { get; set; } = "";
    public string? CardInstanceId { get; set; }
    public string? TargetId { get; set; }
    public int EnergyCost { get; set; }
    public string? Description { get; set; }
}

internal sealed class ActorDto
{
    public ulong NetId { get; set; }
    public int Hp { get; set; }
    public int MaxHp { get; set; }
    public int Block { get; set; }
    public int Energy { get; set; }
    public Dictionary<string, int> Powers { get; set; } = [];

    /// Per player; says nothing about whether everyone has ended.
    public bool EndedTurn { get; set; }

    /// The AI is holding a plan. Only ever true for the AI player.
    public bool Holding { get; set; }

    public List<RelicDto> Relics { get; set; } = [];

    /// Also in the action list, but only here with capacity.
    public List<PotionDto> Potions { get; set; } = [];

    public int MaxPotionCount { get; set; }
}

internal sealed class EnemyDto
{
    /// creature_<combatId>.
    public string Id { get; set; } = "";

    public string Name { get; set; } = "";
    public int Hp { get; set; }
    public int MaxHp { get; set; }
    public int Block { get; set; }

    /// All of them: an enemy can attack and debuff in the same move.
    public List<IntentDto> Intents { get; set; } = [];

    /// Damage *if* this enemy targets me. The game doesn't expose intent targets,
    /// so don't sum this as incoming damage.
    public int DamageIfItHitsMe { get; set; }

    public Dictionary<string, int> Powers { get; set; } = [];
}

internal sealed class IntentDto
{
    /// As drawn over the enemy's head.
    public string? Label { get; set; }

    /// Attack, DebuffStrong, ... for when the label is empty.
    public string? Kind { get; set; }

    /// Tooltip text; the only description for intents without a number.
    public string? Tip { get; set; }
}

internal sealed class RelicDto
{
    /// e.g. `BOTTLED_LIGHTNING`.
    public string Id { get; set; } = "";
}

internal sealed class PotionDto
{
    public string Id { get; set; } = "";

    /// Matches `_<index>_` in the action id.
    public int SlotIndex { get; set; }
}

internal sealed class CardDto
{
    /// combat_<cardId>, as used in ActionIds.
    public string InstanceId { get; set; } = "";

    public string Id { get; set; } = "";
    public string Name { get; set; } = "";
    public string Type { get; set; } = "";
    public int Cost { get; set; }
    public int EstimatedDamage { get; set; }
    public int EstimatedBlock { get; set; }
    public string? Description { get; set; }

    public bool Playable { get; set; }
    public string? UnplayableReason { get; set; }
    public List<string> Keywords { get; set; } = [];
}

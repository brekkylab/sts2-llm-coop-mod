using System;
using System.Collections.Generic;
using System.Linq;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.HoverTips;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.Entities.Potions;
using MegaCrit.Sts2.Core.Entities.Relics;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.MonsterMoves.Intents;
using MegaCrit.Sts2.Core.Runs;

namespace Sts2LlmCoop;

/// Builds the combat state the bridge reads.
///
/// Card views and enemy state come straight from
/// <see cref="DeterministicCombatContextBuilder"/>; rebuilding them here could make
/// an ActionId point at something different from what the controller sees.
internal static class Sts2CoopStateEndpoint
{
    private static readonly DeterministicCombatContextBuilder ContextBuilder = new();

    /// Main thread only.
    public static CombatStateDto BuildCombatState()
    {
        var dto = new CombatStateDto();

        CombatManager? combat = CombatManager.Instance;
        if (combat == null || !combat.IsInProgress) { return dto; }

        Player? me = FindAiPlayer();
        if (me?.Creature?.CombatState == null || me.PlayerCombatState == null) { return dto; }
        if (!AiTeammateDummyController.TryGetControllerFor(me.NetId, out AiTeammateDummyController controller))
        {
            return dto;
        }

        IReadOnlyList<AiTeammateAvailableAction> actions = controller.DiscoverAvailableActions();
        List<AiLegalActionOption> options = actions.Select(a => a.Option).ToList();

        DeterministicCombatContext? ctx = ContextBuilder.Build(me.NetId.ToString(), options);
        if (ctx == null) { return dto; }

        dto.InCombat = true;
        // Distinguishes "nothing to play" from "not this player's moment".
        dto.CanAct = !combat.IsPlayerReadyToEndTurn(me);
        dto.SnapshotId = SnapshotIdOf(actions);
        dto.RunSeed = CurrentRunSeed();
        dto.Turn = me.Creature.CombatState?.RoundNumber ?? 0;
        dto.Phase = me.PlayerCombatState.Phase.ToString();
        dto.Me = BuildActor(me);
        dto.Ally = FindHumanPlayer() is { } human ? BuildActor(human) : null;

        foreach (KeyValuePair<string, DeterministicEnemyState> pair in ctx.EnemiesById)
        {
            Creature enemy = pair.Value.Creature;
            dto.Enemies.Add(new EnemyDto
            {
                Id = pair.Key,
                Name = enemy.Name,
                Hp = enemy.CurrentHp,
                MaxHp = enemy.MaxHp,
                Block = enemy.Block,
                Intents = IntentsOf(enemy),
                DamageIfItHitsMe = pair.Value.IncomingDamage,
                Powers = VisiblePowers(enemy),
            });
        }

        foreach (KeyValuePair<string, ResolvedCardView> pair in ctx.HandCardsByInstanceId)
        {
            ResolvedCardView view = pair.Value;
            CardModel? card = PileType.Hand.GetPile(me).Cards
                .FirstOrDefault(c => c.Id.Entry == view.CardId);
            bool playable = card?.CanPlay(out UnplayableReason _, out _) ?? false;
            UnplayableReason reason = UnplayableReason.None;
            card?.CanPlay(out reason, out _);

            dto.Hand.Add(new CardDto
            {
                InstanceId = pair.Key,
                Id = view.CardId,
                Name = view.Name,
                Type = view.Type.ToString(),
                Cost = view.EffectiveCost,
                EstimatedDamage = view.GetEstimatedDamage(),
                EstimatedBlock = view.GetEstimatedBlock(),
                Description = DescriptionOf(card),
                Playable = playable,
                UnplayableReason = playable ? null : reason.ToString(),
                Keywords = view.Keywords.ToList(),
            });
        }

        dto.DrawCount = PileType.Draw.GetPile(me).Cards.Count;
        dto.DiscardCount = PileType.Discard.GetPile(me).Cards.Count;
        dto.LegalActions = options.Select(o => new LegalActionDto
        {
            ActionId = o.ActionId,
            CardInstanceId = o.CardInstanceId,
            TargetId = o.TargetId,
            EnergyCost = o.EnergyCost ?? 0,
            Description = o.Description,
        }).ToList();

        return dto;
    }

    /// Empty when unreadable; the bridge then leaves the conversation alone.
    private static string CurrentRunSeed()
    {
        try
        {
            return RunManager.Instance.DebugOnlyGetState()?.Rng.StringSeed ?? "";
        }
        catch (Exception)
        {
            return "";
        }
    }

    /// The set of legal actions doubles as a point-in-time marker: playing a card
    /// changes it, which lets stale plans be rejected. Same as the controller's
    /// fingerprint.
    public static string SnapshotIdOf(IReadOnlyList<AiTeammateAvailableAction> actions)
    {
        return string.Join("|", actions.Select(a => a.ActionId));
    }

    internal static Player? FindAiPlayer()
    {
        return AllPlayers().FirstOrDefault(AiTeammateDummyController.IsAiPlayer);
    }

    /// Public alias of `FindHumanPlayer` for the controller.
    public static Player? FindHumanPlayerOrNull() => FindHumanPlayer();

    private static Player? FindHumanPlayer()
    {
        return AllPlayers().FirstOrDefault(p => !AiTeammateDummyController.IsAiPlayer(p));
    }

    /// `RunManager.State` is private; this is the only public path (same as
    /// AiTeammatePeerInputPatches).
    private static IEnumerable<Player> AllPlayers()
    {
        return RunManager.Instance?.DebugOnlyGetState()?.Players ?? Enumerable.Empty<Player>();
    }

    private static ActorDto BuildActor(Player p) => new()
    {
        NetId = p.NetId,
        Hp = p.Creature.CurrentHp,
        MaxHp = p.Creature.MaxHp,
        Block = p.Creature.Block,
        Energy = p.PlayerCombatState?.Energy ?? 0,
        Powers = VisiblePowers(p.Creature),
        EndedTurn = CombatManager.Instance?.IsPlayerReadyToEndTurn(p) ?? false,
        Holding = AiTeammateDummyController.TryGetControllerFor(
            p.NetId, out AiTeammateDummyController heldBy) && heldBy.IsHoldingPlan,
        Relics = RelicsOf(p),
        Potions = PotionsOf(p),
        MaxPotionCount = p.MaxPotionCount,
    };

    /// Ids only: `RelicModel` has no display name or description, and an id like
    /// `BOTTLED_LIGHTNING` is readable enough for the model.
    private static List<RelicDto> RelicsOf(Player p)
    {
        return p.Relics.Select(r => new RelicDto { Id = r.Id.Entry }).ToList();
    }

    /// Slot numbers match the index in potion action ids. Queued potions are left
    /// out, as they are from the action list.
    private static List<PotionDto> PotionsOf(Player p)
    {
        return p.Potions
            .Where(static x => !x.IsQueued)
            .Select((x, i) => new PotionDto { Id = x.Id.Entry, SlotIndex = i })
            .ToList();
    }

    private static string? DescriptionOf(CardModel? card)
    {
        if (card == null) { return null; }
        try
        {
            string text = card.GetDescriptionForPile(PileType.Hand);
            return StripRichText(text).Replace("\n", " ");
        }
        catch (Exception)
        {
            return null;
        }
    }

    /// Same numbers as shown on screen.
    private static Dictionary<string, int> VisiblePowers(Creature c)
    {
        return c.Powers
            .Where(x => x.IsVisible)
            .ToDictionary(x => x.Id.Entry, x => x.DisplayAmount);
    }

    /// The labels the game draws over the enemy, so the model sees what the player
    /// sees.
    private static List<IntentDto> IntentsOf(Creature enemy)
    {
        var out_ = new List<IntentDto>();
        IReadOnlyList<AbstractIntent>? intents = enemy.Monster?.NextMove?.Intents;
        if (intents == null) { return out_; }

        // Must be every player in combat; a single target yields empty text.
        IEnumerable<Creature>? targets = enemy.CombatState?.PlayerCreatures;

        foreach (AbstractIntent intent in intents)
        {
            string kind = intent.IntentType.ToString();
            if (targets == null)
            {
                out_.Add(new IntentDto { Kind = kind });
                continue;
            }

            // Non-attack intents have an empty label; the tooltip carries what they
            // do. LocString.ToString() is a debug form, hence GetFormattedText().
            string label = StripRichText(intent.GetIntentLabel(targets, enemy).GetFormattedText());
            HoverTip hover = intent.GetHoverTip(targets, enemy);
            string? tip = hover.Description is { } desc ? StripRichText(desc) : null;
            if (string.IsNullOrEmpty(tip) && hover.Title is { } title)
            {
                tip = StripRichText(title);
            }

            out_.Add(new IntentDto
            {
                Label = label.Length == 0 ? null : label,
                Kind = kind,
                Tip = string.IsNullOrEmpty(tip) ? null : tip,
            });
        }

        return out_;
    }

    /// Removes BBCode tags like `[color=...]`.
    private static string StripRichText(string text)
    {
        return RichTextTag.Replace(text, string.Empty).Trim();
    }

    private static readonly System.Text.RegularExpressions.Regex RichTextTag =
        new(@"\[/?[a-zA-Z][^\]]*\]", System.Text.RegularExpressions.RegexOptions.Compiled);
}

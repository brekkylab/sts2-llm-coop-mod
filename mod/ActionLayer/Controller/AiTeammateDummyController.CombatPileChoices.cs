using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Models;

namespace Sts2LlmCoop;

internal sealed partial class AiTeammateDummyController
{
    /// Picks cards from a combat pile (e.g. Liquid Memories). Without a prefix here
    /// the game shows the UI to the local player and deadlocks waiting for it.
    public static Task<IEnumerable<CardModel>> ChooseCombatPileCardsAsync(
        Player player,
        IEnumerable<CardModel> options,
        int minSelect,
        int maxSelect)
    {
        List<CardModel> list = options.ToList();
        int want = ComputeSelectionCount(list.Count, minSelect, maxSelect);
        if (want <= 0)
        {
            return Task.FromResult<IEnumerable<CardModel>>([]);
        }

        CardChoiceDecision decision = CardEvaluator.EvaluateCandidates(
            list,
            CardEvaluator.ContextFactory.Create(
                player,
                CardChoiceSource.ForcedChoice,
                skipAllowed: false,
                debugSource: "combat_pile"));

        List<CardEvaluationResult> ranked = decision.RankedResults.ToList();
        if (ranked.Count == 0)
        {
            return Task.FromResult<IEnumerable<CardModel>>(list.Take(want).ToList());
        }

        int energy = player.PlayerCombatState?.Energy ?? 0;
        int weakestEnemyHp = WeakestEnemyEffectiveHp(player);

        // The card goes to hand now, so playability beats deck-building score.
        List<CardEvaluationResult> ordered = ranked
            .OrderByDescending(r => Playable(r, energy))
            .ThenByDescending(r => Finishes(r, weakestEnemyHp))
            .ThenByDescending(r => r.FinalScore)
            .ToList();

        List<CardModel> picked = ordered.Take(want).Select(r => r.CandidateCard).ToList();
        Log.Info($"[AITeammate] Combat pile choice player={player.NetId} "
            + $"want={want} energy={energy} weakestEnemyHp={weakestEnemyHp} "
            + $"picked={string.Join(",", picked.Select(c => c.Id.Entry))} "
            + $"top={ordered[0].Describe()}");
        return Task.FromResult<IEnumerable<CardModel>>(picked);
    }

    /// Cards to discard or remove: the tail of the ranking.
    public static Task<IEnumerable<CardModel>> ChooseWorstCardsAsync(
        Player player,
        IEnumerable<CardModel> options,
        int minSelect,
        int maxSelect,
        string debugSource)
    {
        List<CardModel> list = options.ToList();
        int want = ComputeSelectionCount(list.Count, minSelect, maxSelect);
        if (want <= 0)
        {
            return Task.FromResult<IEnumerable<CardModel>>([]);
        }

        CardChoiceDecision decision = CardEvaluator.EvaluateCandidates(
            list,
            CardEvaluator.ContextFactory.Create(
                player,
                CardChoiceSource.ForcedChoice,
                skipAllowed: false,
                debugSource: debugSource));

        List<CardEvaluationResult> ranked = decision.RankedResults.ToList();
        if (ranked.Count == 0)
        {
            return Task.FromResult<IEnumerable<CardModel>>(list.Take(want).ToList());
        }

        List<CardModel> picked = ranked
            .OrderBy(r => r.FinalScore)
            .Take(want)
            .Select(r => r.CandidateCard)
            .ToList();
        Log.Info($"[AITeammate] Worst-card choice player={player.NetId} source={debugSource} "
            + $"want={want} picked={string.Join(",", picked.Select(c => c.Id.Entry))}");
        return Task.FromResult<IEnumerable<CardModel>>(picked);
    }

    private static bool Playable(CardEvaluationResult r, int energy)
    {
        return r.Candidate.EffectiveCost <= energy;
    }

    /// Single card only; summing the hand would need energy and ordering to be right.
    private static bool Finishes(CardEvaluationResult r, int weakestEnemyHp)
    {
        return weakestEnemyHp > 0
            && r.Candidate.Type == CardType.Attack
            && r.Candidate.GetEstimatedDamage() >= weakestEnemyHp;
    }

    private static int WeakestEnemyEffectiveHp(Player player)
    {
        IReadOnlyList<Creature>? enemies = player.Creature?.CombatState?.HittableEnemies;
        if (enemies == null || enemies.Count == 0)
        {
            return 0;
        }

        return enemies.Min(e => e.CurrentHp + e.Block);
    }
}

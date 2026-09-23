using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Threading.Tasks;
using HarmonyLib;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.GameActions;
using MegaCrit.Sts2.Core.GameActions.Multiplayer;
using MegaCrit.Sts2.Core.Helpers;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Runs;

namespace Sts2LlmCoop;

internal sealed partial class AiTeammateDummyController
{
    private static readonly FieldInfo? PotionBeforeUseField =
        AccessTools.Field(typeof(PotionModel), "BeforeUse");
    private static readonly FieldInfo? PotionIsQueuedField =
        AccessTools.Field(typeof(PotionModel), "<IsQueued>k__BackingField");

    private IReadOnlyList<AiTeammateAvailableAction> DiscoverCombatActions(Player player)
    {
        List<AiTeammateAvailableAction> actions = [];
        Log.Debug($"[AITeammate] DiscoverCombatActions player={player.NetId} roomCount={player.RunState.CurrentRoomCount} currentRoom={player.RunState.CurrentRoom?.GetType().Name ?? "null"} inProgress={CombatManager.Instance.IsInProgress} playPhase={CombatManager.Instance.IsInProgress}");

        // Tells a bad hand from bad action discovery when a plan looks wrong.
        var hand = PileType.Hand.GetPile(player).Cards;
        Log.Info($"[AITeammate][DIAG] player={player.NetId} hand={hand.Count} "
            + $"draw={PileType.Draw.GetPile(player).Cards.Count} "
            + $"discard={PileType.Discard.GetPile(player).Cards.Count} "
            + $"phase={player.PlayerCombatState?.Phase} "
            + $"combatStateType={player.Creature?.CombatState?.GetType().Name ?? "null"}");

        int playableCount = 0;
        bool blockedOnlyByEnergy = true;

        foreach (CardModel card in PileType.Hand.GetPile(player).Cards)
        {
            UnplayableReason reason;
            MegaCrit.Sts2.Core.Models.AbstractModel? preventer;
            if (!card.CanPlay(out reason, out preventer))
            {
                Log.Info($"[AITeammate][DIAG] card={card.Id.Entry} CanPlay=false reason={reason} preventer={preventer?.GetType().Name ?? "null"}");
                if (reason != UnplayableReason.EnergyCostTooHigh)
                {
                    blockedOnlyByEnergy = false;
                }
                continue;
            }
            playableCount++;

            bool addedAction = false;
            // One action per possible target. Taking only the first sent every ally
            // card to the partner and made a second enemy untargetable.
            foreach (Creature? target in GetOrderedTargets(card.TargetType, player))
            {
                if (target == null || IsPlayableTarget(card, target, player))
                {
                    AddPlayCardAction(actions, card, target);
                    addedAction = true;
                }
            }

            if (!addedAction)
            {
                Log.Debug($"[AITeammate] Skipped combat action for card={card.Id.Entry} instance={GetCardInstanceId(card)} because no playable target was found for targetType={card.TargetType}.");
            }
        }

        List<PotionModel> potions = player.Potions.Where(static potion => !potion.IsQueued).ToList();
        for (int potionIndex = 0; potionIndex < potions.Count; potionIndex++)
        {
            PotionModel potion = potions[potionIndex];
            foreach (Creature? target in GetOrderedTargets(potion.TargetType, player))
            {
            if (potion.TargetType.IsSingleTarget() && target == null)
            {
                continue;
            }

            string targetName = DescribeTarget(target);
            string actionId = BuildUsePotionActionId(potion, target, potionIndex);
            actions.Add(new AiTeammateAvailableAction(
                new AiLegalActionOption
                {
                    ActionId = actionId,
                    ActionType = AiTeammateActionKind.UsePotion.ToString(),
                    Description = $"Use potion {potion.Id.Entry} -> {targetName}",
                    Label = $"Use potion {potion.Id.Entry}",
                    Summary = $"Use potion {potion.Id.Entry} targeting {targetName}.",
                    CardId = potion.Id.Entry,
                    TargetId = GetTargetId(target),
                    TargetLabel = targetName
                },
                () =>
                {
                    (PotionBeforeUseField?.GetValue(potion) as System.Action)?.Invoke();
                    UsePotionAction usePotionAction = new(potion, target, CombatManager.Instance.IsInProgress);
                    PotionIsQueuedField?.SetValue(potion, true);
                    RunManager.Instance.ActionQueueSynchronizer.RequestEnqueue(usePotionAction);
                    return Task.FromResult(new AiActionExecutionResult
                    {
                        GameAction = usePotionAction,
                        WaitForQueueSettle = true
                    });
                },
                // Target is part of the key, or per-target actions collapse into one.
                deduplicationKey: $"potion:{potion.Id.Entry}:{potionIndex}:{GetTargetId(target)}"));
            }
        }

        // Out of energy is normal; warn only when something else blocks the hand.
        if (playableCount == 0 && hand.Count > 0 && !blockedOnlyByEnergy)
        {
            Log.Info($"[AITeammate][DIAG] WARNING player={player.NetId} has {hand.Count} card(s) "
                + "but none is playable for a reason other than energy; only end turn is offered.");
        }

        actions.Add(new AiTeammateAvailableAction(
            new AiLegalActionOption
            {
                ActionId = BuildEndTurnActionId(player),
                ActionType = AiTeammateActionKind.EndTurn.ToString(),
                Description = "End turn",
                Label = "End turn",
                Summary = "Finish the actor's current turn."
            },
            () =>
            {
                int roundNumber = player.Creature.CombatState?.RoundNumber ?? 0;
                EndPlayerTurnAction endTurnAction = new(player, roundNumber);
                RunManager.Instance.ActionQueueSynchronizer.RequestEnqueue(endTurnAction);
                return Task.FromResult(new AiActionExecutionResult
                {
                    GameAction = endTurnAction,
                    WaitForQueueSettle = true
                });
            }));

        return actions;
    }

    private static IEnumerable<Creature?> GetOrderedTargets(TargetType targetType, Player player)
    {
        CombatState? combatState = player.Creature.CombatState as CombatState;
        if (combatState == null)
        {
            return new Creature?[] { null };
        }

        return targetType switch
        {
            TargetType.AnyEnemy => combatState.HittableEnemies.OrderBy(static creature => creature.CombatId ?? uint.MaxValue).Cast<Creature?>(),
            TargetType.AnyAlly => combatState.PlayerCreatures.Where(static creature => creature.IsAlive).OrderBy(static creature => creature.Player?.NetId ?? 0UL).Cast<Creature?>(),
            TargetType.AnyPlayer => combatState.PlayerCreatures.Where(static creature => creature.IsAlive).OrderBy(static creature => creature.Player?.NetId ?? 0UL).Cast<Creature?>(),
            TargetType.Self => new Creature?[] { player.Creature },
            _ => new Creature?[] { null },
        };
    }

    private static bool IsPlayableTarget(CardModel card, Creature target, Player player)
    {
        if (card.TargetType == TargetType.Self && ReferenceEquals(target, player.Creature))
        {
            return true;
        }

        return card.CanPlayTargeting(target);
    }

    private static void AddPlayCardAction(List<AiTeammateAvailableAction> actions, CardModel card, Creature? target)
    {
        Creature? executionTarget = card.TargetType == TargetType.Self ? null : target;
        string targetName = card.TargetType == TargetType.Self ? "the AI teammate" : DescribeTarget(target);
        string actionId = BuildPlayCardActionId(card, executionTarget);
        actions.Add(new AiTeammateAvailableAction(
            new AiLegalActionOption
            {
                ActionId = actionId,
                ActionType = AiTeammateActionKind.PlayCard.ToString(),
                Description = $"Play {card.Id.Entry} -> {targetName}",
                Label = $"Play {card.Id.Entry}",
                Summary = $"Play {card.Id.Entry} targeting {targetName}.",
                CardId = card.Id.Entry,
                CardInstanceId = GetCardInstanceId(card),
                TargetId = GetTargetId(executionTarget),
                TargetLabel = targetName,
                EnergyCost = card.EnergyCost.GetAmountToSpend()
            },
            () =>
            {
                TaskHelper.RunSafely(card.OnEnqueuePlayVfx(executionTarget));
                PlayCardAction playCardAction = new(card, executionTarget);
                RunManager.Instance.ActionQueueSynchronizer.RequestEnqueue(playCardAction);
                return Task.FromResult(new AiActionExecutionResult
                {
                    GameAction = playCardAction,
                    WaitForQueueSettle = true
                });
            },
            // Target is part of the key, or per-target actions collapse into one.
            deduplicationKey: $"card:{GetCardInstanceId(card)}:{GetTargetId(executionTarget)}"));
    }

    private static string BuildPlayCardActionId(CardModel card, Creature? target)
    {
        return $"play_card_{GetCardInstanceId(card)}_target_{GetTargetId(target)}";
    }

    private static string BuildUsePotionActionId(PotionModel potion, Creature? target, int potionIndex)
    {
        return $"use_potion_{SanitizeActionToken(potion.Id.Entry)}_{potionIndex}_target_{GetTargetId(target)}";
    }

    private static string BuildEndTurnActionId(Player player)
    {
        return $"end_turn_player_{player.NetId}";
    }

    private static string GetCardInstanceId(CardModel card)
    {
        return NetCombatCardDb.Instance.TryGetCardId(card, out uint cardId)
            ? $"combat_{cardId}"
            : SanitizeActionToken(card.Id.ToString());
    }

    /// The only thing the agent has to tell who a card lands on. Roles, not names
    /// (neither node names nor nicknames say self vs. partner), and no "me"/"myself":
    /// both the agent and the human read this string, and a first-person word would
    /// flip meaning depending on the reader.
    private static string DescribeTarget(Creature? target)
    {
        if (target == null)
        {
            return "none";
        }

        if (target.Player is { } p)
        {
            return Sts2CoopStateEndpoint.FindAiPlayer()?.NetId == p.NetId
                ? "the AI teammate"
                : "the human player";
        }

        return target.ToString() ?? "unknown";
    }

    private static string GetTargetId(Creature? target)
    {
        if (target == null)
        {
            return "none";
        }

        if (target.Player != null)
        {
            return $"player_{target.Player.NetId}";
        }

        return $"creature_{target.CombatId?.ToString() ?? SanitizeActionToken(target.ToString())}";
    }

    private static string SanitizeActionToken(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return "unknown";
        }

        return value.Replace(':', '_').Replace('/', '_').Replace(' ', '_');
    }
}

// ReSharper disable InconsistentNaming
using System;
using System.Diagnostics.CodeAnalysis;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using HarmonyLib;
using MegaCrit.Sts2.Core.CardSelection;
using MegaCrit.Sts2.Core.Commands;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.GameActions.Multiplayer;
using MegaCrit.Sts2.Core.Models;

namespace Sts2LlmCoop;

/// Four CardSelectCmd entry points AiTeammateCardSelectionPatches misses. New game
/// methods open silent gaps like these: if the AI hangs on a new selection screen,
/// compare both lists against the game assembly first.
[SuppressMessage("ReSharper", "UnusedType.Global")]
internal static class AiTeammateCombatPileSelectionPatches
{
    [HarmonyPatch(typeof(CardSelectCmd), nameof(CardSelectCmd.FromCombatPile), new[] { typeof(PlayerChoiceContext), typeof(CardPile), typeof(Player), typeof(CardSelectorPrefs) })]
    [SuppressMessage("ReSharper", "UnusedType.Global")]
    private static class CardSelectCombatPilePatch
    {
        private static bool Prefix(
            PlayerChoiceContext context,
            CardPile pile,
            Player player,
            CardSelectorPrefs prefs,
            ref Task<IEnumerable<CardModel>> __result)
        {
            if (!AiTeammateDummyController.IsAiPlayer(player))
            {
                return true;
            }

            __result = AiTeammateDummyController.ChooseCombatPileCardsAsync(
                player,
                pile.Cards,
                prefs.MinSelect,
                prefs.MaxSelect);
            return false;
        }
    }

    [HarmonyPatch(typeof(CardSelectCmd), nameof(CardSelectCmd.FromCombatPile), new[] { typeof(PlayerChoiceContext), typeof(CardPile), typeof(Player), typeof(CardSelectorPrefs), typeof(Func<CardModel, bool>) })]
    [SuppressMessage("ReSharper", "UnusedType.Global")]
    private static class CardSelectCombatPileFilteredPatch
    {
        private static bool Prefix(
            PlayerChoiceContext context,
            CardPile pile,
            Player player,
            CardSelectorPrefs prefs,
            Func<CardModel, bool>? filter,
            ref Task<IEnumerable<CardModel>> __result)
        {
            if (!AiTeammateDummyController.IsAiPlayer(player))
            {
                return true;
            }

            IEnumerable<CardModel> options = pile.Cards;
            if (filter != null)
            {
                options = options.Where(filter);
            }

            __result = AiTeammateDummyController.ChooseCombatPileCardsAsync(
                player,
                options,
                prefs.MinSelect,
                prefs.MaxSelect);
            return false;
        }
    }

    [HarmonyPatch(typeof(CardSelectCmd), nameof(CardSelectCmd.FromHandForDiscard))]
    [SuppressMessage("ReSharper", "UnusedType.Global")]
    private static class CardSelectHandDiscardPatch
    {
        private static bool Prefix(
            PlayerChoiceContext context,
            Player player,
            CardSelectorPrefs prefs,
            Func<CardModel, bool>? filter,
            ref Task<IEnumerable<CardModel>> __result)
        {
            if (!AiTeammateDummyController.IsAiPlayer(player))
            {
                return true;
            }

            IEnumerable<CardModel> options = PileType.Hand.GetPile(player).Cards;
            if (filter != null)
            {
                options = options.Where(filter);
            }

            __result = AiTeammateDummyController.ChooseWorstCardsAsync(
                player,
                options,
                prefs.MinSelect,
                prefs.MaxSelect,
                "hand_discard");
            return false;
        }
    }

    // Irreversible; picking the first card would throw away a good one.
    [HarmonyPatch(typeof(CardSelectCmd), nameof(CardSelectCmd.FromDeckForRemoval))]
    [SuppressMessage("ReSharper", "UnusedType.Global")]
    private static class CardSelectDeckRemovalPatch
    {
        private static bool Prefix(
            Player player,
            CardSelectorPrefs prefs,
            Func<CardModel, bool>? filter,
            ref Task<IEnumerable<CardModel>> __result)
        {
            if (!AiTeammateDummyController.IsAiPlayer(player))
            {
                return true;
            }

            IEnumerable<CardModel> options = PileType.Deck.GetPile(player).Cards;
            if (filter != null)
            {
                options = options.Where(filter);
            }

            __result = AiTeammateDummyController.ChooseWorstCardsAsync(
                player,
                options,
                prefs.MinSelect,
                prefs.MaxSelect,
                "deck_removal");
            return false;
        }
    }
}

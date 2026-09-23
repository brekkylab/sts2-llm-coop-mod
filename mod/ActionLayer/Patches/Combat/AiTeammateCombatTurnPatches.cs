using System.Linq;
using System.Diagnostics.CodeAnalysis;
using HarmonyLib;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Runs;

namespace STS2AiTeammate;

[SuppressMessage("ReSharper", "UnusedType.Global")]
internal static class AiTeammateCombatTurnPatches
{
    private static bool _syncingAiEnemyTurnReady;

    [HarmonyPatch(typeof(CombatManager), nameof(CombatManager.SetReadyToEndTurn))]
[SuppressMessage("ReSharper", "UnusedType.Global")]
    private static class CombatManagerReadyToEndTurnPatch
    {
        private static void Postfix(Player player, bool canBackOut)
        {
            AiTeammateSessionState? session = AiTeammateSessionRegistry.Current;
            if (session == null || RunManager.Instance.NetService is not AiTeammateLoopbackHostGameService)
            {
                return;
            }

            Log.Info($"[AITeammate] ReadyToEndTurn player={player.NetId} host={session.HostPlayerId} canBackOut={canBackOut} allReady={CombatManager.Instance.AllPlayersReadyToEndTurn()}");
        }
    }

    [HarmonyPatch(typeof(CombatManager), nameof(CombatManager.SetReadyToBeginEnemyTurn))]
[SuppressMessage("ReSharper", "UnusedType.Global")]
    private static class CombatManagerReadyToBeginEnemyTurnPatch
    {
        private static void Postfix(Player player)
        {
            AiTeammateSessionState? session = AiTeammateSessionRegistry.Current;
            if (session == null || RunManager.Instance.NetService is not AiTeammateLoopbackHostGameService)
            {
                return;
            }

            Log.Info($"[AITeammate] ReadyToBeginEnemyTurn player={player.NetId} host={session.HostPlayerId}");
            if (_syncingAiEnemyTurnReady || player.NetId != session.HostPlayerId)
            {
                return;
            }

            try
            {
                _syncingAiEnemyTurnReady = true;
                foreach (AiTeammateSessionParticipant participant in session.Participants.Where(static participant => !participant.IsHost))
                {
                    Player? aiPlayer = RunManager.Instance.DebugOnlyGetState()?.GetPlayer(participant.PlayerId);
                    if (aiPlayer == null)
                    {
                        continue;
                    }

                    if (CombatManager.Instance.IsPlayerReadyToEndTurn(aiPlayer))
                    {
                        Log.Info($"[AITeammate] Auto-marking AI ready to begin enemy turn. aiPlayer={participant.PlayerId}");
                        CombatManager.Instance.SetReadyToBeginEnemyTurn(aiPlayer);
                    }
                }
            }
            finally
            {
                _syncingAiEnemyTurnReady = false;
            }
        }
    }
}

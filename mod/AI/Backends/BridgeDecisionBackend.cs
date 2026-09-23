using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using MegaCrit.Sts2.Core.Logging;

namespace Sts2LlmCoop;

/// Asks the bridge and fails if there is no answer. Deliberately has no heuristic
/// fallback: after an `await` it would run on the thread pool and read game state
/// there. The controller falls back on the main thread instead.
internal sealed class BridgeDecisionBackend
{
    /// Read every time so in-game changes apply; this instance outlives combats.
    public static TimeSpan Budget =>
        TimeSpan.FromSeconds(Sts2CoopConfig.DecisionBudgetSeconds);

    private static int CallsPerCombat => Sts2CoopConfig.CallsPerCombat;

    private int _callsThisCombat;

    public void ResetForNewCombat() => _callsThisCombat = 0;

    /// Returns the pending Task for the caller to hold (not await), or null once the
    /// per-combat call cap is reached.
    public Task<AiDecisionResult>? TryAsk(AiDecisionRequest request, CancellationToken ct)
    {
        if (_callsThisCombat >= CallsPerCombat)
        {
            Log.Info($"[Sts2Coop] call limit ({CallsPerCombat}) reached; heuristic from here");
            return null;
        }

        _callsThisCombat++;

        // Not request.SnapshotId, which is a timestamp; the bridge compares against
        // the action-set fingerprint from /state/combat.
        string snapshotId = string.Join("|", request.LegalActions.Select(static a => a.ActionId));
        return AskAndValidate(request, snapshotId, ct);
    }

    private static async Task<AiDecisionResult> AskAndValidate(
        AiDecisionRequest request, string snapshotId, CancellationToken ct)
    {
        DecisionReplyDto reply = await Sts2CoopDecisionEndpoint.AskAsync(snapshotId, Budget, ct);

        // Older bridges send a single action.
        List<string> planned = reply.ActionIds is { Count: > 0 } ids
            ? ids
            : [reply.ActionId];

        // Early rejection of invented ids only. Later steps really depend on the board
        // after earlier ones; the controller re-checks each step before playing it.
        foreach (string id in planned)
        {
            if (request.LegalActions.All(a => !string.Equals(a.ActionId, id, StringComparison.Ordinal)))
            {
                throw new InvalidOperationException(
                    $"answer names an action that is not legal: {id}");
            }
        }

        return new AiDecisionResult
        {
            ChosenActionId = planned[0],
            PlannedActionIds = planned,
            ActNow = reply.Now,
            Reason = reply.Say ?? "decided by the bridge",
        };
    }
}

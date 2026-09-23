using System;
using System.Linq;
using System.Threading.Tasks;
using MegaCrit.Sts2.Core.Logging;

namespace Sts2LlmCoop;

internal sealed partial class AiTeammateDummyController
{
    /// Runs an action requested over HTTP and waits until it has settled. Unlike
    /// <see cref="TryExecuteActionById"/>, which only issues it: replying on issue
    /// would let the next request pass `CanPlay` before energy is spent.
    ///
    /// Returns null on success, otherwise the reason.
    public async Task<string?> ExecuteForServerAsync(string actionId, string? expectedSnapshotId)
    {
        // Read once, so what is checked is what gets executed.
        var available = DiscoverAvailableActions();

        // Snapshot first, so a stale plan is reported as such rather than as a
        // missing card.
        if (!string.IsNullOrEmpty(expectedSnapshotId))
        {
            string current = Sts2CoopStateEndpoint.SnapshotIdOf(available);
            if (!string.Equals(expectedSnapshotId, current, StringComparison.Ordinal))
            {
                return "stale snapshot: the set of legal actions changed since you read it";
            }
        }

        AiTeammateAvailableAction? action = available
            .FirstOrDefault(candidate => string.Equals(candidate.ActionId, actionId, StringComparison.Ordinal));
        if (action == null)
        {
            return $"no such action right now: {actionId}";
        }

        _isExecutingAction = true;
        try
        {
            AiActionExecutionResult result = await action.ExecuteAsync();

            if (!string.IsNullOrEmpty(action.DeduplicationKey))
            {
                _lastDeduplicationKey = action.DeduplicationKey;
            }

            if (!result.HasTrackedGameAction)
            {
                Log.Info($"[Sts2Coop] server executed non-tracked actionId={action.ActionId}");
                return null;
            }

            BeginIssuedActionSettlement(action, result);
            Log.Info($"[Sts2Coop] server issued actionId={action.ActionId}; waiting for completion");

            Task completion = result.GameAction!.CompletionTask;
            if (await Task.WhenAny(completion, Task.Delay(ActionSettleTimeout)) != completion)
            {
                return $"action did not settle within {ActionSettleTimeout.TotalSeconds:0}s: {actionId}";
            }

            await completion;
            return null;
        }
        catch (Exception e)
        {
            return $"{e.GetType().Name}: {e.Message}";
        }
        finally
        {
            _isExecutingAction = false;
        }
    }
}

using System.Threading.Tasks;
using MegaCrit.Sts2.Core.Entities.Players;

namespace Sts2LlmCoop;

internal sealed class ActionRequestDto
{
    public string ActionId { get; set; } = "";

    /// Rejected if the legal actions changed since. Empty skips the check (for manual
    /// testing).
    public string SnapshotId { get; set; } = "";
}

internal static class Sts2CoopActionEndpoint
{
    /// Null on success, otherwise the reason. Called on the HTTP thread; issues on the
    /// main thread and waits here.
    public static Task<string?> ExecuteAsync(ActionRequestDto req)
    {
        if (string.IsNullOrWhiteSpace(req.ActionId))
        {
            return Task.FromResult<string?>("actionId required");
        }

        return Sts2CoopMainThread.RunAsync(() =>
        {
            Player? me = Sts2CoopStateEndpoint.FindAiPlayer();
            if (me == null)
            {
                return Task.FromResult<string?>("no ai player in this run");
            }

            if (!AiTeammateDummyController.TryGetControllerFor(me.NetId, out AiTeammateDummyController controller))
            {
                return Task.FromResult<string?>("ai player has no controller");
            }

            return controller.ExecuteForServerAsync(req.ActionId, req.SnapshotId);
        });
    }
}

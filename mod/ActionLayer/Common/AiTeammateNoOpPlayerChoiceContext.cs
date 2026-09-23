using System.Threading.Tasks;
using MegaCrit.Sts2.Core.Entities.Multiplayer;
using MegaCrit.Sts2.Core.GameActions.Multiplayer;

namespace Sts2LlmCoop;

internal sealed class AiTeammateNoOpPlayerChoiceContext : PlayerChoiceContext
{
    public override Task SignalPlayerChoiceBegun(PlayerChoiceOptions options)
    {
        return Task.CompletedTask;
    }

    public override Task SignalPlayerChoiceEnded()
    {
        return Task.CompletedTask;
    }
}

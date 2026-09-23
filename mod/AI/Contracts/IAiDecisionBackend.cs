using System.Threading;
using System.Threading.Tasks;

namespace STS2AiTeammate;

internal interface IAiDecisionBackend
{
    Task<AiDecisionResult> DecideAsync(AiDecisionRequest request, CancellationToken ct);
}
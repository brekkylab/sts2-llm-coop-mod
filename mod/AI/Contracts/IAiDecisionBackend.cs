using System.Threading;
using System.Threading.Tasks;

namespace Sts2LlmCoop;

internal interface IAiDecisionBackend
{
    Task<AiDecisionResult> DecideAsync(AiDecisionRequest request, CancellationToken ct);
}
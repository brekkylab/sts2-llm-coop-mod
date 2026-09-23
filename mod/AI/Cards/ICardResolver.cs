using MegaCrit.Sts2.Core.Models;

namespace Sts2LlmCoop;

internal interface ICardResolver
{
    ResolvedCardView Resolve(CardModel liveCard, string cardInstanceId);
}

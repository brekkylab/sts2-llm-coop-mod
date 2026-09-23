using MegaCrit.Sts2.Core.Models;

namespace STS2AiTeammate;

internal interface ICardResolver
{
    ResolvedCardView Resolve(CardModel liveCard, string cardInstanceId);
}

using System.Collections.Generic;

namespace STS2AiTeammate;

internal sealed class CardSemanticProfile
{
    public IReadOnlyList<NormalizedEffectDescriptor> Effects { get; init; } = [];
}

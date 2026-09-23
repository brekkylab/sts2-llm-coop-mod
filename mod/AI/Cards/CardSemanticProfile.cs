using System.Collections.Generic;

namespace Sts2LlmCoop;

internal sealed class CardSemanticProfile
{
    public IReadOnlyList<NormalizedEffectDescriptor> Effects { get; init; } = [];
}

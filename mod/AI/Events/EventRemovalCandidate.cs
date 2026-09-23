using System.Collections.Generic;
using MegaCrit.Sts2.Core.Models;

namespace Sts2LlmCoop;

internal sealed class EventRemovalCandidate
{
    public required string CardId { get; init; }

    public required string Name { get; init; }

    public required double BurdenScore { get; init; }

    public required CardModel RuntimeCard { get; init; }

    public IReadOnlyList<string> Reasons { get; init; } = [];

    public string Describe()
    {
        return $"card={CardId} name={Name} burden={BurdenScore:F1} reasons=[{string.Join("; ", Reasons)}]";
    }
}

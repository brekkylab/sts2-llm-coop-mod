using System.Collections.Generic;
using System.Text.Json.Serialization;

namespace Sts2LlmCoop;

internal sealed class AiDecisionRequest
{
    [JsonPropertyName("request_id")]
    public required string RequestId { get; init; }

    [JsonPropertyName("snapshot_id")]
    public required string SnapshotId { get; init; }

    [JsonPropertyName("actor_id")]
    public required string ActorId { get; init; }

    [JsonPropertyName("legal_actions")]
    public required List<AiLegalActionOption> LegalActions { get; init; }
}

internal sealed class AiDecisionResult
{
    [JsonPropertyName("chosen_action_id")]
    public required string ChosenActionId { get; init; }

    [JsonPropertyName("ranked_action_ids")]
    public List<string> RankedActionIds { get; init; } = [];

    /// A sequence ("this, then that"), unlike `RankedActionIds` ("this, else that").
    /// Empty means just `ChosenActionId`.
    [JsonPropertyName("planned_action_ids")]
    public List<string> PlannedActionIds { get; init; } = [];

    /// Play without waiting for the partner.
    [JsonPropertyName("act_now")]
    public bool ActNow { get; init; }

    [JsonPropertyName("reason")]
    public string? Reason { get; init; }

    [JsonPropertyName("reasoning")]
    public string? Reasoning
    {
        get => Reason;
        init => Reason = value;
    }
}
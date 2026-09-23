using System;
using System.Threading;
using System.Threading.Tasks;

namespace Sts2LlmCoop;

/// Holds the game's open question until the bridge answers. One at a time: the game
/// waits on a single decision, and overlapping ones couldn't be matched to answers.
internal static class Sts2CoopDecisionEndpoint
{
    private sealed class Pending
    {
        public required string RequestId { get; init; }
        public required string SnapshotId { get; init; }
        public required DateTime DeadlineUtc { get; init; }
        public required TaskCompletionSource<DecisionReplyDto> Completion { get; init; }
    }

    private static Pending? _pending;
    private static readonly object Gate = new();

    /// Ends with TaskCanceledException on timeout or cancel. Callers hold the Task
    /// rather than awaiting it inside a frame.
    public static async Task<DecisionReplyDto> AskAsync(
        string snapshotId, TimeSpan budget, CancellationToken ct)
    {
        var pending = new Pending
        {
            RequestId = Guid.NewGuid().ToString("N")[..8],
            SnapshotId = snapshotId,
            DeadlineUtc = DateTime.UtcNow + budget,
            Completion = new TaskCompletionSource<DecisionReplyDto>(
                TaskCreationOptions.RunContinuationsAsynchronously),
        };

        lock (Gate) { _pending = pending; }
        try
        {
            using var timeout = CancellationTokenSource.CreateLinkedTokenSource(ct);
            timeout.CancelAfter(budget);
            return await pending.Completion.Task.WaitAsync(timeout.Token);
        }
        finally
        {
            lock (Gate) { if (ReferenceEquals(_pending, pending)) { _pending = null; } }
        }
    }

    /// For `GET /decision/pending`.
    public static PendingDto Peek()
    {
        lock (Gate)
        {
            if (_pending is not { } p) { return new PendingDto { Pending = false }; }
            int left = (int)Math.Max(0, (p.DeadlineUtc - DateTime.UtcNow).TotalMilliseconds);
            return new PendingDto
            {
                Pending = true,
                RequestId = p.RequestId,
                SnapshotId = p.SnapshotId,
                DeadlineMs = left,
            };
        }
    }

    /// For `POST /decision`. Rejects late answers and answers to another question.
    public static string? Answer(DecisionReplyDto reply)
    {
        lock (Gate)
        {
            if (_pending is not { } p) { return "nothing is waiting for a decision"; }
            if (!string.Equals(p.RequestId, reply.RequestId, StringComparison.Ordinal))
            {
                return $"stale requestId: waiting on {p.RequestId}";
            }
            return p.Completion.TrySetResult(reply) ? null : "that decision already resolved";
        }
    }
}

internal sealed class PendingDto
{
    public bool Pending { get; set; }
    public string? RequestId { get; set; }

    /// Action-set fingerprint, comparable with `snapshotId` from /state/combat.
    public string? SnapshotId { get; set; }
    public int DeadlineMs { get; set; }
}

internal sealed class DecisionReplyDto
{
    public string RequestId { get; set; } = "";
    public string ActionId { get; set; } = "";

    /// The turn's plan in order; `ActionId` is its first step. Absent from older
    /// clients, where `ActionId` is the whole plan.
    public List<string>? ActionIds { get; set; }

    /// Optional line shown in the speech bubble.
    public string? Say { get; set; }

    /// Play without waiting for the partner. The bridge currently always sends false;
    /// the approve button covers this case.
    public bool Now { get; set; }
}

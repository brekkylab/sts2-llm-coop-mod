using System;

namespace Sts2LlmCoop;

/// Agent state as reported by the bridge; the game only draws it.
internal static class Sts2CoopAgentStatus
{
    private static readonly object Gate = new();
    private static string _state = "idle";
    private static string? _bubble;
    private static DateTime _changedAtUtc = DateTime.MinValue;

    public static (string State, string? Bubble, DateTime ChangedAtUtc) Snapshot()
    {
        lock (Gate) { return (_state, _bubble, _changedAtUtc); }
    }

    public static void Set(string state, string? bubble)
    {
        lock (Gate)
        {
            // Keep the timestamp on repeats, or the thinking grace never elapses.
            if (!string.Equals(_state, state, StringComparison.Ordinal))
            {
                _state = state;
                _changedAtUtc = DateTime.UtcNow;
            }

            if (bubble != null)
            {
                _bubble = bubble;
            }
        }
    }
}

internal sealed class AgentStatusDto
{
    public string State { get; set; } = "idle";
    public string? Bubble { get; set; }
}

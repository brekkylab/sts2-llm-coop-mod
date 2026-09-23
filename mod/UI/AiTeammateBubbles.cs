using System;
using Godot;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Nodes.Vfx;

namespace Sts2LlmCoop;

/// Shows the agent's state with the game's own thought/speech bubbles.
///
/// `Compute()` decides what should be on screen; `Apply()` reconciles the nodes
/// with that. There is no explicit "clear" path: when nothing is wanted, the
/// bubble goes away. (Clearing at each exit point was tried and got wrong five
/// different ways.)
internal static class AiTeammateBubbles
{
    /// Short decisions show no thought bubble; a flash is worse than nothing.
    private static readonly TimeSpan ThinkingGrace = TimeSpan.FromMilliseconds(800);

    private const float SpeechLift = 90f;

    /// Safety net after a fade, in case the game doesn't free the node itself.
    private static readonly TimeSpan FadeGrace = TimeSpan.FromSeconds(1.5);

    /// 60 cut plans off mid-sentence.
    private const int SpeechLimit = 120;

    private enum Kind
    {
        None,
        Thinking,
        Speech,
    }

    /// `Text` is set only for `Speech`.
    private readonly record struct Want(Kind Kind, string? Text);

    private static NThoughtBubbleVfx? _thinking;
    private static NSpeechBubbleVfx? _speech;

    /// Text of the bubble currently on screen (not the last one seen), so it goes
    /// away with the node.
    private static string? _shown;

    public static void Tick()
    {
        DumpTreeOnce();
        Apply(Compute());
    }

    public static void Clear()
    {
        Apply(new Want(Kind.None, null));
    }

    /// Reads game state only.
    private static Want Compute()
    {
        Player? me = Sts2CoopStateEndpoint.FindAiPlayer();
        if (me?.Creature == null)
        {
            return new Want(Kind.None, null);
        }

        // Victory and abandon arrive here with the player alive and the session
        // intact; without this the bubble follows onto the reward screen.
        var combat = MegaCrit.Sts2.Core.Combat.CombatManager.Instance;
        if (combat == null || !combat.IsInProgress)
        {
            return new Want(Kind.None, null);
        }

        (string state, string? bubble, DateTime changedAt) = Sts2CoopAgentStatus.Snapshot();

        if (state == "thinking")
        {
            return DateTime.UtcNow - changedAt >= ThinkingGrace
                ? new Want(Kind.Thinking, null)
                : new Want(Kind.None, null);
        }

        // The speech bubble lives exactly as long as the held plan.
        return bubble != null && IsHoldingPlan()
            ? new Want(Kind.Speech, bubble)
            : new Want(Kind.None, null);
    }

    private static bool IsHoldingPlan()
    {
        AiTeammateSessionState? session = AiTeammateSessionRegistry.Current;
        if (session == null)
        {
            return false;
        }

        foreach (AiTeammateDummyController c in session.AiControllers.Values)
        {
            if (c.IsHoldingPlan)
            {
                return true;
            }
        }

        return false;
    }

    private static void Apply(Want want)
    {
        if (want.Kind != Kind.Thinking && _thinking != null)
        {
            Free(_thinking);
            _thinking = null;
        }

        // Same text keeps the node, or the pop-in animation restarts every frame.
        bool speechStale = want.Kind != Kind.Speech
            || !string.Equals(want.Text, _shown, StringComparison.Ordinal);
        if (speechStale && _speech != null)
        {
            Fade(_speech);
            _speech = null;
            _shown = null;
        }

        Player? me = Sts2CoopStateEndpoint.FindAiPlayer();
        if (me?.Creature == null)
        {
            return;
        }

        if (want.Kind == Kind.Thinking && _thinking == null)
        {
            ShowThinking(me);
        }
        else if (want.Kind == Kind.Speech && _speech == null && want.Text != null)
        {
            ShowSpeech(me, want.Text);
            _shown = want.Text;
        }
    }

    private static void ShowThinking(Player me)
    {
        try
        {
            // No secondsToDisplay: stays until we remove it.
            _thinking = NThoughtBubbleVfx.Create("…", me.Creature, null);
            Attach(_thinking);
        }
        catch (Exception e)
        {
            Log.Warn($"[Sts2Coop] thought bubble failed: {e.Message}");
            _thinking = null;
        }
    }

    private static void ShowSpeech(Player me, string text)
    {
        try
        {
            string shown = text.Length > SpeechLimit
                ? text[..(SpeechLimit - 1)] + "…"
                : text;

            // Stays until the plan is played, however long the partner thinks. The
            // 300 s cap only guards against it never being removed.
            NSpeechBubbleVfx bubble = NSpeechBubbleVfx.Create(shown, me.Creature, 300.0, VfxColor.White);
            Attach(bubble);
            _speech = bubble;

            // In co-op the default position covers the other character.
            bubble.Position += new Vector2(0, -SpeechLift);
        }
        catch (Exception e)
        {
            Log.Warn($"[Sts2Coop] speech bubble failed: {e.Message}");
            _speech = null;
        }
    }

    /// If bubbles ever land off screen, find the combat root in the
    /// `DumpTreeOnce` log and attach there instead of `CurrentScene`.
    private static void Attach(Node node)
    {
        Node? parent = (Engine.GetMainLoop() as SceneTree)?.CurrentScene;
        if (parent == null)
        {
            Log.Warn("[Sts2Coop] no current scene; bubble dropped");
            return;
        }

        parent.AddChild(node);
    }

    /// Tells the bubble its display time is up so the game fades it out its own way,
    /// instead of an abrupt `QueueFree`.
    private static void Fade(Node? node)
    {
        if (node == null || !GodotObject.IsInstanceValid(node)) { return; }

        try
        {
            if (node is NSpeechBubbleVfx speech) { speech.SecondsToDisplay = 0; }
        }
        catch (Exception) { /* field renamed; the timer below frees it */ }

        FreeAfter(node, FadeGrace);
    }

    /// The timer belongs to the SceneTree, not the node, so it still fires after a
    /// scene change.
    private static void FreeAfter(Node node, TimeSpan delay)
    {
        if (Engine.GetMainLoop() is not SceneTree tree)
        {
            Free(node);
            return;
        }

        try
        {
            SceneTreeTimer timer = tree.CreateTimer(delay.TotalSeconds);
            timer.Timeout += () => Free(node);
        }
        catch (Exception)
        {
            Free(node);
        }
    }

    private static void Free(Node? node)
    {
        if (node == null || !GodotObject.IsInstanceValid(node)) { return; }
        try { node.QueueFree(); }
        catch (Exception) { /* already out of the tree */ }
    }

    /// One-time scene tree dump: the only way to find the right parent for `Attach`.
    private static bool _dumped;

    private static void DumpTreeOnce()
    {
        if (_dumped) { return; }
        _dumped = true;

        Node? root = (Engine.GetMainLoop() as SceneTree)?.CurrentScene;
        if (root == null) { Log.Warn("[Sts2Coop] no current scene"); return; }

        void Walk(Node n, int depth)
        {
            if (depth > 3) { return; }
            Log.Info($"[Sts2Coop][TREE] {new string(' ', depth * 2)}{n.Name} : {n.GetType().Name}");
            foreach (Node c in n.GetChildren()) { Walk(c, depth + 1); }
        }

        Walk(root, 0);
    }
}

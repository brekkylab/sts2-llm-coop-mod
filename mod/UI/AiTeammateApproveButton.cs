using System;
using Godot;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Nodes.Vfx;

namespace Sts2LlmCoop;

/// "Act now" button shown while the AI holds a plan. Replaces asking the model to
/// detect "go first" in chat, which it misread.
///
/// Same shape as the bubbles: `Tick` recomputes whether it should exist.
internal static class AiTeammateApproveButton
{
    /// Offset from the speech bubble's tail (`GetCreatureSpeechPosition` is the
    /// mouth). Tuned by eye; bottom-centre is the hand, so not there.
    private const float LiftAboveTail = 10f;

    private const float NudgeRight = 5f;

    private static Button? _button;

    /// Follows the talk language setting.
    private static string Label => Sts2CoopConfig.Language switch
    {
        Sts2CoopConfig.TalkLanguage.English => "Go now",
        Sts2CoopConfig.TalkLanguage.Japanese => "今すぐ",
        _ => "지금 해",
    };

    /// Every frame, next to `AiTeammateBubbles.Tick()`.
    public static void Tick()
    {
        AiTeammateDummyController? waiting = FindWaitingController();
        if (waiting == null)
        {
            Clear();
            return;
        }

        if (_button == null)
        {
            Show(waiting);
        }
        else
        {
            Place();
        }
    }

    /// The controller waiting for approval, or null.
    private static AiTeammateDummyController? FindWaitingController()
    {
        AiTeammateSessionState? session = AiTeammateSessionRegistry.Current;
        if (session == null)
        {
            return null;
        }

        foreach (AiTeammateDummyController c in session.AiControllers.Values)
        {
            if (c.IsWaitingForApproval)
            {
                return c;
            }
        }

        return null;
    }

    private static void Show(AiTeammateDummyController controller)
    {
        try
        {
            var button = new Button { Text = Label };

            button.Pressed += () =>
            {
                controller.ApproveHeldPlan();
                // Next Tick would remove it anyway; this prevents a double press
                // in between.
                Clear();
            };

            Node? parent = (Engine.GetMainLoop() as SceneTree)?.CurrentScene;
            if (parent == null)
            {
                Log.Warn("[Sts2Coop] no current scene; approve button dropped");
                return;
            }

            parent.AddChild(button);
            _button = button;
            Place();
        }
        catch (Exception e)
        {
            Log.Warn($"[Sts2Coop] approve button failed: {e.Message}");
            _button = null;
        }
    }

    /// Recomputed every frame to follow the creature.
    private static void Place()
    {
        if (_button == null || !GodotObject.IsInstanceValid(_button))
        {
            return;
        }

        try
        {
            Player? me = Sts2CoopStateEndpoint.FindAiPlayer();
            if (me?.Creature == null)
            {
                return;
            }

            Vector2 world = NSpeechBubbleVfx.GetCreatureSpeechPosition(me.Creature);

            // World to screen: a Control, unlike the Node2D bubble, ignores the camera.
            Vector2 anchor = _button.GetViewport().GetCanvasTransform() * world;
            _button.Position = anchor
                + new Vector2(-_button.Size.X / 2f + NudgeRight, -LiftAboveTail);
        }
        catch (Exception)
        {
            /* out of combat or creature left the tree; retry next frame */
        }
    }

    public static void Clear()
    {
        if (_button == null)
        {
            return;
        }

        Button b = _button;
        _button = null;
        if (!GodotObject.IsInstanceValid(b))
        {
            return;
        }

        try { b.QueueFree(); }
        catch (Exception) { /* already out of the tree */ }
    }
}

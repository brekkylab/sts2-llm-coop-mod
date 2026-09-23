using System.Diagnostics.CodeAnalysis;
using System.Reflection;
using BaseLib.Config;
using HarmonyLib;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Modding;
using Godot;

namespace Sts2LlmCoop;

[ModInitializer(nameof(Initialize))]
[SuppressMessage("ReSharper", "UnusedType.Global")]
public static class ModEntry
{
    public const string ModId = "Sts2LlmCoop";

    private static readonly Lock Lock = new();
    private static bool _initialized;
    private static Harmony? _harmony;

    public static void Initialize()
    {
        lock (Lock)
        {
            if (_initialized) return;
            _initialized = true;
        }

        Log.Info($"[{ModId}] ============================================================");
        Log.Info($"[{ModId}] Initializing {ModId}");

        _harmony = new Harmony(ModId);
        try
        {
            _harmony.PatchAll(Assembly.GetExecutingAssembly());
            Log.Info($"[{ModId}] Harmony patches installed successfully");
        }
        catch (Exception e)
        {
            Log.Error($"[{ModId}] Harmony patching failed: {e.GetType().Name}: {e.Message}");
            if (e.InnerException != null)
                Log.Error($"[{ModId}]   \u2192 inner: {e.InnerException.GetType().Name}: {e.InnerException.Message}");
        }

        // BaseLib builds the settings screen from this. Must precede the server,
        // which reads the config.
        try
        {
            ModConfigRegistry.Register(ModId, new Sts2CoopConfig());
            Log.Info($"[{ModId}] mod config registered");
        }
        catch (Exception e)
        {
            Log.Error($"[{ModId}] mod config registration failed: {e.GetType().Name}: {e.Message}");
        }

        // Local HTTP for state and actions. A failure here must not stop the mod
        // from working as the original did.
        try
        {
            Sts2CoopHttpServer.Start((SceneTree)Engine.GetMainLoop());
        }
        catch (Exception e)
        {
            Log.Error($"[{ModId}] coop server failed to start: {e.GetType().Name}: {e.Message}");
        }

        try
        {
            BridgeProcess.StartIfConfigured();
        }
        catch (Exception e)
        {
            Log.Error($"[{ModId}] bridge autostart failed: {e.GetType().Name}: {e.Message}");
        }

        // Godot has never been seen to fire this, even on a clean quit. The real
        // cleanup is the bridge exiting on its own via STS2_EXIT_WITH_GAME; this
        // only helps if it ever does fire.
        AppDomain.CurrentDomain.ProcessExit += static (_, _) => BridgeProcess.Stop();

        Log.Info($"[{ModId}] Initialization complete");
        Log.Info($"[{ModId}] ============================================================");
    }
}
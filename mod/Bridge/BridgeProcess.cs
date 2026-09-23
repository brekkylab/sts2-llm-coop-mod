using System.Diagnostics;
using System.Linq;
using System.Reflection;
using MegaCrit.Sts2.Core.Logging;

namespace Sts2LlmCoop;

/// Ties the bridge's lifetime to the game's.
///
/// The bridge exits by itself once the game is unreachable for 5 s
/// (STS2_EXIT_WITH_GAME), however the game died. If even the bridge is killed and
/// leaves a FUSE mount behind, the next `run-bridge.sh` cleans it up. `Stop()` is a
/// convenience on top; Godot doesn't seem to fire the exit hook (see ModEntry).
internal static class BridgeProcess
{
    private static Process? _proc;

    /// Written inside agent/, not /tmp, where a pre-planted symlink could redirect it.
    private const string LogName = "bridge.log";

    /// Baked in at build time by Sts2LlmCoop.csproj (absolute path to the sibling
    /// agent/). Once copied into the game folder the mod can't find it otherwise.
    private static string AgentDir()
    {
        return Assembly.GetExecutingAssembly()
            .GetCustomAttributes<AssemblyMetadataAttribute>()
            .FirstOrDefault(a => a.Key == "AgentDir")?.Value ?? "";
    }

    /// Never blocks the game: without a bridge the AI falls back to the heuristic.
    public static void StartIfConfigured()
    {
        if (!Sts2CoopConfig.AutoStart)
        {
            return;
        }

        string dir = AgentDir();
        if (dir.Length == 0)
        {
            Log.Warn("[Sts2Coop] the build did not record where the agent lives");
            return;
        }

        string script = System.IO.Path.Combine(dir, "run-bridge.sh");
        if (!System.IO.File.Exists(script))
        {
            Log.Warn($"[Sts2Coop] no run-bridge.sh at {script}");
            return;
        }

        try
        {
            // Output goes to a file. As a pipe to the game it would block the bridge's
            // next println! once the game dies, and the bridge would hang without
            // noticing the game is gone.
            var info = new ProcessStartInfo
            {
                // sh runs the script without +x and does the redirect, which
                // ProcessStartInfo can't.
                FileName = "/bin/sh",
                WorkingDirectory = dir,
                UseShellExecute = false,
            };
            string log = System.IO.Path.Combine(dir, LogName);
            info.ArgumentList.Add("-c");
            info.ArgumentList.Add($"exec '{script}' >> '{log}' 2>&1");

            // The real safety net: Stop() never runs if the game is killed.
            info.Environment["STS2_EXIT_WITH_GAME"] = "1";

            _proc = Process.Start(info);
            Log.Info($"[Sts2Coop] bridge started (pid={_proc?.Id}), log at {log}");
        }
        catch (Exception e)
        {
            Log.Warn($"[Sts2Coop] bridge failed to start: {e.GetType().Name}: {e.Message}");
            _proc = null;
        }
    }

    /// Gives the bridge time to stop its console server and unmount before killing it.
    public static void Stop()
    {
        Process? p = _proc;
        _proc = null;
        if (p == null || p.HasExited)
        {
            return;
        }

        try
        {
            Log.Info("[Sts2Coop] stopping the bridge");
            p.Kill(entireProcessTree: true);
            p.WaitForExit(3000);
        }
        catch (Exception e)
        {
            Log.Warn($"[Sts2Coop] bridge stop failed: {e.GetType().Name}: {e.Message}");
        }
    }
}

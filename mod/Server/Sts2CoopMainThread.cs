using System;
using System.Collections.Concurrent;
using System.Threading.Tasks;
using Godot;

namespace Sts2LlmCoop;

/// Game state is main-thread only; HTTP handlers run elsewhere, so state access goes
/// through this queue.
internal static class Sts2CoopMainThread
{
    private static readonly ConcurrentQueue<Action> Queue = new();
    private static int _mainThreadId = -1;

    public static void Attach(SceneTree tree)
    {
        _mainThreadId = System.Environment.CurrentManagedThreadId;
        tree.Connect(SceneTree.SignalName.ProcessFrame, Callable.From(Pump));
    }

    /// The pump runs on ProcessFrame, so blocking the main thread stalls the queue.
    public static bool IsMainThread() => System.Environment.CurrentManagedThreadId == _mainThreadId;

    /// Capped per frame so a backlog can't eat a whole frame.
    private static void Pump()
    {
        int processed = 0;
        while (processed < 10 && Queue.TryDequeue(out Action? action))
        {
            try
            {
                action();
            }
            catch (Exception e)
            {
                GD.PrintErr($"[Sts2Coop] main-thread action failed: {e}");
            }
            processed++;
        }
    }

    public static Task<T> Run<T>(Func<T> func)
    {
        var tcs = new TaskCompletionSource<T>();
        Queue.Enqueue(() =>
        {
            try { tcs.SetResult(func()); }
            catch (Exception e) { tcs.SetException(e); }
        });
        return tcs.Task;
    }

    /// Starts on the main thread and hands the wait to the caller; an action settles
    /// over several frames.
    public static Task<T> RunAsync<T>(Func<Task<T>> func)
    {
        var tcs = new TaskCompletionSource<Task<T>>();
        Queue.Enqueue(() =>
        {
            try { tcs.SetResult(func()); }
            catch (Exception e) { tcs.SetException(e); }
        });
        return tcs.Task.Unwrap();
    }
}

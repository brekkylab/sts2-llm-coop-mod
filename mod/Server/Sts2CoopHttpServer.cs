using System;
using System.Net;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Godot;

namespace Sts2LlmCoop;

/// Local HTTP over the game's combat state and actions. Knows nothing about agents
/// or LLMs.
internal static class Sts2CoopHttpServer
{
    private const int Port = 15527;

    private static readonly JsonSerializerOptions JsonOpts = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        PropertyNameCaseInsensitive = true,
    };

    private static HttpListener? _listener;
    private static Thread? _thread;

    /// No server, no bridge: decides whether the bridge backend is created.
    public static bool IsRunning => _listener?.IsListening == true;

    public static void Start(SceneTree tree)
    {
        Sts2CoopMainThread.Attach(tree);
        _listener = new HttpListener();
        _listener.Prefixes.Add($"http://127.0.0.1:{Port}/");
        _listener.Start();
        _thread = new Thread(Loop) { IsBackground = true, Name = "Sts2CoopHttp" };
        _thread.Start();
        GD.Print($"[Sts2Coop] listening on 127.0.0.1:{Port}");
    }

    public static void Stop()
    {
        try { _listener?.Stop(); }
        catch (Exception) { /* already closed during shutdown */ }
        _listener = null;
    }

    private static void Loop()
    {
        while (_listener?.IsListening == true)
        {
            HttpListenerContext context;
            try { context = _listener.GetContext(); }
            catch (Exception) { return; }   // from Stop()
            ThreadPool.QueueUserWorkItem(_ => _ = Handle(context));
        }
    }

    private static async Task Handle(HttpListenerContext ctx)
    {
        try
        {
            string path = ctx.Request.Url?.AbsolutePath ?? "/";
            string method = ctx.Request.HttpMethod;

            if (path == "/state/combat" && method == "GET")
            {
                CombatStateDto dto = await Sts2CoopMainThread.Run(Sts2CoopStateEndpoint.BuildCombatState);
                await WriteJson(ctx.Response, 200, dto);
                return;
            }

            if (path == "/action" && method == "POST")
            {
                using var reader = new System.IO.StreamReader(ctx.Request.InputStream);
                string body = await reader.ReadToEndAsync();
                ActionRequestDto? req;
                try { req = JsonSerializer.Deserialize<ActionRequestDto>(body, JsonOpts); }
                catch (JsonException e)
                {
                    await WriteJson(ctx.Response, 400, new { ok = false, error = $"bad json: {e.Message}" });
                    return;
                }

                if (req == null)
                {
                    await WriteJson(ctx.Response, 400, new { ok = false, error = "empty body" });
                    return;
                }

                string? err = await Sts2CoopActionEndpoint.ExecuteAsync(req);
                if (err == null) { await WriteJson(ctx.Response, 200, new { ok = true }); }
                else { await WriteJson(ctx.Response, 409, new { ok = false, error = err }); }
                return;
            }

            // These two skip Sts2CoopMainThread: they don't touch game state, and
            // queueing them behind a waiting decision would deadlock the answer.
            if (path == "/decision/pending" && method == "GET")
            {
                await WriteJson(ctx.Response, 200, Sts2CoopDecisionEndpoint.Peek());
                return;
            }

            if (path == "/decision" && method == "POST")
            {
                using var reader = new System.IO.StreamReader(ctx.Request.InputStream);
                string body = await reader.ReadToEndAsync();
                DecisionReplyDto? reply;
                try { reply = JsonSerializer.Deserialize<DecisionReplyDto>(body, JsonOpts); }
                catch (JsonException e)
                {
                    await WriteJson(ctx.Response, 400, new { ok = false, error = $"bad json: {e.Message}" });
                    return;
                }

                if (reply == null)
                {
                    await WriteJson(ctx.Response, 400, new { ok = false, error = "empty body" });
                    return;
                }

                string? decisionErr = Sts2CoopDecisionEndpoint.Answer(reply);
                if (decisionErr == null) { await WriteJson(ctx.Response, 200, new { ok = true }); }
                else { await WriteJson(ctx.Response, 409, new { ok = false, error = decisionErr }); }
                return;
            }


            // No game state either.
            if (path == "/agent/status" && method == "POST")
            {
                using var reader = new System.IO.StreamReader(ctx.Request.InputStream);
                string body = await reader.ReadToEndAsync();
                AgentStatusDto? dto;
                try { dto = JsonSerializer.Deserialize<AgentStatusDto>(body, JsonOpts); }
                catch (JsonException e)
                {
                    await WriteJson(ctx.Response, 400, new { ok = false, error = $"bad json: {e.Message}" });
                    return;
                }

                if (dto == null)
                {
                    await WriteJson(ctx.Response, 400, new { ok = false, error = "empty body" });
                    return;
                }

                Sts2CoopAgentStatus.Set(dto.State, dto.Bubble);
                await WriteJson(ctx.Response, 200, new { ok = true });
                return;
            }

            // Only drops the held plan; carries no answer. The next Tick re-asks through
            // the normal path, so budget, call cap and heuristic fallback all apply.
            if (path == "/replan" && method == "POST")
            {
                bool dropped = Sts2CoopStateEndpoint.FindAiPlayer() is { } me
                    && AiTeammateDummyController.TryGetControllerFor(
                        me.NetId, out AiTeammateDummyController controller)
                    && controller.DropHeldPlan();

                await WriteJson(ctx.Response, 200, new { ok = true, dropped });
                return;
            }

            // BaseLib config values are static, safe from any thread.
            if (path == "/config" && method == "GET")
            {
                await WriteJson(ctx.Response, 200, new CoopConfigDto
                {
                    Language = Sts2CoopConfig.LanguageName,
                    DecisionBudgetSeconds = Sts2CoopConfig.DecisionBudgetSeconds,
                    CallsPerCombat = Sts2CoopConfig.CallsPerCombat,
                });
                return;
            }

            await WriteJson(ctx.Response, 404, new { error = "not found", path });
        }
        catch (Exception e)
        {
            try { await WriteJson(ctx.Response, 500, new { error = e.Message }); }
            catch (Exception) { /* response stream already closed */ }
        }
    }

    internal static async Task WriteJson(HttpListenerResponse res, int status, object body)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(JsonSerializer.Serialize(body, JsonOpts));
        res.StatusCode = status;
        res.ContentType = "application/json; charset=utf-8";
        res.ContentLength64 = bytes.Length;
        await res.OutputStream.WriteAsync(bytes);
        res.Close();
    }
}

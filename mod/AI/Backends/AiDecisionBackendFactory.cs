namespace Sts2LlmCoop;

internal static class AiDecisionBackendFactory
{
    /// The heuristic: fallback in combat, the only backend outside it.
    public static IAiDecisionBackend CreateDefault()
    {
        return new DeterministicCombatDecisionBackend(new DeterministicDecisionBackend());
    }

    /// Null unless the server is running.
    public static BridgeDecisionBackend? CreateBridge()
    {
        return Sts2CoopHttpServer.IsRunning ? new BridgeDecisionBackend() : null;
    }
}

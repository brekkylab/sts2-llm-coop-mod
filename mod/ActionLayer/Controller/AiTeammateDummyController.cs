using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using MegaCrit.Sts2.Core.Entities.Actions;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.GameActions;
using MegaCrit.Sts2.Core.GameActions.Multiplayer;
using MegaCrit.Sts2.Core.Helpers;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Runs;

namespace Sts2LlmCoop;

internal sealed partial class AiTeammateDummyController
{
    private static readonly IAiDecisionBackend DecisionBackend = AiDecisionBackendFactory.CreateDefault();
    private static readonly TimeSpan IdleTickInterval = TimeSpan.FromMilliseconds(250);
    private static readonly TimeSpan EndTurnGraceInterval = TimeSpan.FromMilliseconds(400);
    private static readonly TimeSpan ActionSettleTimeout = TimeSpan.FromMilliseconds(5000);
    private static readonly TimeSpan QueueSettleTimeout = TimeSpan.FromMilliseconds(2500);
    private static readonly TimeSpan PostSettleGraceInterval = TimeSpan.FromMilliseconds(500);
    private static readonly TimeSpan MaxInitialCombatDecisionStagger = TimeSpan.FromMilliseconds(200);

    private DateTime _nextDecisionAtUtc = DateTime.MinValue;
    private bool _isExecutingAction;
    private string? _pendingEndTurnActionId;
    private string? _pendingEndTurnActionSetFingerprint;
    private DateTime _pendingEndTurnCommitAtUtc = DateTime.MinValue;
    private string? _lastDeduplicationKey;
    private int _lastCompletedEndTurnRound = -1;
    private int _lastCombatRoundWithInitialStagger = -1;
    private PendingIssuedActionSettlement? _pendingIssuedActionSettlement;

    // A decision asked of the bridge. Not awaited: a later Tick picks it up, the same
    // way _isExecutingAction and friends observe work that spans frames.
    private Task<AiDecisionResult>? _pendingDecision;
    private string? _pendingDecisionFingerprint;
    private CancellationTokenSource? _pendingDecisionCts;

    private BridgeDecisionBackend? _bridge;
    private bool _bridgeResolved;
    private bool _wasInCombat;

    // The round in which the bridge failed to answer; the rest of it goes to the
    // heuristic. A miss usually has a lasting cause (bridge down, model slow), so
    // asking again would burn another 20 s each time. Retried next round.
    private int _bridgeSilentInRound = -1;

    /// The bridge's plan for this turn, played front to back once the partner ends
    /// their turn. Kept apart from the heuristic's `_pendingEndTurn`, which expires
    /// on a timer and is cancelled when a better action appears; mixing the two
    /// caused an end-turn loop.
    private readonly Queue<string> _heldPlan = new();
    private string? _heldPlanFingerprint;

    /// Play the plan without waiting for the partner to end their turn. Set by the
    /// approve button (or the bridge's act_now flag).
    private bool _heldPlanActNow;

    /// Exposed in state so the bridge only asks for a replan when there is one.
    public bool IsHoldingPlan => _heldPlan.Count > 0;

    /// Whether the approve button should show.
    public bool IsWaitingForApproval => _heldPlan.Count > 0 && !_heldPlanActNow;

    /// The partner pressed "act now"; the next `Tick()` starts playing the plan.
    public void ApproveHeldPlan()
    {
        if (_heldPlan.Count == 0)
        {
            return;
        }

        Log.Info($"[Sts2Coop] Player={PlayerId} approved by hand");
        _heldPlanActNow = true;
    }

    /// Resolved lazily: a static initializer could run before the server is up.
    private BridgeDecisionBackend? Bridge
    {
        get
        {
            if (!_bridgeResolved)
            {
                _bridge = AiDecisionBackendFactory.CreateBridge();
                _bridgeResolved = true;
                Log.Info($"[Sts2Coop] Player={PlayerId} bridge backend: {(_bridge == null ? "off" : "on")}");
            }

            return _bridge;
        }
    }

    public AiTeammateDummyController(int slotIndex, ulong playerId, CharacterModel character)
    {
        SlotIndex = slotIndex;
        PlayerId = playerId;
        Character = character;
    }

    public int SlotIndex { get; }

    public ulong PlayerId { get; }

    public CharacterModel Character { get; }

    public void Tick()
    {
        if (TryGetControlledPlayer(out Player controlledPlayer, out RunState controlledRunState))
        {
            AiTeammateTestCombatHelper.ApplyOneHpEnemiesIfNeeded(controlledPlayer, controlledRunState);
        }

        if (_isExecutingAction || DateTime.UtcNow < _nextDecisionAtUtc)
        {
            return;
        }

        if (TryWaitForIssuedActionSettlement())
        {
            return;
        }

        IReadOnlyList<AiTeammateAvailableAction> availableActions = DiscoverAvailableActions();
        List<AiTeammateAvailableAction> decisionActions = BuildDecisionActions(availableActions);
        bool isCombatDecision = TryGetControlledPlayer(out Player promptPlayer, out _)
            && IsCombatDecisionWindow(promptPlayer);
        ResetCompletedEndTurnTrackingIfNeeded(isCombatDecision ? promptPlayer : null, isCombatDecision);
        string actionSetFingerprint = decisionActions.Count > 0
            ? BuildActionSetFingerprint(decisionActions)
            : string.Empty;

        ResetBridgeBudgetOnNewCombat();

        if (TryTakePendingDecision(decisionActions, actionSetFingerprint, isCombatDecision))
        {
            return;
        }

        // Before TryHandlePendingEndTurn: while a plan is held the heuristic's
        // delayed end turn must not run at all.
        if (TryHandleHeldPlan(decisionActions, actionSetFingerprint, isCombatDecision))
        {
            return;
        }

        if (TryApplyInitialCombatDecisionStagger(decisionActions, promptPlayer, isCombatDecision))
        {
            return;
        }

        if (TryHandlePendingEndTurn(decisionActions, actionSetFingerprint, isCombatDecision))
        {
            return;
        }

        if (decisionActions.Count == 0)
        {
            _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
            return;
        }

        if (ShouldSuppressRepeatedEndTurn(decisionActions, promptPlayer, isCombatDecision))
        {
            _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
            return;
        }

        Log.Info($"[AITeammate] Player={PlayerId} legal actions: {string.Join(", ", decisionActions.Select(static action => action.ActionId))}");

        // A single legal combat action is not worth a model call.
        if (isCombatDecision && decisionActions.Count == 1)
        {
            Log.Info($"[AITeammate] Player={PlayerId} only one action; skipping the bridge");
            CommitResolvedAction(decisionActions[0].ActionId, actionSetFingerprint, allowDelayedEndTurn: false);
            return;
        }

        AiDecisionRequest request = BuildDecisionRequest(decisionActions);

        if (isCombatDecision && ShouldScheduleDelayedEndTurn(decisionActions))
        {
            string endTurnActionId = decisionActions[0].ActionId;
            if (!string.Equals(_pendingEndTurnActionId, endTurnActionId, StringComparison.Ordinal))
            {
                _pendingEndTurnActionId = endTurnActionId;
                _pendingEndTurnActionSetFingerprint = actionSetFingerprint;
                _pendingEndTurnCommitAtUtc = DateTime.UtcNow + EndTurnGraceInterval;
                _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
                Log.Info($"[AITeammate] Player={PlayerId} scheduled delayed end turn actionId={endTurnActionId} graceMs={(int)EndTurnGraceInterval.TotalMilliseconds}");
            }

            return;
        }

        if (isCombatDecision && ShouldExecuteImmediateCombatDecision(decisionActions))
        {
            Log.Info($"[AITeammate] Player={PlayerId} using immediate combat decision path actionId={decisionActions[0].ActionId}");
        }

        ExecuteImmediateDecision(request, actionSetFingerprint, isCombatDecision);
    }

    /// -1 outside combat.
    private int CurrentCombatRound()
    {
        return TryGetControlledPlayer(out Player player, out _)
            ? player.Creature.CombatState?.RoundNumber ?? -1
            : -1;
    }

    /// Per-combat, not per-turn: `IsCombatDecisionWindow` flips every turn, so it
    /// cannot mark a new combat.
    private void ResetBridgeBudgetOnNewCombat()
    {
        bool inCombat = MegaCrit.Sts2.Core.Combat.CombatManager.Instance?.IsInProgress == true;
        if (inCombat && !_wasInCombat)
        {
            Bridge?.ResetForNewCombat();

            // Round numbers restart each combat.
            _bridgeSilentInRound = -1;
        }

        _wasInCombat = inCombat;
    }

    /// Drop the pending decision if the action set changed, wait if it is still
    /// running, or take it. A failed answer falls back to the heuristic here,
    /// because this is the main thread.
    private bool TryTakePendingDecision(
        IReadOnlyList<AiTeammateAvailableAction> decisionActions,
        string actionSetFingerprint,
        bool isCombatDecision)
    {
        if (_pendingDecision is not { } decision)
        {
            return false;
        }

        if (!string.Equals(_pendingDecisionFingerprint, actionSetFingerprint, StringComparison.Ordinal))
        {
            Log.Info($"[Sts2Coop] Player={PlayerId} dropped a pending decision; the action set changed");
            ClearPendingDecision("snapshot_changed");
            return false;
        }

        if (!decision.IsCompleted)
        {
            _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
            return true;
        }

        AiDecisionResult? answer = decision.IsCompletedSuccessfully ? decision.Result : null;
        string why = decision.IsCompletedSuccessfully ? "answered"
            : decision.IsCanceled ? "budget_spent"
            : "agent_error";
        ClearPendingDecision(why);

        if (answer != null)
        {
            Log.Info($"[Sts2Coop] Player={PlayerId} bridge answered actionId={answer.ChosenActionId} reason={answer.Reason ?? "none"}");

            // Hold, don't play: the partner needs time to read the plan and object.
            _heldPlan.Clear();
            foreach (string id in answer.PlannedActionIds.Count > 0
                ? answer.PlannedActionIds
                : [answer.ChosenActionId])
            {
                _heldPlan.Enqueue(id);
            }

            _heldPlanFingerprint = actionSetFingerprint;
            _heldPlanActNow = answer.ActNow;
            _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
            Log.Info($"[Sts2Coop] Player={PlayerId} holding {_heldPlan.Count} action(s){(_heldPlanActNow ? " [act now]" : "")}: {string.Join(" → ", _heldPlan)}");
            return true;
        }

        _bridgeSilentInRound = CurrentCombatRound();
        Log.Info($"[Sts2Coop] Player={PlayerId} {why}; heuristic for the rest of round {_bridgeSilentInRound}");

        if (decisionActions.Count == 0)
        {
            _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
            return true;
        }

        RunHeuristicNow(BuildDecisionRequest(decisionActions), actionSetFingerprint, isCombatDecision);
        return true;
    }

    /// Drop the plan when combat ends, wait while the partner is still playing,
    /// otherwise play the next step. Returns false on drop so `Tick()` falls
    /// through and asks again in the same frame.
    ///
    /// The fingerprint is checked only while waiting. Once playing, every step
    /// changes it; from then on only the next step's legality is checked.
    private bool TryHandleHeldPlan(
        IReadOnlyList<AiTeammateAvailableAction> decisionActions,
        string actionSetFingerprint,
        bool isCombatDecision)
    {
        if (_heldPlan.Count == 0)
        {
            return false;
        }

        if (!isCombatDecision)
        {
            ClearHeldPlan("left_combat");
            return false;
        }

        bool humanEnded = _heldPlanActNow || HumanEndedTurn();

        // The partner changed the board while we waited; the plan's premise is gone.
        if (!humanEnded
            && !string.Equals(_heldPlanFingerprint, actionSetFingerprint, StringComparison.Ordinal))
        {
            ClearHeldPlan("action_set_changed");
            return false;
        }

        // An earlier step may have made this one illegal; drop the rest and re-ask.
        string next = _heldPlan.Peek();
        if (!decisionActions.Any(a => string.Equals(a.ActionId, next, StringComparison.Ordinal)))
        {
            ClearHeldPlan("action_missing");
            return false;
        }

        if (!humanEnded)
        {
            _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
            return true;
        }

        // Remaining steps follow on later frames without another bridge round trip.
        _heldPlan.Dequeue();
        string fingerprint = _heldPlanFingerprint ?? actionSetFingerprint;

        _heldPlanFingerprint = actionSetFingerprint;

        if (_heldPlan.Count == 0)
        {
            _heldPlanFingerprint = null;
            _heldPlanActNow = false;
        }

        Log.Info($"[Sts2Coop] Player={PlayerId} committing actionId={next} ({_heldPlan.Count} left)");
        CommitResolvedAction(next, fingerprint, allowDelayedEndTurn: false);
        return true;
    }

    /// Also called when the bridge reports the partner spoke. Returns whether there
    /// was a plan, so the bridge can tell a no-op.
    public bool DropHeldPlan()
    {
        if (_heldPlan.Count == 0)
        {
            return false;
        }

        ClearHeldPlan("human_said");
        return true;
    }

    private void ClearHeldPlan(string why)
    {
        if (_heldPlan.Count == 0)
        {
            return;
        }

        Log.Info($"[Sts2Coop] Player={PlayerId} dropped the held plan ({why}, {_heldPlan.Count} left)");
        _heldPlan.Clear();
        _heldPlanFingerprint = null;
        _heldPlanActNow = false;
    }

    /// True when the partner cannot be found. False would hold the plan until the
    /// end of combat, since nothing else would release it.
    private static bool HumanEndedTurn()
    {
        var combat = MegaCrit.Sts2.Core.Combat.CombatManager.Instance;
        if (combat == null || !combat.IsInProgress)
        {
            return true;
        }

        Player? human = Sts2CoopStateEndpoint.FindHumanPlayerOrNull();
        return human == null || combat.IsPlayerReadyToEndTurn(human);
    }

    private void ClearPendingDecision(string why)
    {
        if (_pendingDecision is { IsFaulted: true } faulted)
        {
            Log.Warn($"[Sts2Coop] Player={PlayerId} pending decision faulted ({why}): {faulted.Exception?.GetBaseException().Message}");
        }

        try { _pendingDecisionCts?.Cancel(); }
        catch (ObjectDisposedException) { /* already disposed */ }

        _pendingDecisionCts?.Dispose();
        _pendingDecisionCts = null;
        _pendingDecision = null;
        _pendingDecisionFingerprint = null;
    }

    private AiDecisionRequest BuildDecisionRequest(IReadOnlyList<AiTeammateAvailableAction> decisionActions)
    {
        return new AiDecisionRequest
        {
            RequestId = BuildDecisionRequestId(),
            SnapshotId = BuildDecisionSnapshotId(),
            ActorId = PlayerId.ToString(),
            LegalActions = decisionActions.Select(static action => action.Option).ToList()
        };
    }

    public IReadOnlyList<AiTeammateAvailableAction> DiscoverAvailableActions()
    {
        if (!TryGetControlledPlayer(out Player player, out RunState runState))
        {
            return Array.Empty<AiTeammateAvailableAction>();
        }

        if (!player.Creature.IsAlive)
        {
            return Array.Empty<AiTeammateAvailableAction>();
        }

        if (IsCombatDecisionWindow(player))
        {
            return DiscoverCombatActions(player);
        }

        if (runState.CurrentRoom is MegaCrit.Sts2.Core.Rooms.EventRoom)
        {
            return DiscoverEventActions(player);
        }

        if (runState.CurrentRoom is MegaCrit.Sts2.Core.Rooms.RestSiteRoom)
        {
            return DiscoverRestSiteActions(player);
        }

        if (runState.CurrentRoom is MegaCrit.Sts2.Core.Rooms.MerchantRoom)
        {
            return DiscoverMerchantActions(player);
        }

        return Array.Empty<AiTeammateAvailableAction>();
    }

    public static bool IsAiPlayer(Player? player)
    {
        return player != null &&
               AiTeammateSessionRegistry.Current?.AiControllers.ContainsKey(player.NetId) == true;
    }

    public static bool TryGetControllerFor(ulong playerId, out AiTeammateDummyController controller)
    {
        if (AiTeammateSessionRegistry.Current is { } session &&
            session.AiControllers.TryGetValue(playerId, out AiTeammateDummyController? foundController))
        {
            controller = foundController;
            return true;
        }

        controller = null!;
        return false;
    }

    private static bool IsCombatDecisionWindow(Player player)
    {
        // CombatManager.IsPlayPhase is gone in v0.107.1. The per-player phase is more
        // accurate in co-op anyway; without it the AI plans before its hand is dealt
        // and can only end the turn.
        return MegaCrit.Sts2.Core.Combat.CombatManager.Instance.IsInProgress &&
               player.PlayerCombatState?.Phase == MegaCrit.Sts2.Core.Combat.PlayerTurnPhase.Play &&
               player.Creature.CombatState?.CurrentSide == player.Creature.Side &&
               !MegaCrit.Sts2.Core.Combat.CombatManager.Instance.IsPlayerReadyToEndTurn(player);
    }

    private List<AiTeammateAvailableAction> BuildDecisionActions(IReadOnlyList<AiTeammateAvailableAction> actions)
    {
        return actions
            .Where(action => action.DeduplicationKey == null || action.DeduplicationKey != _lastDeduplicationKey)
            .ToList();
    }

    private bool TryExecuteActionById(string actionId)
    {
        AiTeammateAvailableAction? action = DiscoverAvailableActions()
            .FirstOrDefault(candidate => string.Equals(candidate.ActionId, actionId, StringComparison.Ordinal));
        if (action == null)
        {
            Log.Warn($"[AITeammate] Player={PlayerId} could not resolve actionId={actionId} at commit time.");
            return false;
        }

        _isExecutingAction = true;
        TaskHelper.RunSafely(ExecuteResolvedActionAsync(action));
        return true;
    }

    private async Task ExecuteResolvedActionAsync(AiTeammateAvailableAction action)
    {
        try
        {
            AiActionExecutionResult executionResult = await action.ExecuteAsync();
            if (!string.IsNullOrEmpty(action.DeduplicationKey))
            {
                _lastDeduplicationKey = action.DeduplicationKey;
            }

            if (executionResult.HasTrackedGameAction)
            {
                BeginIssuedActionSettlement(action, executionResult);
                Log.Info($"[AITeammate] Player={PlayerId} issued actionId={action.ActionId} tracking={DescribeTrackedAction(executionResult.GameAction!)} queueSettle={executionResult.WaitForQueueSettle}");
            }
            else
            {
                Log.Info($"[AITeammate] Player={PlayerId} executed non-tracked actionId={action.ActionId}");
                if (IsCombatEndTurnAction(action.ActionId) &&
                    TryGetControlledPlayer(out Player controlledPlayer, out _))
                {
                    _lastCompletedEndTurnRound = controlledPlayer.Creature.CombatState?.RoundNumber ?? _lastCompletedEndTurnRound;
                }
            }
        }
        catch (Exception exception)
        {
            Log.Warn($"[AITeammate] Dummy controller {PlayerId} failed to execute actionId={action.ActionId}: {exception}");
        }
        finally
        {
            _isExecutingAction = false;
        }
    }

    private static string BuildActionSetFingerprint(IReadOnlyList<AiTeammateAvailableAction> actions)
    {
        return string.Join("|", actions.Select(static action => action.ActionId));
    }

    private static bool ShouldExecuteImmediateCombatDecision(IReadOnlyList<AiTeammateAvailableAction> actions)
    {
        return actions.Count == 1 && !IsEndTurnAction(actions[0]);
    }

    private static bool ShouldScheduleDelayedEndTurn(IReadOnlyList<AiTeammateAvailableAction> actions)
    {
        return actions.Count == 1 && IsEndTurnAction(actions[0]);
    }

    private bool TryHandlePendingEndTurn(
        IReadOnlyList<AiTeammateAvailableAction> decisionActions,
        string actionSetFingerprint,
        bool isCombatDecision)
    {
        if (string.IsNullOrEmpty(_pendingEndTurnActionId))
        {
            return false;
        }

        if (!isCombatDecision)
        {
            ClearPendingEndTurn("left_combat_window");
            return false;
        }

        if (!decisionActions.Any(action => string.Equals(action.ActionId, _pendingEndTurnActionId, StringComparison.Ordinal)))
        {
            ClearPendingEndTurn("action_missing");
            return false;
        }

        bool hasNonEndTurnAction = decisionActions.Any(action => !IsEndTurnAction(action));
        if (hasNonEndTurnAction)
        {
            ClearPendingEndTurn("better_actions_available");
            return false;
        }

        if (DateTime.UtcNow < _pendingEndTurnCommitAtUtc)
        {
            _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
            return true;
        }

        string actionId = _pendingEndTurnActionId;
        string fingerprint = _pendingEndTurnActionSetFingerprint ?? actionSetFingerprint;
        ClearPendingEndTurn("commit");
        Log.Info($"[AITeammate] Player={PlayerId} committing delayed end turn actionId={actionId}");
        CommitResolvedAction(actionId, fingerprint, allowDelayedEndTurn: false);
        return true;
    }

    /// Combat decisions go to the bridge and return immediately; a later frame's
    /// `TryTakePendingDecision` picks up the answer. Out-of-combat decisions skip
    /// the bridge, which only reads combat state.
    private void ExecuteImmediateDecision(
        AiDecisionRequest request, string actionSetFingerprint, bool isCombatDecision)
    {
        Log.Debug($"[Sts2Coop][DIAG] Player={PlayerId} decide on thread={Environment.CurrentManagedThreadId} isMain={Sts2CoopMainThread.IsMainThread()}");

        int round = CurrentCombatRound();
        if (isCombatDecision && Bridge is { } bridge && _bridgeSilentInRound != round)
        {
            var cts = new CancellationTokenSource();
            try
            {
                Task<AiDecisionResult>? asked = bridge.TryAsk(request, cts.Token);
                if (asked != null)
                {
                    _pendingDecision = asked;
                    _pendingDecisionFingerprint = actionSetFingerprint;
                    _pendingDecisionCts = cts;
                    _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
                    Log.Info($"[Sts2Coop] Player={PlayerId} asked the bridge; budget={BridgeDecisionBackend.Budget.TotalSeconds:0}s");
                    return;
                }
            }
            catch (Exception exception)
            {
                Log.Warn($"[Sts2Coop] Player={PlayerId} could not ask the bridge: {exception.Message}");
            }

            cts.Dispose();
        }

        RunHeuristicNow(request, actionSetFingerprint, isCombatDecision);
    }

    /// The heuristic completes synchronously (`Task.FromResult`), so the result is
    /// taken here and game state is read only on the main thread.
    private void RunHeuristicNow(
        AiDecisionRequest request, string actionSetFingerprint, bool isCombatDecision)
    {
        try
        {
            Task<AiDecisionResult> task = DecisionBackend.DecideAsync(request, CancellationToken.None);
            if (!task.IsCompletedSuccessfully)
            {
                Log.Warn($"[AITeammate] Player={PlayerId} heuristic backend did not complete synchronously; skipping tick");
                _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
                return;
            }

            AiDecisionResult result = task.Result;
            Log.Info($"[AITeammate] Player={PlayerId} chose actionId={result.ChosenActionId} reason={result.Reason ?? "none"}");

            // Hold the heuristic's choice too, so a bridge failure doesn't turn into
            // the AI silently playing its turn. No bubble (nothing was said), just the
            // approve button. Never outside combat: TryHandleHeldPlan would drop it.
            if (isCombatDecision)
            {
                _heldPlan.Clear();
                _heldPlan.Enqueue(result.ChosenActionId);
                _heldPlanFingerprint = actionSetFingerprint;
                _heldPlanActNow = false;
                _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
                Log.Info($"[Sts2Coop] Player={PlayerId} holding the heuristic pick, waiting");
                return;
            }

            CommitResolvedAction(result.ChosenActionId, actionSetFingerprint);
        }
        catch (Exception exception)
        {
            Log.Warn($"[AITeammate] Dummy controller {PlayerId} failed to choose an action: {exception}");
            _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
        }
    }

    private void CommitResolvedAction(string actionId, string actionSetFingerprint)
    {
        CommitResolvedAction(actionId, actionSetFingerprint, allowDelayedEndTurn: true);
    }

    private void CommitResolvedAction(string actionId, string actionSetFingerprint, bool allowDelayedEndTurn)
    {
        if (allowDelayedEndTurn && IsCombatEndTurnAction(actionId))
        {
            if (!string.Equals(_pendingEndTurnActionId, actionId, StringComparison.Ordinal))
            {
                _pendingEndTurnActionId = actionId;
                _pendingEndTurnActionSetFingerprint = actionSetFingerprint;
                _pendingEndTurnCommitAtUtc = DateTime.UtcNow + EndTurnGraceInterval;
                Log.Info($"[AITeammate] Player={PlayerId} scheduled delayed end turn actionId={actionId} graceMs={(int)EndTurnGraceInterval.TotalMilliseconds}");
            }

            _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
            return;
        }

        Log.Info($"[AITeammate] Player={PlayerId} chose actionId={actionId}");
        _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
        if (!TryExecuteActionById(actionId))
        {
            _nextDecisionAtUtc = DateTime.UtcNow + IdleTickInterval;
        }
    }

    private void ClearPendingEndTurn(string reason)
    {
        if (string.IsNullOrEmpty(_pendingEndTurnActionId))
        {
            return;
        }

        Log.Info($"[AITeammate] Player={PlayerId} canceled delayed end turn actionId={_pendingEndTurnActionId} reason={reason}");
        _pendingEndTurnActionId = null;
        _pendingEndTurnActionSetFingerprint = null;
        _pendingEndTurnCommitAtUtc = DateTime.MinValue;
    }

    private string BuildDecisionRequestId()
    {
        return $"player_{PlayerId}_request_{DateTime.UtcNow.Ticks}";
    }

    private string BuildDecisionSnapshotId()
    {
        return $"player_{PlayerId}_ticks_{DateTime.UtcNow.Ticks}";
    }

    private bool ShouldSuppressRepeatedEndTurn(
        IReadOnlyList<AiTeammateAvailableAction> decisionActions,
        Player player,
        bool isCombatDecision)
    {
        if (!isCombatDecision)
        {
            return false;
        }

        int currentRound = player.Creature.CombatState?.RoundNumber ?? -1;
        if (currentRound != _lastCompletedEndTurnRound)
        {
            return false;
        }

        if (decisionActions.Count == 0 || decisionActions.Any(action => !IsEndTurnAction(action)))
        {
            return false;
        }

        Log.Info($"[AITeammate] Player={PlayerId} suppressing repeated end turn for round={currentRound} while combat state settles.");
        return true;
    }

    private void ResetCompletedEndTurnTrackingIfNeeded(Player? player, bool isCombatDecision)
    {
        if (!isCombatDecision || player?.Creature?.CombatState == null)
        {
            _lastCompletedEndTurnRound = -1;
            _lastCombatRoundWithInitialStagger = -1;
            return;
        }

        int currentRound = player.Creature.CombatState.RoundNumber;
        if (currentRound != _lastCompletedEndTurnRound)
        {
            _lastCompletedEndTurnRound = -1;
        }
    }

    private bool TryApplyInitialCombatDecisionStagger(
        IReadOnlyList<AiTeammateAvailableAction> decisionActions,
        Player player,
        bool isCombatDecision)
    {
        if (!isCombatDecision || player.Creature?.CombatState == null)
        {
            _lastCombatRoundWithInitialStagger = -1;
            return false;
        }

        if (decisionActions.Count == 0 || decisionActions.All(IsEndTurnAction))
        {
            return false;
        }

        int currentRound = player.Creature.CombatState.RoundNumber;
        if (currentRound == _lastCombatRoundWithInitialStagger)
        {
            return false;
        }

        _lastCombatRoundWithInitialStagger = currentRound;
        int delayMs = Random.Shared.Next(0, (int)MaxInitialCombatDecisionStagger.TotalMilliseconds + 1);
        if (delayMs <= 0)
        {
            return false;
        }

        _nextDecisionAtUtc = DateTime.UtcNow + TimeSpan.FromMilliseconds(delayMs);
        Log.Info($"[AITeammate] Player={PlayerId} applying initial combat decision stagger round={currentRound} delayMs={delayMs}");
        return true;
    }

    private static bool IsCombatEndTurnAction(string actionId)
    {
        return string.Equals(actionId, "end_turn", StringComparison.Ordinal)
               || actionId.StartsWith("end_turn_", StringComparison.Ordinal);
    }

    private static bool IsEndTurnAction(AiTeammateAvailableAction action)
    {
        return string.Equals(action.ActionType, AiTeammateActionKind.EndTurn.ToString(), StringComparison.Ordinal)
               || IsCombatEndTurnAction(action.ActionId);
    }

    private void BeginIssuedActionSettlement(AiTeammateAvailableAction action, AiActionExecutionResult executionResult)
    {
        _pendingIssuedActionSettlement = new PendingIssuedActionSettlement
        {
            ActionId = action.ActionId,
            ActionType = action.ActionType,
            GameAction = executionResult.GameAction!,
            IssuedAtUtc = DateTime.UtcNow,
            WaitForQueueSettle = executionResult.WaitForQueueSettle
        };
    }

    private bool TryWaitForIssuedActionSettlement()
    {
        if (_pendingIssuedActionSettlement == null)
        {
            return false;
        }

        PendingIssuedActionSettlement settlement = _pendingIssuedActionSettlement;
        DateTime now = DateTime.UtcNow;

        if (!settlement.ActionCompleted)
        {
            if (settlement.GameAction.CompletionTask.IsCompleted ||
                settlement.GameAction.State is GameActionState.Finished or GameActionState.Canceled)
            {
                settlement.ActionCompleted = true;
                settlement.ActionCompletedAtUtc = now;
                if (IsCombatEndTurnAction(settlement.ActionId) &&
                    TryGetControlledPlayer(out Player controlledPlayer, out _))
                {
                    settlement.CompletedEndTurnRound = controlledPlayer.Creature.CombatState?.RoundNumber;
                }

                Log.Info($"[AITeammate] Player={PlayerId} action settled actionId={settlement.ActionId} state={settlement.GameAction.State}");
            }
            else if (now - settlement.IssuedAtUtc >= ActionSettleTimeout)
            {
                settlement.ActionCompleted = true;
                settlement.ActionCompletedAtUtc = now;
                settlement.WasTimeoutFallback = true;
                Log.Warn($"[AITeammate] Player={PlayerId} action settle timeout actionId={settlement.ActionId} state={settlement.GameAction.State}; falling back to queue settle check.");
            }
            else
            {
                _nextDecisionAtUtc = now + IdleTickInterval;
                return true;
            }
        }

        if (settlement.WaitForQueueSettle && !settlement.QueueSettled)
        {
            if (IsQueueSettledForReplan(settlement))
            {
                settlement.QueueSettled = true;
                settlement.QueueSettledAtUtc = now;
                settlement.GraceEndsAtUtc = now + PostSettleGraceInterval;
                Log.Info($"[AITeammate] Player={PlayerId} queue settled after actionId={settlement.ActionId}; graceMs={(int)PostSettleGraceInterval.TotalMilliseconds}");
            }
            else if (settlement.ActionCompletedAtUtc.HasValue &&
                     now - settlement.ActionCompletedAtUtc.Value >= QueueSettleTimeout)
            {
                settlement.QueueSettled = true;
                settlement.QueueSettledAtUtc = now;
                settlement.GraceEndsAtUtc = now + PostSettleGraceInterval;
                settlement.WasTimeoutFallback = true;
                Log.Warn($"[AITeammate] Player={PlayerId} queue settle timeout after actionId={settlement.ActionId}; continuing after grace period.");
            }
            else
            {
                _nextDecisionAtUtc = now + IdleTickInterval;
                return true;
            }
        }

        if (settlement.GraceEndsAtUtc.HasValue && now < settlement.GraceEndsAtUtc.Value)
        {
            _nextDecisionAtUtc = settlement.GraceEndsAtUtc.Value;
            return true;
        }

        if (settlement.CompletedEndTurnRound.HasValue)
        {
            _lastCompletedEndTurnRound = settlement.CompletedEndTurnRound.Value;
        }

        Log.Info($"[AITeammate] Player={PlayerId} ready to replan after actionId={settlement.ActionId} timeoutFallback={settlement.WasTimeoutFallback}");
        _pendingIssuedActionSettlement = null;
        return false;
    }

    private bool IsQueueSettledForReplan(PendingIssuedActionSettlement settlement)
    {
        GameAction? runningAction = RunManager.Instance.ActionExecutor.CurrentlyRunningAction;
        if (runningAction != null &&
            ActionQueueSet.IsGameActionPlayerDriven(runningAction) &&
            runningAction.OwnerId == PlayerId)
        {
            return false;
        }

        GameAction? readyAction = RunManager.Instance.ActionQueueSet.GetReadyAction();
        if (readyAction != null &&
            ActionQueueSet.IsGameActionPlayerDriven(readyAction) &&
            readyAction.OwnerId == PlayerId)
        {
            return false;
        }

        Log.Debug($"[AITeammate] Player={PlayerId} treating queue as settled for actionId={settlement.ActionId}; runningOwner={runningAction?.OwnerId.ToString() ?? "none"} readyOwner={readyAction?.OwnerId.ToString() ?? "none"}");
        return true;
    }

    private static string DescribeTrackedAction(GameAction action)
    {
        return $"{action.GetType().Name}:{action.State}";
    }

    private bool TryGetControlledPlayer(out Player player, out RunState runState)
    {
        runState = RunManager.Instance.DebugOnlyGetState()!;
        player = runState?.GetPlayer(PlayerId)!;
        return runState != null && player != null;
    }

    private sealed class PendingIssuedActionSettlement
    {
        public required string ActionId { get; init; }

        public required string ActionType { get; init; }

        public required GameAction GameAction { get; init; }

        public required DateTime IssuedAtUtc { get; init; }

        public bool WaitForQueueSettle { get; init; }

        public bool ActionCompleted { get; set; }

        public DateTime? ActionCompletedAtUtc { get; set; }

        public bool QueueSettled { get; set; }

        public DateTime? QueueSettledAtUtc { get; set; }

        public DateTime? GraceEndsAtUtc { get; set; }

        public int? CompletedEndTurnRound { get; set; }

        public bool WasTimeoutFallback { get; set; }
    }
}

// ReSharper disable InconsistentNaming
using HarmonyLib;
using System.Diagnostics.CodeAnalysis;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Events;

namespace Sts2LlmCoop;

[SuppressMessage("ReSharper", "UnusedType.Global")]
internal static class AiTeammateEventSafetyPatches
{
    [HarmonyPatch(typeof(ActModel), nameof(ActModel.GenerateRooms))]
[SuppressMessage("ReSharper", "UnusedType.Global")]
    private static class ActModelGenerateRoomsPatch
    {
        private static void Postfix(ActModel __instance)
        {
            if (AiTeammateSessionRegistry.Current == null)
            {
                return;
            }

            ExcludeUnsafeEvent<CrystalSphere>(__instance);
        }

        private static void ExcludeUnsafeEvent<TEvent>(ActModel actModel)
            where TEvent : EventModel
        {
            EventModel canonicalEvent = ModelDb.Event<TEvent>();
            bool wasPresent = actModel.AllEvents.Any(eventModel => eventModel.Id == canonicalEvent.Id) ||
                              ModelDb.AllSharedEvents.Any(eventModel => eventModel.Id == canonicalEvent.Id);
            if (!wasPresent)
            {
                return;
            }

            actModel.RemoveEventFromSet(canonicalEvent);
            Log.Info($"[AITeammate][EventSafety] Excluded {canonicalEvent.GetType().Name} from generated event pool for AI teammate session.");
        }
    }
}

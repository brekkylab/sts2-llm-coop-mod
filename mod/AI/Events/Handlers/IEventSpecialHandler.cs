namespace STS2AiTeammate;

internal interface IEventSpecialHandler
{
    string HandlerName { get; }

    bool CanHandle(EventVisitState snapshot);

    EventOptionDescriptor Normalize(EventVisitState snapshot, EventOptionDescriptor option);
}

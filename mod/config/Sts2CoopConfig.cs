using BaseLib.Config;

namespace Sts2LlmCoop;

/// In-game settings. BaseLib requires static properties and persists them to
/// `~/Library/Application Support/SlayTheSpire2/mod_configs/Sts2LlmCoop.cfg`.
internal sealed class Sts2CoopConfig : SimpleModConfig
{
    /// ASCII names: BaseLib derives labels and keys from them, and non-ASCII
    /// handling is unverified.
    public enum TalkLanguage { Korean, English, Japanese }

    [ConfigSection("대화")]
    public static TalkLanguage Language { get; set; } = TalkLanguage.Korean;

    /// The language's own name, for the prompt. [ConfigIgnore] keeps BaseLib from
    /// rendering this getter-only property as a setting.
    [ConfigIgnore]
    public static string LanguageName => Language switch
    {
        TalkLanguage.English => "English",
        TalkLanguage.Japanese => "日本語",
        _ => "한국어",
    };

    /// Measured median 4.0 s, max 7.5 s; 20 s hides inside a human's own turn.
    [ConfigSection("판단")]
    [ConfigSlider(5, 60, 5)]
    public static int DecisionBudgetSeconds { get; set; } = 20;

    /// Beyond this the rest of the combat goes to the heuristic.
    [ConfigSlider(10, 100, 10)]
    public static int CallsPerCombat { get; set; } = 30;

    /// Start and stop the bridge with the game. Turn off to run it by hand in a
    /// terminal. Its location is baked in at build time.
    [ConfigSection("브리지")]
    public static bool AutoStart { get; set; } = true;
}

internal sealed class CoopConfigDto
{
    public string Language { get; set; } = "한국어";
    public int DecisionBudgetSeconds { get; set; }
    public int CallsPerCombat { get; set; }
}

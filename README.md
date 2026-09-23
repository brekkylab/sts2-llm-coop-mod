# sts2-llm-coop-mod

A Slay the Spire 2 co-op mod where your teammate is an LLM agent.

You play one character and the agent plays the other. On each of its turns the agent
plans the whole turn, says what it intends in a speech bubble, and waits: it plays
once you end your own turn, or right away if you press **Act now**. You can talk to it
at any time, and what you say goes into its next decision.

This is an experimental example built on [ailoy](https://github.com/brekkylab/ailoy)
and [cortex](https://github.com/brekkylab/cortex). It runs on **macOS only** and was
tested with **Slay the Spire 2 v0.107.1**.

> **It does not build against public ailoy as is.** It was developed against a
> locally patched ailoy that is not published. See [The ailoy patch](#the-ailoy-patch).

## Credits

- The mod is based on [SallyHong2347/sts2-ai-teammate](https://github.com/SallyHong2347/sts2-ai-teammate)
  (MIT) by Sally Hong: an AI-teammate mod that plays combat, rewards, shops and events
  with heuristics. Most of `mod/`, including the heuristic the agent falls back on, is
  her code, and her commit history is preserved here.
- The port to game v0.107.1 comes from [yehuoshun/STS2-AiTeammate](https://github.com/yehuoshun/STS2-AiTeammate) (MIT).
- This repository adds the local HTTP server, the bridge to the LLM, speech bubbles,
  the approve button and in-game settings.

## How it fits together

```
Slay the Spire 2
  └─ mod/ (C#, Harmony)  ── HTTP 127.0.0.1:15527 ──  agent/sts2-bridge (Rust)  ──  LLM (via ailoy)
                                                        ├─ window (egui)
                                                        └─ cortex console, used only by the post-combat summary
```

The bridge always initiates: it polls the mod every 200 ms. When the game needs a
combat decision, it waits up to 20 s for an answer. The bridge sends the model a
briefing (board, hand, enemies, relics, potions) with the numbered legal actions and
posts back a plan such as `CHOSEN: 1, 3`. The mod holds the plan until the partner
ends their turn, then plays it step by step. If the bridge doesn't answer in time, or
the answer is unusable, the game uses the original heuristic for the rest of that
round. Decisions outside combat (map, rewards, shops) always use the heuristic.

Within a combat the agent keeps one conversation. After each decision its full
briefing is shrunk to a one-line summary, so old hands can't leak into later turns.
When a combat ends, the agent revises its notes about you (`notes/learned.md`) with
file tools; that is the only place it uses tools.

## Repository layout

| Path | What |
|---|---|
| `mod/` | The C# mod (loaded by the game via BaseLib) |
| `agent/sts2-core` | HTTP client for the mod, DTOs, briefing renderer |
| `agent/sts2-bridge` | The bridge and its window |
| `agent/sts2-cli` | Small tools: `sts2-say`, `sts2-play`, `sts2-end-turn`, `sts2-replay` |
| `agent/run-bridge.sh` | Starts the bridge, cleaning up after a previous crashed run |

## Requirements

- macOS (FUSE-T, `diskutil` and a system font path are macOS-specific)
- Slay the Spire 2 v0.107.1 (Steam) and [BaseLib](https://steamcommunity.com/sharedfiles/filedetails/?id=3737335127) from the Steam Workshop
- .NET 9 SDK
- Rust 1.95+
- [FUSE-T](https://www.fuse-t.org/) and `pkg-config`
- Sibling checkouts of `ailoy` (patched, see below) and `cortex`
- An API key: `ANTHROPIC_API_KEY`, or `AWS_BEARER_TOKEN_BEDROCK` for Bedrock

### The ailoy patch

The bridge depends on ailoy by path and uses four things that public ailoy (`develop`,
as of 2026-09) doesn't have. All of them target the Anthropic Messages API:

| Used in the bridge | Sends | Why |
|---|---|---|
| `AgentBuilder::max_tokens(n)` | `"max_tokens": n` | Smaller limits left no room for the answer after thinking |
| `AgentBuilder::effort(e)` | `"output_config": {"effort": e}` | The thinking knob; Sonnet 5 rejects `thinking.budget_tokens` and `temperature` |
| `AgentBuilder::thinking_display(d)` | `"thinking": {"type": "adaptive", "display": d}` | Makes the model's reasoning visible in the window |
| Prompt-cache breakpoints | `"cache_control": {"type": "ephemeral"}` on the system block and the last message | Not needed to build; only affects the reported cache reads |

To build with a stock ailoy, either add these three builder methods or remove those
calls from `agent/sts2-bridge/src/agent.rs` (`ask()` and `summarize()`). The call
sites are marked with comments.

## Build

Put this repository next to `ailoy` and `cortex`:

```
workspace/
├── ailoy/
├── cortex/
└── sts2-llm-coop-mod/
```

```sh
# cortex's console server, used by the bridge
(cd cortex && cargo build --release -p cortex-local-console)

# the bridge and tools
(cd sts2-llm-coop-mod/agent && cargo build --release)

# the mod; copies the DLL into the game's mods folder
(cd sts2-llm-coop-mod/mod && dotnet build -c Release)
```

Build the mod as **Release**: Debug skips the Publicizer and does not compile. Quit
the game before building. Replacing the DLL under a running game corrupts it, and the
errors then look like bugs in the new code.

## Run

1. Put your API key in a `.env` at the workspace root. The default model is
   `bedrock/global.anthropic.claude-sonnet-5`; to use the Anthropic API directly, set
   `STS2_MODEL=anthropic/<model id>`.
2. Start the game. With **auto start** on (the default), the mod launches
   `agent/run-bridge.sh` and the bridge window opens.
3. Start a co-op run with an AI teammate from the mod's menu.

To run the bridge by hand in a terminal, turn auto start off in the mod settings and
run `agent/run-bridge.sh` (add `--no-gui` for no window).

### In-game settings

Mod settings (BaseLib): talk language (Korean, English, Japanese), decision budget
(seconds), calls per combat, and auto start. The briefing and the bridge's logs are
in Korean regardless of the talk language.

### Talking to the agent

Type in the box at the bottom of the window, or run `sts2-say "..."`.

## The window

- **Right:** each turn's decisions, what the agent said, token counts, and (in debug
  mode) the raw model output.
- **Left:** four session files.

| File | In the window | What it is |
|---|---|---|
| `memory.md` | editable | Your standing instructions to the agent. One per line, each starting with `- `; other lines are ignored. |
| `notes/learned.md` | editable | What the agent has learned about you, rewritten after each combat. Delete wrong lines here. |
| `last_prompt.md` | read-only | Everything the agent received in its last decision |
| `brief.md` | read-only | The current board |

When your instructions and the agent's notes disagree, yours win.

## Session directory and records

The session lives in `/tmp/sts2-session` (`STS2_SESSION`): the four files above, plus
`said/`, one file per utterance, cleared when a combat ends. Each decision is also
appended to `agent/run_result/run-*/decisions.jsonl`, with its trigger
(`turn_start` or `human_said`), latency and outcome. An outcome of `replaced` means
the agent changed its plan after you spoke.

## Environment variables

| Variable | Default | |
|---|---|---|
| `STS2_MODEL` | `bedrock/global.anthropic.claude-sonnet-5` | Model id, as understood by ailoy |
| `STS2_ADDR` | `http://127.0.0.1:15527` | The mod's server |
| `STS2_SESSION` | `/tmp/sts2-session` | Session directory |
| `STS2_MOUNT` | `/tmp/sts2-mnt` | FUSE-T mount point for the agent's console |
| `STS2_CORTEX_BIN` | `<workspace>/cortex/target/release` | Directory containing `cortex-local-console` |
| `STS2_RUN_DIR` | `run_result` | Where decision records go |
| `STS2_EXIT_WITH_GAME` | set by the mod | Exit when the game has been unreachable for 5 s |

## Tools

- `sts2-say "text"` sends a line to the agent.
- `sts2-play <card> [--target <enemy>]` plays a card from the AI's hand by name.
- `sts2-end-turn` ends the AI's turn.
- `sts2-replay <samples...>` replays decisions against saved combat states, without
  the game, to see whether a prompt change made choices worse or less consistent.
  Samples are in `agent/run_result/samples/`.

## Known limitations

- macOS only.
- Combat only; everything outside combat uses the original heuristic.
- The post-combat summary blocks the poll loop while it runs. Entering the next
  combat very quickly can miss the first decision.
- The prompt cache never hits in decisions, because the history is rewritten every
  turn. The window's cache counter shows 0 by design.
- The agent's console mount is **not a sandbox**. cortex's local console resolves
  paths by joining, and the shell tool runs on the host.
- Game updates break Harmony patches. Only v0.107.1 has been tested.

## License

MIT, see [`LICENSE`](LICENSE). `mod/LICENSE` is the original mod's notice.

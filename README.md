# sts2-llm-coop-mod

A Slay the Spire 2 co-op mod where your teammate is an LLM agent. The agent plans
its whole turn, says what it intends in a speech bubble, and plays once you end your
turn (or press **Act now**). You can talk to it at any time.

An experimental example for [ailoy](https://github.com/brekkylab/ailoy) and
[cortex](https://github.com/brekkylab/cortex). macOS only, tested with game v0.107.1.

> Built against a locally patched ailoy that isn't published, so it doesn't build
> against public ailoy as is. See [ailoy patch](#ailoy-patch).

## Directory structure

```
mod/                    C# mod, loaded by the game through BaseLib
agent/                  Rust workspace: everything outside the game
├── sts2-core/          Talks to the mod and turns game state into text
│   ├── client.rs         HTTP client for the mod's server
│   ├── dto.rs            Combat state, legal actions, pending decision
│   ├── render.rs         The briefing the model reads
│   └── said.rs           The conversation, one file per utterance
├── sts2-bridge/        The bridge process and its window
│   ├── bridge.rs         Poll loop (every 200 ms): combat entry/exit, summary
│   ├── decision.rs       Answers the game's questions, reacts to the partner
│   ├── agent.rs          Model calls: decide, respond, post-combat summary
│   ├── history.rs        One conversation per combat, briefings shrunk to a line
│   ├── session.rs        Session directory (/tmp/sts2-session)
│   ├── memory.rs         The two note layers: the human's and the agent's
│   ├── console.rs        cortex console for the agent's file tools
│   └── gui/              egui window: activity stream and session files
├── sts2-cli/           Small command-line tools
│   ├── sts2-say          Say something to the agent
│   ├── sts2-play         Play a card from the AI's hand by name
│   ├── sts2-end-turn     End the AI's turn
│   └── sts2-replay       Replay decisions on saved states to compare prompts
└── run-bridge.sh       Starts the bridge, cleaning up after a crashed run
```

## How it works

The mod exposes combat state and actions over HTTP (`127.0.0.1:15527`), and the
bridge polls it. When the game needs a combat decision it waits up to 20 s; the
bridge sends the model a briefing with numbered legal actions and posts back a plan
such as `CHOSEN: 1, 3`. The mod holds the plan until you end your turn. If there's no
usable answer in time, the original heuristic plays the rest of that round. Outside
combat the heuristic always decides.

After each combat the agent rewrites its notes about you (`notes/learned.md`) with
file tools, the only time it uses tools.

## Build and run

Place the repository next to `ailoy` and `cortex`, then:

```sh
(cd cortex && cargo build --release -p cortex-local-console)
(cd sts2-llm-coop-mod/agent && cargo build --release)
(cd sts2-llm-coop-mod/mod && dotnet build -c Release)   # copies into the game's mods folder
```

Requires macOS, FUSE-T and `pkg-config`, Rust 1.95+, the .NET 9 SDK, and the game with
[BaseLib](https://steamcommunity.com/sharedfiles/filedetails/?id=3737335127). Build the
mod as Release (Debug doesn't compile), with the game closed.

Put `ANTHROPIC_API_KEY` (or `AWS_BEARER_TOKEN_BEDROCK`) in a `.env` at the workspace
root, start the game, and start a co-op run with an AI teammate. The mod launches the
bridge and its window. The default model is `bedrock/global.anthropic.claude-sonnet-5`;
override with `STS2_MODEL`.

Mod settings: talk language, decision budget, calls per combat, auto-start. The
briefing and the bridge's logs are in Korean whatever the talk language.

## The window

The right side shows each turn's decisions and what was said; type at the bottom to
talk to the agent. The left side shows four session files:

| File | | |
|---|---|---|
| `memory.md` | editable | Your standing instructions. One per line, starting with `- `. |
| `notes/learned.md` | editable | The agent's notes about you. Delete wrong lines here. |
| `last_prompt.md` | read-only | Everything the agent received in its last decision |
| `brief.md` | read-only | The current board |

Your instructions win over the agent's notes. Decisions are also logged to
`agent/run_result/run-*/decisions.jsonl`; outcome `replaced` means the agent changed its
plan after you spoke.

## ailoy patch

The bridge uses four things public ailoy (`develop`, 2026-09) doesn't have:

| In the bridge | Sends (Anthropic Messages API) |
|---|---|
| `AgentBuilder::max_tokens(n)` | `"max_tokens": n` |
| `AgentBuilder::effort(e)` | `"output_config": {"effort": e}` |
| `AgentBuilder::thinking_display(d)` | `"thinking": {"type": "adaptive", "display": d}` |
| prompt-cache breakpoints | `"cache_control": {"type": "ephemeral"}` (not needed to build) |

With a stock ailoy, add the three builder methods or remove their calls in
`agent/sts2-bridge/src/agent.rs`; the call sites are marked.

## Known limitations

- macOS only; only game v0.107.1 tested.
- The post-combat summary blocks polling while it runs.
- The prompt cache never hits in decisions, since the history is rewritten each turn.
- The agent's console mount is not a sandbox: paths are joined, and the shell tool
  runs on the host.

## Credits and license

Based on [SallyHong2347/sts2-ai-teammate](https://github.com/SallyHong2347/sts2-ai-teammate)
by Sally Hong, whose code is most of `mod/` and whose history is preserved here, with
the v0.107.1 port from [yehuoshun/STS2-AiTeammate](https://github.com/yehuoshun/STS2-AiTeammate).
Both are MIT.

MIT, see [`LICENSE`](LICENSE). `mod/LICENSE` is the original mod's notice.

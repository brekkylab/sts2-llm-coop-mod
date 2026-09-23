# sts2-llm-coop-mod

A Slay the Spire 2 co-op mod where your teammate is an LLM agent. It plans its turn,
tells you the plan in a speech bubble, and plays once you end your turn. You can talk
to it at any time.

An example for [ailoy](https://github.com/brekkylab/ailoy) and
[cortex](https://github.com/brekkylab/cortex). macOS only, tested with game v0.107.1.

## How it works

```
 in the game                     outside the game
┌──────────────┐    HTTP     ┌──────────────────┐    API     ┌─────────┐
│  game + mod  │ ◀────────── │ agent (bridge)   │ ─────────▶ │  model  │
│  plays cards │  asks every │ decides what to  │            │ (Claude)│
│              │    0.2 s    │ do, shows window │            │         │
│    SERVER    │             │      CLIENT      │            │  SERVER │
└──────────────┘             └──────────────────┘            └─────────┘
```

The mod is an HTTP server on `127.0.0.1:15527`; the bridge is its only client, and
also the client of the model's API. So the mod never calls out: it only answers, and
the bridge always asks first, even to post its status for the speech bubbles. One
decision goes like this:

1. The game needs a combat move and waits up to 20 s. The bridge sees this on its next poll.
2. The bridge sends the model the board and a numbered list of legal actions.
3. The model answers with a plan, e.g. `CHOSEN: 1, 3`.
4. The bridge posts the plan to the mod, which holds it and shows it in a speech bubble.
5. When you end your turn (or press **Act now**), the mod plays 1, then 3.

No usable answer in 20 s? The original mod's heuristic plays instead. Outside combat
(map, rewards, shops) the heuristic always decides.

## Directory structure

```
mod/                 The C# mod inside the game
agent/sts2-core      Talks to the mod and turns game state into text for the model
agent/sts2-bridge    The program you run while playing: polls the game, calls the model, shows the window
agent/sts2-cli       Command-line tools to talk, play a card or end a turn by hand, and to replay decisions
```

## Build and run

Needs macOS, FUSE-T, Rust 1.95+, the .NET 9 SDK, and the game with
[BaseLib](https://steamcommunity.com/sharedfiles/filedetails/?id=3737335127). Place this
repository next to `ailoy` and `cortex`:

```sh
(cd cortex && cargo build --release -p cortex-local-console)
(cd sts2-llm-coop-mod/agent && cargo build --release)
(cd sts2-llm-coop-mod/mod && dotnet build -c Release)   # game closed; copies into its mods folder
```

Put `ANTHROPIC_API_KEY` (or `AWS_BEARER_TOKEN_BEDROCK`) in `.env` at the workspace root,
start the game, and start a co-op run with an AI teammate. The bridge and its window
start with the game.

In the window, `memory.md` holds your standing instructions to the agent: one per
line, starting with `- `.

## ailoy patch

This was built against a locally patched ailoy, not published. Public ailoy lacks
`AgentBuilder::max_tokens`, `effort` and `thinking_display`; add them or remove the
marked calls in `agent/sts2-bridge/src/agent.rs`.

## Credits and license

Based on [SallyHong2347/sts2-ai-teammate](https://github.com/SallyHong2347/sts2-ai-teammate)
by Sally Hong, whose code is most of `mod/` and whose history is kept here, with the
v0.107.1 port from [yehuoshun/STS2-AiTeammate](https://github.com/yehuoshun/STS2-AiTeammate).
Both MIT. This repository is MIT too; see [`LICENSE`](LICENSE).

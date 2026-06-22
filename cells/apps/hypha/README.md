# Hypha

> **Hypha** (sợi nấm) — a single living thread of the *Mycelium* that threads through every Cell
> and coordinates them. Cellos's first **real** application: a native Tier-1 Rust AI agent.

Unlike the demo cells (`hello-cell`, `robot-dashboard`, `https-demo`, …) which each prove one
primitive, Hypha is a useful app *and* a showcase of what makes Cellos unique:

- **Capability-isolated tools** — each tool is a separate Cell whose manifest declares exactly
  its authority; kernel-enforced. The agent core holds no dangerous capability and delegates all
  side-effects to tool Cells.
- **Never-die** — supervisor respawns a crashed gateway/tool; the agent reconnects via service
  lookup and continues.
- **Zero-copy IPC** — multi-KB prompts/responses move via Grant, not message-copy.
- **Natural-language robot control** — ties into the G1 robot demo (sensor → reason → actuator).

## Strategic role

Building Hypha **surfaces the missing modules of Cellos**. It is both the deliverable and the
forcing function for maturing the OS — each phase reveals gaps that get filled incrementally.

## Status

Design + planning complete; implementation not started. Full design, phase roadmap, and the
**OS-gaps register** live in the plan folder:

- `.agents/260621-1433-hypha-ai-agent/plan.md` — overview + phase roadmap (P0–P7)
- `.agents/260621-1433-hypha-ai-agent/architecture.md` — topology, agentic loop, IPC, capabilities
- `.agents/260621-1433-hypha-ai-agent/os-gaps.md` — living register of missing OS modules

## Structure (planned)

```
cells/apps/hypha/
├── README.md          # this file
├── llm-gateway/       # P0: HTTPS LLM client Cell (network cap + API key)
├── core/              # P1: the agent brain (agentic loop, conversation, tool dispatch)
└── tools/             # P2+: tool-fs, tool-sys, tool-spawn, tool-peripheral, tool-net
```
(shared IPC types will live in `libs/agent-proto/`.)

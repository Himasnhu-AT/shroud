# ADR 0001: Stock Bevy, plus ashlar's control port and telemetry

- **Status:** accepted (2026-10-19)

## Context
We researched building our own engine on Bevy (an ashlar-style "Kiln" façade, a custom wgpu renderer,
native DX12/Vulkan/Metal backends); see [ENGINE.md](../ENGINE.md). Bevy already covers DX12/Vulkan/Metal/GL
through wgpu, and the game is small (one arena, 5 enemies), so the engine is not the bottleneck.
What we actually lack is a way for agents/tests to drive the game, and crash/performance visibility.

## Decision
1. Use **Bevy `=0.19.1` directly** (with avian3d `=0.7.0`). No façade crate, no fork, no custom renderer.
2. Add a **control port for testing** using `ashlar-control` + `ashlar-command` (+ `ashlar-mcp` for MCP), drained by a Bevy exclusive system. It sits behind a `control` cargo feature and is excluded from release builds.
3. Add **crash & performance reporting** using `ashlar-trace` with a self-hosted `ashlar-trace-server`, wired by a small `TracePlugin`.
4. Both live in `crates/game/src/ops/`. Design: [CONTROL_AND_TELEMETRY.md](../CONTROL_AND_TELEMETRY.md).

## Consequences
- + Zero engine maintenance; Bevy upgrades are ordinary dependency bumps.
- + Automated playtests and AI regression tests via the control port; Claude can run test sessions over MCP. Shipped builds carry none of it.
- + Crashes, hangs and frame-time regressions are visible per version.
- − Depends on ashlar crates that are pre-0.1: pin a git rev and upgrade deliberately.
- − Peak rendering performance is whatever Bevy gives; acceptable for this scope.

## Revisit when
Profiling (via ashlar-trace frame histograms) shows Bevy's renderer is the bottleneck on target hardware.

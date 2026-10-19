# ADR 0001: Build "Kiln" as a façade over Bevy, not a Bevy fork or a native engine

- **Status:** proposed (2026-10-19)

## Context
We want an ashlar-style, single-dependency engine that makes games quick to launch and is fast on
Windows (DX12), Linux (Vulkan), and macOS (Metal) across AMD, NVIDIA, Intel, and Apple GPUs.
Research is in [ENGINE.md](../ENGINE.md). Bevy is modular, already renders through wgpu
(DX12/Vulkan/Metal/GL), and gains major renderer features each release. It also breaks its API every few months.

## Decision
1. Ship Shroud on stock Bevy (`=0.19.1`) first; the AI stays in the engine-free `brain` crate.
2. Build **Kiln**: ranked crates (`kiln`, `kiln-core`, `kiln-frame`, `kiln-control`, `kiln-trace`, `kiln-platform`, `xtask`)
   that wrap pinned Bevy crates behind a stable `Game` builder with presets, a control port, telemetry, and frame pacing.
3. **No fork of Bevy.** Fixes go upstream or into a small patch crate.
4. A **custom renderer (`kiln-render`, wgpu, GPU-driven)** is allowed only when profiling of the benchmark
   scene shows the renderer is the bottleneck. Bevy's renderer stays available as a fallback.
5. **No hand-written DX12/Vulkan/Metal backends.** Vendor features (DLSS/FSR/XeSS, Reflex/Anti-Lag, MetalFX)
   go through wgpu `as_hal` interop, and `unsafe` stays confined to `kiln-platform` (same rule as ashlar ADR 0001).

## Consequences
- + Game work starts now; engine work happens alongside it and doesn't block it.
- + One code path covers all desktop platforms and GPU vendors through wgpu.
- + Bevy upgrades touch only `kiln-core`.
- − Peak draw-call throughput is below a native engine until Phase 2 (GPU-driven rendering mostly closes the gap).
- − Consoles stay out of scope (they need NDA SDKs).

## Revisit when
- p99 frame time on the benchmark scene misses target (e.g. 4 ms at 1440p on a mid-range GPU) because of renderer CPU cost, or
- wgpu lacks a feature we need and the `as_hal` escape hatch can't provide it.

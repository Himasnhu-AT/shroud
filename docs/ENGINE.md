# Engine research: our own high-performance engine on top of Bevy?

**Question:** can we take Bevy and build our own engine (ashlar-style: one dependency, opinionated,
fast to launch a game) that is hyper-performant across DX12, Vulkan (AMD/NVIDIA), Metal, and Linux?

**Short answer:** yes, but not by forking Bevy. Build a **thin engine crate ("Kiln") over Bevy's
modular crates**, and keep the option to **swap Bevy's renderer for our own GPU-driven wgpu renderer**
when profiling says so. Writing separate native DX12/Vulkan/Metal backends ourselves is not worth it now.
Decision record: [adr/0001-engine-strategy.md](adr/0001-engine-strategy.md).

## What "like ashlar" means here
`~/codes/synthai/ashlar` is a single-dependency framework over egui: a builder API (`App::new().commands(..).view(..).run()`),
commands that also drive a CLI/MCP control port, self-hosted telemetry (`ashlar-trace`), crash safety,
zero CPU when idle, ranked crates, and a performance gate. The game-engine equivalent:

```rust
use kiln::prelude::*;

fn main() -> kiln::Result<()> {
    Game::new("shroud", env!("CARGO_PKG_VERSION"))
        .preset(Preset::Fps)                 // camera, input map, fixed tick, physics, audio
        .states::<GameState>()
        .plugin(ShroudPlugin)
        .control_port()                      // agents/tests drive the game (MCP), like ashlar
        .telemetry(Trace::self_hosted())     // reuse ashlar-trace
        .run()                               // crash guard, frame pacing, settings, save dirs
}
```

## The facts that decide it
| Fact | Implication |
|---|---|
| Bevy is ~50 modular crates (`bevy_ecs`, `bevy_app`, `bevy_asset`, `bevy_render`, `bevy_pbr`…), and each can be used alone | We can keep the parts that are good and swap the renderer without forking |
| Bevy renders through **wgpu**; wgpu 29/30 has backends for **DX12, Vulkan, Metal, GLES/WebGL, WebGPU** | One renderer covers Windows (DX12 or Vulkan), Linux (Vulkan), macOS/iOS (Metal), and AMD/NVIDIA/Intel/Apple GPUs |
| wgpu 30 exposes `MULTI_DRAW_INDIRECT_COUNT`, bindless binding arrays, `SUBGROUP`, `SHADER_INT64`, `PIPELINE_CACHE`, timestamp queries, and **experimental** ray query, ray-tracing pipelines, mesh shaders, and cooperative matrix (checked in local `wgpu-types-30.0.1` source) | Modern GPU-driven rendering is possible without writing native backends |
| Bevy 0.20 (Oct 8, 2026) already ships Solari real-time path tracing (now on Metal), DLSS-RR 4.5, initial mesh shaders, and WESL shaders | Bevy's renderer is already strong; replacing it costs a lot of features |
| wgpu adds CPU overhead on draw-call-heavy work (old anecdotal numbers: 10–20% when tuned, up to 2× when naive); `wgpu-hal` is a thinner, unsafe layer with near-native speed | Use GPU-driven rendering (few draw calls) and drop to `wgpu-hal` for hot paths instead of rewriting backends |
| Tiny Glade (shipped Rust game) reportedly used **Bevy ECS + its own Vulkan renderer** (forum sources, not confirmed by the developers) | The "Bevy ECS + custom renderer" model is proven in practice |
| Bevy breaks its API every ~4 months (0.19 in June 2026, 0.20 in October 2026) and ecosystem crates lag (Avian still targets 0.19) | A façade crate protects game code from upgrades; a hard fork means maintaining all of it ourselves |
| Consoles (PS5/Xbox/Switch) need NDA SDKs; wgpu doesn't support them publicly | Out of scope for any option; it would need a licensed native backend later |

## Options compared
| | A. Fork Bevy | **B. Kiln façade over Bevy** | **C. B + own wgpu renderer** | D. Own native backends (ash / windows-rs D3D12 / objc2-metal) |
|---|---|---|---|---|
| What we own | everything (~300k+ lines) | API, presets, tooling (~5–10k lines) | B + renderer (~20–40k lines) | C + 3 graphics backends |
| Platforms | all Bevy targets | all Bevy targets | DX12/Vulkan/Metal/GL via wgpu | only the ones we write |
| Peak performance | = Bevy | = Bevy | tuned for FPS: GPU-driven, our culling, our frame graph | highest (direct vendor APIs) |
| Time to first game | weeks of upkeep before game work | **days** | B now, renderer in ~3–6 months | 12+ months |
| Upgrade pain | merge every Bevy release by hand | bump Bevy behind a stable API | same as B; renderer is ours | none from Bevy; huge from GPU APIs |
| Verdict | ✗ | **✓ now** | **✓ later, if profiling demands** | ✗ (keep an escape hatch) |

## Recommended plan
### Phase 0: ship on stock Bevy (now)
Shroud as designed. The AI is in the engine-free `brain` crate, so nothing here blocks it.
Measure frame time, input latency, and draw calls with `ashlar-trace`-style telemetry from day one.

### Phase 1: Kiln façade (2–4 weeks, overlaps Phase 0)
A workspace of ranked crates, mirroring ashlar:
| Crate | Role |
|---|---|
| `kiln` | the one dependency: `Game` builder, `prelude`, presets (`Fps`, `TopDown`…) |
| `kiln-core` | re-exports the pinned Bevy crates we use, plus a small stable API over them |
| `kiln-frame` | frame pacing, fixed tick, present-mode policy (low-latency: Mailbox/Immediate + frame cap), latency markers |
| `kiln-control` | control port + MCP: agents/tests spawn, move, shoot, screenshot (reuses the ashlar control protocol idea) |
| `kiln-trace` | telemetry: crash, frame-time percentiles, GPU timestamps, hitches (reuse `ashlar-trace`) |
| `kiln-platform` | `unsafe` OS/GPU FFI kept in one crate (ashlar ADR 0001 pattern): vendor SDKs, power, window quirks |
| `xtask` | `cargo xtask package {mac,win,linux}`, asset cooking, perf gate in CI |

**Performance gate (like ashlar's):** CI fails if p99 frame time on the benchmark scene regresses by > 5%.

### Phase 2: own renderer (only if Phase 0 profiling shows the renderer is the bottleneck)
Keep `bevy_ecs`, `bevy_app`, `bevy_asset`, input, audio, windowing (winit). Replace `bevy_render`/`bevy_pbr`
with `kiln-render` on wgpu, built for FPS:
- GPU-driven: one bindless material table, `multi_draw_indirect_count`, GPU frustum + Hi-Z occlusion culling.
- Clustered forward+ lighting (cheap MSAA, good for thin geometry like guns), TAA or upscaler.
- Async compute for culling/particles where the backend supports it.
- Pipeline cache + shader prewarming → no first-shot hitches.
- Optional: ray-query shadows/AO behind wgpu's experimental features on capable GPUs.

### Phase 3: vendor extras via the escape hatch
Through `wgpu`'s `as_hal` interop in `kiln-platform`, without leaving wgpu:
| Feature | Windows DX12/Vulkan | Linux Vulkan | macOS Metal |
|---|---|---|---|
| Upscaling | DLSS (NVIDIA; Bevy 0.20 has DLSS-RR), FSR (AMD, open), XeSS | FSR, DLSS (Vulkan) | MetalFX |
| Low latency | NVIDIA Reflex, AMD Anti-Lag 2 | Reflex (Vulkan) | present timing |
| Profiling | PIX, Nsight, RGP markers | RGP / Nsight / RenderDoc | Xcode GPU capture |

## Platform matrix (via wgpu)
| OS | Primary backend | Fallback | GPUs |
|---|---|---|---|
| Windows 10/11 | DX12 | Vulkan | NVIDIA, AMD, Intel |
| Linux (incl. Steam Deck native) | Vulkan | GLES | AMD (RADV), NVIDIA, Intel |
| macOS / iOS | Metal | — | Apple Silicon, AMD (Intel Macs) |
| Web (later) | WebGPU | WebGL2 | browser |

## Risks
- **Renderer scope creep** (Phase 2): shadows, PBR, post-processing, and tooling take years to mature. Mitigate: only start it with profiler evidence, and keep Bevy's renderer as a fallback plugin.
- **wgpu experimental features** (ray tracing, mesh shaders) change between releases. Use them only behind feature flags with fallbacks.
- **Bevy upgrade churn:** pin exact versions and upgrade once per Bevy release in a dedicated branch.
- **Team size:** C is one GPU-literate engineer for months; D is a team.

## Sources
- Bevy 0.20 release notes: https://bevy.org/news/bevy-0-20/
- Bevy 0.20 coverage (Solari on Metal, DLSS-RR, mesh shaders): https://zeli.app/story/50013610 , https://alternativeto.net/news/2026/10/bevy-0-20-brings-major-rendering-shading-and-ui-enhancements-to-open-source-game-engine/
- Bevy rendering overview: https://hackmd.io/@bevy/rendering_summary
- wgpu releases: https://newreleases.io/project/github/gfx-rs/wgpu/release/v29.0.1
- wgpu feature flags: local source `~/.cargo/registry/src/*/wgpu-types-30.0.1/src/`
- wgpu vs Vulkan overhead discussion (2021, anecdotal): https://redlib.hbubli.cc/r/rust_gamedev/comments/p2ihhz/wgpu_vs_vulkan
- Blade FAQ (wgpu-hal throughput): https://github.com/kvark/blade/blob/cda61274d271b70bc7174849266f0b344000aabf/blade-graphics/etc/FAQ.md
- DX12 vs Vulkan API overhead: https://www.anandtech.com/show/11223
- Tiny Glade stack (forum, unconfirmed): https://lemmy.korz.dev/post/1328354/2037060 , https://redlib.hbubli.cc/r/rust_gamedev/comments/1izyj0l/rust_graphics_libraries
- ashlar: `~/codes/synthai/ashlar` (README, docs/architecture.md, docs/adr/0001-native-ffi.md)

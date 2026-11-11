# Perf monitor

Code: the engine's `devtools/{perf, perf_sys, perf_stats, perf_overlay, session_log}` (moved out of
Pattern of Life in the engine port; Pattern of Life adds the `ai` / `weapons` spans and its metrics keys in
`crates/game/src/dev/mod.rs`, dev builds only). Toggle the overlay with F2, start with `--perf`, or send `perf.overlay`. Use
`perf.stats` for the latest snapshot and `perf.report` for the session summary. Every second a
`perf` line goes to `<data>/logs/session-*.jsonl`.

## Diagnosis (2026-10-19, M5 Pro, offscreen play, 120 fps)

Values were compared with ground truth (`ps -o time` deltas, `top`, `vmmap --summary`, `ioreg`
IOAccelerator `PerformanceStatistics` and `AGXDeviceUserClient` `AppUsage`) while the game was
running.

| Problem | Evidence | Fix |
|---|---|---|
| Device GPU % came from one driver read per second. The driver counter covers a very short window, so single reads jump between 0 and 100 | ioreg reads taken back to back: `57,100,43,57,55,0,54…`. The log showed `dev 4` and `dev 0` while the game alone used 60 % | The sampler now polls 8 times a second, averages the reads, then applies an EMA |
| No smoothing; CPU and GPU dipped every ~3 s | `cpu 142,145,83,146…` and `gpu 64,64,36,63…` | 1 Hz samples are smoothed with an EMA (α 0.3) and show min/max over the last 60 s. The dips no longer show up |
| fps was the mean over a 600-frame ring (5–10 s), so it lagged and disagreed with p50 | fps rose 46→78→91→100 over 4 s and stayed at "100" while p50 was 7 ms | fps is now frames divided by real seconds over the last second, plus an EMA. The percentiles still use the ring |
| Unnamed threads at ~15 % each (`thread-6477676`) | `sample` shows them as Metal GCD workers (`com.Metal.CommandQueueDispatch` / `CompletionQueueDispatch`) | Labelled `gcd worker`. Bevy pools are shortened to `compute N` / `async N` / `io N`, plus `trainer`, `evolve`, `audio`, and per-pool sums |
| CPU % had no unit: was 140 % per core or of the machine? | — | Labelled "% of one core (15 cores = 1500 %)", with "= x % machine" added |
| No process-wise view. sysinfo returns **0 processes**, because Bevy enables sysinfo's `apple-app-store` feature, which stubs process listing | Unit test `proc_probe_sees_this_process` | On macOS the probe uses libproc directly (`proc_listallpids`, `proc_pid_rusage`, `proc_name`); other platforms use sysinfo. `ri_user_time` is in mach ticks and is converted with `mach_timebase_info` |
| GPU ms/frame divided the sampler's GPU delta by the main thread's frame count. The two windows are not aligned | — | Now computed as process GPU busy fraction × mean frame time |
| Missing: renderer/tiler split, device-allocated GPU memory, per-process GPU | — | Added |
| Correct before and after: footprint, RSS, process GPU %, Metal allocations | Footprint 991 vs vmmap 992.8 MiB. Process GPU 60 vs 62 % | — |

**Ground truth after the fix** (one 21 s window, the monitor's 1 Hz values averaged):

| Metric | Monitor | Ground truth |
|---|---|---|
| Process CPU, % of one core | 161.7 | 167.9 (`ps` cputime delta) |
| Process GPU % | 59.5 | 61.0 (AppUsage delta) |
| Device / renderer / tiler % | 54.9 / 49.5 / 27.0 | 57.4 / 51.5 / 28.4 (mean of 220 ioreg reads) |
| GPU in use, MiB | 1390 | 1394 |
| Footprint, MiB | 973 | 970.8 (vmmap) |
| RSS, MiB | 535 | 538 |

`top -l 2` reported 108–146 % for the same window. Its one-second sample reads low, so trust
cputime deltas.

**Limits.** Processes owned by other users (root, `_windowserver`; 258 of 628 pids here)
can't be read without root, so they have no CPU or RAM figure. They still appear in the GPU table,
because IOKit lists every client. `powermetrics` needs sudo and isn't used.

## How each number is computed

All sampling runs on the `perf-sampler` thread. The frame does only cheap work: it pushes the
frame time, wraps EnemySet, physics and WeaponSet in span timers, and once a second sends an
aggregate over a channel.

- **fps**: frames divided by real seconds over the last 1 s window. **avg ms** is 1000 / fps.
  **p50/p95/p99** come from the last 600 frame times; **max** is the worst frame of the last second.
- **CPU process**: `getrusage(RUSAGE_SELF)` user+sys delta divided by wall time, as % of one
  core. **machine %** is that value divided by the core count.
- **Threads**: `task_threads` and `thread_info(THREAD_EXTENDED_INFO)` CPU ns delta, as % of
  one core. Main and render are identified by thread id. Other names come from `perf_stats::thread_label`.
- **RAM footprint**: `proc_pid_rusage` `ri_phys_footprint`, the same figure as Activity Monitor's
  "Memory" and vmmap. **rss** is `ri_resident_size`. MiB everywhere.
- **GPU device / renderer / tiler**: IOAccelerator `PerformanceStatistics`
  `Device|Renderer|Tiler Utilization %`, polled every 125 ms and averaged per second. Device
  utilisation covers all apps.
- **GPU this game**: the delta of `accumulatedGPUTime` (ns), summed over this pid's
  `AGXDeviceUserClient` children, divided by wall time. **ms/frame** is that fraction × avg frame ms.
- **GPU memory**: game = `-[MTLDevice currentAllocatedSize]`. device = `In use system memory`
  / `Alloc system memory`.
- **Top processes** (default 6; set `SHROUD_PERF_TOP` to change): libproc CPU delta (% of one core) and
  footprint, plus per-pid GPU % from the IOKit clients. The tables are ranked by CPU, by RAM and by
  GPU. The game is always listed and marked `>` / `me`.
- **Smoothing**: every metric is a `Stat {now, ema, min, max}`, where min/max cover the last 60
  samples. The overlay shows the EMA and draws 60 s sparklines (60×32 textures).
- **perf.report**: min, avg, p95 and max of every 1 Hz metric over the session. The result is also written
  to the log as a `perf.report` line.

## Budget

Measured with the overlay on, from the session log's `overhead_us`:

- Main-thread cost: 1.5–3 µs/frame on average, at most 94 µs in the worst frame (the frame that rebuilds the overlay).
  The budget is 300 µs.
- With the overlay off: about 1 µs on average, 17 µs in the worst frame.
- Sampler thread: about 1.8 ms per second, off the frame (≈0.2 % of one core).
- The overlay rebuilds at most 4 times a second, and only when a new snapshot has arrived (so in practice once a second).
  Text and colour are written only when they change.

`overhead_us` does not include Bevy's own text relayout after a change.

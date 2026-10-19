# Player model — learning *you*

`brain::player_model::PlayerModel` is a decayed profile of how you play, built from per-match
telemetry (`game/src/telemetry.rs` → `MatchTelemetry`).

## What gets recorded (sampled at 10 Hz unless noted)
| Signal | Representation |
|---|---|
| Position heatmap | 32×32 grid (2 m cells), time share per cell |
| Routes | cell-to-cell transition counts (coarse 8×8 grid) |
| Death spots / kill spots | heat grids (event-based) |
| Engagements by range | 6 bins: 0–5, 5–10, 10–15, 15–20, 20–30, 30+ m; shots, hits, kills per bin |
| Strafe bias | signed lateral movement while shooting, −1 (left) … +1 (right) |
| Reaction time | enemy enters view → first shot at it (ms) |
| Camping ratio | share of time with speed < 0.5 m/s |
| Crouch usage | share of time crouched while engaged |
| Facing | per-zone histogram of where you look (8 sectors) |
| Accuracy / headshot rate | totals |

## Update rule
After each match: `model = lerp(model, match, α)` with α = 0.4 for the first 3 matches, then 0.3.
New habits show up within 2–3 matches, and old ones fade. `confidence = 1 − 0.6^matches_observed`
scales how much enemies trust it (multiplied by each enemy's `prediction_trust`).

## How enemies use it
| Use | Mechanism |
|---|---|
| Ambush | predicted position = heatmap ⊗ route transitions from last known cell |
| Fight at your weak range | utility Push/Attack shift `preferred_range` toward your lowest-accuracy bin |
| Flank your blind side | flank side = sector you watch least in your current zone |
| Dodge your aim | NEAT input 13 (your accuracy at this range); strafe against your strafe bias |
| Ghost player | trainer's simulated opponent copies routes, ranges, strafe, reaction, accuracy |

## Insights (shown on the debrief)
Generated from the model, top 3 by confidence × how unusual the value is. Examples:
- "They noticed you hold **the south yard** 41% of the time."
- "You're deadliest at **10–15 m** (48% accuracy). They'll avoid that range."
- "You strafe **left** 70% of the time when shooting."
- "You react in **~310 ms**. Expect them to peek and pull back faster."
- "You camp a lot (55% of the time stationary). Expect more flanks."

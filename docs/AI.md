# Enemy AI

Each enemy is an `EnemyMind` built from **one trait genome + one NEAT genome**, and it can read
the shared **player model**. Three layers run every tick:

| Layer | Decides | Rate | Evolved by |
|---|---|---|---|
| Traits | personality / style | fixed per match | genetic algorithm |
| Utility AI | goal + destination | ~4 Hz (or on events) | uses traits + player model |
| NEAT micro | strafe, peek, fire gate, aim lead | every tick while engaged | NEAT |

## 1. Trait genome
Each gene is stored in [0, 1] and mapped to a real range. Ranges marked * are clamped by fairness limits.

| Gene | Range | Effect |
|---|---|---|
| `aggression` | 0–1 | weight on Push/Attack vs Cover/Retreat |
| `preferred_range` | 6–30 m | distance they try to fight at |
| `flank_bias` | 0–1 | how willing to take Flank routes |
| `cover_affinity` | 0–1 | weight on TakeCover, crouch-peeking |
| `reaction_time` | limit–700 ms* | delay from first sight to first shot |
| `aim_error` | limit–8°* | base aim spread (gaussian sigma) |
| `burst_length` | 2–10 shots | before re-aiming |
| `retreat_health` | 0–0.6 | HP fraction that triggers Retreat |
| `team_cohesion` | 0–1 | spacing, waiting for allies before pushing |
| `prediction_trust` | 0–1 | how much they act on the player model (ambushes) |

**GA:** population 24, 5 deployed per match (top 3 elites + 2 exploratory). Tournament selection (k=3),
uniform crossover, gaussian mutation (σ = 0.08, rate 0.2 per gene), 2 elites carried over.

## 2. Utility AI (tactics)
Each goal gets a score in [0, 1] from curves over the observation, traits, and player model. The highest wins.
Hysteresis (+0.1 to the current goal) prevents flip-flopping.

| Goal | Rises with | Destination |
|---|---|---|
| **Attack** | player visible, HP high, near preferred range | hold; NEAT micro handles movement |
| **Push** | aggression, player HP low, player reloading | path toward player to preferred range |
| **Flank** | flank_bias, allies already engaged, player facing away | point 90° off the player's facing, on the side the player model says they watch least |
| **TakeCover** | taking fire, HP lost, cover_affinity | nearest cover point whose block cuts LOS from the player |
| **Retreat** | HP < retreat_health | cover point far from the player, near allies |
| **Ambush** | player unseen, prediction_trust × model confidence | cell at preferred range with LOS to the player's predicted spot (heatmap + routes) |
| **Hunt** | player unseen (fallback) | last known position / heard shot |

Squad rules: at most 2 flankers at once, and spacing ≥ 3 m (scaled by cohesion). Allies' goals are shared in `Observation.allies`.

## 3. NEAT micro (combat moves)
Active while the player is visible or seen in the last 1.5 s. Movement is expressed in the
**engagement frame** (forward = toward player), so behavior generalizes across positions.

**Inputs (14 + bias)**
| # | Input |
|---|---|
| 0 | distance to player / 30 |
| 1 | (distance − preferred_range) / 20 |
| 2 | player visible (0/1) |
| 3 | time since seen / 3 (clamped) |
| 4 | own HP fraction |
| 5 | ammo fraction |
| 6 | player HP fraction |
| 7 | player lateral velocity (engagement frame) / 6 |
| 8 | player radial velocity / 6 |
| 9 | player is aiming at me (cos of angle, from player facing) |
| 10 | taking fire (0/1) |
| 11 | nearest cover distance / 10 |
| 12 | nearest cover lateral side (−1/1) |
| 13 | player-model accuracy at current range (0–1) |

**Outputs (4, tanh)**
| # | Output | Use |
|---|---|---|
| 0 | forward | −1 back off … +1 close in |
| 1 | lateral | strafe left/right |
| 2 | fire gate | > 0: allowed to shoot when aim is settled |
| 3 | crouch/peek | > 0.3 crouch; oscillation = peeking |

Final move = `path_dir × w_path + micro × w_micro`, with `w_micro` = 1.0 in Attack, 0.4 in Push/Flank, and 0 otherwise.

## 4. Aiming (human-like)
- On first sight, wait `reaction_time` (+ random jitter of ±20%) before the first shot.
- Aim point = player pos + player vel × lead, with lead learned implicitly through NEAT positioning plus a fixed lag model.
- Spread σ = `aim_error` × (1 + own speed / 3) × settle factor. Settle starts at 2.0 and decays to 1.0 over 0.6 s on target.
- The aim turns at a capped rate (≤ 360°/s), so a fast flick by the player breaks their aim.

## 5. Fairness limits (`limits.rs`)
Floors that loosen with generation `g` and never go below human-plausible values:

| Limit | Formula | Gen 0 → Gen 20+ |
|---|---|---|
| min reaction time | max(180, 450 − 15·g) ms | 450 → 180 ms |
| min aim error | max(1.2, 5 − 0.25·g)° | 5° → 1.2° |
| enemy damage per hit | 8 + 0.25·g, cap 12 | 8 → 12 |
| squad size | 5 (fixed in prototype) | — |

The point: most of the difficulty increase must come from **better decisions** (traits, tactics, micro, prediction),
not from raw stats.

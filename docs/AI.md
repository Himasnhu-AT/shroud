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
| `reaction_time` | inert (fixed: limit × 1.3) | delay from first sight to first shot; not evolved |
| `aim_error` | inert (fixed: limit × 1.3) | base aim spread (gaussian sigma); not evolved |
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
Fixed for every generation (since 2026-11-03): evolution improves **technique only**
(positioning, flanking, cover, timing, teamwork, weapon choice, NEAT micro), never raw mechanics.
Difficulty is the only scale (`difficulty.rs` multiplies these).

| Limit | Value (before difficulty) |
|---|---|
| min reaction time | 440 ms (enemies use × 1.3; × 0.6 in combat, floored at the limit) |
| min aim error | 5° (enemies use × 1.3) |
| enemy damage per hit | 8 (rifle, scaled per gun) |

`ReactionTime` and `AimError` genes are kept in the genome so old saves load, but `decode` ignores
them and the debrief hides them (`Gene::is_technique`). `tests/brain.rs` checks that evolution
still beats fresh squads under Recruit mechanics (54% → 77% squad round wins).

## Squad tactics v2 (learned plays, anchors, utility, difficulty)

- **Play selection** (`brain::bandit`): disjoint LinUCB over Crossfire / PinFlank / Trade / Bait.
  Context: rush vs hold, zone focus, re-peek rate, strafe bias, his range, his accuracy, squad
  alive/hp, his hp. The old hand weights are the prior (fading with data). Reward per engagement
  window (<= 9 s or until the play changes): damage dealt, player forced back, minus untraded
  losses. Stored in `BrainState::tactics`, copied into the coordinator each round and back on
  round exit; forgets 15% per match. `ai.squad` shows context and arm scores.
- **Anchors**: the coordinator tracks re-peeks (lost at a spot, seen there again within 5 s,
  3 m). A member with a view holds and pre-aims the spot; after two re-peeks it pre-fires a
  ~0.3 s burst (1.6x spread, glint telegraph) in a window around his usual delay.
- **Breachers**: frag a player camping / hugging cover (after ~2-3 s still), flashbang ahead of a
  flank or entry. Wind-up, 7 s squad cooldown, one of each per member, per-round budget by
  difficulty (`Difficulty::grenades`). Game: `enemy/grenade.rs`; trainer: `grenade::SimGrenade`
  (frag LOS damage, ghost dashes to cover, flash blinds the ghost), so evolution trains with it
  (willingness keys off aggression / team cohesion genes).
- **Body language**: lean out past tall corners (eye, hitboxes and model shift), crouch-peek
  behind low cover, habit callouts ("He keeps re-peeking that crate - holding it",
  "He's camping the east lane - frag out").
- **Difficulty**: squad size, limits, coordination, attack-token cap and grenades come from
  `Settings.difficulty` at each round spawn; more enemies than evolved slots reuse genomes.
- The background trainer cycles over every map.

## Squad tactics v3 (split search, report and encircle)

- **Search** (`brain::search`, `Play::Search`): whenever the player is unknown (unaware, or
  lost for > 3 s) the squad splits up instead of hunting one shared point. Spots are local
  maxima of the belief grid plus cover near the hot spot that nobody can see into; a spot
  someone looks into (or reaches) is off the list for 14 s. Greedy assignment with
  reservations: mass - path distance - repulsion between targets (sigma 10 m) + a bonus inside
  the member's own geodesic Voronoi region; softmax over each member's top 3 (seeded), and a
  member keeps its spot until it is searched. At most two members (the closest by path) check
  where he was last seen / heard, along different routes. Callouts: "Lost him - spreading
  out", "Checking the yard", "Checking that noise".
- **Encircle** (`brain::encircle`, `Play::Encircle`): a fresh contact is radioed (no wallhacks:
  the board only knows what someone saw, heard or felt). The spotter(s) (1, or 2 in a squad of
  4+) keep fighting on their own; the rest take distinct approach sectors: flood the nav grid
  from him to 8-15 m, bin by bearing, one staging cell per sector (out of his sight, a step
  from a line of fire). Exhaustive assignment minimising path length + k * angular crowding
  (shooters included), hysteresis 15%, commit 2.5 s, softmax over the best 3. Routes are planned
  in turn with a cost band around each committed route and a cost on his line of fire toward
  the guns, and reach the mind as `Order::via`. When everyone is staged (or after 7 s) they
  all peek at once ("All set - go go go"); 4 s later the bandit's plays take over.
- **Stagger**: on a play change each member (except the one who sees him) reacts after
  0-0.5 s. Not applied to `force_play`.
- Metrics: `tests/squad_spread.rs` (search spread / coverage, encircle angular gap / shared
  route cells, all maps) and `examples/squad_bench.rs` (ghost win rate per map and size).

## Individual bot tactics (shoot on sight, posts, toughness)

- **Shoot on sight.** Any member with a clear shot fires once its reaction passes. The
  reaction is latched when a sighting starts: in combat it is `0.6 × reaction_time`, caught
  unaware `1.8 ×`, and never below `Limits::min_reaction_s`. Attack tokens no longer hold
  fire. They decide who shoots at full accuracy: a member without one fires with
  `SUPPRESS_SPREAD` (1.6×) wider spread unless it is under fire or within 12 m. Tokens still
  damp Attack/Push in utility. First shots are 1.6× wider and narrow over 0.6 s on target.
- **Posts (`posts.rs`), peek-shoot-hide.** A member is *isolated* when no living ally is within
  `ISOLATION_M` (15 m, about 4 s on foot). An isolated member in contact works cover posts. A
  post is a cover point within 8 m that hides it crouched, with a peek cell 0.8–1.2 m sideways
  (or standing up, for low cover) that sees the player. Posts are penalised for walking toward
  him, crossing his view, and being within 3 m of peeks from the last 10 s. The loop is: move,
  hide 0.3–0.8 s, peek with a burst of 0.4–0.9 s (leaning, `Action.lean`), hide 1.2–3 s
  (reloading behind cover), then move to a new post at least 2.5 m away. Reload or hurt
  windows (`player_hp < 0.35`) drop the loop so it pushes. Members without a token, and any
  member in TakeCover, run one cycle instead of standing in the open. Sticky orders are never
  interrupted.
- **No hive mind.** Radio updates to the player's position reach each member after its own
  seeded 0.2–0.6 s delay, while its own sightings are instant. Burst lengths jitter ±30%.
- **Toughness.** Strafes are biased toward the nearest cover. The net's crouch call holds for
  0.5–1.1 s. A member reloads behind cover within 4 m instead of in the open. A badly hurt
  player who breaks line of sight gets pushed ("finish").
- Bench: `memgate -- cargo run -p brain --example toughness -- 40` (gen 0, ghost v2, grenades
  off, all maps × difficulties). It reports how often the player kills the whole squad. Before
  these changes: 0.542. After: 0.475. The sim also counts its 60 s timeouts as player wins;
  by that measure it is 0.682 → 0.678, because hiding solo bots run the clock out more often.
  The game itself has no round clock.

## Adaptive doctrine (player archetypes → squad counters)

`brain::doctrine`, run by the squad coordinator (spec §5). Technique only: counters change
where enemies stand, how they peek and when they throw, rush or wallbang. Aim error, reaction
and damage stay fixed by difficulty (`limits.rs`).

- **`PlayerStyleProfile`** (fed every tick by the game, `enemy::think_and_act`, and by the
  trainer from the ghost): EMA horizontal speed (τ 1 s), seconds standing still on one angle
  (±20°, 1 m drift), ADS share of shots and doorways crossed at a run (≥ 3.5 m/s) over 20 s;
  headshot rate and reloads with > 50% of the mag left over 60 s (they are sparse).
- **Archetypes** (several at once, hysteresis): angle holder (speed < 1.2 and hold > 4 s; off
  above 2 m/s), entry fragger (speed > 4.2 and a room entry at a run; off below 2.5 m/s or no
  entry in 20 s), compulsive reloader (≥ 2 early reloads; off at 0), precision aimer
  (> 45% headshots over ≥ 4 hits; off below 35%).
- **`AiTactic`**: the active archetype with the highest severity × strength picks the
  doctrine (`JigglePeekAndFlash`, `OffAngleTrap`, `ReloadPunish`, `LeanPeekStagger`, else
  `Default`); 1.5 s dwell, a rival must score 1.3× higher. Each switch is a callout, an
  `ai.tactic {from, to, reason}` info line, a session-log event and a trace event.
- **Counters** (each archetype's counter is on while it is active, several at once):
  - Angle holder: a member walks (≤ 14 m) to a corner hidden from him with a peek cell a step
    aside and jiggle-peeks (0.25 m out and back in ~0.12 s, lean on the step, then a 0.35–1.1 s
    wait); the next grenade is a flash before any frag (the difficulty's budget and the 7 s
    cooldown still rule: Recruit/Regular throw nothing); a member with a penetrating gun and
    no sight of him fires a 0.7 s pre-fire burst through thin cover (`Arena::penetration`, the
    same rule as the player's bullets; damage scaled by what gets through). Enemy bullets now
    penetrate like the player's. **Lights**: if lamps light his spot (`Arena::lamps_lighting`:
    lit, within the lamp's reach, a clear line), one member out of contact walks (≤ 14 m) to a
    spot that sees the fixture under every lintel (`Arena::clear_line_3d` from muzzle height,
    also a step around the spot) but not him (`doctrine::light_spot`), and taps it (one aimed
    shot per 0.35 s, the same aim error as any shot; the game aims from the muzzle) until it
    breaks: `Role::Lights`, "Shooting out his light". Up to two lamps a round, 15 s each; the
    jiggler waits at its hide spot meanwhile. No lamps (Sirocco, Gantry streets): nothing.
  - Entry fragger: one or two members out of contact hold a spot 3.5–8 m inside a door he
    would come through, 35–80° off the door line, hidden from outside it
    (`doctrine::off_angle_spot`); adopters drop to a crouch as he comes into view.
  - Compulsive reloader: members that hear his mag come out (`MAG_OUT_RANGE` 14 m, walls
    muffle it) send the nearest one (two with a strong counter) sprinting at him (`Rusher`,
    3 s).
  - Precision aimer: adopters fight from posts (lean-peek, hide, re-peek elsewhere) and strafe
    in short irregular bursts (0.12–0.42 s) instead of an A-D rhythm.
- **Evolution**: four doctrine genes per trait genome (`TraitGenome::doctrine`, old saves
  seed them like the loadout genes). The squad's mean gene g gives strength 0.25 + 0.75 g:
  how soon it flashes, how often it jiggles and wallbangs, one or two rushers / trappers,
  and which members adopt crouch/stagger counters (a member adopts when its fixed roll < s).
  The trainer profiles the ghost the same way, so genes for the styles the ghost shows get
  selection pressure (the ghost never ADSes and has a fixed headshot rate, so the precision
  gene mostly drifts).
- **Across matches**: seconds each archetype was active go into `MatchTelemetry::style_time`
  and the model's `style` summary (EMA). A habit active ≥ 35% of last matches starts the next
  round's doctrine for 25 s unless the live profile confirms it. The debrief leads with one
  line ("They noticed you hold angles; they started jiggle-peeking and flashing.").
- **Debug**: F3 shows a doctrine panel (tactic, reason, archetypes, profile) and draws the
  jiggle spot, wallbang line, rush arrows and traps; `ai.doctrine` on the control port;
  `enemy.list` has `doctrine` and `jiggling`. Check: `scripts/doctrine-camper.json`
  (Veteran, Sirocco, player holds a doorway): switch at 4.0 s, a jiggler, two flashes.
- Tests: `crates/brain/tests/doctrine.rs` (detection, spec check 6, each counter, saves).

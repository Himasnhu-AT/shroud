# Evolution pipeline

Runs in the **Evolving** state on a background thread (target ≤ 10 s), with a progress bar.

## Steps
1. **Ingest:** `MatchReport` → `PlayerModel.ingest()`.
2. **Real fitness:** per deployed enemy (trait id + NEAT id):
   `fitness = 1.0·damage_dealt + 40·kills + 0.5·seconds_alive + 0.3·seconds_engaged − 20·(never_engaged)`.
   Normalized per match.
3. **Offline training vs ghost:**
   - Build a `GhostPlayer` from the model (see below).
   - Run NEAT generations: each generation evaluates every NEAT genome in ≥ 3 rounds (5 enemies per round,
     paired with trait genomes sampled from the elites). Budget: ~15 NEAT generations, or stop at the time cap.
   - Trait genomes get averaged sim fitness from the same rounds.
4. **Blend:** trait fitness = 0.6·sim + 0.4·real for deployed genomes; others use sim only.
   NEAT fitness from sim; deployed NEAT genomes get the real bonus.
5. **GA step** on traits; the NEAT population has already advanced in step 3.
6. **Pick the next squad:** top 3 traits + 2 exploratory, each paired with one of the top NEAT genomes (champion + species reps).
7. **Save** `brain.json`, then go to Debrief.

## 2D trainer sim (`brain::sim`)
- Same `Arena`, nav grid, LOS rules (low cover blocks only crouched), and `EnemyMind` as the game.
- Fixed dt 0.05 s, rounds up to 60 s, collision via `Arena::resolve_collision`.
- Hitscan resolved by spread angle vs target angular size; damage matches the game.
- Deterministic with a seed → reproducible tests and `--sim-bench`.

## Ghost player
A scripted opponent with its parameters taken from the player model:
- Picks destinations by sampling the heatmap; moves along modeled routes.
- Engages at modeled preferred range, strafes with modeled bias, crouches at the modeled rate.
- Reaction time and per-range accuracy come from the model (+ small noise).
- Before any real match (no model yet): a default "average player" profile.

## Save format (`brain.json` in `~/Library/Application Support/shroud/`)
```json
{
  "version": 1,
  "generation": 7,
  "seed": 12345,
  "traits": { "population": [...], "deployed": [0, 3, 5, 11, 19] },
  "neat":   { "population": { ... }, "deployed": [ ... ] },
  "player_model": { ... },
  "history": [ { "match": 6, "player_won": true, "rounds": [2, 1], "avg_fitness": 113.2, "insights": [...] } ]
}
```
Corrupt or old-version file → back it up as `brain.json.bak` and start fresh. A menu option "Reset AI" does the same on purpose.

## Success criterion (testable without a human)
In the sim, against a fixed ghost, **gen-10 squads beat gen-0 squads** (higher win rate and damage dealt, seed-averaged).

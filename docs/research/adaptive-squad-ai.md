# Learning One Player: Architecture for an Adaptive Enemy Squad in Shroud

*Research report, October 2026. Scope: `crates/brain` and the parts of `crates/game` that feed it.*

## Abstract

Shroud's enemy squad is supposed to get better at beating one particular player from match to match. Right now it doesn't do that convincingly. Players win 2–0 again and again at generation 8, and the debrief insights are things like "you hold the south yard 70%". This report reviews published work on learned and adaptive game agents: NVIDIA's ACE and GPU simulation work, DeepMind's Capture the Flag agents, GT Sophy, OpenAI Five, Forza's Drivatars, learned bots at Ubisoft and EA, Microsoft's WHAM, neuroevolution and quality diversity, player modelling, dynamic difficulty, shipped tactical FPS AI, and bot believability. The main conclusion is that **none of the systems that reached human or superhuman level did it with a handful of games of data.** They used hundreds of thousands of games (FTW: 450K [8]), days of console time (GT Sophy: 10–21 PS4s for 7–12 days [10]), or months of distributed self-play (OpenAI Five: 10 months [11]). Shroud gets roughly 2–3 rounds of real data per match. The project's own measurement shows where the current loop breaks. Against the default scripted ghost, the squad's simulated win rate climbs from about 4% to 90–100% by generation 8, yet the real player still wins every generation 2–0. Simulation progress does not transfer. So the architecture should not try to learn a policy from the player's matches. It should **learn a model of the player** from those matches, then spend cheap, unlimited simulation **planning and evolving against that model**, and pick among a diverse archive of ready-made squad styles using a sample-efficient online bandit. The central recommendation is a **high-fidelity player clone for training**. It is behaviour-cloned from the player's recorded trajectories and duels, calibrated so that clone-versus-squad outcomes match real outcomes, and validated each generation by the gap between simulated and real win rate. Around it we propose: a situation-conditioned Bayesian player model; a real squad layer (shared belief, roles, bounding overwatch, crossfires, and Killzone-style position evaluation whose weights evolve); a lower-noise training loop built on a MAP-Elites archive and an opponent pool; optional small-scale RL; explicit fairness limits; and a CPU-first compute plan for a 24 GB Apple M5 laptop. We also give an evaluation protocol that separates "smarter" from "stronger", a method for producing statistically sound counterfactual debrief insights, and a prioritised roadmap mapped onto the existing modules.

---

## 1. Problem statement

**Goal.** Build an enemy squad (5 agents) that, over a small number of matches against one human:

1. measurably improves its win rate **against that human specifically**, and not just in general;
2. does it through **decisions** (positioning, timing, coordination, prediction) rather than stats (aim, reaction, damage), with human-plausible limits;
3. makes its adaptation **legible**, so the player can tell the squad learned something;
4. produces **accurate, actionable** feedback about the player's habits.

**Constraints.** About 2–3 rounds of real data per match, with a 60-second cap per round (`sim.rs: ROUND_TIME`) and a best-of-3 format (`match_flow.rs: ROUNDS_TO_WIN = 2`). That is a few hundred engagement seconds per match. Training runs locally on a laptop in seconds to minutes. The game is a Rust/Bevy binary. All learning stays on the player's machine.

### 1.1 Why the current loop plateaus

Reading `evolve.rs`, `sim.rs`, `mind.rs`, `utility.rs` and `player_model.rs` turns up six structural causes. None of them is a tuning problem.

| # | Cause | Where | Consequence |
|---|---|---|---|
| D1 | **The ghost is not the player.** The ghost hits by drawing a Bernoulli per shot at its modelled accuracy, faces the way it walks, never pre-aims an angle, never deliberately uses cover or peeks, and chooses destinations by sampling the *marginal* heatmap. `ghost_skill` calibration adjusts a single scalar. | `sim.rs: Ghost`, `run_round` | The squads are optimised against an opponent that doesn't exist. **Measured:** `cargo run -p brain --example sim_bench -- 8 2` shows the squad's win rate against the default ghost rising from about 4% to 90–100% by generation 8, while the real player wins every generation 2–0. The sim-to-real gap is about 0.9 in win rate. Once squads beat the ghost nearly every time, fitness also saturates and selection loses its gradient. |
| D2 | **The model is marginal, not conditional.** Heat, routes and per-range accuracy are averaged over all situations. A player's behaviour is conditional: what they do when shot from the side, when two enemies are visible, right after a kill. | `player_model.rs` | Prediction stays crude, and insights can only describe where the player stands, not what they do or why they lose. |
| D3 | **Evaluation is too noisy for selection.** 40 NEAT genomes each play about 3 stochastic 60-second rounds per generation, paired with randomly sampled trait genomes. The trait GA steps once per real match, so generation 8 means 8 GA steps. | `evolve.rs: evaluate`, `EvolveConfig` | Fitness differences fall inside the noise, so selection is close to a random walk. "Generation" counts matches, not compute. |
| D4 | **The objective rewards the wrong thing.** Individual damage, kills and time alive, plus a flat +30 for a squad win. Nothing rewards setting up a teammate's kill, trading, or exploiting a *player-specific* weakness. | `sim.rs: EnemyStats::fitness` | Evolution drifts toward safe, individualistic play. |
| D5 | **The behaviour space lacks the behaviours players read as smart.** Evolution tunes 10 scalars and a 14→4 micro net. The tactics themselves are hand-written utility curves. Coordination is now a radio blackboard (sightings shared every 0.35 s), reload-punish pushes and crossfire-aware flank selection (recently added in `mind.rs`/`utility.rs`), plus the ≤ 2 flanker cap and spacing. | `utility.rs`, `mind.rs` | Bounding overwatch, trading, pinning with suppression, pre-aiming the player's favourite angle, and collapsing on a sound can't be discovered because they can't be expressed. |
| D6 | **Adaptation is invisible.** There are no callouts, and the debrief doesn't show what the squad tried. | game UI | Even real adaptation goes unnoticed. Shipped games (Section 3.6) show that perceived intelligence depends heavily on legibility. |

The literature below explains why D1–D3 matter most, and D1 most of all. Every successful learned-agent system either had a faithful simulator of its opponent (the real game, self-play) or enormous amounts of data. Shroud has neither, so the fidelity of the player model is the bottleneck.

---

## 2. Method of this review

Primary sources were read where possible: arXiv papers, journal pages, and the original GDC/AIIDE slide decks, whose text was extracted from the PDFs. Claims marked **[unverified]** come from secondary sources or marketing copy that could not be checked against a primary technical description. Numbers are quoted as the sources state them.

---

## 3. Related work

### 3.1 NVIDIA

**We found no NVIDIA publication on combat NPCs that learn an individual player's tactics.** What NVIDIA has published is as follows.

- **ACE autonomous game characters (CES 2025)** [1]. The pitch is characters that "perceive, plan, and act like human players". The stack is on-device models: Mistral-NeMo-Minitron language models at 8B, 4B and 2B (the 2B "fits in as little as 1.5GB of VRAM"), plus audio, vision and retrieval models. The only adaptation claim is marketing copy saying MIR5 bosses "continuously learn from player behavior" **[unverified: no technical source]**.
- **PUBG Ally (KRAFTON, beta June 2026)** [2]. A quantised 2B language model (*System 2*) interprets speech and events and issues intents. A behaviour tree (*System 1*) does movement, aiming and combat, so "reflex-level actions never have to wait for the model". Cross-match adaptation is *memory* (preferred weapons, drop spots). There is no RL or imitation of combat. (The CES page names 8B for Ally [1]; the 2026 write-up says 2B.) *Lesson:* the useful part is the fast/slow split and grounded memory. ACE is not a way to make a squad tactically smarter.
- **Isaac Gym** [3] runs simulation and training together on the GPU and reports 2–3 orders of magnitude speed-ups for robot RL. The transferable idea is batched simulation, also applied to game-style ECS by Madrona [51]. Section 4.6 explains why it mostly doesn't pay off for Shroud.
- **Research that is relevant, but not to squads:**
  - **ASE** [4]: adversarial imitation of motion clips into reusable skills.
  - **Eureka** [5]: an LLM evolves reward code, beating human-written rewards on 83% of 29 tasks.
  - **Voyager** [6]: a GPT-4 Minecraft agent.
  - **NitroGen** [7]: a ~500M-parameter vision-to-gamepad model behaviour-cloned from ~40,000 h of video across 1,000+ games, with up to 52% relative gain on unseen games.

  Together they show that imitation scales with *data*, which is the one thing Shroud doesn't have for a single player.

### 3.2 Population RL and self-play at scale

- **DeepMind FTW (Quake III CTF)** [8].
  - **Training:** a population of 30 agents over 450K games. Each agent optimised a *learned internal reward* (a table over 13 game events). Population-Based Training [9] selected those rewards and hyperparameters on *win probability*: an agent below 70% estimated win rate copied a better agent, then mutated. Opponents were matched by Elo.
  - **Fairness finding:** FTW reacted in 258 ms against 559 ms for humans, with 80% accuracy against 48%. With accuracy cut to human level it still won more often. Added delay up to 375 ms made no difference. Much of its edge was strategic.
- **GT Sophy** [10] used model-free RL, curated "mixed-scenario" training starts, and sportsmanship penalties in the reward. It acted at 10 Hz, with no gain above that. Training took 10–21 PS4s for 7–12 days.
- **OpenAI Five** [11] trained for 10 months. Self-play split 80% against the latest policy and 20% against past versions. Its "team spirit" τ blends individual and team-mean reward, r_i = (1−τ)ρ_i + τρ̄, annealed toward 1. Average reaction time was 217 ms.
- **AlphaStar** [12] used a league of main agents, *main exploiters* that target the current main agent, and *league exploiters*, under negotiated limits on action rate.

*Lessons for Shroud:*
- Select on winning and let shaping evolve (FTW).
- Curate training scenarios (Sophy).
- Play past opponents and use team spirit (Five).
- Treat the squad as an exploiter of a player model, kept robust by a pool (AlphaStar).
- Their data budgets (10⁵–10⁶ games) rule out learning a policy from real matches.

### 3.3 Learning from one player: Drivatars

Forza's Drivatar is the closest shipped analogue [13][14]. The original Microsoft Research system (2005) used small per-player Bayesian neural networks. The modern system splits tracks into segments of 12 turn types. It learns each player's speed and line per segment, and **falls back to the player's average for that turn type** on unseen tracks. Uploads are time-stamped so old data counts less. A control layer filters antisocial behaviour, and rewinds are excluded from training [13]. The modern internals are unpublished, and the claim that it uses RL is **[unverified]**. *Lesson:* use **hierarchical pooling with recency weighting**. That is the statistical shape of Section 4.1, with "segment" replaced by *situation*.

### 3.4 Learned bots in shooters, and world models

- **EA SEED** bootstrapped Battlefield 1 agents from about 30 minutes of human play, then trained them with RL for about 6 days [15]. SEED's later work uses RL mainly for **automated testing** in FPS games, including Battlefield 2042 [16][17].
- **Ubisoft La Forge** built RL navigation with player-like controls [18], and the **Minimax Exploiter** [19]: a data-efficient exploiter, tested in *For Honor*, whose reward uses the opponent's value estimates.
- **modl.ai and Riot** cloned human trajectories for a VALORANT-like 2v2 shooter. They used **ray-cast sensors instead of pixels** so inference runs on modest CPUs, and human evaluations found the bots human-like [22].
- **Counter-Strike behavioural cloning** from about 4M scraped frames reached medium built-in bot strength "whilst adopting a humanlike play style" [21].
- **Microsoft's WHAM/Muse** [20] is a 1.6B-parameter world-and-action model trained on about 7 years of *Bleeding Edge* play. It is a tool for gameplay ideation, not an NPC controller.

*Trade-off:* imitation is human-like but tops out at its data and needs far more than a few matches. RL is strong but needs days of simulation.

### 3.5 Player and opponent modelling, few-shot adaptation, difficulty

- **Player modelling** is surveyed in [29][30], and opponent modelling in [36].
- **Position prediction:** Hladky and Bulitko learned models from Counter-Strike: Source records that predict hidden opponents' positions, using only what a human could know, with "similar or better accuracy than human experts" [35]. This supports a **belief-state** squad.
- **Few-shot individual models:** fine-tuned per-player chess models predict and even identify individuals [38]. The 2025 follow-up says those needed about 5,000 games; its Maia4All method needs **about 20**, by initialising a per-player embedding from population prototypes [39]. *This is the template for a Shroud player clone:* a population prior plus a small per-player embedding.
- **Fast selection from a library:** Bayesian Policy Reuse [37] keeps a belief over which known task type it is facing and picks the policy from a library that handles it best, converging within a few episodes. For Shroud, the library is the squad archive and the task is this player.
- **Dynamic difficulty adjustment:**
  - Hunicke argued for DDA in games [31], and her Hamlet system for Half-Life adjusted item supply [32]. Chen's flow framing ties challenge to ability [33].
  - Spronck's **dynamic scripting** [34] adapted online within tens of encounters, and shipped as a *Neverwinter Nights* demonstration. It managed this because it learns only **weights over a fixed set of sensible tactics**.
- **Strong but human-like:** piKL [40] adds a KL penalty toward an imitation-learned human policy. It keeps human-prediction accuracy while gaining much strength.

### 3.6 Tactical FPS AI in shipped games

- **F.E.A.R.** [41] used GOAP: A* over actions, with a three-state FSM underneath. Squads were deliberately simple. A coordinator clusters AI by proximity, and each squad runs at most one of Get-to-Cover, Advance-Cover, Orderly-Advance or Search. Each behaviour fills slots and sends *orders*, for example one member suppressing while the others move. Orkin: the interplay "often create[s] the illusion of more complex squad behavior than what actually exists". Dialogue was picked *after* the decision, to explain it ("I've got nowhere to go!").
- **Killzone** [42] scored candidate positions with **weighted position evaluation functions**: proximity 20; line of fire to the primary threat, 40 with partial cover, otherwise 20; cover from other threats 20; preferred range 10. The weights are set per action and personality. A* link costs added exposure and crossing friendly fire. Suppression targets *the positions the threat wants to attack from*. Killzone 2 added HTN planning and strategy/squad/individual layers [43]. That thesis also states the industry worry: an AI that learns a new strategy each match is unpredictable and frustrating [43].
- **Halo** [44] playtests: "very intelligent" ratings rose from 8% for weak enemies to 43% for tough ones ("Tougher = Smarter"). The game had 166 dialogue types and 5,147 lines of combat chatter. Halo 2 introduced behaviour trees ([45], secondary source).
- **Left 4 Dead's Director** [46] tracks survivor "intensity" (damage, incapacitation, nearby kills; decays when not engaged). It cycles Build Up → Sustain Peak (3–5 s) → Peak Fade → Relax (30–45 s). It adjusts difficulty through *pacing*, not stats.

*Synthesis:* squads that ship and feel smart are **good positional reasoning, simple coordination and legibility**. These are the things Shroud can't currently express (D5, D6).

### 3.7 Neuroevolution and quality diversity

- **NEAT** [24] complexifies topologies, with speciation to protect new structure. **rtNEAT/NERO** [25] evolved combat teams *during* play under player-set fitness sliders. It is Shroud's direct ancestor, and it worked because fitness was dense and player-directed.
- **Novelty search** [27] shows that pure objective search gets trapped by deception.
- **MAP-Elites** [26] keeps the best solution per cell of a behaviour space. **CMA-ME** [28] found stronger and more diverse Hearthstone decks than either CMA-ES or MAP-Elites.

*Trade-off:* quality diversity spends evaluations on breadth, but it yields a **library of distinct, competent squads**, which is what a fast online selector needs.

### 3.8 Human-likeness and fairness

- **BotPrize** [47] ran a Turing test for Unreal Tournament bots. In 2012, UT^2 and MirrorBot were rated 52% human, against 40% on average for the human players [48]. UT^2 reportedly replayed human traces when stuck **[unverified detail]**.
- The **Navigation Turing Test** [23] learns a classifier that stands in for human judges on trajectories.
- Fairness limits in practice: Sophy's 10 Hz [10], Five's 217 ms [11], AlphaStar's action-rate caps [12], and FTW's accuracy and delay ablations [8].

### 3.9 Summary

| Approach | Data needed | Human-like | Fits a handful of matches? |
|---|---|---|---|
| Deep RL / self-play [8][10][11] | 10⁵–10⁶ games | only if constrained | **No.** Use offline, against a model. |
| Behaviour cloning [21][22][7] | hours to thousands of hours | yes | **Only with a population prior** [39] |
| Bayesian/pooled player model [13][35] | few matches | n/a | **Yes** |
| Weights over fixed tactics [34][42] | tens of encounters | yes | **Yes** |
| Policy library + Bayesian selection [37] | few episodes | depends on library | **Yes** |
| QD/NEAT offline vs a model [25][26] | sim only | depends on model | **Yes**, if the model is faithful |
| LLM characters (ACE) [1][2] | n/a | conversational | Memory and communication only |

---

## 4. Proposed architecture for Shroud

**Principle:** *Real matches are for **measuring the player**. Simulation is for **learning to beat them**. Deployment is a **bandit** over a diverse archive.*

```
 real match ──► telemetry v2 (engagement log, 10 Hz trajectories)
                    │
                    ▼
 ┌──────────── Player model v2 ─────────────┐
 │ L1 hierarchical Bayesian situation stats │──► insights v2 (debrief)
 │ L2 belief/motion model (where they go)   │──► squad belief grid (in match)
 │ L3 calibrated player clone (training opp)│
 └──────────────┬───────────────────────────┘
                ▼ calibrated opponent pool (clone posterior samples, past selves, archetypes)
 ┌──────────── Background trainer (CPU) ────┐
 │ MAP-Elites archive over squad styles      │
 │ genome = traits + PEF weights + micro net │
 │ fitness = team-spirit win prob vs pool    │
 └──────────────┬───────────────────────────┘
                ▼
   Thompson-sampling selector over archive cells ──► next squad
                ▲                                          │
                └──────── real round outcomes ◄────────────┘
```

### 4.1 Player model v2

**L0, the data substrate: an engagement log.** Add to `MatchTelemetry` one record per **duel**, meaning a contiguous period in which the player and at least one enemy have line of sight. Each record holds: start time; who saw whom first; the player's zone and cover state (exposed, low cover, peeking from tall cover, and which side); range bin; number of enemies visible; player HP and ammo; whether the player was moving, crouched or ADS; pre-aim error at first sight (angle between the player's crosshair and the enemy when it appeared); time to first shot; damage traded; outcome (win, loss, disengage); and whether the player re-peeked the same angle within 5 s. Also keep the raw 10 Hz trajectory (position, velocity, yaw, pitch, fire, crouch, HP) for L3 and for replay. The cost is a few kB per match.

**L1, hierarchical Bayesian situation statistics.** Replace the point-estimate EMAs with **conjugate posteriors and partial pooling**, which is the Drivatar segment → type → global pattern [13]:

- Duel win rate per situation key *s* = (zone, peek side, range bin, odds): a Beta posterior whose prior is centred on the parent level (zone-agnostic situation, then global), with prior strength κ (about 5–10 pseudo-duels). With n duels and w wins, the posterior mean is (w + κ·p_parent)/(n + κ). Sparse cells shrink toward their parent automatically, so no ad-hoc "≥ 5 shots" thresholds are needed.
- Reaction time: log-normal with a Normal-Gamma prior. Route choices: Dirichlet-multinomial over exits from each route cell, with the map-average prior.
- **Forgetting:** multiply all pseudo-counts by γ (about 0.7) after each match instead of lerping the means. Recent matches still dominate, but the **uncertainty is preserved**. That uncertainty is what makes insights trustworthy (Section 6) and what the bandit needs (Section 4.3).
- Confidence is now per statistic (posterior width), not a global `1 − 0.6^n`.

**L2, belief and motion model.** A semi-Markov model over route cells with dwell-time distributions, conditioned on round phase and the last contact zone. It drives a **particle filter** (a few hundred particles) of where the player might be, updated by sightings, heard shots, and *negative* information: if an enemy is looking at a cell and doesn't see the player, the player isn't there. Hladky's Counter-Strike results [35] suggest this kind of learned prediction can reach expert-human accuracy without cheating. It replaces `PlayerModel::predict`, and it feeds the squad blackboard (Section 4.2) and the ghost.

### 4.1.1 L3: a calibrated player clone for training (central recommendation)

The measured gap (D1) means every hour of training so far has optimised against the wrong opponent. A squad that wins 90–100% against the ghost and 0 of the real rounds has learned to beat the ghost. **Nothing else in this report pays off until the training opponent behaves, and loses, like the player.** The clone is that opponent. Every successful system in Section 3 had a faithful opponent, either the real game in self-play or a large dataset. For one player, Shroud has to build it from a few matches.

**What the clone is.** A simulated player whose *decisions* are fitted to the player's recorded behaviour (behaviour cloning) and whose *outcomes* are then calibrated so that it wins and loses against squads at the rate the real player does. It is built in two stages.

**Stage C1: structured behaviour cloning (recommended first).** Rather than one network, fit a small supervised model for each decision the ghost makes. Each is trained on the player's own decisions from the engagement log (L0) and 10 Hz trajectories, with hierarchical priors (L1) so sparse situations borrow strength:

| Clone component | Fitted from | Model |
|---|---|---|
| Where to go and how long to stay | Route-cell transitions and dwell times by round phase | Semi-Markov chain (L2), Dirichlet priors |
| Engage, hold, push, retreat | Duel records: odds, HP, ammo, just-got-a-kill, taking-fire-from-flank | Logistic heads with pooled priors |
| How to peek | Peek side per cover, re-peek within 5 s, crouch, strafe pattern while firing | Categorical/Bernoulli per situation |
| Where the crosshair rests | Yaw histogram per zone while not engaged; pre-aim error at first sight | Per-zone mixture over angles |
| Reaction and flick | First-sight → first-shot time; yaw-speed profile of the first 300 ms | Log-normal reaction; turn-rate and settle-time fit |
| Hit probability | Per-shot log: range, target lateral speed, own speed, time on target, angular error at first sight | Logistic regression (replaces today's per-range Bernoulli) |
| Awareness | How quickly the player turns to an enemy that shoots from outside the view cone; turning on sounds | Delay distribution |

This needs one telemetry addition beyond L0: **a per-shot record** (range, hit, target angular speed, own speed, time since the player first saw that enemy). It runs at the same tick rate and with the same `Arena`, LOS and damage rules as `sim::run_round`, so it slots in where `Ghost` is. Each head has a handful of parameters, so a few matches are enough, and the Drivatar pooling pattern [13] covers situations the player hasn't been in yet.

**Stage C2: neural clone (optional, gated).** A small policy π(a | o, z):
- **o** is an egocentric ray-cast observation as in [22]: about 32 rays, plus visible enemies, own HP and ammo, and damage direction.
- **a** is a discretised move × look × fire × crouch action.
- **z** is an 8–16-dimensional per-player embedding.

Pre-train the network on a *population* of C1 clones with sampled parameters, plus any recorded human sessions. Then fit only z for each player (the Maia4All recipe [39]). Behaviour cloning drifts once it leaves the data distribution, and DAgger-style corrections [49] aren't possible with a human. So C2 replaces C1 **only if** it wins on the validation metrics below.

**Calibration to outcomes.** Behaviour cloning matches *what* the player does. Calibration makes sure the clone *wins and loses* like the player. It works as follows.

1. **Store every deployed squad.** Save the genomes deployed in each real match (add them to `MatchSummary` in `history`), together with the real result. Each past generation becomes a labelled data point: (squad g, real rounds won and lost).
2. **Replay those squads against the clone.** For each stored squad, run it against the clone for N ≈ 200 seeded rounds. That gives a predicted player round-win rate p̂_sim(g).
3. **Fit a few residual parameters to the real results.** Use CMA-ES or coordinate search on 4–6 multipliers: aim scale, reaction scale, awareness delay, aggression bias, and pre-aim sharpness. Minimise
   L = Σ_g n_g · BCE(p̂_sim(g), real round wins_g / n_g) + λ · Σ_s n_s (duel-win_sim(s) − duel-win_real(s))².
   Squads from every generation spread across a range of strengths, so this fits the whole curve, not one point. It is the multi-dimensional successor to today's scalar `ghost_skill`. A scalar can't close the gap when the ghost lacks capabilities the player has, such as pre-aim, peeking and fast flicks.
4. **Train against the calibrated clone's posterior.** Use posterior samples of the clone (Section 4.3), biased slightly toward the stronger end, because humans also improve between matches.

**Validation: the sim-to-real dashboard, logged every generation.**

| Metric | Definition | Today (default ghost) | Target |
|---|---|---|---|
| **Per-generation outcome gap Δ_g** | p̂_sim(g) − p_real(g). p̂_sim is the clone's round-win rate against squad g; p_real is the player's real round-win rate in match g, as a Beta posterior because only 2–3 rounds are played. | ≈ 0.0–0.1 vs 1.0, so Δ ≈ −0.9 | mean \|Δ\| < 0.15 over the last 3 generations |
| **Predictive coverage** | Share of real matches whose round outcomes fall inside the clone's 90% predictive interval | — | ≥ 80% |
| **Rank agreement** | Spearman correlation between clone-predicted and real squad strength across generations (≥ 6 generations) | — | ρ > 0.5. Training only needs the ranking to be right. |
| **Duel calibration** | Brier score of per-situation duel win rate, sim vs real | — | < 0.2 |
| **Behavioural fidelity** | Held-out-match log-likelihood per decision head (route, peek side, reaction) vs the default ghost | baseline | better on every head |
| **Indistinguishability** | AUC of a trajectory classifier, clone vs player (ANTT-style [23]) | — | → 0.5 |

**Gate.** Sim fitness drives selection only while mean |Δ| < 0.15. When the gate fails, the trainer marks sim results as *untrusted*. Deployment then leans on the real-outcome bandit (Section 4.3), and the debrief's developer overlay shows the gap. That turns the current silent failure ("gen 8 sim says 100%, the player says 2–0") into a visible, measurable number that every later change has to improve.

### 4.2 Squad coordination

Shipped games show that legible squad structure does more for perceived intelligence than better individual micro [41][42][44]. **Already done** (in `mind.rs`/`utility.rs`): a squad blackboard that radios sightings every 0.35 s, pushes that punish the player's reload, and flank selection that picks the bearing which forms a crossfire with the engaged allies. The remaining items extend these in a new module `squad.rs`:

1. **Blackboard v2** (extends the radio blackboard): the particle-filter belief over the player's position (L2); *claims*, i.e. which cover points and angles are held by whom; the current squad *play*; and a short event log for callouts. Radio latency stays (`RADIO_PERIOD_S`), so knowledge is shared fairly.
2. **Plays with slots** (the F.E.A.R. pattern [41]). A squad coordinator picks at most one play per sub-squad and fills its roles; individuals keep their utility AI and treat orders as high-priority goals.
   - *Bounding overwatch:* A holds the angle the belief says the player will appear from, B moves to the next cover, then they swap.
   - *Crossfire:* flank selection already prefers crossfire bearings. Promote it to a play with two claimed positions whose lines to the predicted player spot differ by more than 60°.
   - *Trade:* a second member follows the entry at 4–8 m so that a player kill is immediately traded.
   - *Pin and flank:* suppression on the player's best attack positions (computed from the player's perspective, as in Killzone [42]) while one member flanks the *least-watched sector*.
   - *Collapse:* converge on a heard shot along different routes.
   - *Bait:* show one member at the player's weak range to pull them off cover.
3. **Position evaluation functions with evolvable weights** [42]. Score each candidate cover point by Σ_k w_k·f_k, with terms for:
   - line of fire to the belief-weighted player position;
   - partial cover;
   - proximity;
   - preferred range;
   - **player's duel win rate at that range and side (from L1)**;
   - angular separation from teammates;
   - exposure to the player's habitual pre-aim angles;
   - distance from the player's death-spot heat.

   Use the same terms as A* link costs for tactical paths. The **weights w_k per play and role become genes**. This is the key move against D5: evolution now searches a space where every point is a sensible tactic, which is the reason dynamic scripting [34] adapts in tens of encounters.
4. **Legibility.** Barks, generated after the decision as in F.E.A.R. [41]: "Moving — cover me", "He's holding tower left", "Flanking north". Also a subtle post-round replay arrow showing the play the squad ran. This addresses D6 at almost no AI cost, and it makes the adaptation *counterable*, which answers the predictability concern in [43].

### 4.3 Training: archive, opponent pool and objective

**Objective (D4).**
- Team-spirit reward r_i = (1−τ)ρ_i + τρ̄ [11], with τ annealed from 0.3 to 1 within each training session.
- Explicit credit for *assists* (damage within 3 s before a teammate's kill), *trades*, and *duels started in the player's weak situations*.
- Following FTW [8], keep the shaping weights as evolvable genes and **select on win probability** against the pool. Shaping helps the inner search, and selection keeps it honest.

**Noise reduction (D3).**
- **Common random numbers:** every candidate in a generation plays the same seeds and spawns.
- Evaluate **whole squads** as units, not random trait × NEAT pairings.
- Use more rounds on fewer candidates, and drop clear losers early (successive halving).
- Run **continuously in the background** whenever the game is in menus or debrief, with checkpoints, not only in the 8-second evolve budget. Report progress in **simulated rounds**, not "generations".

**Archive (QD).** MAP-Elites [26] (or CMA-ME [28] once the genome is mostly continuous) over a 2–3-dimensional squad *style* descriptor: median engagement range (close / mid / long), flank share (share of damage dealt from outside the player's facing cone), and tempo (time to first contact). Each cell keeps its best squad against the current pool. This gives about 30–50 qualitatively different squads, all decent and each with its own character.

**Opponent pool** (the AlphaStar and Five pattern [11][12]):
- 4–8 **posterior samples** of the player clone (sample L1 parameters from their posteriors), so squads beat the *range* of plausible versions of this player rather than one point estimate. This guards against overfitting to a clone artefact.
- **Past snapshots** of the player model (last 3–5 matches), since the player also adapts.
- 3 fixed **archetypes** (rusher, anchor, roamer) as regularisers.
- These samples come from the calibrated clone (C1, or C2 once gated in). The uncalibrated default ghost is kept only as one archetype.

Weight the pool 60% current-player samples, 25% past selves, 15% archetypes. Optionally add a Minimax-Exploiter-style [19] bonus for reaching states where the clone's estimated duel win rate is low.

**Deployment as a bandit (sample efficiency on the real player).** Treat each archive cell as an arm. Keep a Beta posterior on the *real* round-win probability against the player for each arm. Initialise it from the cell's sim win rate against the current-player samples, discounted by the measured sim-to-real calibration error. Choose next match's squad by **Thompson sampling**, constrained to a target difficulty band (Section 4.5). This is Bayesian Policy Reuse [37] in its simplest form. Real outcomes now update a quantity that the next decision actually uses, instead of nudging a GA that steps once per match. Expect useful identification within about 3–6 matches, because the arms are few and their priors are informative.

**PBT over training hyperparameters** [9]: mutation σ, τ schedule, and the shaping weights, exploited and explored every N simulated rounds. This is cheap to add on top of the archive.

### 4.4 Optional small-RL fine-tuning

Only after 4.1–4.3 are done. Replace or augment the NEAT micro (14→4) with a 2×64 MLP trained by PPO [50] in the Rust sim against the pool, **KL-regularised toward a human-like prior policy** (piKL [40]). The prior can be the current NEAT champion or a behaviour-cloned micro policy fitted to the *player's own* engagement data, mirrored as an enemy. At 10 Hz decisions (GT Sophy found no benefit above 10 Hz [10]), 1–5 million micro-steps is a plausible budget. That is roughly hours of background CPU time, and it needs a benchmark (Section 4.6). RL is attractive for micro (peek timing, jiggle peeks, strafe rhythm) because those reward signals are dense. It is not recommended for tactics, where the PEF/plays structure already gives a good search space.

### 4.5 Fairness constraints

Keep `limits.rs` and make the squad's information explicitly human-equivalent:

1. **Perception parity.** Enemies know only what they saw or heard, or what a teammate radioed with latency. The belief grid is built only from these sources and the *historical* model. Using the player's history is fair, since a human opponent also remembers.
2. **No pre-fire without evidence.** Firing at an unseen position requires belief mass above a threshold *and* a recent cue (sound, sighting under 3 s ago).
3. **Motor limits.** Keep the reaction floor of at least 180 ms (FTW's mean was 258 ms against 559 ms for humans [8], and OpenAI Five's was 217 ms [11]). Keep the turn-rate cap, the settle time, and spread growing with movement. Add a cap on decision rate (10 Hz micro, 4 Hz tactics), matching GT Sophy's 10 Hz [10].
4. **Difficulty band, not stat inflation.** Target a real round-win rate for the squad of 40–60% by choosing *which archive cell* to deploy. If needed, add L4D-style pacing [46], such as how hard the squad pushes after the player gets a multi-kill. Do not use the gen-scaled `damage_per_hit` as the main lever. Halo's playtests show toughness *is* read as intelligence [44], so a small amount is legitimate, but it must not stand in for decisions.
5. **Bounded change per match.** Shift style by at most one archive "step" per match, and announce it in the debrief. This makes the adaptation learnable and counterable for the player, as recommended in [43].

### 4.6 What to compute where (24 GB Apple M5)

The M5 has a 10-core GPU with a Neural Accelerator in each core, a CPU of up to 10 cores, and 153 GB/s of unified memory bandwidth [52].

| Workload | Recommendation | Why |
|---|---|---|
| 2D sim rounds (LOS, A*, utility) | **CPU, rayon, 4–8 threads, low priority** | Branchy, irregular work. Batched GPU sim (Isaac Gym/Madrona [3][51]) pays only for vectorised physics plus NN policies, and would mean rewriting nav and LOS. The GPU is also busy rendering during play. |
| MAP-Elites, bandit, Bayesian updates | CPU, single thread | Microseconds per update. |
| Player clone (≈50–200k params) training | CPU first. GPU via Burn (wgpu/Metal backend) [53] if needed | About 10⁵ samples × small MLP or GRU: seconds to minutes on CPU. |
| PPO micro (optional) | CPU rollouts plus CPU or Metal updates | The policy is tiny, and rollout cost dominates. |
| Debrief phrasing with an SLM (optional) | GPU/ANE, a ~2B model quantised (~1.5 GB [1]) | Only for wording. Facts must come from the statistics, following PUBG Ally's "tool results are the only ground truth" [2]. |

Memory is not a constraint: archive plus models are under 50 MB. **The binding constraint is wall-clock CPU while the player waits.** Hence the background trainer. Before choosing budgets, use `--sim-bench` to measure simulated rounds per second per core. If one round costs t ms on one core, 10 minutes of menu and match time on 4 cores gives 2.4×10⁶/t rounds, which is enough for thousands of evaluations even at t = 50 ms.

---

## 5. Evaluation methodology: "smarter", not just "stronger"

Most of the evaluation can run **offline against held-out player models**, so it doesn't need many human matches. The headline claims still need real-match confirmation. Use common random numbers and report bootstrap 95% CIs throughout.

1. **Sim-to-real fidelity (gate for everything else).** Use the per-generation dashboard from Section 4.1.1: the outcome gap Δ_g between the clone's win rate and the real win rate for each deployed squad, predictive coverage, and rank agreement. In addition, for each situation key, compare the predicted duel win rate in sim with the observed real rate. Report the Brier score and a calibration plot, and also the round-length and time-to-kill distributions (Kolmogorov–Smirnov). If fidelity is poor, gains in sim mean nothing.
2. **Strength.** Elo or Bradley–Terry ratings of squads across training checkpoints against a fixed held-out pool (archetypes plus models of other players).
3. **Specificity (the main "learned *you*" metric).** Build a cross-play matrix: squads specialised for players A, B, C… against models of A, B, C…. Specificity = mean diagonal win rate − mean off-diagonal win rate. A smarter squad is strong *on its own diagonal*, and a generically stronger squad raises whole rows.
4. **Weakness exploitation.** The share of duels initiated in the player's bottom-quartile situations (by L1 posterior), against the share under a non-adaptive baseline squad. This should rise, and the squad's duel win rate in those situations should exceed the baseline's.
5. **Adaptivity.** Inject a synthetic style shift (e.g. the player model switches from anchor to rusher at match k) and measure **matches to recover** 90% of the pre-shift advantage.
6. **Sample efficiency.** Real matches needed for the bandit to settle on a cell within 5 percentage points of the best cell, measured on synthetic players with known best responses.
7. **Diversity.** QD-score and archive coverage; entropy of deployed plays across matches; the share of matches in which a returning player meets a *visibly different* plan.
8. **Human-likeness.** An ANTT-style classifier [23] trained to tell human trajectories from bot trajectories: report its AUC (closer to 0.5 is better). Also check reaction-time and spread distributions against human ranges, and run a small blind BotPrize-style rating [47][48] with playtesters.
9. **Player experience.** Real round-win rate within the 40–60% band. Post-match single-item ratings ("they felt smarter than last time", "I understood why I lost"). Halo's playtests show such ratings are measurable and sensitive [44].
10. **Ablations.** Remove each of: calibration (clone vs default ghost), C2 vs C1, the PEF genes, plays, the bandit, and the pool. A component that doesn't move metrics 3–5 gets removed.

Success criterion to replace the one in `EVOLUTION.md`: **mean |Δ_g| < 0.15 (simulated and real win rates agree), specificity > 0 with CI excluding 0, weakness-exploitation share up by at least 10 percentage points, calibration Brier score under 0.2, and the real win rate moving toward the band within 6 matches.**

---

## 6. Better insights

Today's insights describe *where* the player is ("south yard 41%"). Useful insights describe *what loses rounds* and *what to change*, with honest uncertainty. The engagement log (L0) and the posteriors (L1) make this possible.

**6.1 Situation insights with credible intervals.** For each situation key with n ≥ 6 duels, compute the posterior P(win rate here < the player's overall duel win rate). Report only when that probability is at least 0.9. Example: "You lose **70%** of duels when peeking **tower's left side** (7 of 10; likely range 45–88%). On your other peeks you lose 42%." Rank by **expected rounds at stake** = frequency of the situation per round × (gap in win rate). That puts first the habit that costs the most, not the most extreme statistic.

**6.2 Counterfactual comparisons.** For pairs of situations that differ in one controllable choice (left vs right peek of the same cover, crouch vs stand, re-peek vs reposition after a kill, swing vs hold), estimate the difference with matching on range and odds: "When you **re-peek the same angle** within 5 s, you win 31% of duels. When you reposition first, you win 58%." These are observational associations, and the text says "when you…" rather than "because…".

**6.3 Simulated counterfactuals.** For the top insight, run it as an intervention in the calibrated sim: replay the player model with that one habit changed, against the current squad, for a few hundred rounds. "Simulated: if you reset after a kill instead of pushing, you'd win about **+18% of rounds** against this squad (±6%)." This is the only kind of insight that supports a causal claim, and only to the extent that the sim is calibrated. Show the calibration score alongside it.

**6.4 Trends against previous matches.** The posteriors from each match are stored, so changes can be reported with a significance gate: "Reaction time 340 → 290 ms over your last 4 matches." "You now hold the yard 40% less, and the squad's ambushes there stopped working." Run a simple change-point test (posterior of the difference crosses 0.9) to detect when a habit was dropped.

**6.5 "What they learned → what they did → did it work."** One card per match that ties the model to the squad's behaviour: "They expected you at **tower left** (you went there in 6 of 8 rounds). They set a **crossfire** from the catwalk and the gate. It won **3 of 4** duels. Counter: take the tower from the right side or use the yard route." This is the single most important item for making adaptation *perceived* (D6), and it is what F.E.A.R.'s after-the-fact dialogue did in-game [41].

**6.6 Accuracy guarantees.** Unit-test the insight generator on **synthetic players with planted weaknesses**, e.g. a C1 clone configured to lose 75% of left peeks. Measure precision and recall of the generated insights over many seeds. Require precision ≥ 0.9, since a false insight costs more trust than a missing one. Optionally phrase the final text with a small local LLM, passing it only the computed facts, and reject any output that contains a number not in the facts.

---

## 7. Prioritised roadmap

Modules refer to the current tree. Costs assume one engineer. Impact is relative to this report's diagnosis.

| P | Item | Modules | Cost | Expected impact |
|---|---|---|---|---|
| ✅ done | Radio squad blackboard (sightings every 0.35 s), reload-punish pushes, crossfire flank selection | `mind.rs`, `utility.rs` | — | First real coordination. Its value can't be measured until P0-3 and P1-1 land. |
| **P0-1** | **Engagement log (L0)** plus situation keys | `game/telemetry.rs`, `brain/player_model.rs: MatchTelemetry` | 1–2 d | Foundation for insights, calibration and the bandit. Nothing below works without it. |
| **P0-2** | **Insights v2**: Beta posteriors with shrinkage, credibility gate, rounds-at-stake ranking, trends, "what they learned" card | `player_model.rs: insights`, `game/ui/debrief.rs`, history in `evolve.rs` | 2–3 d | Fixes the "shallow insights" complaint directly. High user-visible value. |
| **P0-3** | **Sim-to-real dashboard plus evaluation hygiene**: store deployed genomes per generation in `history`; per-shot log; log Δ_g each generation; common random numbers, whole-squad evaluation, more rounds per candidate | `evolve.rs: evaluate, MatchSummary`, `sim.rs`, `game/telemetry.rs`, `examples/sim_bench` | 2 d | Puts the measured ~0.9 gap on a dashboard every generation. Turns selection from noise into signal (D3). |
| **P0-4** | **Callouts/barks** generated from goals and plays | `game/enemy/*`, `game/audio.rs` | 1–2 d | Large perceived-intelligence gain for little cost (D6) [41][44]. |
| **P1-1** | **Player clone C1 (central)**: structured behaviour cloning of route, engagement, peek, pre-aim, reaction and hit heads, plus outcome calibration against stored squads, plus the gate | `sim.rs: Ghost` → new `clone.rs`, `player_model.rs`, `evolve.rs` | 6–8 d | The single biggest lever (D1). Until Δ_g closes, sim training cannot improve real results. |
| **P1-2** | **Squad layer** (builds on the radio blackboard, reload-punish pushes and crossfire flank selection, which are done): belief grid, plays with slots (overwatch, trade, collapse), PEF scoring with genes, tactical A* costs | new `squad.rs`, `mind.rs`, `nav.rs`, `genome.rs`, `utility.rs` | 6–10 d | Makes the behaviours that read as smart expressible (D5). Evolution gets a meaningful search space. |
| **P1-3** | **Team-spirit objective** plus assist and trade credit; select on win probability | `sim.rs: EnemyStats`, `evolve.rs` | 1 d | Aligns evolution with squad play (D4). |
| **P1-4** | **Background trainer** with checkpoints; progress in simulated rounds | `game/evolving.rs`, `evolve.rs` | 2–3 d | 10–100× more evaluation per match at no wait time. |
| **P2-1** | **MAP-Elites archive** over squad style plus **Thompson-sampling deployment** constrained to a difficulty band | `genome.rs: pick_squad`, `evolve.rs` | 4–5 d | Sample-efficient adaptation on real matches. Visible variety. Principled difficulty. |
| **P2-2** | **Opponent pool** (posterior samples, past selves, archetypes) | `sim.rs`, `evolve.rs` | 2 d | Robustness: stops overfitting to one ghost. |
| **P2-3** | **Particle-filter prediction** replacing `predict` | `player_model.rs`, `squad.rs` | 3 d | Better ambushes and collapses, while staying fair. |
| **P2-4** | **Neural clone C2**: ray-cast observations, population prior plus per-player embedding; replaces C1 only if it wins on the dashboard | `clone.rs` (hand-rolled MLP or Burn) | 2–3 wk | Higher fidelity in aim and movement details, *if* the validation metrics improve. |
| **P3-2** | **PPO micro with KL to a human-like prior** | `mind.rs`, new `rl.rs` | 2–3 wk | Sharper peeks and strafes. Do it only after everything above. |
| **P3-3** | SLM phrasing of debrief | `game/ui` | 3–5 d | Nicer wording only. Optional. |

**Recommended first milestone (about 2–3 weeks):** P0-1, P0-3 and **P1-1 (the calibrated clone)** first, then P0-2, P0-4 and P1-2 to P1-3. Exit criterion: mean |Δ_g| < 0.15 on the dashboard. Only then are sim-trained squads expected to win real rounds, and only then is it meaningful to measure the squad features built on top.

---

## 8. Risks

1. **Overfitting to the model rather than the player.** Squads may exploit artefacts of the clone. *Mitigations:* posterior-sample pool, archetype regularisers, the calibration gate, and real-outcome bandit priors discounted by calibration error.
2. **Non-stationarity and cycling.** The player adapts to the squad, and the squad to the player. *Mitigations:* past-self pool [11], bounded change per match, and recency weighting via γ.
3. **"It's cheating" perception.** Prediction-driven ambushes can feel like wallhacks. *Mitigations:* the pre-fire evidence rule, radio latency, and debrief cards that explain *why* they knew ("you went there in 6 of 8 rounds").
4. **Unpredictability and frustration.** The concern raised by Guerrilla [43]. *Mitigations:* a discrete archive of recognisable plays, at most one style step per match, and announced adaptations.
5. **False insights.** Small samples create spurious patterns. *Mitigations:* the credibility gate, shrinkage, the planted-weakness test suite, and conservative wording.
6. **Sim-to-real gap persists** even with the calibrated clone, because human aim and attention are hard to model. *Mitigations:* report calibration openly, lean on the bandit (which learns from real outcomes), and keep the sim's role to *stocking* the archive rather than *choosing*.
7. **Behaviour-cloning compounding errors** [49] make the clone drift off the player's distribution. *Mitigation:* the gate; the clone is optional.
8. **Compute and thermals** on a laptop during play. *Mitigations:* low-priority threads, pausing the trainer while a match is running, and caps set from `--sim-bench`.
9. **Complexity creep.** Six layers is a lot. *Mitigation:* the ablation discipline in Section 5.10. Each layer must move a metric to stay.
10. **Difficulty spiral.** A squad that keeps winning drives players away. *Mitigation:* the difficulty band is a hard constraint on arm selection, not a soft goal.

---

## 9. Verification notes

- **NVIDIA:** we verified ACE [1], PUBG Ally's architecture [2], Isaac Gym [3], ASE [4], Eureka [5], Voyager [6] and NitroGen [7]. We found **no** NVIDIA publication on opponents that learn individual player tactics. The MIR5 "ever-learning bosses" claim is marketing copy with no technical source **[unverified]**.
- **Drivatar:** the details of the modern system come from a well-researched secondary article [13]. Turn 10 has not published the internals, and the "reinforcement learning" claim is **[unverified]**.
- **Halo 2 behaviour trees (Isla, GDC 2005):** known only through secondary course slides. The original article was not retrieved **[partially verified]**.
- **Hladky and Bulitko's model types** (hidden semi-Markov models, particle filters) come from catalogue keywords. The accuracy claim is from the abstract [35].
- **BotPrize 2012** figures come from the University of Texas press release [48]. The details of UT^2's method are summarised from coverage **[partially verified]**.
- All compute estimates in Section 4.6 are projections to be checked with `--sim-bench`.

---

## 10. References

1. NVIDIA. "NVIDIA Redefines Game AI With ACE Autonomous Game Characters." GeForce News, Jan 2025. https://www.nvidia.com/en-eu/geforce/news/nvidia-ace-autonomous-ai-companions-pubg-naraka-bladepoint/
2. NVIDIA Technical Blog. "How KRAFTON Built PUBG Ally, a Co-Playable Character Powered by NVIDIA ACE." June 2026. https://developer.nvidia.com/blog/how-krafton-built-pubg-ally-a-co-playable-character-powered-by-nvidia-ace/
3. Makoviychuk, V. et al. "Isaac Gym: High Performance GPU-Based Physics Simulation for Robot Learning." NeurIPS Datasets & Benchmarks, 2021. https://arxiv.org/abs/2108.10470
4. Peng, X. B., Guo, Y., Halper, L., Levine, S., Fidler, S. "ASE: Large-Scale Reusable Adversarial Skill Embeddings for Physically Simulated Characters." SIGGRAPH 2022. https://arxiv.org/abs/2205.01906
5. Ma, Y. J. et al. "Eureka: Human-Level Reward Design via Coding Large Language Models." ICLR 2024. https://arxiv.org/abs/2310.12931
6. Wang, G. et al. "Voyager: An Open-Ended Embodied Agent with Large Language Models." 2023. https://arxiv.org/abs/2305.16291
7. Magne, L. et al. "NitroGen: An Open Foundation Model for Generalist Gaming Agents." 2026. https://arxiv.org/abs/2601.02427
8. Jaderberg, M. et al. "Human-level performance in 3D multiplayer games with population-based reinforcement learning." Science 364(6443), 2019. https://doi.org/10.1126/science.aau6249 (preprint: https://arxiv.org/abs/1807.01281)
9. Jaderberg, M. et al. "Population Based Training of Neural Networks." 2017. https://arxiv.org/abs/1711.09846
10. Wurman, P. R. et al. "Outracing champion Gran Turismo drivers with deep reinforcement learning." Nature 602, 223–228, 2022. https://doi.org/10.1038/s41586-021-04357-7
11. OpenAI (Berner, C. et al.). "Dota 2 with Large Scale Deep Reinforcement Learning." 2019. https://arxiv.org/abs/1912.06680
12. Vinyals, O. et al. "Grandmaster level in StarCraft II using multi-agent reinforcement learning." Nature 575, 2019. https://doi.org/10.1038/s41586-019-1724-z ; blog: https://deepmind.google/discover/blog/alphastar-grandmaster-level-in-starcraft-ii-using-multi-agent-reinforcement-learning/
13. Thompson, T. "How Forza's Drivatar Actually Works." Game Developer, 2021. https://www.gamedeveloper.com/design/how-forza-s-drivatar-actually-works
14. Xbox Wire. "Forza Horizon 2: What's a Drivatar, and Why Should I Care?" 2014. https://news.xbox.com/en-us/2014/09/30/games-forza-horizon-2-drivatars/
15. EA SEED. "Self-Learning Agents Play Battlefield 1." https://www.ea.com/seed/news/self-learning-agents-play-bf1
16. Bergdahl, J., Gordillo, C., Tollmar, K., Gisslén, L. "Augmenting Automated Game Testing with Deep Reinforcement Learning." IEEE CoG 2020. https://arxiv.org/abs/2103.15819
17. EA SEED. "Technical Challenges of Deploying Reinforcement Learning Agents for Game Testing in AAA Games." IEEE CoG 2023. https://arxiv.org/abs/2307.11105
18. Alonso, E., Peter, M., Goumard, D., Romoff, J. "Deep Reinforcement Learning for Navigation in AAA Video Games." IJCAI 2021. https://arxiv.org/abs/2011.04764
19. Bairamian, D. et al. "Minimax Exploiter: A Data Efficient Approach for Competitive Self-Play." AAMAS 2024. https://arxiv.org/abs/2311.17190
20. Kanervisto, A. et al. "World and Human Action Models towards gameplay ideation." Nature 638, 656–663, 2025. https://doi.org/10.1038/s41586-025-08600-3 ; blog: https://www.microsoft.com/en-us/research/blog/introducing-muse-our-first-generative-ai-model-designed-for-gameplay-ideation/
21. Pearce, T., Zhu, J. "Counter-Strike Deathmatch with Large-Scale Behavioural Cloning." IEEE CoG 2022 / NeurIPS Offline RL Workshop 2021. https://arxiv.org/abs/2104.04258
22. Justesen, N. et al. (modl.ai, Riot Games). "Human-like Bots for Tactical Shooters Using Compute-Efficient Sensors." 2024. https://arxiv.org/abs/2501.00078
23. Devlin, S. et al. "Navigation Turing Test (NTT): Learning to Evaluate Human-Like Navigation." ICML 2021. https://arxiv.org/abs/2105.09637
24. Stanley, K. O., Miikkulainen, R. "Evolving Neural Networks through Augmenting Topologies." Evolutionary Computation 10(2), 2002. https://www.cse.unr.edu/~sushil/class/gas/papers/NEAT.pdf
25. Stanley, K. O., Bryant, B. D., Miikkulainen, R. "Real-Time Neuroevolution in the NERO Video Game." IEEE Trans. Evolutionary Computation 9(6), 2005. https://www.cse.unr.edu/~bdbryant/papers/stanley-2005-tec.pdf
26. Mouret, J.-B., Clune, J. "Illuminating search spaces by mapping elites." 2015. https://arxiv.org/abs/1504.04909
27. Lehman, J., Stanley, K. O. "Abandoning Objectives: Evolution through the Search for Novelty Alone." Evolutionary Computation 19(2), 2011. https://www.cs.swarthmore.edu/~meeden/cs81/f17/papers/lehman_ecj11.pdf
28. Fontaine, M. C., Togelius, J., Nikolaidis, S., Hoover, A. K. "Covariance Matrix Adaptation for the Rapid Illumination of Behavior Space." GECCO 2020. https://arxiv.org/abs/1912.02400
29. Yannakakis, G. N., Togelius, J. *Artificial Intelligence and Games.* Springer, 2018. https://gameaibook.org
30. Yannakakis, G. N., Spronck, P., Loiacono, D., André, E. "Player Modeling." Dagstuhl Follow-Ups 6, 2013. https://doi.org/10.4230/DFU.Vol6.12191.45
31. Hunicke, R. "The case for dynamic difficulty adjustment in games." ACE 2005. https://doi.org/10.1145/1178477.1178573
32. Hunicke, R., Chapman, V. "AI for Dynamic Difficulty Adjustment in Games." AAAI Workshop on Challenges in Game AI, 2004. https://users.cs.northwestern.edu/~hunicke/pubs/Hamlet.pdf
33. Chen, J. "Flow in Games (and Everything Else)." Communications of the ACM 50(4), 2007. https://doi.org/10.1145/1232743.1232769
34. Spronck, P., Ponsen, M., Sprinkhuizen-Kuyper, I., Postma, E. "Adaptive game AI with dynamic scripting." Machine Learning 63(3), 2006. https://doi.org/10.1007/s10994-006-6205-6
35. Hladky, S. "Predicting Opponent Locations in First-Person Shooter Video Games." MSc thesis, University of Alberta, 2009 (see also Hladky & Bulitko, IEEE CIG 2008). https://era.library.ualberta.ca/items/85d65107-ed06-4745-9e67-ff3392bad546
36. "A Survey of Opponent Modeling in Adversarial Domains." Journal of Artificial Intelligence Research 73, 2022. https://doi.org/10.1613/jair.1.12889
37. Rosman, B., Hawasly, M., Ramamoorthy, S. "Bayesian Policy Reuse." Machine Learning 104(1), 2016. https://arxiv.org/abs/1505.00284
38. McIlroy-Young, R., Wang, R., Sen, S., Kleinberg, J., Anderson, A. "Learning Models of Individual Behavior in Chess." KDD 2022. https://arxiv.org/abs/2008.10086
39. Tang, Z. et al. "Learning to Imitate with Less: Efficient Individual Behavior Modeling in Chess." 2025. https://arxiv.org/abs/2507.21488
40. Jacob, A. P. et al. "Modeling Strong and Human-Like Gameplay with KL-Regularized Search." ICML 2022. https://arxiv.org/abs/2112.07544
41. Orkin, J. "Three States and a Plan: The A.I. of F.E.A.R." GDC 2006. https://pages.cs.wisc.edu/~dyer/cs540/handouts/gdc2006_orkin_jeff_fear.pdf
42. Beij, A., Straatman, R. "Killzone's AI: Dynamic Procedural Tactics." GDC Europe 2005. https://www.guerrilla-games.com/media/News/Files/gdce05_killzone_ai.pdf
43. Verweij, T. "A hierarchically-layered multiplayer bot system for a first-person shooter." MSc thesis, VU Amsterdam / Guerrilla, 2007. https://www.guerrilla-games.com/media/News/Files/VUA07_Verweij_Hierarchically-Layered-MP-Bot_System.pdf
44. Butcher, C., Griesemer, J. "The Illusion of Intelligence: The Integration of AI and Level Design in Halo." GDC 2002. https://www.jmeiners.com/shamans/papers/ai/the_illusion_of_intelligence.pdf
45. Isla, D. "Handling Complexity in the Halo 2 AI." GDC 2005, as summarised in course slides (secondary). https://web.eecs.umich.edu/~sugih/courses/eecs494/fall06/lectures/lecture18-halo2.pdf
46. Booth, M. "The AI Systems of Left 4 Dead." AIIDE 2009 keynote (Valve). https://cdn.fastly.steamstatic.com/apps/valve/2009/ai_systems_of_l4d_mike_booth.pdf
47. Hingston, P. "A Turing Test for Computer Game Bots." IEEE Trans. Computational Intelligence and AI in Games 1(3), 2009. https://doi.org/10.1109/TCIAIG.2009.2032534
48. University of Texas at Austin. "Artificially Intelligent Game Bots Pass the Turing Test on Turing's Centenary." 2012. https://news.utexas.edu/2012/09/26/artificially-intelligent-game-bots-pass-the-turing-test-on-turings-centenary/
49. Ross, S., Gordon, G., Bagnell, J. A. "A Reduction of Imitation Learning and Structured Prediction to No-Regret Online Learning" (DAgger). AISTATS 2011. https://arxiv.org/abs/1011.0686
50. Schulman, J. et al. "Proximal Policy Optimization Algorithms." 2017. https://arxiv.org/abs/1707.06347
51. Shacklett, B. et al. "An Extensible, Data-Oriented Architecture for High-Performance, Many-World Simulation" (Madrona). SIGGRAPH 2023. https://github.com/shacklettbp/madrona
52. Apple. "Apple unleashes M5, the next big leap in AI performance for Apple silicon." Oct 2025. https://www.apple.com/newsroom/2025/10/apple-unleashes-m5-the-next-big-leap-in-ai-performance-for-apple-silicon/
53. Tracel AI. "Burn: a deep learning framework in Rust" (wgpu/Metal backends). https://github.com/tracel-ai/burn

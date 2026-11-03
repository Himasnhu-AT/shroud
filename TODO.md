# Shroud: CS2-style tactical loop + bodycam roadmap

Started 2026-11-03. Two sources:
- **Ask A (chat):** CS2-like maps, operator skins, guns, buy menu, simple menu
  (choose map → play), 5-round match vs an AI that improves only in technique.
- **Ask B (spec):** `~/Downloads/bodycam_fps_ai_agent_implementation_spec_bevy_0_20.md`
  (bodycam optics, free-aim, procedural springs, adaptive AI doctrine).

Rules that apply throughout: original maps, names and art only (genre-alike, no copies of real
CS2 maps or skins); CC0 assets with a primitive fallback; every cargo run through `memgate`.

## Roadmap

| Phase | What | Status |
|---|---|---|
| 0 | Technique-only evolution + shared price table | done |
| 1 | Match loop: 5 rounds, freeze time, economy, buy menu, simple menu | in progress |
| 2 | CS-style defusal maps (A/B sites, mid, spawns, buy zones) | in progress |
| 3 | Guns + operator skins (CS feel, original designs) | in progress |
| 4 | Bodycam optics (spec §4): post-process shader, muzzle flash light, auto-exposure | planned |
| 5 | Adaptive doctrine (spec §5): player archetypes → squad counters | planned |
| 6 | Bodycam handling (spec §2–3): free-aim deadzone, springs, wall compression, dutch roll | planned |
| 7 | Realistic assets (spec §6): HDRI, 4K PBR, rigged arms, realistic guns | blocked on downloads |

## Checklist

### Phase 0: AI improves only in technique
- [x] Fairness limits fixed (no aim/reaction/damage growth with generation) (`brain/src/limits.rs`)
- [x] Reaction and aim genes inert; every enemy uses the fixed level from the difficulty (`genome.rs`)
- [x] Debrief shows only technique genes (`evolve.rs`)
- [x] Test proves technique-only gain (Recruit eval: 54% → 77% squad win rate)
- [x] `WeaponKind::price()` / `kill_reward()` contract (`brain/src/weapons.rs`)
- [ ] Docs: update `docs/EVOLUTION.md` / `docs/AI.md` §5 for fixed limits

### Phase 1: match loop
- [ ] 5-round match: first to 3, max 5 rounds (`match_flow.rs`)
- [ ] Freeze time at round start (~10 s): can buy and look, can't move or shoot
- [ ] Economy: start $800, win $3250, loss $1400 + $500 per loss streak (cap $3400), kill rewards per gun
- [ ] Buy menu (B, during freeze time / in buy zone): pistols, SMGs, rifles, heavy, armor, helmet, grenades
- [ ] Keep weapons you survived with; lose them on death
- [ ] Enemy economy: squad buys from the same prices (eco / force / full buy)
- [ ] HUD: money, freeze countdown, buy hint
- [ ] Main menu: Play → map select (cards) + difficulty → Start; primary-weapon picker removed
- [ ] Tests: economy math, buy rules, round count

### Phase 2: maps
- [ ] 2 new original defusal-style maps: attacker spawn, defender spawn, A and B sites, mid, long/short routes, connectors, chokes
- [ ] Site markers (painted "A"/"B" on walls/floor), spawn buy-zone markers
- [ ] Zones named for callouts (A long, B tunnels, mid, CT, T spawn…)
- [ ] Nav + sim round passes on each new map (`brain/tests/maps.rs`)
- [ ] Map cards in the menu show the new maps first
- [ ] Screenshot tour script

### Phase 3: guns + skins
- [ ] More guns: heavy pistol, second rifle, bolt sniper (AWP-like role), with prices
- [ ] Gun models read as real firearms (gunmetal/polymer materials, or CC0 realistic models)
- [ ] Enemy operators: helmet, plate carrier, team colours, no pink mannequins
- [ ] Player viewmodel sleeves/gloves in team colours
- [ ] Enemies use new guns (loadout genes)

### Phase 4: bodycam optics (spec §4)
- [ ] Optional "Bodycam" video mode (off by default; conflicts with the CS HUD, see notes)
- [ ] WGSL post-process: barrel distortion, chromatic aberration, sensor grain, vignette, sharpen
- [ ] HDR + AgX + bloom + auto-exposure on the main camera
- [ ] 45 ms muzzle-flash point light
- [ ] [REC] timestamp stamp overlay in bodycam mode

### Phase 5: adaptive doctrine (spec §5)
- [ ] `PlayerStyleProfile`: speed EMA, angle-hold time, ADS ratio, headshot rate, early reloads
  (extends the existing `brain::player_model`)
- [ ] Archetype → counter: camper → jiggle-peek + flash + wallbang; rusher → off-angles + crouch;
  reloader → rush on mag-out sound; headshot aimer → lean-peek + micro-strafe
- [ ] Log state transitions (e.g. `AiTactic::JigglePeekAndFlash` after > 4 s holding a door)
- [ ] These are technique counters, so they fit Phase 0's rule

### Phase 6: bodycam handling (spec §2–3)
- [ ] Free-aim deadzone (18°×12°) that collapses on ADS
- [ ] 6-DOF spring rig on the viewmodel; recoil impulses
- [ ] Wall compression raycast (0.65 m barrel)
- [ ] Shots from the muzzle socket, not screen centre (bodycam mode only; CS mode keeps centre aim)
- [ ] Dutch roll from yaw speed and strafing

### Phase 7: realistic assets (spec §6)
- [ ] Poly Haven HDRIs, ambientCG 4K sets (CC0, scriptable downloads)
- [ ] Rigged arms / operator bodies: Mixamo and Sketchfab need accounts and manual download.
  The user has to fetch these; licences must be checked per model (CC0/CC-BY only)

## Notes and conflicts
- Spec §4.3 says "zero health/ammo on screen"; the CS2 loop needs money, timer and ammo. Plan: the
  bodycam look is a toggle and CS HUD is the default.
- Spec §2 free-aim (shots from the barrel) changes how CS-style precision works. Bodycam mode only.
- Spec asks for `bevy_mod_inverse_kinematics`/rapier; we stay on avian3d 0.8 and a hand-written
  2-bone IK if needed (pinned deps).

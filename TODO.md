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
| 1 | Match loop: 5 rounds, freeze time, economy, buy menu, simple menu | done (player grenades not buyable yet) |
| 2 | CS-style defusal maps (A/B sites, mid, spawns, buy zones) | done (Sirocco, Gantry; Depot signs) |
| 3 | Guns + operator skins (CS feel, original designs) | done (hands are placeholder blocks) |
| 4 | Bodycam optics (spec §4): post-process shader, muzzle flash light, auto-exposure | planned |
| 5 | Adaptive doctrine (spec §5): player archetypes → squad counters | done (lights not breakable: skipped) |
| 6 | Bodycam handling (spec §2–3): free-aim deadzone, springs, wall compression, dutch roll | planned |
| 7 | Realistic assets (spec §6): HDRI, 4K PBR, rigged arms | partly: CC0 realistic guns in; arms/bodies need user downloads |

## Checklist

### Phase 0: AI improves only in technique
- [x] Fairness limits fixed (no aim/reaction/damage growth with generation) (`brain/src/limits.rs`)
- [x] Reaction and aim genes inert; every enemy uses the fixed level from the difficulty (`genome.rs`)
- [x] Debrief shows only technique genes (`evolve.rs`)
- [x] Test proves technique-only gain (Recruit eval: 54% → 77% squad win rate)
- [x] `WeaponKind::price()` / `kill_reward()` contract (`brain/src/weapons.rs`)
- [x] Docs: `docs/AI.md` §1 and §5 updated for fixed limits

### Phase 1: match loop
- [x] 5-round match: first to 3, max 5 rounds (`match_flow.rs`)
- [x] Freeze time at round start (8 s): can buy and look, can't move or shoot
- [x] Economy: start $800, win $3250, loss $1400 + $500 per loss streak (cap $3400), kill rewards per gun
- [x] Buy menu (B, during freeze time / in buy zone): pistols, SMGs & shotguns, rifles & snipers, armor, helmet
- [ ] Player grenades in the buy menu (needs player throwing; enemy grenades exist)
- [x] Keep weapons you survived with; lose them on death
- [x] Enemy economy: squad buys from the same prices (eco / force / full buy)
- [x] HUD: money, freeze countdown, buy hint
- [x] Main menu: Play → map select (cards) + difficulty → Start; primary-weapon picker removed
- [x] Tests: economy math, buy rules, round count

### Phase 2: maps
- [x] 2 new original defusal-style maps: Sirocco (hill town) and Gantry (harbour rail yard), in `brain/src/maps/`
- [x] Site markers (painted "A"/"B" on walls/floor, direction signs), spawn borders (attackers orange, defenders blue); Depot got signs too
- [x] Zones named for callouts (A long, A short, mid, mid doors, B tunnels, B quay, connectors, attacker/defender spawn…)
- [x] Nav + sim round passes on each new map (`brain/tests/maps.rs` runs every `MapId::ALL`)
- [x] Map cards in the menu show the new maps first (the map row wraps, three per row)
- [x] Screenshot tour scripts: `scripts/sirocco-tour.json`, `scripts/gantry-tour.json`; frame time: `scripts/maps-perf.json`
- [ ] Optional: sandstone / cobblestone texture sets (the maps reuse stucco, paving and tiles with per-map tints)

### Phase 3: guns + skins
- [x] More guns: Warden heavy pistol (700), Shrike battle rifle (2700, the attacker rifle), Osprey
  bolt sniper (4750, one-shot body kill). Longbow retuned as the cheap scout (1700)
- [x] Gun models read as real firearms (Pichuliru CC0 set + per-gun steel/polymer/wood finishes)
- [x] Enemy operators: helmet, plate carrier, pack/radio, gloves, boots, red armband; 3 variants
- [x] Player viewmodel sleeves/gloves in team colours (primitive hands; no finger rig)
- [x] Enemies use new guns (six loadout genes; old four-gene saves load and seed the new two)
- [ ] Real rigged first-person arms (needs an asset; see Phase 7)

### Phase 4: bodycam optics (spec §4)
- [x] Optional "Bodycam" camera style (Settings → Video, off by default; `video.camera_style`)
- [x] WGSL post-process: barrel distortion, chromatic aberration, sensor grain, vignette, sharpen
  (one `FullscreenMaterial` pass after tonemapping/FXAA; also on offscreen captures)
- [x] AgX + auto-exposure (fast up, slow down) + bloom tuning; bloom only on tiers that have it
- [x] 45 ms muzzle-flash point light (both styles; pooled, no shadows)
- [x] [REC] timestamp stamp overlay; no crosshair / health / ammo (R on a full mag = ammo check)
- [ ] Muzzle light casting shadows (spec): needs a governor-aware budget; a 45 ms shadowed point
  light is six extra views for a few frames
- [ ] Chest offset for the camera (spec §2.1 `spine_02`): kept at eye height so cover and
  peeking play the same in both styles

### Phase 5: adaptive doctrine (spec §5)
- [x] `PlayerStyleProfile` (`brain/src/doctrine.rs`): speed EMA, angle-hold time, ADS ratio, headshot
  rate, early reloads, rapid room entries; 20 s / 60 s windows; per-match summary in the player model
- [x] Archetypes with hysteresis (several at once) → `AiTactic`: camper → jiggle-peek + flash-first +
  wallbang through thin cover; rusher → off-angle traps + crouch on contact; reloader → rush on the
  mag-out sound; headshot aimer → lean-peeks from posts + micro-strafes
- [x] Four evolvable doctrine genes (how hard each counter is used); old saves seed them
- [x] `ai.tactic {from, to, reason}` log / trace / callout; F3 doctrine panel; `ai.doctrine`; debrief line
- [x] Tests: `brain/tests/doctrine.rs` (spec check 6: holding a doorway > 4 s → `JigglePeekAndFlash`)
- [x] In-game check: `scripts/doctrine-camper.json` (switch at 4.0 s, a member jiggles, two flashes)
- [ ] Shoot out lights (lamps aren't breakable yet; worth it with Phase 4's auto-exposure)
- [x] These are technique counters, so they fit Phase 0's rule

### Phase 6: bodycam handling (spec §2–3)
- [x] Free-aim deadzone (18°×12°) that collapses on ADS (stiffness 14 hip / 35 ADS)
- [x] 6-DOF spring rig on the viewmodel (k 210/d 16, k 180/d 14); spec recoil impulses ×5
  (at 1× the gun moves under a centimetre)
- [x] Wall compression raycast (0.65 m of barrel past the body's 0.35 m clearance) → high ready
- [x] Shots from the muzzle along the barrel (bodycam only; Classic keeps centre aim)
- [x] Dutch roll from yaw speed and strafing (±7°), vest shock per shot
- [ ] Two-bone arm IK (spec §3.2): the hands are primitive blocks parented to the gun; needs a
  rigged arms asset first (Phase 7)

### Phase 7: realistic assets (spec §6)
- [ ] Poly Haven HDRIs, ambientCG 4K sets (CC0, scriptable downloads)
- [ ] Rigged arms / operator bodies: Mixamo and Sketchfab need accounts and manual download.
  The user has to fetch these; licences must be checked per model (CC0/CC-BY only)

## Known issues
- Fixed: `contact_encircles` was never nondeterministic. Each commit gives the same number on every run
  (514fd60: 182, ab8f41c: 184, 065b63b: 178 deg); a 3-seed mean on a cliff moved with unrelated changes
  (gun loadouts). Cutoffs stalled on door jambs (sim collision now circle-vs-box), the NEAT back-off
  slowed their run, they walked every door and were picked to throw mid-route. Depot x6 now 132 deg on
  six seeds; `encircle_is_deterministic` guards it.
- Fixed: Gantry's attacker spawn is back at (0, 32), behind the parked trailer (encircle 135/93 deg).

## Notes and conflicts
- Spec §4.3 says "zero health/ammo on screen"; the CS2 loop needs money, timer and ammo. Plan: the
  bodycam look is a toggle and CS HUD is the default.
- Spec §2 free-aim (shots from the barrel) changes how CS-style precision works. Bodycam mode only.
- Spec §7 checks for the bodycam: 1 (0.20 APIs, required components) yes; 2 (decoupled aim, ADS
  aligns the sight) yes; 3 (true barrel) yes; 4 (wall compression) yes; 5 optics yes, flash light
  yes but unshadowed and too short to move the auto exposure much.
- The bodycam stamp uses a fictional device id (`SHROUD BODY 4`), not the spec's real brand.
- Spec asks for `bevy_mod_inverse_kinematics`/rapier; we stay on avian3d 0.8 and a hand-written
  2-bone IK if needed (pinned deps).

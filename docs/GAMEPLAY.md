# Gameplay

## Loop
`Menu → Playing (match) → Evolving → Debrief → Playing …`

**Match:** first to 3 rounds (at most 5), you vs the squad. Main menu "Play" → setup (map cards, then
difficulty) → "Start match". A round ends when you die, the whole squad is dead, or the clock runs out.
No respawns within a round. Full HP and refilled ammo each round. The squad only evolves *between
matches*, and only in technique.

## Round economy (`economy.rs`, rules in `brain::economy`)
- **Freeze time** (8 s) at round start: look and buy, no moving or firing; the squad holds too. The
  HUD timer shows the countdown. Headless/test-driven runs have none (`round.freeze` sets it).
- **Money:** start $800; round win +$3,250; loss +$1,400, +$500 per further loss in a row (cap
  $3,400); kill reward per gun (`WeaponKind::kill_reward`); max $16,000. Resets each match.
- **Buy menu (B):** during freeze time, and for 20 s after it within 8 m of your spawn. Columns:
  Pistols, SMGs & Shotguns, Rifles & Snipers (sorted from each gun's spec, so new guns appear by
  themselves), Gear (Kevlar $650, Kevlar + helmet $1,000, helmet upgrade $350). Keys 1–9, 0 or click;
  owned items are marked, unaffordable ones greyed. Esc/B closes.
- **Keep what you survive with:** survive a round → keep guns and armor; die → pistol only, no armor.
  Round 1 is pistol + $800.
- **Armor:** the vest takes 50% of each hit (60% with a helmet) until its 100 points run out; headshots
  skip the vest unless you wear a helmet.
- **Squad economy:** each enemy has its own wallet under the same rules. Poor squads eco (keep
  survivors' guns, buy nothing), must-win or pistol rounds force-buy, rich squads full-buy. Which gun a
  member buys among those it can afford comes from its evolved loadout genes.
- Control port: `econ.state`, `econ.set_money`, `buy {item}`, `buy.menu {open}`, `round.freeze
  {seconds}`. Visual check: `scripts/buy-round.json`.

## Maps (64 × 64 m, flat; `brain::arena::Arena::build(MapId)`)
Picked in the menu (`Settings.map`); the map switches when a new match starts. All maps are
axis-aligned boxes so the 2D AI and trainer see the same layout as the 3D game. Low cover is 1.1 m:
crouch behind it to hide, stand to shoot over it. The player spawns south; the squad (6+ spawns) north.
Named zones are used in callouts and debrief text. `cargo run -p brain --example map_ascii -- depot`
prints a map's nav grid.

- **Foundry**: open industrial yard. Center tower, lane walls at x = ±14, yard pillars, lane blockers.
- **Depot**: three lanes between sun-baked buildings. *Long A* (west, with the long doors choke),
  *mid* (contested double doors into north mid, connector alleys to long A and the lower tunnel),
  roofed serpentine *B tunnels* (east). Two crate-and-container sites, *A site* and *B site*, at the north end.
- **Office**: one roofed floor, close quarters. Two long corridors with flank rooms (records, copy room,
  server room, kitchen …), open-plan cubicles around a conference room, break room north, reception and
  lobby south. Every room has 2+ doorways (2.2 m) so any position can be flanked.

## Player
| | |
|---|---|
| Move | WASD, Shift sprint, Space jump, Ctrl/C crouch (eye height 1.0 m) |
| Look | mouse (cursor locked while playing; Esc releases) |
| Shoot / reload | LMB / R |
| HP | 100 |

## Weapons (prototype)
| | Rifle | Pistol (stretch) |
|---|---|---|
| Damage | 25 (×2 headshot) | 34 |
| Fire rate | 10/s | 4/s |
| Mag | 30 | 12 |
| Model | hitscan + recoil kick/recovery | hitscan |

Enemies use the same rifle at 8–12 damage per hit (see fairness limits in [AI.md](AI.md)).

## HUD
Crosshair, hit marker, HP, armor, money (with +/- changes), ammo, enemies left, round score and
round N/5, freeze countdown, a "B Buy menu" hint while buying is possible, and **Generation N** in
the corner.

## Camera style: Bodycam (Settings → Video; off by default)
Classic is the CS-style default above. Bodycam (`bodycam/`) turns the round into chest-cam
footage:
- **Lens**: ~130° horizontal (100° vertical) plus one post pass after tonemapping: barrel
  distortion (corners kept), radial chromatic aberration, unsharp mask, shadow-weighted sensor
  grain at 30 Hz, heavy vignette (`bodycam_post.wesl`). Auto exposure adapts dark → bright fast
  (3.5 EV/s) and bright → dark slowly (1.2 EV/s); AgX; bloom 0.28 where the quality tier has
  bloom (High, Ultra).
- **Free aim**: the mouse moves the gun inside an 18° × 12° box; the camera follows only past
  its edge (stiffness 14, 35 aimed); ADS collapses the box so the sight lines up. Dutch roll
  from turning speed and strafing (±7°), a small vest shock per shot.
- **Handling**: 6-DOF spring rig on the gun (recoil impulses: kick back, climb, twist); near a
  wall (0.65 m of barrel past the body) the gun folds up to high ready. **Shots leave the
  muzzle along the barrel**, not the screen centre (no crosshair).
- **HUD**: [REC] stamp with local time and a device id; no crosshair, health or ammo numbers.
  Money, timer, score, kill feed, freeze time and the buy menu stay (the round loop needs them).
  **Ammo check**: R on a full magazine turns the gun and shows the ammo panel for 2 s (it also
  shows while reloading). This replaces the spec's press-check animation (no arm rig yet).
- Muzzle flash light (both styles): one pooled 45 ms point light at the muzzle, no shadows.
- Control port: `video.camera_style {style}`, `video.bodycam` (state; `{turn, strafe, fire, ms}`
  drives the player for screenshots). Visual check: `scripts/bodycam.json`.

## Debrief screen
- Match result + round scores.
- **"What they learned"**: top 3 player-model insights.
- **"How they changed"**: biggest trait shifts of the new squad vs the last one, e.g. `Flanking ▲ +32%`, `Reaction 410 → 360 ms`.
- Small chart: squad fitness and your win rate by generation.
- Buttons: Next match, Reset AI, Quit.

## Debug overlay (F3)
Gizmos per enemy: current goal label, path, vision cone, predicted player spot, and the NEAT output bars.

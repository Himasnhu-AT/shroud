# Gameplay

## Loop
`Menu → Playing (match) → Evolving → Debrief → Playing …`

**Match:** first to 3 rounds (at most 5), you vs the squad. Main menu "Play" → setup (map cards, then
difficulty) → "Start match". Rounds are bomb defusal (see Objective below). No respawns within a round. Full HP and refilled ammo each round. The squad only evolves *between
matches*, and only in technique.

## Round economy (`economy.rs`, rules in `brain::economy`)
- **Freeze time** (8 s) at round start: look and buy, no moving or firing; the squad holds too. The
  HUD timer shows the countdown. Headless/test-driven runs have none (`round.freeze` sets it).
- **Money:** start $800; round win +$3,250; loss +$1,400, +$500 per further loss in a row (cap
  $3,400); kill reward per gun (`WeaponKind::kill_reward`); max $16,000. Resets each match.
- **Buy menu (B):** during freeze time, and for 20 s after it within 8 m of your spawn. Columns:
  Pistols, SMGs & Shotguns, Rifles & Snipers (sorted from each gun's spec, so new guns appear by
  themselves), Gear (Vest $650, Vest + helmet $1,000, helmet upgrade $350). Keys 1–9, 0 or click;
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

## Maps (flat, 64–90 m; `brain::arena::Arena::build(MapId)`)
Picked in the menu (`Settings.map`); the map switches when a new match starts. All maps are
axis-aligned boxes so the 2D AI and trainer see the same layout as the 3D game. Low cover is 1.1 m:
crouch behind it to hide, stand to shoot over it. Every map is a **bomb-defusal** layout: the player
(attacker) spawns south inside an orange border; the squad (6+ spawns) defends north inside a blue
one, between two plant zones, **A** and **B** (`Arena::sites`, declared with `Builder::site` or zones
named "A site" / "B site"). Sites carry big painted letters on walls and floor and crates or
containers as default plant cover; yellow direction signs ("< A", "B >", "A B ^") lead from spawn.
The sign font has only A, B, D, I, M, T and arrows, so signs carry site letters, not callout names.
Named zones are used in callouts and debrief text. `cargo run -p brain --example map_ascii -- foundry`
prints a map's nav grid. `brain/tests/maps.rs` checks every map: sites A and B with plantable floor,
reachable from both spawns, no overlapping geometry, a sim round that plays out.

- **Foundry** (80 m, default): a steelworks on a hazy evening. **A** is the casting hall (furnace,
  ladles, ingot moulds, a control room; roller door off the slag road, a side door off the hall
  apron, the defenders' door on the east). **B** is the loading docks (trailers backed onto the
  dock shed, steel coils, a container). Routes: *the slag road* through the scrap yard and an
  S-bend between two scrap balers; *the boiler house* off the ladle track to *the hall apron*;
  *the ladle track* with a ladle car on rails and a conveyor overhead, held from *the beam gate*;
  *the scale house* into the docks; *the rail spur* past hopper wagons and through *the ramp
  gate*. *The conveyor cut* and *the spur cut* link the lanes; defenders rotate through the
  hall's east door and *the dock arch*. Tour: `scripts/foundry-tour.json`.
- **Depot** (64 m): a sun-baked bus depot built around a hub. **A** is the coach yard in the
  middle of the north, right in front of the defender spawn (two parked coaches, a box stack;
  the plant zone is the yard's west part, up to the east coach); **B** is the cistern hall, a
  roofed pump hall in the north-east with three doors (the plant zone is the back of the hall,
  past the cistern tank). The two plant zones are 14 m apart, so the squad can tell which site
  a sighting is on. From *the bus
  bays* (the open lot by the attacker spawn) three ways lead to *the forecourt* in front of A:
  *the freight lane* (an open container yard on the west, which also enters A from the side
  through the *freight gate*), *the arcade* (two shops, a door on every side), and *the
  gatehouse* passage under a gate arch. B is entered up *the underpass* (the east road, under a
  road bridge), through a side door off the forecourt corner of A, or by the defenders' back
  door. Tours: `scripts/maps-tour.json`, `scripts/sites-tour.json`.
- **Ledger** (map id `office`): one roofed floor of a records office, close quarters. **A** is the server room (east corridor door, a
  kitchen door, the north-east office behind); **B** is the records room on the west flank
  (west corridor door, the west office, the north-west office behind). Two long corridors with flank rooms, open-plan cubicles around a
  conference room, reception and lobby south. Every room has 2+ doorways (2.2 m).
- **Township** (90 m): a town block on a late afternoon. **A** is the warehouse loading yard
  (north-west), **B** the office courtyard (north-east); the squad spawns on north Main Street
  between them, the player south behind the bus.
- **Sirocco** (80 m): whitewashed hill town, two lanes and a church. **A** is the fountain plaza
  (north-west), **B** the market under a canopy (east, half-way up the map). West lane: *the lower
  street* along the town wall, a jog east into *the high street*, which opens into A from the
  south; *the baker's house* is a second way in. East lane: *the dog-leg*, an L-shaped alley off
  the square that weaves north between stair houses into the side of B. In the middle the
  attackers' square (*the procession road*, deep enough to fight in) leads into *the church*
  (pews, an altar screen, doors offset so no line runs straight through;
  a side door onto *the vine alley*), whose north
  door opens on *the bell arch* square in front of the defender spawn. From there defenders take
  *the west arch* to A and *the east arch* to B. *The cellars* are a roofed cross-link from the
  lower street to the square. Tour: `scripts/sirocco-tour.json`.
- **Gantry** (80 m): harbour rail yard. A inside the warehouse, parked trains across the rail
  yard (*the centre track* runs through *the signal gate*), B on the quay
  under the crane. Tour: `scripts/gantry-tour.json`.
- `scripts/sites-tour.json` shoots the sites of Ledger (`office`), Township and Depot.

Display names come from `MapId::label`. Saves store the variant (`Office`), and the website keys
maps by the lower-case variant (`office`), so a display rename never breaks either.

## Player
| | |
|---|---|
| Move | WASD, Shift sprint, Space jump, Ctrl/C crouch (eye height 1.0 m) |
| Look | mouse (cursor locked while playing; Esc releases) |
| Shoot / reload | LMB / R |
| Weapon light | T (on the gun, both camera styles; stays on across rounds) |
| Plant bomb / take gun | hold E (in a site with the bomb: plant; a gun in reach wins) |
| HP | 100 |

## Objective: bomb defusal (`bomb.rs`, rules in `brain::objective`)
You attack, the squad defends. You start every round carrying the bomb ("BOMB" tag above health).
- **Plant:** stand in site A or B (painted outline and letter on the floor), round live, hold **E**
  for 3.2 s. You can't move or fire while planting; letting go resets. +$300.
- **Planted:** the round timer becomes a pulsing red bomb with no exact fuse shown. It beeps faster
  as its 40 s fuse runs, then explodes: lethal within 8 m, falling off to nothing at 22 m (halved
  behind cover past 8 m), screen shake and flash.
- **Defuse:** a defender standing on the bomb for 7 s defuses it ("DEFUSING" under the enemy pips,
  and you hear it). Stepping off or dying resets it.
- **Who wins:** before a plant: squad dead → you; you dead or the clock (1:55) out → the squad.
  After a plant the clock and your death don't matter: explodes → you, defused → the squad, squad dead
  → you. Dead after planting, you stay down and watch ("Bomb ticking") until the bomb decides it.
- **Money:** planted but lost → +$800 on top of the loss pay.
- Control port: `bomb.state`, `bomb.plant {site, fuse, plant}`, `bomb.defuse {member, teleport}`;
  trace events `bomb.planted`, `bomb.defused`, `bomb.exploded`. Visual check:
  `scripts/bomb-plant.json`.

## Career: XP, ranks, match objectives (`career.rs`, rules in `brain::career`)
- **Ranks:** Cadet (0 XP), Private (1,000), Corporal (2,500), Sergeant (4,500), Staff Sergeant
  (7,000), Lieutenant (10,000), Captain (14,000), Major (19,000). XP only goes up. Rank names never
  reuse a difficulty name (Recruit, Regular, Veteran, Elite). The save stores only XP; the rank is
  derived, so renaming ranks never breaks a save.
- **XP per finished match:** win +500 (loss +100), +50 per round won, plus each completed
  objective's XP. An abandoned match (pause → Restart match / Main menu) pays nothing.
- **Objectives:** every match rolls 3 (Cadet) to 6 (Captain, Major) distinct objectives from the
  rank, the map's site letters and a per-match seed, harder and worth more as you climb: plant on
  site X, plant N times, let it explode, N kills / headshots / kills with a named gun, N kills in
  one round, ace, kill a defuser, fast plant, flawless round, win without a primary, win streak.
  Only kills with your guns count (not the bomb, not frags).
- **HUD:** the tracker sits top left under the money: each objective with "3/5" or a green check
  and its XP, the rank and a thin XP bar. In round 1 (freeze time and the first 6 s) it opens up as
  "MATCH OBJECTIVES" with each reward. Completing one mid-match shows "OBJECTIVE COMPLETE +200 XP".
- **Debrief:** the career panel lists match/round/objective XP and the total, fills the XP bar from
  the old XP to the new, and pops "PROMOTED TO SERGEANT" when it crosses a rank; the objectives are
  listed done or not. **Main menu:** a "Your rank" card with XP to the next rank.
- **Save:** `<data dir>/career.json` (`xp`, `matches`, `wins`, `objectives_done`), written after
  each credited match. Missing → fresh career; corrupt → moved to `career.json.corrupt`.
- Control port: `career.state` (rank, XP, objectives with progress, the match log, last award),
  `career.set_xp {xp}`, `career.reroll {seed}`; trace events `career.objectives`,
  `career.objective`, `career.credited`, `career.promoted`. Visual check: `scripts/career.json`.

## Weapons (`brain::weapons`, one table for the player, the squad and the trainer)
| Gun | Type | Price | Damage (headshot ×) | Rate | Mag | Notes |
|---|---|---|---|---|---|---|
| Vector-9 | pistol, sidearm | $200 | 22 (×2.0) | 6.5/s semi | 15 | everyone carries one |
| Warden | heavy pistol, sidearm | $700 | 48 (×2.4) | 2.6/s semi | 7 | one-tap headshot up close |
| Breacher | pump shotgun | $1,100 | 13 × 9 pellets (×1.5) | 0.85 s pump | 6 | brutal inside 8 m |
| Hornet | SMG | $1,250 | 18 (×1.8) | 14/s auto | 32 | close range, fast |
| Longbow | scoped bolt scout | $1,700 | 70 (×2.5) | 1.1 s bolt | 5 | cheap long gun, mobile |
| Shrike | battle rifle | $2,700 | 34 (×3.0) | 8.5/s auto | 30 | one-tap headshot, more kick |
| Kestrel | assault rifle | $2,900 | 25 (×2.0) | 10/s auto | 30 | the all-rounder |
| Osprey | heavy bolt sniper | $4,750 | 115 (×2.0) | 1.5 s bolt | 5 | one-shot body kill |

Hitscan with spread, recoil kick and range falloff; rifles, scopes and the Warden penetrate thin
cover. Grenades: frag $300, flashbang $200 (carry 2), smoke $300. Enemy hits use the fixed
fairness damage, scaled only by difficulty (see [AI.md](AI.md) §5).

### Combat feel (visual and sound only; no stat above depends on it)
- **People.** The enemy is the MetaHuman (`assets/models/metahuman/enemy_mh.glb`, local only)
  whenever it exists, else the CC0 mannequin. Over its uniform: loose ripstop trouser legs and
  sleeves on the limb bones, combat boots fitted to the feet, leather gloves, a nylon (cordura)
  plate carrier and helmet cover per squad look (`enemy/skin.rs`). Animation: aim offsets
  (Up/Neutral/Down blend + spine look-at), pelvis twist for strafing, hit reacts on chest and
  head, deaths that fall away from the shot (`enemy/animation.rs`).
- **First person.** MetaHuman hands in black leather gloves, a charcoal combat shirt
  (`weapon/viewmodel.rs`). Guns are photoscanned black coated steel and scuffed polymer on
  box-projected UVs (`weapon/finish.rs`, `meshprep.rs`). Recoil kick and recovery, look sway on a
  per-gun spring (heavier guns swing further), lag when strafing or falling, sprint pose,
  reloads that drop the spent magazine (it lies on the floor for 12 s).
- **Bullets** (`weapon/fx.rs`): per-surface impacts (concrete/plaster dust and chips, metal
  sparks, wood splinters) and persistent bullet holes (96 pooled decals, oldest reused; generated
  textures in `weapon/fx/decals.rs`); a muzzle flash with a short real light (shadowed on High and
  Ultra only while lit); brass that tumbles and bounces with its tinkle; a tracer on about one
  round in three. Hits on people: a short blood mist and droplets, spatter on the surface behind
  (≤ 2.6 m) and sometimes drops below. **Settings → Gameplay → Blood** (on by default) turns
  that into a grey puff with no decals.
- **Sound.** Enemy rounds passing close crack (supersonic N-wave + whiz); the player's shots
  carry a room tail under a roof and an open-air rumble with slap-back outside
  (`weapon/sound.rs`; the layers are made by `scripts/make-combat-sounds.py`).
- Close-up checks: `scripts/combat-look.json`, `impacts-look.json`, `reload-look.json`;
  cost: `scripts/combat-perf.json` ([PERF.md](PERF.md)).

## HUD
Crosshair, hit marker, HP, armor, money (with +/- changes), ammo, enemies left, round score and
round N/5, freeze countdown, a "B Buy menu" hint while buying is possible, and **Generation N** in
the corner.

## Camera style: Bodycam (Settings → Video; the default)
Bodycam (`bodycam/`) is the default camera style and turns the round into chest-cam footage.
Classic (crosshair, full HUD, the HUD above) stays in Settings → Video. Settings files from before
Nov 2026 switch to Bodycam once (`ShroudSettings::rev`); picking Classic afterwards sticks.
- **Lens and sensor**: the engine's body camera (`engine::look::BodycamLook`, engine
  docs/LOOK.md "Body camera"), `chest_cam` preset tuned in `bodycam::lens::chest_cam`, rendered
  at 100° vertical (≈ 130° horizontal): a true fisheye (corners kept, centre magnified), a
  metering exposure that lags, overshoots and clips (≈ 4 s to adapt into a dark room, faster
  back out; it replaces auto exposure on the camera; the sensor's gain runs out at EV 3, so dark
  rooms stay dark and noisy), crushed blacks and hard-clipped highlights, a little
  desaturation, noise that grows in the dark, sharpening halos, motion smear and compression
  blocks on fast turns, rolling shutter, edge chromatic aberration, vignette, halation around
  what clips. What runs depends on the quality tier: Low fisheye + curve + vignette, Medium +
  CA, sharpening, noise, High/Ultra + smear, blocks, rolling shutter; Ultra renders 1.25×
  overscan. No pipeline rebuild when switching styles.
- **Free aim**: the mouse moves the gun inside a 6° × 4° box; past its edge the camera turns
  with the mouse in the same frame (no smoothing: turning is immediate); ADS collapses the box so
  the sight lines up. Dutch roll from turning speed and strafing (±7°), a small vest shock per
  shot. The chest mount is pulled back towards the eyes by a 0.12 m sphere cast so the camera
  never ends up in or past a wall (near plane 0.03 m).
- **Handling**: 6-DOF spring rig on the gun (recoil impulses: kick back, climb, twist); near a
  wall (0.65 m of barrel past the body) the gun folds up to high ready. **Shots leave the
  muzzle along the barrel**, not the screen centre (no crosshair).
- **HUD**: [REC] stamp with local time and a device id (`UNIT 4 // BWC-X84-9921`); no crosshair, health or ammo numbers.
  The hit marker appears where the hit landed, mapped through the fisheye
  (`engine::look::world_to_screen`), not at the screen centre.
  Money, timer, score, kill feed, freeze time and the buy menu stay (the round loop needs them).
  **Ammo check**: R on a full magazine turns the gun and shows the ammo panel for 2 s (it also
  shows while reloading). This replaces the spec's press-check animation (no arm rig yet).
- **Mount** (Video → "Bodycam mount", shown only in Bodycam): Head (default, eye height) or
  Chest (spec §2.1: 0.22 m lower, 0.08 m right, the dutch roll ×1.35, a side-to-side step bob
  and a breath). Only the view moves: shots still leave the barrel, and the body, hitboxes
  and every AI sight line stay at the head. Check: `scripts/bodycam-chest.json`.
- **Weapon light** (both styles, **T**): the engine's `WeaponLight::tactical()` (a shadowed
  spot plus a dim spill) riding the gun's rail just ahead of the muzzle and along the barrel
  (`weapon::light`), so its hot spot is where the shots go and props throw thin hard shadows the
  lens sees. It never loses its shadow to the lamp budget; the map lamps give up one shadow slot
  while it shines. Lights don't change what the enemies perceive. Control port: `player.light {on?}`.
- Muzzle flash light (both styles): one pooled 45 ms point light at the muzzle. It casts
  shadows (only while lit) on High and Ultra unless the render governor is cutting game
  levels; measured cost on High: p95 within ±0.3 ms on Ledger and Sirocco
  (`scripts/muzzle-shadows-perf.json`; dev A/B `muzzle.shadows {on}`).
- Control port: `video.camera_style {style}`, `video.bodycam` (state, incl. `mount_offset`;
  `{turn, strafe, fire, ms}` drives the player for screenshots); the engine's `look.get` /
  `look.set` show and patch the live bodycam (`lens.bodycam`). Visual checks:
  `scripts/bodycam.json`, `scripts/look-foundry.json`, `scripts/look-office.json`.

## Smoothness: quality governor, warm-up, power-on
- The engine's quality governor (`quality.rs`) never changes anything during a live round: it
  may step once per round, in freeze time after 3 s (pipelines warm), shadow resolution and the
  lamp-shadow budget before any render-scale step (never below 75 %). During live play its
  ladder is cut at the level it chose. No toast in a round (logged; perf overlay). A render
  scale change mid-play shows a black frame and a mis-sized frame (`.tmp` evidence in the
  render branch report), which is what the owner saw as glitches.
- Round start: pipelines are warmed (`gfx.rs`), plus one impact per surface and a tracer inside
  the floor slab (`weapon/warm.rs`; the first shot and first impact stalled 0.3 s and 1 s),
  under a black "camera power-on" cover that fades in once the warm-up frames are done.
- Dev: `--governor` turns the governor on in play.py runs; `scripts/governor-rounds.json`,
  `scripts/spawn-hitches.json` measure it.

## Lighting: per-map looks and baked light
Each map has one engine `Look` (`map/look.rs`, `look_of`): sun, sky, fog, volumetric shafts
(Foundry; Ultra only, they cost 5-6 ms GPU on High), exposure for Classic, grading like camera footage (`footage`: a little desaturated,
contrasty mids, hot highlights; shade is cool because the blue sky lights it, sunlit faces warm).
Sun and sky are in physical units (lux, cd/m²); flat ambient is only a fallback fill for maps
without a bake. SSAO, contact shadows and TAA come with High and Ultra. Ledger (Office) is
baked (draft quality, 4.8 MB in `assets/baked/office/`); Foundry is not yet. Lamp-out check:
`scripts/bake-lamp-out.json`; looks: `scripts/look-foundry.json`, `scripts/look-office.json`;
cost on High: `scripts/perf-render.json`.

**Baked lighting** (`map/bake.rs`, engine docs/BAKE.md): lightmaps on the static level and one
light-probe volume per map, from headless Blender, cached in `assets/baked/<map>/`
(`MapId::Office` → `office`). A map without a cache keeps the look's ambient and sky light.
- Static = every `ArenaPiece` (`Scenery`); moving things are `Actor` and never baked; lamp glows
  are `NoBake`.
- Map lamps are baked `Indirect`: direct light stays realtime (they can be shot out); their
  bounce is baked and stays when one dies, until the round ends (a fraction of the fill at the
  spot; see `bake.rs`). Sun and sky are baked (the sun's direct light stays realtime).
- Probes: one box over the playable area, floor to the highest roof, ≤ 1.5 m apart.
- Bake a map (Blender 4.2+ in /Applications): `python3 -I scripts/play.py
  scripts/bake-<map>.json --out .tmp/bake` (scripts exist for Office and Foundry; copy one for
  another map), then commit `assets/baked/<map>/`. A changed sun/sky/lamp or level geometry
  makes the bake stale (`bake.status`): re-bake. `bake.unload` A/Bs it live.

## Breakable lights
Every map lamp (`Decor::lamps`; its index is the lamp id) breaks when any bullet's path, the
player's or an enemy's, crosses its fixture: the light goes off, the panel or bulb turns to dark
glass, sparks, an impact sound and a few shards. It stays out for the round (all relit at round
start); `Arena::dark_lamps` tells the AI. Shot-out lamps leave the nearest-K shadow ranking so the
next lamp in line takes the shadow. Control port: `lamp.info` (fixtures, broken ids, the lamps
lighting the player), `lamp.break {id}`. Checks: `scripts/lamps-office.json` (shoot one out),
`scripts/doctrine-lamps.json` (the squad shoots out the lamp over a camper).

## Debrief screen
- Match result + round scores.
- **Career**: XP breakdown, XP bar, promotion, objectives (see Career above). It replaces the
  "How they changed / History" row, and trims "What to work on" by one line per objective past three
  so the screen still fits.
- **"What they learned"**: top 3 player-model insights.
- **"How they changed"**: biggest technique shifts of the new squad vs the last one, e.g.
  `Flanking ▲ +32%`, `Teamwork ▲ +18%`. Never aim or reaction: those are fixed (they get smarter,
  never faster).
- Small chart: squad fitness and your win rate by generation.
- Buttons: Next match, Retire this squad (starts a fresh squad), Quit.

## Debug overlay (F3)
Gizmos per enemy: current goal label, path, vision cone, predicted player spot, and the NEAT output bars.

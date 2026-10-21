# Gameplay

## Loop
`Menu → Playing (match) → Evolving → Debrief → Playing …`

**Match:** best of 3 rounds, you vs a squad of 5. A round ends when you die or the whole squad is dead.
No respawns within a round. Full HP and ammo reset between rounds. The squad only evolves *between matches*.

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
Crosshair, hit marker, HP, ammo, enemies left, round score, and **Generation N** in the corner.

## Debrief screen
- Match result + round scores.
- **"What they learned"**: top 3 player-model insights.
- **"How they changed"**: biggest trait shifts of the new squad vs the last one, e.g. `Flanking ▲ +32%`, `Reaction 410 → 360 ms`.
- Small chart: squad fitness and your win rate by generation.
- Buttons: Next match, Reset AI, Quit.

## Debug overlay (F3)
Gizmos per enemy: current goal label, path, vision cone, predicted player spot, and the NEAT output bars.

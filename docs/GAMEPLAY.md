# Gameplay

## Loop
`Menu → Playing (match) → Evolving → Debrief → Playing …`

**Match:** best of 3 rounds, you vs a squad of 5. A round ends when you die or the whole squad is dead.
No respawns within a round. Full HP and ammo reset between rounds. The squad only evolves *between matches*.

## Arena: "Foundry" (64 × 64 m, flat)
- Center tower (tall), lane walls at x = ±14 with a gap mid-lane, yard pillars, lane blockers.
- Low cover (1.1 m): crates, long low walls by the spawns. Crouch behind it to hide; stand to shoot over it.
- Player spawns south (0, 29); enemies spawn along the north edge.
- Named zones (center tower, north/south yard, east/west lanes, four corners, inner sides) are used in debrief text.

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

# Assets

All assets are **CC0**. The game must always run without them (primitive fallback).

| Path | Use | Source | Fallback |
|---|---|---|---|
| `assets/models/weapons/*.glb` | guns (first person, enemy hands, pickups) and their optics | Pichuliru CC0 gun set (poly.pizza); finishes applied in `weapon/finish.rs` | box primitives per gun |
| `assets/textures/<set>/*` | PBR sets (cooked KTX2; 4K sets ship KTX2 only); `gun_metal`, `gun_polymer` (gun finishes), `glove_leather` (gloves, boots), `cordura` (plate carriers, helmet covers) | ambientCG | flat colours |
| `assets/models/metahuman/*.glb` | MetaHuman enemy and first-person hands, **local only** (Epic MetaHuman licence; gitignored, never committed or shipped), made by `scripts/make-metahuman.sh` | Epic MetaHuman | the CC0 mannequin and primitive hands |
| `assets/sky/*.ktx2` | HDRI sky cubemaps (Sirocco, Gantry; Office fill) from Poly Haven via `scripts/hdri_to_cube.py` | Poly Haven | the map's gradient sky |
| `assets/models/enemy.glb` | enemy character with Idle / Run / Shoot / Death clips | Quaternius animated characters, or Kenney **Animated Characters** (glTF) | capsule + visor box |
| `assets/models/props/*.glb` | crates, barrels, sci-fi set dressing | Kenney **Space Kit** / Quaternius sci-fi | none (plain blocks) |
| `assets/audio/*.ogg` | shots, hits, footsteps, foley; `bullet_crack_*`, `shot_tail_*` are synthesized by `scripts/make-combat-sounds.py` | Kenney audio packs, this project | silent |

Rules:
- Gameplay collision always comes from `brain::arena` blocks, never from mesh colliders. Models are visual only.
- Animation clip names are mapped in one table in `enemy/animation.rs`; if a clip is missing, keep the current pose.
- Check each pack's license file before adding it, and record it in `assets/CREDITS.md` (pack, author, URL, license).
- Keep the total under ~50 MB (disk is tight). Prefer `.glb` and drop unused files from the packs.

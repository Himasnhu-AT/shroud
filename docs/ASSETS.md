# Assets

All assets are **CC0**. The game must always run without them (primitive fallback).

| Path | Use | Source | Fallback |
|---|---|---|---|
| `assets/models/weapons/*.glb` | guns (first person, enemy hands, pickups) and their optics | Pichuliru CC0 gun set (poly.pizza); finishes applied in `weapon/finish.rs` | box primitives per gun |
| `assets/models/enemy.glb` | enemy character with Idle / Run / Shoot / Death clips | Quaternius animated characters, or Kenney **Animated Characters** (glTF) | capsule + visor box |
| `assets/models/props/*.glb` | crates, barrels, sci-fi set dressing | Kenney **Space Kit** / Quaternius sci-fi | none (plain blocks) |
| `assets/audio/*.ogg` | shots, hits, footsteps (stretch) | Kenney audio packs | silent |

Rules:
- Gameplay collision always comes from `brain::arena` blocks, never from mesh colliders. Models are visual only.
- Animation clip names are mapped in one table in `enemy/animation.rs`; if a clip is missing, keep the current pose.
- Check each pack's license file before adding it, and record it in `assets/CREDITS.md` (pack, author, URL, license).
- Keep the total under ~50 MB (disk is tight). Prefer `.glb` and drop unused files from the packs.

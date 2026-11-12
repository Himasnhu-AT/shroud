# Assets

All committed assets are **CC0**. The game must always run without them (primitive fallback).
Real scans from Fab (Megascans under the Fab Standard License, a CC-BY pallet) are **local only**:
see "Fab scans" below.

| Path | Use | Source | Fallback |
|---|---|---|---|
| `assets/models/weapons/*.glb` | guns (first person, enemy hands, pickups) and their optics | Pichuliru CC0 gun set (poly.pizza); finishes applied in `weapon/finish.rs` | box primitives per gun |
| `assets/textures/<set>/*` | PBR sets (cooked KTX2; 4K sets ship KTX2 only); `gun_metal`, `gun_polymer` (gun finishes), `glove_leather` (gloves, boots), `cordura` (plate carriers, helmet covers) | ambientCG | flat colours |
| `assets/library/{textures,decals,props}/*` | Foundry's photoscanned surfaces, decals, fixtures and props (imported with the engine's `assets.py`, Megascans layout; ledger `library/LICENSES.json`) | Poly Haven, ambientCG | flat colours / procedural grime / box fixtures |
| `assets/library/{textures,props}/ms-*`, `fab-*` | Foundry's real scans: floor, painted walls, barrier, drums, rubble, workbench, pallets, scrap sheet (engine importer: decimated LODs + one cooked texture set per prop) | Quixel Megascans and sellers on Fab, **local only** (never committed) | the CC0 stand-ins above (scan-only extras are left out) |
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

## Fab scans (local only)

The owner downloads Fab listings by hand; `~/codes/game/engine/scripts/assets.py import <zip>`
(with `ENGINE_ASSETS_ROOT` set to this repo) recognises Megascans "High" FBX downloads, the
"UE" PNG surface layout and plain FBX scan packs, decimates LODs with Blender and cooks one 2K
KTX2 set per item (engine `docs/ASSETS.md`, "Real Fab downloads"). The exact commands, listing
URLs and LOD caps are in `assets/CREDITS.md`.

- **Never committed.** Fab's Standard License forbids redistributing the files, so `.gitignore`
  drops `assets/library/*/ms-*/` and `*/fab-*/` whole; only their `library/LICENSES.json` entries
  and `CREDITS.md` rows are committed. Re-import from the owner's zips to get them back.
- **Fallback.** Every use names the scan and its CC0 stand-in; the scan wins only when its files
  are on disk (`look.rs` `Spec::or_scan` for surfaces, `industry::Model::present` /
  `pick_model` for props). A fresh clone builds Foundry from the stand-ins, and the scan-only
  extras (corner rubble, workbench, pallets, scrap sheets) are simply absent.
- **Baked light.** The bake matches meshes by geometry and placement, not material
  (engine `docs/BAKE.md`): walls and floors are the same merged boxes either way, so their
  lightmaps apply on a clone without the scans. Props that differ (the 5.4 m barrier against the
  1.6 m stand-in, the scanned drums on their pallet against Poly Haven's drums) are lightmapped in
  the committed bake only as scans; on a clone the stand-ins have no entry and take the probe
  volume's light instead (the bake reports them as `stale` and still applies, `allow_stale`).
  Their floor shadows are baked from the scans, which fill the same blocks. The scan-only extras
  are `NoBake`, so a clone shows no baked shadow of something that isn't there.

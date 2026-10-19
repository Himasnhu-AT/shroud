# Asset credits

All assets are CC0 1.0 Universal (public domain dedication). Attribution is not required; it is given here anyway.

| File | Pack | Author | URL | License |
|---|---|---|---|---|
| `models/rifle.glb` | Blaster Kit 2.1 (model `blaster-e`, with its `scope` and `magazine` meshes) | Kenney (kenney.nl) | https://kenney.nl/assets/blaster-kit | CC0 1.0 |
| `models/enemy_rifle.glb` | Blaster Kit 2.1 (model `blaster-a`, with its `magazine` mesh) | Kenney (kenney.nl) | https://kenney.nl/assets/blaster-kit | CC0 1.0 |
| `models/props/crate-medium.glb` | Blaster Kit 2.1 (`crate-medium`) | Kenney (kenney.nl) | https://kenney.nl/assets/blaster-kit | CC0 1.0 |
| `models/props/crate-small.glb` | Blaster Kit 2.1 (`crate-small`) | Kenney (kenney.nl) | https://kenney.nl/assets/blaster-kit | CC0 1.0 |
| `models/enemy.glb` | Universal Animation Library [Standard] (`Unreal-Godot/UAL1_Standard.glb`, unmodified) | Quaternius (quaternius.com) | https://quaternius.itch.io/universal-animation-library · https://quaternius.com/packs/universalanimationlibrary.html | CC0 1.0 |
| `textures/floor/*` | Concrete047A (1K JPG: Color, NormalGL, Roughness, AmbientOcclusion) | ambientCG (Lennart Demes) | https://ambientcg.com/view?id=Concrete047A | CC0 1.0 |
| `textures/wall/*` | Concrete034 (1K JPG: Color, NormalGL, Roughness) | ambientCG (Lennart Demes) | https://ambientcg.com/view?id=Concrete034 | CC0 1.0 |
| `textures/tall/*` | PaintedMetal006 (1K JPG: Color, NormalGL, Roughness, AmbientOcclusion) | ambientCG (Lennart Demes) | https://ambientcg.com/view?id=PaintedMetal006 | CC0 1.0 |

Notes:
- ambientCG maps are renamed to `color.jpg`, `normal.jpg` (OpenGL convention), `rough.jpg` and `ao.jpg` and are otherwise unmodified. Mipmaps are built at load time (`gfx.rs`).
- The Blaster Kit GLBs referenced their colour map as an external file (`Textures/colormap.png`). The texture was embedded into each GLB so the file is self-contained. Geometry and materials are otherwise unchanged.
- The Kenney "Animated Characters" packs were checked and rejected: they only ship FBX and contain idle, run and jump clips, with no shoot or death.
- Animation clip names are listed in `ANIMATIONS.md`.

## Audio (`audio/*.ogg`), all CC0
Processed with ffmpeg (trim, pitch, filter, layering) and re-encoded to Vorbis.

| File | Source | Author | URL |
|---|---|---|---|
| `audio/rifle_shot.ogg` | Sci-fi Sounds `lowFrequency_explosion_000` + `laserSmall_000`, Impact Sounds `impactMetal_heavy_000`, plus synthesized noise crack | Kenney | https://kenney.nl/assets/sci-fi-sounds · https://kenney.nl/assets/impact-sounds |
| `audio/enemy_shot.ogg` | Sci-fi `lowFrequency_explosion_001` + Impact `impactMetal_heavy_002`, lowpassed with echo tail (distant) | Kenney | same |
| `audio/impact_0..2.ogg` | Impact Sounds `impactGeneric_light_000/002`, `impactPlate_light_001` | Kenney | https://kenney.nl/assets/impact-sounds |
| `audio/ricochet.ogg` | Sci-fi Sounds `laserRetro_002`, pitched up | Kenney | https://kenney.nl/assets/sci-fi-sounds |
| `audio/hit_marker.ogg` | Interface Sounds `tick_002` | Kenney | https://kenney.nl/assets/interface-sounds |
| `audio/headshot.ogg` | Impact Sounds `impactBell_heavy_000` | Kenney | https://kenney.nl/assets/impact-sounds |
| `audio/kill_confirm.ogg` | Interface Sounds `confirmation_004` | Kenney | https://kenney.nl/assets/interface-sounds |
| `audio/player_hurt.ogg` | Impact Sounds `impactPunch_heavy_000` | Kenney | https://kenney.nl/assets/impact-sounds |
| `audio/heartbeat.ogg` | Synthesized with ffmpeg | this project | n/a |
| `audio/step_0..4.ogg` | Impact Sounds `footstep_concrete_000..004` | Kenney | https://kenney.nl/assets/impact-sounds |
| `audio/reload_start.ogg`, `reload_end.ogg` | "Gun reload sounds" `assaultriflereload1` split in two | SpringySpringo (OpenGameArt) | https://opengameart.org/content/gun-reload-sounds |
| `audio/empty_click.ogg` | Impact Sounds `impactMetal_light_002` | Kenney | https://kenney.nl/assets/impact-sounds |
| `audio/round_start.ogg` | Sci-fi Sounds `doorClose_001` | Kenney | https://kenney.nl/assets/sci-fi-sounds |
| `audio/round_won.ogg` | Interface Sounds `confirmation_002` + `003` | Kenney | https://kenney.nl/assets/interface-sounds |
| `audio/round_lost.ogg` | Interface Sounds `bong_001`, pitched down | Kenney | https://kenney.nl/assets/interface-sounds |
| `audio/ambient.ogg` | Sci-fi Sounds `spaceEngineLow_001`, lowpassed | Kenney | https://kenney.nl/assets/sci-fi-sounds |

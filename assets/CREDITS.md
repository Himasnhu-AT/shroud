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

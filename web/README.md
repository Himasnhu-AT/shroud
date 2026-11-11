# Pattern of Life website

Launch site for Pattern of Life (codename Shroud). Astro (static), GSAP + Lenis, a 2D canvas for the live belief grid.
Design record: `DESIGN.md`. All copy: `src/data/site.ts`.

```sh
pnpm install
pnpm build                 # static site in dist/
pnpm preview               # or any static server on dist/
pnpm check                 # type check
```

- **Map plans** in `src/data/maps.json` come from the game's real geometry
  (`crates/brain/src/arena.rs`). Regenerate after map changes:
  `pnpm arena` (runs `tools/arena-dump`, a tiny Rust program that depends on `brain`). Each map
  carries its own `half` (maps are 64 to 90 m across). Map pages exist only for maps that are both
  in `maps.json` and in `maps` in `src/data/site.ts`; the build warns when a name or blurb there
  differs from the game's.
- **The live grid** (`src/motion/belief.ts`) is a browser sketch of the squad coordinator's rules
  on those plans. It is labelled as an illustration on the page.
- **Images** are in-game captures, kept as WebP masters in `src/assets/shots/` and served as
  AVIF/WebP. The raw PNG captures live in `_source-shots/` (git-ignored).
- **Download links** go to the itch.io page (`ITCH_URL` / `download` in `src/data/site.ts`).
- **Before deploying**, replace every remaining placeholder in `src/data/site.ts` (release date,
  studio, repo) and set the real domain in `src/data/site.ts`,
  (`site.url`; `astro.config.mjs` and the generated `robots.txt` read it; currently the
  placeholder `shroud.marlgames.com`). Then regenerate `public/og.png` from the hero and rebuild.

## Renaming the game

The name is one constant, `NAME` in `src/data/site.ts`; every page, title, meta tag, header,
footer and alt text reads it. Still spelled out by hand: `site.url` (placeholder domain),
`public/og.png` (the image shows the name), `package.json` (`shroud-web`), the
`shroud-radio` session-storage key (`src/layouts/Base.astro`, `src/motion/setup.ts`; internal),
code comments, `DESIGN.md`, this README, and the in-game captures (the main menu shows the title).
A longer name may not fit the hero's one-line mega type: check the home page at 390 px.

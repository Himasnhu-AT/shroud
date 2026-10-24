# Shroud website

Launch site for Shroud. Astro (static), GSAP + Lenis, a 2D canvas for the live belief grid.
Design record: `DESIGN.md`. All copy: `src/data/site.ts`.

```sh
pnpm install
pnpm build                 # static site in dist/
pnpm preview               # or any static server on dist/
pnpm check                 # type check
```

- **Map plans** in `src/data/maps.json` come from the game's real geometry
  (`crates/brain/src/arena.rs`). Regenerate after map changes:
  `memgate -- pnpm arena` (runs `tools/arena-dump`, a tiny Rust program that depends on `brain`).
- **The live grid** (`src/motion/belief.ts`) is a browser sketch of the squad coordinator's rules
  on those plans. It is labelled as an illustration on the page.
- **Images** are in-game captures, kept as WebP masters in `src/assets/shots/` and served as
  AVIF/WebP. The raw PNG captures live in `_source-shots/` (git-ignored).
- **Before deploying**, replace every placeholder in `src/data/site.ts` (release, platforms,
  price, store link, studio, repo) and set the real domain in `src/data/site.ts`,
  `astro.config.mjs` and `public/robots.txt` (currently the placeholder
  `shroud.marlgames.com`). Then regenerate `public/og.png` from the hero and rebuild.

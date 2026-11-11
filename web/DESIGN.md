# Shroud website: design record

## Phase 0: Brief

- **Subject:** Shroud, a first-person shooter in Rust (Bevy 0.20 + avian3d). Solo player against
  an AI squad, bomb-defusal rounds, first to 3. The squad learns from your past matches, but only
  in technique: aim and reaction are fixed ("They get smarter, never faster"). (Written at Bevy
  0.19 with three maps; the game now has six, and Office is called Ledger.)
- **Audience:** FPS players who are tired of dumb bots; developers and Rust/gamedev people who
  will read how the AI works.
- **Job:** launch microsite. Make people understand in 3 seconds that the enemies learn, show
  the real game, explain the squad brain with real numbers, and point to where to get it
  (unknown yet: placeholder).
- **Assets:** real 1920×1080 captures (`_source-shots/`, not shipped), the game's own fonts
  (Barlow, Barlow Condensed, OFL), the real map geometry exported from `crates/brain/src/arena.rs`
  by `tools/arena-dump` into `src/data/maps.json`.
- **Language:** English. **Constraints:** no fake reviews, awards, player counts or quotes.
- **Placeholders (`placeholder: true` in `src/data/site.ts`):** release date, platforms, price,
  download/Steam link, studio name ("Marl Games" is only a candidate), repo URL, production
  domain.

## Phase 1: Concept

| | A. The belief grid (material of the AI) | B. Radio net (how the squad works) | C. Squad gen (lineage, formal) |
|---|---|---|---|
| Idea | The site watches you the way the squad does: a 16×16 belief grid over a real map plan heats up where you probably are, and four callsigns spread out to search it, then close in from different sides | Every section arrives as a staggered radio call from a callsign; the page is the squad's comms log | Scrolling advances the squad generation; each section is one generation and the layout mutates as you go |
| Anchor | `SquadCoordinator`'s shared belief grid (16×16 cells over the 64 m arena), split Search, report-and-encircle | staggered per-bot radio calls, callsigns Alpha to Foxtrot, real barks ("All set - go go go") | the HUD's "Squad gen N", GA + NEAT between matches |
| Signature | the visitor's pointer is the player: heat follows it with lag, cells clear where a squad member can see and you are not, markers take distinct routes | radio lines type in per callsign as sections enter | a generation counter ticking with mutating genome bars |
| Emotion | being hunted, calmly | eavesdropping | watching something grow |
| Screenshot | dark map plan, an orange heat bloom around your cursor, four red markers converging on it from four sides, SHROUD set huge | a column of callouts | a counter at gen 40 |
| O / F / Fe / P | 5 / 5 / 4 / 1 | 3 / 4 / 5 / 3 | 3 / 3 / 3 / 3 (needs invented numbers) |

**Pick A.** It is literally the thing that makes Shroud different, it can only belong to this
game (it runs on the game's own map geometry), and it gives the visitor the player's seat.
**Steal from B** for microcopy: radio calls are the live log beside the grid, the 404 is "Lost
contact", and the loader is a radio check.

### Concept card

```
Idea:        The site hunts you like the squad does: a belief grid over a real Shroud map
             guesses where you are, and the squad spreads out, reports, and closes in.
Metaphor:    the squad's tactical picture (plan view, occupancy heat, callsigns, radio)
Signature:   live belief grid on the hero and in "How they hunt" (src/motion/belief.ts):
             pointer = player, heat diffuses with time, cells clear under squad vision,
             Search spreads markers with repulsion (max 2 on the hot spot), Encircle takes
             distinct sectors and moves on a "go". Chapters pin the mode.
Emotion:     first 3 s: "something on this page is already looking for me."
Motion verb: spread, then close in (things fan out from a point with stagger, then converge)
Screenshot:  the hero: Foundry plan in the dark, heat on your cursor, four red chevrons
             converging, SHROUD in the game's condensed caps.
```

## Phase 2: Art direction

**Palette** (taken from the game's own HUD and menus, sampled from the captures):

| Role | Hex | Source |
|---|---|---|
| ground | `#121417` | menu backdrop slate |
| ground-2 | `#1b1f24` | HUD panels |
| ink | `#ecebe7` | menu text |
| muted | `#99a0a8` | secondary menu text (AA on ground: 7.3:1) |
| line | `rgb(236 235 231 / 0.12)` | hairlines, grid |
| accent (heat) | `#f2a33a` | the menu's selection orange; used ONLY for belief heat, the selected state and focus |
| you | `#55d6a8` | HUD "YOU" teal |
| squad | `#ee5a4f` | HUD "SQUAD" red |
| curtain | `#121417` cells with accent edges | |

Near-black + one orange accent is a listed AI default; here it is the game's own palette (its
menus are exactly this), so the site and the game read as one product. The orange carries
meaning (belief heat, selection), teal is always "you", red is always "the squad".

**Type:** the game's own families. Barlow Condensed 700 (display, uppercase, as in the game's
menus) and Barlow 400/500/600 (text and UI, tabular figures for data). Display tracking −0.01 to
−0.03em (condensed caps need less), leading 0.84. UI labels 13 to 14 px Barlow 600, tracked
+0.04em uppercase only where the game does it (callsigns, tab labels).

**Scale:** `--t-mega clamp(5.5rem, 1rem + 19vw, 21rem)` for SHROUD; xl, l, m, body 17 px.

**Grid:** 12 columns, gutter = outer margin. Vertical lines at col 1, col 5 and col 9; text
blocks start there. Section rhythm: hero full viewport, statement airy, sticky chapters long,
facts tight, maps normal, ladder tight, gallery airy, footer full.

**Shape:** radius 2 px everywhere (the game's panels are near-square). Markers are chevrons,
cells are squares.

**Motion language:** ease `expo.out` for entrances, `expo.inOut` for the transition, `power3.out`
for hovers. Spread = stagger from a point (`stagger.from` the click or the element center, grid
aware). Close in = four corner ticks converge on the target.

- Hover, primary (CTAs, nav, map rows, cards): **encircle ticks**. Four corner ticks sit 8 px
  outside at rest (hidden), and close in onto the box with 30 ms stagger; the label shifts 2 px.
  The current nav item keeps its ticks (touch sees state).
- Map rows: the ticks plus the real screenshot preview that follows the pointer (inline
  thumbnail on touch).
- Inline links: underline at rest that thickens into an orange band.
- Utility links: color only.
- Page transition: the screen is covered by grid cells that fill outward from the click point
  (belief collapsing onto a sighting), then clear outward from the centre on the new page.
- Loader (first visit only, ≤ 2.6 s): "Radio check": the same cells light in order of distance
  from a spawn point with real progress, then clear into the hero.
- Idle: the belief grid runs on gsap.ticker time: with no pointer a ghost path walks the map
  and the squad keeps searching. Paused off screen, static frame under reduced motion.

**Composition plan** (each family once on home):

```
HOME
1 Scene (hero)          [nav: SHROUD            How they hunt  Maps  Get it]
                        [            live Foundry plan + belief grid (cols 5-12)     ]
                        [ feed: first-person capture (cols 1-4)  | radio log           ]
                        [ SHROUD (mega, full width, bottom)                          ]
                        [ one line: what it is        | CTA: see how they hunt       ]
2 Statement             [ col 1-2 label |  cols 3-11 large paragraph                 ]
3 Sticky narrative      [ sticky 16x16 grid (cols 1-6) | chapters (cols 8-12) x4     ]
4 Data / facts          [ three before/after numerals on the col 1/5/9 lines         ]
5 Index list            [ Foundry / Depot / Office rows + pointer preview            ]
6 Ladder (asym)         [ Recruit..Elite stepping down-right, squad pips 3..6        ]
7 Gallery wall          [ menu, setup, settings, pause, metrics hung with empty cells ]
8 Footer finale         [ "They are watching." mega | release placeholders | links   ]

HOW THEY HUNT (/squad/)  case-study structure: title + meta, hero capture, belief, choosing a
                         play (bandit, real context features), the ten plays (index), search,
                         encircle, alone, between matches, measured results, next: maps.
MAP (/maps/<id>/)        poster title + blurb, plan (real geometry, zones, 7 spawns), capture
                         pair, Office: Low/High quality comparison, next map.
404                      "Lost contact": the grid with all heat gone cold, last-seen zone.
```

**Explainer figure:** the anatomy skill is not installed. The belief grid itself is the
explainer and is drawn from the real map data, so no extra figure.

**Genericness test** (what a generic model would do, and what changed):
- Hero over a gradient blob or a looping trailer: replaced by the live plan view on real map
  geometry.
- "Features" as three equal cards: replaced by sticky chapters driven by the same simulation.
- Fake "players say" quotes: none; measured benchmark numbers from the repo instead.
- Generic gamer neon/glitch: none; the palette and type are the game's own UI.

**Honesty:** the browser grid is an illustration that follows the same rules as the game's
coordinator on the same map plan; it is labelled as such. The real one is Rust in `crates/brain`.

## QA decisions

- **Still first screens on inner pages.** /squad/ and /maps/ open on type and a real capture,
  not the live grid; the grid runs further down (Depot on /squad/, each map's own plan on its
  page). The home hero and the 404 keep the concept moving at rest. This is deliberate: the
  inner pages are for reading.
- **Loader: home only, once per session, desktop only.** The radio check runs at 768 px and up
  on the first home visit; every other arrival goes straight in, and in-site navigation uses the
  cell transition (which names the destination: "Moving to Depot").
- **"Status", not "Get Shroud".** There is nothing to buy or download yet, so the nav item and
  the finale show what is known and one line of what is still to be announced.
- **Focus** shows the accent outline on every target; the corner ticks are the hover and
  current-page treatment.
- **Heat scale** is absolute (probability × walkable cells), so a lost-contact grid reads as
  a faint wash and a fresh sighting as one hot cell.

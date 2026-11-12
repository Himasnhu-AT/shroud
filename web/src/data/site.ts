// All visible copy for the site lives here. The game's name is NAME below.
//
// Facts come from the game repo (codename Shroud: crates/brain, crates/game, README) and the squad AI rework
// benchmarks. Map geometry comes from `src/data/maps.json`, exported from crates/brain/src/arena.rs
// by tools/arena-dump.
//
// Assumptions and unknowns (every one is `placeholder: true` below and listed in the hand-off):
// - release date, platforms, price, store/download link, studio name, repo URL, production domain.
// - "Marl Games" / marlgames.com is only a candidate studio name and domain: not shown as fact.

import geo from './maps.json';

export type Placeholder = { label: string; value: string; placeholder: boolean; href?: string };

// The game's name (formerly the codename Shroud). Change it here. Other spots that still spell it out
// (none of them visible copy) are listed in README.md under "Renaming".
export const NAME = 'Tells';
// The itch.io page: live, free download for Windows and macOS.
export const ITCH_URL = 'https://himanshuat.itch.io/tells';

export const download = {
  href: ITCH_URL,
  label: 'Download free on itch.io',
  short: 'Download',
  note: 'Free for Windows and macOS.',
} as const;

export const site = {
  name: NAME,
  // Production URL: canonical, OG and sitemap read it. PLACEHOLDER: marlgames.com is only a
  // candidate studio domain; replace before deploying.
  url: 'https://shroud.marlgames.com',
  urlPlaceholder: true,
  title: `${NAME}: they get smarter, never faster`,
  description:
    `${NAME} is a tactical shooter where the enemy squad reads your tells, and plays against them. They get smarter, never faster.`,
  tagline: 'They get smarter, never faster.',
  kind: 'Tactical squad shooter',
} as const;

export const nav = [
  { href: '/squad/', label: 'How they hunt' },
  { href: '/maps/', label: 'Maps' },
  { href: '/#get', label: 'Status' },
] as const;

// What is known today, shown in the finale.
export const facts = [
  { label: 'Status', value: 'Playable, in development' },
  { label: 'Get it', value: 'Free on itch.io, for Windows and macOS' },
  { label: 'Genre', value: 'First-person shooter, solo against a squad of 3 to 6' },
  { label: 'Mode', value: 'Bomb defusal, first to 3 rounds, with a round economy and buy menu' },
  { label: 'Maps', value: 'Sirocco, Gantry, Depot, Township, Ledger, Foundry' },
  { label: 'Difficulty', value: 'Recruit, Regular, Veteran, Elite' },
  { label: 'Camera', value: 'Classic, or bodycam: a wide chest-cam lens with free aim and no crosshair' },
  { label: 'Fairness', value: 'Aim and reaction never improve. They play smarter, never faster.' },
  { label: 'Built with', value: 'Rust, Bevy 0.20, avian3d' },
] as const;

// Release details. `placeholder: true` items are still unknown and must be replaced by the owner.
export const release: Placeholder[] = [
  { label: 'Release', value: 'Date to be announced', placeholder: true },
  { label: 'Platforms', value: 'Windows and macOS', placeholder: false },
  { label: 'Price', value: 'Free', placeholder: false },
  { label: 'Store page', value: 'itch.io', placeholder: false, href: ITCH_URL },
  { label: 'Studio', value: 'To be announced', placeholder: true },
  { label: 'Source', value: 'Repository link to come', placeholder: true },
];

export const home = {
  heroLine: 'A tactical shooter where the enemy squad reads your tells, and plays against them.',
  heroHint: 'Live on Foundry: point at or tap the map. That is you.',
  heroCta: { href: '/squad/', label: 'See how they hunt' },
  statement:
    'One of you against one squad of three to six. You carry the bomb, they hold two sites: first to three rounds. Buy guns, armor and grenades between rounds. When the match ends the squad trains against a ghost of you, and the next squad is bred from the ones that did best against you. They get smarter, never faster: their aim and reaction stay fixed.',
  statementNote: 'Six maps. Pick one before each match.',
  chapters: [
    {
      id: 'belief',
      title: 'Where you probably are',
      text: 'The squad shares one picture of the map: a grid of 4 metre squares, each holding how likely you are to be there. While nobody sees you, that guess spreads a little every second. A cell only goes cold when someone looks at it and a crouched player there would have been spotted.',
      call: 'Lost him - last seen at the copy room',
    },
    {
      id: 'search',
      title: 'Split up and search',
      text: 'Lost contact means fan out. Members head for different hot spots and push away from each other, and no more than two check the hottest one. You stop meeting the whole squad at a single doorway.',
      call: 'Lost him - spreading out',
    },
    {
      id: 'encircle',
      title: 'Report and surround',
      text: 'Whoever sees you radios it and keeps you busy. The rest take different sectors around you by different routes, wait until everyone is in place, then move on one call.',
      call: 'All set - go go go',
    },
    {
      id: 'play',
      title: 'Keep what works on you',
      text: 'Before a fight the coordinator picks a play with a LinUCB contextual bandit: crossfire, pin and flank, trade or bait. It weighs your habits and the state of the fight, and the plays that paid off against you come back.',
      call: 'Flanking north-west',
    },
  ],
  results: {
    title: 'What the rework changed',
    lead: 'Benchmarks from the squad AI rework, before and after.',
    items: [
      { label: 'Search spread', before: '4.6 to 8.3', after: '24.7 to 33.5', unit: 'm', note: 'Distance the squad covers when it has lost you. It no longer swarms one point.' },
      { label: 'Toughness benchmark', before: '0.682', after: '0.601', unit: '', note: 'Player win rate across the toughness benchmark.' },
    ],
    ghost: {
      label: 'Ghost survival on Elite',
      note: 'A scripted ghost player that only tries to stay alive. Its win rate, per map, measured on the maps as they were then: Foundry has since been rebuilt as a steelworks.',
      rows: [
        { map: 'Foundry', before: 0.17, after: 0.06 },
        { map: 'Depot', before: 0.23, after: 0.15 },
        { map: 'Ledger', before: 0.25, after: 0.11 },
      ],
    },
  },
  difficulty: {
    title: 'Pick how hard they hunt',
    lead: 'Reaction, aim, damage, coordination, grenade use and how many attack at once all scale with it. Evolution never does: it only changes how they play.',
  },
  build: {
    title: 'Rust from the squad brain up',
    text: 'The game runs on Bevy 0.20 with avian3d physics. The squad brain is a separate Rust crate with no engine dependency, so the same code runs in the game and in a 2D trainer that plays rounds against your ghost while you sit in the menus.',
    engine:
      'Window, settings, menus, pause, UI kit, FPS controller, render budget, devtools and the metrics window come from a shared in-house engine.',
    port: { before: '13,467', after: '7,833', label: `lines of ${NAME} game code before and after the move onto the engine` },
  },
  get: {
    title: 'They are watching.',
    note: 'Before your first match, that is all the main menu tells you.',
    pending: 'The studio name is still to be announced.',
  },
} as const;

export const difficulties = [
  { name: 'Recruit', squad: 3, blurb: '3 enemies. Slow reactions, loose aim, no grenades.' },
  { name: 'Regular', squad: 4, blurb: '4 enemies. Fair fights; the squad coordinates.' },
  { name: 'Veteran', squad: 5, blurb: '5 enemies. Sharp aim, flanks and flashbangs.' },
  { name: 'Elite', squad: 6, blurb: "6 enemies. Every trick they've learned, used against you." },
] as const;

export type MapInfo = {
  id: 'foundry' | 'depot' | 'office' | 'sirocco' | 'gantry' | 'township'; // MapId variant, lower case
  name: string; // MapId::label
  blurb: string; // MapId::blurb, verbatim
  about: string;
  cover?: string; // key in src/assets/shots; maps without a capture yet show only the plan
  coverAlt?: string;
  coverCaption?: string; // shown under the cover on the map page (e.g. to date an old capture)
  shots: { src: string; alt: string; caption: string }[];
  compare?: { low: string; high: string; alt: string; caption: string }[];
};

export const maps: MapInfo[] = [
  {
    id: 'foundry',
    name: 'Foundry',
    blurb: 'Bomb sites in a steelworks. A in the casting hall, B at the loading docks, a ladle car on the rails down mid.',
    about:
      'A steelworks on a hazy evening, 80 metres across. A is the casting hall, with its furnace, ladles and a control room; B is the loading docks, where trailers back onto the dock shed. A long runs through the scrap yard and an S-bend between two balers, A short cuts off mid through the boiler house, mid has a ladle car on rails under a conveyor, and B ramp climbs the rail spur past hopper wagons. The west and east cuts link the lanes, and the defenders rotate through the hall’s east door and the B connector.',
    // CAPTURES: these three predate the steelworks; they show the original open yard. Replace with
    // new captures of the casting hall, mid and the loading docks.
    cover: 'enemies_foundry',
    coverAlt: 'An earlier build of Foundry, when it was an open yard: two rust-green walls frame a long sand-coloured low wall, rifle in hand.',
    coverCaption: 'An earlier build: Foundry before it became a steelworks. New captures to come.',
    shots: [
      { src: 'player_firing', alt: 'The player fires in the original Foundry yard with a red damage vignette around the screen, an enemy at the right wall and radio lines from Charlie and Delta.', caption: 'Firing back at the right flank, health at 93. Earlier build, original yard.' },
      { src: 'hud_killfeed', alt: 'The original Foundry yard: an enemy at the far end of a long low wall, and the kill feed top right listing Alpha and Bravo down.', caption: 'Two down, two pips left. Earlier build, original yard.' },
    ],
  },
  {
    id: 'depot',
    name: 'Depot',
    blurb: 'Bomb sites at a sun-baked bus depot. The freight lane west, the bus bays through the gatehouse, the roofed cistern east.',
    about:
      'A bus depot baking in the sun. The freight lane runs up the west side to the freight gate, the bus bays fill the middle and open through the gatehouse onto the forecourt, and the cistern winds up the east side under a roof, lit by a few lamps. The arcade and the underpass cut across between them. You spawn south with the bomb; the squad holds the two sites at the north end.',
    cover: 'depot_long_a',
    coverAlt: 'Depot’s freight lane: sand underfoot between a wooden fence and stucco buildings, a crate stack halfway down.',
    shots: [
      { src: 'depot_b_tunnels', alt: 'Inside Depot’s cistern: a narrow plaster corridor lit by one ceiling lamp, crates at the entrance.', caption: 'The cistern on the east side: close range, one lamp.' },
      { src: 'round_over', alt: 'Depot round over screen reading Round won, one to zero, in front of a red shipping container.', caption: 'Round won, one to nil. The next round starts on its own.' },
    ],
  },
  {
    id: 'office',
    name: 'Ledger',
    blurb: 'Bomb sites on a records-office floor. A in the server room, B in the records room; tight corridors, every corner a fight.',
    about:
      'One roofed floor of a records office: the server room (A) on the east flank, the records room (B) on the west, offices off two long corridors, open-plan cubicles around a conference room, a break room north and reception south. Doorways everywhere, so any room can be flanked.',
    cover: 'office_server_room',
    coverAlt: 'Ledger’s server room: a row of dark racks with green status lights under a grid ceiling.',
    shots: [
      { src: 'office_west_corridor', alt: 'Ledger’s west corridor: a long bright hallway with doorways on both sides and ceiling lights running to the far end.', caption: 'The west corridor: doorways on both sides, nowhere to hide.' },
      { src: 'enemies_office', alt: 'A dark conference table in front of a long wall with two doorways under a ceiling grid.', caption: 'Two doorways, one table: any room in Ledger can be flanked.' },
    ],
    compare: [
      { low: 'low_office_cubicles', high: 'high_office_cubicles', alt: 'The Ledger cubicles', caption: 'Cubicles at Low and High quality.' },
      { low: 'low_office_lobby_long', high: 'high_office_lobby_long', alt: 'The long view down the Ledger lobby', caption: 'The lobby at Low and High quality.' },
    ],
  },
  // No captures yet: these get a page with the plan only, once tools/arena-dump has exported them
  // into maps.json (`pnpm arena`). Text from docs/GAMEPLAY.md.
  {
    id: 'gantry',
    name: 'Gantry',
    blurb: 'Bomb sites in a harbour rail yard. A inside the warehouse, trains across mid, B out on the quay.',
    about:
      'A harbour rail yard, 80 metres across. A is inside the warehouse, trains stand across mid, and B is out on the quay under the crane.',
    shots: [],
  },
  {
    id: 'township',
    name: 'Township',
    blurb: 'Bomb sites in a town block on a late afternoon. A in the warehouse loading yard, B in the office courtyard; long streets, alleys, houses with windows.',
    about:
      'The biggest map, 90 metres across: a town block on a late afternoon. A is the warehouse loading yard in the north-west, B the office courtyard in the north-east. The squad spawns on north Main Street between them; you start south, behind the bus.',
    shots: [],
  },
  {
    id: 'sirocco',
    name: 'Sirocco',
    blurb: 'Bomb sites in a whitewashed hill town. A on the fountain plaza, B in the market; the high street through the town gate, the vine alley, the procession road, the cellars.',
    about:
      'A whitewashed hill town, 80 metres across. A is the fountain plaza, B the market. The lower and high streets come in through the town gate, the vine alley runs under a pergola or through the baker’s house, the procession road is the long sight line from the bell arch, and the cellars are two roofed corridors to the cellar mouth.',
    shots: [],
  },
];

type Geo = (typeof geo.maps)[number] & { half?: number };
export type ShownMap = MapInfo & { geo: Geo; half: number };

/** The maps the site can show: those with geometry in maps.json, in the order above. */
export const shownMaps: ShownMap[] = maps.flatMap((m) => {
  const g = (geo.maps as Geo[]).find((x) => x.id === m.id);
  if (!g) return [];
  if (g.label !== m.name || g.blurb !== m.blurb)
    console.warn(`[site] ${m.id}: name or blurb differs from maps.json; update src/data/site.ts or run pnpm arena`);
  return [{ ...m, geo: g, half: g.half ?? (geo as { half?: number }).half ?? 32 }];
});

const WORDS = ['no', 'one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight'];
export const countWord = (n: number) => WORDS[n] ?? String(n);
export const TOTAL_MAPS = 6; // MapId::ALL in crates/brain/src/arena.rs

export const squadPage = {
  title: 'How they hunt',
  summary:
    'The squad brain is a plain Rust crate called brain. It has no engine dependency, so the same code drives the game and a 2D trainer. This is what it does inside a round, and what it keeps between matches.',
  meta: [
    { k: 'Crate', v: 'brain' },
    { k: 'Language', v: 'Rust, no engine dependency' },
    { k: 'Runs in', v: 'the game and a 2D trainer sim' },
    { k: 'Squad', v: '3 to 6 callsigns, Alpha to Foxtrot' },
  ],
  heroAlt: 'The original Foundry yard, in an earlier build, under fire: a red tracer crosses a sand-coloured crate and the screen edges glow red with damage.',
  sections: {
    belief: {
      title: 'One shared guess',
      paras: [
        'The squad coordinator keeps a belief grid: 24 by 24 cells, each 4 metres across, enough to cover the biggest map (Township, 90 metres). It holds how likely you are to be in each cell.',
        'A sighting collapses the grid onto where you were seen. After that, every second you stay hidden spreads the guess outward along walkable ground. Cells are cleared only when a member looks at them and even a crouched player there would have been seen, so hiding behind low cover still counts.',
      ],
    },
    bandit: {
      title: 'Choosing a play',
      paras: [
        'Once it knows where you are, the squad tries plays, keeps the ones that beat you, and still tests the ones it has not tried enough. Under the hood that is a disjoint LinUCB contextual bandit: each play keeps a small ridge regression, plus an optimism bonus for plays with little data.',
        'The reward for a fight window is damage dealt plus pushing you back, minus members lost without a trade. Between matches each model is pulled 15 percent back toward its prior, so it keeps up when you change how you play.',
      ],
      arms: ['Crossfire', 'Pin and flank', 'Trade', 'Bait'],
      context: ['rush', 'zone focus', 're-peek rate', 'strafe bias', 'his range', 'squad alive', 'squad hp', 'his hp', 'his accuracy'],
    },
    plays: {
      title: 'The playbook',
      lead: 'Every play the coordinator can run. The four marked ones are the bandit’s arms.',
      list: [
        { name: 'Free', note: 'No coordination. Each member uses its own utility AI.' },
        { name: 'Bounding overwatch', note: 'One team holds the angle while the other moves.' },
        { name: 'Crossfire', note: 'The two closest members spread more than 60° apart around you, so you cannot face both. Radio: “Spread out - crossfire”.', arm: true },
        { name: 'Trade', note: 'One pushes in, a second follows about 3 m behind on the same path to trade the kill, often behind a flashbang. Radio: “Going in - trade me”.', arm: true },
        { name: 'Pin and flank', note: 'The rest keep you pinned while the most flank-minded member circles to the side the squad watches least. Radio: “Pinning him - go”.', arm: true },
        { name: 'Collapse', note: 'Heard shots somewhere: everyone converges on it.' },
        { name: 'Bait', note: 'The healthiest member steps out at your weakest range while the others hold the spot you must stand in to shoot. Radio: “I’ll draw him out”.', arm: true },
        { name: 'Sweep', note: 'You are lost and far away: converge on the hot spot by different routes.' },
        { name: 'Search', note: 'You are unknown: split over the search spots, at most two on the hot one.' },
        { name: 'Encircle', note: 'You are known: the spotter keeps shooting, the rest take your exits from distinct sides.' },
      ],
    },
    search: {
      title: 'Search, split',
      paras: [
        'Search picks spots from the belief grid and pushes members apart so they do not stack on one hotspot. At most two investigate the hottest cell; the rest check other likely places, each calling the zone they take.',
      ],
      calls: ['Lost him - spreading out', 'Checking the copy room', 'Checking his last position'],
    },
    encircle: {
      title: 'Encircle, report and surround',
      paras: [
        'The member who sees you calls the contact with the zone and the side, then keeps firing. The others are assigned sectors around you and routed through different corridors, so they do not walk in single file. When everyone is a step from their line of fire, or the stage takes too long, one call sends them all at once.',
      ],
      calls: ['He’s at the server room, left side', 'Flanking north-west', 'Cutting him off - south', 'All set - go go go'],
    },
    alone: {
      title: 'On their own',
      paras: [
        'A member without orders shoots on sight. From a post it peeks, shoots and hides again. Members lean around corners, throw grenades and flashbangs, and their radio calls are staggered so they do not all talk at once.',
      ],
      calls: ['Reloading', 'Taking fire', 'He’s reloading - pushing', 'Man down - Bravo is down'],
    },
    between: {
      title: 'Between matches',
      paras: [
        'Each enemy carries a trait genome evolved with a genetic algorithm. Only technique evolves: aggression, preferred range, flank bias, cover affinity, burst length, retreat health, team cohesion and prediction trust, plus which guns it buys and which counters it reaches for.',
        'NEAT networks evolve combat moves. A player model keeps a decayed profile of you: a 48 by 48 heat grid (2 metre cells) of where you fight, your ranges and your habits. A 2D trainer runs simulated rounds against a ghost built from that model, in small steps while you are in the menus, and the HUD shows the squad generation going up.',
        'They get smarter, never faster. Reaction time, aim error and damage per hit are fixed limits: a generation-20 squad aims exactly like a generation-1 squad. Only the difficulty you pick scales them. Everything the squad gains between matches is in how it plays you.',
      ],
    },
  },
} as const;

export const notFound = {
  title: 'Lost contact',
  text: 'Nothing at this address. The squad is spreading out to look for it.',
  call: 'Lost him - spreading out',
};

export const gallery = [
  { src: 'main_menu', alt: `The main menu from an earlier build, under the codename Shroud: the title, the line They learn from you, Play, Settings and Quit, and an Enemy brain panel reading Gen 1 and 2268 sim rounds.`, caption: 'Main menu, earlier build. The enemy brain panel counts the trainer’s simulated rounds.' },
  { src: 'match_setup', alt: 'Match setup screen with four difficulty cards, Recruit to Elite, and three map cards, Foundry, Depot and Office (since renamed Ledger).', caption: 'Match setup: difficulty and map. An earlier build with three maps.' },
  { src: 'settings_gameplay', alt: 'Settings, Gameplay tab: difficulty, squad callouts, crosshair colour and size, a crosshair preview and an About this setting panel.', caption: 'Every setting explains itself, with a live crosshair preview.' },
  { src: 'pause', alt: 'Pause menu over the frozen match with the score line, Resume, Settings, Restart match, Main menu and Quit game.', caption: 'Esc pauses with the score line.' },
  { src: 'metrics_window', alt: 'Dev metrics window with frame time, CPU, GPU and memory graphs and a table of per-run stats.', caption: 'The dev metrics window, from the shared engine.' },
] as const;

// All visible copy for the Shroud site lives here.
//
// Facts come from the Shroud repo (crates/brain, crates/game, README) and the squad AI rework
// benchmarks. Map geometry comes from `src/data/maps.json`, exported from crates/brain/src/arena.rs
// by tools/arena-dump.
//
// Assumptions and unknowns (every one is `placeholder: true` below and listed in the hand-off):
// - release date, platforms, price, store/download link, studio name, repo URL, production domain.
// - "Marl Games" / marlgames.com is only a candidate studio name and domain: not shown as fact.

export type Placeholder = { label: string; value: string; placeholder: true; href?: string };

export const site = {
  name: 'Shroud',
  // Production URL: canonical, OG and sitemap read it. PLACEHOLDER: marlgames.com is only a
  // candidate studio domain; replace before deploying.
  url: 'https://shroud.marlgames.com',
  urlPlaceholder: true,
  title: 'Shroud: the squad learns from you',
  description:
    'Shroud is a first-person shooter written in Rust where one enemy squad hunts you round after round and learns from every match you play.',
  tagline: 'They learn from you.',
  kind: 'Adaptive squad shooter',
} as const;

export const nav = [
  { href: '/squad/', label: 'How they hunt' },
  { href: '/maps/', label: 'Maps' },
  { href: '/#get', label: 'Status' },
] as const;

// What is known today, shown in the finale.
export const facts = [
  { label: 'Status', value: 'In development' },
  { label: 'Genre', value: 'First-person shooter, solo against a squad of 3 to 6' },
  { label: 'Maps', value: 'Foundry, Depot, Office' },
  { label: 'Difficulty', value: 'Recruit, Regular, Veteran, Elite' },
  { label: 'Built with', value: 'Rust, Bevy 0.19, avian3d' },
] as const;

// Unknown: shown as one honest line, every item flagged for the owner to replace.
export const release: Placeholder[] = [
  { label: 'Release', value: 'Date to be announced', placeholder: true },
  { label: 'Platforms', value: 'To be announced', placeholder: true },
  { label: 'Price', value: 'To be announced', placeholder: true },
  { label: 'Store page', value: 'Link to come', placeholder: true },
  { label: 'Studio', value: 'To be announced', placeholder: true },
  { label: 'Source', value: 'Repository link to come', placeholder: true },
];

export const home = {
  heroLine: 'A first-person shooter where the enemy squad learns from your past matches.',
  heroHint: 'Live on Foundry: point at or tap the map. That is you.',
  heroCta: { href: '/squad/', label: 'See how they hunt' },
  statement:
    'One of you against one squad of three to six. Rounds, first to the target score. When the match ends the squad trains against a ghost of you, and the next squad is bred from the ones that did best against you.',
  statementNote: 'Foundry, Depot and Office rotate every match.',
  chapters: [
    {
      id: 'belief',
      title: 'Where you probably are',
      text: 'The squad shares one picture of the map: a 16 by 16 grid of where you might be. While nobody sees you, that guess spreads a little every second. A cell only goes cold when someone looks at it and a crouched player there would have been spotted.',
      call: 'Lost him - last seen at the south yard',
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
      note: 'A scripted ghost player that only tries to stay alive. Its win rate, per map.',
      rows: [
        { map: 'Foundry', before: 0.17, after: 0.06 },
        { map: 'Depot', before: 0.23, after: 0.15 },
        { map: 'Office', before: 0.25, after: 0.11 },
      ],
    },
  },
  difficulty: {
    title: 'Pick how hard they hunt',
    lead: 'Reaction, aim, damage, coordination, grenade use and how many attack at once all scale with it.',
  },
  build: {
    title: 'Rust from the squad brain up',
    text: 'The game runs on Bevy 0.19 with avian3d physics. The squad brain is a separate Rust crate with no engine dependency, so the same code runs in the game and in a 2D trainer that plays rounds against your ghost while you sit in the menus.',
    engine:
      'Window, settings, menus, pause, UI kit, FPS controller, render budget, devtools and the metrics window come from a shared in-house engine.',
    port: { before: '13,467', after: '7,833', label: 'lines of Shroud game code before and after the move onto the engine' },
  },
  get: {
    title: 'They are watching.',
    note: 'Before your first match, that is all the main menu tells you.',
    pending: 'Release date, platforms, price, store page and studio name are still to be announced.',
  },
} as const;

export const difficulties = [
  { name: 'Recruit', squad: 3, blurb: '3 enemies. Slow reactions, loose aim, no grenades.' },
  { name: 'Regular', squad: 4, blurb: '4 enemies. Fair fights; the squad coordinates.' },
  { name: 'Veteran', squad: 5, blurb: '5 enemies. Sharp aim, flanks and flashbangs.' },
  { name: 'Elite', squad: 6, blurb: "6 enemies. Every trick they've learned, used against you." },
] as const;

export type MapInfo = {
  id: 'foundry' | 'depot' | 'office';
  name: string;
  blurb: string; // the in-game blurb, verbatim
  about: string;
  cover: string; // key in src/assets/shots
  coverAlt: string;
  shots: { src: string; alt: string; caption: string }[];
  compare?: { low: string; high: string; alt: string; caption: string }[];
};

export const maps: MapInfo[] = [
  {
    id: 'foundry',
    name: 'Foundry',
    blurb: 'Open industrial yard around a central tower.',
    about:
      'The original arena and a mirror image of itself. Lane walls with a gap at the middle, crates near the tower, pillars in the yards and long low walls by the spawns. Open enough that the squad sees you early, which is when it pays to surround.',
    cover: 'enemies_foundry',
    coverAlt: 'First-person view across Foundry: two rust-green walls frame a long sand-coloured low wall, rifle in hand.',
    shots: [
      { src: 'player_firing', alt: 'The player fires on Foundry with a red damage vignette around the screen and an enemy at the right wall.', caption: 'Firing back at the right flank, health at 93.' },
      { src: 'hud_killfeed', alt: 'Foundry HUD with the radio line: Delta, he is at the south yard, left side.', caption: 'A contact call names the zone and the side.' },
    ],
  },
  {
    id: 'depot',
    name: 'Depot',
    blurb: 'Three lanes, long sightlines, one contested middle.',
    about:
      'Three lanes between sun-baked buildings. Long A on the west with a door choke, a contested mid with double doors, roofed B tunnels on the east and two crate sites at the north end. You spawn south; the squad holds the north.',
    cover: 'depot_long_a',
    coverAlt: 'Depot’s long A lane: sand underfoot between a wooden fence and stucco buildings, a crate stack halfway down.',
    shots: [
      { src: 'depot_b_tunnels', alt: 'Inside Depot’s roofed B tunnels: a narrow plaster corridor lit by one ceiling lamp, crates at the entrance.', caption: 'The roofed B tunnels on the east side: close range, one lamp.' },
      { src: 'round_over', alt: 'Depot round over screen reading Round won, one to zero, in front of a red shipping container.', caption: 'Round won, one to nil. The next round starts on its own.' },
    ],
  },
  {
    id: 'office',
    name: 'Office',
    blurb: 'Tight corridors and rooms. Every corner is a fight.',
    about:
      'One roofed floor of rooms and corridors: offices on both flanks off two long corridors, open-plan cubicles around a conference room, a break room north and reception south. Doorways everywhere, so any room can be flanked.',
    cover: 'office_server_room',
    coverAlt: 'Office server room: a row of dark racks with green status lights under a grid ceiling.',
    shots: [
      { src: 'office_west_corridor', alt: 'Office west corridor: a long bright hallway with doorways on both sides and ceiling lights running to the far end.', caption: 'The west corridor: doorways on both sides, nowhere to hide.' },
      { src: 'enemies_office', alt: 'A dark conference table in front of a long wall with two doorways under a ceiling grid.', caption: 'Two doorways, one table: any room in Office can be flanked.' },
    ],
    compare: [
      { low: 'low_office_cubicles', high: 'high_office_cubicles', alt: 'The Office cubicles', caption: 'Cubicles at Low and High quality.' },
      { low: 'low_office_lobby_long', high: 'high_office_lobby_long', alt: 'The long view down the Office lobby', caption: 'The lobby at Low and High quality.' },
    ],
  },
];

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
  heroAlt: 'Foundry under fire: a red tracer crosses a sand-coloured crate and the screen edges glow red with damage.',
  sections: {
    belief: {
      title: 'One shared guess',
      paras: [
        'The squad coordinator keeps a belief grid: 16 by 16 cells over the 64 metre arena, so each cell is 4 metres across. It holds how likely you are to be in each cell.',
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
      calls: ['He’s at the center tower, left side', 'Flanking north-west', 'Cutting him off - south', 'All set - go go go'],
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
        'Each enemy carries a trait genome of ten genes, evolved with a genetic algorithm: aggression, preferred range, flank bias, cover affinity, reaction time, aim error, burst length, retreat health, team cohesion and prediction trust.',
        'NEAT networks evolve combat moves. A player model keeps a decayed profile of you: a 32 by 32 heat grid of where you fight, your ranges and your habits. A 2D trainer runs simulated rounds against a ghost built from that model, in small steps while you are in the menus, and the HUD shows the squad generation going up.',
        'Fairness floors keep it honest. Minimum reaction time starts at 0.45 seconds and never goes below 0.18; minimum aim error starts at 5 degrees and never goes below 1.2. They loosen a little each generation, so the squad gets smarter, not inhuman.',
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
  { src: 'main_menu', alt: 'Shroud main menu: the title, the line They learn from you, Play, Settings and Quit, and an Enemy brain panel reading Gen 1 and 2308 sim rounds.', caption: 'Main menu. The enemy brain panel counts the trainer’s simulated rounds.' },
  { src: 'match_setup', alt: 'Match setup screen with four difficulty cards, Recruit to Elite, and three map cards, Foundry, Depot and Office.', caption: 'Match setup: difficulty and map.' },
  { src: 'settings_gameplay', alt: 'Settings, Gameplay tab: difficulty, squad callouts, crosshair colour and size, a crosshair preview and an About this setting panel.', caption: 'Every setting explains itself, with a live crosshair preview.' },
  { src: 'pause', alt: 'Pause menu over the frozen match with the score line, Resume, Settings, Restart match, Main menu and Quit game.', caption: 'Esc pauses with the score line.' },
  { src: 'metrics_window', alt: 'Dev metrics window with frame time, CPU, GPU and memory graphs and a table of per-run stats.', caption: 'The dev metrics window, from the shared engine.' },
] as const;

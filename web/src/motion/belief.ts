// The site's signature: a small browser version of the squad coordinator's belief grid,
// running on the real map plans from crates/brain (src/data/maps.json).
//
// It follows the coordinator's rules in simplified form:
// - a 16x16 belief grid over the 64 m arena; diffusion uses the same formula as Belief::diffuse
// - cells clear only where a member looks and a crouched player would be seen (low cover blocks)
// - Search: spots picked from the grid with repulsion, at most two members on the hottest one
// - Encircle: the spotter holds, the others take distinct sectors, stage, then move on "go"
// - radio calls use the game's own text formats (contact_text, "Checking {zone}", ...)
// The pointer is the player. With no pointer, a ghost walks the map so the scene keeps living.
// Everything runs on gsap.ticker (one clock) and pauses off screen.
import { gsap } from 'gsap';
import data from '../data/maps.json';

type V = { x: number; y: number };
type Block = { x: number; y: number; hx: number; hy: number; h: number; kind: string; skin: string };
type Zone = { name: string; x0: number; y0: number; x1: number; y1: number };
type MapData = { id: string; label: string; blocks: Block[]; zones: Zone[]; playerSpawn: number[]; enemySpawns: number[][] };

const HALF = data.half; // 32 m
const SIZE = HALF * 2;
const BW = 16; // belief cells per side (BELIEF_W)
const BCELL = SIZE / BW; // 4 m
const NW = 64; // nav cells per side (1 m)
const CALLSIGNS = ['Alpha', 'Bravo', 'Charlie', 'Delta', 'Echo', 'Foxtrot'];
const SECTORS = ['east', 'south-east', 'south', 'south-west', 'west', 'north-west', 'north', 'north-east'];

const C = {
  heat: [242, 163, 58] as const,
  squad: '#ee5a4f',
  you: '#55d6a8',
  tall: '#323a43',
  tallEdge: 'rgba(236,235,231,0.3)',
  low: 'rgba(236,235,231,0.3)',
  grid: 'rgba(236,235,231,0.07)',
  label: 'rgba(153,160,168,0.9)',
  zone: 'rgba(236,235,231,0.42)',
};

const dist = (a: V, b: V) => Math.hypot(a.x - b.x, a.y - b.y);
const angDiff = (a: number, b: number) => Math.atan2(Math.sin(a - b), Math.cos(a - b));

/** A seeded PRNG so the reduced-motion still frame and the first frame are stable. */
function rng(seed: number) {
  let s = seed >>> 0;
  return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
}

// ---------------------------------------------------------------- world (map plan + nav + LOS)
class World {
  map: MapData;
  sight: Block[]; // blocks that stop a standing player's sight (walls, tall)
  crouch: Block[]; // plus low cover: what stops sight of a crouched player
  walk = new Uint8Array(NW * NW);
  beliefWalk: boolean[] = [];

  constructor(map: MapData) {
    this.map = map;
    this.sight = map.blocks.filter((b) => b.kind !== 'Low');
    this.crouch = map.blocks;
    for (let j = 0; j < NW; j++)
      for (let i = 0; i < NW; i++) {
        const p = { x: -HALF + i + 0.5, y: -HALF + j + 0.5 };
        this.walk[j * NW + i] = this.inside(p, 0.45) ? 0 : 1;
      }
    for (let j = 0; j < BW; j++)
      for (let i = 0; i < BW; i++) {
        let ok = false;
        for (let a = 0; a < 4 && !ok; a++)
          for (let b = 0; b < 4 && !ok; b++) ok = this.walkableAt({ x: -HALF + i * BCELL + a + 0.5, y: -HALF + j * BCELL + b + 0.5 });
        this.beliefWalk.push(ok);
      }
  }

  inside(p: V, pad = 0) {
    if (Math.abs(p.x) > HALF - 1 - pad || Math.abs(p.y) > HALF - 1 - pad) return true;
    return this.map.blocks.some((b) => b.kind !== 'Wall' && Math.abs(p.x - b.x) < b.hx + pad && Math.abs(p.y - b.y) < b.hy + pad);
  }
  navIndex(p: V) {
    const i = Math.max(0, Math.min(NW - 1, Math.floor(p.x + HALF)));
    const j = Math.max(0, Math.min(NW - 1, Math.floor(p.y + HALF)));
    return j * NW + i;
  }
  walkableAt(p: V) { return this.walk[this.navIndex(p)] === 1; }
  nearestWalkable(p: V): V {
    const start = this.navIndex(p);
    if (this.walk[start]) return p;
    for (let r = 1; r < 12; r++)
      for (let dj = -r; dj <= r; dj++)
        for (let di = -r; di <= r; di++) {
          if (Math.max(Math.abs(di), Math.abs(dj)) !== r) continue;
          const i = (start % NW) + di, j = Math.floor(start / NW) + dj;
          if (i < 0 || j < 0 || i >= NW || j >= NW) continue;
          if (this.walk[j * NW + i]) return { x: -HALF + i + 0.5, y: -HALF + j + 0.5 };
        }
    return p;
  }
  /** Segment vs axis-aligned boxes (slab test). */
  clear(a: V, b: V, blocks: Block[]) {
    const dx = b.x - a.x, dy = b.y - a.y;
    for (const k of blocks) {
      let t0 = 0, t1 = 1;
      const slabs: [number, number, number][] = [[a.x, dx, k.x], [a.y, dy, k.y]];
      let hit = true;
      for (let s = 0; s < 2; s++) {
        const [o, d, c] = slabs[s];
        const h = s === 0 ? k.hx : k.hy;
        if (Math.abs(d) < 1e-9) { if (o < c - h || o > c + h) { hit = false; break; } continue; }
        let u0 = (c - h - o) / d, u1 = (c + h - o) / d;
        if (u0 > u1) [u0, u1] = [u1, u0];
        t0 = Math.max(t0, u0); t1 = Math.min(t1, u1);
        if (t0 > t1) { hit = false; break; }
      }
      if (hit) return false;
    }
    return true;
  }
  zoneOf(p: V) { return this.map.zones.find((z) => p.x >= z.x0 && p.x <= z.x1 && p.y >= z.y0 && p.y <= z.y1); }
  /** A* on the 1 m nav grid, 8-connected without corner cutting, then string-pulled. */
  path(from: V, to: V): V[] {
    const s = this.navIndex(from), g = this.navIndex(this.nearestWalkable(to));
    if (s === g) return [to];
    const N = NW * NW;
    const cost = new Float32Array(N).fill(Infinity);
    const prev = new Int32Array(N).fill(-1);
    const open: number[] = [s];
    const gx = g % NW, gy = Math.floor(g / NW);
    const hcost = (i: number) => Math.hypot((i % NW) - gx, Math.floor(i / NW) - gy);
    const f = new Float32Array(N).fill(Infinity);
    cost[s] = 0; f[s] = hcost(s);
    const closed = new Uint8Array(N);
    while (open.length) {
      let bi = 0;
      for (let k = 1; k < open.length; k++) if (f[open[k]] < f[open[bi]]) bi = k;
      const cur = open[bi];
      open[bi] = open[open.length - 1]; open.pop();
      if (cur === g) break;
      if (closed[cur]) continue;
      closed[cur] = 1;
      const cx = cur % NW, cy = Math.floor(cur / NW);
      for (let dy = -1; dy <= 1; dy++)
        for (let dx = -1; dx <= 1; dx++) {
          if (!dx && !dy) continue;
          const nx = cx + dx, ny = cy + dy;
          if (nx < 0 || ny < 0 || nx >= NW || ny >= NW) continue;
          const n = ny * NW + nx;
          if (!this.walk[n] || closed[n]) continue;
          if (dx && dy && (!this.walk[cy * NW + nx] || !this.walk[ny * NW + cx])) continue;
          const c = cost[cur] + (dx && dy ? 1.4142 : 1);
          if (c < cost[n]) { cost[n] = c; prev[n] = cur; f[n] = c + hcost(n); open.push(n); }
        }
    }
    if (prev[g] === -1) return [this.nearestWalkable(to)];
    const cells: V[] = [];
    for (let c = g; c !== -1 && c !== s; c = prev[c]) cells.push({ x: -HALF + (c % NW) + 0.5, y: -HALF + Math.floor(c / NW) + 0.5 });
    cells.reverse();
    // String pulling: skip waypoints while the straight line stays walkable.
    const out: V[] = [];
    let anchor = from;
    for (let k = 0; k < cells.length; k++) {
      const next = cells[k + 1];
      if (next && this.walkLine(anchor, next)) continue;
      out.push(cells[k]);
      anchor = cells[k];
    }
    return out.length ? out : [this.nearestWalkable(to)];
  }
  walkLine(a: V, b: V) {
    const n = Math.ceil(dist(a, b) / 0.5);
    for (let k = 1; k <= n; k++) {
      const t = k / n;
      if (!this.walkableAt({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t })) return false;
    }
    return true;
  }
}

// ---------------------------------------------------------------- the squad
type Member = { name: string; pos: V; face: number; path: V[]; goal: V | null; arrived: boolean; role: 'search' | 'spotter' | 'flank' | 'hold'; scan: number };

export type Mode = 'live' | 'belief' | 'search' | 'encircle' | 'cold';
export type Bark = (who: string, text: string) => void;

const RANGE = 30;
const FOV = (110 * Math.PI) / 180;
const SPEED = 5.2;
const GHOST_SPEED = 3.4;

class Squad {
  w: World;
  b = new Float32Array(BW * BW);
  m: Member[] = [];
  mode: Mode;
  play: 'search' | 'encircle' = 'search';
  lastSeen = -99;
  lastPos: V = { x: 0, y: 20 };
  spotter = -1;
  stageT = 0;
  go = false;
  t = 0;
  replan = 0;
  clearT = 0;
  rand: () => number;
  bark: Bark;
  barkQueue: [string, string][] = [];
  barkT = 0;
  lastBark = '';

  constructor(w: World, mode: Mode, size: number, seed: number, bark: Bark) {
    this.w = w; this.mode = mode; this.rand = rng(seed); this.bark = bark;
    const spawns = w.map.enemySpawns;
    for (let i = 0; i < size; i++) {
      const s = spawns[i % spawns.length];
      this.m.push({ name: CALLSIGNS[i], pos: w.nearestWalkable({ x: s[0], y: s[1] }), face: Math.PI / 2, path: [], goal: null, arrived: true, role: 'search', scan: this.rand() * 6 });
    }
    this.resetBelief();
  }

  resetBelief() {
    let n = 0;
    this.w.beliefWalk.forEach((ok) => ok && n++);
    this.w.beliefWalk.forEach((ok, i) => (this.b[i] = ok ? 1 / n : 0));
  }
  normalize() {
    let s = 0;
    for (let i = 0; i < this.b.length; i++) s += this.b[i];
    if (s < 1e-6) return this.resetBelief();
    for (let i = 0; i < this.b.length; i++) this.b[i] /= s;
  }
  /** Belief::diffuse, same formula: a = (secs * 4 / cell * 0.5).min(0.8). */
  diffuse(secs: number) {
    const a = Math.min(0.8, (secs * 4) / BCELL * 0.5);
    const old = this.b.slice();
    for (let i = 0; i < old.length; i++) {
      const x = i % BW, z = Math.floor(i / BW);
      let s = 0, n = 0;
      for (const [dx, dz] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
        const nx = x + dx, nz = z + dz;
        if (nx >= 0 && nz >= 0 && nx < BW && nz < BW && this.w.beliefWalk[nz * BW + nx]) { s += old[nz * BW + nx]; n++; }
      }
      if (n > 0) this.b[i] = (1 - a) * old[i] + (a * s) / n;
    }
    this.normalize();
  }
  cellCenter(i: number): V { return { x: -HALF + ((i % BW) + 0.5) * BCELL, y: -HALF + (Math.floor(i / BW) + 0.5) * BCELL }; }
  cellOf(p: V) {
    const c = (v: number) => Math.max(0, Math.min(BW - 1, Math.floor((v + HALF) / BCELL)));
    return c(p.y) * BW + c(p.x);
  }
  sees(m: Member, p: V, blocks: Block[]) {
    const d = dist(m.pos, p);
    if (d > RANGE) return false;
    if (d > 4 && Math.abs(angDiff(Math.atan2(p.y - m.pos.y, p.x - m.pos.x), m.face)) > FOV / 2) return false;
    return this.w.clear(m.pos, p, blocks);
  }
  say(who: string, text: string, urgent = false) {
    if (text === this.lastBark) return;
    this.lastBark = text;
    if (urgent) this.barkQueue.unshift([who, text]);
    else this.barkQueue.push([who, text]);
    if (this.barkQueue.length > 4) this.barkQueue.length = 4;
  }
  contactText(speaker: V, target: V) {
    const z = this.w.zoneOf(target);
    if (!z) return 'Contact';
    const c = { x: (z.x0 + z.x1) / 2, y: (z.y0 + z.y1) / 2 };
    const cross = (c.x - speaker.x) * (target.y - c.y) - (c.y - speaker.y) * (target.x - c.x);
    return `He’s at the ${z.name.replace(/^the /, '')}, ${cross > 0 ? 'right' : 'left'} side`;
  }
  sectorName(angle: number) {
    const s = Math.round((((angle % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI)) / (Math.PI / 4)) % 8;
    return SECTORS[s];
  }
  send(m: Member, goal: V) {
    const g = this.w.nearestWalkable(goal);
    if (m.goal && dist(m.goal, g) < 2) return;
    m.goal = g;
    m.path = this.w.path(m.pos, g);
    m.arrived = false;
  }

  /** One fixed step. `player` is null when nobody is on the map. */
  step(dt: number, player: V | null) {
    this.t += dt;
    const reveal = this.mode !== 'cold' && this.mode !== 'search' && player;
    // 1. Sight
    let seenBy = -1;
    if (reveal) {
      for (let i = 0; i < this.m.length; i++) if (this.sees(this.m[i], player, this.w.sight)) { seenBy = i; break; }
    }
    if (seenBy >= 0 && player) {
      const fresh = this.t - this.lastSeen > 2.5;
      this.lastSeen = this.t;
      this.lastPos = { ...player };
      this.b.fill(0);
      this.b[this.cellOf(player)] = 1;
      if (fresh) {
        this.spotter = seenBy;
        this.say(this.m[seenBy].name, this.contactText(this.m[seenBy].pos, player), true);
      }
    }
    // 2. Belief: spread while hidden, clear what members can see (5 Hz).
    this.clearT += dt;
    if (this.clearT >= 0.2) {
      const secs = this.clearT;
      this.clearT = 0;
      if (seenBy < 0) {
        this.diffuse(secs);
        for (const m of this.m)
          for (let i = 0; i < this.b.length; i++) {
            if (this.b[i] < 1e-5) continue;
            const c = this.cellCenter(i);
            if (this.sees(m, c, this.w.crouch)) this.b[i] *= 0.15;
          }
        this.normalize();
      }
    }
    // 3. Plan (once a second, like REPLAN_S).
    this.replan -= dt;
    const known = this.t - this.lastSeen < 3 && this.mode !== 'search' && this.mode !== 'cold';
    if (this.mode === 'belief') {
      // Chapter one: the squad holds its posts and only watches.
      this.m.forEach((m) => (m.role = 'hold'));
    } else if (known) {
      if (this.play !== 'encircle') { this.play = 'encircle'; this.stageT = 0; this.go = false; this.replan = 0; }
      this.stageT += dt;
      if (this.replan <= 0) this.planEncircle();
    } else {
      if (this.play === 'encircle') {
        this.play = 'search';
        this.say(this.m[Math.max(0, this.spotter)].name, 'Lost him - spreading out');
        this.m.forEach((m) => (m.goal = null));
        this.replan = 0;
      }
      if (this.replan <= 0) this.planSearch();
    }
    if (this.replan <= 0) this.replan = 1;
    // 4. Move and look.
    for (const m of this.m) this.move(m, dt, player);
    // 5. Radio: staggered, one call at a time.
    this.barkT -= dt;
    if (this.barkT <= 0 && this.barkQueue.length) {
      const [who, text] = this.barkQueue.shift()!;
      this.bark(who, text);
      this.barkT = 1.1;
    }
  }

  planSearch() {
    const order = [...this.b.keys()].filter((i) => this.w.beliefWalk[i]).sort((a, b) => this.b[b] - this.b[a]);
    const spots: V[] = [];
    for (const i of order) {
      const c = this.cellCenter(i);
      if (spots.every((s) => dist(s, c) > 14)) spots.push(c); // repulsion between search spots
      if (spots.length >= this.m.length) break;
    }
    const free = new Set(this.m.keys());
    const assign = (spot: V, n: number) => {
      const near = [...free].sort((a, b) => dist(this.m[a].pos, spot) - dist(this.m[b].pos, spot)).slice(0, n);
      near.forEach((i) => { free.delete(i); const m = this.m[i]; m.role = 'search';
        const jitter = { x: spot.x + (this.rand() - 0.5) * 3, y: spot.y + (this.rand() - 0.5) * 3 };
        const was = m.goal; this.send(m, jitter);
        if (m.goal !== was && was && this.rand() < 0.5) { const z = this.w.zoneOf(spot); if (z) this.say(m.name, `Checking ${z.name}`); }
      });
    };
    if (spots[0]) assign(spots[0], Math.min(2, this.m.length)); // at most two on the hot spot
    for (const s of spots.slice(1)) { if (!free.size) break; assign(s, 1); }
  }

  planEncircle() {
    const p = this.lastPos;
    const sp = this.m[this.spotter] ?? this.m[0];
    sp.role = 'spotter';
    sp.goal = null; sp.path = [];
    const base = Math.atan2(sp.pos.y - p.y, sp.pos.x - p.x);
    const movers = this.m.filter((m) => m !== sp);
    const n = movers.length;
    const R = this.go ? 3.5 : 11;
    const bearings = movers.map((_, k) => base + ((k + 1) * 2 * Math.PI) / (n + 1));
    // Greedy: each sector to its nearest free member (distinct sides, distinct routes).
    const taken = new Set<number>();
    for (const b of bearings) {
      let best = -1, bd = Infinity;
      const goal = { x: p.x + Math.cos(b) * R, y: p.y + Math.sin(b) * R };
      movers.forEach((m, k) => { if (!taken.has(k) && dist(m.pos, goal) < bd) { bd = dist(m.pos, goal); best = k; } });
      taken.add(best);
      const m = movers[best];
      const fresh = m.role !== 'flank';
      m.role = 'flank';
      this.send(m, goal);
      if (fresh) this.say(m.name, taken.size === 1 ? `Flanking ${this.sectorName(b)}` : `Cutting him off - ${this.sectorName(b)}`);
    }
    const staged = movers.every((m) => m.arrived);
    if (!this.go && (staged || this.stageT > 7)) {
      this.go = true;
      this.say(sp.name, 'All set - go go go', true);
      this.m.forEach((m) => (m.goal = null));
      this.planEncircle();
    }
  }

  move(m: Member, dt: number, player: V | null) {
    let moving = false;
    if (m.role !== 'hold' && m.role !== 'spotter' && m.path.length) {
      const wp = m.path[0];
      const d = dist(m.pos, wp);
      const stepLen = SPEED * dt;
      if (d <= stepLen) { m.pos = { ...wp }; m.path.shift(); if (!m.path.length) m.arrived = true; }
      else {
        const a = Math.atan2(wp.y - m.pos.y, wp.x - m.pos.x);
        m.pos = { x: m.pos.x + Math.cos(a) * stepLen, y: m.pos.y + Math.sin(a) * stepLen };
        m.face += angDiff(a, m.face) * Math.min(1, dt * 6);
        moving = true;
      }
    }
    if (!moving) {
      // Look toward the target when one is known, otherwise sweep the view slowly.
      const look = this.play === 'encircle' && this.mode !== 'belief' ? Math.atan2(this.lastPos.y - m.pos.y, this.lastPos.x - m.pos.x) : null;
      m.scan += dt;
      const want = look ?? m.face + Math.sin(m.scan * 0.9) * 0.9 * dt * 2;
      m.face += angDiff(want, m.face) * Math.min(1, dt * (look === null ? 1 : 4));
    }
    void player;
  }

  peak() {
    let bi = 0;
    for (let i = 1; i < this.b.length; i++) if (this.b[i] > this.b[bi]) bi = i;
    return { i: bi, v: this.b[bi] };
  }
}

// ---------------------------------------------------------------- the ghost (no pointer)
class Ghost {
  pos: V; path: V[] = []; wait = 0; rand: () => number; w: World;
  constructor(w: World, seed: number) {
    this.w = w; this.rand = rng(seed * 7 + 3);
    const s = w.map.playerSpawn;
    this.pos = w.nearestWalkable({ x: s[0], y: s[1] - 2 });
  }
  step(dt: number) {
    if (this.wait > 0) { this.wait -= dt; return; }
    if (!this.path.length) {
      const goal = { x: (this.rand() - 0.5) * (SIZE - 12), y: (this.rand() - 0.5) * (SIZE - 12) };
      this.path = this.w.path(this.pos, goal);
      this.wait = 0.8 + this.rand() * 1.6; // camps a moment at each spot
      return;
    }
    const wp = this.path[0];
    const d = dist(this.pos, wp), s = GHOST_SPEED * dt;
    if (d <= s) { this.pos = { ...wp }; this.path.shift(); }
    else { const a = Math.atan2(wp.y - this.pos.y, wp.x - this.pos.x); this.pos = { x: this.pos.x + Math.cos(a) * s, y: this.pos.y + Math.sin(a) * s }; }
  }
}

// ---------------------------------------------------------------- view
export type Options = {
  map?: string;
  mode?: Mode;
  squad?: number;
  seed?: number;
  zones?: boolean; // label the zones
  spawns?: boolean; // draw the 7 enemy spawns and the player spawn
  interactive?: boolean; // pointer or tap becomes the player
  onBark?: Bark;
  onState?: (s: { mode: string; peak: string; peakPct: number; seen: number; spread: number }) => void;
};

export function cellName(i: number) {
  return `${String.fromCharCode(65 + (i % BW))}${Math.floor(i / BW) + 1}`;
}

export class BeliefView {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  world: World;
  squad!: Squad;
  ghost!: Ghost;
  o: Required<Omit<Options, 'onBark' | 'onState'>> & Pick<Options, 'onBark' | 'onState'>;
  pointer: V | null = null;
  pointerT = -99;
  acc = 0;
  heat = new Float32Array(BW * BW); // displayed (smoothed) belief
  size = 0;
  dpr = 1;
  running = false;
  stateT = 0;
  off: (() => void)[] = [];

  constructor(canvas: HTMLCanvasElement, opts: Options = {}) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d')!;
    const map = (data.maps as MapData[]).find((m) => m.id === (opts.map || 'foundry')) || (data.maps[0] as MapData);
    this.world = new World(map);
    this.o = { map: map.id, mode: 'live', squad: 4, seed: 7, zones: false, spawns: false, interactive: true, ...opts };
    this.reset(this.o.mode);
    this.resize();
  }

  reset(mode: Mode) {
    this.o.mode = mode;
    this.squad = new Squad(this.world, mode, this.o.squad, this.o.seed, (who, text) => this.o.onBark?.(who, text));
    this.ghost = new Ghost(this.world, this.o.seed);
    if (mode === 'encircle') this.ghost.wait = 9999; // camps: the squad finds and surrounds it
    if (mode === 'belief') {
      // Start from a sighting at the south yard so the spread reads at once.
      const p = this.world.nearestWalkable({ x: 0, y: 20 });
      this.squad.b.fill(0); this.squad.b[this.squad.cellOf(p)] = 1;
      this.squad.lastPos = p;
    }
    if (mode === 'encircle') {
      this.ghost.pos = this.world.nearestWalkable({ x: 4, y: 12 });
    }
    if (mode === 'live') {
      // First frame: contact was lost near your spawn a moment ago; the guess is spreading
      // and the squad is on its way to search it.
      const p = this.ghost.pos;
      this.squad.b.fill(0); this.squad.b[this.squad.cellOf(p)] = 1;
      this.squad.lastPos = { ...p };
    }
    this.heat.set(this.squad.b);
  }

  /** Advance the simulation by `secs` without drawing (warm start / reduced-motion frame). */
  warm(secs: number) {
    for (let t = 0; t < secs; t += 1 / 30) this.tick(1 / 30, true);
    this.heat.set(this.squad.b);
  }

  resize() {
    const r = this.canvas.getBoundingClientRect();
    this.dpr = Math.min(devicePixelRatio || 1, 2);
    this.size = Math.max(1, Math.round(Math.min(r.width, r.height || r.width)));
    this.canvas.width = Math.round(r.width * this.dpr);
    this.canvas.height = Math.round((r.height || r.width) * this.dpr);
  }

  toWorld(clientX: number, clientY: number): V | null {
    const r = this.canvas.getBoundingClientRect();
    const s = Math.min(r.width, r.height);
    const ox = r.left + (r.width - s) / 2, oy = r.top + (r.height - s) / 2;
    const x = ((clientX - ox) / s) * SIZE - HALF, y = ((clientY - oy) / s) * SIZE - HALF;
    if (Math.abs(x) > HALF - 1 || Math.abs(y) > HALF - 1) return null;
    return { x, y };
  }

  setPointer(clientX: number, clientY: number) {
    if (!this.o.interactive) return;
    const p = this.toWorld(clientX, clientY);
    this.pointer = p;
    if (p) { this.pointerT = this.squad.t; this.ghost.pos = this.world.nearestWalkable(p); this.ghost.path = []; this.ghost.wait = 2; }
  }

  player(): V | null {
    if (this.o.mode === 'search' || this.o.mode === 'cold') return null;
    if (this.pointer && this.squad.t - this.pointerT < 6) return this.pointer;
    return this.ghost.pos;
  }

  tick(dt: number, silent = false) {
    this.acc += Math.min(dt, 0.1);
    while (this.acc >= 1 / 30) {
      this.acc -= 1 / 30;
      if (!this.pointer || this.squad.t - this.pointerT >= 6) this.ghost.step(1 / 30);
      this.squad.step(1 / 30, this.player());
      // Encircle chapter: once they have closed in, start the story again.
      if (this.o.mode === 'encircle' && this.squad.go && this.squad.m.every((m) => m.arrived || m.role === 'spotter') && this.squad.t - this.squad.lastSeen < 1) {
        this.loopT = (this.loopT || 0) + 1 / 30;
        if (this.loopT > 2.2) { this.loopT = 0; this.reset('encircle'); }
      }
    }
    if (silent) return;
    const k = 1 - Math.pow(0.001, dt); // displayed heat eases toward the belief
    for (let i = 0; i < this.heat.length; i++) this.heat[i] += (this.squad.b[i] - this.heat[i]) * k;
    this.stateT -= dt;
    if (this.stateT <= 0 && this.o.onState) {
      this.stateT = 0.25;
      const pk = this.squad.peak();
      const ms = this.squad.m;
      let spread = 0, n = 0;
      for (let a = 0; a < ms.length; a++) for (let b = a + 1; b < ms.length; b++) { spread += dist(ms[a].pos, ms[b].pos); n++; }
      this.o.onState({ mode: this.o.mode === 'belief' ? 'Watching' : this.squad.play === 'encircle' ? (this.squad.go ? 'Encircle: go' : 'Encircle: staging') : 'Search', peak: cellName(pk.i), peakPct: pk.v, seen: Math.max(0, this.squad.t - this.squad.lastSeen), spread: n ? spread / n : 0 });
    }
  }
  loopT = 0;

  draw() {
    const { ctx, canvas } = this;
    const W = canvas.width, H = canvas.height;
    const s = Math.min(W, H);
    const ox = (W - s) / 2, oy = (H - s) / 2;
    const k = s / SIZE; // px per metre
    const X = (x: number) => ox + (x + HALF) * k;
    const Y = (y: number) => oy + (y + HALF) * k;
    ctx.clearRect(0, 0, W, H);
    // Heat
    const n = this.world.beliefWalk.filter(Boolean).length;
    const cold = this.o.mode === 'cold' ? 0.55 : 1;
    const cell = BCELL * k;
    const gap = Math.max(1, this.dpr);
    for (let i = 0; i < this.heat.length; i++) {
      const v = Math.min(1, Math.sqrt(this.heat[i] * n) / 4);
      if (v < 0.05) continue;
      ctx.fillStyle = `rgba(${C.heat[0]},${C.heat[1]},${C.heat[2]},${(v * 0.88 * cold).toFixed(3)})`;
      ctx.fillRect(ox + (i % BW) * cell + gap / 2, oy + Math.floor(i / BW) * cell + gap / 2, cell - gap, cell - gap);
    }
    // Grid
    ctx.strokeStyle = C.grid;
    ctx.lineWidth = this.dpr;
    ctx.beginPath();
    for (let i = 0; i <= BW; i++) {
      ctx.moveTo(ox + i * cell, oy); ctx.lineTo(ox + i * cell, oy + s);
      ctx.moveTo(ox, oy + i * cell); ctx.lineTo(ox + s, oy + i * cell);
    }
    ctx.stroke();
    // Plan
    for (const b of this.world.map.blocks) {
      if (b.kind === 'Wall') continue;
      const x = X(b.x - b.hx), y = Y(b.y - b.hy), w = b.hx * 2 * k, h = b.hy * 2 * k;
      if (b.kind === 'Tall') {
        ctx.fillStyle = C.tall; ctx.fillRect(x, y, w, h);
        ctx.strokeStyle = C.tallEdge; ctx.lineWidth = this.dpr; ctx.strokeRect(x + 0.5, y + 0.5, w - 1, h - 1);
      } else {
        ctx.strokeStyle = C.low; ctx.lineWidth = this.dpr; ctx.strokeRect(x + 0.5, y + 0.5, w - 1, h - 1);
      }
    }
    ctx.strokeStyle = 'rgba(236,235,231,0.3)';
    ctx.lineWidth = this.dpr * 1.5;
    ctx.strokeRect(X(-HALF + 1), Y(-HALF + 1), (SIZE - 2) * k, (SIZE - 2) * k);
    // Peak cell outline
    const pk = this.squad.peak();
    ctx.strokeStyle = `rgba(${C.heat.join(',')},0.95)`;
    ctx.lineWidth = this.dpr * 1.5;
    ctx.strokeRect(ox + (pk.i % BW) * cell + gap, oy + Math.floor(pk.i / BW) * cell + gap, cell - gap * 2, cell - gap * 2);
    const font = (px: number, w = 600) => `${w} ${Math.round(px * this.dpr)}px Barlow, system-ui, sans-serif`;
    // Zones
    if (this.o.zones && s / this.dpr >= 480) {
      ctx.fillStyle = C.zone; ctx.font = font(11, 500); ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
      for (const z of this.world.map.zones) {
        if (z.name === 'the cubicles' || z.name === 'the south spawn' || z.name === 'the north spawn') continue;
        ctx.fillText(z.name.replace(/^the /, ''), X((z.x0 + z.x1) / 2), Y((z.y0 + z.y1) / 2));
      }
    }
    if (this.o.spawns) {
      ctx.textAlign = 'center'; ctx.textBaseline = 'middle'; ctx.font = font(10, 600);
      this.world.map.enemySpawns.forEach((p, i) => {
        const r = 7 * this.dpr;
        ctx.strokeStyle = C.squad; ctx.lineWidth = this.dpr * 1.5; ctx.strokeRect(X(p[0]) - r, Y(p[1]) - r, r * 2, r * 2);
        ctx.fillStyle = C.squad; ctx.fillText(String(i + 1), X(p[0]), Y(p[1]) + 0.5);
      });
      const ps = this.world.map.playerSpawn;
      ctx.fillStyle = C.you; ctx.beginPath(); ctx.arc(X(ps[0]), Y(ps[1]), 6 * this.dpr, 0, Math.PI * 2); ctx.fill();
    }
    // Squad: view cones, routes, chevrons
    for (const m of this.squad.m) {
      const px = X(m.pos.x), py = Y(m.pos.y);
      if (this.o.mode !== 'belief' || true) {
        const g = ctx.createRadialGradient(px, py, 0, px, py, RANGE * k * 0.6);
        g.addColorStop(0, 'rgba(238,90,79,0.16)'); g.addColorStop(1, 'rgba(238,90,79,0)');
        ctx.fillStyle = g;
        ctx.beginPath(); ctx.moveTo(px, py); ctx.arc(px, py, RANGE * k * 0.6, m.face - FOV / 2, m.face + FOV / 2); ctx.closePath(); ctx.fill();
      }
      if (m.path.length) {
        ctx.setLineDash([3 * this.dpr, 4 * this.dpr]);
        ctx.strokeStyle = 'rgba(238,90,79,0.55)'; ctx.lineWidth = this.dpr * 1.2;
        ctx.beginPath(); ctx.moveTo(px, py);
        for (const p of m.path) ctx.lineTo(X(p.x), Y(p.y));
        ctx.stroke(); ctx.setLineDash([]);
      }
      const r = 7 * this.dpr;
      ctx.save(); ctx.translate(px, py); ctx.rotate(m.face);
      ctx.fillStyle = C.squad;
      ctx.beginPath(); ctx.moveTo(r, 0); ctx.lineTo(-r * 0.8, -r * 0.75); ctx.lineTo(-r * 0.35, 0); ctx.lineTo(-r * 0.8, r * 0.75); ctx.closePath(); ctx.fill();
      ctx.restore();
      ctx.font = font(11, 700); ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
      halo(ctx, m.name[0], px + r + 3 * this.dpr, py - r, C.squad, this.dpr);
    }
    // Encircle: the ring of sectors around the last known position
    if (this.squad.play === 'encircle' && this.o.mode !== 'belief') {
      const p = this.squad.lastPos;
      ctx.strokeStyle = 'rgba(238,90,79,0.5)'; ctx.lineWidth = this.dpr;
      ctx.setLineDash([2 * this.dpr, 5 * this.dpr]);
      ctx.beginPath(); ctx.arc(X(p.x), Y(p.y), (this.squad.go ? 3.5 : 11) * k, 0, Math.PI * 2); ctx.stroke(); ctx.setLineDash([]);
      const sp = this.squad.m[this.squad.spotter];
      if (sp && this.squad.t - this.squad.lastSeen < 0.3) {
        ctx.strokeStyle = 'rgba(238,90,79,0.9)'; ctx.lineWidth = this.dpr * 1.2;
        ctx.beginPath(); ctx.moveTo(X(sp.pos.x), Y(sp.pos.y)); ctx.lineTo(X(p.x), Y(p.y)); ctx.stroke();
      }
    }
    // The player (or the ghost): teal, like YOU on the HUD.
    const pl = this.player();
    if (pl) {
      const x = X(pl.x), y = Y(pl.y), r = 6 * this.dpr;
      const usingPointer = this.pointer && this.squad.t - this.pointerT < 6;
      ctx.strokeStyle = C.you; ctx.lineWidth = this.dpr * 1.5;
      ctx.beginPath(); ctx.moveTo(x, y - r); ctx.lineTo(x + r, y); ctx.lineTo(x, y + r); ctx.lineTo(x - r, y); ctx.closePath();
      if (this.squad.t - this.squad.lastSeen < 0.3) { ctx.fillStyle = C.you; ctx.fill(); }
      ctx.stroke();
      ctx.font = font(11, 700); ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
      halo(ctx, usingPointer ? 'YOU' : 'GHOST', x + r + 4 * this.dpr, y, C.you, this.dpr);
    }
  }
}

function halo(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, color: string, dpr: number) {
  ctx.lineWidth = 3 * dpr; ctx.strokeStyle = 'rgba(18,20,23,0.9)'; ctx.lineJoin = 'round';
  ctx.strokeText(text, x, y);
  ctx.fillStyle = color; ctx.fillText(text, x, y);
}

/** Mount every [data-belief] canvas on the page. Returns a cleanup. */
export function mountBeliefs(root: ParentNode, reduced: boolean): () => void {
  const views: { v: BeliefView; el: HTMLElement; visible: boolean; io?: IntersectionObserver }[] = [];
  const offs: (() => void)[] = [];
  root.querySelectorAll<HTMLElement>('[data-belief]').forEach((el) => {
    const canvas = el.querySelector('canvas');
    if (!canvas) return;
    const ds = el.dataset;
    const radio = el.closest('[data-belief-scope]')?.querySelector<HTMLElement>('[data-radio]') ?? null;
    const readout = el.closest('[data-belief-scope]')?.querySelectorAll<HTMLElement>('[data-readout]') ?? [];
    const v = new BeliefView(canvas, {
      map: ds.map, mode: (ds.mode as Mode) || 'live', squad: +(ds.squad || 4), seed: +(ds.seed || 7),
      zones: ds.zones === 'true', spawns: ds.spawns === 'true', interactive: ds.interactive !== 'false',
      onBark: (who, text) => radio && pushRadio(radio, who, text, reduced),
      onState: (s) => readout.forEach((r) => {
        const key = r.dataset.readout!;
        const val = key === 'mode' ? s.mode : key === 'peak' ? `${s.peak} at ${Math.round(s.peakPct * 100)}%` : key === 'seen' ? (s.seen < 0.5 ? 'now' : `${s.seen.toFixed(0)} s ago`) : `${s.spread.toFixed(0)} m`;
        if (r.textContent !== val) r.textContent = val;
      }),
    });
    v.warm(reduced ? 14 : +(ds.warm || 6));
    const entry = { v, el, visible: false } as (typeof views)[number];
    views.push(entry);
    v.draw();
    if (reduced) return;
    entry.io = new IntersectionObserver(([e]) => (entry.visible = e.isIntersecting), { rootMargin: '80px' });
    entry.io.observe(el);
    const ro = new ResizeObserver(() => { v.resize(); v.draw(); });
    ro.observe(el);
    offs.push(() => ro.disconnect());
    el.addEventListener('pointerdown', (e) => v.setPointer(e.clientX, e.clientY));
    // Chapters can retarget the mode: data-belief-mode on the scope changes, the view resets.
    const scope = el.closest<HTMLElement>('[data-belief-scope]');
    if (scope) {
      const mo = new MutationObserver(() => {
        const m = scope.dataset.beliefMode as Mode | undefined;
        if (m && m !== v.o.mode) { v.reset(m); v.warm(m === 'belief' ? 1 : 2); if (radio) radio.replaceChildren(); }
      });
      mo.observe(scope, { attributes: true, attributeFilter: ['data-belief-mode'] });
      offs.push(() => mo.disconnect());
    }
  });
  if (reduced || !views.length) return () => views.forEach((e) => e.io?.disconnect());
  const move = (e: PointerEvent) => views.forEach(({ v, visible }) => visible && e.pointerType === 'mouse' && v.setPointer(e.clientX, e.clientY));
  addEventListener('pointermove', move, { passive: true });
  const resize = () => views.forEach(({ v }) => { v.resize(); v.draw(); });
  addEventListener('resize', resize);
  const tick = (_t: number, dms: number) => {
    const dt = Math.min(dms / 1000, 0.1);
    for (const e of views) if (e.visible) { e.v.tick(dt); e.v.draw(); }
  };
  gsap.ticker.add(tick);
  return () => {
    gsap.ticker.remove(tick);
    removeEventListener('pointermove', move);
    removeEventListener('resize', resize);
    views.forEach((e) => e.io?.disconnect());
    offs.forEach((f) => f());
  };
}

function pushRadio(list: HTMLElement, who: string, text: string, reduced: boolean) {
  const li = document.createElement('li');
  const w = document.createElement('b');
  w.textContent = who;
  const t = document.createElement('span');
  t.textContent = text;
  li.append(w, t);
  list.prepend(li);
  while (list.children.length > 4) list.lastElementChild?.remove();
  if (!reduced) gsap.from(li, { yPercent: 100, autoAlpha: 0, duration: 0.5, ease: 'expo.out' });
  [...list.children].forEach((c, i) => (c as HTMLElement).classList.toggle('is-old', i > 0));
}

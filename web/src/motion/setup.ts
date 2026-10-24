// Global motion for Shroud. Runs once on the client before the first page initializes.
// Motion verb: spread, then close in. The transition and the loader are the belief grid:
// cells fill outward from where you clicked (a sighting), then clear outward on the new page.
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { ease, isTouch, onPage, onScroll, prefersReducedMotion, setLoaderExit, setLoaderFormat, setTransition, startScroll, stopScroll } from '../engine';
import { mountBeliefs } from './belief';

gsap.registerPlugin(ScrollTrigger);

type Grid = { cols: number; rows: number; cells: HTMLElement[] };

function buildCells(host: HTMLElement): Grid {
  const cols = innerWidth < 768 ? 8 : 16;
  const size = innerWidth / cols;
  const rows = Math.ceil(innerHeight / size);
  host.style.gridTemplateColumns = `repeat(${cols}, 1fr)`;
  host.style.gridTemplateRows = `repeat(${rows}, ${size}px)`;
  const frag = document.createDocumentFragment();
  const cells: HTMLElement[] = [];
  for (let i = 0; i < cols * rows; i++) { const c = document.createElement('i'); frag.append(c); cells.push(c); }
  host.replaceChildren(frag);
  return { cols, rows, cells };
}

function indexAt(g: Grid, x: number, y: number) {
  const size = innerWidth / g.cols;
  const c = Math.max(0, Math.min(g.cols - 1, Math.floor(x / size)));
  const r = Math.max(0, Math.min(g.rows - 1, Math.floor(y / size)));
  return r * g.cols + c;
}

let point = { x: innerWidth / 2, y: innerHeight / 2 };
let curtainGrid: Grid | null = null;

function curtainCells() {
  const host = document.querySelector<HTMLElement>('#curtain .cells');
  if (!host) return null;
  if (!curtainGrid || curtainGrid.cells[0]?.parentElement !== host || curtainGrid.cols !== (innerWidth < 768 ? 8 : 16)) curtainGrid = buildCells(host);
  return curtainGrid;
}

export function setupMotion() {
  addEventListener('pointerdown', (e) => { point = { x: e.clientX, y: e.clientY }; }, { passive: true });
  document.addEventListener('click', (e) => {
    const a = (e.target as HTMLElement).closest?.('a');
    const label = document.querySelector<HTMLElement>('#curtain .curtain__label');
    if (a && label) label.textContent = a.getAttribute('aria-label')?.replace(/, home$/, '') || a.querySelector('span')?.textContent?.trim() || a.textContent?.trim() || '';
  }, true);
  addEventListener('keydown', (e) => { if (e.key === 'Enter') { const r = (document.activeElement as HTMLElement)?.getBoundingClientRect?.(); if (r) point = { x: r.left + r.width / 2, y: r.top + r.height / 2 }; } });

  setTransition({
    leave: (c) => {
      const g = curtainCells();
      if (!g) return gsap.timeline();
      const from = indexAt(g, point.x, point.y);
      gsap.set(g.cells, { scale: 0, '--heat': 0 });
      return gsap.timeline()
        .set(c, { autoAlpha: 1 })
        .to(g.cells, { scale: 1, duration: 0.42, ease: ease.soft, stagger: { grid: [g.rows, g.cols], from, amount: 0.42 } })
        .to(g.cells, { '--heat': 0.85, duration: 0.12, ease: 'none', stagger: { grid: [g.rows, g.cols], from, amount: 0.42 } }, 0.04)
        .to(g.cells, { '--heat': 0, duration: 0.35, ease: ease.soft, stagger: { grid: [g.rows, g.cols], from, amount: 0.42 } }, 0.16)
        .fromTo(c.querySelector('.curtain__label'), { autoAlpha: 0, yPercent: 30 }, { autoAlpha: 1, yPercent: 0, duration: 0.4, ease: ease.out }, 0.3);
    },
    enter: (c) => {
      const g = curtainCells();
      if (!g) return gsap.timeline().set(c, { autoAlpha: 0 });
      return gsap.timeline()
        .to(c.querySelector('.curtain__label'), { autoAlpha: 0, duration: 0.2 }, 0)
        .to(g.cells, { scale: 0, duration: 0.5, ease: ease.inOut, stagger: { grid: [g.rows, g.cols], from: 'center', amount: 0.4 } })
        .set(c, { autoAlpha: 0 });
    },
  });

  // Loader: a radio check. Cells light outward from a spawn point with real progress.
  // Phones: no loader at all (LCP first); desktop keeps the short radio check.
  let seen = false;
  try { seen = sessionStorage.getItem('shroud-radio') === '1'; sessionStorage.setItem('shroud-radio', '1'); } catch {}
  if (innerWidth < 768 || seen || location.pathname !== '/') document.getElementById('loader')?.remove();
  const loader = document.getElementById('loader');
  const lhost = loader?.querySelector<HTMLElement>('.cells');
  let lg: Grid | null = null;
  let order: HTMLElement[] = [];
  let origin = 0;
  if (lhost && !prefersReducedMotion()) {
    lg = buildCells(lhost);
    origin = indexAt(lg, innerWidth * 0.72, innerHeight * 0.7);
    const oc = origin % lg.cols, orr = Math.floor(origin / lg.cols);
    order = [...lg.cells].sort((a, b) => {
      const ia = lg!.cells.indexOf(a), ib = lg!.cells.indexOf(b);
      const da = Math.hypot((ia % lg!.cols) - oc, Math.floor(ia / lg!.cols) - orr);
      const db = Math.hypot((ib % lg!.cols) - oc, Math.floor(ib / lg!.cols) - orr);
      return db - da; // far cells first: the guess narrows onto the spawn point
    });
  }
  let lit = 0;
  setLoaderFormat((p) => {
    if (order.length) {
      const n = Math.floor((p / 100) * order.length);
      for (; lit < n; lit++) order[lit].style.setProperty('--heat', String(0.08 + 0.5 * (lit / order.length) ** 3));
    }
    return String(Math.round(p)).padStart(3, '0');
  });
  setLoaderExit((root) => {
    const tl = gsap.timeline();
    tl.to(root.querySelectorAll('[data-loader-out]'), { yPercent: 40, autoAlpha: 0, duration: 0.25, ease: ease.soft, stagger: 0.03 });
    if (lg) {
      tl.set(root, { backgroundColor: 'transparent' }, 0.05)
        .to(lg.cells, { scale: 0, duration: 0.4, ease: ease.inOut, stagger: { grid: [lg.rows, lg.cols], from: origin, amount: 0.3 } }, 0.05);
    } else tl.to(root, { autoAlpha: 0, duration: 0.3 });
    return tl;
  });

  // The frame steps aside while scrolling down and comes back on the way up.
  let last = 0;
  onScroll(({ y }) => {
    if (Math.abs(y - last) < 4) return;
    document.documentElement.classList.toggle('frame-away', y > 200 && y > last);
    last = y;
  });

  onPage(() => mountBeliefs(document, prefersReducedMotion()));
  onPage(chapters);
  onPage(menu);
  onPage(indexPreview);
  onPage(compare);
  addEventListener('resize', () => { curtainGrid = null; });
}

// Chapters pin the belief view's mode (belief → search → encircle → live).
function chapters() {
  const scope = document.querySelector<HTMLElement>('.hunt[data-belief-scope]');
  if (!scope) return;
  const list = [...scope.querySelectorAll<HTMLElement>('[data-chapter]')];
  const activate = (el: HTMLElement) => {
    list.forEach((c) => c.classList.toggle('is-active', c === el));
    scope.dataset.beliefMode = el.dataset.chapter!;
  };
  list.forEach((el) => ScrollTrigger.create({ trigger: el, start: 'top 60%', end: 'bottom 40%', onToggle: (s) => s.isActive && activate(el) }));
  activate(list[0]);
}

// Phone menu: full-screen overlay, focus trapped, Escape closes, focus restored.
function menu() {
  const open = document.querySelector<HTMLButtonElement>('[data-menu-open]');
  const panel = document.querySelector<HTMLElement>('[data-menu]');
  const close = panel?.querySelector<HTMLButtonElement>('[data-menu-close]');
  if (!open || !panel || !close) return;
  const links = () => [...panel.querySelectorAll<HTMLElement>('a, button')];
  const key = (e: KeyboardEvent) => {
    if (e.key === 'Escape') hide();
    if (e.key !== 'Tab') return;
    const f = links();
    if (e.shiftKey && document.activeElement === f[0]) { e.preventDefault(); f[f.length - 1].focus(); }
    else if (!e.shiftKey && document.activeElement === f[f.length - 1]) { e.preventDefault(); f[0].focus(); }
  };
  const show = () => {
    panel.hidden = false;
    open.setAttribute('aria-expanded', 'true');
    stopScroll();
    document.addEventListener('keydown', key);
    if (!prefersReducedMotion()) {
      gsap.fromTo(panel, { clipPath: 'inset(0% 0% 100% 0%)' }, { clipPath: 'inset(0% 0% 0% 0%)', duration: 0.6, ease: ease.inOut });
      gsap.from(panel.querySelectorAll('li a span'), { yPercent: 110, duration: 0.7, ease: ease.out, stagger: 0.05, delay: 0.2 });
    }
    panel.querySelector<HTMLElement>('a')?.focus();
  };
  const hide = () => {
    panel.hidden = true;
    open.setAttribute('aria-expanded', 'false');
    startScroll();
    document.removeEventListener('keydown', key);
    open.focus();
  };
  open.addEventListener('click', show);
  close.addEventListener('click', hide);
  panel.querySelectorAll('a').forEach((a) => a.addEventListener('click', () => { panel.hidden = true; open.setAttribute('aria-expanded', 'false'); document.removeEventListener('keydown', key); }));
  return () => { document.removeEventListener('keydown', key); };
}

// Map rows: the real capture follows the pointer (desktop). Touch gets inline thumbnails (CSS).
function indexPreview() {
  const list = document.querySelector<HTMLElement>('[data-index]');
  const box = document.querySelector<HTMLElement>('.preview');
  if (!list || !box || isTouch() || prefersReducedMotion()) return;
  const imgs = [...box.querySelectorAll<HTMLElement>('.preview__img')];
  const x = gsap.quickTo(box, 'x', { duration: 0.6, ease: 'power3.out' });
  const y = gsap.quickTo(box, 'y', { duration: 0.6, ease: 'power3.out' });
  const rot = gsap.quickTo(box, 'rotate', { duration: 0.8, ease: 'power3.out' });
  let lastX = 0, px = 0, py = 0, on = false, movedAt = -1e9;
  const move = (e: PointerEvent) => {
    px = e.clientX; py = e.clientY; movedAt = performance.now();
    x(px + innerWidth * 0.16); y(py); rot(gsap.utils.clamp(-6, 6, (px - lastX) * 0.4)); lastX = px;
  };
  const show = (e: Event) => {
    const el = e.currentTarget as HTMLElement;
    // Chrome sends a synthetic pointerenter when content scrolls under a still pointer: ignore it.
    if (e.type !== 'focus' && performance.now() - movedAt > 400) return;
    const id = el.dataset.preview;
    if (e.type === 'focus') { const r = el.getBoundingClientRect(); px = r.left + r.width * 0.55; py = r.top + r.height / 2; x(px + innerWidth * 0.16); y(py); }
    imgs.forEach((i) => i.classList.toggle('is-on', i.dataset.for === id));
    if (!on) gsap.ticker.add(watch);
    if (!on) gsap.fromTo(box, { autoAlpha: 0, clipPath: 'inset(50% 50% 50% 50%)' }, { autoAlpha: 1, clipPath: 'inset(0% 0% 0% 0%)', duration: 0.55, ease: ease.out });
    on = true;
  };
  const watch = () => { if (on && document.activeElement?.closest('[data-index]') == null && !list.contains(document.elementFromPoint(px, py))) hide(); };
  const hide = () => { if (!on) return; on = false; gsap.ticker.remove(watch); gsap.to(box, { autoAlpha: 0, clipPath: 'inset(50% 50% 50% 50%)', duration: 0.4, ease: ease.soft }); };
  const rows = [...list.querySelectorAll<HTMLElement>('a[data-preview]')];
  rows.forEach((a) => { a.addEventListener('pointerenter', show); a.addEventListener('focus', show); });
  list.addEventListener('pointerleave', hide);
  addEventListener('pointermove', move, { passive: true });
  const off = onScroll(() => { if (on && (!list.matches(':hover') || !list.contains(document.elementFromPoint(px, py)))) hide(); });
  rows.forEach((a) => a.addEventListener('blur', hide));
  const st = ScrollTrigger.create({ trigger: list, start: 'top bottom', end: 'bottom top', onToggle: (s) => !s.isActive && hide() });
  return () => { removeEventListener('pointermove', move); off(); st.kill(); gsap.ticker.remove(watch); gsap.set(box, { autoAlpha: 0 }); };
}

// Low/High compare on the Office page: a native range input drives the split (keyboard works).
function compare() {
  document.querySelectorAll<HTMLElement>('.compare').forEach((el) => {
    const input = el.querySelector<HTMLInputElement>('input');
    if (!input) return;
    const set = () => el.style.setProperty('--pos', `${input.value}%`);
    input.addEventListener('input', set);
    set();
  });
}

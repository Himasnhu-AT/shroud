import type { APIRoute } from 'astro';
import { site } from '../data/site';

// Generated so the sitemap URL follows `site.url` in src/data/site.ts.
export const GET: APIRoute = () =>
  new Response(`User-agent: *\nAllow: /\nSitemap: ${new URL('/sitemap-index.xml', site.url).href}\n`, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });

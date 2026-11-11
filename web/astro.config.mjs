// @ts-check
import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import { site } from './src/data/site.ts';

// The production URL lives in src/data/site.ts (`site.url`, a placeholder domain for now).
export default defineConfig({
  site: site.url,
  integrations: [sitemap()],
  prefetch: { prefetchAll: true, defaultStrategy: 'hover' },
  build: { inlineStylesheets: 'auto' },
  server: { port: 4402 },
});

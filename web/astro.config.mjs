// @ts-check
import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';

// Set `site` to the production URL before deploying (placeholder domain; see src/data/site.ts).
export default defineConfig({
  site: 'https://shroud.marlgames.com',
  integrations: [sitemap()],
  prefetch: { prefetchAll: true, defaultStrategy: 'hover' },
  build: { inlineStylesheets: 'auto' },
  server: { port: 4402 },
});

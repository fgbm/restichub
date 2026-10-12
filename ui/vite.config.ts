import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
	plugins: [
		sveltekit({
			// Single-page application mode (ADR-0012). The control plane serves the
			// built files at the root and answers unknown paths with index.html.
			adapter: adapter({
				fallback: 'index.html',
				strict: false
			}),
			paths: {
				base: ''
			}
		})
	],
	server: {
		port: 5173,
		proxy: {
			// The development proxy sends API calls to the control plane.
			// The key is a regular expression, so that the `/api-tokens` route
			// is not sent to the control plane.
			'^/api/': {
				target: 'http://127.0.0.1:8080',
				changeOrigin: false
			}
		}
	},
	test: {
		environment: 'jsdom',
		include: ['src/**/*.{test,spec}.{js,ts}'],
		setupFiles: ['./src/lib/test/setup.ts'],
		globals: true
	},
	resolve: {
		// Component tests run against the browser build of Svelte.
		conditions: ['browser']
	}
});

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
			},
			// The control plane sends the same policy as an HTTP header. SvelteKit
			// adds the SHA-256 hashes of the inline bootstrap scripts to
			// `script-src`, so the policy needs no `unsafe-inline`.
			csp: {
				mode: 'hash',
				directives: {
					'default-src': ['self'],
					'img-src': ['self', 'data:'],
					'style-src': ['self'],
					// SvelteKit wraps the inline bootstrap script in a `display: contents` div.
					// Its style attribute is allowed by hash only (ADR-0027).
					'style-src-attr': ['self', "'unsafe-hashes'", "'sha256-tcbDxjMo+xKqM21aCGYbs/QAJqB7yUXC06oPWDapBgc='"],
					'script-src': ['self'],
					'connect-src': ['self'],
					'form-action': ['self']
				}
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

import { defineConfig, devices } from '@playwright/test';

// The port is configurable, so a second run on one host does not collide (CI and a local agent).
const port = Number(process.env.E2E_PORT ?? 4173);
const baseURL = `http://127.0.0.1:${port}`;

// The smoke test needs a real control plane, and the documentation screenshots need the pinned
// image: playwright.smoke.config.ts and playwright.docs.config.ts run them. A project keeps this
// list, because its own testIgnore replaces the one below.
const OWN_CONFIG = [/smoke.*\.spec\.ts/, /docs-screens\.spec\.ts/];

export default defineConfig({
	testDir: './e2e',
	// The smoke test needs a real control plane: playwright.smoke.config.ts runs it. The
	// documentation screenshots need the pinned image: playwright.docs.config.ts runs them.
	testIgnore: OWN_CONFIG,
	timeout: 30_000,
	fullyParallel: true,
	use: {
		baseURL
	},
	webServer: {
		command: `PORT=${port} node e2e/server.mjs`,
		url: `${baseURL}/sign-in`,
		reuseExistingServer: false,
		timeout: 30_000
	},
	projects: [
		{
			name: 'chromium',
			use: { ...devices['Desktop Chrome'] },
			testIgnore: [...OWN_CONFIG, /scrollbars\.spec\.ts/]
		},
		{
			name: 'firefox',
			use: { ...devices['Desktop Firefox'] },
			testIgnore: [...OWN_CONFIG, /scrollbars\.spec\.ts/]
		},
		{
			// Chromium hides the window scrollbar in these projects. The layout must also hold in a
			// browser where the scrollbar takes 15 px of the window, as a classic scrollbar does.
			name: 'chromium-classic-scrollbars',
			testMatch: /(scrollbars|layout-width)\.spec\.ts/,
			use: {
				...devices['Desktop Chrome'],
				launchOptions: {
					ignoreDefaultArgs: ['--hide-scrollbars'],
					args: ['--disable-features=OverlayScrollbar']
				}
			}
		}
	]
});

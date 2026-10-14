import { defineConfig, devices } from '@playwright/test';

// The port is configurable, so a second run on one host does not collide (CI and a local agent).
const port = Number(process.env.E2E_PORT ?? 4173);
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
	testDir: './e2e',
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
		{ name: 'chromium', use: { ...devices['Desktop Chrome'] } },
		{ name: 'firefox', use: { ...devices['Desktop Firefox'] } }
	]
});

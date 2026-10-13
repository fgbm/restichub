import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
	testDir: './e2e',
	timeout: 30_000,
	fullyParallel: true,
	use: {
		baseURL: 'http://127.0.0.1:4173'
	},
	webServer: {
		command: 'node e2e/server.mjs',
		url: 'http://127.0.0.1:4173/sign-in',
		reuseExistingServer: false,
		timeout: 30_000
	},
	projects: [
		{ name: 'chromium', use: { ...devices['Desktop Chrome'] } },
		{ name: 'firefox', use: { ...devices['Desktop Firefox'] } }
	]
});

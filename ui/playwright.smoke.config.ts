import { defineConfig, devices } from '@playwright/test';

// The smoke test needs a running control plane that serves the UI (scripts/ui-smoke.sh). It reads
// the address from SMOKE_URL.
export default defineConfig({
	testDir: './e2e',
	testMatch: /smoke.*\.spec\.ts/,
	timeout: 60_000,
	workers: 1,
	use: {
		baseURL: process.env.SMOKE_URL ?? 'http://127.0.0.1:8080'
	},
	projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }]
});

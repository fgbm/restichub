import { defineConfig, devices } from '@playwright/test';

// The documentation screenshots. Run it in the pinned Playwright image (scripts/docs-screens.sh),
// so the fonts match on every machine. `--update-snapshots=all` writes the frames, a plain run
// compares the current UI with them.
const port = Number(process.env.E2E_PORT ?? 4174);
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
	testDir: './e2e',
	testMatch: 'docs-screens.spec.ts',
	timeout: 60_000,
	fullyParallel: true,
	snapshotPathTemplate: '../docs/src/assets/screens/{arg}{ext}',
	expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.01, animations: 'disabled', caret: 'hide' } },
	use: {
		baseURL,
		...devices['Desktop Chrome'],
		viewport: { width: 1280, height: 800 },
		deviceScaleFactor: 1,
		// The font override of the spec is an inline style.
		bypassCSP: true,
		timezoneId: 'UTC'
	},
	webServer: {
		command: `PORT=${port} node e2e/server.mjs`,
		url: `${baseURL}/sign-in`,
		reuseExistingServer: false,
		timeout: 30_000
	}
});

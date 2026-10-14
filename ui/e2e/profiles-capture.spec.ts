import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the Profiles screen for a review. It runs only with CAPTURE and CAPTURE_DIR set, so CI
// skips it (FR-PF-01, FR-PF-04).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

type Labels = {
	nav: string;
	repositories: string;
	profiles: string;
	addProfile: string;
	moreActions: (name: string) => string;
};

const EN: Labels = {
	nav: 'Primary navigation',
	repositories: 'Repositories',
	profiles: 'Profiles',
	addProfile: 'Add profile',
	moreActions: (name) => `More actions for ${name}`
};
const RU: Labels = {
	nav: 'Основная навигация',
	repositories: 'Репозитории',
	profiles: 'Профили',
	addProfile: 'Добавить профиль',
	moreActions: (name) => `Другие действия для ${name}`
};

async function signIn(page: Page, labels: Labels): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: labels.nav })).toBeVisible();
}

async function open(page: Page, labels: Labels, name: string): Promise<void> {
	await page
		.getByRole('navigation', { name: labels.nav })
		.getByRole('link', { name, exact: true })
		.click();
	await page.getByRole('heading', { level: 1 }).waitFor();
}

// A profile needs a repository, and the sample profile points at the first one. The form fields
// use fixed ids, so the helper works in both languages.
async function createRepository(page: Page, labels: Labels): Promise<void> {
	await open(page, labels, labels.repositories);
	await page
		.getByRole('button')
		.filter({ hasText: /(Add (the first )?repository|Добавить (первый )?репозиторий)/ })
		.first()
		.click();
	await page.locator('#repo-name').fill('s3-prod');
	await page.locator('#repo-endpoint').fill('https://s3.example.net');
	await page.locator('#repo-bucket').fill('backups-prod');
	await page.locator('#repo-password').fill('repo-password');
	await page.locator('#repo-s3-access-key').fill('access-key');
	await page.locator('#repo-s3-secret-key').fill('secret-key');
	await page.getByRole('dialog').locator('button[type="submit"]').click();
	await expect(page.getByRole('dialog')).toHaveCount(0);
}

async function shot(page: Page, name: string, scheme: string): Promise<void> {
	await page.evaluate(() =>
		document
			.querySelectorAll<HTMLButtonElement>('.toast button')
			.forEach((button) => button.click())
	);
	await page.waitForTimeout(250);
	await page.screenshot({ path: `${out}/${name}-${scheme}-app.png` });
}

async function capture(page: Page, scheme: string, labels: Labels, suffix: string): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: scheme });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
	});
	if (labels === RU) {
		await page.addInitScript(() => localStorage.setItem('restichub.locale', 'ru'));
	}
	await signIn(page, labels);
	await createRepository(page, labels);

	await open(page, labels, labels.profiles);
	await page.getByText('web-daily').waitFor();
	await shot(page, `profiles-list${suffix}`, scheme);

	// The create dialog.
	await page.getByRole('button', { name: labels.addProfile, exact: true }).click();
	await page.locator('#profile-name').fill('mac-files');
	await page.locator('#profile-paths-all').fill('/Users');
	await page.locator('#profile-tags').fill('macos');
	await page.locator('input[type="checkbox"]').first().check();
	await shot(page, `profiles-create${suffix}`, scheme);
	await page.keyboard.press('Escape');

	// The preview panel.
	await page.getByRole('button', { name: labels.moreActions('web-daily'), exact: true }).click();
	await page.getByRole('menuitem').first().click();
	await page.locator('#preview-host').selectOption({ index: 1 });
	await expect(page.getByText('restic backup', { exact: false })).toBeVisible();
	await shot(page, `profiles-preview${suffix}`, scheme);
	await page.keyboard.press('Escape');
}

for (const scheme of ['light', 'dark'] as const) {
	test(`capture the profiles screen in the ${scheme} theme`, async ({ page }) => {
		await capture(page, scheme, EN, '');
	});
}

test('capture the profiles screen in Russian', async ({ page }) => {
	await capture(page, 'light', RU, '-ru');
});

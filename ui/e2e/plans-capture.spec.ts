import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the Plans screen for a review. It runs only with CAPTURE and CAPTURE_DIR set, so CI
// skips it (FR-SC-01, FR-SC-07).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

type Labels = {
	nav: string;
	plans: string;
	addPlan: string;
	edit: string;
	moreActions: (name: string) => string;
};

const EN: Labels = {
	nav: 'Primary navigation',
	plans: 'Plans',
	addPlan: 'Add plan',
	edit: 'Edit',
	moreActions: (name) => `More actions for ${name}`
};

const RU: Labels = {
	nav: 'Основная навигация',
	plans: 'Планы',
	addPlan: 'Добавить план',
	edit: 'Изменить',
	moreActions: (name) => `Другие действия для ${name}`
};

async function signIn(page: Page, labels: Labels): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: labels.nav })).toBeVisible();
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
		localStorage.setItem('restichub.mock.plans', 'sample');
	});
	if (labels === RU) {
		await page.addInitScript(() => localStorage.setItem('restichub.locale', 'ru'));
	}
	await signIn(page, labels);

	// The list holds one plan.
	await page
		.getByRole('navigation', { name: labels.nav })
		.getByRole('link', { name: labels.plans, exact: true })
		.click();
	await page.getByText('nightly-web').waitFor();
	await shot(page, `plans-list${suffix}`, scheme);

	// The create dialog with the next-runs section.
	await page.getByRole('button', { name: labels.addPlan, exact: true }).click();
	await page.locator('#plan-name').fill('nightly-db');
	await page.locator('#plan-profile').selectOption({ index: 1 });
	await page.locator('#plan-agent').selectOption({ index: 1 });
	await page.locator('#plan-cron').fill('30 3 * * *');
	await page.locator('#plan-time-zone').fill('Europe/Berlin');
	await page.locator('.runs').waitFor();
	await page.locator('.runs').scrollIntoViewIfNeeded();
	await page.waitForTimeout(150);
	await shot(page, `plans-create${suffix}`, scheme);
	await page.keyboard.press('Escape');

	// The edit dialog shows the next five runs of the stored plan.
	await page.getByRole('button', { name: labels.moreActions('nightly-web'), exact: true }).click();
	await page.getByRole('menuitem', { name: labels.edit }).click();
	await expect(page.getByRole('dialog')).toBeVisible();
	await page.locator('.runs li').first().waitFor();
	await page.locator('.runs').scrollIntoViewIfNeeded();
	await page.waitForTimeout(150);
	await shot(page, `plans-edit-next-runs${suffix}`, scheme);
	await page.keyboard.press('Escape');
}

for (const scheme of ['light', 'dark'] as const) {
	test(`capture the plans screen in the ${scheme} theme`, async ({ page }) => {
		await capture(page, scheme, EN, '');
	});
}

test('capture the plans screen in Russian', async ({ page }) => {
	await capture(page, 'light', RU, '-ru');
});

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
	jobs: string;
	addPlan: string;
};

const EN: Labels = {
	nav: 'Primary navigation',
	plans: 'Plans',
	jobs: 'Jobs',
	addPlan: 'Add plan'
};

const RU: Labels = {
	nav: 'Основная навигация',
	plans: 'Планы',
	jobs: 'Задания',
	addPlan: 'Добавить план'
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

async function capture(
	page: Page,
	scheme: string,
	labels: Labels,
	suffix: string,
	width = 1440
): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width, height: 900 });
	await page.emulateMedia({ colorScheme: scheme });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
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
	// The Overview lists the sample plan too, so wait for the Plans screen first.
	await page.getByRole('heading', { level: 1, name: labels.plans }).waitFor();
	await page.getByRole('link', { name: 'nightly-web', exact: true }).waitFor();
	await shot(page, `plans-list${suffix}`, scheme);

	// The new plan page with the next runs of the form schedule.
	await page.getByRole('button', { name: labels.addPlan, exact: true }).click();
	await page.locator('#plan-name').fill('nightly-db');
	await page.locator('#plan-profile').selectOption({ index: 1 });
	await page.locator('#plan-cron').fill('30 3 * * *');
	await page.locator('#plan-time-zone').fill('Europe/Berlin');
	await page.locator('.runs li').first().waitFor();
	await shot(page, `plans-new${suffix}`, scheme);
	await page.mouse.wheel(0, 4000);
	await shot(page, `plans-new-bottom${suffix}`, scheme);
	// Leaving the filled form asks for a confirmation.
	page.once('dialog', (dialog) => void dialog.accept());
	await page.goBack();
	await page.getByRole('link', { name: 'nightly-web', exact: true }).waitFor();

	// The page of the stored plan.
	await page.getByRole('link', { name: 'nightly-web', exact: true }).click();
	await page.locator('.runs li').first().waitFor();
	await shot(page, `plans-edit${suffix}`, scheme);
	await page.mouse.wheel(0, 4000);
	await shot(page, `plans-edit-bottom${suffix}`, scheme);

	// The Jobs screen lists the sample jobs in every lifecycle state.
	await page
		.getByRole('navigation', { name: labels.nav })
		.getByRole('link', { name: labels.jobs, exact: true })
		.click();
	await page.getByRole('table').getByText('win-fs01').first().waitFor();
	await shot(page, `jobs-list${suffix}`, scheme);
}

for (const scheme of ['light', 'dark'] as const) {
	test(`capture the plans screen in the ${scheme} theme`, async ({ page }) => {
		await capture(page, scheme, EN, '');
	});
}

test('capture the plans screen in Russian', async ({ page }) => {
	await capture(page, 'light', RU, '-ru');
});

test('capture the plans screen in Russian on a phone', async ({ page }) => {
	await capture(page, 'light', RU, '-ru-390', 390);
});

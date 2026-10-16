import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the Repositories screen with the maintenance section for a review. It runs only with
// CAPTURE and CAPTURE_DIR set, so CI skips it (FR-RP-08).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

type Labels = {
	nav: string;
	repositories: string;
	addRepository: string;
	save: string;
	cancel: string;
	edit: string;
	initialize: string;
	start: string;
	moreActions: (name: string) => string;
	hostName: string;
	unlockAll: string;
};

const EN: Labels = {
	nav: 'Primary navigation',
	repositories: 'Repositories',
	addRepository: 'Add repository',
	save: 'Save',
	cancel: 'Cancel',
	edit: 'Edit',
	initialize: 'Initialize',
	start: 'Start',
	moreActions: (name) => `More actions for ${name}`,
	hostName: 'nas-backup',
	unlockAll: 'Unlock all'
};

const RU: Labels = {
	nav: 'Основная навигация',
	repositories: 'Репозитории',
	addRepository: 'Добавить репозиторий',
	save: 'Сохранить',
	cancel: 'Отмена',
	edit: 'Изменить',
	initialize: 'Инициализировать',
	start: 'Запустить',
	moreActions: (name) => `Другие действия для ${name}`,
	hostName: 'nas-backup',
	unlockAll: 'Снять все блокировки'
};

async function shot(page: Page, name: string): Promise<void> {
	await page.evaluate(() =>
		document
			.querySelectorAll<HTMLButtonElement>('.toast button')
			.forEach((button) => button.click())
	);
	await page.waitForTimeout(250);
	await page.screenshot({ path: `${out}/${name}.png` });
}

async function addLocal(page: Page, labels: Labels, name: string, host: string | null) {
	await page.getByRole('button', { name: labels.addRepository, exact: true }).click();
	await page.locator('#repo-name').fill(name);
	await page.locator('#repo-kind').selectOption('local');
	await page.locator('#repo-path').fill(`/srv/backup/${name}`);
	await page.locator('#repo-password').fill('repo-password');
	if (host) await page.locator('#repo-agent').selectOption({ label: host });
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByText(name, { exact: true })).toBeVisible();
}

async function capture(page: Page, labels: Labels, suffix: string): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: Number(process.env.CAPTURE_WIDTH ?? 1440), height: 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
	if (suffix) await page.addInitScript(() => localStorage.setItem('restichub.locale', 'ru'));
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: labels.nav })).toBeVisible();
	await page
		.getByRole('navigation', { name: labels.nav })
		.getByRole('link', { name: labels.repositories, exact: true })
		.click();

	await page.getByRole('button', { name: labels.addRepository, exact: true }).click();
	await page.locator('.runs li').first().waitFor();
	await page.waitForTimeout(150);
	await shot(page, `new-repository${suffix}`);
	await page
		.locator('[data-section="retention"]')
		.evaluate((element) => element.scrollIntoView({ block: 'start' }));
	await page.waitForTimeout(150);
	await shot(page, `new-repository-maintenance${suffix}`);
	await page.getByRole('button', { name: labels.cancel, exact: true }).click();

	await addLocal(page, labels, 'backups-prod', labels.hostName);
	await addLocal(page, labels, 'scratch-local', null);
	await page.getByRole('button', { name: labels.moreActions('backups-prod') }).click();
	await page.getByRole('menuitem', { name: labels.initialize }).click();
	await page.getByRole('dialog').getByRole('button', { name: labels.start }).click();
	await page
		.getByRole('dialog')
		.getByText(/ready|готов/i)
		.waitFor({ timeout: 10_000 });
	await page.keyboard.press('Escape');
	await shot(page, `repositories-list${suffix}`);

	await page.getByRole('button', { name: labels.moreActions('backups-prod') }).click();
	await page.getByRole('menuitem', { name: labels.unlockAll }).click();
	await page.locator('#unlock-all-confirm').waitFor();
	await shot(page, `unlock-all${suffix}`);
	await page.keyboard.press('Escape');

	await page.getByRole('button', { name: labels.moreActions('backups-prod') }).click();
	await page.getByRole('menuitem', { name: labels.edit }).click();
	await page.locator('.runs li').first().waitFor();
	await page
		.locator('[data-section="retention"]')
		.evaluate((element) => element.scrollIntoView({ block: 'start' }));
	await page.waitForTimeout(150);
	await shot(page, `repository-maintenance${suffix}`);
	await page
		.locator('[data-section="check"]')
		.evaluate((element) => element.scrollIntoView({ block: 'end' }));
	await page.waitForTimeout(150);
	await shot(page, `repository-maintenance-check${suffix}`);
	await page.getByRole('button', { name: labels.cancel, exact: true }).click();

	await page.getByRole('button', { name: labels.moreActions('scratch-local') }).click();
	await page.getByRole('menuitem', { name: labels.edit }).click();
	await page.locator('#repo-agent').waitFor();
	await page
		.locator('#repo-agent')
		.evaluate((element) => element.scrollIntoView({ block: 'center' }));
	await page.waitForTimeout(150);
	await shot(page, `repository-maintenance-no-host${suffix}`);
}

test('capture the repositories screen in the light theme', async ({ page }) => {
	await capture(page, EN, '');
});

test('capture the repositories screen in Russian', async ({ page }) => {
	await capture(page, RU, '-ru');
});

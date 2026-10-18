import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the screens 4, 5 and 8 to 13 of the parameters design for the owner in Russian, at
// 1440 px, light and dark. It runs only with CAPTURE and CAPTURE_DIR set, so CI skips it
// (ADR-0047).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

const nav = (page: Page) => page.getByRole('navigation', { name: 'Основная навигация' });

async function start(page: Page, scheme: 'light' | 'dark'): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 1000 });
	await page.emulateMedia({ colorScheme: scheme });
	await page.addInitScript(() => {
		for (const key of ['agents', 'profiles', 'plans', 'scripts', 'repositories']) {
			localStorage.setItem(`restichub.mock.${key}`, 'sample');
		}
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Войти', exact: true }).click();
	await expect(nav(page)).toBeVisible();
}

async function shot(page: Page, name: string): Promise<void> {
	await page.screenshot({ path: `${out}/${name}.png`, fullPage: true });
}

async function openProfile(page: Page, name: string): Promise<void> {
	await nav(page).getByRole('link', { name: 'Профили', exact: true }).click();
	await page.getByRole('link', { name }).click();
	await page.getByRole('button', { name: 'Развернуть всё' }).click();
}

async function openHost(page: Page): Promise<void> {
	await nav(page).getByRole('link', { name: 'Хосты', exact: true }).click();
	await page.getByRole('link', { name: 'srv-db-01' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'srv-db-01' })).toBeVisible();
}

async function switchOrganization(page: Page, name: string): Promise<void> {
	await page.getByRole('button', { name: /Сменить организацию/ }).click();
	await page.getByRole('menuitemradio', { name: new RegExp(name) }).click();
	await expect(page.getByRole('button', { name: /Сменить организацию/ })).toContainText(name);
}

for (const scheme of ['light', 'dark'] as const) {
	test(`screen 4: the profile list with the count of the parameters (${scheme})`, async ({
		page
	}) => {
		await start(page, scheme);
		await nav(page).getByRole('link', { name: 'Профили', exact: true }).click();
		await expect(page.getByRole('heading', { level: 1, name: 'Профили' })).toBeVisible();
		await shot(page, `04-profiles-list-ru-${scheme}`);
	});

	test(`screen 5: the profile picker with the chip and the search (${scheme})`, async ({
		page
	}) => {
		await start(page, scheme);
		await nav(page).getByRole('link', { name: 'Планы', exact: true }).click();
		await page.getByRole('button', { name: 'Добавить план' }).click();
		await expect(page.getByRole('heading', { level: 1, name: 'Новый план' })).toBeVisible();
		await page.locator('#plan-profile').click();
		await page.getByPlaceholder('Профиль, путь или параметр').fill('pg');
		await shot(page, `05-profile-picker-ru-${scheme}`);
	});

	test(`screen 8: the parameters of a profile (${scheme})`, async ({ page }) => {
		await start(page, scheme);
		await openProfile(page, 'db-nightly');
		await expect(page.locator('[data-section="parameters"]')).toBeVisible();
		await shot(page, `08-profile-parameters-ru-${scheme}`);
	});

	test(`screen 9: a script variable of the kind "profile parameter" (${scheme})`, async ({
		page
	}) => {
		await start(page, scheme);
		await openProfile(page, 'db-nightly');
		// The dump script reads its directory from the parameter pg_dir of the profile.
		await page.getByRole('button', { name: 'Показать скрипт' }).nth(1).click();
		await expect(page.getByRole('combobox', { name: /^Параметр/ })).toBeVisible();
		await page.getByRole('combobox', { name: /^Параметр/ }).scrollIntoViewIfNeeded();
		await shot(page, `09-script-parameter-ru-${scheme}`);
	});

	test(`screen 10: the values of a host (${scheme})`, async ({ page }) => {
		await start(page, scheme);
		await openHost(page);
		await shot(page, `10-host-values-admin-ru-${scheme}`);

		await switchOrganization(page, 'globex');
		await openHost(page);
		await shot(page, `10-host-values-operator-ru-${scheme}`);

		await switchOrganization(page, 'initech');
		await openHost(page);
		await shot(page, `10-host-values-viewer-ru-${scheme}`);
	});

	test(`screen 11: the parameters of the profile on the plan page (${scheme})`, async ({
		page
	}) => {
		await start(page, scheme);
		await nav(page).getByRole('link', { name: 'Планы', exact: true }).click();
		await page.getByRole('link', { name: 'nightly-db' }).click();
		await expect(page.locator('[data-section="parameters"]')).toContainText('хост');
		await shot(page, `11-plan-parameters-ok-ru-${scheme}`);

		// A plan on a host without values: a required parameter is missing, so the save is disabled.
		await nav(page).getByRole('link', { name: 'Планы', exact: true }).click();
		await page.getByRole('button', { name: 'Добавить план' }).click();
		await page.locator('#plan-profile').click();
		await page.getByRole('option', { name: /^db-nightly/ }).click();
		await page.locator('#plan-agent').click();
		await page.getByRole('option', { name: 'win-fs01', exact: true }).click();
		await expect(page.locator('[data-section="parameters"]')).toContainText('не задано');
		await shot(page, `11-plan-parameters-missing-ru-${scheme}`);
	});

	test(`screen 12: the profile save is refused with the plans (${scheme})`, async ({ page }) => {
		await start(page, scheme);
		await openProfile(page, 'db-nightly');
		await page.getByRole('button', { name: 'Добавить параметр' }).click();
		await page.getByLabel('Параметр', { exact: true }).last().fill('pg_port');
		await page.getByLabel('Обязательный', { exact: true }).last().check();
		await page.getByRole('button', { name: 'Сохранить', exact: true }).click();
		await expect(page.getByText(/Профиль не сохранён/)).toBeVisible();
		await shot(page, `12-profile-refused-ru-${scheme}`);
	});

	test(`screen 13: the values are masked in the commands and the log (${scheme})`, async ({
		page
	}) => {
		await start(page, scheme);
		await openProfile(page, 'db-nightly');
		await page.getByRole('button', { name: /^Команды restic/ }).click();
		await expect(page.getByText(/Значения в путях/)).toBeVisible();
		await shot(page, `13-commands-masked-ru-${scheme}`);
		await page.getByRole('button', { name: 'Закрыть', exact: true }).click();

		await nav(page).getByRole('link', { name: 'Задания', exact: true }).click();
		await page
			.getByRole('link', { name: /nightly-db/ })
			.first()
			.click();
		await page.waitForURL(/\/jobs\/[^/]+$/);
		await page
			.getByRole('button', { name: /Показать журнал шага «Dump the PostgreSQL database»/ })
			.click();
		await expect(page.getByText(/PGPASSWORD=/)).toBeVisible();
		await shot(page, `13-job-log-masked-ru-${scheme}`);
	});
}

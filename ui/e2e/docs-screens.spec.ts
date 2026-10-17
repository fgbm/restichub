import { expect, test, type Page } from '@playwright/test';
import { en, type MessageKey } from '../src/lib/i18n/en';
import { ru } from '../src/lib/i18n/ru';

// The frames of the documentation (docs/src/assets/screens). playwright.docs.config.ts runs this
// spec in the pinned image: scripts/docs-screens.sh writes the frames, CI compares them.
// The demo data set and the fixed clock make each frame the same on every run.

const NOW = new Date('2026-10-14T09:42:00Z');

type Lang = 'en' | 'ru';
const catalogs: Record<Lang, Record<MessageKey, string>> = { en, ru };

type Text = (key: MessageKey) => string;

interface Frame {
	name: string;
	path: string;
	/** Takes the whole page, for a list that is longer than the window. */
	full?: boolean;
	/** Opens a dialog or selects a row after the page is ready. */
	act?: (page: Page, text: Text) => Promise<void>;
}

async function openSnapshot(page: Page, text: Text): Promise<void> {
	await page.getByRole('combobox', { name: text('restore.repository') }).selectOption('r_2');
	const snapshot = page.getByRole('link', { name: openSnapshotName(text) }).first();
	await tick(page, () => snapshot.isVisible());
	await snapshot.click();
	const expand = page.getByRole('button', { name: text('restore.expand').replace('{name}', 'srv') });
	await tick(page, () => expand.isVisible());
	await expand.click();
	const child = page.getByRole('button', { name: text('restore.expand').replace('{name}', 'www') });
	await tick(page, () => child.isVisible());
}

function openSnapshotName(text: Text): RegExp {
	return new RegExp(`^${text('restore.openSnapshot').replace('{id}', '')}`);
}

/**
 * The mock answers a snapshot or a tree query when its clock passes the ready time, and the fixed
 * clock never moves on its own. The clock steps one second until the element appears.
 */
const elapsed = new WeakMap<Page, number>();

async function tick(page: Page, until: () => Promise<boolean>): Promise<void> {
	for (let step = 0; step < 20 && !(await until()); step++) {
		const seconds = (elapsed.get(page) ?? 0) + 1;
		elapsed.set(page, seconds);
		await page.clock.setFixedTime(NOW.getTime() + seconds * 1_000);
		await page.waitForTimeout(400);
	}
}

const frames: Frame[] = [
	{ name: 'overview', path: '/' },
	{ name: 'hosts-list-online', path: '/hosts' },
	{
		name: 'hosts-add-command',
		path: '/hosts',
		act: async (page, text) => {
			await page.getByRole('button', { name: text('hosts.add'), exact: true }).click();
			await page.getByRole('button', { name: text('hosts.createToken'), exact: true }).click();
			await page.locator('.copy-field').first().waitFor();
		}
	},
	{ name: 'repositories-list', path: '/repositories' },
	{ name: 'repository-new', path: '/repositories/new' },
	{ name: 'profiles-list', path: '/profiles' },
	{ name: 'profile-editor', path: '/profiles/p_1' },
	{ name: 'plans-list', path: '/plans' },
	{ name: 'plan-page', path: '/plans/pl_1' },
	{ name: 'jobs-list', path: '/jobs', full: true },
	{ name: 'job-detail', path: '/jobs/0199e035-2b7c-7e91-85d6-4a0f9c3b1e05' },
	{ name: 'restore-snapshot', path: '/restore', act: openSnapshot },
	{
		name: 'restore-dialog',
		path: '/restore',
		act: async (page, text) => {
			await openSnapshot(page, text);
			await page.getByRole('button', { name: text('restore.restore'), exact: true }).click();
			await page.getByRole('dialog').waitFor();
		}
	},
	{ name: 'alerts-list', path: '/alerts' },
	{ name: 'settings-access', path: '/settings/access/users' },
	{ name: 'audit-log', path: '/audit' }
];

async function prepare(
	page: Page,
	lang: Lang,
	theme: 'light' | 'dark',
	data: 'docs' | 'setup' = 'docs'
): Promise<void> {
	await page.clock.setFixedTime(NOW);
	await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
	await page.addInitScript(
		([locale, scheme, set]) => {
			localStorage.setItem('restichub.mock.docs', set);
			localStorage.setItem('restichub.locale', locale);
			localStorage.setItem('restichub.theme', scheme);
		},
		[lang, theme, data]
	);
	// The pinned image maps system-ui to a CJK font with one weight, so the frames would lose
	// the bold text. Liberation has every weight that the UI uses.
	await page.addInitScript(() =>
		document.addEventListener('DOMContentLoaded', () => {
			const style = document.createElement('style');
			style.textContent =
				":root { --rh-font-sans: 'Liberation Sans', sans-serif; --rh-font-mono: 'Liberation Mono', monospace; }";
			document.head.append(style);
		})
	);
}

async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await page.locator('nav').first().waitFor();
}

for (const lang of ['en', 'ru'] as const) {
	for (const theme of ['light', 'dark'] as const) {
		const text: Text = (key) => catalogs[lang][key];

		test(`setup-first-admin.${lang}.${theme}`, async ({ page }) => {
			await prepare(page, lang, theme, 'setup');
			await page.goto('/setup');
			await page.getByRole('heading', { level: 1 }).waitFor();
			await expect(page).toHaveScreenshot(`setup-first-admin.${lang}.${theme}.png`);
		});

		for (const frame of frames) {
			test(`${frame.name}.${lang}.${theme}`, async ({ page }) => {
				await prepare(page, lang, theme);
				await signIn(page);
				await page.goto(frame.path);
				await page.getByRole('heading', { level: 1 }).first().waitFor();
				await frame.act?.(page, text);
				await page.waitForLoadState('networkidle');
				// A toast of an action would cover the frame.
				await page.evaluate(() =>
					document
						.querySelectorAll<HTMLButtonElement>('.toast button')
						.forEach((button) => button.click())
				);
				await expect(page).toHaveScreenshot(`${frame.name}.${lang}.${theme}.png`, {
					fullPage: frame.full
				});
			});
		}
	}
}

import { expect, test, type Page } from '@playwright/test';

// A late preview, summary or banner must not move a control that is already on the page: a click
// that aims at a control then lands on the wrong one. The check samples the top of each control
// that has an id from the first paint of the page to 1500 ms later.
const EDITORS = [
	{ link: 'Plans', add: 'Add plan', then: 'Daily at 02:00' },
	{ link: 'Profiles', add: 'Add profile', then: null },
	{ link: 'Repositories', add: 'Add repository', then: 'Daily at 03:00' }
];

async function signIn(page: Page): Promise<void> {
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function shifts(page: Page): Promise<string[]> {
	return page.evaluate(
		() =>
			new Promise<string[]>((done) => {
				const first = new Map<string, number>();
				const moved = new Map<string, number>();
				const started = performance.now();
				const sample = () => {
					const seen = new Map<string, number>();
					for (const element of document.querySelectorAll(
						'main input, main select, main textarea, main button, main a[href]'
					)) {
						// The page-level save bar is fixed, so it cannot shift with the content.
						if (element.closest('.record__bar')) continue;
						const rect = element.getBoundingClientRect();
						if (rect.width === 0 && rect.height === 0) continue;
						const label = `${element.tagName.toLowerCase()} ${
							element.id ||
							element.getAttribute('aria-label') ||
							element.textContent?.trim().slice(0, 40)
						}`;
						// Two controls can share a label, so the position in the page tells them apart.
						const count = (seen.get(label) ?? 0) + 1;
						seen.set(label, count);
						const key = `${label} #${count}`;
						// The content may scroll inside the shell, so the scroll of each ancestor is added back.
						let top = rect.top;
						for (let node = element.parentElement; node; node = node.parentElement) {
							top += node.scrollTop;
						}
						const before = first.get(key);
						if (before === undefined) first.set(key, top);
						else if (Math.abs(top - before) > 1) moved.set(key, top - before);
					}
					if (performance.now() - started < 1500) requestAnimationFrame(sample);
					else done([...moved].map(([key, delta]) => `${key} moved ${Math.round(delta)} px`));
				};
				requestAnimationFrame(sample);
			})
	);
}

for (const { link, add, then } of EDITORS) {
	test(`ac_ui_layout_shift: no control moves after load on the ${link} editor`, async ({
		page
	}) => {
		await signIn(page);
		await page
			.getByRole('navigation', { name: 'Primary navigation' })
			.getByRole('link', { name: link, exact: true })
			.click();
		await page.getByRole('button', { name: add }).click();
		// The list page is still on screen until the editor renders, and the list is not the subject.
		await expect(page.getByRole('button', { name: 'Expand all' })).toBeVisible();
		const moved = shifts(page);
		// A preset fills the cron, so the preview of the next runs loads while the page is watched.
		if (then) await page.getByRole('button', { name: then, exact: true }).first().click();
		expect(await moved).toEqual([]);
	});
}

// An existing record loads its related lists (jobs, plans, alerts) after the record itself.
const RECORDS = [
	{ link: 'Hosts', section: 'hosts' },
	{ link: 'Profiles', section: 'profiles' },
	{ link: 'Repositories', section: 'repositories' }
];

for (const { link, section } of RECORDS) {
	test(`ac_ui_layout_shift: no control moves after load on a ${link} record`, async ({ page }) => {
		await signIn(page);
		await page
			.getByRole('navigation', { name: 'Primary navigation' })
			.getByRole('link', { name: link, exact: true })
			.click();
		await page.locator(`main a[href*="/${section}/"]`).first().click();
		await expect(page.locator('h1')).toBeVisible();
		expect(await shifts(page)).toEqual([]);
	});
}

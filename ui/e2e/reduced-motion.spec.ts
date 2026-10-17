import { expect, test, type Locator, type Page } from '@playwright/test';
import { chooseTab } from './filters';

// A spinner is a status signal: it turns with and without the reduced-motion setting, slower with
// it. A skeleton is decoration: its shine stops (design/style/ui-principles.md, 4.3).
async function signIn(page: Page): Promise<void> {
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function expectTurns(spinner: Locator, duration: string): Promise<void> {
	await expect(spinner).toHaveCSS('animation-iteration-count', 'infinite');
	await expect(spinner).toHaveCSS('animation-duration', duration);
	const transform = () => spinner.evaluate((element) => getComputedStyle(element).transform);
	const before = await transform();
	await spinner.page().waitForTimeout(500);
	expect(await transform()).not.toBe(before);
}

for (const reducedMotion of ['no-preference', 'reduce'] as const) {
	const duration = reducedMotion === 'reduce' ? '2.8s' : '1.4s';

	test.describe(`reduced motion: ${reducedMotion}`, () => {
		test.beforeEach(async ({ page }) => {
			await page.emulateMedia({ reducedMotion });
		});

		test('the spinner of a running job turns', async ({ page }) => {
			await signIn(page);
			await page
				.getByRole('navigation', { name: 'Primary navigation' })
				.getByRole('link', { name: 'Jobs', exact: true })
				.click();
			await chooseTab(page, 'Running');
			const list = page.getByRole('list', { name: 'Jobs' });
			await expectTurns(list.locator('.rh-spinner').first(), duration);
		});

		test('the spinner of a busy button turns', async ({ page }) => {
			await page.goto('/components');
			await expectTurns(page.locator('.btn__spinner').first(), duration);
		});

		test('the shine of a skeleton runs only without the setting', async ({ page }) => {
			await page.goto('/components');
			const skeleton = page.locator('.skeleton').first();
			await expect(skeleton).toBeVisible();
			const running = await skeleton.evaluate(
				(element) => element.getAnimations({ subtree: true }).length
			);
			expect(running).toBe(reducedMotion === 'reduce' ? 0 : 1);
		});
	});
}

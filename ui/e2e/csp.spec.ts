import { expect, test, type Page } from '@playwright/test';

// The production policy is `style-src 'self'` without `unsafe-inline` (ADR-0027).
// The test fails on any `securitypolicyviolation` event and on the CSP console error.

async function watch(page: Page): Promise<string[]> {
	const violations: string[] = [];
	await page.exposeFunction('__cspViolation', (detail: string) => violations.push(detail));
	await page.addInitScript(() => {
		document.addEventListener('securitypolicyviolation', (event) => {
			const target = event.target as HTMLElement | null;
			const detail = target
				? target.tagName + ' style=' + (target.getAttribute('style') ?? '')
				: 'null target';
			(window as unknown as { __cspViolation: (detail: string) => void }).__cspViolation(
				event.violatedDirective + ' | ' + detail
			);
		});
	});
	return violations;
}

test('the built page carries the strict policy', async ({ page }) => {
	const violations = await watch(page);
	const response = await page.goto('/sign-in');
	await expect(page.getByRole('heading', { level: 1 })).toBeVisible();

	const policy = response?.headers()['content-security-policy'] ?? '';
	expect(policy).toContain("style-src 'self'");
	expect(policy).not.toContain('unsafe-inline');
	expect(violations).toEqual([]);
});

test('the floating primitives need no inline style', async ({ page }) => {
	const violations = await watch(page);

	// The mock installation needs the first-run setup, then the sign-in.
	await page.goto('/setup');
	await page.locator('#setup-token').fill('token');
	await page.locator('#setup-username').fill('admin');
	await page.locator('#setup-display-name').fill('Anna Petrova');
	await page.locator('#setup-password').fill('password1234');
	await page.locator('#setup-confirm').fill('password1234');
	await page.getByRole('button', { name: 'Create the Platform Admin' }).click();

	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in' }).click();

	// The shell renders after the sign-in.
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();

	// The organization switcher is a menu; opening it mounts the floating layer.
	await page.locator('button[aria-haspopup="menu"]').first().click();
	await expect(page.getByRole('menuitemradio').first()).toBeVisible();

	expect(violations).toEqual([]);
});

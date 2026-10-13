import { expect, test, type Page } from '@playwright/test';

// The production policy is `style-src 'self'` without `unsafe-inline` (ADR-0027).
// The test fails on any `securitypolicyviolation` event and on the CSP console error.

async function watch(page: Page): Promise<string[]> {
	const violations: string[] = [];
	await page.exposeFunction('__cspViolation', (detail: string) => violations.push(detail));
	await page.addInitScript(() => {
		document.addEventListener('securitypolicyviolation', (event) => {
			(window as unknown as { __cspViolation: (detail: string) => void }).__cspViolation(
				event.violatedDirective + ' ' + event.blockedURI
			);
		});
	});
	return violations;
}

test('the sign-in page needs no unsafe-inline', async ({ page }) => {
	const violations = await watch(page);
	const response = await page.goto('/sign-in');
	await expect(page.getByRole('heading', { level: 1 })).toBeVisible();

	const policy = response?.headers()['content-security-policy'] ?? '';
	expect(policy).toContain("style-src 'self'");
	expect(policy).not.toContain('unsafe-inline');

	// The password toggle is a component; it must render without an inline style.
	await page.getByRole('button', { name: 'Show' }).click();
	expect(violations).toEqual([]);
});

test('the floating primitives need no inline style', async ({ page }) => {
	const violations = await watch(page);
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in' }).click();

	// The shell renders after the sign-in.
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();

	// The tenant switcher is a menu; opening it mounts the floating layer.
	await page.locator('button[aria-haspopup="menu"]').first().click();
	await expect(page.getByRole('menuitemradio').first()).toBeVisible();

	expect(violations).toEqual([]);
});

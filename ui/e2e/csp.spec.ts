import { expect, test, type Page } from '@playwright/test';

// The production policy is `style-src 'self'` without `unsafe-inline` (ADR-0027).
// The test fails on any `securitypolicyviolation` event and on the CSP console error.

async function watch(page: Page): Promise<string[]> {
	const violations: string[] = [];
	await page.exposeFunction('__cspViolation', (detail: string) => violations.push(detail));
	await page.addInitScript(() => {
		document.addEventListener('securitypolicyviolation', (event) => {
			// Chromium reports a violation of a style attribute in a template as a Document
			// target, so the target is not always an element.
			const target = event.target;
			const element = target instanceof Element ? target : null;
			const detail = element
				? element.tagName + ' style=' + (element.getAttribute('style') ?? '')
				: String(target);
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
	// The heading names the sign-in screen, so a redirect to the setup screen fails here.
	await expect(page.getByRole('heading', { level: 1, name: 'Sign in' })).toBeVisible();

	const policy = response?.headers()['content-security-policy'] ?? '';
	expect(policy).toContain("style-src 'self'");
	expect(policy).toContain("style-src-attr 'unsafe-hashes' 'sha256-");
	expect(policy).not.toContain('unsafe-inline');

	// The password toggle is a component; it must render without an inline style.
	const password = page
		.locator('.rh-input-affix')
		.filter({ has: page.locator('#signin-password') });
	await password.getByRole('button', { name: 'Show' }).click();
	await expect(password.locator('#signin-password')).toHaveAttribute('type', 'text');
	expect(violations).toEqual([]);
});

test('the floating primitives need no inline style', async ({ page }) => {
	const violations = await watch(page);
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();

	// The shell renders after the sign-in.
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();

	// The account menu is a bits-ui dropdown; opening it mounts the floating layer.
	await page.locator('.rh-menu__trigger').click();
	await expect(page.getByRole('menuitem', { name: 'Sign out' })).toBeVisible();

	expect(violations).toEqual([]);
});

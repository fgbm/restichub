import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Icon from './Icon.svelte';

describe('Icon', () => {
	it('renders the symbol of the sprite and hides it from assistive tech by default', () => {
		const { container } = render(Icon, { props: { name: 'i-host' } });
		const icon = container.querySelector('svg.icon');
		expect(icon).not.toBeNull();
		expect(icon).toHaveAttribute('aria-hidden', 'true');
		expect(icon?.querySelector('use')).toHaveAttribute('href', '/icons.svg#i-host');
	});

	it('carries a label as an img role when one is given', () => {
		render(Icon, { props: { name: 'i-refresh', label: 'Refresh' } });
		const icon = screen.getByRole('img', { name: 'Refresh' });
		expect(icon).not.toHaveAttribute('aria-hidden');
	});

	it('applies the size class', () => {
		const { container } = render(Icon, { props: { name: 'i-x', size: 'xs' } });
		expect(container.querySelector('svg.icon')).toHaveClass('icon--xs');
	});
});

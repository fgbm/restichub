import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import DropdownMenu from './DropdownMenu.svelte';
import type { MenuItem } from './types';

function trigger(value: string): Snippet {
	return createRawSnippet(() => ({ render: () => `<span>${value}</span>` }));
}

describe('DropdownMenu', () => {
	it('opens on the trigger and calls the item action', async () => {
		const user = userEvent.setup();
		const onSelect = vi.fn();
		const items: MenuItem[] = [
			{ label: 'Edit', onSelect },
			{ label: 'Delete', tone: 'danger', separatorBefore: true }
		];
		render(DropdownMenu, { props: { items, children: trigger('Actions') } });

		const button = screen.getByRole('button', { name: 'Actions' });
		expect(button).toHaveAttribute('aria-expanded', 'false');

		await user.click(button);
		const item = await screen.findByText('Edit');
		expect(item).toHaveAttribute('role', 'menuitem');
		await user.click(item);
		expect(onSelect).toHaveBeenCalledOnce();
	});

	it('shows a hint below the label and keeps the name of the item', async () => {
		const user = userEvent.setup();
		const items: MenuItem[] = [
			{ label: 'About ResticHub', hint: '0.2.0-rc.2', onSelect: () => {} }
		];
		render(DropdownMenu, { props: { items, children: trigger('Account') } });

		await user.click(screen.getByRole('button', { name: 'Account' }));
		const hint = await screen.findByText('0.2.0-rc.2');
		expect(hint.closest('[role="menuitem"]')).toHaveTextContent('About ResticHub');
	});
});

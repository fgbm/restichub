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

	it('the_about_item_shows_the_version_in_the_label_row', async () => {
		const user = userEvent.setup();
		const items: MenuItem[] = [
			{ label: 'О ResticHub', icon: 'info', trailing: '0.2.0-rc.2', onSelect: () => {} }
		];
		render(DropdownMenu, { props: { items, children: trigger('Account') } });

		await user.click(screen.getByRole('button', { name: 'Account' }));
		const label = await screen.findByText('О ResticHub');
		const item = label.closest('[role="menuitem"]')!;
		expect(item).not.toBeNull();
		const version = screen.getByText('0.2.0-rc.2');

		// The version sits in the row of the label, after the label text.
		const row = version.parentElement;
		expect(row).toHaveClass('rh-menu__row');
		const labelNode = Array.from(row?.childNodes ?? []).find(
			(node) => node.textContent?.trim() === 'О ResticHub'
		);
		expect(labelNode).toBeDefined();
		expect(
			labelNode!.compareDocumentPosition(version) & Node.DOCUMENT_POSITION_FOLLOWING
		).toBeTruthy();

		// No second line holds the version, so the item has no column of two lines.
		expect(item.querySelector('.rh-menu__hint')).toBeNull();
		expect(item.querySelector('.rh-menu__text')).toBeNull();

		// The name of the item comes from its content, which holds the label once and the version
		// once. jsdom gives no accessible name for the bits-ui menuitem, so the content is asserted.
		const name = item.textContent?.replace(/\s+/g, ' ').trim() ?? '';
		expect(name.split('О ResticHub')).toHaveLength(2);
		expect(name.split('0.2.0-rc.2')).toHaveLength(2);
		expect(name).toBe('О ResticHub 0.2.0-rc.2');
	});
});

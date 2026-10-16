import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import RowActions from './RowActions.svelte';
import type { MenuItem } from './types';

const item = (label: string, onSelect = vi.fn()): MenuItem => ({ label, icon: 'eye', onSelect });

describe('RowActions', () => {
	it('shows nothing for a row without an action', () => {
		const { container } = render(RowActions, { props: { items: [], moreLabel: 'More' } });
		expect(container.querySelector('button')).toBeNull();
	});

	it('shows one or two actions as icon buttons with a tooltip and an accessible name', async () => {
		const user = userEvent.setup();
		const open = vi.fn();
		render(RowActions, {
			props: { items: [item('Open', open), item('Copy')], moreLabel: 'More' }
		});
		const button = screen.getByRole('button', { name: 'Open' });
		expect(button).toHaveAttribute('title', 'Open');
		expect(screen.queryByRole('button', { name: 'More' })).not.toBeInTheDocument();
		await user.click(button);
		expect(open).toHaveBeenCalledOnce();
	});

	it('shows three actions or more behind one … button only', () => {
		render(RowActions, {
			props: { items: [item('Open'), item('Copy'), item('Delete')], moreLabel: 'More' }
		});
		expect(screen.getByRole('button', { name: 'More' })).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Open' })).not.toBeInTheDocument();
	});
});

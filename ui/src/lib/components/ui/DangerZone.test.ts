import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import DangerZone from './DangerZone.svelte';
import type { DangerAction } from './types';

function action(overrides: Partial<DangerAction> = {}): DangerAction {
	return {
		key: 'delete',
		text: 'The plan stops. The jobs stay.',
		label: 'Delete plan',
		icon: 'trash',
		tone: 'danger',
		onclick: vi.fn(),
		...overrides
	};
}

describe('DangerZone', () => {
	it('shows the sentence of each action before its button', () => {
		const { container } = render(DangerZone, {
			actions: [
				action(),
				action({ key: 'export', text: 'The export is removed.', label: 'Delete export' })
			]
		});
		const items = container.querySelectorAll('li');
		expect(items).toHaveLength(2);
		expect(items[0]).toHaveTextContent('The plan stops. The jobs stay.');
		expect(items[0]).toHaveTextContent('Delete plan');
		expect(items[1]).toHaveTextContent('The export is removed.');
		expect(screen.getByRole('button', { name: 'Delete export' })).toHaveClass('btn--danger');
	});

	it('runs the action and shows the reason when it is not available', async () => {
		const user = userEvent.setup();
		const onclick = vi.fn();
		render(DangerZone, {
			actions: [
				action({ onclick }),
				action({
					key: 'self',
					label: 'Disable user',
					tone: 'secondary',
					disabled: true,
					hint: 'You cannot disable your own account'
				})
			]
		});
		await user.click(screen.getByRole('button', { name: 'Delete plan' }));
		expect(onclick).toHaveBeenCalledTimes(1);
		const disabled = screen.getByRole('button', { name: 'Disable user' });
		expect(disabled).toBeDisabled();
		expect(disabled).toHaveAttribute('title', 'You cannot disable your own account');
		expect(disabled).toHaveClass('btn--secondary');
	});
});

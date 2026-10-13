import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import Switch from './Switch.svelte';

describe('Switch', () => {
	it('exposes the switch role with the label and toggles the state', async () => {
		const user = userEvent.setup();
		render(Switch, { props: { label: 'Active', checked: false } });

		const control = screen.getByRole('switch', { name: 'Active' });
		expect(control).toHaveAttribute('data-state', 'unchecked');

		await user.click(control);
		expect(control).toHaveAttribute('data-state', 'checked');
	});
});

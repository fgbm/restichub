import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import SwitchField from './SwitchField.svelte';

describe('SwitchField', () => {
	it('names the switch by the label, shows the help and toggles', async () => {
		const user = userEvent.setup();
		render(SwitchField, { props: { label: 'Send alerts', help: 'No alert when off.' } });

		const control = screen.getByRole('switch', { name: 'Send alerts' });
		expect(screen.getByText('No alert when off.')).toBeInTheDocument();
		expect(control).toHaveAttribute('data-state', 'unchecked');
		await user.click(control);
		expect(control).toHaveAttribute('data-state', 'checked');
	});
});

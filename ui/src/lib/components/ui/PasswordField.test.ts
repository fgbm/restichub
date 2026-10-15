import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import PasswordField from './PasswordField.svelte';

describe('PasswordField', () => {
	it('hides the value and reveals it on the toggle', async () => {
		const user = userEvent.setup();
		render(PasswordField, { props: { label: 'Password', value: 'secret' } });

		const input = screen.getByLabelText('Password');
		expect(input).toHaveAttribute('type', 'password');

		await user.click(screen.getByRole('button', { name: 'Show' }));
		expect(input).toHaveAttribute('type', 'text');

		await user.click(screen.getByRole('button', { name: 'Hide' }));
		expect(input).toHaveAttribute('type', 'password');
	});

	it('keeps the base class next to a class of the caller', () => {
		render(PasswordField, { props: { label: 'Password', class: 'extra' } });
		expect(screen.getByLabelText('Password')).toHaveClass('rh-input', 'extra');
	});
});

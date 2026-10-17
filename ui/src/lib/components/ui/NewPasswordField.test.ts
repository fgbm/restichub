import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import NewPasswordField from './NewPasswordField.svelte';

function ruleOf(input: HTMLElement): HTMLElement {
	const id = input.getAttribute('aria-describedby');
	const rule = id ? document.getElementById(id) : null;
	expect(rule).not.toBeNull();
	return rule as HTMLElement;
}

describe('NewPasswordField', () => {
	it('shows the rule of the server for an empty field', () => {
		render(NewPasswordField, { props: { label: 'Password', id: 'pw' } });
		const input = screen.getByLabelText(/^Password/);
		expect(ruleOf(input)).toHaveTextContent('At least 12 characters.');
		expect(ruleOf(input)).toHaveClass('rh-field__help');
		expect(input).not.toHaveAttribute('aria-invalid');
		expect(input).toHaveAttribute('autocomplete', 'new-password');
	});

	it('shows the progress in the normal tone while the person types', async () => {
		const user = userEvent.setup();
		render(NewPasswordField, { props: { label: 'Password', id: 'pw' } });
		const input = screen.getByLabelText(/^Password/);
		await user.type(input, 'abcdef');
		expect(ruleOf(input)).toHaveTextContent('6 of 12 characters');
		expect(ruleOf(input)).toHaveClass('rh-field__help');
		expect(input).not.toHaveAttribute('aria-invalid');
	});

	it('says that a long enough password is enough, with a check icon', async () => {
		const user = userEvent.setup();
		render(NewPasswordField, { props: { label: 'Password', id: 'pw' } });
		const input = screen.getByLabelText(/^Password/);
		await user.type(input, 'abcdefghijklm');
		expect(ruleOf(input)).toHaveTextContent('13 characters — long enough');
		expect(ruleOf(input).querySelector('svg')).not.toBeNull();
		await user.tab();
		expect(input).not.toHaveAttribute('aria-invalid');
	});

	it('turns a short password into an error after the person leaves the field', async () => {
		const user = userEvent.setup();
		render(NewPasswordField, { props: { label: 'Password', id: 'pw' } });
		const input = screen.getByLabelText(/^Password/);
		await user.type(input, 'abcdef');
		await user.tab();
		expect(input).toHaveAttribute('aria-invalid', 'true');
		expect(ruleOf(input)).toHaveClass('rh-field__error');
		expect(ruleOf(input)).toHaveTextContent('6 of 12 characters');
	});

	it('keeps an empty field quiet on blur and shows the rule as an error after a submit', async () => {
		const user = userEvent.setup();
		const { rerender } = render(NewPasswordField, { props: { label: 'Password', id: 'pw' } });
		const input = screen.getByLabelText(/^Password/);
		await user.click(input);
		await user.tab();
		expect(input).not.toHaveAttribute('aria-invalid');

		await rerender({ label: 'Password', id: 'pw', submitted: true });
		expect(input).toHaveAttribute('aria-invalid', 'true');
		expect(ruleOf(input)).toHaveTextContent('At least 12 characters.');
	});

	it('shows an error of the server in place of the rule', () => {
		render(NewPasswordField, {
			props: { label: 'Password', id: 'pw', value: 'abcdefghijkl', error: 'Refused.' }
		});
		const input = screen.getByLabelText(/^Password/);
		expect(input).toHaveAttribute('aria-invalid', 'true');
		expect(ruleOf(input)).toHaveTextContent('Refused.');
	});

	it('refuses a password above the byte limit at once', () => {
		render(NewPasswordField, { props: { label: 'Password', id: 'pw', value: 'a'.repeat(1025) } });
		const input = screen.getByLabelText(/^Password/);
		expect(input).toHaveAttribute('aria-invalid', 'true');
		expect(ruleOf(input)).toHaveTextContent('The password is too long: 1024 bytes at most.');
	});
});

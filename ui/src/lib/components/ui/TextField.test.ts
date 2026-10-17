import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import TextField from './TextField.svelte';

describe('TextField', () => {
	it('ties the label to the input', () => {
		render(TextField, { props: { label: 'Name', value: 'host-01' } });
		expect(screen.getByLabelText('Name')).toHaveValue('host-01');
	});

	it('describes the input with the help text', () => {
		render(TextField, { props: { label: 'Name', help: 'Lower case only' } });
		expect(screen.getByLabelText('Name')).toHaveAccessibleDescription('Lower case only');
	});

	it('marks the input invalid and describes it with the error', () => {
		render(TextField, { props: { label: 'Name', error: 'Required' } });
		const input = screen.getByLabelText('Name');
		expect(input).toHaveAttribute('aria-invalid', 'true');
		expect(input).toHaveAccessibleDescription('Required');
	});

	it('keeps the base class next to a class of the caller', () => {
		render(TextField, { props: { label: 'Name', class: 'extra' } });
		expect(screen.getByLabelText('Name')).toHaveClass('rh-input', 'extra');
	});

	it('shows the example placeholder in an editable field', () => {
		render(TextField, { props: { label: 'Prefix', placeholder: 'e.g. snapshots/prod' } });
		expect(screen.getByLabelText('Prefix')).toHaveAttribute('placeholder', 'e.g. snapshots/prod');
		expect(screen.queryByText('Not set')).not.toBeInTheDocument();
	});

	it.each([{ disabled: true }, { readonly: true }])(
		'says "Not set" and drops the placeholder in an empty locked field (%o)',
		(lock) => {
			render(TextField, {
				props: { label: 'Prefix', placeholder: 'e.g. snapshots/prod', ...lock }
			});
			const input = screen.getByLabelText('Prefix');
			expect(input).toHaveValue('');
			expect(input).not.toHaveAttribute('placeholder');
			expect(screen.getByText('Not set')).toBeInTheDocument();
			expect(input).toHaveAccessibleDescription('Not set');
		}
	);

	it('shows the stored value in a locked field', () => {
		render(TextField, {
			props: { label: 'Prefix', placeholder: 'e.g. snapshots/prod', value: 'nas', disabled: true }
		});
		const input = screen.getByLabelText('Prefix');
		expect(input).toHaveValue('nas');
		expect(input).not.toHaveAttribute('placeholder');
		expect(screen.queryByText('Not set')).not.toBeInTheDocument();
	});
});

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
});

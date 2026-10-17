import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import type { Me } from '#lib/api/index.ts';
import SectionsProbe from '#lib/test/SectionsProbe.svelte';
import { sectionsFrom } from './section-state.svelte.ts';
import { setSession } from '#lib/stores/session.svelte.ts';

const KEY = 'restichub.v1.t_01.u_01.editor.probe.sections';
const DEFAULTS = { basics: true, paths: true, tags: false };

beforeEach(() => {
	setSession({ user: { id: 'u_01' }, tenant: { id: 't_01' }, role: 'admin' } as Me);
});

describe('ac_ui_10_editor_sections_are_kept', () => {
	it('uses the defaults when nothing is kept', () => {
		expect(sectionsFrom(['basics', 'paths', 'tags'], DEFAULTS, null)).toEqual(DEFAULTS);
	});

	it('opens a section that the kept list does not name, and ignores an unknown id', () => {
		expect(sectionsFrom(['basics', 'tags'], DEFAULTS, ['basics', 'gone'])).toEqual({
			basics: false,
			tags: true
		});
	});

	it('keeps the closed sections of the kind and applies them to the next record', async () => {
		const user = userEvent.setup();
		const first = render(SectionsProbe);
		expect(localStorage.length).toBe(0);
		await user.click(screen.getByLabelText('paths'));
		await user.click(screen.getByLabelText('tags'));
		expect(JSON.parse(localStorage.getItem(KEY) ?? 'null')).toEqual(['paths']);
		first.unmount();

		render(SectionsProbe);
		expect(screen.getByLabelText('basics')).toBeChecked();
		expect(screen.getByLabelText('paths')).not.toBeChecked();
		expect(screen.getByLabelText('tags')).toBeChecked();
	});

	it('keeps the kinds apart', async () => {
		const user = userEvent.setup();
		render(SectionsProbe, { kind: 'plan' });
		await user.click(screen.getByLabelText('basics'));
		expect(localStorage.getItem(KEY)).toBeNull();
		expect(localStorage.getItem('restichub.v1.t_01.u_01.editor.plan.sections')).not.toBeNull();
	});

	it('ignores a kept value that is not a list', () => {
		localStorage.setItem(KEY, '"x"');
		render(SectionsProbe);
		expect(screen.getByLabelText('basics')).toBeChecked();
		expect(localStorage.getItem(KEY)).toBeNull();
	});
});

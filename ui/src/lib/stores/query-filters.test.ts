/*
 * The list view that the UI keeps and shows again (FR-UI-05). The probe page calls `queryFilters`
 * the way each list page does.
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Me } from '#lib/api/index.ts';
import FiltersProbe from '#lib/test/FiltersProbe.svelte';
import { clearSession, setSession } from './session.svelte.ts';

const route = vi.hoisted(() => ({
	url: new URL('http://localhost/probe'),
	route: { id: '/(app)/probe' }
}));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/navigation', () => ({
	goto: (address: string) => {
		route.url = new URL(address, 'http://localhost');
	}
}));

const KEY = 'restichub.v1.t_01.u_01.list./(app)/probe';

function signIn(user: string, tenant = 't_01'): void {
	setSession({ user: { id: user }, tenant: { id: tenant }, role: 'admin' } as Me);
}

function shown(): Record<string, string> {
	return JSON.parse(screen.getByTestId('state').textContent ?? '{}');
}

beforeEach(() => {
	route.url = new URL('http://localhost/probe');
	signIn('u_01');
});

describe('ac_ui_04_lists_keep_the_view', () => {
	it('applies the kept tab, sort and page size and writes them into the address', async () => {
		const user = userEvent.setup();
		const first = render(FiltersProbe);
		await user.click(screen.getByRole('button', { name: 'view' }));
		await user.click(screen.getByRole('button', { name: 'host' }));
		first.unmount();

		route.url = new URL('http://localhost/probe');
		render(FiltersProbe);

		expect(shown()).toMatchObject({
			state: 'failed',
			size: '50',
			sort: 'host',
			order: 'desc',
			host: 'h2'
		});
		expect(screen.getByTestId('restored')).toHaveTextContent('true');
		await waitFor(() => expect(route.url.searchParams.get('state')).toBe('failed'));
		expect(route.url.searchParams.get('size')).toBe('50');
		expect(route.url.searchParams.get('host')).toBe('h2');
	});

	it('opens with the defaults when nothing is kept', () => {
		render(FiltersProbe);
		expect(shown().state).toBe('');
		expect(screen.getByTestId('restored')).toHaveTextContent('false');
	});
});

describe('ac_ui_05_link_wins_over_kept_view', () => {
	it('uses the parameters of the link and leaves the kept view as it is', () => {
		const kept = JSON.stringify({ state: 'failed', size: '50' });
		localStorage.setItem(KEY, kept);
		route.url = new URL('http://localhost/probe?host=h1');
		render(FiltersProbe);

		expect(shown()).toMatchObject({ host: 'h1', state: '', size: '' });
		expect(screen.getByTestId('restored')).toHaveTextContent('false');
		expect(localStorage.getItem(KEY)).toBe(kept);
	});
});

describe('ac_ui_06_reset_clears_the_kept_filters', () => {
	it('keeps no filter after the reset and keeps the view', async () => {
		const user = userEvent.setup();
		localStorage.setItem(KEY, JSON.stringify({ state: 'failed', host: 'h2', size: '50' }));
		render(FiltersProbe);
		await user.click(screen.getByRole('button', { name: 'Reset' }));

		expect(JSON.parse(localStorage.getItem(KEY) ?? '{}')).toEqual({ size: '50' });
	});
});

describe('ac_ui_07_stale_kept_values_are_dropped', () => {
	it('drops an unknown tab, sort and id and applies the rest', async () => {
		localStorage.setItem(
			KEY,
			JSON.stringify({ state: 'old-tab', sort: 'gone', host: 'h9', size: '50' })
		);
		render(FiltersProbe);

		await waitFor(() => expect(shown()).toMatchObject({ state: '', sort: '', host: '' }));
		expect(shown().size).toBe('50');
		expect(JSON.parse(localStorage.getItem(KEY) ?? '{}')).toEqual({ size: '50' });
	});

	it('keeps the id while the list of ids is not loaded', () => {
		localStorage.setItem(KEY, JSON.stringify({ host: 'h9' }));
		render(FiltersProbe, { hosts: [], hostsLoaded: false });
		expect(shown().host).toBe('h9');
	});

	it('removes a kept view that is not a map of texts', () => {
		localStorage.setItem(KEY, '[1,2]');
		render(FiltersProbe);
		expect(shown().state).toBe('');
		expect(localStorage.getItem(KEY)).toBeNull();
	});
});

describe('ac_ui_08_kept_view_belongs_to_one_user', () => {
	it('shows nothing of another user or another tenant', async () => {
		const user = userEvent.setup();
		const first = render(FiltersProbe);
		await user.click(screen.getByRole('button', { name: 'view' }));
		first.unmount();

		signIn('u_02');
		route.url = new URL('http://localhost/probe');
		const second = render(FiltersProbe);
		expect(shown().state).toBe('');
		second.unmount();

		signIn('u_01', 't_02');
		route.url = new URL('http://localhost/probe');
		render(FiltersProbe);
		expect(shown().state).toBe('');
	});

	it('keeps nothing without a user', async () => {
		const user = userEvent.setup();
		clearSession();
		render(FiltersProbe);
		await user.click(screen.getByRole('button', { name: 'view' }));
		expect(localStorage.length).toBe(0);
	});
});

describe('ac_ui_12_search_and_page_are_not_kept', () => {
	it('keeps neither the search text nor the page', async () => {
		const user = userEvent.setup();
		render(FiltersProbe);
		await user.click(screen.getByRole('button', { name: 'view' }));
		await user.click(screen.getByRole('button', { name: 'search' }));

		const record = JSON.parse(localStorage.getItem(KEY) ?? '{}');
		expect(record).not.toHaveProperty('q');
		expect(record).not.toHaveProperty('page');
		expect(record.state).toBe('failed');
	});
});

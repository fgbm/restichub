/*
 * The About ResticHub screen shows the version, the revision and the protocol versions to every
 * role, and the installation facts to the Platform Admin only (AC-AU-02).
 */
import { render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { About } from '#lib/api/index.ts';
import AboutPage from './+page.svelte';

const state = vi.hoisted(() => ({ about: null as About | null }));

vi.mock('#lib/stores/about.svelte.ts', () => ({
	getAbout: () => state.about,
	loadAbout: () => Promise.resolve()
}));

vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

function about(overrides: Partial<About> = {}): About {
	return {
		version: '0.2.0',
		prerelease: false,
		revision: '3b21922',
		protocol: { major: 1, minor: 1 },
		agent_protocol: { major: 1, minimum_minor: 0 },
		...overrides
	};
}

beforeEach(() => {
	state.about = about();
});

describe('the About ResticHub screen', () => {
	it('ac_au_02_shows_the_version_and_the_protocol_of_the_control_plane', () => {
		render(AboutPage);
		expect(screen.getByRole('heading', { level: 1, name: 'About ResticHub' })).toBeInTheDocument();
		expect(screen.getByText('0.2.0')).toBeInTheDocument();
		expect(screen.getByText('3b21922')).toBeInTheDocument();
		expect(screen.getByText('1.1')).toBeInTheDocument();
		expect(screen.getByText('1.0 and later')).toBeInTheDocument();
		expect(screen.queryByText('Pre-release')).not.toBeInTheDocument();
		expect(screen.queryByText('Database schema level')).not.toBeInTheDocument();
	});

	it('ac_au_02_marks_a_pre_release_build', () => {
		state.about = about({ prerelease: true, version: '0.2.0-rc.2' });
		render(AboutPage);
		expect(screen.getByText('Pre-release')).toBeInTheDocument();
		expect(screen.getByText('0.2.0-rc.2')).toBeInTheDocument();
	});

	it('ac_au_02_shows_the_installation_facts_of_the_platform_admin', () => {
		state.about = about({ tenancy_mode: 'multi', schema_level: 60 });
		render(AboutPage);
		expect(screen.getByText('Several organizations')).toBeInTheDocument();
		expect(screen.getByText('60')).toBeInTheDocument();
	});
});

/*
 * The job page of a restore shows the parameters that the operator chose (FR-RS-03, FR-RS-04).
 */
import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import RestoreParams from './RestoreParams.svelte';

const restore = {
	snapshot_id: 'a1b2c3d4'.padEnd(64, '0'),
	includes: [] as string[],
	excludes: [] as string[],
	target: '/var/lib/restichub-restore/a1b2c3d4',
	overwrite: 'always' as const,
	dry_run: false,
	verify: true
};

describe('RestoreParams', () => {
	it('fr_rs_03_lists_the_snapshot_the_host_the_target_and_the_flags', () => {
		render(RestoreParams, { restore, hostname: 'srv-db-01' });
		expect(screen.getByText('Restore parameters')).toBeVisible();
		expect(screen.getByText('a1b2c3d4')).toHaveAttribute('title', restore.snapshot_id);
		expect(screen.getByText('srv-db-01')).toBeVisible();
		expect(screen.getByText('/var/lib/restichub-restore/a1b2c3d4')).toBeVisible();
		expect(screen.getByText('The whole snapshot')).toBeVisible();
		expect(screen.getByText('Replace every file')).toBeVisible();
		expect(screen.queryByText('Excluded paths')).toBeNull();
		const verify = screen.getByText('Verify').nextElementSibling;
		const dryRun = screen.getByText('Dry run').nextElementSibling;
		expect(verify).toHaveTextContent('Yes');
		expect(dryRun).toHaveTextContent('No');
	});

	it('fr_rs_03_lists_the_included_and_excluded_paths', () => {
		render(RestoreParams, {
			restore: { ...restore, includes: ['/srv/data', '/etc/nginx'], excludes: ['*.tmp'] },
			hostname: null
		});
		expect(screen.getByText('/srv/data')).toBeVisible();
		expect(screen.getByText('/etc/nginx')).toBeVisible();
		expect(screen.queryByText('The whole snapshot')).toBeNull();
		expect(screen.getByText('Excluded paths')).toBeVisible();
		expect(screen.getByText('*.tmp')).toBeVisible();
	});
});

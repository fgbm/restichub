import { describe, expect, it } from 'vitest';
import type { Repository } from '#lib/api/index.ts';
import {
	repositoryAddress,
	repositoryCount,
	repositoryDescription,
	repositoryLocation,
	repositoryOptions,
	withRepositories
} from './repository-picker';

function repository(overrides: Partial<Repository> = {}): Repository {
	return {
		id: 'r1',
		name: 'yandex-prod',
		backend: { kind: 's3', endpoint: 'storage.yandexcloud.net', bucket: 'rh-backups' },
		credentials: { password: true, s3_access_key: true, s3_secret_key: true, rest_password: false },
		initialized: true,
		maintenance_agent_id: null,
		created_at: '2026-01-01T00:00:00Z',
		updated_at: '2026-01-01T00:00:00Z',
		lock_state: 'free',
		shares_location_with: [],
		...overrides
	} as Repository;
}

const agents = [{ id: 'a1', hostname: 'fs-01' }];

describe('the repository picker options', () => {
	it('names the backend kind and the location of each storage', () => {
		expect(repositoryDescription(repository(), agents)).toEqual([
			{ tag: 'S3' },
			'storage.yandexcloud.net/rh-backups'
		]);
		expect(
			repositoryDescription(
				repository({ backend: { kind: 'rest', endpoint: 'https://rest.example.net' } }),
				agents
			)
		).toEqual([{ tag: 'REST' }, 'https://rest.example.net']);
		expect(
			repositoryDescription(
				repository({ backend: { kind: 'local', path: '/srv/restic/nas' } }),
				agents
			)
		).toEqual([{ tag: 'Local' }, '/srv/restic/nas', 'no host']);
	});

	it('names the host of a local path', () => {
		const local = repository({
			backend: { kind: 'local', path: '/mnt/backup' },
			maintenance_agent_id: 'a1'
		});
		expect(repositoryLocation(local)).toBe('/mnt/backup');
		expect(repositoryDescription(local, agents)).toEqual([
			{ tag: 'Local' },
			'/mnt/backup',
			'host fs-01'
		]);
		expect(repositoryOptions([local], agents)[0].detail).toBe('Local · /mnt/backup · fs-01');
	});

	it('shows the kind and the location in the closed field', () => {
		expect(repositoryOptions([repository()], agents)).toEqual([
			{
				value: 'r1',
				label: 'yandex-prod',
				description: [{ tag: 'S3' }, 'storage.yandexcloud.net/rh-backups'],
				detail: 'S3 · storage.yandexcloud.net/rh-backups',
				disabled: false,
				reason: undefined
			}
		]);
	});

	it('keeps a repository without an init step in the list with the reason', () => {
		const options = repositoryOptions([repository({ initialized: false })], agents);
		expect(options[0]).toMatchObject({
			disabled: true,
			reason: 'Not initialized. Open the repository and start the initialization.'
		});
	});

	it('gives the facet values the repository line', () => {
		expect(
			withRepositories(
				[
					{ value: 'r1', count: 2 },
					{ value: 'r9', count: 1 }
				],
				[repository()],
				agents
			)
		).toEqual([
			{
				value: 'r1',
				count: 2,
				label: 'yandex-prod',
				description: [{ tag: 'S3' }, 'storage.yandexcloud.net/rh-backups']
			},
			{ value: 'r9', count: 1 }
		]);
	});

	it('counts the repositories', () => {
		expect(repositoryCount(1)).toBe('1 repository');
		expect(repositoryCount(4)).toBe('4 repositories');
	});
});

describe('the restic address of a repository', () => {
	it('takes the scheme from the protocol of the backend', () => {
		expect(repositoryAddress(repository())).toBe('s3:https://storage.yandexcloud.net/rh-backups');
		expect(
			repositoryAddress(
				repository({
					backend: {
						kind: 's3',
						endpoint: 'https://s3.example.net',
						bucket: 'b',
						prefix: 'snapshots/prod'
					}
				})
			)
		).toBe('s3:https://s3.example.net/b/snapshots/prod');
		expect(
			repositoryAddress(
				repository({ backend: { kind: 'rest', endpoint: 'https://rest.example.net' } })
			)
		).toBe('rest:https://rest.example.net');
		expect(repositoryAddress(repository({ backend: { kind: 'local', path: '/srv/restic' } }))).toBe(
			'/srv/restic'
		);
	});
});

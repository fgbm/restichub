import { describe, expect, it } from 'vitest';
import { profileOptions } from './profile-picker';

const repositories = [
	{ id: 'r1', name: 'test-s3-yandex', backend: { kind: 's3', bucket: 'b' } },
	{ id: 'r2', name: 'rest-main', backend: { kind: 'rest', endpoint: 'https://rest.example.net' } },
	{
		id: 'r3',
		name: 'nas-local',
		backend: { kind: 'local', path: '/srv/restic' },
		maintenance_agent_id: 'a1'
	},
	{ id: 'r4', name: 'usb-local', backend: { kind: 'local', path: '/mnt/usb' } }
] as Parameters<typeof profileOptions>[1];
const agents = [{ id: 'a1', hostname: 'MINDCORE' }];

describe('profileOptions', () => {
	it('gives each profile its repository, the backend tag and the host of a local path', () => {
		const options = profileOptions(
			[
				{ id: 'p1', name: 'user-profiles', repository_id: 'r1' },
				{ id: 'p2', name: 'db-hourly', repository_id: 'r2' },
				{ id: 'p3', name: 'nas-files', repository_id: 'r3' },
				{ id: 'p4', name: 'usb-files', repository_id: 'r4' }
			],
			repositories,
			agents
		);

		expect(options).toEqual([
			{ value: 'p1', label: 'user-profiles', description: ['test-s3-yandex', { tag: 'S3' }] },
			{ value: 'p2', label: 'db-hourly', description: ['rest-main', { tag: 'REST' }] },
			{
				value: 'p3',
				label: 'nas-files',
				description: ['nas-local', { tag: 'Local' }, 'host MINDCORE']
			},
			{ value: 'p4', label: 'usb-files', description: ['usb-local', { tag: 'Local' }, 'no host'] }
		]);
	});

	it('leaves the secondary line empty when the repository is unknown', () => {
		expect(
			profileOptions([{ id: 'p1', name: 'orphan', repository_id: 'r9' }], repositories, agents)
		).toEqual([{ value: 'p1', label: 'orphan', description: [] }]);
	});
});

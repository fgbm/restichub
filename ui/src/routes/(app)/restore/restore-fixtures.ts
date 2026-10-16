/* Sample data that the tests of both Restore pages share. */

export const repository = {
	id: 'r_1',
	name: 'offsite-s3',
	backend: { kind: 's3', endpoint: 's3.example.net', bucket: 'b' },
	credentials: { password: true, s3_access_key: true, s3_secret_key: true, rest_password: false },
	initialized: true,
	maintenance_agent_id: 'a_1',
	created_at: '2026-10-01T00:00:00Z',
	updated_at: '2026-10-01T00:00:00Z'
};

export const agent = {
	id: 'a_1',
	hostname: 'nas-backup',
	os: 'linux',
	arch: 'amd64',
	agent_version: '0.4.2',
	restic_version: '0.19.0',
	labels: [],
	restore_roots: ['/var/lib/restichub-restore'],
	status: 'active',
	last_seen_at: '2026-10-15T00:00:00Z',
	created_at: '2026-09-01T00:00:00Z',
	certificate_expires_at: '2027-09-01T00:00:00Z',
	certificate_serial: '0f01',
	revoked_at: null
};

export function snapshot(id: string, host: string) {
	return {
		id: id.padEnd(64, '0'),
		short_id: id,
		time: '2026-10-14T12:00:00Z',
		hostname: host,
		username: 'root',
		tags: ['daily'],
		paths: ['/srv'],
		program_version: 'restic 0.18.1',
		total_files: 10,
		total_bytes: 1024
	};
}

export function listing(state: string, snapshots: ReturnType<typeof snapshot>[] = []) {
	return {
		state,
		job_id: 'b_1',
		agent_id: 'a_1',
		fetched_at: '2026-10-15T00:00:00Z',
		age_seconds: 0,
		truncated: false,
		hosts: [],
		tags: [],
		paths: [],
		error_code: null,
		snapshots
	};
}

export function node(name: string, kind: 'dir' | 'file' | 'symlink', parent = '') {
	return {
		name,
		kind,
		path: `${parent}/${name}`,
		size: kind === 'file' ? 2048 : 0,
		permissions: kind === 'dir' ? 'drwxr-xr-x' : '-rw-r--r--',
		mtime: '2026-10-14T12:00:00Z',
		link_target: kind === 'symlink' ? 'target-dir' : ''
	};
}

export function treePage(nodes: ReturnType<typeof node>[], path = '') {
	return {
		state: 'done',
		job_id: 'l_1',
		snapshot_id: 'a1b2c3d4'.padEnd(64, '0'),
		path,
		offset: 0,
		limit: 1000,
		fetched_at: '2026-10-15T00:00:00Z',
		cached: false,
		last: true,
		next_offset: null,
		error_code: null,
		nodes
	};
}

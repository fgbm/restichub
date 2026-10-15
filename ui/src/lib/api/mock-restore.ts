/*
 * Sample data of the Restore screen for the mock service: the snapshots of a repository and a
 * snapshot tree with a large directory (FR-RS-01, FR-RS-02). Development and tests use it only.
 */
import type { LsView, NodeItem, SnapshotItem, SnapshotsQuery } from './types';

const HOUR = 3_600_000;

/** The entries of the large directory, so a test pages through more than one page. */
export const LARGE_DIRECTORY = '/srv/www/uploads';
export const LARGE_DIRECTORY_SIZE = 2_500;

function snapshot(
	index: number,
	hostname: string,
	tags: string[],
	paths: string[],
	hoursAgo: number
): SnapshotItem {
	const id = `${(0xa1b2c3d4 + index * 0x1111).toString(16)}${'0'.repeat(56)}`.slice(0, 64);
	return {
		id,
		short_id: id.slice(0, 8),
		time: new Date(Date.now() - hoursAgo * HOUR).toISOString(),
		hostname,
		username: 'root',
		tags,
		paths,
		program_version: 'restic 0.18.1',
		total_files: 128_402 - index * 311,
		total_bytes: 52_917_000_000 - index * 41_000_000
	};
}

/** The snapshots of a repository, the newest first. */
export function makeSnapshots(): SnapshotItem[] {
	return [
		snapshot(0, 'nas-backup', ['daily'], ['/srv', '/etc'], 3),
		snapshot(1, 'ws-anna', ['daily'], ['/home/anna'], 9),
		snapshot(2, 'nas-backup', ['daily'], ['/srv', '/etc'], 27),
		snapshot(3, 'nas-backup', ['weekly', 'monthly'], ['/srv'], 80),
		snapshot(4, 'ws-anna', ['daily'], ['/home/anna'], 33),
		snapshot(5, 'ws-boris', ['laptop'], ['/home/boris/projects'], 120),
		snapshot(6, 'nas-backup', ['weekly'], ['/srv', '/etc'], 200)
	].sort((a, b) => (b.time ?? '').localeCompare(a.time ?? ''));
}

/** The snapshots that match the filters. restic applies them, so the mock does the same. */
export function filterSnapshots(
	snapshots: SnapshotItem[],
	query: Partial<SnapshotsQuery>
): SnapshotItem[] {
	const hosts = query.hosts ?? [];
	const tags = query.tags ?? [];
	const paths = query.paths ?? [];
	return snapshots.filter(
		(item) =>
			(hosts.length === 0 || hosts.includes(item.hostname)) &&
			tags.every((tag) => item.tags.includes(tag)) &&
			paths.every((path) => item.paths.includes(path))
	);
}

/** The snapshots made inside the date range, as the control plane filters them. */
export function inRange(snapshots: SnapshotItem[], from?: string, to?: string): SnapshotItem[] {
	const start = from ? Date.parse(from) : Number.NEGATIVE_INFINITY;
	const end = to ? Date.parse(to) : Number.POSITIVE_INFINITY;
	return snapshots.filter((item) => {
		const time = item.time ? Date.parse(item.time) : Number.NaN;
		return time >= start && time <= end;
	});
}

function node(
	path: string,
	kind: NodeItem['kind'],
	size = 0,
	mtime: string | null = '2026-10-12T21:04:00Z'
): NodeItem {
	const name = path.slice(path.lastIndexOf('/') + 1);
	return {
		name,
		kind,
		path,
		size,
		permissions: kind === 'dir' ? 'drwxr-xr-x' : '-rw-r--r--',
		mtime,
		link_target: ''
	};
}

const DIRECTORIES: Record<string, NodeItem[]> = {
	'': [node('/etc', 'dir'), node('/home', 'dir'), node('/srv', 'dir'), node('/var', 'dir')],
	'/etc': [
		node('/etc/fstab', 'file', 742),
		node('/etc/hostname', 'file', 10),
		node('/etc/nginx', 'dir'),
		node('/etc/passwd', 'file', 2_201)
	],
	'/etc/nginx': [node('/etc/nginx/nginx.conf', 'file', 1_482)],
	'/home': [node('/home/anna', 'dir'), node('/home/boris', 'dir')],
	'/home/anna': [
		node('/home/anna/Documents', 'dir'),
		node('/home/anna/notes.txt', 'file', 5_120),
		node('/home/anna/photo.jpg', 'file', 3_914_880)
	],
	'/home/anna/Documents': [node('/home/anna/Documents/report.odt', 'file', 88_113)],
	'/home/boris': [node('/home/boris/projects', 'dir')],
	'/home/boris/projects': [node('/home/boris/projects/site', 'dir')],
	'/home/boris/projects/site': [node('/home/boris/projects/site/index.html', 'file', 4_011)],
	'/srv': [node('/srv/data', 'dir'), node('/srv/www', 'dir')],
	'/srv/data': [
		node('/srv/data/backup.sql', 'file', 1_204_511_232),
		node('/srv/data/latest', 'file', 0)
	],
	'/srv/www': [node('/srv/www/index.php', 'file', 3_201), node(LARGE_DIRECTORY, 'dir')],
	'/var': [node('/var/log', 'dir')],
	'/var/log': [node('/var/log/syslog', 'file', 9_204_113)]
};

function largeEntry(index: number): NodeItem {
	const name = `upload-${String(index).padStart(5, '0')}.dat`;
	return node(`${LARGE_DIRECTORY}/${name}`, 'file', 1_024 * (index + 1));
}

/** One page of the entries of a directory, with the offset of the next page. */
export function listDirectory(
	path: string,
	offset: number,
	limit: number
): Pick<LsView, 'nodes' | 'last' | 'next_offset'> {
	const directory = path === '/' ? '' : path.replace(/\/+$/, '');
	if (directory === LARGE_DIRECTORY) {
		const end = Math.min(LARGE_DIRECTORY_SIZE, offset + limit);
		const nodes = Array.from({ length: Math.max(0, end - offset) }, (_, index) =>
			largeEntry(offset + index)
		);
		const last = end >= LARGE_DIRECTORY_SIZE;
		return { nodes, last, next_offset: last ? null : end };
	}
	const entries = DIRECTORIES[directory] ?? [];
	const nodes = entries.slice(offset, offset + limit);
	const last = offset + nodes.length >= entries.length;
	return { nodes, last, next_offset: last ? null : offset + nodes.length };
}

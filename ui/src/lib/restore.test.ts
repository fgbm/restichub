import { describe, expect, it } from 'vitest';
import {
	EMPTY_FILTERS,
	escapeGlob,
	filtersFromQuery,
	filtersToQuery,
	hostPath,
	iconOfNode,
	includePaths,
	isAbsolutePath,
	isCovered,
	isSystemPath,
	isWindowsSnapshot,
	isWithinRoots,
	landingPath,
	parsePatterns,
	profileOfTag,
	snapshotTagChips,
	suggestTarget,
	treePath,
	warnsSystemPath
} from './restore';

describe('restore helpers (FR-RS-02, FR-RS-04)', () => {
	it('fr_rs_04_warns_for_a_system_path_and_for_no_other', () => {
		for (const path of [
			'/',
			'/etc',
			'/etc/nginx',
			'/usr/local/bin',
			'/var/lib/docker',
			'C:\\Windows'
		]) {
			expect(isSystemPath(path), path).toBe(true);
		}
		for (const path of ['C:\\Windows\\Temp', 'c:/program files (x86)/app', 'D:\\', 'C:']) {
			expect(isSystemPath(path), path).toBe(true);
		}
		for (const path of [
			'/srv/restore',
			'/home/anna/back',
			'/var/tmp/x',
			'/etcetera',
			'D:\\restore'
		]) {
			expect(isSystemPath(path), path).toBe(false);
		}
	});

	it('fr_rs_03_reads_a_backslash_as_a_separator_for_a_windows_target_only', () => {
		expect(isWithinRoots('C:\\Restore\\a', ['C:\\restore'], true)).toBe(true);
		expect(isWithinRoots('/srv/restore\\a', ['/srv/restore'], false)).toBe(false);
		expect(isWithinRoots('/srv/restore/a', ['/srv\\restore'], false)).toBe(false);
		expect(isWithinRoots('/srv/restore/a', ['/srv/restore'], false)).toBe(true);
	});

	it('fr_rs_04_knows_an_absolute_path', () => {
		for (const path of ['/srv', 'C:\\data', 'd:/data']) expect(isAbsolutePath(path)).toBe(true);
		for (const path of ['srv', '', '../etc', 'C:data']) expect(isAbsolutePath(path)).toBe(false);
	});

	it('fr_rs_02_a_selected_directory_covers_its_content', () => {
		const selected = new Set(['/srv/data', '/srv/data/backup.sql', '/etc/hostname', '/srvx']);
		expect(isCovered('/srv/data/backup.sql', selected)).toBe(true);
		expect(isCovered('/srv/data', selected)).toBe(false);
		expect(isCovered('/srvx/a', new Set(['/srv']))).toBe(false);
		expect(includePaths(selected)).toEqual(['/etc/hostname', '/srv/data', '/srvx']);
		expect(includePaths(new Set())).toEqual([]);
	});

	it('fr_rs_04_escapes_the_wildcards_of_a_path_from_the_tree', () => {
		// The Rust test `ac_rs_04_a_path_with_wildcard_characters_restores_only_itself` restores
		// with exactly these patterns.
		const table: [string, string][] = [
			['/data/a.txt', '/data/a.txt'],
			['/data/[ab].txt', '/data/[[]ab].txt'],
			['/data/a*.txt', '/data/a[*].txt'],
			['/data/what?.txt', '/data/what[?].txt'],
			['/data/a].txt', '/data/a].txt'],
			['/data/back\\slash', '/data/back[\\\\]slash'],
			['!notes', '[!]notes'],
			['/data/!notes', '/data/!notes']
		];
		for (const [path, pattern] of table) expect(escapeGlob(path), path).toBe(pattern);
	});

	it('fr_rs_03_a_target_inside_a_restore_root_is_accepted', () => {
		const posix = ['/var/lib/restichub-restore', '/srv/restore/'];
		expect(isWithinRoots('/var/lib/restichub-restore/job-1', posix, false)).toBe(true);
		expect(isWithinRoots('/srv/restore', posix, false)).toBe(true);
		for (const path of ['/etc', '/', '/var/lib/restichub-restore2', '/srv/restore/../etc']) {
			expect(isWithinRoots(path, posix, false), path).toBe(false);
		}
		const windows = ['C:\\ProgramData\\ResticHub-Restore'];
		expect(isWithinRoots('c:/programdata/resticHUB-restore/x', windows, true)).toBe(true);
		expect(isWithinRoots('D:\\restore', windows, true)).toBe(false);
		expect(isWithinRoots('/anything', ['/'], false)).toBe(true);
		expect(isWithinRoots('/anything', [], false)).toBe(false);
	});

	it('fr_rs_03_suggests_a_folder_inside_the_first_root', () => {
		expect(suggestTarget('/var/lib/restichub-restore', 'a1b2c3d4', false)).toBe(
			'/var/lib/restichub-restore/a1b2c3d4'
		);
		expect(suggestTarget('C:\\ProgramData\\ResticHub-Restore\\', 'a1b2c3d4', true)).toBe(
			'C:\\ProgramData\\ResticHub-Restore\\a1b2c3d4'
		);
		expect(suggestTarget(undefined, 'a1b2c3d4', false)).toBe('');
	});

	it('fr_rs_03_a_narrow_root_below_a_system_directory_gives_no_warning', () => {
		const roots = ['/var/lib/restichub-restore'];
		expect(warnsSystemPath('/var/lib/restichub-restore/a1', roots, false)).toBe(false);
		expect(warnsSystemPath('/var/lib/docker', roots, false)).toBe(true);
		expect(warnsSystemPath('/etc', ['/'], false)).toBe(true);
		expect(warnsSystemPath('/srv/restore', ['/'], false)).toBe(false);
		expect(warnsSystemPath('C:\\Windows', ['C:\\'], true)).toBe(true);
	});
});

describe('restore page helpers (FR-RS-01, FR-RS-02)', () => {
	it('fr_rs_02_picks_an_icon_for_each_kind_of_entry', () => {
		const file = (name: string, permissions = '-rw-r--r--') => ({
			kind: 'file' as const,
			name,
			permissions
		});
		expect(iconOfNode({ kind: 'dir', name: 'etc' })).toBe('folder');
		expect(iconOfNode({ kind: 'dir', name: 'etc' }, true)).toBe('folder-open');
		expect(iconOfNode({ kind: 'symlink', name: 'latest' })).toBe('link');
		expect(iconOfNode({ kind: 'other', name: 'tty' })).toBe('file-other');
		expect(iconOfNode(file('a.PNG'))).toBe('file-image');
		expect(iconOfNode(file('song.mp3'))).toBe('file-audio');
		expect(iconOfNode(file('clip.mkv'))).toBe('file-video');
		expect(iconOfNode(file('backup.tar.gz'))).toBe('file-archive');
		expect(iconOfNode(file('budget.xlsx'))).toBe('file-spreadsheet');
		expect(iconOfNode(file('manual.pdf'))).toBe('file-document');
		expect(iconOfNode(file('main.rs'))).toBe('file-code');
		expect(iconOfNode(file('notes.txt'))).toBe('file-text');
		expect(iconOfNode(file('setup.exe'))).toBe('file-executable');
		expect(iconOfNode(file('tool', '-rwxr-xr-x'))).toBe('file-executable');
		expect(iconOfNode(file('data'))).toBe('file');
		expect(iconOfNode(file('.bashrc'))).toBe('file');
	});

	it('fr_rs_01_reads_the_profile_of_an_automatic_tag', () => {
		const id = '0b9f4a2c-3e1d-4c55-8a21-9d7e6f5a4b3c';
		expect(profileOfTag(`restichub-profile-${id}`)).toBe(id);
		expect(profileOfTag('restichub-profile-nothing')).toBeNull();
		expect(profileOfTag('daily')).toBeNull();
	});

	it('fr_rs_01_builds_the_tag_chips_of_a_snapshot_row_with_the_profile_first', () => {
		const id = '0b9f4a2c-3e1d-4c55-8a21-9d7e6f5a4b3c';
		const other = '7c1e5d90-2a44-4f1b-9b0e-3d6a8c2f1e55';
		const profiles = new Map([[id, 'Office files']]);
		const options = {
			href: (profile: string) => `/profiles/${profile}`,
			title: (name: string) => `open ${name}`
		};
		const chips = snapshotTagChips(
			['daily', `restichub-profile-${id}`, `restichub-profile-${other}`],
			profiles,
			options
		);
		expect(chips.map((chip) => chip.label)).toEqual([
			'Office files',
			'daily',
			`restichub-profile-${other}`
		]);
		expect(chips[0]).toEqual({
			label: 'Office files',
			tone: 'running',
			href: `/profiles/${id}`,
			title: 'open Office files'
		});
		expect(chips[1]).toEqual({ label: 'daily' });
		expect(snapshotTagChips([], profiles, options)).toEqual([]);
	});

	it('fr_rs_01_keeps_an_unknown_profile_tag_as_a_plain_tag', () => {
		const known = '0b9f4a2c-3e1d-4c55-8a21-9d7e6f5a4b3c';
		const unknown = '7c1e5d90-2a44-4f1b-9b0e-3d6a8c2f1e55';
		const options = {
			href: (profile: string) => `/profiles/${profile}`,
			title: (name: string) => `open ${name}`
		};
		const chips = snapshotTagChips(
			[`restichub-profile-${unknown}`, `restichub-profile-${known}`],
			new Map([[known, 'Office files']]),
			options
		);
		expect(chips).toEqual([
			{
				label: 'Office files',
				tone: 'running',
				href: `/profiles/${known}`,
				title: 'open Office files'
			},
			{ label: `restichub-profile-${unknown}` }
		]);
	});

	it('fr_rs_01_keeps_the_filters_in_the_query_string', () => {
		const filters = {
			...EMPTY_FILTERS,
			host: 'nas',
			tag: 'daily',
			from: '2026-10-01',
			period: 'custom'
		};
		const query = filtersToQuery(filters, { repository: 'r_1' });
		expect(query.toString()).toBe(
			'host=nas&tag=daily&from=2026-10-01&period=custom&repository=r_1'
		);
		expect(filtersFromQuery(query)).toEqual(filters);
		expect(filtersToQuery(EMPTY_FILTERS).toString()).toBe('');
	});

	it('fr_rs_04_splits_exclude_patterns', () => {
		expect(parsePatterns(' *.tmp\n/var/cache\n\n node_modules ')).toEqual([
			'*.tmp',
			'/var/cache',
			'node_modules'
		]);
		expect(parsePatterns('a,b\nc d')).toEqual(['a,b', 'c d']);
		expect(parsePatterns('  ')).toEqual([]);
	});

	it('fr_rs_04_shows_where_a_path_lands', () => {
		expect(landingPath('/srv/restore/', '/etc/fstab', false)).toBe('/srv/restore/etc/fstab');
		expect(landingPath('D:\\restore', '/etc', true)).toBe('D:\\restore\\etc');
		expect(landingPath('/srv/restore', '/', false)).toBe('/srv/restore');
	});
});

describe('restore paths in the format of the host (FR-RS-04)', () => {
	it('shows the tree path of a Windows snapshot with the drive letter', () => {
		expect(isWindowsSnapshot(['C:\\Users\\vch', 'D:\\Projects'])).toBe(true);
		expect(isWindowsSnapshot(['/srv', '/etc'])).toBe(false);
		expect(hostPath('/C/Users/vch/notes.txt', true)).toBe('C:\\Users\\vch\\notes.txt');
		expect(hostPath('/D', true)).toBe('D:\\');
		expect(hostPath('/srv/www', false)).toBe('/srv/www');
		expect(hostPath('/srv/www', true)).toBe('/srv/www');
	});

	it('gives the tree form of a snapshot path, where restic puts the files', () => {
		expect(treePath('C:\\Users\\vch')).toBe('/C/Users/vch');
		expect(treePath('D:\\')).toBe('/D');
		expect(treePath('/srv')).toBe('/srv');
		expect(landingPath('D:\\restore\\a1b2', treePath('C:\\Users\\vch'), true)).toBe(
			'D:\\restore\\a1b2\\C\\Users\\vch'
		);
	});
});

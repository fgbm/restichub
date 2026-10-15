import { describe, expect, it } from 'vitest';
import { includePaths, isAbsolutePath, isCovered, isSystemPath } from './restore';

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
});

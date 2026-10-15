/*
 * Pure helpers of the Restore screen: the selection of files, the warning for a system path and the
 * overwrite modes (FR-RS-02, FR-RS-04).
 */
import type { OverwriteMode } from '#lib/api/index.ts';

export const OVERWRITE_MODES: OverwriteMode[] = ['always', 'if-changed', 'if-newer', 'never'];

// A restore into one of these places can replace files that the operating system needs. The screen
// warns; it does not block, because the Operator may mean it.
const UNIX_SYSTEM = [
	'/bin',
	'/boot',
	'/dev',
	'/etc',
	'/lib',
	'/lib32',
	'/lib64',
	'/proc',
	'/root',
	'/run',
	'/sbin',
	'/sys',
	'/usr',
	'/var/lib'
];

const WINDOWS_SYSTEM = /^[a-z]:\/(windows|program files|program files \(x86\)|programdata)(\/|$)/;

/** True when the path is the root of a drive or of the file system, or a system directory. */
export function isSystemPath(target: string): boolean {
	const path = target.trim().replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase();
	if (path === '' || /^[a-z]:$/.test(path)) return true;
	if (WINDOWS_SYSTEM.test(`${path}/`)) return true;
	return UNIX_SYSTEM.some((root) => path === root || path.startsWith(`${root}/`));
}

/** True for a path that starts at a root: `/`, or a drive letter and a separator. */
export function isAbsolutePath(path: string): boolean {
	return path.startsWith('/') || /^[a-zA-Z]:[\\/]/.test(path);
}

/** True when `path` lies inside `ancestor`. */
export function isInside(path: string, ancestor: string): boolean {
	return path !== ancestor && path.startsWith(ancestor.endsWith('/') ? ancestor : `${ancestor}/`);
}

/** True when an ancestor of the path is selected, so the selection already holds the path. */
export function isCovered(path: string, selected: ReadonlySet<string>): boolean {
	for (const item of selected) if (isInside(path, item)) return true;
	return false;
}

/**
 * The paths that a restore includes. A selected directory holds its content, so a selected path
 * inside it adds nothing.
 */
export function includePaths(selected: ReadonlySet<string>): string[] {
	return [...selected].filter((path) => !isCovered(path, selected)).sort();
}

/**
 * The restic pattern that matches one path only. A path from the tree is a literal name, but restic
 * reads `*`, `?` and `[` as a wildcard and a leading `!` as a negation. A backslash cannot escape on
 * Windows, where restic reads it as a separator, so each character stands in a one-character class.
 * A `]` outside a class is already literal.
 */
export function escapeGlob(path: string): string {
	const escaped = path.replace(/[*?[\\]/g, (char) => (char === '\\' ? '[\\\\]' : `[${char}]`));
	return escaped.startsWith('!') ? `[!]${escaped.slice(1)}` : escaped;
}

function normalize(path: string, windows: boolean): string {
	const unified = path.trim().replaceAll('\\', '/').replace(/\/+$/, '');
	return windows ? unified.toLowerCase() : unified;
}

/**
 * True when `target` is a restore root or lies inside one. The agent checks the path again and
 * resolves links, so this is the early answer. A Windows path ignores case.
 */
export function isWithinRoots(target: string, roots: readonly string[], windows: boolean): boolean {
	const path = normalize(target, windows);
	// The agent refuses a `.` or `..` component, so the answer here is no.
	if (path.split('/').some((part) => part === '.' || part === '..')) return false;
	return roots.some((root) => {
		const base = normalize(root, windows);
		return path === base || path.startsWith(`${base}/`);
	});
}

/** The target that the dialog suggests: a folder named after the snapshot inside the root. */
export function suggestTarget(root: string | undefined, name: string, windows: boolean): string {
	if (!root) return '';
	const separator = windows ? '\\' : '/';
	return `${root.replace(/[\\/]+$/, '')}${separator}${name}`;
}

/**
 * True when the screen warns about the target. A target inside a narrow restore root is the
 * choice of the administrator, even when the root lies below a system directory. A root that is a
 * whole file system or drive keeps the warning.
 */
export function warnsSystemPath(
	target: string,
	roots: readonly string[],
	windows: boolean
): boolean {
	if (!isSystemPath(target)) return false;
	const narrow = roots.filter((root) => {
		const path = normalize(root, windows);
		return path !== '' && !/^[a-z]:$/i.test(path);
	});
	return !isWithinRoots(target, narrow, windows);
}

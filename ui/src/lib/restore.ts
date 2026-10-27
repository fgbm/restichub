/*
 * Pure helpers of the Restore screen: the selection of files, the warning for a system path and the
 * overwrite modes (FR-RS-02, FR-RS-04).
 */
import type { OverwriteMode } from '#lib/api/index.ts';
import type { IconName, ListTag } from '#lib/components/ui/index.ts';

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
	const trimmed = path.trim();
	// A backslash is a name character on Linux, so only a Windows path gets it as a separator.
	const unified = (windows ? trimmed.replaceAll('\\', '/') : trimmed).replace(/\/+$/, '');
	return windows ? unified.toLowerCase() : unified;
}

/**
 * True when `target` is a restore directory or lies inside one. The agent checks the path again and
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
 * True when the screen warns about the target. A target inside a narrow restore directory is the
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

const PROFILE_TAG = /^restichub-profile-([0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12})$/i;

/** The profile identifier inside an automatic `restichub-profile-<id>` tag, or null for any other tag. */
export function profileOfTag(tag: string): string | null {
	return PROFILE_TAG.exec(tag)?.[1] ?? null;
}

/**
 * The tag chips of a snapshot row (FR-RS-01). The chip of a profile comes first, so a cut line
 * keeps it; the other tags follow in the order of the snapshot. A profile that the screen does not
 * know stays a plain tag, so that a chip never leads nowhere.
 */
export function snapshotTagChips(
	tags: readonly string[],
	profiles: ReadonlyMap<string, string>,
	options: { href: (id: string) => string; title: (name: string) => string }
): ListTag[] {
	const chips = tags.map((tag): ListTag => {
		const profile = profileOfTag(tag);
		const name = profile === null ? undefined : profiles.get(profile);
		if (profile === null || name === undefined) return { label: tag };
		return {
			label: name,
			tone: 'running',
			href: options.href(profile),
			title: options.title(name)
		};
	});
	const known = chips.findIndex((chip) => chip.tone === 'running');
	if (known > 0) chips.unshift(...chips.splice(known, 1));
	return chips;
}

const EXTENSIONS: [IconName, string[]][] = [
	['file-archive', ['zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', 'zst', '7z', 'rar', 'iso']],
	['file-image', ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'tif', 'tiff', 'heic', 'ico']],
	['file-audio', ['mp3', 'wav', 'flac', 'ogg', 'aac', 'm4a', 'opus']],
	['file-video', ['mp4', 'mkv', 'avi', 'mov', 'webm', 'wmv']],
	['file-spreadsheet', ['xls', 'xlsx', 'ods', 'csv', 'tsv']],
	['file-document', ['pdf', 'doc', 'docx', 'odt', 'rtf', 'ppt', 'pptx', 'odp', 'epub']],
	['file-executable', ['exe', 'msi', 'dll', 'so', 'bin', 'app', 'com']],
	[
		'file-code',
		['js', 'ts', 'py', 'rs', 'go', 'php', 'java', 'c', 'h', 'cpp', 'cs', 'rb', 'sh', 'ps1', 'bat']
	],
	[
		'file-code',
		['html', 'css', 'json', 'yml', 'yaml', 'toml', 'xml', 'sql', 'conf', 'ini', 'cfg', 'service']
	],
	['file-text', ['txt', 'md', 'log', 'readme', 'license']]
];

const BY_EXTENSION = new Map(
	EXTENSIONS.flatMap(([icon, list]) => list.map((extension) => [extension, icon] as const))
);

/**
 * The icon of a tree entry. A directory and a link have their own icons; a file gets one by its
 * extension, and a file with an execute bit and no known extension counts as executable.
 */
export function iconOfNode(
	node: { kind: 'file' | 'dir' | 'symlink' | 'other'; name: string; permissions?: string },
	expanded = false
): IconName {
	if (node.kind === 'dir') return expanded ? 'folder-open' : 'folder';
	if (node.kind === 'symlink') return 'link';
	if (node.kind === 'other') return 'file-other';
	const dot = node.name.lastIndexOf('.');
	const extension = dot > 0 ? node.name.slice(dot + 1).toLowerCase() : '';
	const known = BY_EXTENSION.get(extension);
	if (known) return known;
	const mode = node.permissions ?? '';
	return [3, 6, 9].some((index) => mode[index] === 'x') ? 'file-executable' : 'file';
}

/** The filters of the snapshot list, as they travel in the query string of both Restore pages. A host
 * or a tag holds several values joined by commas. */
export interface RestoreFilters {
	host: string;
	tag: string;
	path: string;
	from: string;
	to: string;
	/** A preset such as `7d`, or `custom` for the dates in `from` and `to`. Empty means all time. */
	period: string;
}

export const EMPTY_FILTERS: RestoreFilters = {
	host: '',
	tag: '',
	path: '',
	from: '',
	to: '',
	period: ''
};

/** The query string of the filters. A filter without a value stays out. */
export function filtersToQuery(
	filters: RestoreFilters,
	extra: Record<string, string> = {}
): URLSearchParams {
	const query = new URLSearchParams();
	for (const [key, value] of Object.entries({ ...filters, ...extra })) {
		if (value) query.set(key, value);
	}
	return query;
}

/** The query parameter that names the host the restore lands on. */
export const TARGET_PARAM = 'target';

/** The address parameters that the restore pages keep: the preselected target host, if any. */
export function targetExtra(query: { get(name: string): string | null }): Record<string, string> {
	const target = query.get(TARGET_PARAM);
	return target ? { [TARGET_PARAM]: target } : {};
}

export function filtersFromQuery(query: { get(name: string): string | null }): RestoreFilters {
	return {
		host: query.get('host') ?? '',
		tag: query.get('tag') ?? '',
		path: query.get('path') ?? '',
		from: query.get('from') ?? '',
		to: query.get('to') ?? '',
		period: query.get('period') ?? ''
	};
}

/** The patterns of an exclude field: one for each line, without blanks. A pattern can hold a comma. */
export function parsePatterns(text: string): string[] {
	return text
		.split('\n')
		.map((item) => item.trim())
		.filter((item) => item !== '');
}

/** The path where a restored path lands: the target followed by the path from the snapshot. */
export function landingPath(target: string, snapshotPath: string, windows: boolean): string {
	const base = target.trim().replace(/[\\/]+$/, '');
	const separator = windows ? '\\' : '/';
	const rest = snapshotPath.replace(/^\/+/, '');
	return rest === '' ? base : `${base}${separator}${windows ? rest.replaceAll('/', '\\') : rest}`;
}

const DRIVE_PATH = /^([a-zA-Z]):(?:[\\/](.*))?$/;
const TREE_DRIVE_PATH = /^\/([a-zA-Z])(?:\/(.*))?$/;

/** True when the snapshot holds Windows paths: restic stores them with a drive letter. */
export function isWindowsSnapshot(snapshotPaths: readonly string[]): boolean {
	return snapshotPaths.some((path) => DRIVE_PATH.test(path));
}

/** A path of the file tree in the format of the host that made the snapshot: `/C/Users` is `C:\Users`. */
export function hostPath(treePath: string, windows: boolean): string {
	const match = windows ? TREE_DRIVE_PATH.exec(treePath) : null;
	if (!match) return treePath;
	return `${match[1].toUpperCase()}:\\${(match[2] ?? '').replaceAll('/', '\\')}`;
}

/** A snapshot path in the format of the file tree, the way restic restores it: `C:\Users` is `/C/Users`. */
export function treePath(path: string): string {
	const match = DRIVE_PATH.exec(path);
	if (!match) return path;
	const rest = (match[2] ?? '').replaceAll('\\', '/').replace(/\/+$/, '');
	return rest === '' ? `/${match[1]}` : `/${match[1]}/${rest}`;
}

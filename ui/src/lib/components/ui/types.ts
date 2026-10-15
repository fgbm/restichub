import type { Snippet } from 'svelte';
import type { IconName } from './icons';

export interface SelectOption {
	value: string;
	label: string;
}

export type BadgeTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'info';

export type StatusTone =
	'success' | 'running' | 'queued' | 'warning' | 'failed' | 'cancelled' | 'neutral' | 'info';

export interface TabItem {
	value: string;
	label: string;
	icon?: IconName;
	count?: number;
}

export interface MenuItem {
	label: string;
	icon?: IconName;
	/** A menu item without an action is a heading, not a command. */
	onSelect?: () => void;
	tone?: 'default' | 'danger';
	disabled?: boolean;
	separatorBefore?: boolean;
	/** Items with the same group are exclusive; `checked` marks the current one. */
	group?: string;
	/** Heading of the group; read from the first item of the group. */
	groupLabel?: string;
	checked?: boolean;
}

export interface TableColumn<T> {
	key: string;
	header: string;
	mono?: boolean;
	/** Sort the column. Default: `true`. */
	sortable?: boolean;
	/** Keep the cell on one line, for a date, a relative time or a status. */
	nowrap?: boolean;
	/** Align the cell text. Default: `left`. */
	align?: 'left' | 'right' | 'center';
	/** Value used for sorting and filtering. Default: `String(row[key])`. */
	value?: (row: T) => string;
	/** Cell text. Default: the value. */
	cell?: (row: T) => string;
	badge?: (row: T) => BadgeTone | null;
	/** Cell markup; wins over `cell` and `badge`. */
	render?: Snippet<[T]>;
}

export interface KeyValueItem {
	key: string;
	label: string;
	value: string;
	mono?: boolean;
}

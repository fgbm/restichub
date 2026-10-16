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
	/** Tooltip of the icon button in a row with one or two actions, instead of the label. */
	hint?: string;
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
	/** Hide the column on a narrow screen, so the row actions stay in view. */
	hideNarrow?: boolean;
	/** Align the cell text. Default: `left`. */
	align?: 'left' | 'right' | 'center';
	/** Value used for sorting and filtering. Default: `String(row[key])`. */
	value?: (row: T) => string;
	/** Cell text. Default: the value. */
	cell?: (row: T) => string;
	badge?: (row: T) => BadgeTone | null;
	/** Cell markup; wins over `cell` and `badge`. */
	render?: Snippet<[T]>;
	/** The cell that holds the link to the record. Default: the first column. */
	primary?: boolean;
	/** Cut a long value with an ellipsis; a tooltip shows the full value. */
	truncate?: boolean | 'sm' | 'md' | 'lg';
	/** The value is an RFC 3339 time: a relative or an absolute time on one line. */
	date?: 'relative' | 'absolute';
}

export interface RowAction {
	id: string;
	label: string;
	icon: IconName;
	onSelect: () => void;
	/** Tooltip of the icon button, when it says more than the label. */
	hint?: string;
	danger?: boolean;
	hidden?: boolean;
	disabled?: boolean;
}

export interface TableSort {
	key: string;
	direction: 'asc' | 'desc';
}

export interface KeyValueItem {
	key: string;
	label: string;
	value: string;
	mono?: boolean;
	/** Value markup, such as a pill or a copy button; wins over `value`. */
	render?: Snippet;
}

export interface FilterSearch {
	/** The query parameter of the search text. */
	key: string;
	label: string;
	placeholder?: string;
	mono?: boolean;
	/** Default: `filter-` and the key. */
	id?: string;
}

export interface FilterOption {
	value: string;
	/** Default: the value. */
	label?: string;
	/** The number of items with this value. */
	count?: number;
}

export interface FilterFacet {
	/** The query parameter of the facet. */
	key: string;
	label: string;
	/** A facet without options takes a free text, such as an identifier. */
	options?: FilterOption[];
	/** Several values at once. Default: `true`. */
	multiple?: boolean;
	/** The placeholder of a free-text facet. */
	placeholder?: string;
	mono?: boolean;
}

import type { Snippet } from 'svelte';
import type { IconName } from './icons';

export interface SelectOption {
	value: string;
	label: string;
}

/** A part of the secondary line of a record: a muted text or a small tag chip. */
export type RecordPart = string | { tag: string; icon?: IconName };

/** A native select cannot draw a secondary line, so only `RecordPicker` takes this type. */
export interface RecordOption extends SelectOption {
	/** The closed field shows the first part after the label. */
	description?: RecordPart[];
	/**
	 * The content after the label in the closed field. Default: the first part of `description`.
	 * Parts render as in the secondary line, so a chip stays a chip.
	 */
	detail?: string | RecordPart[];
	/** A status dot before the label. `state` gives the same fact in words. */
	dot?: RecordDot;
	/** The state in words at the right of the option. */
	state?: string;
	disabled?: boolean;
	/** Why the option cannot be selected; it shows below the option. */
	reason?: string;
	/** Extra texts that the search finds, which the option does not show. */
	search?: string[];
}

export type RecordDot = 'success' | 'warning' | 'neutral';

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
	/**
	 * A short fact below the label, for example the version of the build. A disabled menu item puts
	 * its reason here.
	 */
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

/** One action of the section "Dangerous actions" of a record page (UI principles 1.7). */
export interface DangerAction {
	/** Key of the action in the section. */
	key: string;
	/** One sentence on what the action does to the record. */
	text: string;
	label: string;
	icon: IconName;
	/** `danger` for an action that removes data or blocks the record. Default: `secondary`. */
	tone?: 'secondary' | 'danger';
	/** Why the action is not available now. The button shows it as its tooltip. */
	hint?: string;
	disabled?: boolean;
	loading?: boolean;
	onclick: () => void;
}

/** The tones that color a list item; the usual state has no tone. */
export type ListTone = 'danger' | 'warning' | 'running' | 'success' | 'neutral';

export interface ListIcon {
	name: IconName;
	/** Only `danger`, `warning`, `running` and `success` color the icon. */
	tone?: ListTone;
	spin?: boolean;
	/** The state that the icon shows, for the tooltip and the accessible name. */
	label?: string;
}

export interface ListPill {
	tone: ListTone;
	label: string;
	/** The full meaning of a short label, for the tooltip and the accessible name. */
	full?: string;
}

/** A tag chip of line 2. The profile tag of a snapshot is the one chip that links out (ui-principles 2.6). */
export interface ListTag {
	label: string;
	/** The resolved address of the record of the tag; the chip becomes a link. */
	href?: string;
	/** `running` marks the automatic profile tag. The usual tag is neutral. */
	tone?: 'neutral' | 'running';
	/** The tooltip of the chip; a link also takes it as the accessible name. */
	title?: string;
}

export interface ListField {
	/** The text of the field. A field with `tags` has none. */
	text?: string;
	mono?: boolean;
	/** A Lucide icon before the text; the field shows as a neutral chip, for example the count of parameters. */
	icon?: IconName;
	/** An RFC 3339 time: the field is a `<time>` element with the full time in a tooltip. */
	time?: string;
	/** The tooltip of the field, for example the cron expression of a schedule. */
	title?: string;
	/** Tags as chips. Line 2 keeps two chips; the rest fold into one chip that opens the whole set. */
	tags?: readonly ListTag[];
}

/** An empty field is left out, so the order of the other fields stays. */
export type ListMeta = ListField | string | null | undefined | false;

export interface FilterTab {
	value: string;
	label: string;
	count?: number;
	/** The server counted only up to its limit: the tab shows the count with a "+". */
	capped?: boolean;
	/** The count is red when it is more than zero, for the failure tab only. */
	danger?: boolean;
}

export interface FilterTabs {
	/** The query parameter of the tab. Default: `state`. */
	key?: string;
	options: FilterTab[];
	/** The tab that an empty parameter selects. */
	default: string;
}

export interface FilterSortOption {
	value: string;
	label: string;
	/** The order that the field starts with. Default: `asc`. */
	order?: 'asc' | 'desc';
}

export interface FilterSort {
	options: FilterSortOption[];
	/** The field that an empty `sort` parameter selects. */
	default: string;
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
	/** A secondary line below the label; the facet search also matches it. */
	description?: RecordPart[];
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
	/** A short muted line at the top of the popover. */
	hint?: string;
	/** Show the search field at any number of options. Default: from seven options. */
	search?: boolean;
	/** The options hold every possible value, so a kept value outside them is dropped. */
	exact?: boolean;
}

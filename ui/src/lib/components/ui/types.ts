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
	count?: number;
}

export interface MenuItem {
	label: string;
	/** A menu item without an action is a heading, not a command. */
	onSelect?: () => void;
	tone?: 'default' | 'danger';
	disabled?: boolean;
	separatorBefore?: boolean;
}

export interface TableColumn<T> {
	key: string;
	header: string;
	mono?: boolean;
	align?: 'left' | 'right' | 'center';
	/** Cell text. Default: `String(row[key])`. */
	cell?: (row: T) => string;
}

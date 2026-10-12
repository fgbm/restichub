/** One option of a select control. */
export interface SelectOption {
	value: string;
	label: string;
}

/** One item of a key-value list. */
export interface KeyValueItem {
	/** Stable key. */
	key: string;
	/** Label of the item. */
	label: string;
	/** Value text. */
	value: string;
	/** Render the value as monospace text. */
	mono?: boolean;
}

/** Tone of a badge. */
export type BadgeTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'info';

/** One column of a data table. */
export interface Column<T> {
	/** Stable key. It is also the default field for sorting. */
	key: string;
	/** Header text. */
	header: string;
	/** Render the value as monospace text (identifiers and hashes). */
	mono?: boolean;
	/** Sort the column. Default: `true`. */
	sortable?: boolean;
	/** Align the cell text. Default: `left`. */
	align?: 'left' | 'right' | 'center';
	/** Value used for sorting and filtering. Default: `String(row[key])`. */
	value?: (row: T) => string;
	/** Cell text. Default: the value. */
	cell?: (row: T) => string;
	/** Render the cell text as a badge with this tone. */
	badge?: (row: T) => BadgeTone | null;
}

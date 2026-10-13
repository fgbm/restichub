export interface SelectOption {
	value: string;
	label: string;
}

export interface KeyValueItem {
	key: string;
	label: string;
	value: string;
	mono?: boolean;
}

export type BadgeTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'info';

export interface Column<T> {
	/** Also the default field for sorting. */
	key: string;
	header: string;
	mono?: boolean;
	/** Sort the column. Default: `true`. */
	sortable?: boolean;
	/** Align the cell text. Default: `left`. */
	align?: 'left' | 'right' | 'center';
	/** Value used for sorting and filtering. Default: `String(row[key])`. */
	value?: (row: T) => string;
	/** Cell text. Default: the value. */
	cell?: (row: T) => string;
	badge?: (row: T) => BadgeTone | null;
}

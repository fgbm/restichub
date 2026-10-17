export interface SaveBar {
	dirty: boolean;
	saving?: boolean;
	/** A new record: Save stays enabled before the first change. */
	create?: boolean;
	/** Why the person cannot save, for example the role. The bar shows it in place of the buttons. */
	readonly?: string | null;
	/** A short fact after the state, for example the number of plans that use the record. */
	note?: string;
	oncancel: () => void;
}

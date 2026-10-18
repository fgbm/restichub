export interface SaveBar {
	dirty: boolean;
	saving?: boolean;
	/** A new record: Save stays enabled before the first change. */
	create?: boolean;
	/** Why the person cannot save, for example the role. The bar shows it in place of the buttons. */
	readonly?: string | null;
	/**
	 * Why the server would refuse the save that the form holds now, for example a required
	 * parameter without a value. Save is disabled and the bar shows this reason (ui-principles 5.5).
	 */
	blocked?: string | null;
	/** A short fact after the state, for example the number of plans that use the record. */
	note?: string;
	oncancel: () => void;
}

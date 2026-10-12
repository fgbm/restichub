/*
 * Toast notifications. The notification area reads this store.
 */
export type ToastTone = 'info' | 'success' | 'warning' | 'danger';

export interface Toast {
	id: string;
	tone: ToastTone;
	message: string;
}

let sequence = 0;
let items = $state<Toast[]>([]);

const DEFAULT_TIMEOUT = 5000;

export function getToasts(): Toast[] {
	return items;
}

export function pushToast(tone: ToastTone, message: string, timeout = DEFAULT_TIMEOUT): string {
	const id = `toast-${++sequence}`;
	items = [...items, { id, tone, message }];
	if (timeout > 0) {
		setTimeout(() => dismissToast(id), timeout);
	}
	return id;
}

export function dismissToast(id: string): void {
	items = items.filter((toast) => toast.id !== id);
}

export function clearToasts(): void {
	items = [];
}

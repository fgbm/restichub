/*
 * Svelte action: keep the keyboard focus inside a container.
 *
 * The action moves the focus to the first focusable element and returns the
 * focus to the previous element on destroy. The Tab and Shift+Tab keys cycle
 * through the focusable elements.
 */
const FOCUSABLE =
	'a[href], area[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), iframe, [tabindex]:not([tabindex="-1"])';

function focusableItems(node: HTMLElement): HTMLElement[] {
	return Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE));
}

export function focusTrap(node: HTMLElement) {
	const previous = document.activeElement as HTMLElement | null;

	function onKeydown(event: KeyboardEvent): void {
		if (event.key !== 'Tab') return;
		const items = focusableItems(node);
		if (items.length === 0) {
			event.preventDefault();
			return;
		}
		const first = items[0];
		const last = items[items.length - 1];
		const active = document.activeElement;
		if (event.shiftKey && active === first) {
			event.preventDefault();
			last.focus();
		} else if (!event.shiftKey && active === last) {
			event.preventDefault();
			first.focus();
		}
	}

	node.addEventListener('keydown', onKeydown);
	const initial = focusableItems(node);
	if (initial.length > 0) {
		initial[0].focus();
	} else {
		node.focus();
	}

	return {
		destroy(): void {
			node.removeEventListener('keydown', onKeydown);
			previous?.focus?.();
		}
	};
}

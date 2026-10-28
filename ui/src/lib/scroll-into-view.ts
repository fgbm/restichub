// A row that scrolls sideways keeps a focused entry partly outside its box, because the browser
// reveals a focus target only when it is fully outside. The ring of a partly visible entry is
// clipped as well, so the row moves the entry into view on each focus.
export function revealFocused(node: HTMLElement): () => void {
	const onFocusIn = (event: FocusEvent): void => {
		if (event.target instanceof HTMLElement) {
			event.target.scrollIntoView({ inline: 'nearest', block: 'nearest' });
		}
	};
	node.addEventListener('focusin', onFocusIn);
	return () => node.removeEventListener('focusin', onFocusIn);
}

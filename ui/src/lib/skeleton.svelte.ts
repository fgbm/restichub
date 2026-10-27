// A skeleton that lives for one frame looks like a fault, so it waits until the load is long
// enough to notice (design/style/ui-principles.md, 4.3).
const SKELETON_DELAY_MS = 200;

// Returns a flag that turns true after the delay. Call it at component initialization, so the
// effect follows the life of the component.
export function showSkeleton(loading: () => boolean): () => boolean {
	let shown = $state(false);

	$effect(() => {
		if (!loading()) {
			shown = false;
			return;
		}
		const timer = setTimeout(() => (shown = true), SKELETON_DELAY_MS);
		return () => clearTimeout(timer);
	});

	return () => shown;
}

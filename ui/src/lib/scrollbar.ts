// A classic window scrollbar takes part of the window, an overlay scrollbar takes none. The shell
// always reserves the width in `--rh-scrollbar-width`, so the right edge of the top bar and of a
// toast keeps its place when a long page adds the scrollbar.

// The probe is a block with a scrollbar of its own, so the difference of its two widths is the
// width that the window scrollbar takes on this system.
function measure(): number {
	const probe = document.createElement('div');
	probe.style.setProperty('position', 'absolute');
	probe.style.setProperty('visibility', 'hidden');
	probe.style.setProperty('overflow', 'scroll');
	probe.style.setProperty('width', '100px');
	probe.style.setProperty('height', '100px');
	document.body.append(probe);
	const width = probe.offsetWidth - probe.clientWidth;
	probe.remove();
	return width;
}

export function watchWindowScrollbar(): () => void {
	const root = document.documentElement;
	const apply = (): void => root.style.setProperty('--rh-scrollbar-width', `${measure()}px`);
	apply();

	// A browser zoom changes the width of the scrollbar, and it fires a resize like a window
	// resize does. One measure per frame is enough for a drag of the window edge.
	let frame = 0;
	const onResize = (): void => {
		if (frame) return;
		frame = requestAnimationFrame(() => {
			frame = 0;
			apply();
		});
	};
	window.addEventListener('resize', onResize);
	return () => {
		window.removeEventListener('resize', onResize);
		if (frame) cancelAnimationFrame(frame);
	};
}

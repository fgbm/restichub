# ADR-0027: Web UI component primitives

- Status: accepted
- Date: 2026-10-13
- Requirements: FR-UI-01 to FR-UI-04, NFR-SEC-01

## Context

ADR-0012 fixes the web UI as a Svelte 5 single-page application in the control plane binary. The mockups in [ui/mockups](../../ui/mockups/README.md) fix the design and the component inventory. The inventory needs interactive elements: a dialog, a side panel, dropdown menus, a searchable combobox, a popover, a tooltip, tabs and a switch. Each element needs keyboard behavior and an ARIA role.

The control plane serves the UI with a strict Content Security Policy ([ui.rs](../../crates/restichub-cp/src/ui.rs)). A primitive must work under that policy. A primitive that writes an inline style attribute or injects a style element needs `style-src 'unsafe-inline'`.

The project has one main maintainer. Each added dependency is a maintenance cost.

## Decision

- We use bits-ui (MIT, pinned at 2.19.5 or later) as the headless primitives for these elements:

  - Dialog, also the side panel.
  - DropdownMenu: the tenant menu, the user menu and row actions.
  - Combobox: a searchable choice.
  - Popover and Tooltip.
  - Tabs: in-page tabs only. The top navigation stays links with `aria-current`.
  - Switch.
  - DatePicker and TimeField, loaded lazily at a later step.
- We do not use Tailwind and we do not copy a component kit. The UI keeps its own CSS on the design tokens of `ui/mockups/mockups.css`. The work moves `ui/src/lib/styles/tokens.css` to those tokens. The CSS styles a state through the `data-state` and `data-highlighted` attributes.
- We keep the native `<select>` for a simple choice, because the bits-ui Select has open accessibility issues. We keep the own DataTable (a headless table library arrives only when column hiding, filters or virtualization are needed). We keep the own Toast, Button, fields and badges.
- The policy uses `style-src 'self'` without `unsafe-inline`. bits-ui and Svelte set styles through the CSS Object Model. A verification in Chromium 153 found no violation. No template holds a static `style="..."` attribute (a lint rule). We do not use PinInput, because it injects a style element.
- A CI test opens Dialog, DropdownMenu, Combobox and Tooltip on the production build under the production policy. The test fails on any `securitypolicyviolation` event. It runs in Chromium and Firefox, and in WebKit when the runner has it.
- An upgrade of bits-ui passes that test before the merge. The version stays pinned and the maintainer reviews the changelogs.
- Later, the file tree for restore uses an ARIA tree pattern. That epic decides between an own tree and a headless tree of another library.

## Consequences

- The bundle grows by about 55 kB gzip. A rare part loads lazily.
- The project takes a dependency risk. One main maintainer reviews each upgrade.
- The accessibility of a menu, a combobox and a popover comes from a maintained library, not from own code.
- The CSP stays strict. A template cannot use a static `style` attribute. A component sets a dynamic value through the CSS Object Model or a custom property.
- The tokens of the UI and the mockups agree, so a screenshot comparison is meaningful.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| A component kit with Tailwind | The command line tools have problems with the current SvelteKit. The class-merge helpers add about 21 kB gzip. Every class is rewritten to the project tokens in any case. |
| Hand-written primitives only | The accessibility cost of a menu, a combobox and a popover is high. |
| Another headless library | The libraries are pre-1.0 or use a state-machine API. Keep one as a fallback for the tree. |

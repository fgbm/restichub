# ResticHub UI mockups

These pages fix the design principles and the interface elements before the team writes the Svelte code.
The set has plain HTML, one stylesheet and one script. There is no build step, no downloaded font and no external service.

## Open the mockups

1. Run this command in the repository root:

   ```sh
   python3 -m http.server 8765 --bind 127.0.0.1 --directory ui/mockups
   ```

2. Open the address `http://127.0.0.1:8765/index.html` in a browser.
3. Stop the server with Ctrl+C.

The browser reads `mockups.css`, `mockups.js` and `icons.svg` from the same directory.
Open the pages through the server. A `file://` address does not load the icon sprite.

## Pages

| File | Contents |
|---|---|
| `index.html` | The list of mockups. |
| `principles.html` | The design principles and the color tokens. |
| `components.html` | Every element in its states. |
| `shell-a.html` | The Hosts page with the top tabs. |
| `tenant-switch.html` | The Hosts page with the tenant menu open. |
| `hosts.html` | The Hosts list. |
| `hosts-ru.html` | The Hosts list in Russian. |
| `host.html` | The host detail. |
| `job.html` | The job run. |
| `repositories.html` | The repositories. |
| `audit.html` | The audit log and the change panel. |
| `users.html` | Settings, Access: the users and the create dialog. |
| `api-tokens.html` | Settings, Access: the tokens and the value-once dialog. |
| `secrets.html` | Settings, Access: the secret metadata and the replace dialog. |
| `settings.html` | The tenant settings and the danger zone. |
| `empty-first-run.html` | The Overview before the first host. |
| `sign-in.html` | The sign-in card. |
| `setup.html` | The first setup. |

## Shell and tenant

The screens use shell A: the top bar and the underline tabs under it.
The tenant part of the breadcrumb is a button. The menu lists the tenants of the account.
A switch shows the toast "Switched to name".
Users, API tokens and secrets live in Settings, group Access.

## Theme

Two themes exist: light and dark.
The script reads the stored choice. Without a stored choice it reads `prefers-color-scheme`.
The button in the top bar switches the theme.

## Component inventory

The third column tells if the element needs keyboard behavior, an ARIA role or a label.

| Element | States | Keyboard or assistive behavior |
|---|---|---|
| Button | default, hover, focus, disabled, loading | Yes: a native `button`. |
| Primary button | default, hover, focus, disabled | Yes: a native `button`. |
| Danger button | default, hover, focus, disabled | Yes: a native `button`. |
| Invisible and link button | default, hover, focus | Yes: a native `button`. |
| Icon button | default, hover, focus, disabled | Yes: an `aria-label` carries the name. |
| Split button | default, open | Yes: two buttons, one menu. |
| Button group and segmented | default, selected | Yes: `aria-pressed` on each button. |
| Text input | default, focus, disabled, invalid | Yes: a label binds to the input. |
| Textarea | default, focus, disabled | Yes: a label binds to the input. |
| Input with prefix or suffix | default, focus | No. |
| Search field | default, focus | Yes: the field has a name. |
| Select | default, open, disabled | Yes: a native `select`. |
| Combobox | default, open, selected | Yes: arrow keys and Escape. |
| Checkbox | unchecked, checked, mixed, disabled | Yes: a native checkbox. |
| Radio | unchecked, checked, disabled | Yes: a native radio group. |
| Toggle switch | on, off, disabled | Yes: a checkbox with a role. |
| Form field | default, hint, error | Yes: `aria-describedby` and `aria-invalid`. |
| Label and topic | default | No. |
| Status label | success, running, queued, warning, failed, cancelled | No: the text carries the state. |
| Counter | default, accent, danger | No. |
| Avatar | small, large, square | No. |
| Banner | info, success, attention, danger | Yes: `role="status"` and a dismiss button. |
| Inline alert | info, success, danger | Yes: a live region for a result. |
| Toast | success, danger, info | Yes: `role="status"` and a dismiss button. |
| Tooltip | default | Yes: a keyboard trigger. |
| Action menu | closed, open, danger item, disabled item | Yes: arrow keys, Escape, `aria-expanded`. |
| Dialog | confirm, destructive, form | Yes: a native `dialog` traps focus. |
| Side panel | closed, open | Yes: a native `dialog` with a right edge. |
| Underline tabs | active, inactive | Yes: arrow keys and a roving tab index. |
| Breadcrumb | default, current | Yes: a `nav` with a label. |
| Tenant switcher | closed, open, current tenant | Yes: a button with `aria-expanded`; the menu items are `menuitemradio`; arrow keys move, Escape closes. |
| Tenant switcher menu | tenants with roles, filter, Platform entry | Yes: the current item carries `aria-checked`; the filter field has a name. |
| Pagination | default, current, disabled | Yes: buttons with a name. |
| Box row list | default, hover, selected | No. |
| Data table | sortable, selected, empty, loading | Yes: `aria-sort` and a row name. |
| Empty state | default | No: the text gives the next action. |
| Key-value list | default | No. |
| Timeline | success, running, failed, queued | No. |
| Log viewer | info, warn, error, debug | No: the level word carries the state. |
| JSON diff | added, deleted, context | No. |
| Copy field | default, copied | Yes: a copy button with a name. |
| Progress bar | determinate, indeterminate | Yes: `role="progressbar"` and a value. |
| Spinner | default | Yes: the text next to it carries the state. |
| Skeleton | text, row, avatar | No. |
| Code and key hint | default | No. |

## Notes

A screen uses custom properties from `mockups.css` only.
The pages name no third party product and no design system.

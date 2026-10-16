# UI principles

These rules apply to each change in the web UI (ADR-0012). The UI is a restrained administration console.

Use the shared components of [ui/COMPONENTS.md](../../ui/COMPONENTS.md). They apply these rules, and lint rules report the markup that does not use them.

## 1. Navigation

1. The breadcrumb in the top bar (`AppShell`) is the only "back" navigation. A page does not have its own back link in the body.
2. A page below a list adds its levels to the breadcrumb, for example `tenant / Restore / <repository> / Snapshot <id>`. A parent level keeps the list state. The filters stay in the query string.
3. A record opens on its own page, with the route `/<list>/<id>`. Do not open a record in a large modal.
4. A record page has the sections on the left and a sticky side panel on the right. The side panel shows a summary or the record parameters, with the label above the value (see `RepositoryEditor`, `ProfileEditor` and `HostPage`).
5. A small form or a confirmation stays in a centered modal.

## 2. Tables

1. A click on a row opens the record. The row holds a real link that covers the row. Ctrl-click and middle-click open a new tab. The Tab key and the Enter key work, and a screen reader reads a link. A text selection does not open the record. Do not add an "Open" button or a separate link style on the name.
2. If a table has no record page, the row opens a side panel.
3. One or two row actions are icon buttons. Each icon button has a Lucide icon, a tooltip and an `aria-label`. Three or more row actions are in one "…" button. Do not mix an icon button and a "…" button. Do not put a text button in a row. A click on a row control does not open the row. Opening the record is not an action and does not count.
4. A row has no link to another entity. For example, the host and the repository in a job row are plain text. Related records open from the record page. The list of the items of an entity is a filter.
5. A list shows only the status pill (`StatusPill`). It does not show a reason or an error text under the pill. The details are on the record page.
6. A table fits in 1440 px without a horizontal scroll. A long name is cut with an ellipsis and a tooltip shows the full value. A date and a time do not wrap.
7. A table has one compact filter bar above it. The bar has a search field, popover facets with counts, period presets, and chips of the applied filters. Each chip has a × button. The bar has a Reset button. A filter applies at once. The page context, for example the repository, is in the page header and not in the filter bar. A large list filters and pages on the server (ADR-0009).

## 3. Status and text

1. A status label is a verb in the past tense or a result: Succeeded, Failed, Cancelled.
2. A job that succeeded with warnings shows "Succeeded" in the warning tone. It has no icon. The tooltip and the `aria-label` give the full meaning.
3. A panel or a section appears only when it has content. Do not show a placeholder panel, for example "appears when…".
4. If a job has one step, do not show an overall progress bar next to the step bar.

## 4. Visual style

1. Use Lucide icons only. Do not draw an SVG by hand.
2. Do not put a colored stripe at the left edge of a panel, a callout or a row.
3. Use one duration token for each spinner. The reduced-motion setting stops the spinners.

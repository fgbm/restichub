# UI principles

These rules apply to each change in the web UI (ADR-0012). The UI is a restrained administration console.

Use the shared components of [ui/COMPONENTS.md](../../ui/COMPONENTS.md). They apply these rules, and lint rules report the markup that does not use them.

## 1. Navigation

1. The breadcrumb in the top bar (`AppShell`) is the only "back" navigation. A page does not have its own back link in the body.
2. A page below a list adds its levels to the breadcrumb, for example `tenant / Restore / <repository> / Snapshot <id>`. A parent level keeps the list state. The filters stay in the query string.
3. A record opens on its own page, with the route `/<list>/<id>`. Do not open a record in a large modal.
4. A record page has the sections on the left and a sticky side panel on the right. The side panel shows a summary or the record parameters, with the label above the value (see `RepositoryEditor`, `ProfileEditor` and `HostPage`).
5. A small form or a confirmation stays in a centered modal.

## 2. Lists and tables

1. A list of entities uses a list view, not a column table. This applies to hosts, jobs, profiles, repositories, plans, channels, secrets, API tokens, users, templates, tenants, snapshots, alerts and audit events. The model is the pull request list of GitHub.
2. Each item has two lines. Line 1 has a type or status icon or pill and the name in bold. Line 2 has muted metadata with " · " between the values, for example `windows · x86_64 · agent 0.1.0 · last contact 24 s ago`.
3. The name is the only link in the item. It is a real link to the record page. Ctrl-click and middle-click open a new tab, and a screen reader reads a link. The remaining part of the item is not clickable, so a person can select the text. Do not add an "Open" button.
4. If an entity has no record page, the name opens a side panel.
5. The row actions are at the right of the item. One or two row actions are icon buttons. Each icon button has a Lucide icon, a tooltip and an `aria-label`. Three or more row actions are in one "…" button. Do not mix an icon button and a "…" button. Do not put a text button in an item. Opening the record is not an action and does not count.
6. An item has no link to another entity. For example, the host and the repository in a job item are plain text. Related records open from the record page. The list of the items of an entity is a filter.
7. An item shows only the status pill (`StatusPill`). It does not show a reason or an error text under the pill. The details are on the record page.
8. A list fits in 1440 px without a horizontal scroll. A long name is cut with an ellipsis and a tooltip shows the full value. A date and a time do not wrap.
9. A search field is above the list. The list header has state tabs at the left. Each tab shows a count, for example "Online 312 · Offline 4 · Revoked 7". For jobs, the tabs are the status groups. A tab is a quick filter.
10. At the right, the list header has facet dropdowns and a Sort dropdown. A facet dropdown has a search field in it. The Sort dropdown sets the field and the order.
11. Each applied filter shows as a chip with a × button. The Reset button removes all filters. A filter applies at once. The page context, for example the repository, is in the page header and not in the list header. A large list filters and pages on the server (ADR-0009).
12. A list has no bulk selection.
13. Use a column table (`DataTable`) only for tabular data: file trees, parameters, numbers and comparisons. A table has the same rules for row actions, links and width as a list.
14. Each list uses the standard pages of `DataTable`: the page numbers, the text "Showing X to Y of Z" and the page size. A list that can become large pages on the server. A small list loads all its items and pages in the browser. Do not add a "Load more" button to a list.
15. A file tree is not a list page. It can load more entries of one folder when the person asks for them. A short preview of a list, for example the recent jobs on a host page, does not page. It has a link to the full list.

## 3. Status and text

1. A status label is a verb in the past tense or a result: Succeeded, Failed, Cancelled.
2. A job that succeeded with warnings shows "Succeeded" in the warning tone. It has no icon. The tooltip and the `aria-label` give the full meaning.
3. A panel or a section appears only when it has content. Do not show a placeholder panel, for example "appears when…".
4. If a job has one step, do not show an overall progress bar next to the step bar.

## 4. Visual style

1. Use Lucide icons only. Do not draw an SVG by hand.
2. Do not put a colored stripe at the left edge of a panel, a callout or a row.
3. Use one duration token for each spinner. The reduced-motion setting stops the spinners.

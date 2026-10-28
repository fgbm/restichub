# UI principles

These rules apply to each change in the web UI (ADR-0012). The UI is a restrained administration console.

Use the shared components of [ui/COMPONENTS.md](../../ui/COMPONENTS.md). They apply these rules, and lint rules report the markup that does not use them.

## 1. Navigation

1. The breadcrumb in the top bar (`AppShell`) is the only "back" navigation. A page does not have its own back link in the body.
2. A page below a list adds its levels to the breadcrumb, for example `tenant / Restore / <repository> / Snapshot <id>`. A parent level keeps the list state. The filters stay in the query string.
3. A record opens on its own page, with the route `/<list>/<id>`. Do not open a record in a large modal.
4. A record page has the sections on the left and a sticky side panel on the right. The side panel shows a summary or the record parameters, with the label above the value (see `RepositoryEditor`, `ProfileEditor` and `HostPage`). One exception: a settings record puts the side panel above the sections when the content column is at most 900 px wide (owner decision, `RecordPage`).
5. A small form or a confirmation stays in a centered modal.
6. A record page shows one sentence that defines one record, for example "A host is a computer of the organization with the ResticHub agent." The list page of the record shows the same sentence. Thus the sentence says what one record is, and not what the collection is. Do not add a second sentence about the form. Put a fact that a person needs in the hint of the field or of the section.
7. A record page shows the delete and other destructive actions only in the section «Dangerous actions» at the end of the page.
8. A section keeps its subnav on each page of the section: on the list and on the record page. Thus the content column does not move between the pages.

## 2. Lists and tables

1. A list of entities uses a list view, not a column table. This applies to hosts, jobs, profiles, repositories, plans, channels, secrets, API tokens, users, templates, tenants, snapshots, alerts and audit events. The model is the pull request list of GitHub.
2. Each item has two lines. Line 1 has a status or type icon in a fixed column at the left and the name in bold. Line 2 has muted fields with " · " between them, for example `last contact 24 s ago · Windows · x86_64 · agent 0.4.2`. The fields are in the sequence of their priority. An empty field does not show. A long line 2 is cut at the end. On a narrow screen, line 2 shows only the first two fields. On a narrow screen, the chip of the profile tag of a snapshot also stays, and the other tags go.
3. The name is the only link in the item. It is a real link to the record page. Ctrl-click and middle-click open a new tab, and a screen reader reads a link. The remaining part of the item is not clickable, so a person can select the text. Do not add an "Open" button. One exception: the chip of a profile tag of a snapshot item is a link to the profile (rule 2.6).
4. If an entity has no record page, the name opens a side panel.
5. The row actions are at the right of the item. One or two row actions are icon buttons. Each icon button has a Lucide icon, a tooltip and an `aria-label`. Three or more row actions are in one "…" button. Do not mix an icon button and a "…" button. Do not put a text button in an item. A chip that shows the folded fields as "+N" is a disclosure, and it is not a row action. Opening the record is not an action and does not count.
6. An item has no link to another entity. For example, the host and the repository in a job item are plain text. Related records open from the record page. The list of the items of an entity is a filter. One exception: each profile tag of a snapshot item is a chip, and that chip links to its profile. A person asks for the profile of a snapshot more often than for the snapshot itself.
7. An item shows a status pill (`StatusPill`) only for a state that is not usual, for example "Offline" or "Disabled". The usual state, for example "Online", has no pill. An item has one pill at most. It does not show a reason or an error text under the pill. The details are on the record page.
8. A list fits in 1440 px without a horizontal scroll. A long name is cut with an ellipsis and a tooltip shows the full value. A date and a time do not wrap.
9. A search field is above the list. The list header has state tabs at the left. Each tab shows a count, for example "Online 312 · Offline 4 · Revoked 7". For jobs, the tabs are the status groups. A tab is a quick filter.
10. At the right, the list header has facet dropdowns and a Sort dropdown. A facet dropdown has a search field in it. The Sort dropdown sets the field and the order.
11. Each applied filter shows as a chip with a × button. The Reset button removes all filters. A filter applies at once. The page context, for example the repository, is in the page header and not in the list header. A large list filters and pages on the server (ADR-0009).
12. A list has no bulk selection.
13. Use a column table (`DataTable`) only for tabular data: file trees, parameters, numbers and comparisons. A table has the same rules for row actions, links and width as a list.
14. Each list uses the standard pages of `ListView` and `DataTable`: the page numbers, the text "Showing X to Y of Z" and the page size. A list that can become large pages on the server. A small list loads all its items and pages in the browser. Do not add a "Load more" button to a list.
15. A file tree is not a list page. It can load more entries of one folder when the person asks for them. A short preview of a list, for example the recent jobs on a host page, does not page. It has a link to the full list.
16. A list where a person finds an item by its time has a heading for each day: the snapshots and the audit events.
17. The duration of a job and the size of a snapshot are in a right-aligned column with figures of equal width. Then a person can compare them. On a narrow screen, the number goes to the end of line 2.

## 3. Status and text

1. A status label is a verb in the past tense or a result: Succeeded, Failed, Cancelled.
2. A job that succeeded with warnings shows "Succeeded" in the warning tone. It has no icon. The tooltip and the `aria-label` give the full meaning.
3. A page header shows each fact one time. Do not add a second badge that repeats the status, for example "Live" next to "Running".
4. A panel or a section appears only when it has content. Do not show a placeholder panel, for example "appears when…".
5. If a job has one step, do not show an overall progress bar next to the step bar.
6. Only four tones have a color. Danger (red) is for data that is not safe or work that failed. Warning (yellow) is for a problem that comes soon, or work that finished with warnings. Running (blue) is for work that runs now. Success (green) is for work that finished well and for a healthy live state, for example a host that is online. A usual state is grey, for example a cancelled job or a disabled plan. A success has a green icon and no pill in a list. A warning pill has its own short label, not the label of a success. Each state of each entity has one label, one tone and one Lucide icon (`$lib/vocabulary.ts`).
7. A list shows the time in the formats of `$lib/list-format.ts`. Freshness is a short distance ("7 min ago"). A run is the day and the hours ("today 02:00"). An audit time has seconds and is below the heading of its day. A validity end is a date, or a distance when less than 30 days remain. The tooltip of a time shows the full date, the seconds and the time zone.
8. A list page has no static documentation text. Do not add a note, a footnote or a help panel that does not change with the state and does not lead to an action. The documentation (`docs/`) holds this text. A list page shows a banner only for a state: an error, a warning, an empty list or an action that is necessary. The one-line description in the page header stays.
9. A locked field (disabled or read-only) shows its value. If it has no value, it shows the muted text "Not set" ("Не задано"). It never shows an example placeholder, because an example in a locked field looks like a stored value. `TextField`, `Select` and `Combobox` apply this rule.
10. In an editable field, an example placeholder starts with "e.g." ("например,"), for example "e.g. snapshots/prod". A placeholder that is not an example, for example the hint of a search field, does not have this prefix.
11. A person enters a set of values, for example labels, tags, host names or email addresses, with the tag input (`TagInput`). Each value shows as a chip. Do not ask for one text with separators, for example "comma-separated". A path or a pattern can hold a comma or a space. Thus a list of paths or patterns is a text area with one value on each line. The lint rule `ui/no-separated-text` reports a catalog text that asks for separators.

## 4. Visual style

1. Use Lucide icons only. Do not draw an SVG by hand.
2. Do not put a colored stripe at the left edge of a panel, a callout or a row.
3. Use one duration token for each spinner (`--rh-spin-duration`). A spinner shows the status of work that runs now, thus it is not decoration. When the reduced-motion setting is on, a spinner continues to turn at half speed (`--rh-spin-duration-reduced`). All other motion stops: a skeleton shows a static block and a transition is almost instant. Each spinner has the `rh-spinner` class.
4. A section never holds a framed box. Put a secondary or rare field under a `Disclosure`, a toggle line without a frame. Do not use a native `<details>` with a border or a nested card for this.
5. A skeleton appears only when a load lasts longer than 200 ms. A load that ends sooner shows no placeholder, because a skeleton that lives for one frame looks like a fault.

## 5. Forms

1. A field with a rule, for example the minimum length of a password, shows the rule below the field. While the person types, the text shows the progress, for example "6 of 12 characters", in the usual tone. The text changes to the danger tone only after the person leaves the field or tries to send the form. The field refers to the text with `aria-describedby`.
2. The rule in the UI is the same as the rule of the server. A test compares them.
3. Each field that sets a password uses `NewPasswordField`.
4. A primary action stays enabled when a field is not correct. When the person tries to send the form, each field that is not correct shows its error. The first of these fields gets the focus.
5. If a primary action is disabled, a text next to the action or the tooltip of the action gives the reason.
6. The on/off state of a record is a `Switch`. Examples are a plan, a channel, an alert rule and a maintenance task. The label of the switch says what it does, for example "Run on schedule" or "Send alerts". Do not use a `Checkbox` with the label "Enabled". A `Checkbox` is for an option of a record or of an action, for example "Use VSS" or "Dry run".
7. Put the switch in the header of the section that it controls. In a dialog without sections, make it the last field (`SwitchField`). The field is a row: the label at the left, the switch at the right and a help line below the label. A test fails when a `Checkbox` is bound to a field named `enabled`.
8. A record that is off shows the pill "Disabled" next to its name in the page header and in its list row.

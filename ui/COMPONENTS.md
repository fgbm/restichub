# UI layout and table components

These components apply the rules of [design/style/ui-principles.md](../design/style/ui-principles.md). Use them for each new page and each new table. Then the page obeys the rules without more work.

## Page

`Page` (`$lib/components/layout`) is the frame of a page. It shows the page header and sets the document title. It also declares the breadcrumb levels of the page.

| Property      | Function                                                                                                                                                                         |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `title`       | The heading and the document title.                                                                                                                                              |
| `crumbs`      | The breadcrumb levels below the section, for example `[{ label: hostname }]`. The last level is the current page. A list page has no levels.                                     |
| `listQuery`   | The query string of the section level. Use it when the address of the page holds the filters of its list. By default, the section level keeps the last query string of the list. |
| `description` | The text below the heading.                                                                                                                                                      |
| `count`       | The counter next to the heading.                                                                                                                                                 |
| `status`      | A snippet next to the heading, for example a `StatusPill`.                                                                                                                       |
| `context`     | A snippet for the page context, for example the repository that a list belongs to. Put a `<label>` without a colon before the control. The header makes the label muted.         |
| `actions`     | A snippet with the primary actions of the page.                                                                                                                                  |
| `loading`     | While it is `true`, the page shows a placeholder instead of the header and the body.                                                                                             |
| `error`       | A load error. The page shows the error instead of the header and the body.                                                                                                       |

The header puts the context and the actions at the right end of the title row, in this sequence. Below 720 px, they go below the description and the context uses the full width.

The top bar (`AppShell`) shows the section levels from the navigation, then the levels of the page. Each level above the current page is a link. The section level links to the list with its last query string, so the list opens with the same filters and the same page. Do not add a back link to the page body. The lint rule `ui/no-back-link` reports a back link.

`PageMeta` declares only the title and the breadcrumb levels, for a page that draws its own header.

## RecordPage

`RecordPage` is a `Page` for one record, with the route `/<list>/<id>`. It has the sections on the left and a sticky side panel on the right. Below 1024 px, the side panel moves above the sections and does not stick.

| Property   | Function                                                                              |
| ---------- | ------------------------------------------------------------------------------------- |
| `side`     | A snippet for the side panel. Put one or more `SummaryPanel` components in it.        |
| `banners`  | A snippet for the notices about the full record, above the sections.                  |
| `footer`   | A snippet for a sticky bar below the sections, for example the save bar of an editor. |
| `onsubmit` | When it is set, the sections and the footer are one form.                             |
| `crumbs`   | By default, the title is the only level.                                              |

The other properties are the same as in `Page`.

## SummaryPanel

`SummaryPanel` is a panel of the side panel. It shows `items` (`KeyValueItem[]`) with the label above the value. An item can have a `render` snippet instead of a text value. The panel does not appear when it has no items and no content.

## ListView

`ListView` (`$lib/components/ui`) is the list of the entities: hosts, jobs, plans, repositories, profiles, snapshots, alerts, audit events, channels, users, tokens, secrets, templates and organizations. Each item has two lines. Use `DataTable` only for tabular data.

| Property                        | Function                                                                                                                                                   |
| ------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `items`, `itemKey`, `label`     | The items, the key of an item and the accessible name of the list.                                                                                         |
| `icon(item)`                    | `ListIcon`: `{ name, tone?, spin?, label? }`. The icon is in a fixed column at the left. Only the `danger`, `warning` and `running` tones color it.        |
| `title(item)`, `titleMono`      | The name in bold. A long name is cut with an ellipsis, and a tooltip shows the full name.                                                                  |
| `href(item)`                    | The resolved address of the record page. The name is the only link of the item. The remaining part of the item is not clickable.                           |
| `onOpen(item)`                  | For an entity without a record page. The name opens a side panel.                                                                                          |
| `titleLabel(item)`              | The accessible name of the name, when the name is too short, for example the time of a snapshot.                                                           |
| `pill(item)`                    | `ListPill` or `null`: `{ tone, label, full? }`. Return a pill only for a state that is not usual.                                                          |
| `meta(item)`                    | `ListMeta[]`: the fields of line 2 in the sequence of their priority. A field is a text or `{ text, mono?, time?, title? }`. An empty field does not show. |
| `aside(item)`                   | A number in a right-aligned column of 96 px, for example a duration or a size.                                                                             |
| `actions(item)`, `moreLabel`    | `RowAction[]` as in `DataTable`. One or two actions are icon buttons. Three or more actions are in one "…" menu.                                           |
| `groupBy(item)`                 | The heading of the group of the item, for example the day. Sort the items by the group.                                                                    |
| `mode`, `page`, `pageSize`, …   | The pages, as in `DataTable`.                                                                                                                              |
| `filtered`, `onReset`           | When a filter applies, an empty list shows "Nothing found" and a "Reset filters" button.                                                                   |
| `emptyTitle`, `emptyActions`, … | The first-run state of an empty list, with the primary action.                                                                                             |
| `toolbar`                       | A snippet above the list. Put the `FilterBar` in it.                                                                                                       |
| `flush`                         | For a list in a panel body.                                                                                                                                |

When the list is narrower than 560 px, line 2 shows only the first two fields. The number of `aside` goes to the end of line 2.

`$lib/vocabulary.ts` has the one label, tone and icon of each entity state, for example `jobState`, `hostState` and `planState`. Use `iconOf(state)` and `pillOf(state)` for the `icon` and `pill` properties. `ENTITY_ICONS` has the type icon of each entity. `$lib/list-format.ts` has the formats of the time, the duration, the size and the plural forms.

## DataTable

`DataTable` (`$lib/components/ui`) is the only table component. Use it for tabular data only, for example parameters, numbers and comparisons. A list of entities is a `ListView`. The lint rule `ui/no-raw-table` reports a raw `<table>` element and the older `Table` component.

| Property                                           | Function                                                                                                                                                                                                                                                                      |
| -------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `columns`                                          | `TableColumn[]`. See the column options below.                                                                                                                                                                                                                                |
| `rows`, `rowKey`                                   | The rows and the key of a row.                                                                                                                                                                                                                                                |
| `rowHref(row)`                                     | The resolved address of the record page. The primary cell holds a real link. A click on the row opens the record. A Ctrl click or a middle click opens a new tab. A click that ends a text selection does not open the record.                                                |
| `onRowOpen(row)`                                   | For a table without a record page. The row opens a side panel.                                                                                                                                                                                                                |
| `actions(row)`                                     | `RowAction[]`: `{ id, label, icon, onSelect, hint?, danger?, hidden?, disabled? }`. The table shows one or two visible actions as icon buttons with a tooltip and an accessible name. It shows three or more in one "…" menu. A click on a row control does not open the row. |
| `moreLabel(row)`                                   | The accessible name of the "…" button.                                                                                                                                                                                                                                        |
| `mode`                                             | `client` (default): the table sorts and pages the rows. `server`: the rows are one page from the server.                                                                                                                                                                      |
| `page`, `pageSize`, `total`, `sort`                | The page state. In the server mode, `total` is the number of rows on all pages. `page`, `pageSize` and `sort` are bindable.                                                                                                                                                   |
| `pageSizes`                                        | The choices of the page size. By default, 10, 25, 50 and 100. The server mode shows the choice only with `onPageSizeChange`.                                                                                                                                                  |
| `totalCapped`                                      | The server mode: the server counted the rows only up to its limit. The footer shows the total with a "+".                                                                                                                                                                     |
| `onPageChange`, `onPageSizeChange`, `onSortChange` | The server mode loads another page, another page size or another order.                                                                                                                                                                                                       |
| `toolbar`                                          | A snippet above the table. Put the `FilterBar` in it.                                                                                                                                                                                                                         |
| `loading`, `error`, `empty*`                       | The states of the table.                                                                                                                                                                                                                                                      |
| `flush`                                            | For a table in a panel body.                                                                                                                                                                                                                                                  |

The footer shows "Showing X to Y of Z", the page size and the page numbers. It shows the first page, the last page and the pages next to the current page. A list has no "Load more" button (UI principles 2.14). To keep the page and the page size in the query string, add the `page` and `size` keys to `queryFilters`.

Column options (`TableColumn`):

- `primary`: the cell that holds the record link. By default, it is the first column.
- `truncate`: `true`, `sm`, `md` or `lg`. The cell cuts a long value with an ellipsis, and a tooltip shows the full value.
- `date`: `relative` or `absolute`. The value is an RFC 3339 time. The cell shows a `<time>` element on one line, with the full time in a tooltip.
- `nowrap`, `align`, `mono`, `hideNarrow`, `sortable`, `value`, `cell`, `badge` and `render` are as in the older table.

Do not put a link to another entity in a cell. Do not put a status text below a `StatusPill`. The lint rule `ui/no-row-actions-snippet` reports the older `rowActions` snippet.

## FilterBar

`FilterBar` (`$lib/components/ui`) is the one compact filter bar above a table. Each change applies at once.

| Property   | Function                                                                                                                                                                                                                                                                                       |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `label`    | The accessible name of the bar.                                                                                                                                                                                                                                                                |
| `value`    | The filter state (`FilterState`): one string for each query parameter. A facet with more than one value joins the values with a comma.                                                                                                                                                         |
| `search`   | `{ key, label, placeholder?, mono?, id? }`: the search field.                                                                                                                                                                                                                                  |
| `facets`   | `FilterFacet[]`: `{ key, label, options: { value, label?, count?, description? }[], multiple?, hint?, search? }`. A facet is a popover with the counts. The `hint` is a short muted line at the top of the popover. A `description` is a secondary line below the value, as in `RecordPicker`. |
| `period`   | Show the period facet with the presets and a custom range. It writes the `period`, `from` and `to` keys.                                                                                                                                                                                       |
| `states`   | `FilterTabs`: `{ key?, options: { value, label, count?, danger? }[], default }`. The state tabs with counts. A tab writes the `state` key. A `danger` count is red when it is more than zero.                                                                                                  |
| `total`    | The count of an entity without states, in place of the tabs. `totalNote` adds a short text, for example the period.                                                                                                                                                                            |
| `sort`     | `FilterSort`: `{ options: { value, label, order? }[], default }`. The Sort menu writes the `sort` and `order` keys. An empty key is the default.                                                                                                                                               |
| `hint`     | A short help text behind an information icon.                                                                                                                                                                                                                                                  |
| `end`      | A snippet at the right end of the row, for example the age of the list and a refresh button.                                                                                                                                                                                                   |
| `onchange` | Receives a patch of the state.                                                                                                                                                                                                                                                                 |
| `onreset`  | By default, the Reset button sets each key of the bar to an empty value.                                                                                                                                                                                                                       |

Inside a `ListView`, the bar is the list header. The search field is above it. The tabs or the total are at the left, and the facets and the Sort menu are at the right. Below 640 px, the facets go into one "Filters" popover and the Sort menu becomes an icon button. A facet with seven values or more, or with `search: true`, has a search field in its popover. The search also finds the text of the secondary line. The Reset button also returns the tabs to the default tab. It does not change the sort.

The bar shows a chip with a × button for each applied filter and a Reset button when a filter applies. Put the page context, for example the repository, in the page header and not in the bar.

To keep the filters in the query string, use `queryFilters(keys)` from `$lib/stores/query-filters.svelte.ts`. It reads the keys from the address. `update(patch)` writes them back at once and replaces the history entry. A change of a filter sets the `page` key to the first page. `$lib/filters.ts` has the helpers for a client-side filter: `matchesSearch`, `splitList`, `inPeriod` and `facetCounts`.

## TagInput

`TagInput` (`$lib/components/ui`) is the field for a set of values: labels, tags, host names and email addresses. Each value shows as a chip with a × button. Do not ask for one text with separators (UI principles 3.11). A list of paths or patterns stays a text area with one value on each line, because a path can hold a comma or a space.

| Property                                      | Function                                                                                    |
| --------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `value`                                       | The values, a string array. It is bindable. Send the same array to the API.                 |
| `label`, `help`, `error`, `id`, `placeholder` | As in `TextField`. An `error` from the server replaces the reason of the field.             |
| `separator`                                   | A regular expression for the characters that end a value. By default, a comma or a space.   |
| `check(value)`                                | Returns the reason why a value is not valid, or `null`.                                     |
| `maxLength`                                   | The maximum number of characters in one value. Set it to the limit of the server.           |
| `max`                                         | The maximum number of values. Set it to the limit of the server.                            |
| `mono`                                        | Shows the values in the monospace font, for example labels, host names and email addresses. |

- A separator, Enter or the loss of focus adds the value. A paste of `a, b` adds two values.
- A value that is already in the list is not added again.
- A value that is not valid stays in the field. The field shows the value and the reason, for example "Bad_label: Use only the letters a-z, the digits 0-9 and the hyphen." When the text changes, the reason goes away.
- The × button of a chip removes the value. Backspace in an empty field removes the last value. Enter in an empty field sends the form.
- Each × button has the accessible name "Remove <value>". The field has `aria-invalid` and refers to the reason with `aria-describedby`. The reason has the `alert` role.

## RecordPicker

`RecordPicker` (`$lib/components/ui`) is the field that selects one record when the name of the record is not sufficient, for example a profile and its repository. Each option has two lines: the label, and a muted secondary line with texts and small tag chips. The closed field shows the label and the first part of the secondary line, for example "user-profiles · test-s3-yandex". The open list has a search field at the top. The search finds the label and each part of the secondary line, and it ignores the case of the letters.

| Property                                   | Function                                                                                                       |
| ------------------------------------------ | -------------------------------------------------------------------------------------------------------------- |
| `value`                                    | The value of the selected option. It is bindable.                                                              |
| `options`                                  | `RecordOption[]`: `{ value, label, description? }`. A part of `description` is a text or `{ tag }` for a chip. |
| `label`, `help`, `error`, `id`, `required` | As in `Select`.                                                                                                |
| `disabled`                                 | A locked field. An empty locked field shows "Not set" (UI principles 3.9).                                     |
| `placeholder`, `searchPlaceholder`         | The text of an empty field and the text of the empty search field.                                             |
| `emptyLabel`                               | The text of the list when the search finds no option. By default, "No matches".                                |
| `allLabel`                                 | Adds a first option with the empty value, for example "All" in a filter.                                       |
| `ariaLabel`                                | The accessible name of a field without a visible label.                                                        |
| `onchange(value)`                          | Receives the value that the person selects.                                                                    |

- A click, Enter, Space, the arrow keys or a typed character opens the list. A typed character starts the search. The focus goes to the search field.
- The arrow keys move through the options. Enter selects an option. Escape closes the list. When the list closes, the focus goes back to the field.
- The field has the `combobox` role. Its accessible name is the label and the selected label, and its description is the first part of the secondary line. The secondary line of an option is the accessible description of the option.

`SelectOption` does not have a secondary line, because a native `<select>` cannot show one. A facet of `FilterBar` takes the same secondary line in `FilterOption.description`. Set `search: true` on the facet to show the search field when the facet has fewer than seven values.

`ProfilePicker` (`$lib/components`) is a `RecordPicker` for a profile. It makes the options from the profiles, the repositories and the hosts. An option shows the profile name, the repository name and a chip with the backend kind. For a local repository, it also shows the repository host. `profileDescription` in `$lib/profile-picker.ts` gives the same secondary line for a facet.

## Forms

`NewPasswordField` (`$lib/components/ui`) is the field for each new password. Use it for the initial setup, a new user, a password reset and the grant of the Platform Admin right. Below the field, it shows the password rule of the server from `$lib/password-policy.ts` and the progress, for example "6 of 12 characters" (UI principles 5.1). The text is in the danger tone after the person leaves the field or after a submit attempt (`submitted`). A value that is too long shows the danger tone at once. An `error` from the server replaces the text.

A form does not disable its primary action for a value that is not correct (UI principles 5.4). The submit handler sets `submitted`, calls `focusFirstInvalid` from `$lib/form.ts` and stops when it returns `true`. `emptyError(submitted, value)` gives the error of an empty required field.

## Lint rules

The rules are in `eslint-rules/ui-principles.js`, and `npm run lint` runs them in CI:

- `ui/no-raw-table`: a raw `<table>` or the older `Table` component.
- `ui/no-entity-table`: a `DataTable` with `rowHref`, `onRowOpen` or `detailTitle`. A table whose rows open a record is a list of entities, so use `ListView`.
- `ui/no-row-actions-snippet`: the older `rowActions` snippet.
- `ui/no-back-link`: a link with a back text, a back message key or an `arrow-left` icon.
- `ui/no-load-more`: a "Load more" text or a `loadMore` or `showMore` message key.
- `ui/no-separated-text`: a text in the message catalogs that asks for values with separators, for example "через запятую" or "comma-separated". Use `TagInput`.

Each screen obeys these rules. Do not add an exception to `eslint.config.js`. The file tree (`SnapshotTree`) is the only exception of `ui/no-load-more`, because a file tree is not a list page (UI principles 2.15).

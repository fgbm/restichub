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
| `context`     | A snippet for the page context, for example the repository that a list belongs to.                                                                                               |
| `actions`     | A snippet with the primary actions of the page.                                                                                                                                  |
| `loading`     | While it is `true`, the page shows a placeholder instead of the header and the body.                                                                                             |
| `error`       | A load error. The page shows the error instead of the header and the body.                                                                                                       |

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

## DataTable

`DataTable` (`$lib/components/ui`) is the only table component. The lint rule `ui/no-raw-table` reports a raw `<table>` element and the older `Table` component.

| Property                            | Function                                                                                                                                                                                                                                                                      |
| ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `columns`                           | `TableColumn[]`. See the column options below.                                                                                                                                                                                                                                |
| `rows`, `rowKey`                    | The rows and the key of a row.                                                                                                                                                                                                                                                |
| `rowHref(row)`                      | The resolved address of the record page. The primary cell holds a real link. A click on the row opens the record. A Ctrl click or a middle click opens a new tab. A click that ends a text selection does not open the record.                                                |
| `onRowOpen(row)`                    | For a table without a record page. The row opens a side panel.                                                                                                                                                                                                                |
| `actions(row)`                      | `RowAction[]`: `{ id, label, icon, onSelect, hint?, danger?, hidden?, disabled? }`. The table shows one or two visible actions as icon buttons with a tooltip and an accessible name. It shows three or more in one "…" menu. A click on a row control does not open the row. |
| `moreLabel(row)`                    | The accessible name of the "…" button.                                                                                                                                                                                                                                        |
| `mode`                              | `client` (default): the table sorts and pages the rows. `server`: the rows are one page from the server.                                                                                                                                                                      |
| `page`, `pageSize`, `total`, `sort` | The page state. In the server mode, `total` is the number of rows on all pages. `page` and `sort` are bindable.                                                                                                                                                               |
| `onPageChange`, `onSortChange`      | The server mode loads another page or another order.                                                                                                                                                                                                                          |
| `toolbar`                           | A snippet above the table. Put the `FilterBar` in it.                                                                                                                                                                                                                         |
| `loading`, `error`, `empty*`        | The states of the table.                                                                                                                                                                                                                                                      |
| `flush`                             | For a table in a panel body.                                                                                                                                                                                                                                                  |

Column options (`TableColumn`):

- `primary`: the cell that holds the record link. By default, it is the first column.
- `truncate`: `true`, `sm`, `md` or `lg`. The cell cuts a long value with an ellipsis, and a tooltip shows the full value.
- `date`: `relative` or `absolute`. The value is an RFC 3339 time. The cell shows a `<time>` element on one line, with the full time in a tooltip.
- `nowrap`, `align`, `mono`, `hideNarrow`, `sortable`, `value`, `cell`, `badge` and `render` are as in the older table.

Do not put a link to another entity in a cell. Do not put a status text below a `StatusPill`. The lint rule `ui/no-row-actions-snippet` reports the older `rowActions` snippet.

## FilterBar

`FilterBar` (`$lib/components/ui`) is the one compact filter bar above a table. Each change applies at once.

| Property   | Function                                                                                                                               |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `label`    | The accessible name of the bar.                                                                                                        |
| `value`    | The filter state (`FilterState`): one string for each query parameter. A facet with more than one value joins the values with a comma. |
| `search`   | `{ key, label, placeholder?, mono?, id? }`: the search field.                                                                          |
| `facets`   | `FilterFacet[]`: `{ key, label, options: { value, label?, count? }[], multiple? }`. A facet is a popover with the counts.              |
| `period`   | Show the period facet with the presets and a custom range. It writes the `period`, `from` and `to` keys.                               |
| `hint`     | A short help text behind an information icon.                                                                                          |
| `onchange` | Receives a patch of the state.                                                                                                         |
| `onreset`  | By default, the Reset button sets each key of the bar to an empty value.                                                               |

The bar shows a chip with a × button for each applied filter and a Reset button when a filter applies. Put the page context, for example the repository, in the page header and not in the bar.

To keep the filters in the query string, use `queryFilters(keys)` from `$lib/stores/query-filters.svelte.ts`. It reads the keys from the address. `update(patch)` writes them back at once and replaces the history entry. A change of a filter sets the `page` key to the first page. `$lib/filters.ts` has the helpers for a client-side filter: `matchesSearch`, `splitList`, `inPeriod` and `facetCounts`.

## Lint rules

The rules are in `eslint-rules/ui-principles.js`, and `npm run lint` runs them in CI:

- `ui/no-raw-table`: a raw `<table>` or the older `Table` component.
- `ui/no-row-actions-snippet`: the older `rowActions` snippet.
- `ui/no-back-link`: a link with a back text, a back message key or an `arrow-left` icon.

Each screen obeys these rules. Do not add an exception to `eslint.config.js`.

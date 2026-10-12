# ResticHub web UI

The web UI is the administration console of the ResticHub control plane. It is a Svelte 5 single-page application (ADR-0012). The control plane binary serves the built files at the root. The public API is under `/api/v1`.

## Requirements

You must have Node.js 22 or later. Use `npm`.

## Install

```sh
npm ci
```

## Development

Start the development server:

```sh
npm run dev
```

The server listens on `http://localhost:5173`. Vite sends the requests under `/api/` to the control plane at `http://127.0.0.1:8080`. Start the control plane before you open the UI.

To work without a control plane, use the mock service:

```sh
VITE_USE_MOCK=true npm run dev
```

The mock service holds test data in memory. Do not use it in production.

## Run locally

The Compose file at the repository root starts the full stack: PostgreSQL, the migrations and the control plane with the embedded UI.

1. Create `.env` with random development secrets:

   ```sh
   ./scripts/dev-env.sh
   ```

2. Build and start the stack:

   ```sh
   docker compose up --build
   ```

3. Open `http://127.0.0.1:8080`.

4. Read the one-time setup token from the control plane log:

   ```sh
   docker compose logs cp
   ```

   The token is on the line with `setup_token`. Use it on the setup page to create the first Platform Admin.

5. Stop the stack and remove the data volume:

   ```sh
   docker compose down -v
   ```

The Compose file is for development only. It turns on insecure cookies and sets development passwords.

## Scripts

| Script            | Function                                                     |
| ----------------- | ------------------------------------------------------------ |
| `npm run dev`     | Start the development server.                                |
| `npm run build`   | Build the single-page application into `build/`.             |
| `npm run preview` | Serve the built files.                                       |
| `npm run check`   | Run `svelte-check` with the strict TypeScript configuration. |
| `npm run lint`    | Run Prettier in check mode and ESLint.                       |
| `npm run format`  | Write the Prettier format.                                   |
| `npm test`        | Run the unit tests with Vitest.                              |
| `npm run gen:api` | Generate the API types from the OpenAPI document.            |

## Structure

| Path                        | Contents                                                                                 |
| --------------------------- | ---------------------------------------------------------------------------------------- |
| `src/routes/`               | SvelteKit routes. `(app)` holds the signed-in screens, `(auth)` holds sign-in and setup. |
| `src/lib/components/`       | The user interface components.                                                           |
| `src/lib/screens/`          | Screens that more than one route uses.                                                   |
| `src/lib/api/`              | The fetch client, the endpoint list and the domain types.                                |
| `src/lib/i18n/`             | The English and Russian message catalogs.                                                |
| `src/lib/stores/`           | The session, theme and notification state.                                               |
| `src/lib/styles/tokens.css` | The design tokens.                                                                       |

## Style rules

- Put every color, space, font and radius in `src/lib/styles/tokens.css`. A component uses the custom properties only.
- Do not add a CSS framework. Write plain CSS in the component.
- The console is restrained: neutral gray, one accent color, thin borders, small radii, no gradients.
- Support the light theme and the dark theme. Support English and Russian.
- Do not name a third-party product in code, comments or documentation.

## API client

The file `src/lib/api/schema.d.ts` is generated from the OpenAPI document. The command `npm run gen:api` writes it. Start the control plane before you run the command. The control plane serves the document at `/api/v1/openapi.json`. Set `OPENAPI_URL` to use another address. The endpoint paths live in `src/lib/api/endpoints.ts`.

The client sends the header `X-ResticHub-CSRF: 1` on POST, PUT, PATCH and DELETE. It sends the cookie with `credentials: 'same-origin'`. On status 401 it redirects to `/sign-in?next=...`.

## Localization

The catalogs `src/lib/i18n/en.ts` and `src/lib/i18n/ru.ts` must have the same keys. TypeScript enforces this. Do not write user-visible text in a component. Use the function `t`.

## License

ResticHub is licensed under the Apache License 2.0. Read the [LICENSE](../LICENSE) file.

/// <reference types="vite/client" />

interface ImportMetaEnv {
	/** Use the in-memory mock service. Set to `true` for development and tests. */
	readonly VITE_USE_MOCK?: string;
}

interface ImportMeta {
	readonly env: ImportMetaEnv;
}

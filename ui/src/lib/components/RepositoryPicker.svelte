<script lang="ts">
	import { t } from '#lib/i18n/index.ts';
	import {
		repositoryCount,
		repositoryOptions,
		type RepositoryRef
	} from '#lib/repository-picker.ts';
	import { RecordPicker } from './ui/index.ts';

	let {
		value = $bindable(''),
		repositories,
		agents = [],
		label = t('plans.repository'),
		...rest
	}: {
		value?: string;
		repositories: readonly RepositoryRef[];
		/** The host of a local path is part of the secondary line. */
		agents?: readonly { id: string; hostname: string }[];
		label?: string;
		id?: string;
		help?: string;
		error?: string;
		required?: boolean;
		disabled?: boolean;
		placeholder?: string;
		ariaLabel?: string;
		onchange?: (value: string) => void;
	} = $props();
</script>

<RecordPicker
	bind:value
	{label}
	options={repositoryOptions(repositories, agents)}
	searchPlaceholder={t('repositoryPicker.search')}
	emptyLabel={repositories.length === 0 ? t('repositoryPicker.none') : undefined}
	countLabel={repositoryCount}
	{...rest}
/>

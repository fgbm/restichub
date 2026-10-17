<script lang="ts">
	import type { Agent, Profile, Repository } from '#lib/api/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import { profileOptions } from '#lib/profile-picker.ts';
	import { RecordPicker } from './ui/index.ts';

	let {
		value = $bindable(''),
		profiles,
		repositories,
		agents,
		label = t('plans.profile'),
		...rest
	}: {
		value?: string;
		profiles: readonly Pick<Profile, 'id' | 'name' | 'repository_id'>[];
		repositories: readonly Pick<Repository, 'id' | 'name' | 'backend' | 'maintenance_agent_id'>[];
		agents: readonly Pick<Agent, 'id' | 'hostname'>[];
		label?: string;
		id?: string;
		help?: string;
		error?: string;
		required?: boolean;
		disabled?: boolean;
		placeholder?: string;
		allLabel?: string;
		onchange?: (value: string) => void;
	} = $props();
</script>

<RecordPicker
	bind:value
	{label}
	options={profileOptions(profiles, repositories, agents)}
	searchPlaceholder={t('profilePicker.search')}
	emptyLabel={profiles.length === 0 ? t('profilePicker.none') : undefined}
	{...rest}
/>

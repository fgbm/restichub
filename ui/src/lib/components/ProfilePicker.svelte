<script lang="ts">
	import type { Profile } from '#lib/api/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import { plural } from '#lib/list-format.ts';
	import { profileOptions } from '#lib/profile-picker.ts';
	import { RecordPicker } from './ui/index.ts';

	let {
		value = $bindable(''),
		profiles,
		label = t('plans.profile'),
		...rest
	}: {
		value?: string;
		profiles: readonly (Pick<Profile, 'id' | 'name' | 'paths' | 'retention'> & {
			parameters?: Profile['parameters'];
		})[];
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
	options={profileOptions(profiles)}
	searchPlaceholder={t('profilePicker.search')}
	emptyLabel={profiles.length === 0 ? t('profilePicker.none') : undefined}
	countLabel={(count) => plural('profilePicker.count', count)}
	{...rest}
/>

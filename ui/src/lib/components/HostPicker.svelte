<script lang="ts">
	import { t } from '#lib/i18n/index.ts';
	import { hostCount, hostOptions, type HostRef } from '#lib/host-picker.ts';
	import { RecordPicker } from './ui/index.ts';

	let {
		value = $bindable(''),
		hosts,
		reason,
		label = t('plans.host'),
		...rest
	}: {
		value?: string;
		hosts: readonly HostRef[];
		/** Why a host cannot be selected; such a host shows dimmed with the reason. */
		reason?: (host: HostRef) => string | null | undefined;
		label?: string;
		id?: string;
		help?: string;
		error?: string;
		required?: boolean;
		disabled?: boolean;
		placeholder?: string;
		allLabel?: string;
		ariaLabel?: string;
		onchange?: (value: string) => void;
	} = $props();
</script>

<RecordPicker
	bind:value
	{label}
	options={hostOptions(hosts, reason)}
	searchPlaceholder={t('hostPicker.search')}
	emptyLabel={hosts.length === 0 ? t('hostPicker.none') : undefined}
	countLabel={hostCount}
	{...rest}
/>

<script lang="ts">
	import type { ScheduleFailure } from '#lib/api/index.ts';
	import { formatLocalTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { Alert } from './ui/index.ts';

	let { failure, subject }: { failure: ScheduleFailure; subject: 'plan' | 'maintenance' } =
		$props();

	const reasonMessages: Record<ScheduleFailure['reason'], MessageKey> = {
		database_error: 'schedule.failure.reason.database_error',
		invalid_cron: 'schedule.failure.reason.invalid_cron',
		unknown_time_zone: 'schedule.failure.reason.unknown_time_zone',
		no_fire_time: 'schedule.failure.reason.no_fire_time'
	};
</script>

<Alert
	tone="danger"
	title={t(subject === 'plan' ? 'schedule.failure.titlePlan' : 'schedule.failure.titleMaintenance')}
>
	<p>{t(reasonMessages[failure.reason])}</p>
	<p>
		{t('schedule.failure.since', { time: formatLocalTime(failure.since) })}
	</p>
	{#if failure.retry_after}
		<p>
			{t('schedule.failure.retry', { time: formatLocalTime(failure.retry_after) })}
		</p>
	{:else}
		<p>{t('schedule.failure.edit')}</p>
	{/if}
</Alert>

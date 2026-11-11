<script lang="ts">
	import type { OnComplete, Queue, QueueChanges } from '#lib/api.js';
	import { ACTION_LABEL } from '#lib/countdown.svelte.js';
	import { queues } from '#lib/queues.svelte.js';

	const DAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
	const ACTIONS: OnComplete[] = ['none', 'sleep', 'hibernate', 'shutdown', 'exit'];

	let newName = $state('');

	$effect(() => {
		void queues.start();
	});

	function changesOf(q: Queue): QueueChanges {
		return { name: q.name, maxActive: q.maxActive, schedule: { ...q.schedule }, onComplete: q.onComplete };
	}

	function save(q: Queue, patch: Partial<QueueChanges>) {
		void queues.update(q.id, { ...changesOf(q), ...patch });
	}

	function toggleDay(q: Queue, day: number) {
		const days = q.schedule.days.includes(day)
			? q.schedule.days.filter((d) => d !== day)
			: [...q.schedule.days, day];
		save(q, { schedule: { ...q.schedule, days } });
	}

	async function add(e: SubmitEvent) {
		e.preventDefault();
		if (newName.trim() && (await queues.add(newName))) newName = '';
	}

	function counts(q: Queue): string {
		const parts = [
			q.active && `${q.active} downloading`,
			q.queued && `${q.queued} waiting`,
			q.paused && `${q.paused} paused`
		].filter(Boolean);
		const packages = `${q.packages} package${q.packages === 1 ? '' : 's'}`;
		return parts.length ? `${packages} · ${parts.join(' · ')}` : `${packages}, nothing to do`;
	}
</script>

<div class="page">
	<header>
		<h1>Queues</h1>
		{#if queues.error}<span class="error-text" role="alert">{queues.error}</span>{/if}
	</header>
	<p class="intro">
		Packages start in their queue's order. Each queue can run only at set times, and do something when it has
		finished. Move a package to another queue from its right-click menu on the Downloads page.
	</p>

	{#each queues.list as q (q.id)}
		<section class="card" aria-label={q.name}>
			<div class="head">
				{#if q.id === 1}
					<h2>{q.name}</h2>
				{:else}
					<input
						class="textbox name"
						aria-label="Queue name"
						value={q.name}
						onchange={(e) => save(q, { name: e.currentTarget.value })}
					/>
				{/if}
				<span class="tag" class:accent={q.open}>
					{q.open ? 'Running' : `Waits until ${q.schedule.start}`}
				</span>
				<span class="spacer"></span>
				<span class="counts">{counts(q)}</span>
			</div>

			<div class="grid">
				<label class="label" for="q{q.id}-active">Downloads at once</label>
				<div class="control">
					<input
						id="q{q.id}-active"
						class="textbox narrow"
						type="number"
						min="1"
						max="20"
						value={q.maxActive}
						onchange={(e) => save(q, { maxActive: Number(e.currentTarget.value) })}
					/>
					{#if q.id === 1}<small>Also in Settings.</small>{/if}
				</div>

				<span class="label">Schedule</span>
				<div class="control">
					<label class="check">
						<input
							type="checkbox"
							checked={q.schedule.enabled}
							onchange={(e) => save(q, { schedule: { ...q.schedule, enabled: e.currentTarget.checked } })}
						/>
						<span>Only run at set times</span>
					</label>
					{#if q.schedule.enabled}
						<div class="window">
							<div class="days" role="group" aria-label="Days">
								{#each DAYS as day, d (day)}
									<button
										type="button"
										class="day"
										class:on={q.schedule.days.includes(d)}
										aria-pressed={q.schedule.days.includes(d)}
										aria-label={day}
										title={day}
										onclick={() => toggleDay(q, d)}>{day[0]}</button
									>
								{/each}
							</div>
							<input
								class="textbox time"
								type="time"
								aria-label="Start"
								value={q.schedule.start}
								onchange={(e) => save(q, { schedule: { ...q.schedule, start: e.currentTarget.value } })}
							/>
							<span class="muted">to</span>
							<input
								class="textbox time"
								type="time"
								aria-label="Stop"
								value={q.schedule.stop}
								onchange={(e) => save(q, { schedule: { ...q.schedule, stop: e.currentTarget.value } })}
							/>
						</div>
						<small
							>Outside these times nothing starts, and downloads that are running stop and wait for the next
							window. A stop before the start means the next morning.</small
						>
					{/if}
				</div>

				<label class="label" for="q{q.id}-done">When finished</label>
				<div class="control">
					<select
						id="q{q.id}-done"
						class="select"
						value={q.onComplete}
						onchange={(e) => save(q, { onComplete: e.currentTarget.value as OnComplete })}
					>
						{#each ACTIONS as action (action)}
							<option value={action}>{ACTION_LABEL[action]}</option>
						{/each}
					</select>
					<small
						>Once all its downloads are done (paused ones count as not done). Counts down a minute first, with a
						Cancel button in the app and in a notification.</small
					>
				</div>
			</div>

			{#if q.id !== 1}
				<div class="foot">
					<button type="button" class="btn" onclick={() => queues.remove(q.id)}>Remove queue</button>
					<small>Its packages move to Main.</small>
				</div>
			{/if}
		</section>
	{/each}

	<form class="add" onsubmit={add}>
		<input class="textbox" placeholder="New queue name" aria-label="New queue name" bind:value={newName} />
		<button type="submit" class="btn accent" disabled={!newName.trim()}>Add queue</button>
	</form>
</div>

<style>
	.page {
		display: grid;
		gap: var(--space-4);
		padding: var(--page-pad);
		max-width: var(--content-max);
	}
	header {
		display: flex;
		align-items: baseline;
		gap: var(--space-5);
	}
	h1 {
		margin: 0;
		font: var(--type-title);
	}
	.intro {
		margin: 0;
		font: var(--type-body);
		color: var(--text-secondary);
	}
	.card {
		display: grid;
		gap: var(--space-4);
		padding: var(--space-4);
	}
	.head {
		display: flex;
		align-items: center;
		gap: var(--space-4);
	}
	h2 {
		margin: 0;
		font: var(--type-heading);
	}
	.name {
		width: 240px;
	}
	.counts {
		font: var(--type-small);
		color: var(--text-muted);
	}
	.spacer {
		flex: 1;
	}
	.grid {
		display: grid;
		grid-template-columns: 160px 1fr;
		align-items: start;
		gap: var(--space-4) var(--space-5);
	}
	.label {
		padding-top: var(--space-3);
		font: var(--type-label);
		color: var(--text);
	}
	.control {
		display: flex;
		flex-direction: column;
		gap: var(--space-3);
		min-width: 0;
	}
	.narrow {
		width: 90px;
	}
	.select {
		width: 240px;
	}
	small {
		font: var(--type-small);
		color: var(--text-muted);
	}
	.check {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		font: var(--type-body);
		padding-top: var(--space-3);
	}
	.window {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--space-3);
		font: var(--type-body);
	}
	.time {
		width: 120px;
	}
	.days {
		display: flex;
		gap: var(--space-1);
	}
	.day {
		width: var(--control-h-sm);
		height: var(--control-h-sm);
		padding: 0;
		font: var(--type-label);
		color: var(--text-secondary);
		background: transparent;
		border: 1px solid var(--border);
		border-radius: var(--radius-md);
		cursor: default;
		transition:
			background var(--motion-fast) var(--ease),
			border-color var(--motion-fast) var(--ease),
			color var(--motion-fast) var(--ease);
	}
	.day:hover {
		color: var(--text);
		background: var(--hover);
		border-color: var(--border-strong);
	}
	.day:active {
		background: var(--pressed);
	}
	.day.on {
		color: var(--on-accent);
		background: var(--accent-fill);
		border-color: transparent;
	}
	.day.on:hover {
		background: var(--accent-fill-hover);
	}
	.day.on:active {
		background: var(--accent-fill-pressed);
	}
	.foot {
		display: flex;
		align-items: center;
		gap: var(--space-4);
	}
	.add {
		display: flex;
		gap: var(--space-3);
	}
	.add .textbox {
		width: 240px;
	}
</style>

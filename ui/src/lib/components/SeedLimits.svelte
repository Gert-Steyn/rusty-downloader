<script lang="ts">
	// A torrent's own seeding limits. Mounted per download (keyed by its id),
	// so the drafts start from that torrent's saved values.
	import { untrack } from 'svelte';
	import type { Download } from '#lib/api.js';
	import { downloads } from '#lib/downloads.svelte.js';
	import { settings } from '#lib/settings.svelte.js';

	let { download }: { download: Download } = $props();

	const custom = $derived(download.torrent?.customSeed ?? false);
	// Drafts for the two fields; `null` is no limit.
	let ratio = $state<number | null>(untrack(() => download.torrent?.seedRatio ?? null));
	let minutes = $state<number | null>(untrack(() => download.torrent?.seedMinutes ?? null));

	/** What an unticked torrent seeds by, read from Settings. */
	const fromSettings = $derived.by(() => {
		const s = settings.value;
		if (!s) return 'Seeds by the limits in Settings';
		if (s.seedRatio == null && s.seedMinutes == null) return 'Seeds until stopped (from Settings)';
		const r = s.seedRatio != null ? `to ratio ${s.seedRatio.toFixed(1)}` : 'with no ratio limit';
		const m = s.seedMinutes != null ? `for ${s.seedMinutes} min` : 'no time limit';
		return `Seeds ${r}, ${m} (from Settings)`;
	});

	/** Ticking applies at once, with the Settings values as a start; Save then changes the values. */
	function toggle(on: boolean) {
		if (on) {
			ratio = settings.value?.seedRatio ?? null;
			minutes = settings.value?.seedMinutes ?? null;
			void downloads.setSeedLimits(download.id, true, ratio, minutes);
		} else {
			// Keep the stored values for when the torrent uses its own limits again.
			void downloads.setSeedLimits(
				download.id,
				false,
				download.torrent?.seedRatio ?? null,
				download.torrent?.seedMinutes ?? null
			);
		}
	}

	function save() {
		if (ratio != null && !(ratio > 0)) {
			downloads.error = 'The ratio must be more than 0. Leave it empty for no limit.';
			return;
		}
		if (minutes != null && minutes < 1) {
			downloads.error = 'The time must be at least 1 minute. Leave it empty for no limit.';
			return;
		}
		void downloads.setSeedLimits(download.id, true, ratio, minutes == null ? null : Math.round(minutes));
	}
</script>

<section class="limits" aria-label="Seeding limits">
	<h3>Seeding limits</h3>
	<label class="toggle">
		<input type="checkbox" checked={custom} onchange={(e) => toggle(e.currentTarget.checked)} />
		Use my own limits for this torrent
	</label>
	{#if custom}
		<div class="fields">
			<label class="field">
				<span>Ratio</span>
				<input class="textbox narrow" type="number" min="0.1" step="0.1" placeholder="No limit" bind:value={ratio} />
			</label>
			<label class="field">
				<span>Minutes</span>
				<input class="textbox narrow" type="number" min="1" step="1" placeholder="No limit" bind:value={minutes} />
			</label>
			<button type="button" class="btn small" onclick={save}>Save</button>
		</div>
	{:else}
		<p class="hint">{fromSettings}</p>
	{/if}
</section>

<style>
	.limits {
		display: grid;
		gap: var(--space-3);
		justify-items: start;
	}
	h3 {
		margin: var(--space-4) 0 0;
		font: var(--type-caption);
		color: var(--text-muted);
	}
	.toggle {
		display: inline-flex;
		align-items: center;
		gap: var(--space-3);
		font: var(--type-body);
		color: var(--text);
	}
	.fields {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-end;
		gap: var(--space-3);
	}
	.field {
		display: grid;
		gap: var(--space-2);
		margin: 0;
	}
	.field > span {
		font: var(--type-label);
		color: var(--text);
	}
	.narrow {
		width: 90px;
	}
	.hint {
		margin: 0;
	}
</style>

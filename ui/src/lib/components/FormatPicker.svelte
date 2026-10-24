<script lang="ts">
	// yt-dlp format choice: a preset, or a custom `-f` string typed inline.
	import type { FormatPreset, YtDlpOptions } from '#lib/api.js';
	import { presetsFor } from '#lib/media.js';

	let {
		options,
		heights = [],
		label,
		mixed = false,
		onchange
	}: {
		options: YtDlpOptions;
		heights?: number[];
		label: string;
		/** Several links with different choices: show a placeholder. */
		mixed?: boolean;
		onchange: (options: YtDlpOptions) => void;
	} = $props();

	const choices = $derived(presetsFor(heights, options.preset));
	let custom = $state('');

	$effect(() => {
		custom = options.customFormat ?? '';
	});

	function pick(preset: FormatPreset) {
		onchange({ preset, customFormat: preset === 'custom' ? custom || null : options.customFormat });
	}

	function commitCustom() {
		const value = custom.trim();
		if (value !== (options.customFormat ?? '')) onchange({ preset: 'custom', customFormat: value || null });
	}
</script>

<span class="picker">
	<select
		class="select"
		aria-label={label}
		value={mixed ? '' : options.preset}
		onchange={(e) => pick(e.currentTarget.value as FormatPreset)}
	>
		{#if mixed}<option value="" disabled>Mixed</option>{/if}
		{#each choices as choice (choice.value)}
			<option value={choice.value}>{choice.label}</option>
		{/each}
	</select>
	{#if options.preset === 'custom' && !mixed}
		<input
			class="textbox custom mono"
			aria-label="{label}: yt-dlp -f format"
			placeholder="bv*+ba/b"
			spellcheck="false"
			bind:value={custom}
			onblur={commitCustom}
			onkeydown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
		/>
	{/if}
</span>

<style>
	.picker {
		display: flex;
		gap: 4px;
		min-width: 0;
	}
	.select {
		min-width: 0;
		flex: 1;
	}
	.custom {
		min-height: 28px;
		width: 90px;
		padding: 3px 6px;
	}
	.custom:focus {
		padding-bottom: 2px;
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 12px;
	}
</style>

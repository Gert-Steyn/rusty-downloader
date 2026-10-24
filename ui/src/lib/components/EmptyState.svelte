<script lang="ts">
	import Icon, { type IconName } from './Icon.svelte';

	let {
		icon,
		title,
		body,
		phase
	}: { icon: IconName; title: string; body: string; phase?: string } = $props();
</script>

<div class="empty">
	<div class="glyph"><Icon name={icon} size={32} /></div>
	<h2>{title}</h2>
	<p>{body}</p>
	{#if phase}<span class="phase">{phase}</span>{/if}
</div>

<style>
	.empty {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 4px;
		height: 100%;
		padding: 32px;
		text-align: center;
		color: var(--text-secondary);
	}
	.glyph {
		margin-bottom: 8px;
		color: var(--accent-text-on-layer);
	}
	h2 {
		margin: 0;
		font: 600 20px/28px var(--font-display);
		color: var(--text-primary);
	}
	p {
		margin: 0;
		max-width: 420px;
	}
	.phase {
		margin-top: 12px;
		padding: 2px 8px;
		border-radius: 10px;
		font-size: 12px;
		background: var(--fill-subtle-hover);
	}
</style>

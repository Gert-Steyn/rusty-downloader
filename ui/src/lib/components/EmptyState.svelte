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
	<div class="glyph"><Icon name={icon} size={24} /></div>
	<h2>{title}</h2>
	<p>{body}</p>
	{#if phase}<span class="tag phase">{phase}</span>{/if}
</div>

<style>
	.empty {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: var(--space-2);
		/* Centred, with the text column capped at 320 (padding included). */
		max-width: 320px;
		height: 100%;
		margin: 0 auto;
		padding: var(--space-7) var(--space-5);
		text-align: center;
		color: var(--text-secondary);
	}
	.glyph {
		display: flex;
		margin-bottom: var(--space-3);
		color: var(--text-muted);
	}
	h2 {
		margin: 0;
		font: var(--type-heading);
		color: var(--text);
	}
	p {
		margin: 0;
		font: var(--type-body);
	}
	.phase {
		margin-top: var(--space-3);
	}
</style>

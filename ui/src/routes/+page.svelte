<script lang="ts">
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Icon from '#lib/components/Icon.svelte';
</script>

<div class="page">
	<header class="toolbar">
		<h1>Downloads</h1>
		<span class="spacer"></span>
		<button class="btn accent" disabled title="Arrives in Phase 1"><Icon name="plus" /> Add links</button>
		<button class="btn" disabled title="Arrives in Phase 1"><Icon name="play" /> Resume all</button>
		<button class="btn" disabled title="Arrives in Phase 1"><Icon name="pause" /> Pause all</button>
	</header>

	<EmptyState
		icon="downloads"
		title="No downloads yet"
		body="Paste links, copy them to the clipboard, or send them from your browser. They land in the LinkGrabber first so you can review them."
		phase="Download engine arrives in Phase 1"
	/>
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		height: 100%;
	}
	.toolbar {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 16px 20px 12px;
	}
	h1 {
		margin: 0;
		font: 600 20px/28px var(--font-display);
	}
	.spacer {
		flex: 1;
	}
</style>

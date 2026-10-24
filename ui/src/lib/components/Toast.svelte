<script lang="ts">
	// Bottom-right notice for things that happen in the background
	// (links from the clipboard, downloads queued). Click to act on it.
	let {
		message,
		action,
		onaction
	}: { message: { text: string; id: number } | null; action?: string; onaction?: () => void } =
		$props();

	let visible = $state(false);

	$effect(() => {
		if (!message) return;
		message.id; // re-run for every new message, even with the same text
		visible = true;
		const timer = setTimeout(() => (visible = false), 4000);
		return () => clearTimeout(timer);
	});
</script>

{#if visible && message}
	<div class="toast" role="status">
		<span>{message.text}</span>
		{#if action && onaction}
			<button
				class="btn"
				onclick={() => {
					visible = false;
					onaction();
				}}>{action}</button
			>
		{/if}
	</div>
{/if}

<style>
	.toast {
		position: fixed;
		right: 20px;
		bottom: 44px;
		z-index: 10;
		display: flex;
		align-items: center;
		gap: 12px;
		max-width: 420px;
		padding: 10px 10px 10px 16px;
		border: 1px solid var(--stroke-card);
		border-radius: var(--radius-overlay);
		background: var(--page-fallback);
		box-shadow: 0 8px 16px rgba(0, 0, 0, 0.14);
		animation: rise 160ms ease-out;
	}
	@keyframes rise {
		from {
			transform: translateY(8px);
			opacity: 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.toast {
			animation: none;
		}
	}
</style>

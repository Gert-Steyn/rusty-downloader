<script lang="ts">
	import type { Status } from '#lib/api.js';

	/** `value` is 0–100, or null for "unknown size" (indeterminate). */
	let { value, status }: { value: number | null; status: Status } = $props();
</script>

<div
	class="track"
	class:paused={status === 'paused' || status === 'queued'}
	class:failed={status === 'failed' || status === 'canceled'}
	class:done={status === 'completed'}
	role="progressbar"
	aria-valuemin="0"
	aria-valuemax="100"
	aria-valuenow={value == null ? undefined : Math.round(value)}
>
	{#if value == null}
		{#if status === 'active'}<div class="indeterminate"></div>{/if}
	{:else}
		<div class="fill" style:width="{value}%"></div>
	{/if}
</div>

<style>
	.track {
		position: relative;
		height: 4px;
		border-radius: 2px;
		overflow: hidden;
		background: var(--stroke-divider);
		--bar: var(--accent);
	}
	.paused {
		--bar: var(--caution);
	}
	.failed {
		--bar: var(--critical);
	}
	.done {
		--bar: var(--success);
	}
	.fill {
		height: 100%;
		border-radius: 2px;
		background: var(--bar);
		transition: width 150ms linear;
	}
	.indeterminate {
		position: absolute;
		inset: 0 auto 0 0;
		width: 30%;
		border-radius: 2px;
		background: var(--bar);
		animation: slide 1.2s ease-in-out infinite;
	}
	@keyframes slide {
		from {
			transform: translateX(-100%);
		}
		to {
			transform: translateX(340%);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.fill {
			transition: none;
		}
		.indeterminate {
			animation: none;
			width: 100%;
			opacity: 0.4;
		}
	}
</style>

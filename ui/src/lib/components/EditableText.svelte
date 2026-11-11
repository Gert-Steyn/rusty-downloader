<script lang="ts">
	// Text that turns into an input on double-click (or F2 when focused).
	// Enter or leaving the field saves, Esc cancels. `hint` goes in the tooltip.
	let {
		value,
		label,
		hint,
		onsave
	}: { value: string; label: string; hint?: string; onsave: (next: string) => void } = $props();

	let editing = $state(false);
	let draft = $state('');
	let input: HTMLInputElement | undefined = $state();

	function begin() {
		draft = value;
		editing = true;
		queueMicrotask(() => input?.select());
	}

	function commit() {
		if (!editing) return;
		editing = false;
		const next = draft.trim();
		if (next && next !== value) onsave(next);
	}
</script>

{#if editing}
	<input
		bind:this={input}
		bind:value={draft}
		class="textbox edit"
		aria-label={label}
		onblur={commit}
		onkeydown={(e) => {
			if (e.key === 'Enter') commit();
			else if (e.key === 'Escape') {
				editing = false;
				e.stopPropagation();
			}
		}}
	/>
{:else}
	<button
		type="button"
		class="text"
		title={hint ? `${hint}\nDouble-click to rename.` : 'Double-click to rename'}
		aria-label="{label}: {value}. Press F2 to rename."
		ondblclick={begin}
		onkeydown={(e) => e.key === 'F2' && begin()}
	>
		{value}
	</button>
{/if}

<style>
	.text {
		min-width: 0;
		max-width: 100%;
		/* Inner padding with a matching negative margin, so the text lines up with its container. */
		padding: 0 var(--space-2);
		margin: 0 calc(-1 * var(--space-2));
		border: 0;
		border-radius: var(--radius-md);
		background: transparent;
		color: inherit;
		text-align: left;
		font: inherit;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		transition: background var(--motion-fast) var(--ease);
	}
	.text:hover {
		background: var(--hover);
	}
	.text:active {
		background: var(--pressed);
	}
	.edit {
		min-height: var(--control-h-sm);
		padding: 0 var(--space-3);
	}
</style>

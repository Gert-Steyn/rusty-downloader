<script lang="ts">
	// Text that turns into an input on double-click (or F2 when focused).
	// Enter or leaving the field saves, Esc cancels.
	let {
		value,
		label,
		onsave
	}: { value: string; label: string; onsave: (next: string) => void } = $props();

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
		title="Double-click to rename"
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
		padding: 2px 4px;
		margin: -2px -4px;
		border: 0;
		border-radius: var(--radius-control);
		background: transparent;
		text-align: left;
		font: inherit;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.text:hover {
		background: var(--fill-subtle-hover);
	}
	.edit {
		min-height: 26px;
		padding: 2px 6px;
	}
</style>

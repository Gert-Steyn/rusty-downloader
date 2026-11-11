<script lang="ts" module>
	export type Selection = { kind: 'package' | 'download'; id: number } | null;
</script>

<script lang="ts">
	// Package rows with their downloads underneath, virtualised: only the rows
	// in (or near) the viewport are in the DOM, so thousands stay smooth.
	import { SvelteSet } from 'svelte/reactivity';
	import type { Download, Package } from '#lib/api.js';
	import type { Live } from '#lib/downloads.svelte.js';
	import {
		STATUS_LABELS,
		displayName,
		formatBytes,
		formatEta,
		formatSpeed,
		downloadPercent,
		iconFor,
		percent,
		progressText,
		summarize
	} from '#lib/format.js';
	import Icon from './Icon.svelte';
	import ProgressBar from './ProgressBar.svelte';

	let {
		packages,
		live,
		selection = $bindable(null),
		onactivate,
		onmenu,
		onmovepackage
	}: {
		packages: Package[];
		live: Record<number, Live>;
		selection?: Selection;
		onactivate?: (selection: NonNullable<Selection>) => void;
		/** Right click on a row (already selected), at client coordinates. */
		onmenu?: (selection: NonNullable<Selection>, x: number, y: number) => void;
		/** A package was dragged before another one, or to the end (`null`). */
		onmovepackage?: (id: number, before: number | null) => void;
	} = $props();

	/** Drag type for package rows; the layout's link-drop overlay ignores it. */
	const PACKAGE_DRAG = 'application/x-rdl-package';
	let dragging = $state<number | null>(null);
	/** Where the dragged package would land: before this package, or at the end. */
	let dropBefore = $state<number | 'end' | null>(null);

	function dragOver(e: DragEvent) {
		if (dragging == null) return;
		e.preventDefault();
		if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
		const row = rowAt(e);
		dropBefore = row ? row.pkg.id : 'end';
	}

	function drop(e: DragEvent) {
		if (dragging == null) return;
		e.preventDefault();
		const before = dropBefore === 'end' ? null : dropBefore;
		if (before !== dragging) onmovepackage?.(dragging, before);
		dragging = dropBefore = null;
	}

	const ROW = 32;
	const OVERSCAN = 8;

	type Row =
		| { key: string; kind: 'package'; pkg: Package }
		| { key: string; kind: 'download'; pkg: Package; dl: Download };

	const collapsed = new SvelteSet<number>();

	const rows = $derived.by(() => {
		const out: Row[] = [];
		for (const pkg of packages) {
			out.push({ key: `p${pkg.id}`, kind: 'package', pkg });
			if (collapsed.has(pkg.id)) continue;
			for (const dl of pkg.downloads) out.push({ key: `d${dl.id}`, kind: 'download', pkg, dl });
		}
		return out;
	});

	let body: HTMLDivElement;
	let scrollTop = $state(0);
	let viewport = $state(0);
	const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
	const last = $derived(Math.min(rows.length, Math.ceil((scrollTop + viewport) / ROW) + OVERSCAN));
	const visible = $derived(rows.slice(first, last));

	const selectedIndex = $derived(rows.findIndex((r) => isSelected(r)));

	function selectionOf(row: Row): NonNullable<Selection> {
		return row.kind === 'package'
			? { kind: 'package', id: row.pkg.id }
			: { kind: 'download', id: row.dl.id };
	}

	function isSelected(row: Row): boolean {
		if (!selection) return false;
		return row.kind === selection.kind && (row.kind === 'package' ? row.pkg.id : row.dl.id) === selection.id;
	}

	function toggle(pkgId: number) {
		if (collapsed.has(pkgId)) collapsed.delete(pkgId);
		else collapsed.add(pkgId);
	}

	function speedOf(pkg: Package): number {
		return pkg.downloads.reduce((sum, d) => sum + (live[d.id]?.speedBps ?? 0), 0);
	}

	function moveTo(index: number) {
		const row = rows[Math.max(0, Math.min(rows.length - 1, index))];
		if (!row) return;
		selection = selectionOf(row);
		const top = rows.indexOf(row) * ROW;
		if (top < body.scrollTop) body.scrollTop = top;
		else if (top + ROW > body.scrollTop + viewport) body.scrollTop = top + ROW - viewport;
	}

	/** The row under a click, found by delegation from the grid. */
	function rowAt(e: MouseEvent): Row | undefined {
		const el = (e.target as Element | null)?.closest<HTMLElement>('[data-index]');
		return el ? rows[Number(el.dataset.index)] : undefined;
	}

	function onkeydown(e: KeyboardEvent) {
		const row = rows[selectedIndex];
		switch (e.key) {
			case 'ArrowDown':
				moveTo(selectedIndex + 1);
				break;
			case 'ArrowUp':
				moveTo(selectedIndex < 0 ? 0 : selectedIndex - 1);
				break;
			case 'Home':
				moveTo(0);
				break;
			case 'End':
				moveTo(rows.length - 1);
				break;
			case 'ArrowRight':
				if (row?.kind === 'package' && collapsed.has(row.pkg.id)) toggle(row.pkg.id);
				break;
			case 'ArrowLeft':
				if (row?.kind === 'package' && !collapsed.has(row.pkg.id)) toggle(row.pkg.id);
				else if (row?.kind === 'download') moveTo(rows.findIndex((r) => r.kind === 'package' && r.pkg.id === row.pkg.id));
				break;
			case 'Enter':
				if (selection) onactivate?.(selection);
				break;
			default:
				return;
		}
		e.preventDefault();
	}
</script>

<div
	class="table"
	role="treegrid"
	aria-label="Downloads"
	aria-rowcount={rows.length}
	aria-activedescendant={selectedIndex >= 0 ? rows[selectedIndex].key : undefined}
	tabindex="0"
	{onkeydown}
	onclick={(e) => {
		const row = rowAt(e);
		if (row) selection = selectionOf(row);
	}}
	ondblclick={(e) => {
		const row = rowAt(e);
		if (row?.kind === 'package') toggle(row.pkg.id);
		else if (row) onactivate?.(selectionOf(row));
	}}
	oncontextmenu={(e) => {
		const row = rowAt(e);
		if (!row || !onmenu) return;
		e.preventDefault();
		selection = selectionOf(row);
		onmenu(selection, e.clientX, e.clientY);
	}}
	ondragover={dragOver}
	ondrop={drop}
	ondragleave={(e) => {
		if (!(e.currentTarget as Element).contains(e.relatedTarget as Node)) dropBefore = null;
	}}
>
	<div class="header row-grid" role="row">
		<span role="columnheader">Name</span>
		<span role="columnheader" class="num">Size</span>
		<span role="columnheader">Progress</span>
		<span role="columnheader" class="num">Speed</span>
		<span role="columnheader" class="num">ETA</span>
		<span role="columnheader">Status</span>
	</div>

	<div
		class="body"
		bind:this={body}
		bind:clientHeight={viewport}
		onscroll={() => (scrollTop = body.scrollTop)}
		role="rowgroup"
	>
		<div class="spacer" style:height="{rows.length * ROW}px">
			{#each visible as row, i (row.key)}
				{@const top = (first + i) * ROW}
				{#if row.kind === 'package'}
					{@const s = summarize(row.pkg)}
					{@const speed = speedOf(row.pkg)}
					<div
						id={row.key}
						class="row row-grid package"
						class:selected={isSelected(row)}
						class:drop-before={dropBefore === row.pkg.id && dragging !== row.pkg.id}
						class:dragging={dragging === row.pkg.id}
						style:top="{top}px"
						role="row"
						aria-level="1"
						aria-expanded={!collapsed.has(row.pkg.id)}
						aria-selected={isSelected(row)}
						tabindex="-1"
						data-index={first + i}
						draggable={onmovepackage ? 'true' : undefined}
						title={onmovepackage ? 'Drag to change the order packages start in' : undefined}
						ondragstart={(e) => {
							if (!e.dataTransfer) return;
							e.dataTransfer.setData(PACKAGE_DRAG, String(row.pkg.id));
							e.dataTransfer.effectAllowed = 'move';
							dragging = row.pkg.id;
						}}
						ondragend={() => (dragging = dropBefore = null)}
					>
						<span class="name" role="gridcell">
							<button
								class="chevron"
								tabindex="-1"
								aria-label={collapsed.has(row.pkg.id) ? 'Expand package' : 'Collapse package'}
								onclick={(e) => {
									e.stopPropagation();
									toggle(row.pkg.id);
								}}
							>
								<Icon name={collapsed.has(row.pkg.id) ? 'chevron-right' : 'chevron-down'} />
							</button>
							<Icon name="folder" />
							<span class="label">{row.pkg.name}</span>
							<span class="count">{row.pkg.downloads.length}</span>
						</span>
						<span class="num" role="gridcell">{formatBytes(s.size)}</span>
						<span class="progress" role="gridcell">
							<ProgressBar value={percent(s.downloaded, s.size)} status={s.status} />
						</span>
						<span class="num" role="gridcell">{formatSpeed(speed)}</span>
						<span class="num" role="gridcell">{formatEta(s.size == null ? null : s.size - s.downloaded, speed)}</span>
						<span class="status {s.status}" role="gridcell">
							{s.status === 'completed' ? STATUS_LABELS.completed : `${s.completed}/${row.pkg.downloads.length} done`}
						</span>
					</div>
				{:else}
					{@const d = row.dl}
					{@const pct = downloadPercent(d)}
					{@const speed = live[d.id]?.speedBps ?? 0}
					<div
						id={row.key}
						class="row row-grid download"
						class:selected={isSelected(row)}
						style:top="{top}px"
						role="row"
						aria-level="2"
						aria-selected={isSelected(row)}
						tabindex="-1"
						data-index={first + i}
					>
						<span class="name" role="gridcell">
							<Icon name={iconFor(d.kind)} />
							<span class="label" title={d.sourceUrl}>{displayName(d)}</span>
						</span>
						<span class="num" role="gridcell">{formatBytes(d.size ?? (d.files && d.downloaded ? d.downloaded : null))}</span>
						<span class="progress" role="gridcell">
							<ProgressBar value={pct} status={d.status} />
							<span class="pct" title={d.files ? 'Files done' : undefined}>{progressText(d)}</span>
						</span>
						<span class="num" role="gridcell">{d.status === 'active' ? formatSpeed(speed) : '—'}</span>
						<span class="num" role="gridcell">
							{d.status === 'active' ? formatEta(d.size == null ? null : d.size - d.downloaded, speed) : '—'}
						</span>
						<span class="status {d.status}" role="gridcell" title={d.error ?? undefined}>
							{STATUS_LABELS[d.status]}
						</span>
					</div>
				{/if}
			{/each}
			{#if dropBefore === 'end'}
				<div class="drop-end" style:top="{rows.length * ROW}px" aria-hidden="true"></div>
			{/if}
		</div>
	</div>
</div>


<style>
	.table {
		display: flex;
		flex-direction: column;
		min-height: 0;
		flex: 1;
	}
	.row-grid {
		display: grid;
		grid-template-columns: minmax(220px, 1fr) 88px minmax(120px, 180px) 96px 72px 112px;
		align-items: center;
		gap: var(--space-4);
		padding: 0 var(--space-3);
	}
	/* Inset like the rows (8) plus the same cell padding (8), so columns line up. */
	.header {
		height: var(--control-h);
		margin: 0 var(--space-3);
		font: var(--type-caption);
		color: var(--text-muted);
		border-bottom: 1px solid var(--border-subtle);
	}
	.num {
		text-align: right;
	}
	.table:focus-visible {
		outline: none;
	}
	.body {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
	}
	.table:focus-visible .selected {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
	}
	.spacer {
		position: relative;
	}
	.row.dragging {
		background: var(--pressed);
	}
	.row.drop-before::before {
		content: '';
		position: absolute;
		left: 0;
		right: 0;
		top: 0;
		height: 2px;
		background: var(--accent);
	}
	.drop-end {
		position: absolute;
		left: 0;
		right: 0;
		height: 2px;
		background: var(--accent);
	}
	.row {
		position: absolute;
		left: var(--space-3);
		right: var(--space-3);
		height: var(--row-h);
		border-bottom: 1px solid var(--border-subtle);
		border-radius: var(--radius-md);
		font: var(--type-body);
		transition: background var(--motion-fast) var(--ease);
	}
	.row:hover {
		background: var(--hover);
	}
	.row.selected {
		background: var(--accent-subtle);
	}
	.name {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		min-width: 0;
	}
	/* Indent under the package: inset 8, chevron 22 and gap 8, so the icon lines up with the folder icon. */
	.download .name {
		padding-left: calc(var(--space-3) + 22px + var(--space-3));
		color: var(--text);
	}
	.package .name {
		font: var(--type-label);
	}
	.label {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.count {
		flex-shrink: 0;
		padding: 0 var(--space-2);
		border-radius: var(--radius-sm);
		font: var(--type-caption);
		font-variant-numeric: tabular-nums;
		color: var(--text-secondary);
		background: var(--hover);
	}
	.chevron {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 22px;
		height: 22px;
		padding: 0;
		border: 0;
		border-radius: var(--radius-md);
		background: transparent;
		color: var(--text-muted);
		transition:
			background var(--motion-fast) var(--ease),
			color var(--motion-fast) var(--ease);
	}
	.chevron:hover {
		background: var(--hover);
		color: var(--text);
	}
	.chevron:active {
		background: var(--pressed);
	}
	.progress {
		display: flex;
		align-items: center;
		gap: var(--space-3);
	}
	.progress :global(.track) {
		flex: 1;
	}
	.pct {
		width: 36px;
		font: var(--type-small);
		text-align: right;
		color: var(--text-secondary);
		font-variant-numeric: tabular-nums;
	}
	.status {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--text-secondary);
	}
	.status.active {
		color: var(--accent);
	}
	.status.completed {
		color: var(--success);
	}
	.status.failed {
		color: var(--danger);
	}
</style>

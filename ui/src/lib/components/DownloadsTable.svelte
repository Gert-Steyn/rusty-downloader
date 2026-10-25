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
		onactivate
	}: {
		packages: Package[];
		live: Record<number, Live>;
		selection?: Selection;
		onactivate?: (selection: NonNullable<Selection>) => void;
	} = $props();

	const ROW = 34;
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
						style:top="{top}px"
						role="row"
						aria-level="1"
						aria-expanded={!collapsed.has(row.pkg.id)}
						aria-selected={isSelected(row)}
						tabindex="-1"
						data-index={first + i}
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
								<Icon name={collapsed.has(row.pkg.id) ? 'chevron-right' : 'chevron-down'} size={14} />
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
		gap: 12px;
		padding: 0 16px;
	}
	.header {
		height: 32px;
		font-size: 12px;
		color: var(--text-secondary);
		border-bottom: 1px solid var(--stroke-divider);
	}
	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
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
		outline: 2px solid var(--text-primary);
		outline-offset: -2px;
	}
	.spacer {
		position: relative;
	}
	.row {
		position: absolute;
		left: 4px;
		right: 4px;
		height: 32px;
		margin-top: 2px;
		border-radius: var(--radius-control);
		font-size: 13px;
	}
	.row:hover {
		background: var(--fill-subtle-hover);
	}
	.row.selected {
		background: var(--fill-subtle-hover);
	}
	.row.selected::before {
		content: '';
		position: absolute;
		left: 0;
		top: 8px;
		bottom: 8px;
		width: 3px;
		border-radius: 2px;
		background: var(--accent);
	}
	.name {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
	}
	.download .name {
		padding-left: 46px;
		color: var(--text-primary);
	}
	.package .name {
		font-weight: 600;
	}
	.label {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.count {
		flex-shrink: 0;
		padding: 0 6px;
		border-radius: 8px;
		font-size: 11px;
		font-weight: 400;
		line-height: 16px;
		color: var(--text-secondary);
		background: var(--fill-subtle-hover);
	}
	.chevron {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 22px;
		height: 22px;
		padding: 0;
		border: 0;
		border-radius: var(--radius-control);
		background: transparent;
		color: var(--text-secondary);
	}
	.chevron:hover {
		background: var(--fill-subtle-hover);
	}
	.progress {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.progress :global(.track) {
		flex: 1;
	}
	.pct {
		width: 36px;
		font-size: 12px;
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
		color: var(--accent-text-on-layer);
	}
	.status.completed {
		color: var(--success);
	}
	.status.failed {
		color: var(--critical);
	}
</style>

<script lang="ts">
	// Settings → Network: the adapter every connection goes through (and the
	// kill-switch), proxy profiles, the default, and per-site rules.
	import type { ProxySettings } from '#lib/api.js';
	import { KIND_LABELS, network } from '#lib/network.svelte.js';
	import { ArrowDown, ArrowUp } from '@lucide/svelte';
	import { settings } from '#lib/settings.svelte.js';
	import ProxyDialog from './ProxyDialog.svelte';

	const bound = $derived(settings.value?.bindAdapter ?? null);
	/** The chosen adapter even when it's gone from the list (unplugged, VPN removed). */
	const adapterChoices = $derived(
		bound && !network.adapters.some((a) => a.id === bound.id)
			? [...network.adapters, { id: bound.id, name: bound.name, description: '', address: null, missing: true }]
			: network.adapters
	);

	function bindTo(id: string) {
		const adapter = adapterChoices.find((a) => a.id === id);
		void settings.update({ bindAdapter: adapter ? { id: adapter.id, name: adapter.name } : null });
	}

	let editing = $state<number | null>(null);
	let dialogOpen = $state(false);
	/** Test result per profile id: "Works · 85 ms" or the reason it didn't. */
	let tests = $state<Record<number, { ok: boolean; text: string } | 'running'>>({});

	$effect(() => {
		void network.load();
		void network.loadAdapters();
	});

	function edit(id: number | null) {
		editing = id;
		dialogOpen = true;
	}

	async function test(id: number) {
		tests[id] = 'running';
		try {
			const ms = await network.test({ mode: 'profile', id });
			tests[id] = { ok: true, text: `Works · ${ms} ms` };
		} catch (e) {
			tests[id] = { ok: false, text: e instanceof Error ? e.message : String(e) };
		}
	}

	/** `""` in a select is "Direct". */
	const toId = (value: string) => (value === '' ? null : Number(value));

	function setDefault(value: string) {
		void network.saveSettings({ ...network.settings, default: toId(value) });
	}

	/** Save the rules with `rule` at `i` changed (or removed, or added at the end). */
	function saveRule(i: number, rule: ProxySettings['rules'][number] | null) {
		const rules = [...network.settings.rules];
		if (rule == null) rules.splice(i, 1);
		else rules[i] = rule;
		void network.saveSettings({ ...network.settings, rules });
	}

	function move(i: number, by: number) {
		const rules = [...network.settings.rules];
		const [rule] = rules.splice(i, 1);
		rules.splice(i + by, 0, rule);
		void network.saveSettings({ ...network.settings, rules });
	}
</script>

<section aria-labelledby="set-network">
	<h2 id="set-network">Network</h2>
	{#if network.error}<p class="error-text">{network.error}</p>{/if}

	<h3>Adapter</h3>
	<div class="rule">
		<select
			class="select adapter"
			aria-label="Send everything through"
			value={bound?.id ?? ''}
			onchange={(e) => bindTo(e.currentTarget.value)}
		>
			<option value="">Any connection (Windows decides)</option>
			{#each adapterChoices as a (a.id)}
				<option value={a.id}>
					{a.name} · {'missing' in a ? 'not found' : (a.address ?? 'not connected')}
				</option>
			{/each}
		</select>
		<button type="button" class="btn" onclick={() => network.loadAdapters()}>Refresh</button>
	</div>
	{#if bound}
		{@const s = network.status}
		<p class="state" class:ok={s.state === 'up'} class:bad={s.state === 'down'}>
			{#if s.state === 'up'}
				Connected: everything goes out from {s.address}.
			{:else if s.state === 'down'}
				{s.adapter} is down: nothing connects{s.held ? ', and downloads wait for it' : ''}.
			{:else}
				Checking…
			{/if}
		</p>
		<label class="check">
			<input
				type="checkbox"
				checked={settings.value?.killSwitch ?? true}
				onchange={(e) => settings.update({ killSwitch: e.currentTarget.checked })}
			/>
			<span>
				Pause downloads while it's down
				<small>They stop with their progress kept and carry on by themselves when it's back. Off, they fail instead.</small>
			</span>
		</label>
	{/if}
	<small>
		With an adapter chosen, every connection (downloads, link checks, yt-dlp, gallery-dl, tool updates) goes out
		through it, also to a proxy; while it's down, none go out at all.
	</small>

	<h3>Proxies</h3>
	{#if network.profiles.length === 0}
		<small>No proxies yet: every download connects directly.</small>
	{/if}
	{#each network.profiles as p (p.id)}
		{@const result = tests[p.id]}
		<div class="profile">
			<div class="what">
				<span class="name">{p.name}</span>
				<span class="muted mono">{KIND_LABELS[p.kind]} {p.host}:{p.port}</span>
				{#if p.username}<span class="muted">· {p.username}{p.hasPassword ? ' (password saved)' : ''}</span>{/if}
			</div>
			<div class="actions">
				<button type="button" class="btn" disabled={result === 'running'} onclick={() => test(p.id)}>
					{result === 'running' ? 'Testing…' : 'Test'}
				</button>
				<button type="button" class="btn" onclick={() => edit(p.id)}>Edit</button>
				<button type="button" class="btn" onclick={() => network.remove(p.id)}>Remove</button>
			</div>
			{#if result && result !== 'running'}
				<span class="result" class:ok={result.ok} class:bad={!result.ok}>{result.text}</span>
			{/if}
		</div>
	{/each}
	<div class="row">
		<button type="button" class="btn" onclick={() => edit(null)}>Add a proxy</button>
	</div>
	{#if network.profiles.length > 0}
		<small>Removing a proxy removes its rules and saved password; what used it connects directly.</small>
	{/if}

	{#if network.profiles.length > 0}
		<h3>Which connection</h3>
		<small>
			The first rule that matches a site decides; other sites use the connection below. <span class="mono">example.com</span> also
			covers its subdomains; <span class="mono">*</span> matches anything. Downloads already running keep their connection.
		</small>
		{#each network.settings.rules as rule, i (i)}
			<div class="rule">
				<input
					class="textbox site mono"
					aria-label="Site"
					spellcheck="false"
					value={rule.pattern}
					onchange={(e) => saveRule(i, { ...rule, pattern: e.currentTarget.value })}
				/>
				<span class="muted">goes</span>
				<select
					class="select"
					aria-label="Connection"
					value={rule.profile == null ? '' : String(rule.profile)}
					onchange={(e) => saveRule(i, { ...rule, profile: toId(e.currentTarget.value) })}
				>
					<option value="">directly</option>
					{#each network.profiles as p (p.id)}
						<option value={String(p.id)}>through {p.name}</option>
					{/each}
				</select>
				<button
					type="button"
					class="btn ghost icon-only"
					title="Move up"
					aria-label="Move up"
					disabled={i === 0}
					onclick={() => move(i, -1)}><ArrowUp size={16} strokeWidth={1.5} aria-hidden="true" /></button
				>
				<button
					type="button"
					class="btn ghost icon-only"
					title="Move down"
					aria-label="Move down"
					disabled={i === network.settings.rules.length - 1}
					onclick={() => move(i, 1)}><ArrowDown size={16} strokeWidth={1.5} aria-hidden="true" /></button
				>
				<button type="button" class="btn" onclick={() => saveRule(i, null)}>Remove</button>
			</div>
		{/each}
		<div class="rule">
			<span class="everything">Everything else</span>
			<span class="muted">goes</span>
			<select
				class="select"
				aria-label="Default connection"
				value={network.settings.default == null ? '' : String(network.settings.default)}
				onchange={(e) => setDefault(e.currentTarget.value)}
			>
				<option value="">directly</option>
				{#each network.profiles as p (p.id)}
					<option value={String(p.id)}>through {p.name}</option>
				{/each}
			</select>
		</div>
		<div class="row">
			<button
				type="button"
				class="btn"
				onclick={() => saveRule(network.settings.rules.length, { pattern: 'example.com', profile: null })}
				>Add a rule</button
			>
		</div>
		<small>yt-dlp, gallery-dl and tool updates follow the same rules. A gallery's own proxy option still wins.</small>
	{/if}
</section>

<ProxyDialog bind:open={dialogOpen} id={editing} />

<style>
	section {
		display: grid;
	}
	h2 {
		margin: 0 0 var(--space-3);
		font: var(--type-heading);
		color: var(--text);
	}
	h3 {
		margin: var(--space-5) 0 0;
		font: var(--type-label);
		color: var(--text);
	}
	small {
		font: var(--type-small);
		color: var(--text-muted);
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: var(--space-3);
		padding-top: var(--space-4);
	}
	.profile {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto;
		align-items: center;
		gap: var(--space-2) var(--space-4);
		padding: var(--space-4) 0;
		border-top: 1px solid var(--border-subtle);
		font: var(--type-body);
	}
	.what {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: var(--space-2) var(--space-3);
		min-width: 0;
	}
	.name {
		font: var(--type-label);
		color: var(--text);
	}
	.actions {
		display: flex;
		gap: var(--space-3);
	}
	.result {
		grid-column: 1 / -1;
		font: var(--type-small);
		color: var(--text-muted);
		overflow-wrap: anywhere;
	}
	.result.ok {
		color: var(--success);
	}
	.result.bad {
		color: var(--danger);
	}
	.rule {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--space-3);
		padding: var(--space-4) 0;
		border-top: 1px solid var(--border-subtle);
		font: var(--type-body);
	}
	.site {
		width: 220px;
	}
	.everything {
		width: 220px;
		font: var(--type-label);
		color: var(--text);
	}
	.error-text {
		margin: 0;
	}
	.adapter {
		min-width: 320px;
	}
	.state {
		margin: 0;
		font: var(--type-body);
		color: var(--text-secondary);
	}
	.state.ok {
		color: var(--success);
	}
	.state.bad {
		color: var(--danger);
	}
	.check {
		display: flex;
		align-items: flex-start;
		gap: var(--space-3);
		padding: var(--space-4) 0;
		border-top: 1px solid var(--border-subtle);
		font: var(--type-body);
	}
	.check input {
		margin-top: var(--space-1);
	}
	.check span {
		display: flex;
		flex-direction: column;
		gap: var(--space-1);
	}
</style>

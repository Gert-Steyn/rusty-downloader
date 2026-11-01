<script lang="ts">
	// Settings → Network: proxy profiles, the default, and per-site rules.
	import type { ProxySettings } from '#lib/api.js';
	import { KIND_LABELS, network } from '#lib/network.svelte.js';
	import ProxyDialog from './ProxyDialog.svelte';

	let editing = $state<number | null>(null);
	let dialogOpen = $state(false);
	/** Test result per profile id: "Works · 85 ms" or the reason it didn't. */
	let tests = $state<Record<number, { ok: boolean; text: string } | 'running'>>({});

	$effect(() => {
		void network.load();
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

<section class="card" aria-labelledby="set-network">
	<h2 id="set-network">Network</h2>
	{#if network.error}<p class="error-text">{network.error}</p>{/if}

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
				<button type="button" class="btn icon" aria-label="Move up" disabled={i === 0} onclick={() => move(i, -1)}>↑</button>
				<button
					type="button"
					class="btn icon"
					aria-label="Move down"
					disabled={i === network.settings.rules.length - 1}
					onclick={() => move(i, 1)}>↓</button
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
		gap: 10px;
		padding: 16px;
	}
	h2 {
		margin: 0 0 2px;
		font: 600 14px/20px var(--font-ui);
	}
	h3 {
		margin: 6px 0 0;
		font: 600 13px/18px var(--font-ui);
	}
	small {
		font-size: 12px;
		color: var(--text-secondary);
	}
	.muted {
		color: var(--text-secondary);
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 12px;
	}
	.row {
		display: flex;
		gap: 8px;
	}
	.profile {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto;
		align-items: center;
		gap: 4px 12px;
		font-size: 13px;
	}
	.what {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 4px 8px;
		min-width: 0;
	}
	.name {
		font-weight: 600;
	}
	.actions {
		display: flex;
		gap: 6px;
	}
	.result {
		grid-column: 1 / -1;
		font-size: 12px;
		overflow-wrap: anywhere;
	}
	.result.ok {
		color: var(--success);
	}
	.result.bad {
		color: var(--critical);
	}
	.rule {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
		font-size: 13px;
	}
	.site {
		width: 220px;
	}
	.icon {
		min-width: 32px;
		padding-inline: 6px;
	}
	.everything {
		width: 220px;
	}
	.error-text {
		margin: 0;
	}
</style>

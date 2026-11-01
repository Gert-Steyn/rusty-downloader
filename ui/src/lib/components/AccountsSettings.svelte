<script lang="ts">
	// Settings → Accounts: site logins and their cookies.
	import { open as pickFile } from '@tauri-apps/plugin-dialog';
	import { isTauri } from '#lib/api.js';
	import { network } from '#lib/network.svelte.js';
	import AccountDialog from './AccountDialog.svelte';

	let editing = $state<number | null>(null);
	let dialogOpen = $state(false);
	/** The last import or error per account. */
	let notes = $state<Record<number, { ok: boolean; text: string }>>({});

	$effect(() => {
		void network.loadAccounts();
	});

	function edit(id: number | null) {
		editing = id;
		dialogOpen = true;
	}

	async function importCookies(id: number) {
		const path = await pickFile({
			title: 'Cookies for this account (Netscape cookies.txt)',
			filters: [{ name: 'cookies.txt', extensions: ['txt'] }]
		});
		if (typeof path !== 'string') return;
		try {
			const r = await network.importCookies(id, path);
			const others = r.otherSites ? `; ${r.otherSites} for other sites left out` : '';
			notes[id] = { ok: true, text: `${r.imported} cookie${r.imported === 1 ? '' : 's'} added${others}.` };
		} catch (e) {
			notes[id] = { ok: false, text: e instanceof Error ? e.message : String(e) };
		}
	}
</script>

<section class="card" aria-labelledby="set-accounts">
	<h2 id="set-accounts">Accounts</h2>
	{#if network.accounts.length === 0}
		<small>No accounts yet. Add one for a site you're signed in to, then import its cookies.</small>
	{/if}
	{#each network.accounts as a (a.id)}
		{@const note = notes[a.id]}
		<div class="account" class:off={!a.enabled}>
			<div class="what">
				<span class="name mono">{a.site}</span>
				{#if a.username}<span class="muted">{a.username}{a.hasPassword ? ' (password saved)' : ''}</span>{/if}
				<span class="muted">· {a.cookies} cookie{a.cookies === 1 ? '' : 's'}</span>
				{#if !a.enabled}<span class="muted">· not used</span>{/if}
			</div>
			<div class="actions">
				<button type="button" class="btn" disabled={!isTauri()} onclick={() => importCookies(a.id)}>Import cookies.txt…</button>
				{#if a.cookies > 0}
					<button type="button" class="btn" onclick={() => network.clearCookies(a.id)}>Clear cookies</button>
				{/if}
				<button type="button" class="btn" onclick={() => edit(a.id)}>Edit</button>
				<button type="button" class="btn" onclick={() => network.removeAccount(a.id)}>Remove</button>
			</div>
			{#if note}<span class="note" class:ok={note.ok} class:bad={!note.ok}>{note.text}</span>{/if}
		</div>
	{/each}
	<div class="row">
		<button type="button" class="btn" onclick={() => edit(null)}>Add an account</button>
	</div>
	<small>
		Cookies from the browser extension win over stored ones for the same cookie. Stored cookies are encrypted for your
		Windows user; passwords are kept in Windows Credential Manager.
	</small>
</section>

<AccountDialog bind:open={dialogOpen} id={editing} />

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
	.account {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto;
		align-items: center;
		gap: 4px 12px;
		font-size: 13px;
	}
	.account.off .name {
		color: var(--text-secondary);
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
	.note {
		grid-column: 1 / -1;
		font-size: 12px;
		overflow-wrap: anywhere;
	}
	.note.ok {
		color: var(--success);
	}
	.note.bad {
		color: var(--critical);
	}
</style>

<script lang="ts">
	// Add or edit a site account. The password goes to Windows Credential
	// Manager; an existing one is never shown, only kept, replaced or removed.
	import { network } from '#lib/network.svelte.js';
	import Dialog from './Dialog.svelte';

	let { open = $bindable(false), id }: { open?: boolean; id: number | null } = $props();

	const existing = $derived(network.accounts.find((a) => a.id === id));
	let site = $state('');
	let username = $state('');
	let password = $state('');
	let removePassword = $state(false);
	let enabled = $state(true);
	let error = $state<string | null>(null);
	let saving = $state(false);

	$effect(() => {
		if (!open) return;
		const a = existing;
		site = a?.site ?? '';
		username = a?.username ?? '';
		password = '';
		removePassword = false;
		enabled = a?.enabled ?? true;
		error = null;
	});

	async function save() {
		saving = true;
		try {
			const user = username.trim();
			await network.saveAccount(id, {
				site,
				username: user || null,
				// null keeps the stored password, "" removes it.
				password: password !== '' ? password : removePassword || !user ? '' : null,
				enabled
			});
			open = false;
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			saving = false;
		}
	}
</script>

<Dialog bind:open title={id == null ? 'Add an account' : 'Edit account'} width={460} onsubmit={save}>
	<div class="form">
		<label for="account-site">Site</label>
		<input id="account-site" class="textbox mono" placeholder="example.com" spellcheck="false" bind:value={site} />

		<label for="account-user">User name</label>
		<input id="account-user" class="textbox" placeholder="Optional" autocomplete="off" spellcheck="false" bind:value={username} />

		<label for="account-pass">Password</label>
		<div class="pass">
			<input
				id="account-pass"
				class="textbox"
				type="password"
				autocomplete="new-password"
				placeholder={existing?.hasPassword ? 'Saved; type to replace it' : 'Optional'}
				disabled={!username.trim()}
				bind:value={password}
			/>
			{#if existing?.hasPassword && username.trim()}
				<label class="check"><input type="checkbox" bind:checked={removePassword} /> Remove the saved password</label>
			{/if}
		</div>

		<span></span>
		<label class="check"><input type="checkbox" bind:checked={enabled} /> Use this account</label>
	</div>
	<small>
		<span class="mono">example.com</span> also covers its subdomains. The login goes to yt-dlp and gallery-dl for links to
		the site, and to the site itself when it asks for one (HTTP Basic). Cookies are added from a cookies.txt afterwards.
	</small>
	{#if error}<p class="error-text">{error}</p>{/if}

	{#snippet footer()}
		<button type="button" class="btn ghost" onclick={() => (open = false)}>Cancel</button>
		<button type="submit" class="btn accent" disabled={saving}>Save</button>
	{/snippet}
</Dialog>

<style>
	.form {
		display: grid;
		grid-template-columns: 90px 1fr;
		align-items: center;
		gap: var(--space-3) var(--space-4);
		margin-bottom: var(--space-4);
		font: var(--type-body);
	}
	.form > label:not(.check) {
		font: var(--type-label);
		color: var(--text);
	}
	.pass {
		display: grid;
		gap: var(--space-2);
	}
	.check {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		font: var(--type-body);
		color: var(--text);
	}
	small {
		display: block;
		font: var(--type-small);
		color: var(--text-muted);
	}
	.error-text {
		margin: var(--space-3) 0 0;
		overflow-wrap: anywhere;
	}
</style>

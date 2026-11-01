<script lang="ts">
	// Add or edit a proxy profile. The password goes to Windows Credential
	// Manager; an existing one is never shown, only kept, replaced or removed.
	import type { ProxyKind } from '#lib/api.js';
	import { KIND_LABELS, network } from '#lib/network.svelte.js';
	import Dialog from './Dialog.svelte';

	let { open = $bindable(false), id }: { open?: boolean; id: number | null } = $props();

	const existing = $derived(network.profile(id));
	let name = $state('');
	let kind = $state<ProxyKind>('socks5');
	let host = $state('');
	let port = $state('1080');
	let username = $state('');
	let password = $state('');
	let removePassword = $state(false);
	let error = $state<string | null>(null);
	let saving = $state(false);

	const DEFAULT_PORTS: Record<ProxyKind, string> = { http: '8080', https: '443', socks5: '1080' };

	$effect(() => {
		if (!open) return;
		const p = existing;
		name = p?.name ?? '';
		kind = p?.kind ?? 'socks5';
		host = p?.host ?? '';
		port = String(p?.port ?? 1080);
		username = p?.username ?? '';
		password = '';
		removePassword = false;
		error = null;
	});

	function pickKind(next: ProxyKind) {
		// Follow the type's usual port until one is typed in.
		if (port === DEFAULT_PORTS[kind]) port = DEFAULT_PORTS[next];
		kind = next;
	}

	async function save() {
		const portNumber = Number(port);
		if (!Number.isInteger(portNumber) || portNumber < 1 || portNumber > 65535) {
			error = 'Enter a port from 1 to 65535.';
			return;
		}
		saving = true;
		try {
			await network.save(id, {
				name,
				kind,
				host,
				port: portNumber,
				username: username.trim() || null,
				// null keeps the stored password, "" removes it.
				password: password !== '' ? password : removePassword || !username.trim() ? '' : null
			});
			open = false;
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			saving = false;
		}
	}
</script>

<Dialog bind:open title={id == null ? 'Add a proxy' : 'Edit proxy'} width={480} onsubmit={save}>
	<div class="form">
		<label for="proxy-name">Name</label>
		<input id="proxy-name" class="textbox" placeholder="Home VPN" bind:value={name} />

		<label for="proxy-kind">Type</label>
		<select
			id="proxy-kind"
			class="select"
			value={kind}
			onchange={(e) => pickKind(e.currentTarget.value as ProxyKind)}
		>
			{#each Object.entries(KIND_LABELS) as [value, label] (value)}
				<option {value}>{label}</option>
			{/each}
		</select>

		<label for="proxy-host">Server</label>
		<div class="server">
			<input id="proxy-host" class="textbox" placeholder="proxy.example.com or 10.0.0.2" spellcheck="false" bind:value={host} />
			<span class="muted">:</span>
			<input class="textbox port" aria-label="Port" inputmode="numeric" bind:value={port} />
		</div>

		<label for="proxy-user">User name</label>
		<input id="proxy-user" class="textbox" placeholder="Optional" autocomplete="off" spellcheck="false" bind:value={username} />

		<label for="proxy-pass">Password</label>
		<div class="pass">
			<input
				id="proxy-pass"
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
	</div>
	<small>
		The password is kept in Windows Credential Manager, not in the app's files. SOCKS5 looks names up through the proxy too.
	</small>
	{#if error}<p class="error-text">{error}</p>{/if}

	{#snippet footer()}
		<button type="submit" class="btn accent" disabled={saving}>Save</button>
		<button type="button" class="btn" onclick={() => (open = false)}>Cancel</button>
	{/snippet}
</Dialog>

<style>
	.form {
		display: grid;
		grid-template-columns: 90px 1fr;
		align-items: center;
		gap: 10px 12px;
		margin-bottom: 12px;
		font-size: 13px;
	}
	.server {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.server input:first-child {
		flex: 1;
		min-width: 0;
	}
	.port {
		width: 72px;
	}
	.pass {
		display: grid;
		gap: 6px;
	}
	.check {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: 12px;
	}
	.muted {
		color: var(--text-secondary);
	}
	small {
		display: block;
		font-size: 12px;
		color: var(--text-secondary);
	}
	.error-text {
		margin: 8px 0 0;
		overflow-wrap: anywhere;
	}
</style>

<script lang="ts">
	// Settings → Known SFTP hosts: the host key each server showed first.
	import { network } from '#lib/network.svelte.js';

	$effect(() => {
		void network.loadKnownHosts();
	});
</script>

<section class="card" aria-labelledby="set-hosts">
	<h2 id="set-hosts">Known SFTP hosts</h2>
	{#if network.knownHosts.length === 0}
		<small>None yet. The first time a download connects to an SFTP server, its host key is remembered here.</small>
	{/if}
	{#each network.knownHosts as h (`${h.host}:${h.port}`)}
		<div class="host">
			<div class="what">
				<span class="name mono">{h.port === 22 ? h.host : `${h.host}:${h.port}`}</span>
				<span class="muted mono" title={h.fingerprint}>{h.algorithm} {h.fingerprint}</span>
			</div>
			<button type="button" class="btn" onclick={() => network.forgetHost(h.host, h.port)}>Forget</button>
		</div>
	{/each}
	<small>
		A server that later shows a different key is refused, since someone may be in between. If the server's key really
		changed, forget it here and the next connection trusts the new one.
	</small>
</section>

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
	.host {
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
		overflow-wrap: anywhere;
	}
	.name {
		font-weight: 600;
	}
</style>

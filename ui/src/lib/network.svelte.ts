// Proxy profiles and the rules that pick them (Settings → Network, and the
// download menu's "Connection…"), read on demand; and the adapter all
// connections are bound to, whose state the backend reports as it changes.
import {
	commands,
	events,
	isTauri,
	unwrap,
	type Account,
	type AccountInput,
	type AdapterInfo,
	type CookieImport,
	type KnownHost,
	type NetworkStatus,
	type ProxyChoice,
	type ProxyInput,
	type ProxyProfile,
	type ProxySettings
} from './api.js';

export const KIND_LABELS = { http: 'HTTP', https: 'HTTPS', socks5: 'SOCKS5' } as const;

class NetworkStore {
	profiles = $state<ProxyProfile[]>([]);
	settings = $state<ProxySettings>({ default: null, rules: [] });
	error = $state<string | null>(null);
	loaded = $state(false);
	/** The bound adapter's state, for the status bar and Settings. */
	status = $state<NetworkStatus>({ state: 'any' });
	adapters = $state<AdapterInfo[]>([]);
	#started = false;

	/** Follow the bound adapter's state (once, from the layout). */
	async start() {
		if (this.#started || !isTauri()) return;
		this.#started = true;
		await events.networkStatusChanged.listen((e) => (this.status = e.payload.status));
		this.status = await unwrap(commands.networkStatus());
	}

	accounts = $state<Account[]>([]);

	async loadAccounts() {
		if (!isTauri()) return;
		try {
			this.accounts = await unwrap(commands.accountList());
		} catch (e) {
			this.error = String(e);
		}
	}

	/** Add or change an account; throws so the dialog can show why. */
	async saveAccount(id: number | null, input: AccountInput) {
		if (id == null) await unwrap(commands.accountAdd(input));
		else await unwrap(commands.accountUpdate(id, input));
		await this.loadAccounts();
	}

	async removeAccount(id: number) {
		await this.run(() => unwrap(commands.accountRemove(id)));
		await this.loadAccounts();
	}

	/** Import a cookies.txt into an account; throws with the reason. */
	async importCookies(id: number, path: string): Promise<CookieImport> {
		const result = await unwrap(commands.accountImportCookies(id, path));
		await this.loadAccounts();
		return result;
	}

	async clearCookies(id: number) {
		await this.run(() => unwrap(commands.accountClearCookies(id)));
		await this.loadAccounts();
	}

	/** Host keys trusted for SFTP servers. */
	knownHosts = $state<KnownHost[]>([]);

	async loadKnownHosts() {
		if (!isTauri()) return;
		try {
			this.knownHosts = await unwrap(commands.knownHostsList());
		} catch (e) {
			this.error = String(e);
		}
	}

	async forgetHost(host: string, port: number) {
		await this.run(() => unwrap(commands.knownHostForget(host, port)));
		await this.loadKnownHosts();
	}

	/** The adapters on this machine, asked again each time. */
	async loadAdapters() {
		if (!isTauri()) return;
		try {
			this.adapters = await unwrap(commands.networkAdapters());
		} catch (e) {
			this.error = String(e);
		}
	}

	async load() {
		if (!isTauri()) {
			this.loaded = true;
			return;
		}
		try {
			const overview = await unwrap(commands.proxyOverview());
			this.profiles = overview.profiles;
			this.settings = overview.settings;
			this.error = null;
		} catch (e) {
			this.error = String(e);
		} finally {
			this.loaded = true;
		}
	}

	profile(id: number | null | undefined): ProxyProfile | undefined {
		return this.profiles.find((p) => p.id === id);
	}

	/** "Direct" or the profile's name, for a profile id (`null`: direct). */
	label(id: number | null | undefined): string {
		return id == null ? 'Direct' : (this.profile(id)?.name ?? 'a removed proxy');
	}

	/** A download's choice in words: "Automatic", "Direct" or the proxy's name. */
	choiceLabel(choice: ProxyChoice): string {
		if (choice.mode === 'auto') return 'Automatic';
		if (choice.mode === 'direct') return 'Direct';
		return this.label(choice.id);
	}

	/** Add or change a profile; throws so the dialog can show why. */
	async save(id: number | null, input: ProxyInput) {
		if (id == null) await unwrap(commands.proxyAdd(input));
		else await unwrap(commands.proxyUpdate(id, input));
		await this.load();
	}

	async remove(id: number) {
		await this.run(() => unwrap(commands.proxyRemove(id)));
	}

	/** Save the default and the rules as a whole. */
	async saveSettings(settings: ProxySettings) {
		await this.run(() => unwrap(commands.proxySetSettings(settings)));
	}

	/** Milliseconds the test page took through `choice`; throws with the reason. */
	test = (choice: ProxyChoice, url: string | null = null) => unwrap(commands.proxyTest(choice, url));

	async run(action: () => Promise<unknown>) {
		try {
			await action();
			this.error = null;
		} catch (e) {
			this.error = e instanceof Error ? e.message : String(e);
		}
		await this.load();
	}
}

export const network = new NetworkStore();

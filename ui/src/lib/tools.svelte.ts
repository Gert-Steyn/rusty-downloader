// Live view of the external tools (yt-dlp, ffmpeg), kept in sync by `toolEvents`.
import {
	commands,
	events,
	isTauri,
	unwrap,
	type ToolChannel,
	type ToolEventBatch,
	type ToolInfo,
	type ToolName
} from './api.js';

class ToolsStore {
	tools = $state<ToolInfo[]>([]);
	ready = $state(false);
	error = $state<string | null>(null);

	#started = false;

	async start() {
		if (this.#started) return;
		this.#started = true;
		if (!isTauri()) {
			if (import.meta.env.DEV) {
				const mock = await import('./mock.js');
				this.tools = mock.tools();
			}
			this.ready = true;
			return;
		}
		await events.toolEvents.listen((e) => this.#apply(e.payload));
		await this.refresh();
	}

	async refresh() {
		try {
			this.tools = await unwrap(commands.toolsList());
			this.error = null;
		} catch (e) {
			this.error = String(e);
		} finally {
			this.ready = true;
		}
	}

	tool(name: ToolName): ToolInfo | undefined {
		return this.tools.find((t) => t.name === name);
	}

	// Failures also land on the tool itself (`error`), where the page shows them.
	async #run(name: ToolName, action: () => Promise<unknown>) {
		try {
			await action();
		} catch (e) {
			const tool = this.tool(name);
			if (tool) tool.error = e instanceof Error ? e.message : String(e);
		}
	}

	install = (name: ToolName) => this.#run(name, () => unwrap(commands.toolInstall(name)));
	check = (name: ToolName) => this.#run(name, () => unwrap(commands.toolCheck(name)));
	rollback = (name: ToolName) => this.#run(name, () => unwrap(commands.toolRollback(name)));
	setChannel = (name: ToolName, channel: ToolChannel) =>
		this.#run(name, () => unwrap(commands.toolSetChannel(name, channel)));
	openFolder = () => unwrap(commands.openToolsFolder()).catch((e) => (this.error = String(e)));

	#apply(batch: ToolEventBatch) {
		if (batch.resync) {
			void this.refresh();
			return;
		}
		for (const tool of batch.tools) {
			const i = this.tools.findIndex((t) => t.name === tool.name);
			if (i >= 0) this.tools[i] = tool;
			else this.tools.push(tool);
		}
	}
}

export const tools = new ToolsStore();

// Friendly names and helpers over the generated bindings (bindings.ts).
import { isTauri } from '@tauri-apps/api/core';
import type {
	CommandError,
	Download_Serialize,
	DownloadEvents_Serialize,
	Event_Serialize,
	Package_Serialize
} from './bindings.js';

export { commands, events } from './bindings.js';
export type { AppInfo, Status } from './bindings.js';
export { isTauri };

// The backend serialises these; the UI only ever receives them.
export type Package = Package_Serialize;
export type Download = Download_Serialize;
export type DownloadEvent = Event_Serialize;
export type DownloadEventBatch = DownloadEvents_Serialize;

type CommandResult<T> = { status: 'ok'; data: T } | { status: 'error'; error: CommandError };

/** Await a command and return its data, throwing its error message. */
export async function unwrap<T>(result: Promise<CommandResult<T>>): Promise<T> {
	const r = await result;
	if (r.status === 'error') throw new Error(r.error);
	return r.data;
}

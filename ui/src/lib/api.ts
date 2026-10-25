// Friendly names and helpers over the generated bindings (bindings.ts).
import { isTauri } from '@tauri-apps/api/core';
import type {
	CommandError,
	Download_Serialize,
	DownloadEvents_Serialize,
	Event_Serialize,
	FileCount_Serialize,
	FlagDef_Serialize,
	GalleryDlFlags_Serialize,
	GalleryDlOptions_Serialize,
	Gallery_Serialize,
	GrabbedLink_Serialize,
	GrabberEvents_Serialize,
	MediaSummary_Serialize,
	Package_Serialize,
	ToolEvents_Serialize,
	ToolInfo_Serialize,
	ToolTask_Serialize,
	YtDlpOptions_Serialize
} from './bindings.js';

export { commands, events } from './bindings.js';
export type {
	AppInfo,
	Availability,
	FormatPreset,
	LinkSource,
	Status,
	ToolChannel,
	ToolName
} from './bindings.js';
export { isTauri };

// The backend serialises these; the UI only ever receives them.
export type Package = Package_Serialize;
export type Download = Download_Serialize;
export type DownloadEvent = Event_Serialize;
export type DownloadEventBatch = DownloadEvents_Serialize;
export type GrabbedLink = GrabbedLink_Serialize;
export type GrabberEventBatch = GrabberEvents_Serialize;
export type MediaSummary = MediaSummary_Serialize;
export type YtDlpOptions = YtDlpOptions_Serialize;
export type Gallery = Gallery_Serialize;
export type GalleryDlOptions = GalleryDlOptions_Serialize;
export type GalleryDlFlags = GalleryDlFlags_Serialize;
export type FlagDef = FlagDef_Serialize;
export type FileCount = FileCount_Serialize;
export type ToolInfo = ToolInfo_Serialize;
export type ToolTask = ToolTask_Serialize;
export type ToolEventBatch = ToolEvents_Serialize;

type CommandResult<T> = { status: 'ok'; data: T } | { status: 'error'; error: CommandError };

/** Await a command and return its data, throwing its error message. */
export async function unwrap<T>(result: Promise<CommandResult<T>>): Promise<T> {
	const r = await result;
	if (r.status === 'error') throw new Error(r.error);
	return r.data;
}

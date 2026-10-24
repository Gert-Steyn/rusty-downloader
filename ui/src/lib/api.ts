// Thin typed wrappers over Tauri commands. Shapes mirror src-tauri/src/commands.rs.
import { invoke, isTauri } from '@tauri-apps/api/core';

export { isTauri };

export interface AppInfo {
	version: string;
	portable: boolean;
	dataDir: string;
	dbFile: string;
	logsDir: string;
	schemaVersion: number;
}

export function appInfo(): Promise<AppInfo> {
	return invoke<AppInfo>('app_info');
}

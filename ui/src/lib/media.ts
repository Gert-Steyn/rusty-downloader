// yt-dlp format presets as the UI shows them.
import type { FormatPreset, YtDlpOptions } from './api.js';

export interface PresetChoice {
	value: FormatPreset;
	label: string;
	/** Height cap, for presets that limit resolution. */
	height?: number;
}

export const PRESETS: PresetChoice[] = [
	{ value: 'best', label: 'Best quality' },
	{ value: 'max2160', label: 'Up to 4K', height: 2160 },
	{ value: 'max1080', label: 'Up to 1080p', height: 1080 },
	{ value: 'max720', label: 'Up to 720p', height: 720 },
	{ value: 'max480', label: 'Up to 480p', height: 480 },
	{ value: 'audioMp3', label: 'Audio only (MP3)' },
	{ value: 'audioM4a', label: 'Audio only (M4A)' },
	{ value: 'custom', label: 'Custom format…' }
];

/**
 * Presets worth offering for a video with these heights: a height cap only
 * shows when it would actually pick something lower than the best. Unknown
 * heights (playlist entries) get every preset. `current` is always kept.
 */
export function presetsFor(heights: number[], current?: FormatPreset): PresetChoice[] {
	if (!heights.length) return PRESETS;
	const best = Math.max(...heights);
	return PRESETS.filter((p) => p.value === current || p.height == null || p.height < best);
}

export function presetLabel(options: YtDlpOptions): string {
	if (options.preset === 'custom') return `Custom: ${options.customFormat || 'bv*+ba/b'}`;
	return PRESETS.find((p) => p.value === options.preset)?.label ?? options.preset;
}

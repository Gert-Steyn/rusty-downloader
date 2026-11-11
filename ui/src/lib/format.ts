import type { ChecksumAlgo, Download, Package, Status } from './api.js';

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

export function formatBytes(bytes: number | null | undefined): string {
	if (bytes == null) return '—';
	if (bytes < 1024) return `${bytes} B`;
	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < UNITS.length - 1) {
		value /= 1024;
		unit++;
	}
	const digits = value < 10 ? 2 : value < 100 ? 1 : 0;
	return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

export function formatSpeed(bytesPerSecond: number | null | undefined): string {
	return bytesPerSecond ? `${formatBytes(bytesPerSecond)}/s` : '—';
}

export function formatEta(remaining: number | null, bytesPerSecond: number): string {
	if (remaining == null || remaining <= 0 || !bytesPerSecond) return '—';
	const total = Math.ceil(remaining / bytesPerSecond);
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = total % 60;
	if (h > 0) return `${h}h ${m}m`;
	if (m > 0) return `${m}m ${s}s`;
	return `${s}s`;
}

/** 0–100, or null when the total is unknown. */
export function percent(done: number, total: number | null | undefined): number | null {
	if (!total) return null;
	return Math.min(100, (done / total) * 100);
}

/**
 * Progress of one download, 0–100 or null when unknown. Galleries count
 * files, since their total size is rarely known up front.
 */
export function downloadPercent(d: Download): number | null {
	if (d.status === 'completed') return 100;
	if (d.files) return d.files.total ? percent(d.files.done, d.files.total) : null;
	return percent(d.downloaded, d.size);
}

/** Short progress text: `42%`, or `12/40 files` for a gallery. */
export function progressText(d: Download): string {
	if (d.files && d.status !== 'completed') {
		if (!d.files.done && !d.files.total) return '';
		return d.files.total ? `${d.files.done}/${d.files.total}` : `${d.files.done} files`;
	}
	const pct = downloadPercent(d);
	return pct == null ? '' : `${pct.toFixed(pct < 10 ? 1 : 0)}%`;
}

export function iconFor(kind: string): 'video' | 'gallery' | 'torrent' | 'file' {
	if (kind === 'torrent') return 'torrent';
	return kind === 'ytdlp' ? 'video' : kind === 'gallerydl' ? 'gallery' : 'file';
}

/** `1 h 12 min`, `45 min`, or `Under 1 min` for a seeding time. */
export function formatSeedTime(seconds: number | null | undefined): string {
	const total = Math.floor(seconds ?? 0);
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	if (h > 0) return `${h} h ${m} min`;
	if (m > 0) return `${m} min`;
	return 'Under 1 min';
}

/** Upload ratio (uploaded over size), 2 decimals, or `—` when the size is unknown or 0. */
export function formatRatio(uploaded: number, size: number | null | undefined): string {
	return size ? (uploaded / size).toFixed(2) : '—';
}

/** `3:07` or `1:02:03`. */
export function formatDuration(seconds: number | null | undefined): string {
	if (seconds == null || !Number.isFinite(seconds)) return '';
	const total = Math.round(seconds);
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = String(total % 60).padStart(2, '0');
	return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

/** "just now", "5 minutes ago", "3 days ago", or the date once it's months back. */
export function formatAgo(ms: number, now = Date.now()): string {
	const minutes = Math.round((now - ms) / 60_000);
	const unit = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'} ago`;
	if (minutes < 1) return 'just now';
	if (minutes < 60) return unit(minutes, 'minute');
	const hours = Math.round(minutes / 60);
	if (hours < 24) return unit(hours, 'hour');
	const days = Math.round(hours / 24);
	if (days < 45) return unit(days, 'day');
	return `on ${new Date(ms).toLocaleDateString(undefined, { dateStyle: 'medium' })}`;
}

export function formatDate(ms: number | null | undefined): string {
	if (ms == null) return '—';
	return new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

export const CHECKSUM_LABELS: Record<ChecksumAlgo, string> = {
	md5: 'MD5',
	sha1: 'SHA-1',
	sha256: 'SHA-256',
	crc32: 'CRC32'
};

export const STATUS_LABELS: Record<Status, string> = {
	queued: 'Queued',
	active: 'Downloading',
	paused: 'Paused',
	completed: 'Completed',
	failed: 'Failed',
	canceled: 'Canceled'
};

export function displayName(d: Download): string {
	if (d.filename) return d.filename;
	// yt-dlp picks the name itself: show the planned file, else the title.
	// (A gallery's planned path is its folder.)
	const planned = d.destPath?.split(/[\\/]/).pop();
	if (planned) return planned;
	if (d.title) return d.title;
	try {
		const url = new URL(d.sourceUrl);
		return decodeURIComponent(url.pathname.split('/').filter(Boolean).pop() ?? '') || url.host;
	} catch {
		return d.sourceUrl;
	}
}

export interface PackageSummary {
	size: number | null;
	downloaded: number;
	status: Status;
	completed: number;
}

/** Totals for a package row. Size is unknown if any download's size is. */
export function summarize(p: Package): PackageSummary {
	let size: number | null = 0;
	let downloaded = 0;
	let completed = 0;
	const seen = new Set<Status>();
	for (const d of p.downloads) {
		size = size == null || d.size == null ? null : size + d.size;
		downloaded += d.downloaded;
		if (d.status === 'completed') completed++;
		seen.add(d.status);
	}
	const order: Status[] = ['active', 'queued', 'failed', 'paused', 'canceled', 'completed'];
	const status = order.find((s) => seen.has(s)) ?? 'completed';
	return { size: p.downloads.length ? size : null, downloaded, status, completed };
}

import type { Download, Package, Status } from './api.js';

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

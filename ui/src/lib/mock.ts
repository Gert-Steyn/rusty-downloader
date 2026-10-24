// Sample data for previewing the UI in a plain browser (`pnpm ui:dev`).
// Never loaded inside the app.
import type { Availability, Download, GrabbedLink, Package, Status, ToolInfo } from './api.js';
import type { Live } from './downloads.svelte.js';

const MB = 1024 * 1024;
let nextId = 1;

function dl(
	packageId: number,
	filename: string | null,
	size: number | null,
	fraction: number,
	status: Status,
	error: string | null = null
): Download {
	return {
		id: nextId++,
		packageId,
		kind: 'http',
		sourceUrl: `https://downloads.example.com/files/${filename ?? 'pending-link'}`,
		filename,
		destPath: filename ? `C:\\Users\\You\\Downloads\\${filename}` : null,
		size,
		downloaded: Math.round((size ?? 0) * fraction),
		status,
		error,
		ytdlp: null,
		title: null
	};
}

export function packages(): Package[] {
	const pkg = (id: number, name: string, downloads: Download[]): Package => ({
		id,
		name,
		destDir: 'C:\\Users\\You\\Downloads',
		queueId: 1,
		priority: 0,
		downloads
	});
	return [
		pkg(1, 'Ubuntu ISO', [dl(1, 'ubuntu-26.04-desktop-amd64.iso', 6144 * MB, 0.42, 'active')]),
		pkg(2, 'Dataset (3 parts)', [
			dl(2, 'dataset.part1.rar', 1024 * MB, 1, 'completed'),
			dl(2, 'dataset.part2.rar', 1024 * MB, 0.17, 'active'),
			dl(2, 'dataset.part3.rar', 512 * MB, 0, 'queued')
		]),
		pkg(3, 'Conference talks', [
			dl(3, 'keynote.mp4', 850 * MB, 0.63, 'paused'),
			dl(3, 'panel.mp4', null, 0, 'failed', 'link expired or access denied (HTTP 403)'),
			dl(3, null, null, 0, 'queued')
		]),
		pkg(4, 'Drivers', [dl(4, 'gpu-driver-setup.exe', 742 * MB, 1, 'completed')]),
		pkg(5, 'Big Buck Bunny', [
			{
				...dl(5, null, 412 * MB, 0.3, 'active'),
				kind: 'ytdlp',
				sourceUrl: 'https://www.youtube.com/watch?v=aqz-KE-bpKQ',
				destPath: 'C:\\Users\\You\\Downloads\\Big Buck Bunny [aqz-KE-bpKQ].mp4',
				ytdlp: { preset: 'max1080', customFormat: null },
				title: 'Big Buck Bunny'
			}
		])
	];
}

export function grabbedLinks(): GrabbedLink[] {
	let id = 100;
	const link = (
		path: string,
		pkg: string,
		availability: Availability,
		size: number | null,
		error: string | null = null
	): GrabbedLink => ({
		id: id++,
		url: `https://files.example.net/${path}`,
		host: 'files.example.net',
		filename: availability === 'checking' || availability === 'unknown' ? null : path.split('/').pop()!,
		size,
		availability,
		error,
		package: pkg,
		destDir: null,
		kind: 'http',
		media: null,
		ytdlp: null
	});
	const video = (
		vid: string,
		title: string,
		pkg: string,
		size: number | null,
		heights: number[],
		duration: number
	): GrabbedLink => ({
		id: id++,
		url: `https://www.youtube.com/watch?v=${vid}`,
		host: 'www.youtube.com',
		filename: title,
		size,
		availability: 'online',
		error: null,
		package: pkg,
		destDir: null,
		kind: 'ytdlp',
		media: {
			title,
			uploader: 'Blender',
			duration,
			thumbnail: null,
			extractor: 'Youtube',
			heights,
			isLive: false
		},
		ytdlp: { preset: 'best', customFormat: null }
	});
	return [
		link('movie.part1.rar', 'movie', 'online', 1024 * MB),
		link('movie.part2.rar', 'movie', 'online', 1024 * MB),
		link('movie.part3.rar', 'movie', 'checking', null),
		link('notes.pdf', 'files.example.net', 'online', 2 * MB),
		link('old/archive.zip', 'files.example.net', 'offline', null, 'server returned HTTP 404'),
		link('slow/report.docx', 'files.example.net', 'error', null, 'operation timed out'),
		video('aqz-KE-bpKQ', 'Big Buck Bunny 60fps 4K', 'Big Buck Bunny 60fps 4K', 640 * MB, [2160, 1440, 1080, 720, 480, 360], 634),
		video('eRsGyueVLvQ', 'Sintel', 'Open Movies', null, [], 888),
		video('R6MlUcmOul8', 'Tears of Steel', 'Open Movies', null, [], 734)
	];
}

export function tools(): ToolInfo[] {
	const day = 24 * 60 * 60 * 1000;
	const now = Date.now();
	return [
		{
			name: 'ytdlp',
			channel: 'stable',
			path: 'C:\\Users\\You\\AppData\\Roaming\\rusty-downloader\\tools\\yt-dlp\\2026.10.07-1a2b3c4d\\yt-dlp.exe',
			version: '2026.10.07',
			release: '2026.10.07',
			installedAt: now - 17 * day,
			latestRelease: '2026.10.21',
			updateAvailable: true,
			lastCheck: now - 2 * 60 * 60 * 1000,
			previousVersion: '2026.09.26',
			task: null,
			error: null
		},
		{
			name: 'ffmpeg',
			channel: 'stable',
			path: null,
			version: null,
			release: null,
			installedAt: null,
			latestRelease: null,
			updateAvailable: false,
			lastCheck: null,
			previousVersion: null,
			task: { type: 'downloading', downloaded: 38 * MB, total: 91 * MB },
			error: null
		}
	];
}

export function live(): Record<number, Live> {
	return {
		1: { speedBps: 18.4 * MB, connections: 8 },
		3: { speedBps: 6.1 * MB, connections: 4 }
	};
}

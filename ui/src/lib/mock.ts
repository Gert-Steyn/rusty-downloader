// Sample data for previewing the UI in a plain browser (`pnpm ui:dev`).
// Never loaded inside the app.
import type {
	Availability,
	Download,
	GalleryDlFlags,
	GrabbedLink,
	HistoryEntry,
	Package,
	Queue,
	Settings,
	Status,
	TorrentFileProgress,
	TorrentInfo,
	ToolInfo
} from './api.js';
import type { Live, Seed } from './downloads.svelte.js';

const MB = 1024 * 1024;
let nextId = 1;

/** Sintel's files as its metadata lists them: folders, and one that is skipped by default. */
const SINTEL_FILES: TorrentInfo['files'] = [
	{ index: 0, path: 'Sintel/Sintel.mp4', size: 620 * MB },
	{ index: 1, path: 'Sintel/Subtitles/en.srt', size: 48 * 1024 },
	{ index: 2, path: 'Sintel/Subtitles/de.srt', size: 47 * 1024 },
	{ index: 3, path: 'Sintel/Extras/making-of.mkv', size: 380 * MB }
];

/** The files Sintel's sample download chooses (0, 1, 2): its size on disk. */
const SINTEL_SELECTED_SIZE = SINTEL_FILES.filter((f) => f.index !== 3).reduce((sum, f) => sum + f.size, 0);

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
		gallerydl: null,
		torrent: null,
		files: null,
		title: null,
		speedLimit: null,
		checksum: null,
		proxy: { mode: 'auto' },
		uploaded: 0,
		seeding: false,
		seededSecs: 0
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
			{
				...dl(2, 'dataset.part1.rar', 1024 * MB, 1, 'completed'),
				checksum: { algo: 'crc32', value: '1a2b3c4d', verified: true }
			},
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
		]),
		pkg(6, 'Mountain trip', [
			{
				...dl(6, 'Mountain trip', null, 0, 'active'),
				kind: 'gallerydl',
				sourceUrl: 'https://imgur.com/a/mountain',
				destPath: 'C:\\Users\\You\\Downloads\\Mountain trip',
				downloaded: 31 * MB,
				gallerydl: {
					archive: true,
					userConfig: false,
					flags: { range: '1-40' },
					rawArgs: null,
					keepFolders: false,
					folderPattern: null
				},
				files: { done: 12, total: 40 },
				title: 'Mountain trip'
			}
		]),
		pkg(7, 'Sintel', [
			{
				...dl(7, 'Sintel', SINTEL_SELECTED_SIZE, 1, 'completed'),
				kind: 'torrent',
				sourceUrl: 'magnet:?xt=urn:btih:08ada5a7a6183aae1e09d831df6748d566095a10&dn=Sintel',
				destPath: 'C:\\Users\\You\\Downloads\\Sintel',
				torrent: { onlyFiles: [0, 1, 2], customSeed: false, seedRatio: null, seedMinutes: null },
				uploaded: 400 * MB,
				seeding: true,
				seededSecs: 2400
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
		ytdlp: null,
		gallery: null,
		gallerydl: null,
		torrent: null,
		torrentOptions: null,
		previous:
			path === 'notes.pdf'
				? { finishedAt: Date.now() - 3 * 24 * 3600 * 1000, path: 'C:\\Users\\You\\Downloads\\notes.pdf', exists: true }
				: null
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
		ytdlp: { preset: 'best', customFormat: null },
		gallery: null,
		gallerydl: null,
		torrent: null,
		torrentOptions: null,
		previous: null
	});
	const gallery = (path: string, title: string, files: number, category: string, sub: string): GrabbedLink => ({
		id: id++,
		url: `https://imgur.com/${path}`,
		host: 'imgur.com',
		filename: title,
		size: files * 2 * MB,
		availability: 'online',
		error: null,
		package: title,
		destDir: null,
		kind: 'gallerydl',
		media: null,
		ytdlp: null,
		gallery: {
			title,
			category,
			subcategory: sub,
			files,
			subGalleries: 0,
			size: files * 2 * MB,
			thumbnails: [],
			partial: false
		},
		gallerydl: {
			archive: true,
			userConfig: false,
			flags: {},
			rawArgs: null,
			keepFolders: false,
			folderPattern: null
		},
		torrent: null,
		torrentOptions: null,
		previous: null
	});
	// A magnet link has no host. Its files are known once the torrent's metadata
	// arrived (`torrent`); `onlyFiles` is the selection, and the size is its total.
	const magnet = (
		hash: string,
		title: string,
		pkg: string,
		torrent: TorrentInfo | null,
		onlyFiles: number[] | null = null,
		error: string | null = null
	): GrabbedLink => {
		const files = torrent ? torrent.files.filter((f) => !onlyFiles || onlyFiles.includes(f.index)) : [];
		return {
			id: id++,
			url: `magnet:?xt=urn:btih:${hash}&dn=${encodeURIComponent(title)}`,
			host: '',
			filename: title,
			size: torrent ? files.reduce((sum, f) => sum + f.size, 0) : null,
			availability: 'online',
			error,
			package: pkg,
			destDir: null,
			kind: 'torrent',
			media: null,
			ytdlp: null,
			gallery: null,
			gallerydl: null,
			torrent,
			torrentOptions: torrent ? { onlyFiles, customSeed: false, seedRatio: null, seedMinutes: null } : null,
			previous: null
		};
	};
	const SINTEL_HASH = '08ada5a7a6183aae1e09d831df6748d566095a10';
	const sintel: TorrentInfo = {
		infoHash: SINTEL_HASH,
		name: 'Sintel',
		size: SINTEL_FILES.reduce((sum, f) => sum + f.size, 0),
		files: SINTEL_FILES
	};
	return [
		link('movie.part1.rar', 'movie', 'online', 1024 * MB),
		link('movie.part2.rar', 'movie', 'online', 1024 * MB),
		link('movie.part3.rar', 'movie', 'checking', null),
		link('notes.pdf', 'files.example.net', 'online', 2 * MB),
		link('old/archive.zip', 'files.example.net', 'offline', null, 'server returned HTTP 404'),
		link('slow/report.docx', 'files.example.net', 'error', null, 'operation timed out'),
		video('aqz-KE-bpKQ', 'Big Buck Bunny 60fps 4K', 'Big Buck Bunny 60fps 4K', 640 * MB, [2160, 1440, 1080, 720, 480, 360], 634),
		video('eRsGyueVLvQ', 'Sintel', 'Open Movies', null, [], 888),
		video('R6MlUcmOul8', 'Tears of Steel', 'Open Movies', null, [], 734),
		gallery('a/mountain', 'Mountain trip', 40, 'imgur', 'album'),
		magnet(SINTEL_HASH, 'Sintel', 'Open Movies', sintel, [0, 1, 2]),
		magnet('c9e15763f722f23e98a29decdfae341b98d53056', 'Elephants Dream', 'Open Movies', null, null, "Couldn't get the file list: no peers answered yet")
	];
}

/** The files of the sample torrent download (`Sintel`) while it runs. */
export function torrentFiles(): TorrentFileProgress[] {
	const chosen = [0, 1, 2];
	return SINTEL_FILES.map((f) => {
		const selected = chosen.includes(f.index);
		const downloaded = f.index === 1 ? f.size : selected ? Math.round(f.size * 0.37) : 0;
		return { index: f.index, path: f.path, size: f.size, downloaded, selected };
	});
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
			name: 'gallerydl',
			channel: 'stable',
			path: 'C:\\Users\\You\\AppData\\Roaming\\rusty-downloader\\tools\\gallery-dl\\v1.32.17-77c8f4c6\\gallery-dl.exe',
			version: '1.32.17',
			release: 'v1.32.17',
			installedAt: now - 3 * day,
			latestRelease: 'v1.32.17',
			updateAvailable: false,
			lastCheck: now - 2 * 60 * 60 * 1000,
			previousVersion: null,
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

/** A cut-down schema; the real one is `crates/rdl-core/src/gallerydl/flags.json`. */
export function galleryFlags(): GalleryDlFlags {
	return {
		schema: {
			version: 1,
			groups: [
				{
					id: 'selection',
					label: 'Selection',
					flags: [
						{ id: 'range', flag: '--range', kind: 'text', label: 'Files', help: null, placeholder: '1-20', template: null, choices: [] },
						{ id: 'abort', flag: '--abort', kind: 'number', label: 'Stop after N existing files', help: null, placeholder: '10', template: null, choices: [] }
					]
				},
				{
					id: 'impersonation',
					label: 'Impersonation',
					flags: [
						{
							id: 'browser',
							flag: '-o',
							kind: 'choice',
							label: 'Mimic browser',
							help: "Sends the browser's headers and TLS cipher order.",
							placeholder: null,
							template: 'browser={}',
							choices: [
								{ value: 'firefox', label: 'Firefox' },
								{ value: 'chrome', label: 'Chrome' }
							]
						}
					]
				},
				{
					id: 'postprocess',
					label: 'Metadata and post-processing',
					flags: [
						{ id: 'zip', flag: '--zip', kind: 'switch', label: 'Pack into a .zip', help: null, placeholder: null, template: null, choices: [] },
						{ id: 'ugoira', flag: '--ugoira', kind: 'choice', label: 'Convert Pixiv ugoira to', help: null, placeholder: null, template: null, choices: [{ value: 'webm', label: 'WebM' }] }
					]
				}
			]
		},
		available: ['--range', '--abort', '-o', '--zip']
	};
}

export function live(): Record<number, Live> {
	return {
		1: { speedBps: 18.4 * MB, connections: 8 },
		3: { speedBps: 6.1 * MB, connections: 4 },
	};
}

/** Seeding torrents: the Sintel torrent (download id 11) seeds to two peers. */
export function seeds(): Record<number, Seed> {
	return {
		11: { uploadBps: 540 * 1024, peers: 2 }
	};
}

export function queues(): Queue[] {
	const schedule = { enabled: false, days: [0, 1, 2, 3, 4, 5, 6], start: '01:00', stop: '07:00' };
	return [
		{ id: 1, name: 'Main', maxActive: 3, schedule, onComplete: 'none', open: true, packages: 3, active: 2, queued: 1, paused: 1 },
		{
			id: 2,
			name: 'Overnight',
			maxActive: 2,
			schedule: { ...schedule, enabled: true },
			onComplete: 'shutdown',
			open: false,
			packages: 1,
			active: 0,
			queued: 4,
			paused: 0
		}
	];
}

export function settings(): Settings {
	return {
		downloadDir: null,
		maxActive: 3,
		maxConnections: 8,
		clipboardWatch: true,
		clipboardAsk: false,
		clipboardIgnore: ['*youtube.com/shorts/*'],
		startWithWindows: false,
		startMinimized: true,
		closeToTray: true,
		notifyFinished: true,
		notifyFailed: true,
		warnDuplicates: true,
		keepAwake: true,
		speedLimitOn: false,
		speedLimit: 2 * 1024 * 1024,
		speedSchedule: [{ days: [0, 1, 2, 3, 4], from: '08:00', to: '18:00', limit: 1024 * 1024 }],
		bindAdapter: null,
		killSwitch: true,
		seedRatio: 1,
		seedMinutes: null,
		uploadLimit: null
	};
}

export function history(search = ''): HistoryEntry[] {
	const hour = 3600 * 1000;
	const now = Date.now();
	const entry = (
		id: number,
		name: string,
		kind: string,
		size: number,
		ago: number,
		exists = true,
		pkg: string | null = null
	): HistoryEntry => ({
		id,
		url: `https://downloads.example.com/files/${encodeURIComponent(name)}`,
		path: `C:\\Users\\You\\Downloads\\${name}`,
		name,
		size,
		kind,
		package: pkg ?? name,
		finishedAt: now - ago * hour,
		exists
	});
	const all = [
		entry(1, 'gpu-driver-setup.exe', 'http', 742 * MB, 2, true, 'Drivers'),
		entry(2, 'dataset.part1.rar', 'http', 1024 * MB, 5, true, 'Dataset (3 parts)'),
		entry(3, 'Big Buck Bunny [aqz-KE-bpKQ].mp4', 'ytdlp', 412 * MB, 26),
		entry(4, 'Mountain trip', 'gallerydl', 61 * MB, 50),
		entry(5, 'notes.pdf', 'http', 2 * MB, 72, false, 'files.example.net')
	];
	const q = search.trim().toLowerCase();
	return q ? all.filter((e) => `${e.name} ${e.url} ${e.package}`.toLowerCase().includes(q)) : all;
}

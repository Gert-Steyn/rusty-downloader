<script lang="ts" module>
	// Lucide icons (Graphite icon set), mapped to the app's own names.
	import {
		ArrowDownToLine,
		ArrowUpToLine,
		Copy,
		Download,
		ExternalLink,
		File,
		FileUp,
		Folder,
		Gauge,
		Globe,
		History,
		Images,
		Link,
		ListOrdered,
		Package,
		Pause,
		Play,
		Plus,
		Puzzle,
		RefreshCw,
		Settings,
		ShieldCheck,
		Magnet,
		Trash2,
		Undo2,
		Upload,
		Video,
		Wrench,
		X,
		ChevronDown,
		ChevronRight
	} from '@lucide/svelte';

	const icons = {
		downloads: Download,
		link: Link,
		queues: ListOrdered,
		history: History,
		plugins: Puzzle,
		tools: Wrench,
		settings: Settings,
		plus: Plus,
		play: Play,
		pause: Pause,
		trash: Trash2,
		'chevron-right': ChevronRight,
		'chevron-down': ChevronDown,
		folder: Folder,
		package: Package,
		open: ExternalLink,
		close: X,
		file: File,
		'file-up': FileUp,
		video: Video,
		torrent: Magnet,
		gallery: Images,
		refresh: RefreshCw,
		undo: Undo2,
		copy: Copy,
		'move-top': ArrowUpToLine,
		'move-bottom': ArrowDownToLine,
		speed: Gauge,
		checksum: ShieldCheck,
		network: Globe,
		upload: Upload
	} as const;

	export type IconName = keyof typeof icons;
</script>

<script lang="ts">
	/** Graphite sizes: 16 for UI (the default), 24 only for empty states. */
	let { name, size = 16 }: { name: IconName; size?: 16 | 24 } = $props();
	const Component = $derived(icons[name]);
</script>

<Component {size} strokeWidth={1.5} aria-hidden="true" />

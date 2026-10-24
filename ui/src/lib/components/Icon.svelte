<script lang="ts" module>
	// 24x24 stroke icons, drawn to sit at Fluent's 16/20px icon sizes.
	const paths = {
		downloads: 'M12 3v12M7 10l5 5 5-5M5 21h14',
		link: 'M10 13a5 5 0 0 0 7.07 0l3-3a5 5 0 0 0-7.07-7.07l-1.5 1.5M14 11a5 5 0 0 0-7.07 0l-3 3a5 5 0 0 0 7.07 7.07l1.5-1.5',
		queues: 'M8 6h13M8 12h13M8 18h13M3.5 6h.01M3.5 12h.01M3.5 18h.01',
		history: 'M12 7v5l3 2M21 12a9 9 0 1 1-9-9 9 9 0 0 1 9 9z',
		plugins: 'M9 2v5M15 2v5M6 7h12v4a6 6 0 0 1-12 0zM12 17v5',
		tools: 'M21 8l-9-5-9 5 9 5zM3 8v8l9 5 9-5V8M12 13v8',
		settings: 'M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6',
		plus: 'M12 5v14M5 12h14',
		play: 'M7 4l13 8-13 8z',
		pause: 'M8 4v16M16 4v16',
		trash: 'M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3'
	} as const;

	export type IconName = keyof typeof paths;
</script>

<script lang="ts">
	let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<svg
	width={size}
	height={size}
	viewBox="0 0 24 24"
	fill="none"
	stroke="currentColor"
	stroke-width="1.75"
	stroke-linecap="round"
	stroke-linejoin="round"
	aria-hidden="true"
>
	<path d={paths[name]} />
</svg>

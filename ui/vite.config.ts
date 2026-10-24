import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// Tauri expects a fixed port and must not clear the terminal so Rust errors stay visible.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
	plugins: [
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},

			// Tauri serves a static SPA: no SSR, fallback page handles client-side routing.
			adapter: adapter({ fallback: 'index.html' })
		})
	],
	clearScreen: false,
	server: {
		port: 1420,
		strictPort: true,
		host: host || false,
		hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
		watch: { ignored: ['**/src-tauri/**'] }
	}
});

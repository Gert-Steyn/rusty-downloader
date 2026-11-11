// Appearance choice (System / Dark / Light). Graphite reads `data-theme` on <html>;
// no attribute means "follow the OS". Stored per device in localStorage.

export type ThemeChoice = 'system' | 'dark' | 'light';

const KEY = 'rdl.theme';

export function loadTheme(): ThemeChoice {
	try {
		const v = localStorage.getItem(KEY);
		if (v === 'dark' || v === 'light') return v;
	} catch {
		// Storage unavailable: fall back to the OS setting.
	}
	return 'system';
}

export function applyTheme(choice: ThemeChoice) {
	const root = document.documentElement;
	if (choice === 'system') root.removeAttribute('data-theme');
	else root.setAttribute('data-theme', choice);
}

export function saveTheme(choice: ThemeChoice) {
	applyTheme(choice);
	try {
		if (choice === 'system') localStorage.removeItem(KEY);
		else localStorage.setItem(KEY, choice);
	} catch {
		// Not persisted; the choice still applies for this session.
	}
}

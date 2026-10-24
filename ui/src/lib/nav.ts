import type { IconName } from './components/Icon.svelte';

export interface NavItem {
	href: string;
	label: string;
	icon: IconName;
}

export const mainNav: NavItem[] = [
	{ href: '/', label: 'Downloads', icon: 'downloads' },
	{ href: '/linkgrabber', label: 'LinkGrabber', icon: 'link' },
	{ href: '/queues', label: 'Queues', icon: 'queues' },
	{ href: '/history', label: 'History', icon: 'history' },
	{ href: '/plugins', label: 'Plugins', icon: 'plugins' },
	{ href: '/tools', label: 'Tools', icon: 'tools' }
];

export const footerNav: NavItem[] = [{ href: '/settings', label: 'Settings', icon: 'settings' }];

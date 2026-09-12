import { Store } from 'tauri-plugin-store-api';
import { watch } from 'tauri-plugin-fs-watch-api';
import { invoke } from '@tauri-apps/api';
import { listen } from '@tauri-apps/api/event';
import { warn } from 'tauri-plugin-log-api';
import toast from 'react-hot-toast';
import i18n from '../i18n';
import { createConfigCoordinator } from './config_coordinator';

// Read-only consumers use the installed plugin against the exact native path.
export let store;
export const config = createConfigCoordinator({
    commit: (operations) => invoke('config_commit', { operations }),
    read: () => invoke('config_snapshot'),
    onError: () => toast.error(i18n.t('config.save_failed')),
});

export async function initStore() {
    const path = await invoke('config_path');
    store = new Store(path);
    const unlisten = await listen('config_committed', (event) => config.accept(event.payload));
    await config.refresh();
    // All windows may observe the same save; reload compares complete snapshots
    // under the native lock, so self notifications are harmless no-ops.
    const unwatch = await watch(path, async () => {
        try {
            config.accept(await invoke('reload_store'));
        } catch {
            warn('Config: external reload failed; preserving current settings.');
        }
    });
    window.addEventListener(
        'beforeunload',
        () => {
            unlisten();
            unwatch();
        },
        { once: true }
    );
}

export const commitConfig = (operations) => config.batch(operations);

import { useCallback, useEffect, useRef, useState } from 'react';
import toast from 'react-hot-toast';
import i18n from '../i18n';
import { config } from '../utils/store';

export const useConfig = (key, defaultValue, options = {}) => {
    const { sync = true, initialize = sync } = options;
    const [property, setState] = useState(null);
    const current = useRef(null);
    const edited = useRef(false);
    const setLocal = useCallback((value) => {
        current.current = value;
        setState(value);
    }, []);

    useEffect(() => {
        let active = true;
        edited.current = false;
        const unlisten = config.subscribe(key, (value) => {
            if (active && (sync || !edited.current)) setLocal(value ?? defaultValue);
        });
        config
            .initialize(key, defaultValue, initialize)
            .then((value) => {
                if (active && !edited.current) setLocal(value);
            })
            .catch(() => {
                if (active) {
                    setLocal(defaultValue);
                    toast.error(i18n.t('config.save_failed'));
                }
            });
        return () => {
            active = false;
            unlisten();
        };
    }, [key]);

    const setProperty = useCallback(
        (value, forceSync = false) => {
            edited.current = true;
            setLocal(value);
            if (forceSync || sync) {
                return config.set(key, value, forceSync).catch((error) => {
                    setLocal(config.value(key) ?? defaultValue);
                    throw error;
                });
            }
            return Promise.resolve();
        },
        [key, sync]
    );

    const getProperty = useCallback(() => current.current, []);
    return [property, setProperty, getProperty];
};

export const deleteKey = (key) => config.batch([{ kind: 'delete', key }]);

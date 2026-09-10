import { TbLanguage } from 'react-icons/tb';
import { invoke } from '@tauri-apps/api';
import React from 'react';

import { useConfig } from '../../hooks';

// Rust positions, shows and hides this window; all it does is report that the
// user engaged with it. Fixed blue in both themes - at 18px there is not enough
// surface for a theme-aware treatment to read as anything but grey mush.
export default function PopButton() {
    const [trigger] = useConfig('pop_button_trigger', 'hover');

    const fire = () => {
        void invoke('pop_button_translate');
    };

    return (
        <div
            className='w-screen h-screen flex items-center justify-center rounded-[5px] bg-[#d5e1fa] cursor-pointer select-none'
            onMouseEnter={trigger === 'hover' ? fire : undefined}
            onClick={trigger === 'click' ? fire : undefined}
        >
            <TbLanguage className='text-[12px] text-[#4a7dfc]' />
        </div>
    );
}

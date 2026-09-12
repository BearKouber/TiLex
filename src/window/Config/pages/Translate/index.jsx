import { DropdownTrigger } from '@nextui-org/react';
import { DropdownMenu } from '@nextui-org/react';
import { DropdownItem } from '@nextui-org/react';
import { useTranslation } from 'react-i18next';
import { CardBody } from '@nextui-org/react';
import { Dropdown } from '@nextui-org/react';
import { Input } from '@nextui-org/react';
import { Button } from '@nextui-org/react';
import { Card } from '@nextui-org/react';
import React, { useRef, useState } from 'react';
import toast from 'react-hot-toast';

import { languageList } from '../../../../utils/language';
import { useConfig } from '../../../../hooks/useConfig';
import { useToastStyle } from '../../../../hooks';
import { osType } from '../../../../utils/env';
import { invoke } from '@tauri-apps/api';

// 截图翻译的快捷键住在这一页，不在「服务设置」里 —— 服务设置只管有哪些引擎，
// 怎么触发（划词、快捷键）都归这一页。
//
// 键盘 code → 快捷键字符串里的写法。这张表是从批次 4 删掉的快捷键设置页里捞
// 回来的，凡是 code 名和字面量对不上的都在这儿。
const keyMap = {
    Backquote: '`',
    Backslash: '\\',
    BracketLeft: '[',
    BracketRight: ']',
    Comma: ',',
    Equal: '=',
    Minus: '-',
    Plus: 'PLUS',
    Period: '.',
    Quote: "'",
    Semicolon: ';',
    Slash: '/',
    Backspace: 'Backspace',
    CapsLock: 'Capslock',
    ContextMenu: 'Contextmenu',
    Space: 'Space',
    Tab: 'Tab',
    Convert: 'Convert',
    Delete: 'Delete',
    End: 'End',
    Help: 'Help',
    Home: 'Home',
    PageDown: 'Pagedown',
    PageUp: 'Pageup',
    Escape: 'Esc',
    PrintScreen: 'Printscreen',
    ScrollLock: 'Scrolllock',
    Pause: 'Pause',
    Insert: 'Insert',
    Suspend: 'Suspend',
};

// 把一次按键事件翻译成 "Ctrl+Shift+A" 这种写法。退格键当清空用。
function readHotkey(e) {
    if (e.keyCode === 8) return '';
    const mods = [];
    if (e.ctrlKey) mods.push('Ctrl');
    if (e.shiftKey) mods.push('Shift');
    if (e.metaKey) mods.push(osType === 'Darwin' ? 'Command' : 'Super');
    if (e.altKey) mods.push('Alt');

    let code = e.code;
    if (code.startsWith('Key')) code = code.substring(3);
    else if (code.startsWith('Digit')) code = code.substring(5);
    else if (code.startsWith('Numpad')) code = 'Num' + code.substring(6);
    else if (code.startsWith('Arrow')) code = code.substring(5);
    else if (code.startsWith('Intl')) code = code.substring(4);
    else if (!/^F\d+$/.test(code)) code = keyMap[code] ?? '';

    return [...mods, code].filter(Boolean).join('+');
}
const MODIFIERS = ['Ctrl', 'Shift', 'Super', 'Command', 'Alt'];

// 三个弹出位置的选项，第一项是默认值。config.json 里是缺省或不认识的值时，下拉
// 显示成默认值 —— 和 pop_button.rs 里 match 的 `_` 分支落到同一个值。
const POP_BUTTON_POS = ['bottom_left', 'bottom_right', 'top_right', 'top_left'];
const POP_RESULT_POS = ['bottom_right', 'bottom_left', 'top_right', 'top_left'];
const SCREENSHOT_POS = [
    'box_bottom_left',
    'box_right_top',
    'box_bottom_right',
    'cursor_bottom_right',
    'cursor_bottom_left',
    'cursor_top_right',
    'cursor_top_left',
];
const known = (list, v) => (list.includes(v) ? v : list[0]);

export default function Translate() {
    const [sourceLanguage, setSourceLanguage] = useConfig('translate_source_language', 'auto');
    const [targetLanguage, setTargetLanguage] = useConfig('translate_target_language', 'zh_cn');
    const [detectEngine, setDetectEngine] = useConfig('translate_detect_engine', 'local');
    const [popEnable, setPopEnable] = useConfig('pop_button_enable', false);
    const [popTrigger, setPopTrigger] = useConfig('pop_button_trigger', 'hover');
    const [popExcludeNative, setPopExcludeNative] = useConfig('pop_button_exclude_native', true);
    const [popBlacklist, setPopBlacklist] = useConfig('pop_button_blacklist', '');
    const [popButtonPos, setPopButtonPos] = useConfig('pop_button_pos', POP_BUTTON_POS[0]);
    const [popResultPos, setPopResultPos] = useConfig('pop_result_pos', POP_RESULT_POS[0]);
    const [screenshotPos, setScreenshotPos] = useConfig('screenshot_pos', SCREENSHOT_POS[0]);
    const [hotkey, setHotkey] = useConfig('hotkey_screenshot', '');
    const { t } = useTranslation();
    const toastStyle = useToastStyle();

    // 录制中的快捷键：null = 没在录，框里显示已存的值。recording 用 ref 不用 state：
    // 录完时先 blur，onBlur 读到的必须是「已录完」，state 在同一轮里还是旧值。
    const [draft, setDraft] = useState(null);
    const recording = useRef(false);

    const saveHotkey = async (v) => {
        setHotkey(v);
        try {
            await invoke('register_shortcut', { shortcut: v });
            if (v) toast.success(t('config.hotkey.success'), { style: toastStyle });
        } catch (e) {
            // 键被别的软件占了就是在这儿报出来的
            toast.error(String(e), { style: toastStyle });
        }
    };

    return (
        <Card
            shadow='none'
            className='border-1 border-default-200'
        >
            <CardBody>
                <div className='config-item'>
                    <h3 className='my-auto mx-0'>{t('config.translate.source_language')}</h3>
                    {sourceLanguage !== null && (
                        <Dropdown>
                            <DropdownTrigger>
                                <Button variant='bordered'>{t(`languages.${sourceLanguage}`)}</Button>
                            </DropdownTrigger>
                            <DropdownMenu
                                aria-label='source language'
                                classNames={{ list: 'max-h-[50vh] overflow-y-auto' }}
                                onAction={(key) => {
                                    setSourceLanguage(key);
                                }}
                            >
                                <DropdownItem key='auto'>{t('languages.auto')}</DropdownItem>
                                {languageList.map((item) => {
                                    return <DropdownItem key={item}>{t(`languages.${item}`)}</DropdownItem>;
                                })}
                            </DropdownMenu>
                        </Dropdown>
                    )}
                </div>
                <div className='config-item'>
                    <h3 className='my-auto mx-0'>{t('config.translate.target_language')}</h3>
                    {targetLanguage !== null && (
                        <Dropdown>
                            <DropdownTrigger>
                                <Button variant='bordered'>{t(`languages.${targetLanguage}`)}</Button>
                            </DropdownTrigger>
                            <DropdownMenu
                                aria-label='target language'
                                classNames={{ list: 'max-h-[50vh] overflow-y-auto' }}
                                onAction={(key) => {
                                    setTargetLanguage(key);
                                }}
                            >
                                {languageList.map((item) => {
                                    return <DropdownItem key={item}>{t(`languages.${item}`)}</DropdownItem>;
                                })}
                            </DropdownMenu>
                        </Dropdown>
                    )}
                </div>
                <div className='config-item'>
                    <h3 className='my-auto mx-0'>{t('config.translate.pop_button.exclude_native')}</h3>
                    {popExcludeNative !== null && (
                        <Dropdown isDisabled={!popEnable}>
                            <DropdownTrigger>
                                <Button variant='bordered'>
                                    {t(`config.translate.pop_button.${popExcludeNative ? 'on' : 'off'}`)}
                                </Button>
                            </DropdownTrigger>
                            <DropdownMenu
                                aria-label='exclude native'
                                onAction={(key) => {
                                    setPopExcludeNative(key === 'on');
                                }}
                            >
                                {['on', 'off'].map((key) => (
                                    <DropdownItem key={key}>{t(`config.translate.pop_button.${key}`)}</DropdownItem>
                                ))}
                            </DropdownMenu>
                        </Dropdown>
                    )}
                </div>
                <div className='config-item'>
                    <h3 className='my-auto mx-0'>{t('config.translate.detect_engine')}</h3>
                    {detectEngine !== null && (
                        <Dropdown>
                            <DropdownTrigger>
                                <Button variant='bordered'>{t(`config.translate.${detectEngine}`)}</Button>
                            </DropdownTrigger>
                            <DropdownMenu
                                aria-label='detect engine'
                                onAction={(key) => {
                                    setDetectEngine(key);
                                }}
                            >
                                <DropdownItem key='local'>{t(`config.translate.local`)}</DropdownItem>
                                <DropdownItem key='niutrans'>{t(`config.translate.niutrans`)}</DropdownItem>
                                <DropdownItem key='baidu'>{t(`config.translate.baidu`)}</DropdownItem>
                                <DropdownItem key='google'>{t(`config.translate.google`)}</DropdownItem>
                            </DropdownMenu>
                        </Dropdown>
                    )}
                </div>
                {/* 开关和触发方式合成一个下拉，只是界面上的合并：底层还是
                    pop_button_enable + pop_button_trigger 两个 key，Rust 和 PopButton
                    照旧各读各的。两个 setter 各有自己的 debounce，同时调互不干扰。 */}
                <div className='config-item'>
                    <h3 className='my-auto mx-0'>{t('config.translate.pop_button.enable')}</h3>
                    {popEnable !== null && popTrigger !== null && (
                        <Dropdown>
                            <DropdownTrigger>
                                <Button variant='bordered'>
                                    {t(`config.translate.pop_button.${popEnable ? popTrigger : 'off'}`)}
                                </Button>
                            </DropdownTrigger>
                            <DropdownMenu
                                aria-label='pop button'
                                onAction={(key) => {
                                    if (key === 'off') {
                                        setPopEnable(false);
                                        return;
                                    }
                                    setPopEnable(true);
                                    setPopTrigger(key);
                                }}
                            >
                                {['off', 'hover', 'click'].map((key) => (
                                    <DropdownItem key={key}>{t(`config.translate.pop_button.${key}`)}</DropdownItem>
                                ))}
                            </DropdownMenu>
                        </Dropdown>
                    )}
                </div>
                <div className='config-item'>
                    <h3 className='my-auto mx-0'>{t('config.translate.pop_button.position')}</h3>
                    {popButtonPos !== null && (
                        <Dropdown isDisabled={!popEnable}>
                            <DropdownTrigger>
                                <Button variant='bordered'>
                                    {t(`config.translate.pop_button.${known(POP_BUTTON_POS, popButtonPos)}`)}
                                </Button>
                            </DropdownTrigger>
                            <DropdownMenu
                                aria-label='pop button position'
                                onAction={(key) => {
                                    setPopButtonPos(key);
                                }}
                            >
                                {POP_BUTTON_POS.map((key) => (
                                    <DropdownItem key={key}>{t(`config.translate.pop_button.${key}`)}</DropdownItem>
                                ))}
                            </DropdownMenu>
                        </Dropdown>
                    )}
                </div>
                <div className='config-item'>
                    <h3 className='my-auto mx-0'>{t('config.translate.pop_result.position')}</h3>
                    {popResultPos !== null && (
                        <Dropdown isDisabled={!popEnable}>
                            <DropdownTrigger>
                                <Button variant='bordered'>
                                    {t(`config.translate.pop_result.${known(POP_RESULT_POS, popResultPos)}`)}
                                </Button>
                            </DropdownTrigger>
                            <DropdownMenu
                                aria-label='pop result position'
                                onAction={(key) => {
                                    setPopResultPos(key);
                                }}
                            >
                                {POP_RESULT_POS.map((key) => (
                                    <DropdownItem key={key}>{t(`config.translate.pop_result.${key}`)}</DropdownItem>
                                ))}
                            </DropdownMenu>
                        </Dropdown>
                    )}
                </div>
                <div className='config-item'>
                    <h3 className='my-auto mx-0'>{t('config.translate.screenshot.position')}</h3>
                    {screenshotPos !== null && (
                        <Dropdown>
                            <DropdownTrigger>
                                <Button variant='bordered'>
                                    {t(`config.translate.screenshot.${known(SCREENSHOT_POS, screenshotPos)}`)}
                                </Button>
                            </DropdownTrigger>
                            <DropdownMenu
                                aria-label='screenshot result position'
                                onAction={(key) => {
                                    setScreenshotPos(key);
                                }}
                            >
                                {SCREENSHOT_POS.map((key) => (
                                    <DropdownItem key={key}>{t(`config.translate.screenshot.${key}`)}</DropdownItem>
                                ))}
                            </DropdownMenu>
                        </Dropdown>
                    )}
                </div>
                <div className='config-item'>
                    <h3 className='my-auto mx-0'>{t('config.hotkey.ocr_translate')}</h3>
                    {hotkey !== null && (
                        <Input
                            aria-label={t('config.hotkey.ocr_translate')}
                            variant='bordered'
                            value={draft ?? hotkey}
                            placeholder={t(draft === null ? 'config.hotkey.none' : 'config.hotkey.recording')}
                            className='w-[130px]'
                            classNames={{ input: 'text-center' }}
                            // 一拿到焦点就先把键注销掉，否则录制时按到当前这个键会直接触发截图。
                            onFocus={() => {
                                recording.current = true;
                                setDraft('');
                                void invoke('register_shortcut', { shortcut: '' });
                            }}
                            // 没录完就离开（点别处 / Esc）：还原成原来的键。
                            onBlur={() => {
                                if (!recording.current) return;
                                recording.current = false;
                                setDraft(null);
                                void invoke('register_shortcut', { shortcut: hotkey });
                            }}
                            onKeyDown={(e) => {
                                e.preventDefault();
                                const hk = readHotkey(e);
                                const noMods = !(e.ctrlKey || e.shiftKey || e.altKey || e.metaKey);
                                if (e.key === 'Escape' && noMods) return e.target.blur();
                                // 只按着修饰键：先显示出来，等主键。退格 = 清空，算录完。
                                if (e.keyCode !== 8 && (!hk || MODIFIERS.includes(hk.split('+').pop()))) {
                                    setDraft(hk);
                                    return;
                                }
                                recording.current = false;
                                setDraft(null);
                                void saveHotkey(hk);
                                e.target.blur();
                            }}
                        />
                    )}
                </div>
                <div className='config-item'>
                    <h3 className='my-auto mx-0'>{t('config.translate.pop_button.blacklist')}</h3>
                    {popBlacklist !== null && (
                        <Input
                            aria-label={t('config.translate.pop_button.blacklist')}
                            variant='bordered'
                            value={popBlacklist}
                            placeholder='notepad, code'
                            className='w-[200px]'
                            isDisabled={!popEnable}
                            onValueChange={(v) => {
                                setPopBlacklist(v);
                            }}
                        />
                    )}
                </div>
            </CardBody>
        </Card>
    );
}

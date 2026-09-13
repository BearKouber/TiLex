// Exercise production handlers with controlled React hooks and native boundaries.
// This does not replace desktop layout, keyboard or NextUI interaction checks.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import React from 'react';
import ts from 'typescript';
import * as format from './wordbook_format.js';
import * as selection from './wordbook_selection.js';
import * as savedEntry from './saved_entry.js';

const source = readFileSync(new URL('../window/Config/pages/Wordbook/index.jsx', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        jsx: ts.JsxEmit.React,
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
        esModuleInterop: true,
    },
}).outputText;
const seed = [5, 4, 3, 2, 1].map((id) => ({
    id,
    text: `entry ${id}`,
    type: id % 2 ? 'word' : 'sentence',
    translation: `meaning ${id}`,
    detail: null,
}));
const settle = async () => {
    for (let index = 0; index < 10; index++) await Promise.resolve();
};
function deferred() {
    let resolve;
    let reject;
    const promise = new Promise((yes, no) => {
        resolve = yes;
        reject = no;
    });
    return { promise, resolve, reject };
}

async function mount() {
    const slots = [];
    let cursor = 0;
    let rows = [...seed];
    let executeWait;
    let selectWait;
    let changed;
    const calls = [];
    const errors = [];
    const effects = [];
    const hooks = {
        ...React,
        useState(initial) {
            const index = cursor++;
            if (!(index in slots)) slots[index] = initial;
            return [
                slots[index],
                (value) => {
                    slots[index] = typeof value === 'function' ? value(slots[index]) : value;
                },
            ];
        },
        useRef(value) {
            const index = cursor++;
            if (!(index in slots)) slots[index] = { current: value };
            return slots[index];
        },
        useMemo: (calculate) => calculate(),
        useCallback: (fn) => fn,
        useEffect(effect) {
            const index = cursor++;
            if (!(index in slots)) {
                slots[index] = true;
                effects.push(effect);
            }
        },
    };
    const native = {
        react: hooks,
        '@nextui-org/react': Object.fromEntries(
            [
                'Button',
                'Card',
                'CardBody',
                'Checkbox',
                'Input',
                'Listbox',
                'ListboxItem',
                'Modal',
                'ModalContent',
                'ModalFooter',
                'ModalHeader',
                'Tab',
                'Tabs',
            ].map((name) => [name, name])
        ),
        'react-icons/md': { MdDeleteOutline: 'DeleteIcon', MdDownload: 'DownloadIcon', MdVolumeUp: 'VoiceIcon' },
        'react-hot-toast': { error: (...args) => errors.push(args), success: () => {} },
        'react-i18next': {
            useTranslation: () => ({
                t: (key, values = {}) =>
                    key +
                    (values.count === undefined ? '' : `:${values.count}`) +
                    (values.text === undefined ? '' : `:${values.text}`),
            }),
        },
        '../../../../hooks': { useToastStyle: () => ({}) },
        '../../../../utils/wordbook_format': format,
        '../../../../utils/wordbook_selection': selection,
        '../../../../utils/saved_entry': savedEntry,
        '../../../../components/TranslationResult': 'TranslationResult',
        '../../../../utils/wordbook_export': { exportMarkdown: async () => null },
        '../../../../utils/speak': { speak: () => {} },
        '../../../../utils/wordbook': {
            getDB: async () => ({
                select: async () => (selectWait ? await selectWait : [...rows]),
                execute: async (sql, ids) => {
                    calls.push([sql, ids]);
                    if (executeWait) await executeWait;
                    rows = rows.filter((entry) => !ids.includes(entry.id));
                },
            }),
        },
        '@tauri-apps/api/event': {
            listen: async (_, callback) => {
                changed = callback;
                return () => {};
            },
        },
    };
    const exports = {};
    new Function('require', 'exports', compiled)((name) => {
        assert.ok(Object.hasOwn(native, name), `Unexpected dependency: ${name}`);
        return native[name];
    }, exports);
    function render() {
        cursor = 0;
        const tree = exports.default();
        for (const effect of effects.splice(0)) effect();
        return tree;
    }
    function nodes() {
        const found = [];
        const visit = (node) => {
            if (Array.isArray(node)) {
                node.forEach(visit);
                return;
            }
            if (!React.isValidElement(node)) return;
            found.push(node);
            if (node.type !== 'Modal' || node.props.isOpen) visit(node.props.children);
        };
        visit(render());
        return found;
    }
    const find = (type, predicate = () => true) => {
        const node = nodes().find((item) => item.type === type && predicate(item.props));
        assert.ok(node, `Missing ${type}`);
        return node.props;
    };
    const button = (key) => find('Button', (props) => props.children === key);
    render();
    await settle();
    return {
        nodes,
        find,
        button,
        calls,
        errors,
        detail: () => nodes().find((item) => item.type === 'h2')?.props.children ?? null,
        preview: (id) => find('Listbox').onSelectionChange(new Set([String(id)])),
        singleDelete: () => find('Button', (props) => props['aria-label'] === 'config.wordbook.delete').onPress(),
        check: (id) =>
            find(
                'Checkbox',
                (props) => props['aria-label'] === `config.wordbook.select_entry:entry ${id}`
            ).onValueChange(true),
        waitForDelete: (wait) => {
            executeWait = wait;
        },
        waitForLoad: (wait) => {
            selectWait = wait;
        },
        reload: () => changed(),
    };
}

const single = await mount();
assert.equal(single.detail(), 'entry 5', 'initial fallback remains first entry');
single.preview(3);
await single.singleDelete();
assert.equal(single.detail(), 'entry 2');
await single.singleDelete();
assert.equal(single.detail(), 'entry 1');
await single.singleDelete();
assert.equal(single.detail(), 'entry 4');

const pending = await mount();
pending.preview(3);
const wait = deferred();
pending.waitForDelete(wait.promise);
const deletion = pending.singleDelete();
await settle();
assert.equal(pending.detail(), 'entry 3', 'do not advance before database success');
await pending.singleDelete();
assert.equal(pending.calls.length, 1, 'duplicate requests are blocked before rerender too');
pending.preview(1);
wait.resolve();
await deletion;
assert.equal(pending.detail(), 'entry 1', 'a pending deletion must not steal a newer preview');

const filtered = await mount();
filtered.preview(3);
const filterWait = deferred();
filtered.waitForDelete(filterWait.promise);
const filteredDeletion = filtered.singleDelete();
filtered.find('Tabs').onSelectionChange('sentence');
assert.equal(filtered.detail(), 'entry 4');
filterWait.resolve();
await filteredDeletion;
assert.equal(filtered.detail(), 'entry 4', 'deletion respects the latest filtered fallback');

const failed = await mount();
failed.preview(3);
const failure = deferred();
failed.waitForDelete(failure.promise);
const failedDeletion = failed.singleDelete();
failure.reject(new Error('disk unavailable'));
await failedDeletion;
assert.equal(failed.detail(), 'entry 3');
assert.equal(failed.find('Listbox').children.length, 5);
assert.equal(failed.errors.length, 1);

const batch = await mount();
batch.preview(3);
batch.button('config.wordbook.multi_select').onPress();
assert.equal(batch.button('config.wordbook.delete').isDisabled, true);
assert.equal(
    batch.nodes().filter((node) => node.type === 'ListboxItem').length,
    0,
    'batch rows must not nest independent controls in listbox options'
);
batch.check(3);
batch.check(2);
assert.equal(batch.detail(), 'entry 3', 'checking does not preview');
assert.equal(
    batch.find('Checkbox', (props) => props.children === 'config.wordbook.select_all_results').isIndeterminate,
    true
);
batch.button('config.wordbook.delete').onPress();
assert.equal(batch.find('ModalHeader').children, 'config.wordbook.confirm_delete:2');
batch.button('common.cancel').onPress();
assert.equal(batch.calls.length, 0, 'cancelling never deletes');
assert.equal(
    batch.find('Checkbox', (props) => props['aria-label'] === 'config.wordbook.select_entry:entry 3').isSelected,
    true
);
batch.button('config.wordbook.delete').onPress();
await batch.button('config.wordbook.delete_count:2').onPress();
assert.deepEqual(batch.calls, [['UPDATE entries SET deleted=1 WHERE id IN ($1, $2)', [3, 2]]]);
assert.equal(batch.detail(), 'entry 1');
assert.ok(batch.button('config.wordbook.multi_select'), 'successful batch exits multi-select');

const controls = await mount();
controls.button('config.wordbook.multi_select').onPress();
controls.check(3);
const preview = controls.find('button', (props) => props.children[0].props.children === 'entry 2');
preview.onClick();
assert.equal(controls.detail(), 'entry 2');
assert.equal(
    controls.find('Checkbox', (props) => props['aria-label'] === 'config.wordbook.select_entry:entry 3').isSelected,
    true
);
controls.find('Input').onValueChange('entry 3');
assert.equal(
    controls.button('config.wordbook.delete').isDisabled,
    true,
    'search clears checked IDs without leaving mode'
);
controls.find('Checkbox', (props) => props.children === 'config.wordbook.select_all_results').onValueChange(true);
controls.button('config.wordbook.delete').onPress();
await controls.button('config.wordbook.delete_count:1').onPress();
assert.deepEqual(controls.calls[0][1], [3], 'select all is restricted to the current results');
assert.equal(controls.detail(), null);
controls.find('Input').onValueChange('');
assert.equal(controls.find('Listbox').children.length, 4);

const retry = await mount();
retry.button('config.wordbook.multi_select').onPress();
retry.check(3);
retry.check(1);
retry.button('config.wordbook.delete').onPress();
const rejected = deferred();
retry.waitForDelete(rejected.promise);
const trying = retry.button('config.wordbook.delete_count:2').onPress();
let escapeStopped = false;
const escape = { key: 'Escape', stopPropagation: () => (escapeStopped = true) };
retry.find('ModalContent').onKeyDownCapture(escape);
assert.equal(escapeStopped, true, 'pending Escape cannot reach the app-level close-window shortcut');
rejected.reject(new Error('database rejected batch'));
await trying;
assert.equal(retry.errors.length, 1);
assert.equal(retry.find('Modal').isOpen, true);
escapeStopped = false;
retry.find('ModalContent').onKeyDownCapture(escape);
assert.equal(escapeStopped, false, 'after a failed delete, Escape still reaches normal modal cancellation');
retry.button('common.cancel').onPress();
assert.equal(
    retry.find('Checkbox', (props) => props['aria-label'] === 'config.wordbook.select_entry:entry 3').isSelected,
    true
);
assert.equal(retry.nodes().filter((node) => node.type === 'li').length, 5);
retry.waitForDelete(null);
retry.button('config.wordbook.delete').onPress();
await retry.button('config.wordbook.delete_count:2').onPress();
assert.equal(retry.detail(), 'entry 5', 'deleting other entries preserves current preview');

const stale = await mount();
stale.preview(3);
const oldLoad = deferred();
stale.waitForLoad(oldLoad.promise);
stale.reload();
await settle();
stale.waitForLoad(null);
await stale.singleDelete();
oldLoad.resolve(seed);
await settle();
assert.equal(stale.detail(), 'entry 2');
assert.equal(stale.find('Listbox').children.length, 4, 'an older load cannot resurrect deleted entries');

const membership = await mount();
membership.preview(3);
membership.button('config.wordbook.multi_select').onPress();
membership.check(3);
membership.check(1);
await membership.singleDelete();
assert.equal(membership.detail(), 'entry 2');
assert.equal(membership.nodes().filter((node) => node.type === 'li').length, 4);
assert.equal(
    membership.find('span', (props) => props['aria-live'] === 'polite').children,
    'config.wordbook.selected_count:1'
);
membership.find('Tabs').onSelectionChange('sentence');
assert.equal(membership.button('config.wordbook.delete').isDisabled, true, 'changing category clears checks');
membership.find('Checkbox', (props) => props.children === 'config.wordbook.select_all_results').onValueChange(true);
assert.equal(
    membership.find('Checkbox', (props) => props.children === 'config.wordbook.select_all_results').isSelected,
    true
);
membership.button('config.wordbook.done').onPress();
membership.button('config.wordbook.multi_select').onPress();
assert.equal(membership.button('config.wordbook.delete').isDisabled, true, 'Done clears checked IDs');
membership.find('Input').onValueChange('no matching entry');
assert.equal(
    membership.find('Checkbox', (props) => props.children === 'config.wordbook.select_all_results').isDisabled,
    true
);

const frozen = await mount();
frozen.button('config.wordbook.multi_select').onPress();
frozen.find('Checkbox', (props) => props.children === 'config.wordbook.select_all_results').onValueChange(true);
frozen.button('config.wordbook.delete').onPress();
frozen.waitForLoad(Promise.resolve([{ ...seed[0], id: 6, text: 'new entry' }, ...seed]));
frozen.reload();
await settle();
assert.equal(
    frozen.find('ModalHeader').children,
    'config.wordbook.confirm_delete:5',
    'new entries cannot expand a confirmed deletion set'
);
await frozen.button('config.wordbook.delete_count:5').onPress();
assert.deepEqual(frozen.calls[0][1], [5, 4, 3, 2, 1]);
assert.equal(frozen.detail(), 'new entry');

const overlapping = await mount();
overlapping.preview(3);
const overlapWait = deferred();
overlapping.waitForDelete(overlapWait.promise);
const overlappingDelete = overlapping.singleDelete();
await settle();
overlapping.waitForLoad(
    Promise.resolve([{ ...seed[0], id: 6, text: 'arrived during delete' }, ...seed.filter((entry) => entry.id !== 3)])
);
overlapping.reload();
await settle();
assert.equal(overlapping.detail(), 'entry 3', 'refresh cannot remove the preview before deletion settles');
overlapWait.resolve();
await overlappingDelete;
await settle();
assert.equal(overlapping.detail(), 'entry 2', 'a post-commit reload cannot erase the next-item anchor');
assert.equal(
    overlapping.find('Listbox').children[0].props.textValue,
    'arrived during delete',
    'deferred refresh must preserve unrelated new entries'
);

const late = await mount();
late.preview(3);
const lateLoad = deferred();
late.waitForLoad(lateLoad.promise);
late.reload();
await settle();
const latestRows = [{ ...seed[0], id: 6, text: 'arrived before delete' }, ...seed.filter((entry) => entry.id !== 3)];
late.waitForLoad(Promise.resolve(latestRows));
await late.singleDelete();
await settle();
assert.equal(
    late.find('Listbox').children[0].props.textValue,
    'arrived before delete',
    'invalidated in-flight refresh must be replayed'
);
lateLoad.resolve(seed);
await settle();
assert.equal(late.detail(), 'entry 2');
assert.equal(late.find('Listbox').children[0].props.textValue, 'arrived before delete');

console.log(
    'wordbook_interactions: production single/batch handlers, pending selection/filter changes, cancellation, failure/retry and stale loads passed'
);

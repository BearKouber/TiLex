import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
    normalizeAiConfig,
    effectiveAiConfig,
    buildAiRequest,
    DEFAULT_CUSTOM_INSTRUCTIONS,
    DEFAULT_REQUEST_ARGUMENTS,
    LEGACY_DEFAULT_PROMPTS,
    INTERNAL_PROMPT,
} from './instructions.js';
import { FORMATS } from './protocol.js';

assert.equal(
    DEFAULT_CUSTOM_INSTRUCTIONS,
    '使用所选目标语言，准确、自然、简洁地表达原意。结合上下文理解多义词，专业内容优先使用通行术语。单词和短语突出常用释义与搭配；句子保留原意和语气。必要时提供简短例句或解释，避免无关扩展。'
);
assert.equal(normalizeAiConfig({}).customInstructions, DEFAULT_CUSTOM_INSTRUCTIONS);
assert.equal(normalizeAiConfig({ customInstructions: '' }).customInstructions, '');
assert.equal(normalizeAiConfig({ promptList: [] }).customInstructions, '');
assert.equal(normalizeAiConfig({ promptList: LEGACY_DEFAULT_PROMPTS }).customInstructions, DEFAULT_CUSTOM_INSTRUCTIONS);
assert.equal(normalizeAiConfig({ promptList: null }).customInstructions, '');
assert.equal(
    normalizeAiConfig({ promptList: 'Preserve malformed legacy text.' }).customInstructions,
    'Preserve malformed legacy text.'
);
const old = {
    model: 'model',
    apiKey: 'fixture-key',
    requestPath: 'https://fixture.invalid',
    promptList: [
        LEGACY_DEFAULT_PROMPTS[0],
        { role: 'system', content: 'Use $to for $text.' },
        { role: 'assistant', content: 'Examples with $& and "quotes".' },
        LEGACY_DEFAULT_PROMPTS[1],
    ],
    requestArguments: JSON.stringify({
        temperature: 0.4,
        max_tokens: 1000,
        model: 'wrong',
        messages: [],
        stream: true,
    }),
};
const oldCopy = structuredClone(old);
const migrated = normalizeAiConfig(old);
assert.deepEqual(old, oldCopy, 'Opening a form or translating does not mutate stored settings');
assert.equal(migrated.customInstructions, 'Use $to for $text.\n\nExamples with $& and "quotes".');
assert.deepEqual(migrated.legacyPromptBackup, { promptList: old.promptList, requestArguments: old.requestArguments });
assert.equal('apiKey' in migrated.legacyPromptBackup, false);
assert.equal('promptList' in migrated, false);
assert.deepEqual(migrated.requestArguments, { temperature: 0.4, max_tokens: 1000 });
assert.ok(migrated.legacyReferenceInstructions.includes('$text'));
assert.deepEqual(normalizeAiConfig(migrated), migrated);
assert.deepEqual(normalizeAiConfig({}).legacyPromptBackup, undefined);
assert.deepEqual(normalizeAiConfig(normalizeAiConfig({})), normalizeAiConfig({}));
const edited = normalizeAiConfig({ ...migrated, customInstructions: '', legacyReferenceInstructions: '' });
assert.equal(edited.customInstructions, '');
assert.deepEqual(edited.legacyPromptBackup, migrated.legacyPromptBackup);
const firstBackup = { promptList: [{ role: 'user', content: 'original' }] };
assert.deepEqual(normalizeAiConfig({ ...old, legacyPromptBackup: firstBackup }).legacyPromptBackup, firstBackup);
assert.equal(normalizeAiConfig({ ...old, customInstructions: 'New $text' }).legacyReferenceInstructions, '');
const defaultLookingCustomPrompts = [
    LEGACY_DEFAULT_PROMPTS[0],
    { role: 'assistant', content: LEGACY_DEFAULT_PROMPTS[0].content },
    { role: 'system', content: LEGACY_DEFAULT_PROMPTS[1].content },
    LEGACY_DEFAULT_PROMPTS[1].content,
    LEGACY_DEFAULT_PROMPTS[1],
];
const preservedCustomPrompts = normalizeAiConfig({ promptList: defaultLookingCustomPrompts });
assert.equal(
    preservedCustomPrompts.customInstructions,
    [LEGACY_DEFAULT_PROMPTS[0].content, LEGACY_DEFAULT_PROMPTS[1].content, LEGACY_DEFAULT_PROMPTS[1].content].join(
        '\n\n'
    ),
    'Default-looking content with a custom role or no role is still a custom instruction'
);
assert.deepEqual(preservedCustomPrompts.legacyPromptBackup.promptList, defaultLookingCustomPrompts);
assert.deepEqual(normalizeAiConfig(preservedCustomPrompts), preservedCustomPrompts);
for (const customInstructions of ['New instructions', '']) {
    const mixed = normalizeAiConfig({ ...old, instructionsVersion: 1, customInstructions });
    assert.equal(mixed.customInstructions, customInstructions);
    assert.deepEqual(mixed.legacyPromptBackup, { promptList: old.promptList, requestArguments: old.requestArguments });
    assert.equal('promptList' in mixed, false);
    assert.deepEqual(normalizeAiConfig(mixed), mixed);
}
for (const requestArguments of ['{"temperature":0.4}', 'not JSON', null, { temperature: 0.4, messages: ['old'] }]) {
    const mixed = normalizeAiConfig({ instructionsVersion: 1, customInstructions: '', requestArguments });
    assert.deepEqual(mixed.legacyPromptBackup, { requestArguments });
    assert.deepEqual(normalizeAiConfig(mixed), mixed);
}
assert.equal(
    normalizeAiConfig({ instructionsVersion: 1, customInstructions: '', requestArguments: { temperature: 0.4 } })
        .legacyPromptBackup,
    undefined,
    'Already normalized compatible parameters do not create a migration backup'
);
for (const requestArguments of ['not JSON', 'null', '[]', '5', null, [], 3]) {
    const normalized = normalizeAiConfig({ requestArguments });
    assert.equal(normalized.legacyArgumentsInvalid, true);
    assert.deepEqual(normalized.requestArguments, DEFAULT_REQUEST_ARGUMENTS);
    assert.deepEqual(normalized.legacyPromptBackup.requestArguments, requestArguments);
    assert.deepEqual(normalizeAiConfig(normalized), normalized);
}
assert.equal(normalizeAiConfig({ requestArguments: '{}' }).legacyArgumentsInvalid, false);
assert.deepEqual(normalizeAiConfig({ requestArguments: '{}' }).requestArguments, {});

const effective = effectiveAiConfig(migrated);
assert.deepEqual(effectiveAiConfig(effective), effective);
assert.equal(effective.internalPrompt, INTERNAL_PROMPT);
assert.equal(effective.resultSchemaVersion, 1);
assert.ok(effective.legacyReferenceInstructions);
for (const key of ['legacyPromptBackup', 'legacyArgumentsInvalid', 'instanceName', 'icon', 'iconLocked', 'enable'])
    assert.equal(key in effective, false);
assert.deepEqual(effectiveAiConfig({ ...migrated, icon: 'a', instanceName: 'b', enable: false }), effective);

// Inspect the actual protocol payload after JSON serialization. Neither source
// text nor instructions are evaluated as replacement strings in any format.
const special = '$& $text $from $to $detect "quoted" \\ slash\n```js\nconst a = "$to";\n```';
const conflicting = {
    model: 'bad',
    stream: true,
    messages: ['bad'],
    input: 'bad',
    instructions: 'bad',
    system: 'bad',
    systemInstruction: {},
    contents: [],
    generationConfig: {},
    response_format: {},
    text: {},
    tools: [],
    tool_choice: 'required',
    stop: ['}'],
    previous_response_id: 'old',
    temperature: 0.4,
    max_tokens: 1500,
};
for (const apiFormat of Object.keys(FORMATS)) {
    const config = { ...old, apiFormat, customInstructions: special, requestArguments: conflicting };
    for (const source of ['word', 'two words', 'A complete sentence.', special]) {
        const request = buildAiRequest(source, 'auto', '中文', config, 'English');
        const body = JSON.parse(JSON.stringify(request.body));
        const messages =
            apiFormat === 'google'
                ? body.contents.map((m) => ({ content: m.parts[0].text }))
                : body.messages ?? body.input;
        assert.equal(messages.at(-1).content, source);
        const metadata = JSON.parse(messages.at(-2).content);
        assert.equal(metadata.customInstructions, special);
        assert.equal(metadata.targetLanguage, '中文');
        assert.equal(metadata.detectedLanguage, 'English');
        assert.equal(metadata.kind, request.kind);
        assert.equal(request.kind, ['word', 'two words'].includes(source) ? 'word' : 'sentence');
        if (apiFormat !== 'google') {
            assert.equal(body.model, 'model');
            assert.equal(body.stream, false);
        } else assert.ok(request.url.endsWith('/models/model:generateContent'));
        for (const key of ['response_format', 'tool_choice', 'previous_response_id', 'stop'])
            assert.equal(key in body, false);
    }
    const frozen = effectiveAiConfig(config);
    assert.deepEqual(effectiveAiConfig(frozen), frozen);
    const empty = buildAiRequest('word', 'auto', '中文', { ...config, customInstructions: '' });
    assert.ok(JSON.stringify(empty.body).includes('translation engine'));
}
const anthropic = effectiveAiConfig({ ...old, apiFormat: 'anthropic', requestArguments: '{}' });
assert.deepEqual(anthropic.requestArguments, { max_tokens: 4096 });
assert.deepEqual(
    effectiveAiConfig({ ...old, apiFormat: 'google', requestArguments: '{"frequency_penalty":1}' }).requestArguments,
    {}
);

// Product guards: a local draft can be cancelled; only explicit connection tests
// call translate. Saving awaits the parent's single configuration/list commit.
const configSource = readFileSync(new URL('./Config.jsx', import.meta.url), 'utf8');
assert.match(configSource, /\{ sync: false \}/);
assert.match(configSource, /value=\{aiConfig.customInstructions\}/);
assert.doesNotMatch(configSource, /Prompt List|Request Arguments|promptList\.map|setAiConfig\(config, true\)/);
const submit = configSource.slice(configSource.indexOf('onSubmit='), configSource.indexOf('<IconPickerModal'));
assert.match(submit, /await updateServiceList\(instanceKey, normalizeAiConfig\(aiConfig\)\)/);
assert.ok(submit.indexOf('await updateServiceList') < submit.indexOf('onClose()'));
assert.doesNotMatch(submit, /translate\(/);
console.log('AI instructions migration and four-protocol request tests passed');

import { fetch, Body } from '@tauri-apps/api/http';
import { dictionaryResult } from './dictionary';
import { Language } from './info';
import { effectiveAiConfig, buildAiRequest } from './instructions';
import { formatOf } from './protocol';

export async function translate(text, from, to, options = {}) {
    const { config, detect } = options;
    const effective = effectiveAiConfig(config);
    const request = buildAiRequest(text, from, to, effective, Language[detect] ?? detect ?? '');
    const res = await fetch(request.url, {
        method: 'POST',
        headers: request.headers,
        body: Body.json(request.body),
    });
    if (!res.ok) throw `Http Request Error\nHttp Status: ${res.status}\n${JSON.stringify(res.data)}`;
    const target = formatOf(effective.apiFormat).text(res.data);
    if (typeof target !== 'string' || !target.trim()) throw JSON.stringify(res.data);
    // Failed validation preserves the response verbatim for display/copy/save.
    return dictionaryResult(target, request.kind);
}

export * from './Config';
export * from './info';

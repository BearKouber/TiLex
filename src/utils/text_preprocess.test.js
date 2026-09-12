// Run: node src/utils/text_preprocess.test.js
import assert from 'node:assert/strict';
import { preprocess } from './text_preprocess.js';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { cacheKey, getCached, setCached } from './translate_cache.js';

// Execute the actual PopResult source expression, with its real preprocessing
// function. A reintroduced store read must not silently change this path.
const source = ts.createSourceFile('PopResult.jsx', readFileSync(new URL('../window/PopResult/index.jsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.JSX);
let expression;
const visit = (node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'text' && node.initializer?.getText(source).includes('preprocess(')) {
        expression = node.initializer.getText(source);
    }
    ts.forEachChild(node, visit);
};
visit(source);
assert.ok(expression, 'PopResult must prepare source text before display/request/save');
const prepareSource = new Function('preprocess', 'raw', `return (${expression});`);
for (const flags of [undefined, { deleteNewline: false, codeSplit: false }, { deleteNewline: true, codeSplit: true }]) {
    for (const body of ['10 - 5 = 5', 'inter-\nnational', 'parseUserData', 'user_id', '// comment', '# heading', 'first\n\nsecond', '$text $to $&']) {
        const input = `  ${body}  `;
        assert.equal(preprocess(input, flags), body);
        assert.equal(prepareSource((raw) => preprocess(raw, flags), input), body);
    }
}

const k = cacheKey('hi', 'en', 'zh_cn', 'bing');
assert.equal(getCached(k), undefined);
setCached(k, '你好');
assert.equal(getCached(k), '你好');
// different service must not collide
assert.equal(getCached(cacheKey('hi', 'en', 'zh_cn', 'google')), undefined);

console.log('ok');

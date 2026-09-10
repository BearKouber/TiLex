// Run: node src/utils/text_preprocess.test.js
import assert from 'node:assert/strict';
import { preprocess, splitIdentifiers, stripComments, joinWrappedLines } from './text_preprocess.js';
import { cacheKey, getCached, setCached } from './translate_cache.js';

assert.equal(splitIdentifiers('parseUserData'), 'parse User Data');
assert.equal(splitIdentifiers('user_id'), 'user id');
assert.equal(splitIdentifiers('parseHTMLData'), 'parse HTML Data');
assert.equal(splitIdentifiers('__init__'), 'init');
// prose and plain words survive untouched
assert.equal(splitIdentifiers('The quick brown Fox'), 'The quick brown Fox');

assert.equal(stripComments('// hello\n# world\n * doc'), 'hello\nworld\ndoc');
assert.equal(joinWrappedLines('inter-\nnational'), 'international');

assert.equal(preprocess('  spaced  ', {}), 'spaced');
assert.equal(preprocess('// getUserName\n', { codeSplit: true }), 'get User Name');
assert.equal(preprocess('inter-\nnational text', { deleteNewline: true }), 'international text');

const k = cacheKey('hi', 'en', 'zh_cn', 'bing');
assert.equal(getCached(k), undefined);
setCached(k, '你好');
assert.equal(getCached(k), '你好');
// different service must not collide
assert.equal(getCached(cacheKey('hi', 'en', 'zh_cn', 'google')), undefined);

console.log('ok');

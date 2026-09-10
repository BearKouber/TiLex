// Cleanup applied to selected / OCR'd text before it reaches a translate service.

// Leading comment tokens on a line: // /* */ * # --
const COMMENT_PREFIX = /^\s*(?:\/\/+|\/\*+|\*+\/?|#+|--)\s?/;
// Only words that look like identifiers get split, so prose stays untouched.
const IDENTIFIER = /[A-Za-z_][A-Za-z0-9_]*/g;

export function stripComments(text) {
    return text
        .split('\n')
        .map((line) => line.replace(COMMENT_PREFIX, ''))
        .join('\n');
}

// parseUserData -> parse User Data, user_id -> user id, parseHTMLData -> parse HTML Data
export function splitIdentifiers(text) {
    return text.replace(IDENTIFIER, (word) => {
        if (!word.includes('_') && !/[a-z0-9][A-Z]/.test(word)) return word;
        return word
            .replace(/_+/g, ' ')
            .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
            .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
            .trim();
    });
}

// Merge wrapped lines, rejoin hyphenated line-ends: "inter-\nnational" -> "international"
export function joinWrappedLines(text) {
    return text.replace(/-\s+/g, '').replace(/\s+/g, ' ');
}

export function preprocess(text, { deleteNewline = false, codeSplit = false } = {}) {
    let out = text;
    if (codeSplit) out = splitIdentifiers(stripComments(out));
    if (deleteNewline) out = joinWrappedLines(out);
    return out.trim();
}

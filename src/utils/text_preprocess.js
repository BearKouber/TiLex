// Retired newline/code settings cannot alter source text, requests or saved notes.
export function preprocess(text) {
    return text.trim();
}

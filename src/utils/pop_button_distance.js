// Physical pixels; keep this contract aligned with native icon placement.
export const DEFAULT_POP_BUTTON_DISTANCE = 10;
export const MIN_POP_BUTTON_DISTANCE = 0;
export const MAX_POP_BUTTON_DISTANCE = 20;

export function normalizePopButtonDistance(value) {
    if (!Number.isInteger(value)) return DEFAULT_POP_BUTTON_DISTANCE;
    return Math.min(MAX_POP_BUTTON_DISTANCE, Math.max(MIN_POP_BUTTON_DISTANCE, value));
}

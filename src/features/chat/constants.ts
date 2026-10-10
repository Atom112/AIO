// Chat domain shared constants - used by both ChatPage and its event listener hook (avoids a circular import between component and hook).

/** Placeholder tool-call result text for interrupted/cancelled calls (marked for the model so the context stays understandable) */
export const TOOL_INTERRUPTED = '[Interrupted by user]';

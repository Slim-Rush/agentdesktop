const LOCAL_PATH =
  /(?:\/(?:Users|home|private|var|tmp|opt|etc)\/[^\s"'`),;:]+|[A-Za-z]:\\[^\s"'`),;:]+)/g;

/**
 * Replace local filesystem paths in error text before it reaches the UI.
 * Daemon errors can carry absolute paths; the interface shows a stable
 * message instead of the machine's layout.
 */
export function redactPaths(text: string): string {
  return text.replace(LOCAL_PATH, "<local path>");
}

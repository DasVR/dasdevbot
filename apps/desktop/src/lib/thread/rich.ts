export interface RichPart {
  code: boolean;
  text: string;
}

/** Split inline `code` out of a reply. The rest stays text, never HTML. */
export function richParts(source: string): RichPart[] {
  const parts: RichPart[] = [];
  const pattern = /`([^`]+)`/g;
  let cursor = 0;
  for (const match of source.matchAll(pattern)) {
    const index = match.index ?? 0;
    if (index > cursor) {
      parts.push({ code: false, text: source.slice(cursor, index) });
    }
    parts.push({ code: true, text: match[1] ?? "" });
    cursor = index + match[0].length;
  }
  if (cursor < source.length) {
    parts.push({ code: false, text: source.slice(cursor) });
  }
  if (parts.length === 0) {
    parts.push({ code: false, text: source });
  }
  return parts;
}

/** Word-ish tokens, keeping a trailing backtick span with its punctuation. */
export function tokenize(source: string): string[] {
  const out: string[] = [];
  const pattern = /`[^`]+`[^\s`]*\s*|[^\s`]+\s*/g;
  for (const match of source.matchAll(pattern)) {
    out.push(match[0]);
  }
  if (out.length === 0 && source.length > 0) {
    out.push(source);
  }
  return out;
}

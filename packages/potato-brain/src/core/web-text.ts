/**
 * What `web_fetch` shows the model: a page's title and its words, as plain text. The app fetches the bytes inside the
 * sandbox and sends them as text; this file only reads them. It does not run scripts, and it is not a full HTML parser:
 * it drops the parts that are never readable text (scripts, styles, the head) and turns block tags into line breaks.
 * Nothing here logs.
 */

export interface PageText {
  title: string;
  text: string;
  /** set when the address is not a text page at all (an image, a file to download) */
  note?: string;
}

const TEXT_TYPE = /^(text\/|application\/(json|xml|xhtml\+xml|ld\+json)|[^;]*\+(xml|json)$)/i;
const HTML_TYPE = /html/i;
const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };

function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, body: string) => {
    if (body.startsWith("#")) {
      const code = body[1]?.toLowerCase() === "x" ? parseInt(body.slice(2), 16) : parseInt(body.slice(1), 10);
      return Number.isFinite(code) && code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : whole;
    }
    return ENTITIES[body.toLowerCase()] ?? whole;
  });
}

/** Trims each line, collapses runs of spaces and keeps at most one blank line in a row. */
function tidy(text: string): string {
  return text
    .split("\n")
    .map((line) => line.replace(/[ \t\f\v ]+/g, " ").trim())
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

/** Turns the text of one page into its title and its readable words. `contentType` is the header value (may be empty). */
export function pageText(body: string, contentType: string): PageText {
  const type = contentType.trim().toLowerCase();
  if (type !== "" && !HTML_TYPE.test(type) && !TEXT_TYPE.test(type)) {
    return { title: "", text: "", note: `This address is not a readable text page (${type.split(";")[0]}).` };
  }
  const html = HTML_TYPE.test(type) || (type === "" && /<html|<body|<!doctype html/i.test(body));
  if (!html) return { title: "", text: tidy(decodeEntities(body)) };

  const titleMatch = /<title\b[^>]*>([\s\S]*?)<\/title\s*>/i.exec(body);
  const title = titleMatch ? tidy(decodeEntities((titleMatch[1] ?? "").replace(/<[^>]*>/g, " "))) : "";
  const words = body
    .replace(/<!--[\s\S]*?-->/g, " ")
    .replace(/<(script|style|noscript|template|svg|head|title)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, " ")
    .replace(/<\/?(p|div|br|li|tr|h[1-6]|section|article|header|footer|nav|main|blockquote|pre|table|ul|ol|dt|dd|hr)\b[^>]*>/gi, "\n")
    .replace(/<[^>]*>/g, " ");
  return { title, text: tidy(decodeEntities(words)) };
}

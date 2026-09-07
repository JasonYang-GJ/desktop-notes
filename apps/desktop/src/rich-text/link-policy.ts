export function isApprovedExternalLink(value: string): boolean {
  const href = value.trim();
  const hasWhitespaceOrControl = Array.from(href).some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x20 || codePoint === 0x7f;
  });
  if (href.length === 0 || href.length > 2048 || hasWhitespaceOrControl) return false;
  try {
    const parsed = new URL(href);
    if (parsed.protocol === "http:" || parsed.protocol === "https:") return parsed.hostname.length > 0;
    return parsed.protocol === "mailto:" && parsed.pathname.length > 0;
  } catch {
    return false;
  }
}

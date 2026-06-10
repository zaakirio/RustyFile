import { LanguageDescription } from '@codemirror/language'
import { languages } from '@codemirror/language-data'

/**
 * Match a CodeMirror language description by filename (extension or
 * exact name, e.g. "Dockerfile"). Parsers are lazy-loaded by
 * @codemirror/language-data via desc.load(), so only the matched
 * language is ever fetched.
 */
export function matchLanguage(filename: string): LanguageDescription | null {
  return LanguageDescription.matchFilename(languages, filename)
}

/** Uppercase language label for the status bar; plain text fallback. */
export function languageLabel(filename: string): string {
  return matchLanguage(filename)?.name.toUpperCase() ?? 'TEXT'
}

/** Languages that should soft-wrap long lines (prose-like content). */
export function shouldWrap(filename: string): boolean {
  const desc = matchLanguage(filename)
  if (!desc) return true // plain text
  return desc.name === 'Markdown'
}

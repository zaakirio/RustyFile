import type { ReactNode } from 'react'

/**
 * Markers the backend wraps around matched terms in FTS5 snippets
 * (private-use codepoints, see search_index::SNIPPET_START/END).
 */
const SNIPPET_START = '\ue000'
const SNIPPET_END = '\ue001'

/**
 * Split a marker-delimited snippet into React nodes, wrapping matched
 * terms in <mark>. Plain string splitting — no HTML is ever parsed, so
 * file content can't inject markup.
 */
function snippetNodes(snippet: string): ReactNode[] {
  const nodes: ReactNode[] = []
  let rest = snippet
  let key = 0
  while (rest.length > 0) {
    const start = rest.indexOf(SNIPPET_START)
    if (start === -1) {
      nodes.push(rest)
      break
    }
    if (start > 0) nodes.push(rest.slice(0, start))
    const end = rest.indexOf(SNIPPET_END, start + 1)
    if (end === -1) {
      // Unterminated marker: render the remainder as plain text.
      nodes.push(rest.slice(start + 1))
      break
    }
    nodes.push(
      <mark key={key++} className="bg-primary text-background px-0.5 font-bold">
        {rest.slice(start + 1, end)}
      </mark>
    )
    rest = rest.slice(end + 1)
  }
  return nodes
}

/** A content-match excerpt rendered under a search result row. */
export default function SearchSnippet({ snippet }: { snippet: string }) {
  return (
    <div className="px-4 pb-2 -mt-1 md:pl-12 border-b border-borders/50">
      <span className="font-mono text-[12px] text-muted leading-relaxed break-all">
        {snippetNodes(snippet)}
      </span>
    </div>
  )
}

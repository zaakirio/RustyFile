import { useEffect, useMemo, useState, useCallback } from 'react'
import CodeMirror from '@uiw/react-codemirror'
import { keymap, EditorView, type ViewUpdate } from '@codemirror/view'
import { indentWithTab } from '@codemirror/commands'
import type { Extension } from '@codemirror/state'
import type { LanguageSupport } from '@codemirror/language'
import { editorTheme } from '../lib/editorTheme'
import { matchLanguage, shouldWrap } from '../lib/editorLanguage'

interface CodeEditorProps {
  filename: string
  value: string
  onChange: (value: string) => void
  onCursorChange?: (pos: { line: number; col: number }) => void
}

export default function CodeEditor({
  filename,
  value,
  onChange,
  onCursorChange,
}: CodeEditorProps) {
  // Loaded language support, keyed by the filename it was loaded for so a
  // filename change immediately falls back to plain text until the new
  // parser arrives.
  const [loaded, setLoaded] = useState<{
    filename: string
    support: LanguageSupport
  } | null>(null)
  const language = loaded?.filename === filename ? loaded.support : null

  // Lazy-load the matching language parser (only the matched one is fetched)
  useEffect(() => {
    let cancelled = false
    const desc = matchLanguage(filename)
    if (!desc) return
    desc.load().then(
      (support) => {
        if (!cancelled) setLoaded({ filename, support })
      },
      () => {
        // Parser failed to load; fall back to plain text silently
      },
    )
    return () => {
      cancelled = true
    }
  }, [filename])

  const extensions = useMemo(() => {
    const exts: Extension[] = [
      // Tab indents, but CodeMirror's built-in Escape-then-Tab escape
      // hatch still lets keyboard users move focus out of the editor.
      keymap.of([indentWithTab]),
    ]
    if (language) exts.push(language)
    if (shouldWrap(filename)) exts.push(EditorView.lineWrapping)
    return exts
  }, [language, filename])

  const handleUpdate = useCallback(
    (vu: ViewUpdate) => {
      if (!onCursorChange) return
      if (vu.selectionSet || vu.docChanged || vu.focusChanged) {
        const head = vu.state.selection.main.head
        const line = vu.state.doc.lineAt(head)
        onCursorChange({ line: line.number, col: head - line.from + 1 })
      }
    },
    [onCursorChange],
  )

  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      onUpdate={handleUpdate}
      theme={editorTheme}
      extensions={extensions}
      height="100%"
      className="flex-1 overflow-hidden [&_.cm-editor]:h-full"
      basicSetup={{
        lineNumbers: true,
        highlightActiveLine: true,
        highlightActiveLineGutter: true,
        bracketMatching: true,
        closeBrackets: true,
        foldGutter: false,
        autocompletion: false,
        searchKeymap: true,
        highlightSelectionMatches: true,
      }}
    />
  )
}

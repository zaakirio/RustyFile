import { useState, useEffect, useRef, useCallback, useMemo } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { NavArrowLeft, NavArrowRight } from 'iconoir-react'
import { api } from '../api/client'
import { encodeFsPath, extractFsPath } from '../lib/paths'
import Breadcrumbs from '../components/Breadcrumbs'
import type { FileInfo } from '../lib/types'

function detectLanguage(ext: string | undefined): string {
  const map: Record<string, string> = {
    rs: 'RUST', ts: 'TYPESCRIPT', tsx: 'TSX', js: 'JAVASCRIPT', jsx: 'JSX',
    py: 'PYTHON', yaml: 'YAML', yml: 'YAML', toml: 'TOML', json: 'JSON',
    html: 'HTML', css: 'CSS', md: 'MARKDOWN', txt: 'TEXT', sh: 'SHELL',
    sql: 'SQL', xml: 'XML', go: 'GO', rb: 'RUBY',
  }
  return map[ext ?? ''] ?? 'TEXT'
}

function getExtension(filename: string): string | undefined {
  const parts = filename.split('.')
  return parts.length > 1 ? parts[parts.length - 1].toLowerCase() : undefined
}

function computeLineCol(text: string, pos: number): { line: number; col: number } {
  const before = text.slice(0, pos)
  const lines = before.split('\n')
  return { line: lines.length, col: lines[lines.length - 1].length + 1 }
}

export default function EditorPage() {
  const location = useLocation()
  const navigate = useNavigate()
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const gutterRef = useRef<HTMLDivElement>(null)

  // Extract file path from URL: /edit/path/to/file.txt -> "path/to/file.txt"
  const filePath = extractFsPath(location.pathname, '/edit/')

  const filename = filePath.split('/').pop() ?? ''
  const ext = getExtension(filename)
  const language = detectLanguage(ext)

  const [originalContent, setOriginalContent] = useState('')
  const [content, setContent] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [cursorPos, setCursorPos] = useState({ line: 1, col: 1 })

  const modified = content !== originalContent

  // Warn on browser navigation when modified
  useEffect(() => {
    if (!modified) return
    const handler = (e: BeforeUnloadEvent) => {
      e.preventDefault()
      e.returnValue = ' '
    }
    window.addEventListener('beforeunload', handler)
    return () => window.removeEventListener('beforeunload', handler)
  }, [modified])

  // Load file content on mount
  useEffect(() => {
    let cancelled = false
    const load = async () => {
      setLoading(true)
      setError(null)
      try {
        const info = await api.get<FileInfo>(`/api/fs/${encodeFsPath(filePath)}?content=true`)
        if (!cancelled) {
          const text = info.content ?? ''
          setOriginalContent(text)
          setContent(text)
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : 'Failed to load file')
        }
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    load()
    return () => { cancelled = true }
  }, [filePath])

  // Save handler
  const handleSave = useCallback(async () => {
    if (saving) return
    setSaving(true)
    try {
      await api.put(`/api/fs/${encodeFsPath(filePath)}`, content, true)
      setOriginalContent(content)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save file')
    } finally {
      setSaving(false)
    }
  }, [filePath, content, saving])

  // Close handler - navigate to parent directory
  const handleClose = useCallback(() => {
    if (modified && !window.confirm('You have unsaved changes. Discard them?')) return
    const parts = filePath.split('/')
    parts.pop()
    const parentDir = encodeFsPath(parts.join('/'))
    navigate(`/browse/${parentDir}`)
  }, [filePath, navigate, modified])

  // Keyboard shortcut: Ctrl/Cmd+S to save
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 's') {
        e.preventDefault()
        handleSave()
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [handleSave])

  // Track cursor position
  const updateCursor = useCallback(() => {
    const ta = textareaRef.current
    if (!ta) return
    const pos = ta.selectionStart
    setCursorPos(computeLineCol(ta.value, pos))
  }, [])

  // Sync gutter scroll with textarea scroll
  const handleScroll = useCallback(() => {
    const ta = textareaRef.current
    const gutter = gutterRef.current
    if (ta && gutter) {
      gutter.scrollTop = ta.scrollTop
    }
  }, [])

  // Line numbers
  const lineNumbers = useMemo(() => {
    const count = content.split('\n').length
    return Array.from({ length: count }, (_, i) => i + 1)
  }, [content])

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center font-mono text-primary text-lg tracking-widest uppercase">
        [ LOADING... ]
      </div>
    )
  }

  if (error && !content && !originalContent) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center gap-4">
        <p className="font-mono text-primary text-sm uppercase tracking-widest">
          [ ERROR ]
        </p>
        <p className="font-mono text-muted text-xs">{error}</p>
        <button
          onClick={handleClose}
          className="font-mono text-[13px] font-bold uppercase tracking-widest px-3 py-1.5 bg-transparent border border-borders text-text-main hover:border-text-main hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[4px_4px_0px_#F2F2F2] transition-all"
        >
          CLOSE
        </button>
      </div>
    )
  }

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      {/* Navigation header */}
      <header className="h-14 border-b border-borders flex items-center px-4 md:px-6 shrink-0 gap-4">
        <div className="flex items-center gap-1 shrink-0">
          <button
            onClick={() => {
              if (modified && !window.confirm('You have unsaved changes. Discard them?')) return
              navigate(-1)
            }}
            className="p-1.5 text-muted hover:text-primary transition-colors"
            title="Back"
          >
            <NavArrowLeft width={18} height={18} strokeWidth={1.8} />
          </button>
          <button
            onClick={() => navigate(1)}
            className="p-1.5 text-muted hover:text-primary transition-colors"
            title="Forward"
          >
            <NavArrowRight width={18} height={18} strokeWidth={1.8} />
          </button>
        </div>
        <Breadcrumbs
          path={filePath}
          onNavigate={(p) => {
            if (modified && !window.confirm('You have unsaved changes. Discard them?')) return
            navigate(`/browse/${encodeFsPath(p)}`)
          }}
        />
        <div className="ml-auto flex items-center gap-2 shrink-0">
          <span className="font-mono text-[11px] text-muted uppercase tracking-wider hidden md:block">
            {modified ? 'MODIFIED' : ''}
          </span>
          {error && (
            <span className="font-mono text-[11px] text-primary uppercase tracking-wider mr-2">
              {error}
            </span>
          )}
          <button
            onClick={handleSave}
            disabled={saving || !modified}
            className="font-mono text-[13px] font-bold uppercase tracking-widest px-3 py-1.5 bg-primary text-background hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[4px_4px_0px_#E45301] transition-all disabled:opacity-40 disabled:hover:translate-x-0 disabled:hover:translate-y-0 disabled:hover:shadow-none"
          >
            {saving ? 'SAVING...' : 'SAVE'}
          </button>
          <button
            onClick={handleClose}
            className="font-mono text-[13px] font-bold uppercase tracking-widest px-3 py-1.5 bg-transparent border border-borders text-text-main hover:border-text-main transition-colors"
          >
            CLOSE
          </button>
        </div>
      </header>

      {/* Editor area */}
      <div className="flex-1 flex overflow-hidden">
        {/* Line number gutter */}
        <div
          ref={gutterRef}
          className="w-12 bg-surface border-r border-borders overflow-y-auto shrink-0 select-none [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
          aria-hidden="true"
        >
          <div className="pt-4 pr-2">
            {lineNumbers.map((num) => (
              <div
                key={num}
                className="font-mono text-[13px] leading-[21px] text-muted text-right pr-1"
              >
                {num}
              </div>
            ))}
          </div>
        </div>

        {/* Textarea */}
        <textarea
          ref={textareaRef}
          value={content}
          onChange={(e) => {
            setContent(e.target.value)
            updateCursor()
          }}
          onKeyUp={updateCursor}
          onClick={updateCursor}
          onScroll={handleScroll}
          spellCheck={false}
          className="flex-1 bg-background font-mono text-[13px] leading-[21px] p-4 text-text-main resize-none outline-none overflow-auto"
        />
      </div>

      {/* Status bar */}
      <footer className="h-6 bg-surface border-t border-borders flex items-center px-4 shrink-0">
        <div className="flex items-center gap-4">
          <span className="font-mono text-[11px] text-muted uppercase tracking-wider">
            UTF-8
          </span>
          <span className="font-mono text-[11px] text-muted uppercase tracking-wider">
            {language}
          </span>
        </div>
        <div className="ml-auto">
          <span className="font-mono text-[11px] text-muted uppercase tracking-wider">
            Ln {cursorPos.line}, Col {cursorPos.col}
          </span>
        </div>
      </footer>
    </div>
  )
}

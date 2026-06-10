import { EditorView } from '@codemirror/view'
import { syntaxHighlighting } from '@codemirror/language'
import { oneDarkHighlightStyle } from '@codemirror/theme-one-dark'
import type { Extension } from '@codemirror/state'

/**
 * CodeMirror theme matching the app's dark neo-brutalist palette
 * (see tailwind.css @theme). Chrome (backgrounds, gutters, selection,
 * cursor) uses the app colors; token colors come from the proven
 * one-dark highlight style, which reads well on the near-black background.
 */
const chrome = EditorView.theme(
  {
    '&': {
      backgroundColor: 'var(--color-background)',
      color: 'var(--color-text-main)',
      fontSize: '13px',
      height: '100%',
    },
    '.cm-scroller': {
      fontFamily: 'var(--font-mono)',
      lineHeight: '21px',
    },
    '.cm-content': {
      padding: '16px 0',
      caretColor: 'var(--color-primary)',
    },
    '.cm-line': {
      padding: '0 16px',
    },
    '.cm-cursor, .cm-dropCursor': {
      borderLeftColor: 'var(--color-primary)',
    },
    '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection':
      {
        backgroundColor: 'rgba(247, 96, 21, 0.30)',
      },
    '.cm-activeLine': {
      backgroundColor: 'rgba(242, 242, 242, 0.04)',
    },
    '.cm-gutters': {
      backgroundColor: 'var(--color-surface)',
      color: 'var(--color-muted)',
      borderRight: '1px solid var(--color-borders)',
    },
    '.cm-lineNumbers .cm-gutterElement': {
      minWidth: '40px',
      padding: '0 8px 0 4px',
    },
    '.cm-activeLineGutter': {
      backgroundColor: 'var(--color-surface-high)',
      color: 'var(--color-text-main)',
    },
    '.cm-matchingBracket, &.cm-focused .cm-matchingBracket': {
      backgroundColor: 'rgba(247, 96, 21, 0.25)',
      outline: '1px solid var(--color-primary)',
    },
    '.cm-nonmatchingBracket, &.cm-focused .cm-nonmatchingBracket': {
      backgroundColor: 'rgba(228, 83, 1, 0.15)',
    },
    '.cm-searchMatch': {
      backgroundColor: 'rgba(255, 181, 153, 0.25)',
      outline: '1px solid var(--color-outline)',
    },
    '.cm-searchMatch.cm-searchMatch-selected': {
      backgroundColor: 'rgba(247, 96, 21, 0.45)',
    },
    '.cm-selectionMatch': {
      backgroundColor: 'rgba(242, 242, 242, 0.08)',
    },
    '.cm-panels': {
      backgroundColor: 'var(--color-surface)',
      color: 'var(--color-text-main)',
      fontFamily: 'var(--font-mono)',
    },
    '.cm-panels.cm-panels-top': {
      borderBottom: '1px solid var(--color-borders)',
    },
    '.cm-panels.cm-panels-bottom': {
      borderTop: '1px solid var(--color-borders)',
    },
    '.cm-panel button, .cm-panel input': {
      borderRadius: '0',
    },
    '.cm-tooltip': {
      backgroundColor: 'var(--color-surface-high)',
      border: '1px solid var(--color-borders)',
      borderRadius: '0',
    },
    '.cm-foldPlaceholder': {
      backgroundColor: 'var(--color-surface-bright)',
      border: 'none',
      color: 'var(--color-muted)',
    },
  },
  { dark: true },
)

export const editorTheme: Extension = [
  chrome,
  syntaxHighlighting(oneDarkHighlightStyle),
]

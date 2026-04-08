import { useState, memo } from 'react'
import { Trash, Download, EditPencil } from 'iconoir-react'
import type { FileEntry } from '../lib/types'
import { formatSize, formatDate } from '../lib/format'
import { getFileIcon } from '../lib/icons'
import { isTextFile } from '../lib/paths'
import { encodeFsPath } from '../lib/paths'

interface FileRowProps {
  entry: FileEntry
  onItemClick: (entry: FileEntry) => void
  onDelete: (path: string) => void
  isSelected: boolean
  selectMode: boolean
  onToggleSelect: (path: string) => void
}

export default memo(function FileRow({
  entry,
  onItemClick,
  onDelete,
  isSelected,
  selectMode,
  onToggleSelect,
}: FileRowProps) {
  const [hovered, setHovered] = useState(false)
  const Icon = getFileIcon(entry)

  const handleClick = () => {
    if (selectMode) {
      onToggleSelect(entry.path)
    } else {
      onItemClick(entry)
    }
  }

  return (
    <>
      {/* Desktop row */}
      <div
        className={`hidden md:grid grid-cols-[1fr_120px_150px_120px] items-center h-11 px-4 cursor-pointer transition-colors group relative ${
          isSelected ? 'bg-primary/10' : 'hover:bg-surface'
        }`}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        onClick={handleClick}
      >
        {/* Name */}
        <div className="flex items-center gap-3 min-w-0">
          {/* Checkbox: visible on hover or in select mode */}
          <label
            className={`shrink-0 flex items-center justify-center w-[18px] h-[18px] transition-opacity ${
              selectMode || hovered ? 'opacity-100' : 'opacity-0'
            }`}
            onClick={(e) => e.stopPropagation()}
          >
            <input
              type="checkbox"
              checked={isSelected}
              onChange={() => onToggleSelect(entry.path)}
              className="w-3.5 h-3.5 accent-[var(--color-primary)] cursor-pointer"
            />
          </label>
          <Icon
            width={18}
            height={18}
            strokeWidth={1.8}
            className={`${entry.is_dir ? 'text-primary' : 'text-muted'} shrink-0 ${
              selectMode || hovered ? '' : '-ml-[30px]'
            } transition-all`}
          />
          <span
            className={`truncate text-[14px] ${
              entry.is_dir ? 'font-bold text-text-main' : 'text-text-main'
            }`}
          >
            {entry.name}
          </span>
        </div>

        {/* Size */}
        <span className="font-mono text-[12px] text-muted uppercase tracking-wider">
          {entry.is_dir ? '--' : formatSize(entry.size)}
        </span>

        {/* Modified */}
        <span
          className="font-mono text-[12px] text-muted tracking-wider"
          title={entry.modified}
        >
          {formatDate(entry.modified)}
        </span>

        {/* Quick actions (hover, not in select mode) */}
        <div
          className={`flex items-center justify-end gap-1 transition-opacity ${
            hovered && !selectMode ? 'opacity-100' : 'opacity-0'
          }`}
        >
          {!entry.is_dir && isTextFile(entry) && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation()
                onItemClick(entry)
              }}
              className="p-1.5 text-muted hover:text-primary transition-colors"
              title="Edit"
              aria-label={`Edit ${entry.name}`}
            >
              <EditPencil width={14} height={14} strokeWidth={2} />
            </button>
          )}
          {!entry.is_dir && (
            <a
              href={`/api/fs/download/${encodeFsPath(entry.path)}`}
              onClick={(e) => e.stopPropagation()}
              className="p-1.5 text-muted hover:text-primary transition-colors"
              title="Download"
              aria-label={`Download ${entry.name}`}
            >
              <Download width={14} height={14} strokeWidth={2} />
            </a>
          )}
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onDelete(entry.path)
            }}
            className="p-1.5 text-muted hover:text-primary transition-colors"
            title="Delete"
            aria-label={`Delete ${entry.name}`}
          >
            <Trash width={14} height={14} strokeWidth={2} />
          </button>
        </div>
      </div>

      {/* Mobile row */}
      <div
        className={`md:hidden flex items-center h-12 px-4 cursor-pointer ${
          isSelected ? 'bg-primary/10' : 'active:bg-surface'
        }`}
        onClick={handleClick}
      >
        {selectMode && (
          <label className="shrink-0 mr-3" onClick={(e) => e.stopPropagation()}>
            <input
              type="checkbox"
              checked={isSelected}
              onChange={() => onToggleSelect(entry.path)}
              className="w-4 h-4 accent-[var(--color-primary)] cursor-pointer"
            />
          </label>
        )}
        <Icon
          width={20}
          height={20}
          strokeWidth={1.8}
          className={entry.is_dir ? 'text-primary shrink-0' : 'text-muted shrink-0'}
        />
        <span
          className={`ml-3 flex-1 truncate text-[14px] ${
            entry.is_dir ? 'font-bold' : ''
          }`}
        >
          {entry.name}
        </span>
        <div className="flex flex-col items-end ml-2 shrink-0">
          <span className="font-mono text-[11px] text-muted uppercase tracking-wider">
            {entry.is_dir ? 'DIR' : formatSize(entry.size)}
          </span>
          <span
            className="font-mono text-[10px] text-muted tracking-wider"
            title={entry.modified}
          >
            {formatDate(entry.modified)}
          </span>
        </div>
      </div>
    </>
  )
})

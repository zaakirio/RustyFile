import { useMemo } from 'react'
import type { DirListing, FileEntry } from '../lib/types'
import FileRow from './FileRow'

interface FileListProps {
  listing: DirListing | null
  loading: boolean
  error: string | null
  onItemClick: (entry: FileEntry) => void
  onDelete: (path: string) => void
  selected: Set<string>
  onToggleSelect: (path: string) => void
}

export default function FileList({
  listing,
  loading,
  error,
  onItemClick,
  onDelete,
  selected,
  onToggleSelect,
}: FileListProps) {
  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <span className="font-mono text-[14px] text-muted uppercase tracking-widest">
          [ LOADING... ]
        </span>
      </div>
    )
  }

  if (error) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <span className="font-mono text-[14px] text-primary uppercase tracking-widest">
          [ ERROR: {error} ]
        </span>
      </div>
    )
  }

  if (!listing || listing.items.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <span className="font-mono text-[14px] text-muted uppercase tracking-widest">
          [ EMPTY DIRECTORY ]
        </span>
      </div>
    )
  }

  // Sort: directories first, then files, both alphabetically
  const sorted = useMemo(
    () => [...listing.items].sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1
      return a.name.localeCompare(b.name)
    }),
    [listing.items]
  )

  const selectMode = selected.size > 0

  return (
    <div className="flex-1 overflow-y-auto">
      {/* Desktop column headers */}
      <div className="hidden md:grid grid-cols-[1fr_120px_150px_120px] items-center h-9 px-4 border-b border-borders">
        <span className="font-mono text-[11px] text-muted uppercase tracking-widest">
          NAME
        </span>
        <span className="font-mono text-[11px] text-muted uppercase tracking-widest">
          SIZE
        </span>
        <span className="font-mono text-[11px] text-muted uppercase tracking-widest">
          MODIFIED
        </span>
        <span />
      </div>

      {/* File rows */}
      {sorted.map((entry) => (
        <FileRow
          key={entry.path}
          entry={entry}
          onItemClick={onItemClick}
          onDelete={onDelete}
          isSelected={selected.has(entry.path)}
          selectMode={selectMode}
          onToggleSelect={onToggleSelect}
        />
      ))}

      {/* Footer stats (desktop) */}
      <div className="hidden md:flex items-center h-9 px-4 border-t border-borders mt-auto">
        <span className="font-mono text-[11px] text-muted uppercase tracking-widest">
          {listing.num_dirs} DIR{listing.num_dirs !== 1 ? 'S' : ''} / {listing.num_files} FILE{listing.num_files !== 1 ? 'S' : ''}
        </span>
      </div>
    </div>
  )
}

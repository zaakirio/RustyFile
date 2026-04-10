import { useState, useCallback, useEffect } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { Upload, FolderPlus, Refresh, Xmark, Check, NavArrowLeft, NavArrowRight, Trash, Search as SearchIcon } from 'iconoir-react'
import { api } from '../api/client'
import { useFiles } from '../hooks/useFiles'
import { useTusUpload } from '../hooks/useTusUpload'
import { useDragDrop } from '../hooks/useDragDrop'
import { extractFsPath, encodeFsPath, isTextFile } from '../lib/paths'
import type { FileEntry, SearchParams } from '../lib/types'
import { useSearch } from '../hooks/useSearch'
import FileRow from '../components/FileRow'
import Breadcrumbs from '../components/Breadcrumbs'
import FileList from '../components/FileList'
import DropZone from '../components/DropZone'
import UploadFAB from '../components/UploadFAB'
import UploadManager from '../components/UploadManager'

export default function BrowserPage() {
  const location = useLocation()
  const navigate = useNavigate()

  // Extract path from URL: /browse/path/to/dir -> "path/to/dir"
  const currentPath = extractFsPath(location.pathname, '/browse/')

  const { listing, loading, error, refresh, deleteItem, createDir } = useFiles(currentPath)
  const { items: uploadItems, addFiles, pauseUpload, resumeUpload, clearCompleted } =
    useTusUpload({ currentPath, onAllComplete: refresh })
  const { isDragging, dragHandlers, uploadFromPicker } = useDragDrop(addFiles)

  // Action error state (surfaced in UI instead of console.error)
  const [actionError, setActionError] = useState<string | null>(null)

  // Inline delete confirmation state
  const [pendingDelete, setPendingDelete] = useState<string | null>(null)

  // Inline new folder state
  const [showNewFolder, setShowNewFolder] = useState(false)
  const [newFolderName, setNewFolderName] = useState('')

  // Multi-select state
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [bulkDeleting, setBulkDeleting] = useState(false)

  // Search state
  const { results: searchResults, total: searchTotal, loading: searchLoading, error: searchError, search, clear: clearSearch, isActive: isSearchActive } = useSearch()
  const [searchMode, setSearchMode] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [searchType, setSearchType] = useState<SearchParams['type']>(undefined)

  const toggleSelect = useCallback((path: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(path)) {
        next.delete(path)
      } else {
        next.add(path)
      }
      return next
    })
  }, [])

  const selectAll = useCallback(() => {
    if (!listing) return
    setSelected(new Set(listing.items.map((i) => i.path)))
  }, [listing])

  const clearSelection = useCallback(() => {
    setSelected(new Set())
  }, [])

  const bulkDelete = useCallback(async () => {
    if (selected.size === 0) return
    setActionError(null)
    setBulkDeleting(true)
    try {
      const results = await Promise.allSettled(
        Array.from(selected).map((itemPath) =>
          api.delete(`/api/fs/${encodeFsPath(itemPath)}`)
        )
      )
      const failed = results.filter((r) => r.status === 'rejected').length
      await refresh()
      setSelected(new Set())
      if (failed > 0) {
        setActionError(`${failed} of ${selected.size} item(s) failed to delete`)
      }
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Bulk delete failed')
    } finally {
      setBulkDeleting(false)
    }
  }, [selected, refresh])

  const handleNavigate = useCallback((entry: FileEntry) => {
    const encoded = encodeFsPath(entry.path)
    if (entry.is_dir) {
      navigate(`/browse/${encoded}`)
    } else if (
      entry.mime_type?.startsWith('video/') ||
      entry.mime_type?.startsWith('audio/')
    ) {
      navigate(`/play/${encoded}`)
    } else if (isTextFile(entry)) {
      navigate(`/edit/${encoded}`)
    } else if (entry.mime_type?.startsWith('image/')) {
      navigate(`/preview/${encoded}`)
    } else {
      window.open(`/api/fs/download/${encoded}`, '_blank')
    }
  }, [navigate])

  const handleDelete = useCallback((path: string) => {
    setPendingDelete(path)
  }, [])

  const confirmDelete = useCallback(async () => {
    if (!pendingDelete) return
    setActionError(null)
    try {
      await deleteItem(pendingDelete)
      setPendingDelete(null)
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to delete item')
    }
  }, [pendingDelete, deleteItem])

  const cancelDelete = useCallback(() => {
    setPendingDelete(null)
  }, [])

  const handleCreateDir = useCallback(async () => {
    const name = newFolderName.trim()
    if (!name) return
    setActionError(null)
    try {
      const dirPath = currentPath ? `${currentPath}/${name}` : name
      await createDir(dirPath)
      setShowNewFolder(false)
      setNewFolderName('')
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to create folder')
    }
  }, [currentPath, newFolderName, createDir])

  const cancelNewFolder = useCallback(() => {
    setShowNewFolder(false)
    setNewFolderName('')
  }, [])

  const handleSearchChange = useCallback((value: string) => {
    setSearchQuery(value)
    search({ q: value, type: searchType, path: currentPath || undefined })
  }, [search, searchType, currentPath])

  const handleTypeChange = useCallback((type: SearchParams['type']) => {
    setSearchType(type)
    if (searchQuery.length >= 2) {
      search({ q: searchQuery, type, path: currentPath || undefined })
    }
  }, [search, searchQuery, currentPath])

  const handleClearSearch = useCallback(() => {
    setSearchQuery('')
    setSearchType(undefined)
    clearSearch()
    setSearchMode(false)
  }, [clearSearch])

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'f') {
        e.preventDefault()
        setSearchMode(true)
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [])

  return (
    <div className="relative flex-1 flex flex-col overflow-hidden" {...dragHandlers}>
      {/* Drop zone overlay */}
      <DropZone visible={isDragging} targetPath={currentPath} />

      {/* Upload manager */}
      <UploadManager items={uploadItems} onPause={pauseUpload} onResume={resumeUpload} onClear={clearCompleted} />

      {/* Action error banner */}
      {actionError && (
        <div className="bg-surface border-b border-borders px-4 py-2.5 flex items-center gap-3">
          <span className="font-mono text-[12px] text-primary uppercase tracking-widest font-bold flex-1">
            [ ERROR: {actionError} ]
          </span>
          <button
            onClick={() => setActionError(null)}
            className="p-1 text-muted hover:text-primary transition-colors shrink-0"
            title="Dismiss"
          >
            <Xmark width={16} height={16} strokeWidth={2} />
          </button>
        </div>
      )}

      {/* Inline delete confirmation */}
      {pendingDelete && (
        <div className="bg-surface border-b border-borders px-4 py-3 flex items-center gap-3">
          <span className="font-mono text-[12px] text-primary uppercase tracking-widest font-bold">
            DELETE {pendingDelete.split('/').pop()}?
          </span>
          <button
            onClick={confirmDelete}
            className="font-mono text-[12px] font-bold uppercase tracking-widest px-3 py-1 bg-primary text-background hover:opacity-80 transition-opacity"
          >
            YES
          </button>
          <button
            onClick={cancelDelete}
            className="font-mono text-[12px] font-bold uppercase tracking-widest px-3 py-1 border border-borders text-text-main hover:border-text-main transition-colors"
          >
            NO
          </button>
        </div>
      )}

      {/* Bulk selection bar */}
      {selected.size > 0 && (
        <div className="bg-surface border-b border-borders px-4 py-2.5 flex items-center gap-3">
          <span className="font-mono text-[12px] text-text-main uppercase tracking-widest font-bold">
            {selected.size} SELECTED
          </span>
          <button
            onClick={selectAll}
            className="font-mono text-[11px] uppercase tracking-widest px-2 py-0.5 text-muted hover:text-primary transition-colors"
          >
            ALL
          </button>
          <button
            onClick={clearSelection}
            className="font-mono text-[11px] uppercase tracking-widest px-2 py-0.5 text-muted hover:text-primary transition-colors"
          >
            NONE
          </button>
          <div className="ml-auto flex items-center gap-2">
            <button
              onClick={bulkDelete}
              disabled={bulkDeleting}
              className="flex items-center gap-1.5 font-mono text-[12px] font-bold uppercase tracking-widest px-3 py-1 bg-primary text-background hover:opacity-80 transition-opacity disabled:opacity-50"
            >
              <Trash width={13} height={13} strokeWidth={2} />
              {bulkDeleting ? 'DELETING...' : 'DELETE'}
            </button>
            <button
              onClick={clearSelection}
              className="p-1 text-muted hover:text-primary transition-colors"
              title="Cancel selection"
            >
              <Xmark width={16} height={16} strokeWidth={2} />
            </button>
          </div>
        </div>
      )}

      {/* Header with breadcrumbs + action buttons */}
      <header className="h-14 border-b border-borders flex items-center px-4 md:px-6 shrink-0 gap-4">
        {searchMode ? (
          <div className="flex-1 flex items-center gap-2 min-w-0">
            <SearchIcon width={16} height={16} strokeWidth={1.8} className="text-muted shrink-0" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => handleSearchChange(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Escape') handleClearSearch() }}
              className="flex-1 h-8 bg-background border border-borders text-text-main font-mono text-[13px] px-3 rounded-none focus:border-primary focus:outline-none transition-colors min-w-0"
              placeholder="Search files..."
              autoFocus
            />
            <select
              value={searchType || ''}
              onChange={(e) => handleTypeChange((e.target.value || undefined) as SearchParams['type'])}
              className="h-8 bg-background border border-borders text-text-main font-mono text-[11px] px-2 rounded-none focus:border-primary focus:outline-none uppercase tracking-widest"
            >
              <option value="">ALL</option>
              <option value="file">FILES</option>
              <option value="dir">FOLDERS</option>
              <option value="image">IMAGES</option>
              <option value="video">VIDEO</option>
              <option value="audio">AUDIO</option>
              <option value="document">DOCS</option>
            </select>
            <button onClick={handleClearSearch} className="p-1.5 text-muted hover:text-primary transition-colors" title="Close search">
              <Xmark width={16} height={16} strokeWidth={2} />
            </button>
          </div>
        ) : (
          <>
            <div className="flex items-center gap-1 shrink-0">
              <button
                onClick={() => navigate(-1)}
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
              path={currentPath}
              onNavigate={(p) => navigate(`/browse/${encodeFsPath(p)}`)}
            />

            <div className="ml-auto flex items-center gap-2 shrink-0">
              <button onClick={() => setSearchMode(true)} className="p-2 text-muted hover:text-primary transition-colors" title="Search (Ctrl+F)">
                <SearchIcon width={18} height={18} strokeWidth={1.8} />
              </button>
              <button
                onClick={refresh}
                className="p-2 text-muted hover:text-primary transition-colors"
                title="Refresh"
              >
                <Refresh width={18} height={18} strokeWidth={1.8} />
              </button>
              <button
                onClick={() => setShowNewFolder(true)}
                className="hidden md:flex p-2 text-muted hover:text-primary transition-colors"
                title="New folder"
              >
                <FolderPlus width={18} height={18} strokeWidth={1.8} />
              </button>
              <button
                onClick={uploadFromPicker}
                className="hidden md:flex items-center gap-2 h-9 px-4 bg-primary text-background font-mono text-[12px] font-bold uppercase tracking-widest hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[3px_3px_0px_#F2F2F2] transition-all"
              >
                <Upload width={14} height={14} strokeWidth={2} />
                UPLOAD
              </button>
            </div>
          </>
        )}
      </header>

      {/* Inline new folder input */}
      {showNewFolder && (
        <div className="border-b border-borders px-4 md:px-6 py-2 flex items-center gap-2 bg-surface">
          <span className="font-mono text-[12px] text-muted uppercase tracking-widest shrink-0">
            NAME:
          </span>
          <input
            type="text"
            value={newFolderName}
            onChange={(e) => setNewFolderName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleCreateDir()
              if (e.key === 'Escape') cancelNewFolder()
            }}
            className="flex-1 h-8 bg-background border border-borders text-text-main font-mono text-[13px] px-3 rounded-none focus:border-primary focus:outline-none transition-colors"
            placeholder="folder-name"
            autoFocus
          />
          <button
            onClick={handleCreateDir}
            className="p-1.5 text-muted hover:text-primary transition-colors"
            title="Create"
          >
            <Check width={16} height={16} strokeWidth={2} />
          </button>
          <button
            onClick={cancelNewFolder}
            className="p-1.5 text-muted hover:text-primary transition-colors"
            title="Cancel"
          >
            <Xmark width={16} height={16} strokeWidth={2} />
          </button>
        </div>
      )}

      {/* File listing */}
      {isSearchActive ? (
        <div className="flex-1 overflow-y-auto">
          {searchLoading ? (
            <div className="flex-1 flex items-center justify-center py-12">
              <span className="font-mono text-[14px] text-muted uppercase tracking-widest">[ SEARCHING... ]</span>
            </div>
          ) : searchError ? (
            <div className="flex-1 flex items-center justify-center py-12">
              <span className="font-mono text-[14px] text-primary uppercase tracking-widest">[ ERROR: {searchError} ]</span>
            </div>
          ) : searchResults.length === 0 && searchQuery.length >= 2 ? (
            <div className="flex-1 flex items-center justify-center py-12">
              <span className="font-mono text-[14px] text-muted uppercase tracking-widest">[ NO RESULTS ]</span>
            </div>
          ) : searchResults.length > 0 ? (
            <>
              <div className="hidden md:grid grid-cols-[1fr_120px_150px_120px] items-center h-9 px-4 border-b border-borders">
                <span className="font-mono text-[11px] text-muted uppercase tracking-widest">NAME</span>
                <span className="font-mono text-[11px] text-muted uppercase tracking-widest">SIZE</span>
                <span className="font-mono text-[11px] text-muted uppercase tracking-widest">MODIFIED</span>
                <span />
              </div>
              {searchResults.map((entry) => (
                <FileRow
                  key={entry.path}
                  entry={entry}
                  onItemClick={handleNavigate}
                  onDelete={handleDelete}
                  isSelected={false}
                  selectMode={false}
                  onToggleSelect={() => {}}
                  showFullPath
                />
              ))}
              <div className="hidden md:flex items-center h-9 px-4 border-t border-borders">
                <span className="font-mono text-[11px] text-muted uppercase tracking-widest">
                  {searchResults.length} OF {searchTotal} RESULT{searchTotal !== 1 ? 'S' : ''}
                </span>
              </div>
            </>
          ) : null}
        </div>
      ) : (
        <FileList
          listing={listing}
          loading={loading}
          error={error}
          onItemClick={handleNavigate}
          onDelete={handleDelete}
          selected={selected}
          onToggleSelect={toggleSelect}
        />
      )}

      {/* Mobile upload FAB */}
      <UploadFAB onClick={uploadFromPicker} />
    </div>
  )
}

import { useState, useCallback } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { Upload, FolderPlus, Refresh, Xmark, Check, NavArrowLeft, NavArrowRight, Trash } from 'iconoir-react'
import { useFiles } from '../hooks/useFiles'
import { useTusUpload } from '../hooks/useTusUpload'
import { useDragDrop } from '../hooks/useDragDrop'
import { extractFsPath, encodeFsPath, isTextFile } from '../lib/paths'
import type { FileEntry } from '../lib/types'
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

  // Inline delete confirmation state
  const [pendingDelete, setPendingDelete] = useState<string | null>(null)

  // Inline new folder state
  const [showNewFolder, setShowNewFolder] = useState(false)
  const [newFolderName, setNewFolderName] = useState('')

  // Multi-select state
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [bulkDeleting, setBulkDeleting] = useState(false)

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
    setBulkDeleting(true)
    try {
      for (const path of selected) {
        await deleteItem(path)
      }
      setSelected(new Set())
    } catch (err) {
      console.error('Bulk delete failed:', err)
    } finally {
      setBulkDeleting(false)
    }
  }, [selected, deleteItem])

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
    try {
      await deleteItem(pendingDelete)
      setPendingDelete(null)
    } catch (err) {
      console.error('Delete failed:', err)
    }
  }, [pendingDelete, deleteItem])

  const cancelDelete = useCallback(() => {
    setPendingDelete(null)
  }, [])

  const handleCreateDir = useCallback(async () => {
    const name = newFolderName.trim()
    if (!name) return
    try {
      const dirPath = currentPath ? `${currentPath}/${name}` : name
      await createDir(dirPath)
      setShowNewFolder(false)
      setNewFolderName('')
    } catch (err) {
      console.error('Create directory failed:', err)
    }
  }, [currentPath, newFolderName, createDir])

  const cancelNewFolder = useCallback(() => {
    setShowNewFolder(false)
    setNewFolderName('')
  }, [])

  return (
    <div className="relative flex-1 flex flex-col overflow-hidden" {...dragHandlers}>
      {/* Drop zone overlay */}
      <DropZone visible={isDragging} targetPath={currentPath} />

      {/* Upload manager */}
      <UploadManager items={uploadItems} onPause={pauseUpload} onResume={resumeUpload} onClear={clearCompleted} />

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
      <FileList
        listing={listing}
        loading={loading}
        error={error}
        onItemClick={handleNavigate}
        onDelete={handleDelete}
        selected={selected}
        onToggleSelect={toggleSelect}
      />

      {/* Mobile upload FAB */}
      <UploadFAB onClick={uploadFromPicker} />
    </div>
  )
}

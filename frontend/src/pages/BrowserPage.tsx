import { useState, useCallback } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { Upload, FolderPlus, Refresh, Xmark, Check } from 'iconoir-react'
import { useFiles } from '../hooks/useFiles'
import { useDragDrop } from '../hooks/useDragDrop'
import { extractFsPath, encodeFsPath, isTextFile } from '../lib/paths'
import type { FileEntry } from '../lib/types'
import Breadcrumbs from '../components/Breadcrumbs'
import FileList from '../components/FileList'
import DropZone from '../components/DropZone'
import UploadFAB from '../components/UploadFAB'

export default function BrowserPage() {
  const location = useLocation()
  const navigate = useNavigate()

  // Extract path from URL: /browse/path/to/dir -> "path/to/dir"
  const currentPath = extractFsPath(location.pathname, '/browse/')

  const { listing, loading, error, refresh, deleteItem, createDir } = useFiles(currentPath)
  const { isDragging, uploading, progress, errors, clearErrors, dragHandlers, uploadFromPicker } =
    useDragDrop(currentPath, refresh)

  // Inline delete confirmation state
  const [pendingDelete, setPendingDelete] = useState<string | null>(null)

  // Inline new folder state
  const [showNewFolder, setShowNewFolder] = useState(false)
  const [newFolderName, setNewFolderName] = useState('')

  const handleNavigate = useCallback((entry: FileEntry) => {
    if (entry.is_dir) {
      navigate(`/browse/${entry.path}`)
    } else if (
      entry.mime_type?.startsWith('video/') ||
      entry.mime_type?.startsWith('audio/')
    ) {
      navigate(`/play/${entry.path}`)
    } else if (isTextFile(entry)) {
      navigate(`/edit/${entry.path}`)
    } else {
      // Trigger download for other file types
      window.open(`/api/fs/${encodeFsPath(entry.path)}?download=true`, '_blank')
    }
  }, [navigate])

  const handleDelete = useCallback((path: string) => {
    setPendingDelete(path)
  }, [])

  const confirmDelete = useCallback(async () => {
    if (!pendingDelete) return
    await deleteItem(pendingDelete)
    setPendingDelete(null)
  }, [pendingDelete, deleteItem])

  const cancelDelete = useCallback(() => {
    setPendingDelete(null)
  }, [])

  const handleCreateDir = useCallback(async () => {
    const name = newFolderName.trim()
    if (!name) return
    const dirPath = currentPath ? `${currentPath}/${name}` : name
    await createDir(dirPath)
    setShowNewFolder(false)
    setNewFolderName('')
  }, [currentPath, newFolderName, createDir])

  const cancelNewFolder = useCallback(() => {
    setShowNewFolder(false)
    setNewFolderName('')
  }, [])

  return (
    <div className="relative flex-1 flex flex-col overflow-hidden" {...dragHandlers}>
      {/* Drop zone overlay */}
      <DropZone visible={isDragging} targetPath={currentPath} />

      {/* Upload progress indicator */}
      {uploading && (
        <div className="absolute top-0 left-0 right-0 z-40 bg-surface border-b border-borders px-4 py-3">
          <p className="font-mono text-[12px] text-primary uppercase tracking-widest font-bold mb-2">
            [ UPLOADING... ]
          </p>
          <div className="flex flex-col gap-1">
            {progress.map((p) => (
              <div
                key={p.name}
                className="font-mono text-[11px] tracking-wider flex items-center gap-2"
              >
                <span className={p.done ? 'text-primary' : 'text-muted'}>
                  {p.done ? '[OK]' : '[..]'}
                </span>
                <span className={p.done ? 'text-text-main' : 'text-muted'}>
                  {p.name}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Upload error feedback */}
      {errors.length > 0 && (
        <div className="bg-surface border-b border-borders px-4 py-3">
          <div className="flex items-center justify-between mb-1">
            <p className="font-mono text-[12px] text-primary uppercase tracking-widest font-bold">
              [ UPLOAD FAILED ]
            </p>
            <button
              onClick={clearErrors}
              className="p-1 text-muted hover:text-primary transition-colors"
              title="Dismiss"
            >
              <Xmark width={14} height={14} strokeWidth={2} />
            </button>
          </div>
          <div className="flex flex-col gap-0.5">
            {errors.map((name) => (
              <span key={name} className="font-mono text-[11px] text-muted tracking-wider">
                {name}
              </span>
            ))}
          </div>
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

      {/* Header with breadcrumbs + action buttons */}
      <header className="h-14 border-b border-borders flex items-center px-4 md:px-6 shrink-0 gap-4">
        <Breadcrumbs
          path={currentPath}
          onNavigate={(p) => navigate(`/browse/${p}`)}
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
      />

      {/* Mobile upload FAB */}
      <UploadFAB onClick={uploadFromPicker} />
    </div>
  )
}

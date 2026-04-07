import { useLocation, useNavigate } from 'react-router'
import { Upload, FolderPlus, Refresh } from 'iconoir-react'
import { useFiles } from '../hooks/useFiles'
import type { FileEntry } from '../lib/types'
import Breadcrumbs from '../components/Breadcrumbs'
import FileList from '../components/FileList'

function isTextFile(entry: FileEntry): boolean {
  const mime = entry.mime_type ?? ''
  if (mime.startsWith('text/')) return true
  const textExts = [
    'json', 'yaml', 'yml', 'toml', 'md', 'txt', 'rs', 'py',
    'js', 'ts', 'jsx', 'tsx', 'html', 'htm', 'css', 'sh',
    'go', 'xml', 'csv', 'sql', 'env', 'conf', 'cfg', 'ini', 'log',
  ]
  return textExts.includes(entry.extension?.toLowerCase() ?? '')
}

export default function BrowserPage() {
  const location = useLocation()
  const navigate = useNavigate()

  // Extract path from URL: /browse/path/to/dir -> "path/to/dir"
  const currentPath = location.pathname
    .replace(/^\/browse\/?/, '')
    .replace(/\/$/, '')

  const { listing, loading, error, refresh, deleteItem } = useFiles(currentPath)

  const handleNavigate = (entry: FileEntry) => {
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
      window.open(`/api/fs/${entry.path}?download=true`, '_blank')
    }
  }

  const handleDelete = async (path: string) => {
    if (window.confirm(`Delete "${path.split('/').pop()}"?`)) {
      await deleteItem(path)
    }
  }

  return (
    <>
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
            className="hidden md:flex p-2 text-muted hover:text-primary transition-colors"
            title="New folder"
          >
            <FolderPlus width={18} height={18} strokeWidth={1.8} />
          </button>
          <button className="hidden md:flex items-center gap-2 h-9 px-4 bg-primary text-background font-mono text-[12px] font-bold uppercase tracking-widest hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[3px_3px_0px_#F2F2F2] transition-all">
            <Upload width={14} height={14} strokeWidth={2} />
            UPLOAD
          </button>
        </div>
      </header>

      {/* File listing */}
      <FileList
        listing={listing}
        loading={loading}
        error={error}
        onItemClick={handleNavigate}
        onDelete={handleDelete}
      />
    </>
  )
}

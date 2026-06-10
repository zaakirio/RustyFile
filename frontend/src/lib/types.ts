export interface User {
  id: number
  username: string
  role: 'admin' | 'user'
  created_at: string
  updated_at: string
}

export interface FileEntry {
  name: string
  path: string
  is_dir: boolean
  size: number
  modified: string
  mime_type: string | null
  extension: string | null
}

export interface DirListing {
  is_dir: true
  path: string
  items: FileEntry[]
  num_dirs: number
  num_files: number
}

export interface FileInfo {
  is_dir: false
  name: string
  path: string
  size: number
  modified: string
  mime_type: string | null
  extension: string | null
  content?: string
  subtitles?: string[]
}

export type FsResponse = DirListing | FileInfo

export interface AuthResponse {
  user: User
}

export interface SetupStatus {
  setup_required: boolean
}

export interface ApiError {
  error: string
  code?: string
}

export type SearchScope = 'names' | 'content' | 'both'

export interface SearchParams {
  q: string
  scope?: SearchScope
  type?: 'file' | 'dir' | 'image' | 'video' | 'audio' | 'document'
  min_size?: number
  max_size?: number
  after?: string
  before?: string
  path?: string
  limit?: number
  offset?: number
}

/**
 * A search result entry. Content matches carry a `snippet` with matched
 * terms wrapped in U+E000 / U+E001 markers (rendered as <mark> by the UI).
 */
export interface SearchHit extends FileEntry {
  snippet?: string
}

export interface SearchResponse {
  results: SearchHit[]
  total: number
  query: string
}

export type ShareKind = 'download' | 'drop'

export interface Share {
  token: string
  path: string
  kind: ShareKind
  has_password: boolean
  /** Unix seconds, or null for never. */
  expires_at: number | null
  /** Unix seconds. */
  created_at: number
  download_count: number
  /** Whether the shared path still exists on disk. */
  exists: boolean
}

export interface CreateShareRequest {
  path: string
  kind: ShareKind
  password?: string
  expires_in_hours?: number
}

/**
 * Public share metadata. For password-protected shares without a valid
 * password, the server only returns `name` and `has_password`.
 */
export interface PublicShareMeta {
  name: string
  has_password: boolean
  kind?: ShareKind
  is_dir?: boolean
  size?: number
}

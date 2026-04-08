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
  token: string
  user: User
}

export interface SetupStatus {
  setup_required: boolean
}

export interface ApiError {
  error: string
  code?: string
}

export interface SearchParams {
  q: string
  type?: 'file' | 'dir' | 'image' | 'video' | 'audio' | 'document'
  min_size?: number
  max_size?: number
  after?: string
  before?: string
  path?: string
  limit?: number
  offset?: number
}

export interface SearchResponse {
  results: FileEntry[]
  total: number
  query: string
}

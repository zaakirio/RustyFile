/** Encode a filesystem path for use in API URLs — encodes each segment individually */
export function encodeFsPath(p: string): string {
  if (!p) return ''
  return p.split('/').map(encodeURIComponent).join('/')
}

/** Extract the filesystem path from the current URL for a given route prefix */
export function extractFsPath(pathname: string, prefix: string): string {
  // Handle both "/browse/" and "/browse" (without trailing slash = root)
  const base = prefix.endsWith('/') ? prefix.slice(0, -1) : prefix
  if (pathname === base || pathname === base + '/') return ''
  const raw = pathname.startsWith(prefix) ? pathname.slice(prefix.length) : pathname
  const stripped = raw.startsWith('/') ? raw.slice(1) : raw
  // Decode URI-encoded segments (e.g. spaces, special chars)
  try {
    return decodeURIComponent(stripped)
  } catch {
    return stripped
  }
}

/** Check if a file entry is a text/code file that should open in the editor */
export function isTextFile(entry: { mime_type: string | null; extension: string | null }): boolean {
  if (entry.mime_type?.startsWith('text/')) return true
  const textExts = [
    'json', 'yaml', 'yml', 'toml', 'xml', 'md', 'rs', 'py', 'js', 'ts',
    'tsx', 'jsx', 'css', 'html', 'sh', 'sql', 'cfg', 'ini', 'conf', 'env',
    'dockerfile', 'gitignore', 'makefile', 'txt', 'go', 'csv', 'log', 'htm',
  ]
  return textExts.includes(entry.extension?.toLowerCase() ?? '')
}

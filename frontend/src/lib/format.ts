export function formatSize(bytes: number): string {
  if (bytes === 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(1024))
  const size = bytes / Math.pow(1024, i)
  return `${size < 10 ? size.toFixed(1) : Math.round(size)} ${units[i]}`
}

export function formatDate(iso: string): string {
  const d = new Date(iso)
  const months = ['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec']
  const month = months[d.getUTCMonth()]
  const day = d.getUTCDate()
  const hours = d.getUTCHours().toString().padStart(2, '0')
  const mins = d.getUTCMinutes().toString().padStart(2, '0')
  return `${month} ${day} ${hours}:${mins}`
}

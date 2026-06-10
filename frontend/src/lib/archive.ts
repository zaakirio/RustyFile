/** Archive endpoints expect either JSON or a urlencoded form (see below). */

/** True for files the server can extract (.zip, .tar.gz, .tgz). */
export function isExtractableArchive(entry: { name: string; is_dir: boolean }): boolean {
  if (entry.is_dir) return false
  const name = entry.name.toLowerCase()
  return name.endsWith('.zip') || name.endsWith('.tar.gz') || name.endsWith('.tgz')
}

/**
 * Downloads a selection as a ZIP by submitting a real HTML form POST.
 *
 * A form submission lets the browser stream the (potentially huge) archive
 * natively — progress UI, disk spooling, cancellation — instead of buffering
 * the whole response into a JS blob via fetch(). The server accepts a
 * urlencoded `paths` field containing a JSON-encoded string array; the
 * response's Content-Disposition: attachment keeps the page from navigating.
 */
export function downloadSelectionAsZip(paths: string[]): void {
  if (paths.length === 0) return
  const form = document.createElement('form')
  form.method = 'POST'
  form.action = '/api/archive/download'
  form.style.display = 'none'

  const input = document.createElement('input')
  input.type = 'hidden'
  input.name = 'paths'
  input.value = JSON.stringify(paths)
  form.appendChild(input)

  document.body.appendChild(form)
  form.submit()
  form.remove()
}

/** Plain GET link for downloading a single directory as a ZIP. */
export function directoryZipUrl(path: string): string {
  return `/api/archive/download?path=${encodeURIComponent(path)}`
}

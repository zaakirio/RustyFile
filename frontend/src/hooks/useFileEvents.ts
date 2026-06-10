import { useEffect, useRef } from 'react'

/** Mirror of the backend's serde-tagged FsEvent enum. */
interface FsEvent {
  type: 'dir_changed'
  /** Root-relative directory path; empty string for the root itself. */
  path: string
}

/** Coalesce bursts of change events into a single refresh. */
const REFRESH_DEBOUNCE_MS = 300

/**
 * Subscribes to the server's SSE stream (`GET /api/events`) and invokes
 * `onDirChanged` (debounced) whenever the currently-viewed directory changes
 * on disk.
 *
 * The native EventSource reconnects automatically on error, so the error
 * handler only guards against noise. The connection is opened once per mount
 * and closed on unmount; path/callback changes are tracked via refs so the
 * stream is never torn down on navigation.
 */
export function useFileEvents(currentPath: string, onDirChanged: () => void) {
  const pathRef = useRef(currentPath)
  const onDirChangedRef = useRef(onDirChanged)

  useEffect(() => {
    pathRef.current = currentPath
    onDirChangedRef.current = onDirChanged
  }, [currentPath, onDirChanged])

  useEffect(() => {
    let debounceTimer: number | undefined
    const source = new EventSource('/api/events')

    source.onmessage = (e: MessageEvent<string>) => {
      let event: FsEvent
      try {
        event = JSON.parse(e.data) as FsEvent
      } catch {
        return // ignore malformed frames
      }
      if (event.type !== 'dir_changed' || event.path !== pathRef.current) return
      window.clearTimeout(debounceTimer)
      debounceTimer = window.setTimeout(() => onDirChangedRef.current(), REFRESH_DEBOUNCE_MS)
    }

    source.onerror = () => {
      // EventSource retries automatically (with backoff in modern browsers).
      // Nothing to do here; just keep the handler so errors don't surface
      // as unhandled.
    }

    return () => {
      window.clearTimeout(debounceTimer)
      source.close()
    }
  }, [])
}

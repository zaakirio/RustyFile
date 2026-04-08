import { useState, useCallback, useRef } from 'react'
import * as tus from 'tus-js-client'
import { getToken } from '../api/client'

export interface UploadItem {
  id: string
  name: string
  size: number
  progress: number
  speed: number
  status: 'queued' | 'uploading' | 'complete' | 'error' | 'paused'
  error?: string
}

interface InternalItem extends UploadItem {
  file: File
  tusUpload: tus.Upload | null
  startedAt: number
  bytesAtStart: number
}

interface UseTusUploadOptions {
  currentPath: string
  maxConcurrent?: number
  onAllComplete: () => void
}

export function useTusUpload({
  currentPath,
  maxConcurrent = 3,
  onAllComplete,
}: UseTusUploadOptions) {
  const [items, setItems] = useState<UploadItem[]>([])
  const internalRef = useRef<Map<string, InternalItem>>(new Map())
  const activeCountRef = useRef(0)

  // Store currentPath and onAllComplete in refs so callbacks always see latest values
  const currentPathRef = useRef(currentPath)
  currentPathRef.current = currentPath
  const onAllCompleteRef = useRef(onAllComplete)
  onAllCompleteRef.current = onAllComplete
  const maxConcurrentRef = useRef(maxConcurrent)
  maxConcurrentRef.current = maxConcurrent

  const toPublic = (item: InternalItem): UploadItem => ({
    id: item.id,
    name: item.name,
    size: item.size,
    progress: item.progress,
    speed: item.speed,
    status: item.status,
    error: item.error,
  })

  const syncState = useCallback(() => {
    const map = internalRef.current
    setItems(Array.from(map.values()).map(toPublic))
  }, [])

  const checkAllComplete = useCallback(() => {
    const map = internalRef.current
    const allDone = Array.from(map.values()).every(
      (i) => i.status === 'complete' || i.status === 'error',
    )
    if (allDone && map.size > 0) {
      onAllCompleteRef.current()
    }
  }, [])

  // Use a ref for processQueue/startUpload to break circular dependency
  const processQueueRef = useRef<() => void>(() => {})

  const startUpload = useCallback(
    (id: string) => {
      const map = internalRef.current
      const item = map.get(id)
      if (!item) return

      const path = currentPathRef.current
      const dest = path ? `${path}/${item.name}` : item.name
      const token = getToken()

      item.status = 'uploading'
      item.startedAt = Date.now()
      item.bytesAtStart = 0
      activeCountRef.current++
      syncState()

      const upload = new tus.Upload(item.file, {
        endpoint: '/api/tus',
        retryDelays: [0, 1000, 3000, 5000, 10000],
        chunkSize: 5 * 1024 * 1024,
        metadata: {
          filename: item.name,
          destination: dest,
        },
        headers: {
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        onProgress: (bytesUploaded: number, bytesTotal: number) => {
          const current = map.get(id)
          if (!current) return
          current.progress = Math.round((bytesUploaded / bytesTotal) * 100)
          const elapsed = (Date.now() - current.startedAt) / 1000
          current.speed =
            elapsed > 0 ? (bytesUploaded - current.bytesAtStart) / elapsed : 0
          syncState()
        },
        onSuccess: () => {
          const current = map.get(id)
          if (!current) return
          current.status = 'complete'
          current.progress = 100
          current.speed = 0
          activeCountRef.current--
          syncState()
          processQueueRef.current()
          checkAllComplete()
        },
        onError: (err: tus.DetailedError) => {
          const current = map.get(id)
          if (!current) return
          current.status = 'error'
          current.error = err.message || 'Upload failed'
          current.speed = 0
          activeCountRef.current--
          syncState()
          processQueueRef.current()
          checkAllComplete()
        },
      })

      item.tusUpload = upload
      upload.start()
    },
    [syncState, checkAllComplete],
  )

  // Keep startUpload ref current for processQueue
  const startUploadRef = useRef(startUpload)
  startUploadRef.current = startUpload

  const processQueue = useCallback(() => {
    const map = internalRef.current
    const queued = Array.from(map.values()).filter((i) => i.status === 'queued')
    while (activeCountRef.current < maxConcurrentRef.current && queued.length > 0) {
      const next = queued.shift()!
      startUploadRef.current(next.id)
    }
  }, [])

  // Keep processQueue ref current for tus callbacks
  processQueueRef.current = processQueue

  const addFiles = useCallback(
    (files: File[]) => {
      const map = internalRef.current
      for (const file of files) {
        const id = `${Date.now()}-${Math.random().toString(36).slice(2, 9)}`
        const item: InternalItem = {
          id,
          name: file.name,
          size: file.size,
          progress: 0,
          speed: 0,
          status: 'queued',
          file,
          tusUpload: null,
          startedAt: 0,
          bytesAtStart: 0,
        }
        map.set(id, item)
      }
      syncState()
      processQueue()
    },
    [syncState, processQueue],
  )

  const pauseUpload = useCallback(
    (id: string) => {
      const map = internalRef.current
      const item = map.get(id)
      if (!item || item.status !== 'uploading') return
      item.tusUpload?.abort()
      item.status = 'paused'
      item.speed = 0
      activeCountRef.current--
      syncState()
      processQueue()
    },
    [syncState, processQueue],
  )

  const resumeUpload = useCallback(
    (id: string) => {
      const map = internalRef.current
      const item = map.get(id)
      if (!item || item.status !== 'paused') return
      item.status = 'queued'
      item.speed = 0
      syncState()
      processQueue()
    },
    [syncState, processQueue],
  )

  const clearCompleted = useCallback(() => {
    const map = internalRef.current
    for (const [id, item] of map) {
      if (item.status === 'complete' || item.status === 'error') {
        map.delete(id)
      }
    }
    syncState()
  }, [syncState])

  const hasActive = items.some(
    (i) => i.status === 'uploading' || i.status === 'queued',
  )

  return { items, hasActive, addFiles, pauseUpload, resumeUpload, clearCompleted }
}

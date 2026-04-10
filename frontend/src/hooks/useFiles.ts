import { useState, useEffect, useCallback, useRef } from 'react'
import { api } from '../api/client'
import { encodeFsPath } from '../lib/paths'
import type { DirListing } from '../lib/types'

export function useFiles(path: string) {
  const [listing, setListing] = useState<DirListing | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const mountedRef = useRef(true)

  const fetchListing = useCallback(async (signal?: AbortSignal) => {
    if (!mountedRef.current) return
    setLoading(true)
    setError(null)
    try {
      const res = await api.get<DirListing>(`/api/fs/${encodeFsPath(path)}`, signal)
      if (mountedRef.current) setListing(res)
    } catch (e: unknown) {
      if (e instanceof DOMException && e.name === 'AbortError') return
      if (mountedRef.current) {
        const message = e instanceof Error ? e.message : 'Failed to load'
        setError(message)
      }
    } finally {
      if (mountedRef.current) setLoading(false)
    }
  }, [path])

  useEffect(() => {
    mountedRef.current = true
    const controller = new AbortController()
    fetchListing(controller.signal)
    return () => {
      controller.abort()
      mountedRef.current = false
    }
  }, [fetchListing])

  const deleteItem = useCallback(
    async (itemPath: string) => {
      await api.delete(`/api/fs/${encodeFsPath(itemPath)}`)
      await fetchListing()
    },
    [fetchListing],
  )

  const createDir = useCallback(
    async (dirPath: string) => {
      await api.post(`/api/fs/${encodeFsPath(dirPath)}`, { type: 'directory' })
      await fetchListing()
    },
    [fetchListing],
  )

  return { listing, loading, error, refresh: fetchListing, deleteItem, createDir }
}

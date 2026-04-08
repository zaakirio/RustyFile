import { useState, useEffect, useCallback, useRef } from 'react'
import { api } from '../api/client'
import { encodeFsPath } from '../lib/paths'
import type { DirListing } from '../lib/types'

export function useFiles(path: string) {
  const [listing, setListing] = useState<DirListing | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const mountedRef = useRef(true)

  const fetchListing = useCallback(async () => {
    if (!mountedRef.current) return
    setLoading(true)
    setError(null)
    try {
      const res = await api.get<DirListing>(`/api/fs/${encodeFsPath(path)}`)
      if (mountedRef.current) setListing(res)
    } catch (e: unknown) {
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
    fetchListing()
    return () => { mountedRef.current = false }
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

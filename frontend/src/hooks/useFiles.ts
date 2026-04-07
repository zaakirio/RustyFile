import { useState, useEffect, useCallback } from 'react'
import { api } from '../api/client'
import { encodeFsPath } from '../lib/paths'
import type { DirListing } from '../lib/types'

export function useFiles(path: string) {
  const [listing, setListing] = useState<DirListing | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const fetchListing = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const res = await api.get<DirListing>(`/api/fs/${encodeFsPath(path)}`)
      setListing(res)
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : 'Failed to load'
      setError(message)
    } finally {
      setLoading(false)
    }
  }, [path])

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      setLoading(true)
      setError(null)
      try {
        const res = await api.get<DirListing>(`/api/fs/${encodeFsPath(path)}`)
        if (!cancelled) setListing(res)
      } catch (e: unknown) {
        if (!cancelled) {
          const message = e instanceof Error ? e.message : 'Failed to load'
          setError(message)
        }
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    load()
    return () => { cancelled = true }
  }, [path])

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

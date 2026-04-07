import { useState, useEffect, useCallback } from 'react'
import { api } from '../api/client'
import type { DirListing } from '../lib/types'

export function useFiles(path: string) {
  const [listing, setListing] = useState<DirListing | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const fetchListing = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const res = await api.get<DirListing>(`/api/fs/${path}`)
      setListing(res)
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : 'Failed to load'
      setError(message)
    } finally {
      setLoading(false)
    }
  }, [path])

  useEffect(() => {
    fetchListing()
  }, [fetchListing])

  const deleteItem = useCallback(
    async (itemPath: string) => {
      await api.delete(`/api/fs/${itemPath}`)
      await fetchListing()
    },
    [fetchListing],
  )

  const createDir = useCallback(
    async (dirPath: string) => {
      await api.post(`/api/fs/${dirPath}`, { type: 'directory' })
      await fetchListing()
    },
    [fetchListing],
  )

  return { listing, loading, error, refresh: fetchListing, deleteItem, createDir }
}

import { lazy, Suspense } from 'react'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router'
import { AuthProvider, useAuth } from './hooks/useAuth'
import LoginPage from './pages/LoginPage'
import Layout from './components/Layout'

const BrowserPage = lazy(() => import('./pages/BrowserPage'))
const EditorPage = lazy(() => import('./pages/EditorPage'))
const PlayerPage = lazy(() => import('./pages/PlayerPage'))

function RequireAuth({ children }: { children: React.ReactNode }) {
  const { user, loading } = useAuth()
  if (loading) return <LoadingScreen />
  if (!user) return <Navigate to="/login" replace />
  return children
}

function LoadingScreen() {
  return (
    <div className="flex items-center justify-center h-screen font-mono text-primary text-lg tracking-widest uppercase">
      [ LOADING... ]
    </div>
  )
}

export default function App() {
  return (
    <BrowserRouter>
      <div className="grain-overlay" />
      <AuthProvider>
        <AppRoutes />
      </AuthProvider>
    </BrowserRouter>
  )
}

function AppRoutes() {
  const { setupRequired, loading } = useAuth()

  if (loading) return <LoadingScreen />

  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route
        element={
          <RequireAuth>
            <Layout />
          </RequireAuth>
        }
      >
        <Route path="/browse/*" element={<Suspense fallback={<LoadingScreen />}><BrowserPage /></Suspense>} />
        <Route path="/edit/*" element={<Suspense fallback={<LoadingScreen />}><EditorPage /></Suspense>} />
        <Route path="/play/*" element={<Suspense fallback={<LoadingScreen />}><PlayerPage /></Suspense>} />
        <Route
          path="/stash/*"
          element={
            <div className="flex-1 flex items-center justify-center font-mono text-primary uppercase tracking-widest">
              [ STASH // COMING SOON ]
            </div>
          }
        />
      </Route>
      <Route
        path="*"
        element={
          <Navigate to={setupRequired ? '/login' : '/browse'} replace />
        }
      />
    </Routes>
  )
}

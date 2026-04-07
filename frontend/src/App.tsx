import { BrowserRouter, Routes, Route, Navigate } from 'react-router'
import { AuthProvider, useAuth } from './hooks/useAuth'
import LoginPage from './pages/LoginPage'

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
        path="/browse/*"
        element={
          <RequireAuth>
            <div className="flex items-center justify-center h-screen font-mono text-primary uppercase tracking-widest">
              [ BROWSER // COMING SOON ]
            </div>
          </RequireAuth>
        }
      />
      <Route
        path="*"
        element={
          <Navigate to={setupRequired ? '/login' : '/browse'} replace />
        }
      />
    </Routes>
  )
}

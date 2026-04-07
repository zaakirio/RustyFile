import { BrowserRouter, Routes, Route } from 'react-router'

export default function App() {
  return (
    <BrowserRouter>
      <div className="grain-overlay" />
      <Routes>
        <Route
          path="*"
          element={
            <div className="flex items-center justify-center h-screen font-mono text-primary text-2xl tracking-widest uppercase">
              SYS_DIR // ONLINE
            </div>
          }
        />
      </Routes>
    </BrowserRouter>
  )
}

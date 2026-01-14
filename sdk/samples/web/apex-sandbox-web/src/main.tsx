import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './App'

if (window.location.hostname !== 'localhost' && window.location.hostname !== '127.0.0.1') {
  document.body.innerHTML = '<h2>Localhost only</h2><p>This sandbox may only be run on localhost.</p>'
} else {
  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  )
}

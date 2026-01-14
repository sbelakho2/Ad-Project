import React, { useState } from 'react'
import { sandboxConfig } from './sandboxConfig'

export function App() {
  const [status, setStatus] = useState('SDK: not initialized')

  const init = () => {
    // TODO: Wire this to the real web SDK initialize, using sandboxConfig
    setStatus(`SDK: initialized (stub) → ${sandboxConfig.apiBase}`)
  }

  const loadInterstitial = () => setStatus('Interstitial: loaded (stub)')
  const showInterstitial = () => setStatus('Interstitial: shown (stub)')
  const loadRewarded = () => setStatus('Rewarded: loaded (stub)')
  const showRewarded = () => setStatus('Rewarded: shown (stub)')

  return (
    <div style={{ padding: 16, fontFamily: 'system-ui, sans-serif' }}>
      <h1>Apex Sandbox Web</h1>
      <p>{status}</p>
      <div style={{ display: 'grid', gap: 8, maxWidth: 360 }}>
        <button onClick={init}>Initialize</button>
        <button onClick={loadInterstitial}>Load Interstitial</button>
        <button onClick={showInterstitial}>Show Interstitial</button>
        <button onClick={loadRewarded}>Load Rewarded</button>
        <button onClick={showRewarded}>Show Rewarded</button>
      </div>
      <pre style={{ marginTop: 16, background: '#f6f8fa', padding: 12 }}>
        {JSON.stringify(sandboxConfig, null, 2)}
      </pre>
    </div>
  )
}

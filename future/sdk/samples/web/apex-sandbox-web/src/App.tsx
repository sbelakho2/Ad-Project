import React, { useState } from 'react'
import {
  init,
  isInitialized,
  requestAd,
  setConsent,
  type AdResponse,
} from '@rivalapex/web-sdk'
import { sandboxConfig } from './sandboxConfig'

type Status = { text: string; response?: AdResponse }

export function App() {
  const [status, setStatus] = useState<Status>({ text: 'SDK: not initialized' })

  const usPrivacy = sandboxConfig.consent.ccpa ? '1YYN' : '1NNN'

  const initialize = () => {
    try {
      init({
        endpoint: sandboxConfig.apiBase,
        appId: 'apex-sandbox-web',
        debug: true,
        telemetryEnabled: false,
      })
      setConsent({
        gdprApplies: sandboxConfig.consent.gdpr,
        usPrivacy,
        coppa: sandboxConfig.consent.coppa,
      })
      setStatus({ text: `SDK: initialized against ${sandboxConfig.apiBase}` })
    } catch (error) {
      setStatus({ text: `SDK: initialization failed: ${(error as Error).message}` })
    }
  }

  const request = async (
    placement: string,
    adType: 'interstitial' | 'rewarded',
    label: string,
  ) => {
    if (!isInitialized()) {
      setStatus({ text: 'SDK: initialize before requesting ads' })
      return
    }
    setStatus({ text: `${label}: requesting` })
    try {
      const response = await requestAd({ placement, adType, testMode: true })
      const outcome = response.fill
        ? `${label}: ${response.networkName ?? 'filled'} @ ${response.price ?? 0} ${response.currency ?? ''}`
        : `${label}: no fill (request ${response.requestId})`
      setStatus({ text: outcome, response })
    } catch (error) {
      setStatus({ text: `${label}: failed: ${(error as Error).message}` })
    }
  }

  return (
    <div style={{ padding: 16, fontFamily: 'system-ui, sans-serif' }}>
      <h1>Apex Sandbox Web</h1>
      <p>{status.text}</p>
      <div style={{ display: 'grid', gap: 8, maxWidth: 360 }}>
        <button onClick={initialize}>Initialize</button>
        <button
          onClick={() =>
            request(sandboxConfig.placements.interstitialA, 'interstitial', 'Interstitial')
          }
        >
          Request Interstitial
        </button>
        <button
          onClick={() => request(sandboxConfig.placements.rewardedA, 'rewarded', 'Rewarded')}
        >
          Request Rewarded
        </button>
      </div>
      {status.response?.creative?.html ? (
        <div
          style={{ marginTop: 16, border: '1px solid #d0d7de', padding: 12 }}
          // Sandbox-only echo of the returned creative markup.
          dangerouslySetInnerHTML={{ __html: status.response.creative.html }}
        />
      ) : null}
      <pre style={{ marginTop: 16, background: '#f6f8fa', padding: 12 }}>
        {JSON.stringify(sandboxConfig, null, 2)}
      </pre>
    </div>
  )
}

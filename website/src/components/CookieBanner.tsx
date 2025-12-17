'use client';

import { useEffect, useState } from 'react';

/**
 * Cookie consent banner
 * Reference: Design.md § "Cookie Banner"
 */
export default function CookieBanner() {
  const [isVisible, setIsVisible] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [preferences, setPreferences] = useState({
    analytics: true,
    marketing: false,
    functional: true,
  });

  useEffect(() => {
    // Check if user has already accepted cookies
    const cookieConsent = localStorage.getItem('cookieConsent');
    if (!cookieConsent) {
      setIsVisible(true);
    }
  }, []);

  const persistPreferences = (value: 'accepted' | 'custom', prefs = preferences) => {
    localStorage.setItem(
      'cookieConsent',
      JSON.stringify({ status: value, preferences: prefs })
    );
  };

  const handleAccept = () => {
    const allOn = { analytics: true, marketing: true, functional: true };
    setPreferences(allOn);
    persistPreferences('accepted', allOn);
    setIsVisible(false);
    setShowSettings(false);
  };

  const handleSavePreferences = () => {
    persistPreferences('custom');
    setIsVisible(false);
    setShowSettings(false);
  };

  if (!isVisible) return null;

  return (
    <div className="fixed bottom-0 left-0 right-0 z-50 animate-slide-up" role="dialog" aria-live="polite">
      <div className="bg-white border-t border-gray-200 px-4 py-4 shadow-lg">
        <div className="container flex flex-col items-center justify-between gap-4 sm:flex-row">
          <p className="text-sm text-gray-700">
            We use cookies to improve your experience. By continuing, you accept our cookie policy.
          </p>

          <div className="flex items-center gap-3">
            <button
              onClick={() => setShowSettings(true)}
              className="btn-ghost text-sm"
            >
              Cookie settings
            </button>
            <button
              onClick={handleAccept}
              className="btn-primary text-sm"
            >
              Accept all cookies
            </button>
          </div>
        </div>
      </div>

      {showSettings && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-4 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-labelledby="cookie-settings-title"
          onKeyDown={(e) => {
            if (e.key === 'Escape') setShowSettings(false)
          }}
        >
          <div className="w-full max-w-lg rounded-xl border border-gray-200 bg-white shadow-xl">
            <div className="border-b border-gray-200 p-6">
              <h2 id="cookie-settings-title" className="text-lg font-semibold text-gray-900">
                Cookie Preferences
              </h2>
              <p className="mt-2 text-sm text-gray-600">
                Choose which types of cookies you want to accept. Essential cookies are always on because they make the site work.
              </p>
            </div>

            <div className="space-y-4 p-6">
              <fieldset className="space-y-3">
                <legend className="sr-only">Cookie preferences</legend>

                <CookieSwitch
                  id="functional-cookies"
                  label="Functional Cookies"
                  description="Required for core functionality like authentication and remembering your settings."
                  checked={preferences.functional}
                  disabled
                  onChange={() => {
                    setPreferences((prev) => ({ ...prev, functional: true }));
                  }}
                />

                <CookieSwitch
                  id="analytics-cookies"
                  label="Analytics Cookies"
                  description="Help us understand how the site is used so we can improve performance."
                  checked={preferences.analytics}
                  onChange={(value) =>
                    setPreferences((prev) => ({ ...prev, analytics: value }))
                  }
                />

                <CookieSwitch
                  id="marketing-cookies"
                  label="Marketing Cookies"
                  description="Used to personalize advertising and measure campaign effectiveness."
                  checked={preferences.marketing}
                  onChange={(value) =>
                    setPreferences((prev) => ({ ...prev, marketing: value }))
                  }
                />
              </fieldset>
            </div>

            <div className="flex flex-col justify-end gap-3 border-t border-gray-200 p-6 sm:flex-row">
              <button onClick={() => setShowSettings(false)} className="btn-ghost text-sm">
                Cancel
              </button>
              <button onClick={handleSavePreferences} className="btn-primary text-sm">
                Save preferences
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

interface CookieSwitchProps {
  id: string;
  label: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}

function CookieSwitch({ id, label, description, checked, disabled, onChange }: CookieSwitchProps) {
  return (
    <div className={`rounded p-4 border border-gray-200 ${disabled ? 'bg-gray-100' : 'bg-white'}`}>
      <div className="flex items-start justify-between gap-4">
        <div>
          <label htmlFor={id} className="block font-semibold text-gray-900">
            {label}
          </label>
          <p className="text-sm text-gray-600 mt-1">{description}</p>
        </div>
        <label className={`relative inline-flex h-6 w-12 items-center ${disabled ? 'opacity-60' : ''}`}>
          <input
            id={id}
            type="checkbox"
            checked={checked}
            disabled={disabled}
            onChange={(event) => onChange(event.target.checked)}
            className="sr-only"
          />
          <span
            className={`absolute inset-0 rounded-full transition-colors ${checked ? 'bg-brand-500' : 'bg-gray-300'}`}
            aria-hidden="true"
          />
          <span
            className={`absolute left-1 top-1 h-4 w-4 rounded-full bg-white transition-transform ${
              checked ? 'translate-x-6' : ''
            }`}
            aria-hidden="true"
          />
        </label>
      </div>
    </div>
  );
}

'use client';

import { useState } from 'react';

/**
 * Top notification bar with scalloped bottom edge
 * Reference: Design.md § "Top Notification Bar"
 */
export default function NotificationBar() {
  const [isVisible, setIsVisible] = useState(true);

  if (!isVisible) return null;

  return (
    <div className="relative bg-brand-600 text-white">
      <div className="container mx-auto px-4 py-3 flex items-center justify-between">
        <div className="flex items-center gap-4">
          <span className="font-bold text-brand-100">ApexMediation</span>
        </div>

        <p className="text-sm md:text-base text-white font-medium">
          🎉 Bring-your-own SDKs now ship 15 built-in adapters across Android, iOS, Unity, CTV, and Web.
        </p>

        <button
          onClick={() => setIsVisible(false)}
          className="text-white hover:text-brand-100 transition-colors font-bold text-xl focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
          aria-label="Close notification"
        >
          ×
        </button>
      </div>

      {/* Scalloped bottom edge */}
      <div className="absolute bottom-0 left-0 right-0 translate-y-full">
        <svg
          className="w-full h-4"
          viewBox="0 0 1200 20"
          preserveAspectRatio="none"
          xmlns="http://www.w3.org/2000/svg"
        >
          <path
            d="M0,0 Q30,20 60,0 T120,0 T180,0 T240,0 T300,0 T360,0 T420,0 T480,0 T540,0 T600,0 T660,0 T720,0 T780,0 T840,0 T900,0 T960,0 T1020,0 T1080,0 T1140,0 T1200,0 L1200,20 L0,20 Z"
            fill="rgb(var(--brand-600))"
          />
        </svg>
      </div>
    </div>
  );
}

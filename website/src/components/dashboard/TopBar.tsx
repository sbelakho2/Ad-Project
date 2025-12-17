'use client';

import { Bars3Icon, BellIcon, UserCircleIcon } from '@heroicons/react/24/outline';
import { useRouter } from 'next/navigation';
import { useState } from 'react';

type DashboardTopBarProps = {
  onToggleSidebar?: () => void;
};

export default function DashboardTopBar({ onToggleSidebar }: DashboardTopBarProps) {
  const router = useRouter();
  const [showUserMenu, setShowUserMenu] = useState(false);

  const handleLogout = async () => {
    await fetch('/api/auth/logout', { method: 'POST' });
    window.location.href = 'https://console.apexmediation.ee/login';
  };

  return (
    <div className="sticky top-0 z-20 flex h-16 flex-shrink-0 items-center bg-brand-500 text-white shadow-md backdrop-blur">
      <button
        type="button"
        className="border-r border-white/10 px-4 text-white/90 transition hover:text-white focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white lg:hidden"
        aria-label="Open sidebar"
  onClick={() => onToggleSidebar?.()}
      >
        <Bars3Icon className="h-6 w-6" aria-hidden="true" />
      </button>
      <div className="flex flex-1 items-center justify-between px-4">
        <div className="flex flex-1 items-center gap-3">
          <span className="hidden text-xs font-semibold uppercase tracking-[0.2em] text-white/90 sm:inline-flex">
            ApexMediation Control Center
          </span>
        </div>
        <div className="ml-4 flex items-center gap-3 md:ml-6">
          <button
            type="button"
            className="btn-secondary h-10 !rounded-full !bg-white/10 !text-white hover:!bg-white/20"
            aria-label="View notifications"
            onClick={() => router.push('/dashboard/settings?tab=notifications')}
          >
            <BellIcon className="h-6 w-6" aria-hidden="true" />
          </button>

          {/* Profile dropdown */}
          <div className="relative ml-3">
            <div>
              <button
                type="button"
                className="flex max-w-xs items-center rounded-full border border-white/20 bg-white/10 p-1 text-sm transition hover:bg-white/20"
                onClick={() => setShowUserMenu(!showUserMenu)}
                aria-haspopup="menu"
                aria-expanded={showUserMenu}
              >
                <span className="sr-only">Open user menu</span>
                <UserCircleIcon className="h-8 w-8 text-white" />
              </button>
            </div>

            {showUserMenu && (
              <div
                className="absolute right-0 z-40 mt-2 w-56 origin-top-right rounded-xl bg-white py-2 text-gray-900 shadow-xl ring-1 ring-gray-200 focus:outline-none"
                role="menu"
              >
                <a
                  href="/dashboard/settings"
                  className="block px-4 py-2 text-sm font-semibold transition hover:bg-gray-50"
                  role="menuitem"
                >
                  Your Profile
                </a>
                <a
                  href="/dashboard/settings"
                  className="block px-4 py-2 text-sm font-semibold transition hover:bg-gray-50"
                  role="menuitem"
                >
                  Settings
                </a>
                <button
                  onClick={handleLogout}
                  className="block w-full px-4 py-2 text-left text-sm font-semibold text-danger transition hover:bg-gray-50"
                  type="button"
                  role="menuitem"
                >
                  Sign out
                </button>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

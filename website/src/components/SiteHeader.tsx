'use client';

import HomeNav from '@/components/HomeNav';

/**
 * SiteHeader - Shared Header Component
 * 
 * ⚠️ DO NOT BYPASS THIS COMPONENT ⚠️
 * 
 * The header MUST remain white site-wide to prevent disorienting
 * "inverted" navigation experiences when moving between dark/light pages.
 * 
 * ❌ DO NOT create custom <header> blocks
 * ❌ DO NOT change header background based on page type
 * ❌ DO NOT add inline header navigation
 * 
 * This component ensures:
 * - Consistent white background (bg-white)
 * - Consistent border (border-b border-slate-200/60)
 * - Consistent padding (py-6)
 * - Single navigation implementation
 * 
 * @see DESIGN_SYSTEM.md for full standards
 */

export default function SiteHeader() {
  return (
    <header className="bg-white border-b border-slate-200/60">
      <div className="container mx-auto px-4 py-6">
        <HomeNav />
      </div>
    </header>
  );
}

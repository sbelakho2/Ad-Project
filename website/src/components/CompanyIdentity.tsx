import { legalEntity } from '@/config/legal';

/**
 * CompanyIdentity: Displays legal entity information consistently.
 * Used in Footer and Company page.
 * 
 * Variants:
 * - 'compact': Footer format (stacked text)
 * - 'full': Company page format (card with separator)
 */
interface CompanyIdentityProps {
  variant?: 'compact' | 'full';
  className?: string;
}

export function CompanyIdentity({ variant = 'compact', className = '' }: CompanyIdentityProps) {
  if (variant === 'full') {
    return (
      <div className={`bg-slate-50 rounded-xl p-6 border border-slate-200/70 ${className}`}>
        <p className="font-semibold text-slate-900 mb-2">{legalEntity.name}</p>
        <p className="text-slate-600 mb-1">{legalEntity.address.region}, {legalEntity.address.city}</p>
        <p className="text-slate-600 mb-1">{legalEntity.address.street}</p>
        <p className="text-slate-600 mb-1">{legalEntity.address.postalCode}</p>
        <p className="text-slate-600">{legalEntity.address.country}</p>
        <div className="mt-4 pt-4 border-t border-slate-200">
          <p className="text-sm text-slate-500">
            {legalEntity.fullRegistryLine}
          </p>
        </div>
      </div>
    );
  }

  // Compact variant (for footer)
  return (
    <div className={`text-xs text-gray-500 ${className}`}>
      <p className="font-medium">{legalEntity.name}</p>
      <p>{legalEntity.address.street}</p>
      <p>{legalEntity.address.postalCode} {legalEntity.address.city}, {legalEntity.address.country}</p>
      <p className="mt-1">{legalEntity.registryLine}</p>
    </div>
  );
}

export default CompanyIdentity;

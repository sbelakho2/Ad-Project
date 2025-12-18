/**
 * MetricCard: A reusable component for displaying metric values with labels and captions.
 * Used in Features page and other places where we show numeric stats.
 */

interface MetricCardProps {
  value: string | number;
  label: string;
  caption?: string;
  className?: string;
  layout?: 'stack' | 'inline';
}

export function MetricCard({ value, label, caption, className = '', layout = 'stack' }: MetricCardProps) {
  if (layout === 'inline') {
    return (
      <div className={`flex items-baseline gap-3 ${className}`}>
        <div className="text-3xl md:text-4xl font-bold text-brand-600 leading-none">{value}</div>
        <div className="text-gray-700 font-medium whitespace-pre-wrap">{label}</div>
      </div>
    );
  }
  return (
    <div className={`text-center ${className}`}>
      <div className="text-4xl font-bold text-brand-600 mb-2">{value}</div>
      <div className="text-gray-600 font-medium">{label}</div>
      {caption && <div className="text-xs text-gray-500 mt-1">{caption}</div>}
    </div>
  );
}

interface MetricGridProps {
  metrics: MetricCardProps[];
  className?: string;
}

export function MetricGrid({ metrics, className = '' }: MetricGridProps) {
  const cols = metrics.length <= 3 ? 'grid-cols-' + metrics.length : 'grid-cols-2 md:grid-cols-4';
  return (
    <div className={`grid ${cols} gap-6 ${className}`}>
      {metrics.map((metric, index) => (
        <MetricCard key={index} {...metric} />
      ))}
    </div>
  );
}

export default MetricCard;

'use client'

import React, { cloneElement, isValidElement, useMemo } from 'react'
import { ArrowUpRight, ArrowDownRight } from 'lucide-react'
import { Skeleton } from '@/components/ui'

type IconProps = { className?: string; 'aria-hidden'?: boolean }
type IconRenderable = React.ComponentType<IconProps> | React.ReactElement<Partial<IconProps>> | null

const mergeClassNames = (...classes: Array<string | undefined>) => classes.filter(Boolean).join(' ')

interface MetricCardProps {
  title: string
  value: string
  change?: number
  icon: IconRenderable
  trend?: 'up' | 'down' | 'neutral'
  changeFormatter?: (value: number) => string
  comparisonLabel?: string
  iconLabel?: string
}

const renderIcon = (icon: IconRenderable) => {
  if (!icon) return null
  if (isValidElement(icon)) {
    return cloneElement(icon, {
      className: mergeClassNames('h-5 w-5', icon.props.className),
      'aria-hidden': icon.props['aria-hidden'] ?? true,
    })
  }

  const IconComponent = icon as React.ComponentType<IconProps>
  return <IconComponent className="h-5 w-5" aria-hidden={true} />
}

export function MetricCard({
  title,
  value,
  change,
  icon,
  trend,
  changeFormatter,
  comparisonLabel = 'vs last period',
  iconLabel,
}: MetricCardProps) {
  const resolvedTrend: 'up' | 'down' | 'neutral' =
    trend ??
    (typeof change === 'number'
      ? change > 0
        ? 'up'
        : change < 0
          ? 'down'
          : 'neutral'
      : 'neutral')

  const trendColor =
    resolvedTrend === 'up'
      ? 'text-green-700'
      : resolvedTrend === 'down'
        ? 'text-red-700'
        : 'text-gray-500'

  const suffix = comparisonLabel?.trim() || ''
  const changeText = useMemo(() => {
    if (typeof change !== 'number') return null
    const formatter = changeFormatter ?? ((value: number) => `${value.toFixed(1)}%`)
    const formattedValue = formatter(Math.abs(change))
    return suffix ? `${formattedValue} ${suffix}` : formattedValue
  }, [change, changeFormatter, suffix])
  const neutralComparisonText = suffix ? `No change ${suffix}` : 'No change'

  const iconContent = useMemo(() => renderIcon(icon), [icon])
  const iconAccessibilityProps = iconLabel
    ? { role: 'img' as const, 'aria-label': iconLabel }
    : { 'aria-hidden': true }

  return (
    <div className="card">
      <div className="flex items-start justify-between mb-3">
        <div className="leading-tight">
          <span className="text-xs font-medium text-gray-500 block">
            {title}
          </span>
          <span className="mt-2 text-2xl font-semibold text-gray-900 leading-tight block">
            {value}
          </span>
        </div>
        <div
          className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary-50 text-primary-600"
          {...iconAccessibilityProps}
        >
          {iconContent}
        </div>
      </div>

      {typeof change === 'number' && changeText && (
        <div className={`flex items-center text-sm font-medium ${trendColor}`}>
          {resolvedTrend === 'up' && <ArrowUpRight className="mr-1 h-4 w-4" aria-hidden="true" />}
          {resolvedTrend === 'down' && <ArrowDownRight className="mr-1 h-4 w-4" aria-hidden="true" />}
          <span>{resolvedTrend === 'neutral' ? neutralComparisonText : changeText}</span>
        </div>
      )}
    </div>
  )
}

interface MetricSkeletonProps {
  count?: number
}

export function MetricCardSkeleton({ count = 4 }: MetricSkeletonProps) {
  const skeletonKeys = Array.from({ length: count }, (_, index) => `metric-card-skeleton-${index}`)

  return (
    <>
      {skeletonKeys.map((key) => (
        <div key={key} className="card">
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <Skeleton width="w-24" height="h-4" variant="text" />
              <Skeleton width="w-10" height="h-10" variant="circular" />
            </div>
            <Skeleton width="w-28" height="h-8" variant="rectangular" />
            <Skeleton width="w-32" height="h-4" variant="text" />
          </div>
        </div>
      ))}
    </>
  )
}

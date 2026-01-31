import enMessages from './messages/en.json'

type Messages = typeof enMessages
type MessageKey = string
type TranslateOptions = { fallback?: string }
type CurrencyFormatOptions = Intl.NumberFormatOptions & { fromMinorUnits?: boolean }

const messageCatalog: Record<string, Messages> = {
  en: enMessages,
}

// Simple i18n implementation
// TODO: Extend with additional locales and full pluralization support
export class I18n {
  private messages: Messages
  private locale: string
  private readonly catalogs: Record<string, Messages>
  private readonly missingKeys = new Set<string>()

  constructor(locale = 'en', catalogs: Record<string, Messages> = messageCatalog) {
    this.locale = locale
    this.catalogs = catalogs
    this.messages = this.resolveLocaleMessages(locale)
  }

  private resolveLocaleMessages(locale: string) {
    return this.catalogs[locale] || this.catalogs.en
  }

  private formatString(value: string, params?: Record<string, string | number>) {
    if (!params) return value
    return Object.entries(params).reduce((str, [param, val]) => {
      // Escape regex special characters in param to prevent ReDoS
      const escapedParam = param.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
      return str.replace(new RegExp(`{${escapedParam}}`, 'g'), String(val))
    }, value)
  }

  private handleMissingTranslation(key: string, fallback?: string) {
    if (!this.missingKeys.has(key)) {
      this.missingKeys.add(key)
      // eslint-disable-next-line no-console
      console.warn(`[i18n] Missing translation for "${key}" in locale "${this.locale}"`)
    }
    return fallback ?? key
  }

  t(key: MessageKey, params?: Record<string, string | number>, options?: TranslateOptions): string {
    const keys = key.split('.')
    let value: any = this.messages

    for (const k of keys) {
      if (value && typeof value === 'object' && k in value) {
        value = value[k]
      } else {
        return this.handleMissingTranslation(key, options?.fallback)
      }
    }

    if (typeof value !== 'string') {
      return this.handleMissingTranslation(key, options?.fallback)
    }

    return this.formatString(value, params)
  }

  setLocale(locale: string) {
    this.locale = locale
    this.messages = this.resolveLocaleMessages(locale)
  }

  getLocale() {
    return this.locale
  }

  // Format currency
  formatCurrency(amount: number, currency = 'USD', options: CurrencyFormatOptions = {}): string {
    const { fromMinorUnits = false, ...intlOptions } = options
    const normalizedAmount = fromMinorUnits ? amount / 100 : amount
    return new Intl.NumberFormat(this.locale, {
      style: 'currency',
      currency,
      ...intlOptions,
    }).format(normalizedAmount)
  }

  // Format number
  formatNumber(value: number, options?: Intl.NumberFormatOptions): string {
    return new Intl.NumberFormat(this.locale, options).format(value)
  }

  // Format date
  formatDate(date: string | Date, options?: Intl.DateTimeFormatOptions): string {
    const dateObj = typeof date === 'string' ? new Date(date) : date
    return new Intl.DateTimeFormat(this.locale, options || {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
    }).format(dateObj)
  }

  // Format date range
  formatDateRange(start: string | Date, end: string | Date): string {
    return `${this.formatDate(start)} – ${this.formatDate(end)}`
  }

  // Format relative time
  formatRelativeTime(date: string | Date): string {
    const dateObj = typeof date === 'string' ? new Date(date) : date
    const now = new Date()
    const diffMs = now.getTime() - dateObj.getTime()
    const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24))

    if (diffDays === 0) {
      return 'Today'
    } else if (diffDays === 1) {
      return 'Yesterday'
    } else if (diffDays < 7) {
      return `${diffDays} days ago`
    } else if (diffDays < 30) {
      const weeks = Math.floor(diffDays / 7)
      return `${weeks} week${weeks > 1 ? 's' : ''} ago`
    } else if (diffDays < 365) {
      const months = Math.floor(diffDays / 30)
      return `${months} month${months > 1 ? 's' : ''} ago`
    } else {
      const years = Math.floor(diffDays / 365)
      return `${years} year${years > 1 ? 's' : ''} ago`
    }
  }

  // Format large numbers with abbreviations
  formatLargeNumber(value: number): string {
    if (value >= 1_000_000_000) {
      return `${(value / 1_000_000_000).toFixed(1)}B`
    } else if (value >= 1_000_000) {
      return `${(value / 1_000_000).toFixed(1)}M`
    } else if (value >= 1_000) {
      return `${(value / 1_000).toFixed(1)}K`
    }
    return value.toString()
  }

  // Format percentage
  formatPercentage(value: number, decimals = 1): string {
    return `${value.toFixed(decimals)}%`
  }

  // Format bytes to human readable
  formatBytes(bytes: number, decimals = 2): string {
    if (bytes === 0) return '0 Bytes'

    const k = 1024
    const dm = decimals < 0 ? 0 : decimals
    const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB']

    const i = Math.floor(Math.log(bytes) / Math.log(k))

    return `${parseFloat((bytes / Math.pow(k, i)).toFixed(dm))} ${sizes[i]}`
  }
}

// Create singleton instance
export const i18n = new I18n('en')

export const registerLocaleMessages = (locale: string, messages: Messages) => {
  messageCatalog[locale] = messages
  if (i18n.getLocale() === locale) {
    i18n.setLocale(locale)
  }
}

// Export convenience functions
export const t = (key: MessageKey, params?: Record<string, string | number>, options?: TranslateOptions) =>
  i18n.t(key, params, options)
export const formatCurrency = (amount: number, currency = 'USD', options?: CurrencyFormatOptions) =>
  i18n.formatCurrency(amount, currency, options)
export const formatNumber = (value: number, options?: Intl.NumberFormatOptions) => i18n.formatNumber(value, options)
export const formatDate = (date: string | Date, options?: Intl.DateTimeFormatOptions) => i18n.formatDate(date, options)
export const formatDateRange = (start: string | Date, end: string | Date) => i18n.formatDateRange(start, end)
export const formatRelativeTime = (date: string | Date) => i18n.formatRelativeTime(date)
export const formatLargeNumber = (value: number) => i18n.formatLargeNumber(value)
export const formatPercentage = (value: number, decimals?: number) => i18n.formatPercentage(value, decimals)
export const formatBytes = (bytes: number, decimals?: number) => i18n.formatBytes(bytes, decimals)

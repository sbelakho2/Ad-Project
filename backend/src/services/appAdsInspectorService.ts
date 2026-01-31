import axios from 'axios'

export type VendorRule = {
  vendor: string
  requiredLines: string[]
  optionalLines?: string[]
}

export type InspectorResult = {
  domain: string
  fetched: boolean
  httpStatus?: number
  vendors: Array<{
    vendor: string
    pass: boolean
    missing: string[]
    suggested: string[]
  }>
  rawSample?: string
}

// Minimal initial vendor rules; can be extended. Lines are templates; the console can render with pub info.
export const DEFAULT_VENDOR_RULES: VendorRule[] = [
  { vendor: 'admob', requiredLines: ['google.com, pub-XXXXXXXXXXXXXXXX, DIRECT, f08c47fec0942fa0'] },
  { vendor: 'unity', requiredLines: ['unity.com, XXXXXXXXXXXXXXXX, DIRECT'] },
  { vendor: 'applovin', requiredLines: ['applovin.com, XXXXXXXXXXXXXXXX, DIRECT, 5a86'] },
  { vendor: 'ironsource', requiredLines: ['ironsrc.com, XXXXXXXXXXXXXXXX, DIRECT'] },
  { vendor: 'meta', requiredLines: ['facebook.com, XXXXXXXXXXXXXXXX, DIRECT'] },
  { vendor: 'chartboost', requiredLines: ['chartboost.com, XXXXXXXXXXXXXXXX, DIRECT'] },
  { vendor: 'vungle', requiredLines: ['vungle.com, XXXXXXXXXXXXXXXX, DIRECT'] },
  { vendor: 'pangle', requiredLines: ['bytedance.com, XXXXXXXXXXXXXXXX, DIRECT'] },
  { vendor: 'fyber', requiredLines: ['fyber.com, XXXXXXXXXXXXXXXX, DIRECT'] },
  { vendor: 'mintegral', requiredLines: ['mintegral.com, XXXXXXXXXXXXXXXX, DIRECT'] },
  { vendor: 'tapjoy', requiredLines: ['tapjoy.com, XXXXXXXXXXXXXXXX, DIRECT'] },
  { vendor: 'amazon', requiredLines: ['amazon.com, XXXXXXXXXXXXXXXX, DIRECT'] },
]

/**
 * SECURITY: Validate domain to prevent SSRF attacks
 * P0 Fix #12: Comprehensive domain validation
 */
function validateDomain(domain: string): { valid: boolean; reason?: string } {
  // Domain must be a valid hostname (no paths, no protocols, no ports)
  const domainPattern = /^[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$/;
  if (!domainPattern.test(domain)) {
    return { valid: false, reason: 'Invalid domain format' };
  }
  
  // Block IP addresses
  if (/^(\d{1,3}\.){3}\d{1,3}$/.test(domain)) {
    return { valid: false, reason: 'IP addresses not allowed' };
  }
  
  // Block localhost and internal hostnames
  const blocked = [
    'localhost',
    '127.0.0.1',
    'metadata.google.internal',
    '169.254.169.254',
    '100.100.100.200',
  ];
  if (blocked.includes(domain.toLowerCase())) {
    return { valid: false, reason: 'Internal hostname not allowed' };
  }
  
  // Block internal TLDs
  const blockedSuffixes = ['.local', '.internal', '.localhost', '.lan', '.cluster.local'];
  if (blockedSuffixes.some(suffix => domain.toLowerCase().endsWith(suffix))) {
    return { valid: false, reason: 'Internal TLD not allowed' };
  }
  
  // Must have at least one dot (a real domain)
  if (!domain.includes('.')) {
    return { valid: false, reason: 'Domain must include TLD' };
  }
  
  return { valid: true };
}

export async function fetchAppAdsTxt(domain: string): Promise<{status: number, text: string}> {
  // SECURITY: Validate domain before fetching (P0 Fix #12)
  const validation = validateDomain(domain);
  if (!validation.valid) {
    throw new Error(`Domain validation failed: ${validation.reason}`);
  }
  
  const url = `https://${domain.replace(/\/$/, '')}/app-ads.txt`
  const resp = await axios.get(url, { 
    validateStatus: () => true, 
    responseType: 'text',
    timeout: 10000, // SECURITY: Add timeout to prevent slow loris attacks
    maxRedirects: 3, // SECURITY: Limit redirects
  })
  return { status: resp.status, text: (resp.data ?? '').toString() }
}

export function parseLines(txt: string): string[] {
  return txt
    .split(/\r?\n/)
    .map(l => l.trim())
    .filter(l => l.length > 0 && !l.startsWith('#'))
}

export function inspectAgainstRules(lines: string[], rules: VendorRule[] = DEFAULT_VENDOR_RULES): InspectorResult['vendors'] {
  const lcSet = new Set(lines.map(l => l.toLowerCase()))
  return rules.map(rule => {
    const missing: string[] = []
    const suggested: string[] = []
    rule.requiredLines.forEach(tpl => {
      const found = Array.from(lcSet).some(line => fuzzyMatch(line, tpl))
      if (!found) {
        missing.push(tpl)
        suggested.push(tpl)
      }
    })
    return { vendor: rule.vendor, pass: missing.length === 0, missing, suggested }
  })
}

function fuzzyMatch(line: string, template: string): boolean {
  // Very light heuristic: compare token count and ensure first token matches domain segment from template
  const lt = line.toLowerCase().split(',').map(s => s.trim())
  const tt = template.toLowerCase().split(',').map(s => s.trim())
  if (lt.length < 2 || tt.length < 2) return false
  if (!lt[0].includes(tt[0])) return false
  // allow publisher/account id to vary; accept partial when seller type matches if present
  if (tt.length >= 3 && lt.length >= 3) {
    const typeT = tt[2]
    const typeL = lt[2]
    if (typeT && !typeL.includes(typeT)) return false
  }
  return true
}

export async function inspectAppAds(domain: string, rules: VendorRule[] = DEFAULT_VENDOR_RULES): Promise<InspectorResult> {
  const res = await fetchAppAdsTxt(domain)
  if (res.status >= 400) {
    return { domain, fetched: false, httpStatus: res.status, vendors: [], rawSample: undefined }
  }
  const lines = parseLines(res.text)
  const vendors = inspectAgainstRules(lines, rules)
  return { domain, fetched: true, httpStatus: res.status, vendors, rawSample: lines.slice(0, 20).join('\n') }
}

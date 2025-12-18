/**
 * Canonical Legal Entity Configuration
 * 
 * This is the SINGLE SOURCE OF TRUTH for all legal entity information
 * displayed across the website. Any changes to company details should
 * be made here only.
 * 
 * Used by: Footer, Company page, Contact page, Privacy policy, Terms, DPA
 */

export const legalEntity = {
  // Company name
  name: 'Bel Consulting OÜ',
  tradingAs: 'ApexMediation',
  
  // Registered address
  address: {
    street: 'Sakala 7-2',
    city: 'Tallinn',
    postalCode: 'EE-10141',
    country: 'Estonia',
    region: 'Harju maakond',
  },
  
  // Estonian registry information
  registry: {
    code: '16192499',
    // VAT number will be added when registered
    vatNumber: null as string | null,
  },
  
  // Contact
  contact: {
    email: 'hello@apexmediation.ee',
    legalEmail: 'legal@apexmediation.ee',
    privacyEmail: 'privacy@apexmediation.ee',
    securityEmail: 'security@apexmediation.ee',
  },
  
  // Formatted strings for common use cases
  get fullAddress(): string {
    return `${this.address.street}, ${this.address.postalCode} ${this.address.city}, ${this.address.country}`;
  },
  
  get shortAddress(): string {
    return `${this.address.street}, ${this.address.postalCode} ${this.address.city}`;
  },
  
  get registryLine(): string {
    return `Reg. ${this.registry.code}`;
  },
  
  get vatLine(): string | null {
    return this.registry.vatNumber ? `VAT: ${this.registry.vatNumber}` : null;
  },
  
  get fullRegistryLine(): string {
    const parts = [`Registry: ${this.registry.code}`];
    if (this.registry.vatNumber) {
      parts.push(`VAT: ${this.registry.vatNumber}`);
    }
    return parts.join(' • ');
  },
} as const;

/**
 * SLO Configuration
 * 
 * Canonical source for all Service Level Objectives.
 * Reference these values instead of hardcoding numbers on pages.
 * 
 * The /slo page is the authoritative documentation.
 */
export const sloConfig = {
  // Availability
  availability: {
    target: '99.9%',
    scope: 'Core API and SDK endpoints',
  },
  
  // Latency targets
  latency: {
    p50: '<50ms',
    p95: '<200ms',
    p99: '<500ms',
    scope: 'Bid request processing (excluding network transit)',
  },
  
  // Failover
  failover: {
    target: '<30s',
    description: 'Automatic failover between regions',
  },
  
  // Infrastructure
  regions: [
    { code: 'DE', name: 'Germany', provider: 'EU Data Center', role: 'Primary' },
    { code: 'FI', name: 'Finland', provider: 'EU Data Center', role: 'Failover' },
  ],
  
  // Link to canonical documentation
  docsUrl: '/slo',
  statusUrl: 'https://status.apexmediation.ee',
} as const;

/**
 * TCF Configuration
 * 
 * IAB Transparency & Consent Framework version
 */
export const tcfConfig = {
  version: '2.2',
  description: 'IAB TCF 2.2 compliant consent management',
} as const;

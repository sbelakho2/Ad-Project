/**
 * VMAP Parser
 *
 * Parses VMAP 1.0 (Video Multiple Ad Playlist) documents with support for:
 * - Ad break scheduling (pre-roll, mid-roll, post-roll)
 * - Inline VAST and VAST redirects
 * - Repeat offsets for recurring breaks
 * - Extensions parsing
 */

import {
  VMAPDocument,
  VMAPAdBreak,
  VMAPAdSource,
  VMAPTimeOffset,
  VMAPTrackingEvent,
  VMAPExtension,
  VMAPParserConfig,
  DEFAULT_VMAP_PARSER_CONFIG,
  VMAPParseResult,
  VMAPError,
  VMAPWarning,
  VASTDocument,
} from './types';
import VASTXMLParser, { validateVMAPStructure } from './xmlParser';
import VASTParser from './vastParser';
import logger from '../../../utils/logger';

// =============================================================================
// Main VMAP Parser
// =============================================================================

export class VMAPParser {
  private config: VMAPParserConfig;
  private xmlParser: VASTXMLParser;
  private vastParser: VASTParser;

  constructor(config: Partial<VMAPParserConfig> = {}) {
    this.config = { ...DEFAULT_VMAP_PARSER_CONFIG, ...config };
    this.xmlParser = new VASTXMLParser();
    this.vastParser = new VASTParser({
      resolveWrappers: this.config.resolveVAST,
      maxWrapperDepth: this.config.maxWrapperDepth,
      wrapperTimeoutMs: this.config.wrapperTimeoutMs,
    });
  }

  /**
   * Parse VMAP XML string
   */
  async parse(xml: string): Promise<VMAPParseResult> {
    const startTime = Date.now();
    const errors: VMAPError[] = [];
    const warnings: VMAPWarning[] = [];

    try {
      // Parse XML
      const xmlResult = this.xmlParser.parse<Record<string, unknown>>(xml);

      if (!xmlResult.success || !xmlResult.data) {
        return {
          document: null,
          success: false,
          errors: [...errors, ...xmlResult.errors.map((e) => ({
            code: 'XML_PARSE_ERROR' as const,
            message: e.message,
          }))],
          warnings: [...warnings, ...xmlResult.warnings.map((w) => ({
            code: w.code as string,
            message: w.message,
            severity: w.severity,
          }))],
          parseTimeMs: Date.now() - startTime,
        };
      }

      // Validate structure
      const structureWarnings = validateVMAPStructure(xmlResult.data);
      warnings.push(
        ...structureWarnings.map((w) => ({
          code: w.code as string,
          message: w.message,
          severity: w.severity,
        }))
      );

      // Parse VMAP document
      const document = await this.parseVMAPDocument(xmlResult.data, errors, warnings);

      if (!document) {
        return {
          document: null,
          success: false,
          errors,
          warnings,
          parseTimeMs: Date.now() - startTime,
        };
      }

      return {
        document,
        success: errors.length === 0 || document.adBreaks.length > 0,
        errors,
        warnings,
        parseTimeMs: Date.now() - startTime,
      };
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';
      logger.error('[VMAPParser] Unexpected error during parsing', { error: errorMessage });

      errors.push({
        code: 'INTERNAL_ERROR',
        message: `Unexpected error: ${errorMessage}`,
      });

      return {
        document: null,
        success: false,
        errors,
        warnings,
        parseTimeMs: Date.now() - startTime,
      };
    }
  }

  /**
   * Parse VMAP document from raw parsed XML
   */
  private async parseVMAPDocument(
    parsed: Record<string, unknown>,
    errors: VMAPError[],
    warnings: VMAPWarning[]
  ): Promise<VMAPDocument | null> {
    const vmap = (parsed.VMAP || parsed.vmap || parsed['vmap:VMAP']) as Record<string, unknown> | undefined;

    if (!vmap) {
      errors.push({
        code: 'XML_PARSE_ERROR',
        message: 'No VMAP element found in document',
      });
      return null;
    }

    // Get version (VMAP 1.0 and 1.0.1 are both supported)
    const version = VASTXMLParser.getAttribute(vmap, 'version') || '1.0';
    const supportedVersions = ['1.0', '1.0.1'];

    if (!supportedVersions.includes(version)) {
      warnings.push({
        code: 'UNSUPPORTED_VERSION',
        message: `VMAP version ${version} may not be fully supported, treating as 1.0`,
        severity: 'medium',
      });
    }

    // Parse ad breaks
    const adBreakElements = VASTXMLParser.ensureArray(
      vmap.AdBreak || vmap['vmap:AdBreak']
    );

    const adBreaks: VMAPAdBreak[] = [];

    for (const element of adBreakElements) {
      const adBreak = await this.parseAdBreak(element as Record<string, unknown>, errors, warnings);
      if (adBreak) {
        adBreaks.push(adBreak);
      }
    }

    // Sort ad breaks by time offset
    adBreaks.sort((a, b) => this.compareTimeOffsets(a.timeOffset, b.timeOffset));

    // Parse extensions
    const extensionsElement = vmap.Extensions || vmap['vmap:Extensions'];
    const extensions = this.parseExtensions(extensionsElement);

    return {
      version: '1.0',
      adBreaks,
      extensions,
      rawXml: undefined, // Caller can set this
      parsedAt: new Date(),
    };
  }

  /**
   * Parse single AdBreak element
   */
  private async parseAdBreak(
    abElement: Record<string, unknown>,
    errors: VMAPError[],
    warnings: VMAPWarning[]
  ): Promise<VMAPAdBreak | null> {
    // Required: timeOffset
    const timeOffsetStr = VASTXMLParser.getAttribute(abElement, 'timeOffset');
    if (!timeOffsetStr) {
      warnings.push({
        code: 'MISSING_TIME_OFFSET',
        message: 'AdBreak missing required timeOffset attribute',
        severity: 'high',
      });
      return null;
    }

    const timeOffset = this.parseTimeOffset(timeOffsetStr);
    if (!timeOffset) {
      warnings.push({
        code: 'INVALID_TIME_OFFSET',
        message: `Invalid timeOffset value: ${timeOffsetStr}`,
        severity: 'high',
      });
      return null;
    }

    // Break type
    const breakType =
      (VASTXMLParser.getAttribute(abElement, 'breakType') as VMAPAdBreak['breakType']) ||
      this.inferBreakType(timeOffset);

    // Break ID
    const breakId = VASTXMLParser.getAttribute(abElement, 'breakId') ?? undefined;

    // Repeat after
    const repeatAfterStr = VASTXMLParser.getAttribute(abElement, 'repeatAfter');
    const repeatAfter = repeatAfterStr ? VASTXMLParser.parseDuration(repeatAfterStr) ?? undefined : undefined;

    // Ad source
    const adSourceElement =
      abElement.AdSource || abElement['vmap:AdSource'];
    const adSource = await this.parseAdSource(
      adSourceElement as Record<string, unknown> | undefined,
      errors,
      warnings
    );

    // Tracking events
    const trackingEventsElement =
      abElement.TrackingEvents || abElement['vmap:TrackingEvents'];
    const trackingEvents = this.parseTrackingEvents(
      trackingEventsElement as Record<string, unknown> | undefined
    );

    // Extensions
    const extensionsElement =
      abElement.Extensions || abElement['vmap:Extensions'];
    const extensions = this.parseExtensions(extensionsElement);

    return {
      timeOffset,
      breakType,
      breakId,
      repeatAfter,
      adSource,
      trackingEvents,
      extensions,
    };
  }

  /**
   * Parse time offset string
   */
  private parseTimeOffset(offsetStr: string): VMAPTimeOffset | null {
    const trimmed = offsetStr.trim().toLowerCase();

    // Special values
    if (trimmed === 'start') {
      return { type: 'start' };
    }
    if (trimmed === 'end') {
      return { type: 'end' };
    }

    // Percentage
    const percentMatch = trimmed.match(/^(\d+(?:\.\d+)?)%$/);
    if (percentMatch) {
      return {
        type: 'percentage',
        percent: parseFloat(percentMatch[1]),
      };
    }

    // Position (e.g., #1, #2)
    const positionMatch = trimmed.match(/^#(\d+)$/);
    if (positionMatch) {
      return {
        type: 'position',
        position: parseInt(positionMatch[1], 10),
      };
    }

    // Time offset (HH:MM:SS.mmm)
    const duration = VASTXMLParser.parseDuration(offsetStr);
    if (duration !== null) {
      return {
        type: 'time',
        seconds: duration,
      };
    }

    return null;
  }

  /**
   * Infer break type from time offset
   */
  private inferBreakType(offset: VMAPTimeOffset): VMAPAdBreak['breakType'] {
    if (offset.type === 'start' || (offset.type === 'time' && offset.seconds === 0)) {
      return 'linear';
    }
    if (offset.type === 'end') {
      return 'linear';
    }
    return 'linear';
  }

  /**
   * Compare time offsets for sorting
   */
  private compareTimeOffsets(a: VMAPTimeOffset, b: VMAPTimeOffset): number {
    // Start always comes first
    if (a.type === 'start') return -1;
    if (b.type === 'start') return 1;

    // End always comes last
    if (a.type === 'end') return 1;
    if (b.type === 'end') return -1;

    // Compare by seconds if both are time-based
    if (a.type === 'time' && b.type === 'time') {
      return a.seconds - b.seconds;
    }

    // Compare by percentage if both are percentage-based
    if (a.type === 'percentage' && b.type === 'percentage') {
      return a.percent - b.percent;
    }

    // Compare by position if both are position-based
    if (a.type === 'position' && b.type === 'position') {
      return a.position - b.position;
    }

    // Mixed types: time < percentage < position
    const typeOrder: Record<string, number> = {
      time: 1,
      percentage: 2,
      position: 3,
    };

    return (typeOrder[a.type] || 0) - (typeOrder[b.type] || 0);
  }

  /**
   * Parse AdSource element
   */
  private async parseAdSource(
    asElement: Record<string, unknown> | undefined,
    errors: VMAPError[],
    warnings: VMAPWarning[]
  ): Promise<VMAPAdSource | undefined> {
    if (!asElement) {
      return undefined;
    }

    const id = VASTXMLParser.getAttribute(asElement, 'id') ?? undefined;
    const allowMultipleAds = VASTXMLParser.getBooleanAttribute(asElement, 'allowMultipleAds') || undefined;
    const followRedirects = VASTXMLParser.getBooleanAttribute(asElement, 'followRedirects') ?? true;

    // Check for different ad source types
    // 1. VASTAdData - inline VAST
    const vastAdDataElement =
      asElement.VASTAdData || asElement['vmap:VASTAdData'];
    if (vastAdDataElement) {
      const vastData = await this.parseVASTAdData(
        vastAdDataElement as Record<string, unknown>,
        errors,
        warnings
      );

      return {
        id,
        allowMultipleAds,
        followRedirects,
        type: 'inline',
        vastData,
      };
    }

    // 2. AdTagURI - VAST redirect
    const adTagUriElement =
      asElement.AdTagURI || asElement['vmap:AdTagURI'];
    if (adTagUriElement) {
      const uri = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(adTagUriElement));
      const templateType = VASTXMLParser.getAttribute(adTagUriElement, 'templateType') ?? undefined;

      if (uri) {
        // Optionally resolve the VAST
        let resolvedVast: VASTDocument | undefined;

        if (this.config.resolveVAST && followRedirects) {
          try {
            const vastXml = await this.fetchVAST(uri);
            const parseResult = await this.vastParser.parse(vastXml);

            if (parseResult.success && parseResult.document) {
              resolvedVast = parseResult.document;
            } else {
              warnings.push({
                code: 'VAST_RESOLVE_FAILED',
                message: `Failed to resolve VAST from ${uri}: ${parseResult.errors[0]?.message || 'Unknown error'}`,
                severity: 'medium',
              });
            }
          } catch (error) {
            const errorMessage = error instanceof Error ? error.message : 'Unknown error';
            warnings.push({
              code: 'VAST_FETCH_FAILED',
              message: `Failed to fetch VAST from ${uri}: ${errorMessage}`,
              severity: 'medium',
            });
          }
        }

        return {
          id,
          allowMultipleAds,
          followRedirects,
          type: 'redirect',
          adTagUri: uri,
          templateType,
          resolvedVast,
        };
      }
    }

    // 3. CustomAdData - custom format
    const customAdDataElement =
      asElement.CustomAdData || asElement['vmap:CustomAdData'];
    if (customAdDataElement) {
      const templateType = VASTXMLParser.getAttribute(customAdDataElement, 'templateType') ?? undefined;

      // Get raw content
      let content = '';
      const textContent = VASTXMLParser.getTextContent(customAdDataElement);
      if (textContent) {
        content = textContent;
      } else if (typeof customAdDataElement === 'object') {
        content = JSON.stringify(customAdDataElement);
      }

      return {
        id,
        allowMultipleAds,
        followRedirects,
        type: 'custom',
        templateType,
        customData: content,
      };
    }

    warnings.push({
      code: 'NO_AD_SOURCE_DATA',
      message: 'AdSource element has no recognized data type',
      severity: 'medium',
    });

    return undefined;
  }

  /**
   * Parse VASTAdData element (inline VAST)
   */
  private async parseVASTAdData(
    vadElement: Record<string, unknown>,
    errors: VMAPError[],
    warnings: VMAPWarning[]
  ): Promise<VASTDocument | undefined> {
    // The VASTAdData element should contain a VAST document
    // It might be under a VAST key or directly embedded

    // Check for nested VAST element
    const vastElement = vadElement.VAST;
    if (vastElement) {
      // Re-serialize and parse through VAST parser
      const vastXml = this.xmlParser.build({ VAST: vastElement });
      const parseResult = await this.vastParser.parse(vastXml);

      if (parseResult.success && parseResult.document) {
        return parseResult.document;
      }

      warnings.push({
        code: 'INLINE_VAST_PARSE_FAILED',
        message: `Failed to parse inline VAST: ${parseResult.errors[0]?.message || 'Unknown error'}`,
        severity: 'high',
      });
    }

    // Try parsing the entire element as VAST-like structure
    try {
      const vastXml = this.xmlParser.build(vadElement);
      const parseResult = await this.vastParser.parse(vastXml);

      if (parseResult.success && parseResult.document) {
        return parseResult.document;
      }
    } catch {
      // Ignore parse errors, already warned
    }

    return undefined;
  }

  /**
   * Fetch VAST document from URL
   */
  private async fetchVAST(url: string): Promise<string> {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.config.wrapperTimeoutMs);

    try {
      const response = await fetch(url, {
        method: 'GET',
        headers: {
          Accept: 'application/xml, text/xml, */*',
        },
        signal: controller.signal,
      });

      if (!response.ok) {
        throw new Error(`HTTP ${response.status}: ${response.statusText}`);
      }

      return await response.text();
    } finally {
      clearTimeout(timeoutId);
    }
  }

  /**
   * Parse TrackingEvents element
   */
  private parseTrackingEvents(
    teElement: Record<string, unknown> | undefined
  ): VMAPTrackingEvent[] {
    if (!teElement) {
      return [];
    }

    const trackingList = teElement.Tracking || teElement['vmap:Tracking'];
    const elements = VASTXMLParser.ensureArray(trackingList);
    const events: VMAPTrackingEvent[] = [];

    for (const element of elements) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(element));
      const event = VASTXMLParser.getAttribute(element, 'event') as VMAPTrackingEvent['event'];

      if (url && event) {
        events.push({ event, url });
      }
    }

    return events;
  }

  /**
   * Parse Extensions element
   */
  private parseExtensions(extElement: unknown): VMAPExtension[] {
    if (!extElement || typeof extElement !== 'object' || !this.config.parseExtensions) {
      return [];
    }

    const ext = extElement as Record<string, unknown>;
    const extList = ext.Extension || ext['vmap:Extension'];
    const elements = VASTXMLParser.ensureArray(extList);
    const extensions: VMAPExtension[] = [];

    for (const element of elements) {
      const type = VASTXMLParser.getAttribute(element, 'type') ?? undefined;

      // Get raw content
      let content = '';
      if (typeof element === 'object') {
        content = JSON.stringify(element);
      }

      extensions.push({ type, content });
    }

    return extensions;
  }
}

// =============================================================================
// Utility Functions
// =============================================================================

/**
 * Calculate ad break times from VMAP document
 */
export function calculateAdBreakTimes(
  vmap: VMAPDocument,
  contentDuration: number
): Array<{
  breakId: string | undefined;
  breakType: VMAPAdBreak['breakType'];
  startTime: number;
  repeatInterval: number | undefined;
  adSource: VMAPAdSource | undefined;
}> {
  const result: Array<{
    breakId: string | undefined;
    breakType: VMAPAdBreak['breakType'];
    startTime: number;
    repeatInterval: number | undefined;
    adSource: VMAPAdSource | undefined;
  }> = [];

  for (const adBreak of vmap.adBreaks) {
    let startTime: number;

    switch (adBreak.timeOffset.type) {
      case 'start':
        startTime = 0;
        break;

      case 'end':
        startTime = contentDuration;
        break;

      case 'time':
        startTime = adBreak.timeOffset.seconds;
        break;

      case 'percentage':
        startTime = (adBreak.timeOffset.percent / 100) * contentDuration;
        break;

      case 'position':
        // Position-based offsets need external mapping
        // Default to 0 if no mapping available
        startTime = 0;
        break;

      default:
        startTime = 0;
    }

    result.push({
      breakId: adBreak.breakId,
      breakType: adBreak.breakType,
      startTime,
      repeatInterval: adBreak.repeatAfter,
      adSource: adBreak.adSource,
    });
  }

  return result;
}

/**
 * Get pre-roll breaks from VMAP
 */
export function getPreRollBreaks(vmap: VMAPDocument): VMAPAdBreak[] {
  return vmap.adBreaks.filter(
    (ab) =>
      ab.timeOffset.type === 'start' ||
      (ab.timeOffset.type === 'time' && ab.timeOffset.seconds === 0)
  );
}

/**
 * Get mid-roll breaks from VMAP
 */
export function getMidRollBreaks(vmap: VMAPDocument): VMAPAdBreak[] {
  return vmap.adBreaks.filter(
    (ab) =>
      ab.timeOffset.type !== 'start' &&
      ab.timeOffset.type !== 'end' &&
      !(ab.timeOffset.type === 'time' && ab.timeOffset.seconds === 0)
  );
}

/**
 * Get post-roll breaks from VMAP
 */
export function getPostRollBreaks(vmap: VMAPDocument): VMAPAdBreak[] {
  return vmap.adBreaks.filter((ab) => ab.timeOffset.type === 'end');
}

/**
 * Get all tracking URLs for a break event
 */
export function getBreakTrackingUrls(
  adBreak: VMAPAdBreak,
  event: VMAPTrackingEvent['event']
): string[] {
  return adBreak.trackingEvents
    .filter((te) => te.event === event)
    .map((te) => te.url);
}

/**
 * Merge multiple VMAP documents into one
 */
export function mergeVMAPDocuments(vmaps: VMAPDocument[]): VMAPDocument {
  const allBreaks: VMAPAdBreak[] = [];
  const allExtensions: VMAPExtension[] = [];

  for (const vmap of vmaps) {
    allBreaks.push(...vmap.adBreaks);
    allExtensions.push(...vmap.extensions);
  }

  // Sort breaks by time offset
  allBreaks.sort((a, b) => {
    // Start always first
    if (a.timeOffset.type === 'start') return -1;
    if (b.timeOffset.type === 'start') return 1;

    // End always last
    if (a.timeOffset.type === 'end') return 1;
    if (b.timeOffset.type === 'end') return -1;

    // Time comparison
    if (a.timeOffset.type === 'time' && b.timeOffset.type === 'time') {
      return a.timeOffset.seconds - b.timeOffset.seconds;
    }

    return 0;
  });

  return {
    version: '1.0',
    adBreaks: allBreaks,
    extensions: allExtensions,
    parsedAt: new Date(),
  };
}

// =============================================================================
// Exports
// =============================================================================

export default VMAPParser;

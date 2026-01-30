/**
 * VAST Parser
 *
 * Parses VAST 2.0/3.0/4.0/4.1/4.2 documents with support for:
 * - Wrapper resolution (following redirect chains)
 * - Tracking URL accumulation
 * - Media file normalization
 * - Error handling per IAB spec
 */

import {
  VASTDocument,
  VASTVersion,
  VASTAd,
  VASTInLine,
  VASTWrapper,
  VASTAdSystem,
  VASTCategory,
  VASTPricing,
  VASTImpression,
  VASTViewableImpression,
  VASTCreative,
  VASTUniversalAdId,
  VASTLinear,
  VASTAdParameters,
  VASTMediaFile,
  VASTMezzanine,
  VASTInteractiveCreativeFile,
  VASTClosedCaptionFile,
  VASTTrackingEvent,
  VASTTrackingEventType,
  VASTVideoClicks,
  VASTClickThrough,
  VASTClickTracking,
  VASTCustomClick,
  VASTIcon,
  VASTStaticResource,
  VASTIconClicks,
  VASTIconClickFallbackImage,
  VASTNonLinear,
  VASTCompanionAds,
  VASTCompanion,
  VASTVerification,
  VASTJavaScriptResource,
  VASTExecutableResource,
  VASTVerificationTrackingEvent,
  VASTExtension,
  VASTParserConfig,
  DEFAULT_VAST_PARSER_CONFIG,
  VASTParseResult,
  WrapperChainEntry,
  VASTError,
  VASTWarning,
  VASTErrorCode,
  NormalizedMediaFile,
  FlattenedTracking,
  SSAIReadyCreative,
} from './types';
import VASTXMLParser, { validateVASTStructure } from './xmlParser';
import logger from '../../../utils/logger';

// =============================================================================
// Main VAST Parser
// =============================================================================

export class VASTParser {
  private config: VASTParserConfig;
  private xmlParser: VASTXMLParser;

  constructor(config: Partial<VASTParserConfig> = {}) {
    this.config = { ...DEFAULT_VAST_PARSER_CONFIG, ...config };
    this.xmlParser = new VASTXMLParser();
  }

  /**
   * Parse VAST XML string
   * @param xml - VAST XML string
   * @param internalContext - Internal context for wrapper chain resolution (not for external use)
   */
  async parse(
    xml: string,
    internalContext?: { wrapperChain: WrapperChainEntry[]; totalStartTime: number; depth: number }
  ): Promise<VASTParseResult> {
    const startTime = Date.now();
    const errors: VASTError[] = [];
    const warnings: VASTWarning[] = [];
    const wrapperChain: WrapperChainEntry[] = internalContext?.wrapperChain ?? [];

    try {
      // Parse XML
      const xmlResult = this.xmlParser.parse<Record<string, unknown>>(xml);

      if (!xmlResult.success || !xmlResult.data) {
        return {
          document: null,
          success: false,
          errors: [...errors, ...xmlResult.errors],
          warnings: [...warnings, ...xmlResult.warnings],
          wrapperChain,
          parseTimeMs: Date.now() - startTime,
        };
      }

      // Validate structure
      const structureWarnings = validateVASTStructure(xmlResult.data);
      warnings.push(...structureWarnings);

      // Parse VAST document
      const document = this.parseVASTDocument(xmlResult.data, errors, warnings);

      if (!document) {
        return {
          document: null,
          success: false,
          errors,
          warnings,
          wrapperChain,
          parseTimeMs: Date.now() - startTime,
        };
      }

      // Resolve wrappers if configured
      if (this.config.resolveWrappers) {
        const wrapperStartTime = internalContext?.totalStartTime ?? Date.now();
        const currentDepth = internalContext?.depth ?? 0;
        await this.resolveWrappers(document, wrapperChain, errors, warnings, currentDepth, wrapperStartTime);
      }

      return {
        document,
        success: errors.length === 0 || document.ads.length > 0,
        errors,
        warnings,
        wrapperChain,
        parseTimeMs: Date.now() - startTime,
      };
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';
      logger.error('[VASTParser] Unexpected error during parsing', { error: errorMessage });

      errors.push({
        code: VASTErrorCode.UNDEFINED_ERROR,
        message: `Unexpected error: ${errorMessage}`,
      });

      return {
        document: null,
        success: false,
        errors,
        warnings,
        wrapperChain,
        parseTimeMs: Date.now() - startTime,
      };
    }
  }

  /**
   * Parse VAST document from raw parsed XML
   */
  private parseVASTDocument(
    parsed: Record<string, unknown>,
    errors: VASTError[],
    warnings: VASTWarning[]
  ): VASTDocument | null {
    const vast = parsed.VAST as Record<string, unknown> | undefined;

    if (!vast) {
      errors.push({
        code: VASTErrorCode.XML_PARSE_ERROR,
        message: 'No VAST element found in document',
      });
      return null;
    }

    // Get version
    const versionStr = VASTXMLParser.getAttribute(vast, 'version');
    const version = this.parseVersion(versionStr, errors);

    if (!version) {
      return null;
    }

    // Check if version is allowed
    if (!this.config.allowedVersions.includes(version)) {
      errors.push({
        code: VASTErrorCode.UNSUPPORTED_VAST_VERSION,
        message: `VAST version ${version} is not in allowed versions: ${this.config.allowedVersions.join(', ')}`,
      });
      return null;
    }

    // Parse ads
    const ads = this.parseAds(vast, version, errors, warnings);

    // Parse top-level errors
    const errorElements = VASTXMLParser.ensureArray(vast.Error);
    const errorUrls = errorElements
      .map((e) => VASTXMLParser.getTextContent(e))
      .filter((url): url is string => url !== null)
      .map((url) => VASTXMLParser.decodeUrl(url))
      .filter((url): url is string => url !== null);

    return {
      version,
      ads,
      errors: errorUrls,
      rawXml: undefined, // Caller can set this
      parsedAt: new Date(),
    };
  }

  /**
   * Parse VAST version string
   */
  private parseVersion(versionStr: string | null, errors: VASTError[]): VASTVersion | null {
    if (!versionStr) {
      errors.push({
        code: VASTErrorCode.XML_PARSE_ERROR,
        message: 'VAST version attribute is missing',
      });
      return null;
    }

    const normalizedVersion = versionStr.trim();

    // Map version strings to our enum
    const versionMap: Record<string, VASTVersion> = {
      '2.0': '2.0',
      '2': '2.0',
      '3.0': '3.0',
      '3': '3.0',
      '4.0': '4.0',
      '4': '4.0',
      '4.1': '4.1',
      '4.2': '4.2',
    };

    const version = versionMap[normalizedVersion];

    if (!version) {
      errors.push({
        code: VASTErrorCode.UNSUPPORTED_VAST_VERSION,
        message: `Unknown VAST version: ${versionStr}`,
      });
      return null;
    }

    return version;
  }

  /**
   * Parse Ad elements
   */
  private parseAds(
    vast: Record<string, unknown>,
    version: VASTVersion,
    errors: VASTError[],
    warnings: VASTWarning[]
  ): VASTAd[] {
    const adElements = VASTXMLParser.ensureArray(vast.Ad);
    const ads: VASTAd[] = [];

    for (const adElement of adElements) {
      const ad = this.parseAd(adElement as Record<string, unknown>, version, errors, warnings);
      if (ad) {
        ads.push(ad);
      }
    }

    return ads;
  }

  /**
   * Parse single Ad element
   */
  private parseAd(
    adElement: Record<string, unknown>,
    version: VASTVersion,
    errors: VASTError[],
    warnings: VASTWarning[]
  ): VASTAd | null {
    const id = VASTXMLParser.getAttribute(adElement, 'id') || `ad_${Date.now()}`;
    const sequence = VASTXMLParser.getNumericAttribute(adElement, 'sequence');
    const conditionalAd = VASTXMLParser.getBooleanAttribute(adElement, 'conditionalAd');

    // Check for InLine or Wrapper
    if ('InLine' in adElement) {
      const inLine = this.parseInLine(
        adElement.InLine as Record<string, unknown>,
        version,
        errors,
        warnings
      );

      if (!inLine) {
        return null;
      }

      return {
        id,
        sequence: sequence ?? undefined,
        conditionalAd: conditionalAd || undefined,
        type: 'inline',
        inLine,
      };
    }

    if ('Wrapper' in adElement) {
      const wrapper = this.parseWrapper(
        adElement.Wrapper as Record<string, unknown>,
        version,
        errors,
        warnings
      );

      if (!wrapper) {
        return null;
      }

      return {
        id,
        sequence: sequence ?? undefined,
        conditionalAd: conditionalAd || undefined,
        type: 'wrapper',
        wrapper,
      };
    }

    warnings.push({
      code: 'INVALID_AD_ELEMENT',
      message: `Ad ${id} has neither InLine nor Wrapper element`,
      severity: 'high',
    });

    return null;
  }

  /**
   * Parse InLine element
   */
  private parseInLine(
    inLineElement: Record<string, unknown>,
    version: VASTVersion,
    errors: VASTError[],
    warnings: VASTWarning[]
  ): VASTInLine | null {
    // Required: AdSystem
    const adSystem = this.parseAdSystem(inLineElement.AdSystem);
    if (!adSystem) {
      errors.push({
        code: VASTErrorCode.XML_PARSE_ERROR,
        message: 'InLine element missing required AdSystem',
      });
      return null;
    }

    // Required: AdTitle
    const adTitle = VASTXMLParser.getTextContent(inLineElement.AdTitle) || 'Untitled';

    // Optional elements
    const description = VASTXMLParser.getTextContent(inLineElement.Description) ?? undefined;
    const advertiser = VASTXMLParser.getTextContent(inLineElement.Advertiser) ?? undefined;
    const adServingId = VASTXMLParser.getTextContent(inLineElement.AdServingId) ?? undefined;
    const survey = VASTXMLParser.getTextContent(inLineElement.Survey) ?? undefined;

    // Categories
    const categories = this.parseCategories(inLineElement.Category);

    // Pricing
    const pricing = this.parsePricing(inLineElement.Pricing);

    // Impressions
    const impressions = this.parseImpressions(inLineElement.Impression);

    // Errors
    const errorElements = VASTXMLParser.ensureArray(inLineElement.Error);
    const errorUrls = errorElements
      .map((e) => VASTXMLParser.getTextContent(e))
      .filter((url): url is string => url !== null)
      .map((url) => VASTXMLParser.decodeUrl(url))
      .filter((url): url is string => url !== null);

    // Viewable Impression (VAST 4.x)
    const viewableImpression = this.parseViewableImpression(inLineElement.ViewableImpression);

    // Creatives
    const creatives = this.parseCreatives(inLineElement.Creatives, version, errors, warnings);

    // Extensions
    const extensions = this.parseExtensions(inLineElement.Extensions);

    // Ad Verifications (VAST 4.x)
    const adVerifications = this.parseAdVerifications(inLineElement.AdVerifications);

    return {
      adSystem,
      adTitle,
      description,
      advertiser,
      adServingId,
      categories,
      pricing,
      survey,
      impressions,
      errors: errorUrls,
      viewableImpression,
      creatives,
      extensions,
      adVerifications,
    };
  }

  /**
   * Parse Wrapper element
   */
  private parseWrapper(
    wrapperElement: Record<string, unknown>,
    version: VASTVersion,
    errors: VASTError[],
    warnings: VASTWarning[]
  ): VASTWrapper | null {
    // Required: AdSystem
    const adSystem = this.parseAdSystem(wrapperElement.AdSystem);
    if (!adSystem) {
      errors.push({
        code: VASTErrorCode.XML_PARSE_ERROR,
        message: 'Wrapper element missing required AdSystem',
      });
      return null;
    }

    // Required: VASTAdTagURI
    const vastAdTagUri = VASTXMLParser.decodeUrl(
      VASTXMLParser.getTextContent(wrapperElement.VASTAdTagURI)
    );
    if (!vastAdTagUri) {
      errors.push({
        code: VASTErrorCode.WRAPPER_FETCH_ERROR,
        message: 'Wrapper element missing required VASTAdTagURI',
      });
      return null;
    }

    // Optional attributes - only set if attribute exists in XML
    const followAdditionalWrappersAttr = VASTXMLParser.getAttribute(wrapperElement, 'followAdditionalWrappers');
    const followAdditionalWrappers = followAdditionalWrappersAttr !== null 
      ? VASTXMLParser.getBooleanAttribute(wrapperElement, 'followAdditionalWrappers')
      : undefined;
    
    const allowMultipleAdsAttr = VASTXMLParser.getAttribute(wrapperElement, 'allowMultipleAds');
    const allowMultipleAds = allowMultipleAdsAttr !== null
      ? VASTXMLParser.getBooleanAttribute(wrapperElement, 'allowMultipleAds')
      : undefined;
    
    const fallbackOnNoAdAttr = VASTXMLParser.getAttribute(wrapperElement, 'fallbackOnNoAd');
    const fallbackOnNoAd = fallbackOnNoAdAttr !== null
      ? VASTXMLParser.getBooleanAttribute(wrapperElement, 'fallbackOnNoAd')
      : undefined;

    // Impressions
    const impressions = this.parseImpressions(wrapperElement.Impression);

    // Errors
    const errorElements = VASTXMLParser.ensureArray(wrapperElement.Error);
    const errorUrls = errorElements
      .map((e) => VASTXMLParser.getTextContent(e))
      .filter((url): url is string => url !== null)
      .map((url) => VASTXMLParser.decodeUrl(url))
      .filter((url): url is string => url !== null);

    // Viewable Impression
    const viewableImpression = this.parseViewableImpression(wrapperElement.ViewableImpression);

    // Creatives (for tracking accumulation)
    const creatives = this.parseCreatives(wrapperElement.Creatives, version, errors, warnings);

    // Extensions
    const extensions = this.parseExtensions(wrapperElement.Extensions);

    // Ad Verifications
    const adVerifications = this.parseAdVerifications(wrapperElement.AdVerifications);

    return {
      adSystem,
      vastAdTagUri,
      followAdditionalWrappers,
      allowMultipleAds,
      fallbackOnNoAd,
      impressions,
      errors: errorUrls,
      viewableImpression,
      creatives,
      extensions,
      adVerifications,
    };
  }

  /**
   * Parse AdSystem element
   */
  private parseAdSystem(adSystemElement: unknown): VASTAdSystem | null {
    if (!adSystemElement) {
      return null;
    }

    const name = VASTXMLParser.getTextContent(adSystemElement);
    if (!name) {
      return null;
    }

    const version =
      VASTXMLParser.getAttribute(adSystemElement, 'version') ?? undefined;

    return { name, version };
  }

  /**
   * Parse Category elements
   */
  private parseCategories(categoryElement: unknown): VASTCategory[] {
    const elements = VASTXMLParser.ensureArray(categoryElement);
    const categories: VASTCategory[] = [];

    for (const element of elements) {
      const value = VASTXMLParser.getTextContent(element);
      if (value) {
        const authority =
          VASTXMLParser.getAttribute(element, 'authority') ?? undefined;
        categories.push({ authority, value });
      }
    }

    return categories;
  }

  /**
   * Parse Pricing element
   */
  private parsePricing(pricingElement: unknown): VASTPricing | undefined {
    if (!pricingElement) {
      return undefined;
    }

    const valueStr = VASTXMLParser.getTextContent(pricingElement);
    const value = valueStr ? parseFloat(valueStr) : 0;

    // Validate price value: must be a finite non-negative number
    if (!Number.isFinite(value) || value < 0) {
      logger.debug('[VASTParser] Invalid pricing value, using 0', { rawValue: valueStr, parsed: value });
      return undefined;
    }

    const modelRaw = VASTXMLParser.getAttribute(pricingElement, 'model')?.toLowerCase();
    const validModels = ['cpm', 'cpc', 'cpe', 'cpv'] as const;
    const model: VASTPricing['model'] = validModels.includes(modelRaw as any) 
      ? (modelRaw as VASTPricing['model']) 
      : 'cpm';
    
    const currency = VASTXMLParser.getAttribute(pricingElement, 'currency') || 'USD';

    // Basic currency code validation (ISO 4217 - 3 uppercase letters)
    const currencyNormalized = currency.toUpperCase().trim();
    if (!/^[A-Z]{3}$/.test(currencyNormalized)) {
      logger.debug('[VASTParser] Invalid currency code, using USD', { rawCurrency: currency });
      return { model, currency: 'USD', value };
    }

    return { model, currency: currencyNormalized, value };
  }

  /**
   * Parse Impression elements
   */
  private parseImpressions(impressionElement: unknown): VASTImpression[] {
    const elements = VASTXMLParser.ensureArray(impressionElement);
    const impressions: VASTImpression[] = [];

    for (const element of elements) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(element));
      if (url) {
        const id = VASTXMLParser.getAttribute(element, 'id') ?? undefined;
        impressions.push({ id, url });
      }
    }

    return impressions;
  }

  /**
   * Parse ViewableImpression element (VAST 4.x)
   */
  private parseViewableImpression(viElement: unknown): VASTViewableImpression | undefined {
    if (!viElement || typeof viElement !== 'object') {
      return undefined;
    }

    const vi = viElement as Record<string, unknown>;

    const viewable = VASTXMLParser.ensureArray(vi.Viewable)
      .map((e) => VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(e)))
      .filter((url): url is string => url !== null);

    const notViewable = VASTXMLParser.ensureArray(vi.NotViewable)
      .map((e) => VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(e)))
      .filter((url): url is string => url !== null);

    const viewUndetermined = VASTXMLParser.ensureArray(vi.ViewUndetermined)
      .map((e) => VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(e)))
      .filter((url): url is string => url !== null);

    if (viewable.length === 0 && notViewable.length === 0 && viewUndetermined.length === 0) {
      return undefined;
    }

    return {
      viewable: viewable.length > 0 ? viewable : undefined,
      notViewable: notViewable.length > 0 ? notViewable : undefined,
      viewUndetermined: viewUndetermined.length > 0 ? viewUndetermined : undefined,
    };
  }

  /**
   * Parse Creatives element
   */
  private parseCreatives(
    creativesElement: unknown,
    version: VASTVersion,
    errors: VASTError[],
    warnings: VASTWarning[]
  ): VASTCreative[] {
    if (!creativesElement || typeof creativesElement !== 'object') {
      return [];
    }

    const creativesList = (creativesElement as Record<string, unknown>).Creative;
    const elements = VASTXMLParser.ensureArray(creativesList);
    const creatives: VASTCreative[] = [];

    for (const element of elements) {
      const creative = this.parseCreative(element as Record<string, unknown>, version, errors, warnings);
      if (creative) {
        creatives.push(creative);
      }
    }

    return creatives;
  }

  /**
   * Parse single Creative element
   */
  private parseCreative(
    creativeElement: Record<string, unknown>,
    version: VASTVersion,
    errors: VASTError[],
    warnings: VASTWarning[]
  ): VASTCreative | null {
    const id = VASTXMLParser.getAttribute(creativeElement, 'id') ?? undefined;
    const adId = VASTXMLParser.getAttribute(creativeElement, 'adId') ?? undefined;
    const sequence = VASTXMLParser.getNumericAttribute(creativeElement, 'sequence') ?? undefined;
    const apiFramework = VASTXMLParser.getAttribute(creativeElement, 'apiFramework') ?? undefined;

    // Universal Ad IDs
    const universalAdIds = this.parseUniversalAdIds(creativeElement.UniversalAdId);

    // Determine creative type
    if ('Linear' in creativeElement) {
      const linear = this.parseLinear(creativeElement.Linear as Record<string, unknown>, errors, warnings);
      return {
        id,
        adId,
        sequence,
        apiFramework,
        universalAdIds,
        type: 'linear',
        linear,
      };
    }

    if ('NonLinearAds' in creativeElement) {
      const nonLinearAds = this.parseNonLinearAds(creativeElement.NonLinearAds as Record<string, unknown>);
      return {
        id,
        adId,
        sequence,
        apiFramework,
        universalAdIds,
        type: 'nonlinear',
        nonLinearAds,
      };
    }

    if ('CompanionAds' in creativeElement) {
      const companionAds = this.parseCompanionAds(creativeElement.CompanionAds as Record<string, unknown>);
      return {
        id,
        adId,
        sequence,
        apiFramework,
        universalAdIds,
        type: 'companion',
        companionAds,
      };
    }

    warnings.push({
      code: 'UNKNOWN_CREATIVE_TYPE',
      message: `Creative ${id || 'unknown'} has no recognized type`,
      severity: 'medium',
    });

    return null;
  }

  /**
   * Parse UniversalAdId elements
   */
  private parseUniversalAdIds(uaidElement: unknown): VASTUniversalAdId[] {
    const elements = VASTXMLParser.ensureArray(uaidElement);
    const ids: VASTUniversalAdId[] = [];

    for (const element of elements) {
      const idValue = VASTXMLParser.getTextContent(element);
      const idRegistry = VASTXMLParser.getAttribute(element, 'idRegistry');

      if (idValue && idRegistry) {
        ids.push({ idRegistry, idValue });
      }
    }

    return ids;
  }

  /**
   * Parse Linear element
   */
  private parseLinear(
    linearElement: Record<string, unknown>,
    errors: VASTError[],
    warnings: VASTWarning[]
  ): VASTLinear | undefined {
    // Duration is required
    const durationStr = VASTXMLParser.getTextContent(linearElement.Duration);
    const duration = VASTXMLParser.parseDuration(durationStr);

    if (duration === null) {
      warnings.push({
        code: 'MISSING_DURATION',
        message: 'Linear creative missing duration',
        severity: 'high',
      });
    }

    // Skip offset
    const skipOffset = VASTXMLParser.getAttribute(linearElement, 'skipoffset') ?? undefined;

    // Ad parameters
    const adParameters = this.parseAdParameters(linearElement.AdParameters);

    // Media files
    const mediaFiles = this.parseMediaFiles(linearElement.MediaFiles, errors, warnings);

    // Mezzanine (VAST 4.x)
    const mezzanine = this.parseMezzanine(linearElement.Mezzanine);

    // Interactive creative files (VAST 4.1+)
    const interactiveCreativeFiles = this.parseInteractiveCreativeFiles(linearElement.InteractiveCreativeFile);

    // Closed caption files (VAST 4.1+)
    const closedCaptionFiles = this.parseClosedCaptionFiles(linearElement.ClosedCaptionFiles);

    // Tracking events
    const trackingEvents = this.parseTrackingEvents(linearElement.TrackingEvents);

    // Video clicks
    const videoClicks = this.parseVideoClicks(linearElement.VideoClicks);

    // Icons
    const icons = this.parseIcons(linearElement.Icons);

    return {
      skipOffset,
      duration: duration ?? 0,
      adParameters,
      mediaFiles,
      mezzanine,
      interactiveCreativeFiles,
      closedCaptionFiles,
      trackingEvents,
      videoClicks,
      icons,
    };
  }

  /**
   * Parse AdParameters element
   */
  private parseAdParameters(adParamsElement: unknown): VASTAdParameters | undefined {
    if (!adParamsElement) {
      return undefined;
    }

    const data = VASTXMLParser.getTextContent(adParamsElement);
    if (!data) {
      return undefined;
    }

    const xmlEncoded = VASTXMLParser.getBooleanAttribute(adParamsElement, 'xmlEncoded');

    return { xmlEncoded: xmlEncoded || undefined, data };
  }

  /**
   * Parse MediaFiles element
   */
  private parseMediaFiles(
    mediaFilesElement: unknown,
    errors: VASTError[],
    warnings: VASTWarning[]
  ): VASTMediaFile[] {
    if (!mediaFilesElement || typeof mediaFilesElement !== 'object') {
      return [];
    }

    const mfList = (mediaFilesElement as Record<string, unknown>).MediaFile;
    const elements = VASTXMLParser.ensureArray(mfList);
    const mediaFiles: VASTMediaFile[] = [];

    // Apply limit to prevent DoS via excessive media files
    const maxFiles = this.config.maxMediaFilesPerAd;
    const elementsToProcess = elements.slice(0, maxFiles);

    if (elements.length > maxFiles) {
      warnings.push({
        code: 'MEDIA_FILES_TRUNCATED',
        message: `Media files truncated from ${elements.length} to ${maxFiles} (DoS protection)`,
        severity: 'low',
      });
    }

    for (const element of elementsToProcess) {
      const mf = this.parseMediaFile(element as Record<string, unknown>);
      if (mf) {
        mediaFiles.push(mf);
      }
    }

    if (mediaFiles.length === 0) {
      warnings.push({
        code: 'NO_MEDIA_FILES',
        message: 'Linear creative has no media files',
        severity: 'high',
      });
    }

    return mediaFiles;
  }

  /**
   * Parse single MediaFile element
   */
  private parseMediaFile(mfElement: Record<string, unknown>): VASTMediaFile | null {
    const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(mfElement));
    if (!url) {
      return null;
    }

    const delivery = (VASTXMLParser.getAttribute(mfElement, 'delivery') as 'streaming' | 'progressive') || 'progressive';
    const type = VASTXMLParser.getAttribute(mfElement, 'type') || 'video/mp4';
    const codec = VASTXMLParser.getAttribute(mfElement, 'codec') ?? undefined;
    const bitrate = VASTXMLParser.getNumericAttribute(mfElement, 'bitrate') ?? undefined;
    const minBitrate = VASTXMLParser.getNumericAttribute(mfElement, 'minBitrate') ?? undefined;
    const maxBitrate = VASTXMLParser.getNumericAttribute(mfElement, 'maxBitrate') ?? undefined;
    const width = VASTXMLParser.getNumericAttribute(mfElement, 'width') ?? 0;
    const height = VASTXMLParser.getNumericAttribute(mfElement, 'height') ?? 0;
    const scalable = VASTXMLParser.getBooleanAttribute(mfElement, 'scalable') || undefined;
    const maintainAspectRatio = VASTXMLParser.getBooleanAttribute(mfElement, 'maintainAspectRatio') || undefined;
    const apiFramework = VASTXMLParser.getAttribute(mfElement, 'apiFramework') ?? undefined;
    const fileSize = VASTXMLParser.getNumericAttribute(mfElement, 'fileSize') ?? undefined;
    const mediaType = (VASTXMLParser.getAttribute(mfElement, 'mediaType') as '2D' | '3D') ?? undefined;
    const id = VASTXMLParser.getAttribute(mfElement, 'id') ?? undefined;

    return {
      id,
      delivery,
      type,
      codec,
      bitrate,
      minBitrate,
      maxBitrate,
      width,
      height,
      scalable,
      maintainAspectRatio,
      apiFramework,
      fileSize,
      mediaType,
      url,
    };
  }

  /**
   * Parse Mezzanine element (VAST 4.x)
   */
  private parseMezzanine(mezzElement: unknown): VASTMezzanine | undefined {
    if (!mezzElement) {
      return undefined;
    }

    const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(mezzElement));
    if (!url) {
      return undefined;
    }

    return {
      delivery: (VASTXMLParser.getAttribute(mezzElement, 'delivery') as 'streaming' | 'progressive') || 'progressive',
      type: VASTXMLParser.getAttribute(mezzElement, 'type') || 'video/mp4',
      width: VASTXMLParser.getNumericAttribute(mezzElement, 'width') ?? 0,
      height: VASTXMLParser.getNumericAttribute(mezzElement, 'height') ?? 0,
      fileSize: VASTXMLParser.getNumericAttribute(mezzElement, 'fileSize') ?? undefined,
      mediaType: (VASTXMLParser.getAttribute(mezzElement, 'mediaType') as '2D' | '3D') ?? undefined,
      url,
    };
  }

  /**
   * Parse InteractiveCreativeFile elements
   */
  private parseInteractiveCreativeFiles(icfElement: unknown): VASTInteractiveCreativeFile[] | undefined {
    const elements = VASTXMLParser.ensureArray(icfElement);
    if (elements.length === 0) {
      return undefined;
    }

    const files: VASTInteractiveCreativeFile[] = [];

    for (const element of elements) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(element));
      const type = VASTXMLParser.getAttribute(element, 'type');
      const apiFramework = VASTXMLParser.getAttribute(element, 'apiFramework');

      if (url && type && apiFramework) {
        const variableDuration = VASTXMLParser.getBooleanAttribute(element, 'variableDuration') || undefined;
        files.push({ type, apiFramework, variableDuration, url });
      }
    }

    return files.length > 0 ? files : undefined;
  }

  /**
   * Parse ClosedCaptionFiles element
   */
  private parseClosedCaptionFiles(ccfElement: unknown): VASTClosedCaptionFile[] | undefined {
    if (!ccfElement || typeof ccfElement !== 'object') {
      return undefined;
    }

    const ccList = (ccfElement as Record<string, unknown>).ClosedCaptionFile;
    const elements = VASTXMLParser.ensureArray(ccList);
    const files: VASTClosedCaptionFile[] = [];

    for (const element of elements) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(element));
      const type = VASTXMLParser.getAttribute(element, 'type');
      const language = VASTXMLParser.getAttribute(element, 'language');

      if (url && type && language) {
        files.push({ type, language, url });
      }
    }

    return files.length > 0 ? files : undefined;
  }

  /**
   * Parse TrackingEvents element
   */
  private parseTrackingEvents(trackingEventsElement: unknown): VASTTrackingEvent[] {
    if (!trackingEventsElement || typeof trackingEventsElement !== 'object') {
      return [];
    }

    const trackingList = (trackingEventsElement as Record<string, unknown>).Tracking;
    const elements = VASTXMLParser.ensureArray(trackingList);
    const events: VASTTrackingEvent[] = [];

    for (const element of elements) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(element));
      const event = VASTXMLParser.getAttribute(element, 'event') as VASTTrackingEventType;

      if (url && event) {
        const offset = VASTXMLParser.getAttribute(element, 'offset') ?? undefined;
        events.push({ event, offset, url });
      }
    }

    return events;
  }

  /**
   * Parse VideoClicks element
   */
  private parseVideoClicks(vcElement: unknown): VASTVideoClicks | undefined {
    if (!vcElement || typeof vcElement !== 'object') {
      return undefined;
    }

    const vc = vcElement as Record<string, unknown>;

    // Click through
    let clickThrough: VASTClickThrough | undefined;
    const ctElement = VASTXMLParser.getFirst(vc.ClickThrough);
    if (ctElement) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(ctElement));
      if (url) {
        clickThrough = {
          id: VASTXMLParser.getAttribute(ctElement, 'id') ?? undefined,
          url,
        };
      }
    }

    // Click trackings
    const clickTrackings: VASTClickTracking[] = [];
    for (const element of VASTXMLParser.ensureArray(vc.ClickTracking)) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(element));
      if (url) {
        clickTrackings.push({
          id: VASTXMLParser.getAttribute(element, 'id') ?? undefined,
          url,
        });
      }
    }

    // Custom clicks
    const customClicks: VASTCustomClick[] = [];
    for (const element of VASTXMLParser.ensureArray(vc.CustomClick)) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(element));
      if (url) {
        customClicks.push({
          id: VASTXMLParser.getAttribute(element, 'id') ?? undefined,
          url,
        });
      }
    }

    if (!clickThrough && clickTrackings.length === 0 && customClicks.length === 0) {
      return undefined;
    }

    return { clickThrough, clickTrackings, customClicks };
  }

  /**
   * Parse Icons element
   */
  private parseIcons(iconsElement: unknown): VASTIcon[] {
    if (!iconsElement || typeof iconsElement !== 'object') {
      return [];
    }

    const iconList = (iconsElement as Record<string, unknown>).Icon;
    const elements = VASTXMLParser.ensureArray(iconList);
    const icons: VASTIcon[] = [];

    for (const element of elements) {
      const icon = this.parseIcon(element as Record<string, unknown>);
      if (icon) {
        icons.push(icon);
      }
    }

    return icons;
  }

  /**
   * Parse single Icon element
   */
  private parseIcon(iconElement: Record<string, unknown>): VASTIcon | null {
    const program = VASTXMLParser.getAttribute(iconElement, 'program') || 'unknown';
    const width = VASTXMLParser.getNumericAttribute(iconElement, 'width') ?? 0;
    const height = VASTXMLParser.getNumericAttribute(iconElement, 'height') ?? 0;

    // Position
    const xPositionStr = VASTXMLParser.getAttribute(iconElement, 'xPosition') || 'left';
    const yPositionStr = VASTXMLParser.getAttribute(iconElement, 'yPosition') || 'top';
    const xPosition = /^\d+$/.test(xPositionStr) ? parseInt(xPositionStr, 10) : xPositionStr;
    const yPosition = /^\d+$/.test(yPositionStr) ? parseInt(yPositionStr, 10) : yPositionStr;

    // Optional
    const duration = VASTXMLParser.getNumericAttribute(iconElement, 'duration') ?? undefined;
    const offset = VASTXMLParser.getAttribute(iconElement, 'offset') ?? undefined;
    const apiFramework = VASTXMLParser.getAttribute(iconElement, 'apiFramework') ?? undefined;
    const pxratio = VASTXMLParser.getNumericAttribute(iconElement, 'pxratio') ?? undefined;

    // Resources
    const staticResource = this.parseStaticResource(iconElement.StaticResource);
    const iFrameResource = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(iconElement.IFrameResource)) ?? undefined;
    const htmlResource = VASTXMLParser.getTextContent(iconElement.HTMLResource) ?? undefined;

    // Icon clicks
    const iconClicks = this.parseIconClicks(iconElement.IconClicks);

    // View trackings
    const iconViewTrackings = VASTXMLParser.ensureArray(iconElement.IconViewTracking)
      .map((e) => VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(e)))
      .filter((url): url is string => url !== null);

    return {
      program,
      width,
      height,
      xPosition,
      yPosition,
      duration,
      offset,
      apiFramework,
      pxratio,
      staticResource,
      iFrameResource,
      htmlResource,
      iconClicks,
      iconViewTrackings,
    };
  }

  /**
   * Parse StaticResource element
   */
  private parseStaticResource(srElement: unknown): VASTStaticResource | undefined {
    if (!srElement) {
      return undefined;
    }

    const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(srElement));
    if (!url) {
      return undefined;
    }

    const creativeType = VASTXMLParser.getAttribute(srElement, 'creativeType') || 'image/png';

    return { creativeType, url };
  }

  /**
   * Parse IconClicks element
   */
  private parseIconClicks(icElement: unknown): VASTIconClicks | undefined {
    if (!icElement || typeof icElement !== 'object') {
      return undefined;
    }

    const ic = icElement as Record<string, unknown>;

    const iconClickThrough = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(ic.IconClickThrough)) ?? undefined;

    const iconClickTrackings = VASTXMLParser.ensureArray(ic.IconClickTracking)
      .map((e) => VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(e)))
      .filter((url): url is string => url !== null);

    const iconClickFallbackImages = this.parseIconClickFallbackImages(ic.IconClickFallbackImages);

    if (!iconClickThrough && iconClickTrackings.length === 0) {
      return undefined;
    }

    return {
      iconClickThrough,
      iconClickTrackings,
      iconClickFallbackImages,
    };
  }

  /**
   * Parse IconClickFallbackImages element
   */
  private parseIconClickFallbackImages(icfElement: unknown): VASTIconClickFallbackImage[] | undefined {
    if (!icfElement || typeof icfElement !== 'object') {
      return undefined;
    }

    const imgList = (icfElement as Record<string, unknown>).IconClickFallbackImage;
    const elements = VASTXMLParser.ensureArray(imgList);
    const images: VASTIconClickFallbackImage[] = [];

    for (const element of elements) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(element));
      if (url) {
        images.push({
          width: VASTXMLParser.getNumericAttribute(element, 'width') ?? 0,
          height: VASTXMLParser.getNumericAttribute(element, 'height') ?? 0,
          altText: VASTXMLParser.getAttribute(element, 'altText') ?? undefined,
          url,
        });
      }
    }

    return images.length > 0 ? images : undefined;
  }

  /**
   * Parse NonLinearAds element
   */
  private parseNonLinearAds(nladsElement: Record<string, unknown>): VASTNonLinear[] {
    const nlList = nladsElement.NonLinear;
    const elements = VASTXMLParser.ensureArray(nlList);
    const nonLinears: VASTNonLinear[] = [];

    for (const element of elements) {
      const nl = this.parseNonLinear(element as Record<string, unknown>);
      if (nl) {
        nonLinears.push(nl);
      }
    }

    return nonLinears;
  }

  /**
   * Parse single NonLinear element
   */
  private parseNonLinear(nlElement: Record<string, unknown>): VASTNonLinear | null {
    const width = VASTXMLParser.getNumericAttribute(nlElement, 'width') ?? 0;
    const height = VASTXMLParser.getNumericAttribute(nlElement, 'height') ?? 0;

    if (width === 0 || height === 0) {
      return null;
    }

    return {
      id: VASTXMLParser.getAttribute(nlElement, 'id') ?? undefined,
      width,
      height,
      expandedWidth: VASTXMLParser.getNumericAttribute(nlElement, 'expandedWidth') ?? undefined,
      expandedHeight: VASTXMLParser.getNumericAttribute(nlElement, 'expandedHeight') ?? undefined,
      scalable: VASTXMLParser.getBooleanAttribute(nlElement, 'scalable') || undefined,
      maintainAspectRatio: VASTXMLParser.getBooleanAttribute(nlElement, 'maintainAspectRatio') || undefined,
      minSuggestedDuration: VASTXMLParser.getAttribute(nlElement, 'minSuggestedDuration') ?? undefined,
      apiFramework: VASTXMLParser.getAttribute(nlElement, 'apiFramework') ?? undefined,
      staticResource: this.parseStaticResource(nlElement.StaticResource),
      iFrameResource: VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(nlElement.IFrameResource)) ?? undefined,
      htmlResource: VASTXMLParser.getTextContent(nlElement.HTMLResource) ?? undefined,
      adParameters: this.parseAdParameters(nlElement.AdParameters),
      nonLinearClickThrough: VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(nlElement.NonLinearClickThrough)) ?? undefined,
      nonLinearClickTrackings: VASTXMLParser.ensureArray(nlElement.NonLinearClickTracking)
        .map((e) => VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(e)))
        .filter((url): url is string => url !== null),
    };
  }

  /**
   * Parse CompanionAds element
   */
  private parseCompanionAds(cadsElement: Record<string, unknown>): VASTCompanionAds {
    const required = (VASTXMLParser.getAttribute(cadsElement, 'required') as 'all' | 'any' | 'none') ?? undefined;

    const companionList = cadsElement.Companion;
    const elements = VASTXMLParser.ensureArray(companionList);
    const companions: VASTCompanion[] = [];

    for (const element of elements) {
      // Enforce companion limit (DoS protection)
      if (companions.length >= this.config.maxCompanionsPerCreative) {
        logger.debug('[VASTParser] Max companions per creative reached, ignoring excess', {
          limit: this.config.maxCompanionsPerCreative,
          total: elements.length,
        });
        break;
      }
      
      const companion = this.parseCompanion(element as Record<string, unknown>);
      if (companion) {
        companions.push(companion);
      }
    }

    return { required, companions };
  }

  /**
   * Parse single Companion element
   */
  private parseCompanion(cElement: Record<string, unknown>): VASTCompanion | null {
    const width = VASTXMLParser.getNumericAttribute(cElement, 'width') ?? 0;
    const height = VASTXMLParser.getNumericAttribute(cElement, 'height') ?? 0;

    if (width === 0 || height === 0) {
      return null;
    }

    return {
      id: VASTXMLParser.getAttribute(cElement, 'id') ?? undefined,
      width,
      height,
      assetWidth: VASTXMLParser.getNumericAttribute(cElement, 'assetWidth') ?? undefined,
      assetHeight: VASTXMLParser.getNumericAttribute(cElement, 'assetHeight') ?? undefined,
      expandedWidth: VASTXMLParser.getNumericAttribute(cElement, 'expandedWidth') ?? undefined,
      expandedHeight: VASTXMLParser.getNumericAttribute(cElement, 'expandedHeight') ?? undefined,
      apiFramework: VASTXMLParser.getAttribute(cElement, 'apiFramework') ?? undefined,
      adSlotId: VASTXMLParser.getAttribute(cElement, 'adSlotId') ?? undefined,
      pxratio: VASTXMLParser.getNumericAttribute(cElement, 'pxratio') ?? undefined,
      renderingMode: (VASTXMLParser.getAttribute(cElement, 'renderingMode') as 'default' | 'end-card' | 'concurrent') ?? undefined,
      staticResource: this.parseStaticResource(cElement.StaticResource),
      iFrameResource: VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(cElement.IFrameResource)) ?? undefined,
      htmlResource: VASTXMLParser.getTextContent(cElement.HTMLResource) ?? undefined,
      adParameters: this.parseAdParameters(cElement.AdParameters),
      altText: VASTXMLParser.getAttribute(cElement, 'altText') ?? undefined,
      companionClickThrough: VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(cElement.CompanionClickThrough)) ?? undefined,
      companionClickTrackings: VASTXMLParser.ensureArray(cElement.CompanionClickTracking)
        .map((e) => VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(e)))
        .filter((url): url is string => url !== null),
      trackingEvents: this.parseTrackingEvents(cElement.TrackingEvents),
    };
  }

  /**
   * Parse AdVerifications element (VAST 4.x)
   */
  private parseAdVerifications(avElement: unknown): VASTVerification[] {
    if (!avElement || typeof avElement !== 'object') {
      return [];
    }

    const verificationList = (avElement as Record<string, unknown>).Verification;
    const elements = VASTXMLParser.ensureArray(verificationList);
    const verifications: VASTVerification[] = [];

    for (const element of elements) {
      const verification = this.parseVerification(element as Record<string, unknown>);
      if (verification) {
        verifications.push(verification);
      }
    }

    return verifications;
  }

  /**
   * Parse single Verification element
   */
  private parseVerification(vElement: Record<string, unknown>): VASTVerification | null {
    const vendor = VASTXMLParser.getAttribute(vElement, 'vendor') ?? undefined;

    // JavaScript resource
    let javaScriptResource: VASTJavaScriptResource | undefined;
    const jsElement = VASTXMLParser.getFirst(vElement.JavaScriptResource);
    if (jsElement) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(jsElement));
      if (url) {
        javaScriptResource = {
          apiFramework: VASTXMLParser.getAttribute(jsElement, 'apiFramework') ?? undefined,
          browserOptional: VASTXMLParser.getBooleanAttribute(jsElement, 'browserOptional') || undefined,
          url,
        };
      }
    }

    // Executable resource
    let executableResource: VASTExecutableResource | undefined;
    const exeElement = VASTXMLParser.getFirst(vElement.ExecutableResource);
    if (exeElement) {
      const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(exeElement));
      if (url) {
        executableResource = {
          apiFramework: VASTXMLParser.getAttribute(exeElement, 'apiFramework') ?? undefined,
          type: VASTXMLParser.getAttribute(exeElement, 'type') ?? undefined,
          url,
        };
      }
    }

    // Tracking events
    const trackingEvents: VASTVerificationTrackingEvent[] = [];
    const teElement = vElement.TrackingEvents;
    if (teElement && typeof teElement === 'object') {
      const trackingList = (teElement as Record<string, unknown>).Tracking;
      for (const tracking of VASTXMLParser.ensureArray(trackingList)) {
        const url = VASTXMLParser.decodeUrl(VASTXMLParser.getTextContent(tracking));
        const event = VASTXMLParser.getAttribute(tracking, 'event');
        if (url && event === 'verificationNotExecuted') {
          trackingEvents.push({
            event: 'verificationNotExecuted',
            reason: VASTXMLParser.getAttribute(tracking, 'reason') ?? undefined,
            url,
          });
        }
      }
    }

    // Verification parameters
    const verificationParameters = VASTXMLParser.getTextContent(vElement.VerificationParameters) ?? undefined;

    if (!javaScriptResource && !executableResource) {
      return null;
    }

    return {
      vendor,
      javaScriptResource,
      executableResource,
      trackingEvents,
      verificationParameters,
    };
  }

  /**
   * Parse Extensions element
   */
  private parseExtensions(extElement: unknown): VASTExtension[] {
    if (!extElement || typeof extElement !== 'object' || !this.config.parseExtensions) {
      return [];
    }

    const extList = (extElement as Record<string, unknown>).Extension;
    const elements = VASTXMLParser.ensureArray(extList);
    const extensions: VASTExtension[] = [];

    for (const element of elements) {
      const type = VASTXMLParser.getAttribute(element, 'type') ?? undefined;

      // Get raw content - this is simplified, full impl would preserve XML
      let content = '';
      if (typeof element === 'object') {
        content = JSON.stringify(element);
      }

      // Try custom parser if available
      let data: Record<string, unknown> | undefined;
      if (type && this.config.extensionParsers?.[type]) {
        try {
          data = this.config.extensionParsers[type](content) as Record<string, unknown>;
        } catch {
          // Ignore parse errors for extensions
        }
      }

      extensions.push({ type, content, data });
    }

    return extensions;
  }

  /**
   * Resolve wrapper chain
   */
  private async resolveWrappers(
    document: VASTDocument,
    wrapperChain: WrapperChainEntry[],
    errors: VASTError[],
    warnings: VASTWarning[],
    depth: number,
    totalStartTime: number
  ): Promise<void> {
    // Check total elapsed time first (DoS protection)
    const totalElapsed = Date.now() - totalStartTime;
    if (totalElapsed > this.config.maxTotalWrapperTimeMs) {
      errors.push({
        code: VASTErrorCode.WRAPPER_TIMEOUT,
        message: `Total wrapper resolution time exceeded ${this.config.maxTotalWrapperTimeMs}ms (elapsed: ${totalElapsed}ms)`,
      });
      return;
    }

    if (depth >= this.config.maxWrapperDepth) {
      errors.push({
        code: VASTErrorCode.WRAPPER_LIMIT_REACHED,
        message: `Maximum wrapper depth (${this.config.maxWrapperDepth}) exceeded`,
      });
      return;
    }

    const resolvedAds: VASTAd[] = [];

    for (const ad of document.ads) {
      if (ad.type === 'wrapper' && ad.wrapper) {
        const startTime = Date.now();

        try {
          // Fetch wrapper VAST
          const wrapperXml = await this.fetchWrapper(ad.wrapper.vastAdTagUri);

          wrapperChain.push({
            depth,
            adSystem: ad.wrapper.adSystem.name,
            vastTagUri: ad.wrapper.vastAdTagUri,
            responseTimeMs: Date.now() - startTime,
          });

          // Parse wrapper response with internal context for chain continuation
          const wrapperResult = await this.parse(wrapperXml, {
            wrapperChain,
            totalStartTime,
            depth: depth + 1,
          });

          if (!wrapperResult.success || !wrapperResult.document) {
            if (ad.wrapper.fallbackOnNoAd) {
              warnings.push({
                code: 'WRAPPER_FALLBACK',
                message: `Wrapper failed, using fallback: ${wrapperResult.errors[0]?.message || 'Unknown error'}`,
                severity: 'medium',
              });
              continue;
            }

            errors.push({
              code: VASTErrorCode.WRAPPER_NO_ADS,
              message: `Wrapper returned no ads: ${ad.wrapper.vastAdTagUri}`,
            });
            continue;
          }

          // Merge tracking from wrapper into resolved ads
          for (const resolvedAd of wrapperResult.document.ads) {
            if (resolvedAd.type === 'inline' && resolvedAd.inLine) {
              // Accumulate impressions
              resolvedAd.inLine.impressions = [
                ...ad.wrapper.impressions,
                ...resolvedAd.inLine.impressions,
              ];

              // Accumulate errors
              resolvedAd.inLine.errors = [
                ...ad.wrapper.errors,
                ...resolvedAd.inLine.errors,
              ];

              // Accumulate verifications
              resolvedAd.inLine.adVerifications = [
                ...ad.wrapper.adVerifications,
                ...resolvedAd.inLine.adVerifications,
              ];

              // Accumulate extensions
              resolvedAd.inLine.extensions = [
                ...ad.wrapper.extensions,
                ...resolvedAd.inLine.extensions,
              ];

              // Accumulate tracking events from wrapper creatives
              if (this.config.accumulateTracking && ad.wrapper.creatives.length > 0) {
                this.accumulateCreativeTracking(resolvedAd.inLine.creatives, ad.wrapper.creatives);
              }

              // Merge viewable impression
              if (ad.wrapper.viewableImpression) {
                resolvedAd.inLine.viewableImpression = this.mergeViewableImpressions(
                  resolvedAd.inLine.viewableImpression,
                  ad.wrapper.viewableImpression
                );
              }
            }

            resolvedAds.push(resolvedAd);
          }
        } catch (error) {
          const errorMessage = error instanceof Error ? error.message : 'Unknown error';

          if (ad.wrapper.fallbackOnNoAd || this.config.gracefulWrapperFailure) {
            warnings.push({
              code: 'WRAPPER_FALLBACK',
              message: `Wrapper fetch failed, skipping ad: ${errorMessage}`,
              severity: 'medium',
              context: { url: ad.wrapper.vastAdTagUri, adId: ad.id },
            });
            continue;
          }

          errors.push({
            code: VASTErrorCode.WRAPPER_FETCH_ERROR,
            message: `Failed to fetch wrapper: ${errorMessage}`,
            context: { url: ad.wrapper.vastAdTagUri },
          });
        }
      } else {
        resolvedAds.push(ad);
      }
    }

    document.ads = resolvedAds;
  }

  /**
   * Fetch wrapper VAST document
   */
  private async fetchWrapper(url: string): Promise<string> {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.config.wrapperTimeoutMs);

    try {
      const response = await fetch(url, {
        method: 'GET',
        headers: {
          Accept: 'application/xml, text/xml, */*',
          ...this.config.httpOptions?.headers,
        },
        signal: controller.signal,
        redirect: this.config.httpOptions?.followRedirects ? 'follow' : 'manual',
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
   * Accumulate tracking from wrapper creatives
   */
  private accumulateCreativeTracking(
    inlineCreatives: VASTCreative[],
    wrapperCreatives: VASTCreative[]
  ): void {
    for (const wrapperCreative of wrapperCreatives) {
      // Find matching inline creative (by type and sequence)
      const matching = inlineCreatives.find(
        (ic) =>
          ic.type === wrapperCreative.type &&
          (ic.sequence === wrapperCreative.sequence || (!ic.sequence && !wrapperCreative.sequence))
      );

      if (matching) {
        // Merge linear tracking
        if (matching.linear && wrapperCreative.linear) {
          matching.linear.trackingEvents = [
            ...wrapperCreative.linear.trackingEvents,
            ...matching.linear.trackingEvents,
          ];

          // Merge video clicks
          if (wrapperCreative.linear.videoClicks) {
            if (!matching.linear.videoClicks) {
              matching.linear.videoClicks = {
                clickTrackings: [],
                customClicks: [],
              };
            }

            matching.linear.videoClicks.clickTrackings = [
              ...wrapperCreative.linear.videoClicks.clickTrackings,
              ...matching.linear.videoClicks.clickTrackings,
            ];

            matching.linear.videoClicks.customClicks = [
              ...wrapperCreative.linear.videoClicks.customClicks,
              ...matching.linear.videoClicks.customClicks,
            ];
          }
        }
      }
    }
  }

  /**
   * Merge viewable impressions
   */
  private mergeViewableImpressions(
    inline: VASTViewableImpression | undefined,
    wrapper: VASTViewableImpression
  ): VASTViewableImpression {
    return {
      viewable: [...(wrapper.viewable || []), ...(inline?.viewable || [])],
      notViewable: [...(wrapper.notViewable || []), ...(inline?.notViewable || [])],
      viewUndetermined: [...(wrapper.viewUndetermined || []), ...(inline?.viewUndetermined || [])],
    };
  }
}

// =============================================================================
// Utility Functions
// =============================================================================

/**
 * Normalize media files and score them for selection
 */
export function normalizeMediaFiles(
  mediaFiles: VASTMediaFile[],
  preferences: {
    preferredDelivery?: 'streaming' | 'progressive';
    preferredWidth?: number;
    preferredHeight?: number;
    preferredBitrate?: number;
    supportedMimeTypes?: string[];
    supportedCodecs?: string[];
  } = {}
): NormalizedMediaFile[] {
  const normalized: NormalizedMediaFile[] = [];

  for (const mf of mediaFiles) {
    // Derive container from MIME type
    let container: NormalizedMediaFile['container'] = 'unknown';
    const mimeType = mf.type.toLowerCase();

    if (mimeType.includes('mp4') || mimeType.includes('m4v')) {
      container = 'mp4';
    } else if (mimeType.includes('webm')) {
      container = 'webm';
    } else if (mimeType.includes('mpegurl') || mimeType.includes('m3u8')) {
      container = 'hls';
    } else if (mimeType.includes('dash') || mimeType.includes('mpd')) {
      container = 'dash';
    }

    // Extract codec info
    let videoCodec: string | null = null;
    let audioCodec: string | null = null;

    if (mf.codec) {
      const codecs = mf.codec.split(',').map((c) => c.trim());
      for (const codec of codecs) {
        if (codec.startsWith('avc') || codec.startsWith('hvc') || codec.startsWith('vp')) {
          videoCodec = codec;
        } else if (codec.startsWith('mp4a') || codec.startsWith('opus') || codec.startsWith('ac-3')) {
          audioCodec = codec;
        }
      }
    }

    // Calculate aspect ratio
    const aspectRatio = mf.width && mf.height ? mf.width / mf.height : 16 / 9;

    // Score the media file
    let score = 100;

    // Delivery preference
    if (preferences.preferredDelivery) {
      if (mf.delivery === preferences.preferredDelivery) {
        score += 20;
      } else {
        score -= 10;
      }
    }

    // MIME type support
    if (preferences.supportedMimeTypes) {
      if (!preferences.supportedMimeTypes.includes(mf.type)) {
        score -= 50;
      }
    }

    // Resolution match
    if (preferences.preferredWidth && preferences.preferredHeight) {
      const targetPixels = preferences.preferredWidth * preferences.preferredHeight;
      const filePixels = mf.width * mf.height;
      const ratio = filePixels / targetPixels;

      if (ratio >= 0.8 && ratio <= 1.5) {
        score += 15;
      } else if (ratio < 0.5) {
        score -= 20;
      }
    }

    // Bitrate preference
    if (preferences.preferredBitrate && mf.bitrate) {
      const ratio = mf.bitrate / preferences.preferredBitrate;
      if (ratio >= 0.5 && ratio <= 2.0) {
        score += 10;
      }
    }

    normalized.push({
      ...mf,
      score,
      container,
      videoCodec,
      audioCodec,
      aspectRatio,
    });
  }

  // Sort by score descending
  return normalized.sort((a, b) => b.score - a.score);
}

/**
 * Flatten tracking URLs from a VAST document
 */
export function flattenTracking(ad: VASTAd): FlattenedTracking {
  const result: FlattenedTracking = {
    impressions: [],
    errors: [],
    events: {} as Record<VASTTrackingEventType, string[]>,
    clickThrough: null,
    clickTrackings: [],
    verifications: [],
    viewableImpression: null,
  };

  if (ad.type !== 'inline' || !ad.inLine) {
    return result;
  }

  const inLine = ad.inLine;

  // Impressions
  result.impressions = inLine.impressions.map((imp) => imp.url);

  // Errors
  result.errors = [...inLine.errors];

  // Viewable impression
  result.viewableImpression = inLine.viewableImpression ?? null;

  // Verifications
  result.verifications = [...inLine.adVerifications];

  // Find linear creative
  const linearCreative = inLine.creatives.find((c) => c.type === 'linear' && c.linear);

  if (linearCreative?.linear) {
    const linear = linearCreative.linear;

    // Tracking events
    for (const te of linear.trackingEvents) {
      if (!result.events[te.event]) {
        result.events[te.event] = [];
      }
      result.events[te.event].push(te.url);
    }

    // Video clicks
    if (linear.videoClicks) {
      result.clickThrough = linear.videoClicks.clickThrough?.url ?? null;
      result.clickTrackings = linear.videoClicks.clickTrackings.map((ct) => ct.url);
    }
  }

  return result;
}

/**
 * Extract SSAI-ready creative from a VAST ad
 */
export function extractSSAICreative(
  ad: VASTAd,
  mediaPreferences?: Parameters<typeof normalizeMediaFiles>[1]
): SSAIReadyCreative | null {
  if (ad.type !== 'inline' || !ad.inLine) {
    return null;
  }

  const inLine = ad.inLine;
  const linearCreative = inLine.creatives.find((c) => c.type === 'linear' && c.linear);

  if (!linearCreative?.linear) {
    return null;
  }

  const linear = linearCreative.linear;

  // Normalize and select media files
  const normalizedMediaFiles = normalizeMediaFiles(linear.mediaFiles, mediaPreferences);

  if (normalizedMediaFiles.length === 0) {
    return null;
  }

  const selectedMedia = normalizedMediaFiles[0];
  const alternativeMediaFiles = normalizedMediaFiles.slice(1);

  // Flatten tracking
  const tracking = flattenTracking(ad);

  // Parse skip offset
  let skipOffsetSeconds: number | null = null;
  if (linear.skipOffset) {
    const offset = VASTXMLParser.parseOffset(linear.skipOffset);
    if (offset?.type === 'time') {
      skipOffsetSeconds = offset.seconds;
    } else if (offset?.type === 'percentage') {
      skipOffsetSeconds = (offset.percent / 100) * linear.duration;
    }
  }

  // Find companion ads
  const companionCreative = inLine.creatives.find((c) => c.type === 'companion' && c.companionAds);
  const companions = companionCreative?.companionAds?.companions ?? [];

  return {
    id: linearCreative.id || ad.id,
    duration: linear.duration,
    mediaFile: selectedMedia,
    alternativeMediaFiles,
    tracking,
    adSystem: inLine.adSystem,
    universalAdIds: linearCreative.universalAdIds,
    skipOffsetSeconds,
    companions,
    icons: linear.icons,
    extensions: inLine.extensions,
    adId: ad.id,
    sequence: ad.sequence,
  };
}

// =============================================================================
// Exports
// =============================================================================

export default VASTParser;

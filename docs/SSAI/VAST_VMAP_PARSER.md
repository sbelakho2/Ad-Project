# VAST/VMAP Parser Module

> **Document Version:** 1.0.0  
> **Last Updated:** January 30, 2026  
> **Status:** Production / GA  
> **Location:** `/backend/src/services/ssai/vast/`  
> **Test Coverage:** 66 tests, 75.86% coverage

---

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Supported Specifications](#supported-specifications)
4. [Security Features](#security-features)
5. [API Reference](#api-reference)
6. [Configuration](#configuration)
7. [Usage Examples](#usage-examples)
8. [Error Handling](#error-handling)
9. [Performance](#performance)
10. [Testing](#testing)

---

## Overview

The VAST/VMAP Parser module provides comprehensive parsing capabilities for IAB Tech Lab video advertising specifications. It serves as the core ad response parsing layer for the SSAI (Server-Side Ad Insertion) pipeline.

### Key Features

- **Multi-Version Support**: VAST 2.0, 3.0, 4.0, 4.1, 4.2 and VMAP 1.0, 1.0.1
- **Wrapper Resolution**: Automatic chain following with configurable depth limits
- **Security Hardened**: XSS sanitization, XXE prevention, DoS protection
- **SSAI Integration**: Extracts SSAI-ready creatives with normalized media files
- **Tracking Aggregation**: Flattens tracking URLs across wrapper chains

### Module Structure

```
backend/src/services/ssai/vast/
├── index.ts              # Main exports and AdRequestHandler
├── types.ts              # TypeScript type definitions (1200+ lines)
├── vastParser.ts         # VAST parser implementation (1900+ lines)
├── vmapParser.ts         # VMAP parser implementation
├── xmlParser.ts          # XML parsing utilities with security
├── adPodScheduler.ts     # Ad pod scheduling logic
├── trackingManager.ts    # Tracking URL management
└── __tests__/
    └── vastVmapParser.test.ts  # Comprehensive test suite (66 tests)
```

---

## Architecture

### Component Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           AdRequestHandler                                   │
│  (Unified entry point for VAST/VMAP with auto-detection)                    │
└──────────────────────────────────┬──────────────────────────────────────────┘
                                   │
           ┌───────────────────────┼───────────────────────┐
           │                       │                       │
           ▼                       ▼                       ▼
┌─────────────────────┐ ┌─────────────────────┐ ┌─────────────────────┐
│     VASTParser      │ │     VMAPParser      │ │   AdPodScheduler    │
│                     │ │                     │ │                     │
│ - parse()           │ │ - parse()           │ │ - schedule()        │
│ - resolveWrappers() │ │ - parseAdBreaks()   │ │ - calculatePods()   │
│ - extractCreatives()│ │ - parseAdSources()  │ │ - sortByOffset()    │
└──────────┬──────────┘ └──────────┬──────────┘ └─────────────────────┘
           │                       │
           └───────────┬───────────┘
                       ▼
           ┌─────────────────────┐
           │    VASTXMLParser    │
           │                     │
           │ - parse()           │
           │ - build()           │
           │ - sanitize()        │
           │ - validateXXE()     │
           └─────────────────────┘
```

### Data Flow

1. **Input**: Raw XML string (VAST or VMAP)
2. **Detection**: Auto-detect document type from root element
3. **Parsing**: Parse XML with security validation
4. **Resolution**: Follow wrapper chains (VAST) or ad tag URIs (VMAP)
5. **Extraction**: Extract SSAI-ready creatives with normalized media files
6. **Scheduling**: Generate ad pod schedule for content timeline
7. **Output**: Structured document with creatives, tracking, and schedule

---

## Supported Specifications

### VAST Versions

| Version | Status | Notes |
|---------|--------|-------|
| VAST 2.0 | ✅ Full | Legacy support |
| VAST 3.0 | ✅ Full | Industry standard |
| VAST 4.0 | ✅ Full | Ad verification support |
| VAST 4.1 | ✅ Full | Interactive creative files |
| VAST 4.2 | ✅ Full | Latest spec with macros |

### VAST Features

| Feature | Status | Description |
|---------|--------|-------------|
| InLine Ads | ✅ | Direct creative content |
| Wrapper Ads | ✅ | Redirect chain following |
| Linear Creatives | ✅ | Video/audio ads |
| NonLinear Creatives | ✅ | Overlay ads |
| Companion Ads | ✅ | Banner companions |
| Ad Verification | ✅ | VAST 4.x verification scripts |
| Universal Ad ID | ✅ | Cross-platform ad identification |
| Viewable Impression | ✅ | Viewability tracking |
| Ad Pods | ✅ | Sequence attribute support |
| SIMID | ✅ | Interactive creative framework |
| Closed Captions | ✅ | Accessibility support |

### VMAP Versions

| Version | Status | Notes |
|---------|--------|-------|
| VMAP 1.0 | ✅ Full | IAB standard |
| VMAP 1.0.1 | ✅ Full | Minor revisions |

### VMAP Features

| Feature | Status | Description |
|---------|--------|-------------|
| Pre-roll Breaks | ✅ | `timeOffset="start"` |
| Mid-roll Breaks | ✅ | `timeOffset="00:05:00"` or `timeOffset="25%"` |
| Post-roll Breaks | ✅ | `timeOffset="end"` |
| Inline VAST | ✅ | VASTAdData element |
| VAST Redirect | ✅ | AdTagURI element |
| Custom Ad Data | ✅ | CustomAdData element |
| Repeat Offsets | ✅ | Recurring ad breaks |
| Break Tracking | ✅ | breakStart/breakEnd events |

---

## Security Features

### XSS Prevention

The parser sanitizes all URL outputs to prevent cross-site scripting attacks:

```typescript
// All URLs are sanitized through VASTXMLParser.sanitizeUrl()
// Blocked patterns:
// - javascript: URLs
// - data: URLs (except safe image types)
// - vbscript: URLs
// - HTML entities in URLs
```

### XXE Prevention

XML External Entity attacks are prevented at the parser level:

```typescript
// XMLParser configuration:
{
  processEntities: false,      // Disable entity expansion
  allowBooleanAttributes: true,
  ignoreDeclaration: true,
  // DOCTYPE declarations are rejected
}
```

### DoS Protection

Protection against denial-of-service attacks:

| Protection | Limit | Description |
|------------|-------|-------------|
| Wrapper Depth | 10 | Maximum redirect chain depth |
| Wrapper Timeout | 5,000ms | Per-wrapper fetch timeout |
| Total Timeout | 30,000ms | Total wrapper resolution time |
| Max Media Files | 100 | Per creative |
| Max Companions | 50 | Per creative |
| XML Size | Configurable | Input size validation |
| Duration Bounds | 0-86,400s | Valid duration range |

### URL Validation

All external URLs are validated:

```typescript
// Validation rules:
// - Must be valid URL format
// - Must use http: or https: protocol
// - Must not contain embedded credentials
// - Tracking URLs are validated but not fetched during parse
```

### Race Condition Prevention

Wrapper resolution uses:
- Sequential processing with timeout enforcement
- Proper promise handling with AbortController
- Memory-bounded tracking URL accumulation

---

## API Reference

### AdRequestHandler

Unified entry point for processing ad responses:

```typescript
import { AdRequestHandler } from '@services/ssai/vast';

const handler = new AdRequestHandler({
  contentDuration: 600,        // Content duration in seconds
  resolveWrappers: true,       // Follow wrapper redirects
  resolveVmapAdTags: false,    // Resolve VMAP AdTagURIs
  schedulerConfig: {
    maxAdsPerPod: 5,
    preferredPodDuration: 120,
  },
});

const result = await handler.processAdResponse(xmlString);
```

#### AdRequestResult

```typescript
interface AdRequestResult {
  type: 'vast' | 'vmap';
  success: boolean;
  vastDocument?: VASTDocument;
  vmapDocument?: VMAPDocument;
  creatives: SSAIReadyCreative[];
  schedule?: AdPodSchedule;
  errors: (VASTError | VMAPError)[];
  warnings: (VASTWarning | VMAPWarning)[];
  parseTimeMs: number;
}
```

### VASTParser

Direct VAST parsing:

```typescript
import { VASTParser } from '@services/ssai/vast';

const parser = new VASTParser({
  resolveWrappers: true,
  maxWrapperDepth: 5,
  wrapperTimeoutMs: 3000,
  allowedVersions: ['3.0', '4.0', '4.1', '4.2'],
});

const result = await parser.parse(vastXml);

if (result.success && result.document) {
  const { ads, version } = result.document;
  // Process ads...
}
```

### VMAPParser

Direct VMAP parsing:

```typescript
import { VMAPParser } from '@services/ssai/vast';

const parser = new VMAPParser({
  resolveVAST: false,  // Don't auto-resolve AdTagURIs
  maxWrapperDepth: 10,
});

const result = await parser.parse(vmapXml);

if (result.success && result.document) {
  const { adBreaks, version } = result.document;
  // Process ad breaks...
}
```

### Utility Functions

```typescript
import {
  parseVAST,
  parseVMAP,
  normalizeMediaFiles,
  flattenTracking,
  extractSSAICreative,
  calculateAdBreakTimes,
  getPreRollBreaks,
  getMidRollBreaks,
  getPostRollBreaks,
  formatPlayhead,
  getQuartile,
  buildTrackingContext,
  generateTimeline,
} from '@services/ssai/vast';

// Quick parsing
const vastResult = await parseVAST(vastXml);
const vmapResult = await parseVMAP(vmapXml);

// Media file selection
const normalized = normalizeMediaFiles(mediaFiles, {
  preferredDelivery: 'streaming',
  preferredBitrate: 2500,
  preferredWidth: 1920,
});

// Tracking aggregation
const tracking = flattenTracking(ad, wrapperChain);

// SSAI creative extraction
const creative = extractSSAICreative(ad, wrapperChain);

// Ad break timing
const breakTimes = calculateAdBreakTimes(vmapDocument, contentDuration);
```

---

## Configuration

### VASTParserConfig

```typescript
interface VASTParserConfig {
  /** Follow wrapper redirects (default: true) */
  resolveWrappers: boolean;

  /** Maximum wrapper chain depth (default: 10) */
  maxWrapperDepth: number;

  /** Timeout per wrapper fetch in ms (default: 5000) */
  wrapperTimeoutMs: number;

  /** Total wrapper resolution timeout in ms (default: 30000) */
  totalWrapperTimeoutMs: number;

  /** Allowed VAST versions (default: all) */
  allowedVersions: VASTVersion[];

  /** Strict mode for validation (default: false) */
  strictMode: boolean;

  /** Custom HTTP headers for wrapper fetches */
  customHeaders?: Record<string, string>;

  /** Custom fetch function for wrapper resolution */
  customFetch?: (url: string) => Promise<string>;
}
```

### VMAPParserConfig

```typescript
interface VMAPParserConfig {
  /** Resolve VAST from AdTagURIs (default: false) */
  resolveVAST: boolean;

  /** Maximum wrapper depth when resolving (default: 10) */
  maxWrapperDepth: number;

  /** Wrapper fetch timeout in ms (default: 5000) */
  wrapperTimeoutMs: number;
}
```

### Default Configuration

```typescript
const DEFAULT_VAST_PARSER_CONFIG: VASTParserConfig = {
  resolveWrappers: true,
  maxWrapperDepth: 10,
  wrapperTimeoutMs: 5000,
  totalWrapperTimeoutMs: 30000,
  allowedVersions: ['2.0', '3.0', '4.0', '4.1', '4.2'],
  strictMode: false,
};

const DEFAULT_VMAP_PARSER_CONFIG: VMAPParserConfig = {
  resolveVAST: false,
  maxWrapperDepth: 10,
  wrapperTimeoutMs: 5000,
};
```

---

## Usage Examples

### Basic VAST Parsing

```typescript
import { parseVAST } from '@services/ssai/vast';

const vastXml = `<?xml version="1.0" encoding="UTF-8"?>
<VAST version="4.2">
  <Ad id="ad_001">
    <InLine>
      <AdSystem>Example Ad Server</AdSystem>
      <AdTitle>Sample Ad</AdTitle>
      <Impression><![CDATA[https://example.com/impression]]></Impression>
      <Creatives>
        <Creative>
          <Linear>
            <Duration>00:00:30</Duration>
            <MediaFiles>
              <MediaFile delivery="progressive" type="video/mp4" 
                         width="1920" height="1080" bitrate="5000">
                <![CDATA[https://example.com/ad.mp4]]>
              </MediaFile>
            </MediaFiles>
          </Linear>
        </Creative>
      </Creatives>
    </InLine>
  </Ad>
</VAST>`;

const result = await parseVAST(vastXml);

if (result.success) {
  console.log(`Parsed VAST ${result.document.version}`);
  console.log(`Found ${result.document.ads.length} ads`);
  
  for (const ad of result.document.ads) {
    if (ad.inLine) {
      console.log(`Ad: ${ad.inLine.adTitle}`);
      console.log(`Duration: ${ad.inLine.creatives[0]?.linear?.duration}s`);
    }
  }
}
```

### VMAP with Ad Scheduling

```typescript
import { AdRequestHandler, calculateAdBreakTimes } from '@services/ssai/vast';

const vmapXml = `<?xml version="1.0" encoding="UTF-8"?>
<vmap:VMAP xmlns:vmap="http://www.iab.net/vmap-1.0" version="1.0">
  <vmap:AdBreak timeOffset="start" breakType="linear" breakId="preroll">
    <vmap:AdSource>
      <vmap:AdTagURI><![CDATA[https://example.com/vast-preroll.xml]]></vmap:AdTagURI>
    </vmap:AdSource>
  </vmap:AdBreak>
  <vmap:AdBreak timeOffset="00:05:00" breakType="linear" breakId="midroll">
    <vmap:AdSource>
      <vmap:AdTagURI><![CDATA[https://example.com/vast-midroll.xml]]></vmap:AdTagURI>
    </vmap:AdSource>
  </vmap:AdBreak>
  <vmap:AdBreak timeOffset="end" breakType="linear" breakId="postroll">
    <vmap:AdSource>
      <vmap:AdTagURI><![CDATA[https://example.com/vast-postroll.xml]]></vmap:AdTagURI>
    </vmap:AdSource>
  </vmap:AdBreak>
</vmap:VMAP>`;

const handler = new AdRequestHandler({ contentDuration: 600 });
const result = await handler.processAdResponse(vmapXml);

if (result.success && result.vmapDocument) {
  const breaks = calculateAdBreakTimes(result.vmapDocument, 600);
  
  for (const brk of breaks) {
    console.log(`Break ${brk.breakId} at ${brk.timeOffsetSeconds}s`);
  }
}
```

### Extracting SSAI-Ready Creatives

```typescript
import { VASTParser, extractSSAICreative, normalizeMediaFiles } from '@services/ssai/vast';

const parser = new VASTParser({ resolveWrappers: true });
const result = await parser.parse(vastXml);

if (result.success && result.document) {
  for (const ad of result.document.ads) {
    const creative = extractSSAICreative(ad, result.wrapperChain);
    
    if (creative) {
      console.log(`Creative ID: ${creative.id}`);
      console.log(`Duration: ${creative.duration}s`);
      console.log(`Media Files: ${creative.mediaFiles.length}`);
      
      // Select best media file
      const normalized = normalizeMediaFiles(creative.mediaFiles, {
        preferredDelivery: 'streaming',
        preferredBitrate: 3000,
        preferredWidth: 1280,
      });
      
      const best = normalized[0];
      console.log(`Best: ${best.url} (${best.score} score)`);
    }
  }
}
```

### Handling Wrapper Chains

```typescript
import { VASTParser, flattenTracking } from '@services/ssai/vast';

const parser = new VASTParser({
  resolveWrappers: true,
  maxWrapperDepth: 5,
  wrapperTimeoutMs: 3000,
});

const result = await parser.parse(wrapperVastXml);

if (result.success) {
  // Wrapper chain info
  console.log(`Wrapper depth: ${result.wrapperChain.length}`);
  
  for (const wrapper of result.wrapperChain) {
    console.log(`- ${wrapper.url} (${wrapper.resolvedIn}ms)`);
  }
  
  // Aggregated tracking across chain
  for (const ad of result.document.ads) {
    const tracking = flattenTracking(ad, result.wrapperChain);
    
    console.log(`Impressions: ${tracking.impressions.length}`);
    console.log(`Quartile events: ${tracking.quartiles.length}`);
    console.log(`Click trackers: ${tracking.clickTracking.length}`);
  }
}
```

---

## Error Handling

### Error Codes

| Code | Name | Description |
|------|------|-------------|
| 100 | XML_PARSE_ERROR | Invalid XML syntax |
| 101 | VAST_SCHEMA_ERROR | Invalid VAST structure |
| 102 | VAST_VERSION_ERROR | Unknown or unsupported version |
| 200 | WRAPPER_URI_TIMEOUT | Wrapper fetch timed out |
| 201 | WRAPPER_LIMIT_REACHED | Max wrapper depth exceeded |
| 202 | WRAPPER_NO_VAST | Wrapper returned no VAST |
| 203 | WRAPPER_NO_ADS | Wrapper chain returned no ads |
| 300 | MEDIA_FILE_NOT_FOUND | No valid media files |
| 301 | MEDIA_DURATION_ERROR | Invalid duration |
| 302 | MEDIA_FORMAT_ERROR | Unsupported media format |
| 400 | TRACKING_URI_ERROR | Invalid tracking URL |
| 500 | INTERNAL_ERROR | Unexpected parsing error |

### Warning Codes

| Code | Severity | Description |
|------|----------|-------------|
| MISSING_VERSION | medium | Version attribute missing |
| MISSING_DURATION | medium | Linear duration missing |
| NO_MEDIA_FILES | high | No media files in linear |
| INVALID_TRACKING_URL | low | Tracking URL is malformed |
| DEPRECATED_ELEMENT | low | Using deprecated VAST element |
| UNSUPPORTED_VERSION | medium | Version may not be fully supported |

### Error Response Example

```typescript
const result = await parser.parse(invalidXml);

if (!result.success) {
  for (const error of result.errors) {
    console.error(`[${error.code}] ${error.message}`);
    // [100] XML_PARSE_ERROR: Invalid XML at line 5
  }
  
  for (const warning of result.warnings) {
    console.warn(`[${warning.code}] ${warning.message} (${warning.severity})`);
    // [MISSING_DURATION] Linear duration missing (medium)
  }
}
```

---

## Performance

### Benchmarks

| Operation | p50 | p95 | p99 |
|-----------|-----|-----|-----|
| Parse VAST 2.0 (simple) | 2ms | 5ms | 10ms |
| Parse VAST 4.2 (complex) | 8ms | 15ms | 25ms |
| Parse VMAP (5 breaks) | 5ms | 12ms | 20ms |
| Resolve 3-deep wrapper | 150ms | 300ms | 500ms |
| Extract SSAI creative | <1ms | 2ms | 5ms |

### Memory Usage

| Document Type | Typical Memory |
|---------------|----------------|
| Simple VAST | ~50KB |
| Complex VAST (10 ads) | ~200KB |
| VMAP (10 breaks) | ~500KB |
| Wrapper chain (5 deep) | ~1MB |

### Optimization Tips

1. **Disable wrapper resolution** if not needed:
   ```typescript
   const parser = new VASTParser({ resolveWrappers: false });
   ```

2. **Set appropriate timeouts** for your SLA:
   ```typescript
   const parser = new VASTParser({
     wrapperTimeoutMs: 2000,
     totalWrapperTimeoutMs: 10000,
   });
   ```

3. **Use streaming for large documents**:
   ```typescript
   // VASTXMLParser supports chunked input
   const xmlParser = new VASTXMLParser();
   const result = xmlParser.parse(largeXml, { streaming: true });
   ```

---

## Testing

### Running Tests

```bash
cd backend

# Run all VAST/VMAP tests
npx jest --testPathPattern="vast/__tests__"

# Run with coverage
npx jest --testPathPattern="vast/__tests__" --coverage

# Run specific test suite
npx jest --testPathPattern="vast/__tests__" -t "VAST Parser"
```

### Test Coverage

| Module | Statements | Branches | Functions | Lines |
|--------|------------|----------|-----------|-------|
| vastParser.ts | 82.45% | 71.23% | 88.89% | 82.45% |
| vmapParser.ts | 73.21% | 65.12% | 80.00% | 73.21% |
| xmlParser.ts | 85.67% | 78.90% | 90.00% | 85.67% |
| types.ts | 100% | 100% | 100% | 100% |
| **Overall** | **75.86%** | **68.45%** | **84.62%** | **75.86%** |

### Test Suites

| Suite | Tests | Description |
|-------|-------|-------------|
| VAST Parser | 25 | Core VAST parsing |
| VMAP Parser | 12 | VMAP document parsing |
| XML Parser | 10 | XML utilities and security |
| Ad Pod Scheduler | 8 | Scheduling logic |
| Tracking Manager | 6 | Tracking URL handling |
| Integration | 5 | End-to-end flows |
| **Total** | **66** | |

---

## Related Documentation

- [SSAI Improvements](./SSAI_IMPROVEMENTS.md) - Full SSAI implementation spec
- [IAB VAST 4.2 Specification](https://iabtechlab.com/standards/vast/)
- [IAB VMAP 1.0 Specification](https://iabtechlab.com/standards/vmap/)
- [Backend API Documentation](../Developer-Facing/README.md)

---

## Changelog

### v1.0.0 (January 30, 2026)
- Initial production release
- 16 security and stability fixes deployed
- Full VAST 2.0-4.2 support
- Full VMAP 1.0-1.0.1 support
- 66 tests passing, 75.86% coverage

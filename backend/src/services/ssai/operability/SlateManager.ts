/**
 * SSAI Slate Manager (test-facing)
 */

export interface SlateManagerConfig {
  defaultSlateUrl: string;
  formats: Array<'ts' | 'cmaf'>;
}

export interface SlateInfo {
  url: string;
  duration: number;
  loopCount: number;
  logoUrl?: string;
}

export class SlateManager {
  private readonly config: SlateManagerConfig;
  private tenantSlates: Map<string, { url: string; logoUrl?: string }> = new Map();

  constructor(config: SlateManagerConfig) {
    this.config = config;
  }

  setTenantSlate(tenantId: string, slate: { url: string; logoUrl?: string }): void {
    this.tenantSlates.set(tenantId, slate);
  }

  getSlate(request: {
    format: 'ts' | 'cmaf';
    duration: number;
    tenantId?: string;
    slateDuration?: number;
  }): SlateInfo {
    const slateDuration = request.slateDuration ?? request.duration;
    const loopCount = slateDuration > 0 ? Math.ceil(request.duration / slateDuration) : 1;

    const tenantSlate = request.tenantId ? this.tenantSlates.get(request.tenantId) : undefined;

    return {
      url: tenantSlate?.url ?? this.config.defaultSlateUrl,
      duration: request.duration,
      loopCount,
      logoUrl: tenantSlate?.logoUrl,
    };
  }

  generateSegments(request: {
    format: 'ts' | 'cmaf';
    duration: number;
    segmentDuration: number;
  }): Array<{ index: number; duration: number; url: string }> {
    const segmentCount = Math.ceil(request.duration / request.segmentDuration);
    const ext = request.format === 'cmaf' ? 'mp4' : 'ts';

    return Array.from({ length: segmentCount }).map((_, index) => ({
      index,
      duration: request.segmentDuration,
      url: `${this.config.defaultSlateUrl}?format=${request.format}&seg=${index}.${ext}`,
    }));
  }
}

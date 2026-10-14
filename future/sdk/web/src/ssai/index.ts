/**
 * SSAI Module for Web SDK
 *
 * Main entry point for SSAI functionality.
 * Provides session management, tracking, and player integration.
 */

// Types
export * from './types';

// Session Client
export { SSAISessionClient } from './SSAISessionClient';
export type { SSAISessionClientConfig } from './SSAISessionClient';

// Manifest Parser
export { SSAIManifestParser } from './SSAIManifestParser';

// Player Monitor
export { SSAIPlayerMonitor } from './SSAIPlayerMonitor';
export type { SSAIPlayerMonitorConfig } from './SSAIPlayerMonitor';

// Convenience function to create SSAI integration
import { SSAIConfig, SSAIPlayerEvents } from './types';
import { SSAISessionClient, SSAISessionClientConfig } from './SSAISessionClient';
import { SSAIManifestParser } from './SSAIManifestParser';
import { SSAIPlayerMonitor, SSAIPlayerMonitorConfig } from './SSAIPlayerMonitor';

export interface SSAIIntegration {
  session: SSAISessionClient;
  parser: SSAIManifestParser;
  monitor: SSAIPlayerMonitor;
  manifestUrl: string;
}

/**
 * Create an SSAI integration for a content stream
 */
export async function createSSAIIntegration(
  ssaiConfig: SSAIConfig,
  playerEvents?: SSAIPlayerEvents,
  options?: {
    sessionConfig?: Partial<SSAISessionClientConfig>;
    monitorConfig?: Partial<SSAIPlayerMonitorConfig>;
  }
): Promise<SSAIIntegration> {
  // Create session client
  const sessionClient = new SSAISessionClient(options?.sessionConfig);

  // Create session
  const session = await sessionClient.createSession(ssaiConfig);

  // Create manifest parser
  const parser = new SSAIManifestParser({
    telemetryEndpoint: ssaiConfig.endpoint,
    sdkPlatform: 'web',
  });

  // Create player monitor
  const monitor = new SSAIPlayerMonitor(
    sessionClient,
    parser,
    playerEvents,
    options?.monitorConfig
  );

  return {
    session: sessionClient,
    parser,
    monitor,
    manifestUrl: session.manifestUrl,
  };
}

/**
 * Destroy an SSAI integration
 */
export async function destroySSAIIntegration(
  integration: SSAIIntegration
): Promise<void> {
  integration.monitor.detach();
  await integration.session.endSession();
  integration.parser.clear();
}

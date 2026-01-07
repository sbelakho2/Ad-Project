/**
 * Stripe Type Extensions
 * 
 * Extends Stripe SDK types for newer API versions not yet in the types package.
 * This allows using newer API versions without `as any` casts.
 */

import Stripe from 'stripe';

// Extend Stripe API versions to include newer releases
declare module 'stripe' {
  namespace Stripe {
    type LatestApiVersion =
      | '2025-10-29.clover'
      | '2025-01-15.acacia'
      | '2024-12-18.acacia'
      | '2024-11-20.acacia'
      | '2024-10-28.acacia'
      | '2024-09-30.acacia'
      | '2024-06-20'
      | '2024-04-10'
      | '2023-10-16'
      | '2023-08-16';
  }
}

/**
 * Type-safe Stripe API version
 */
export type StripeApiVersion = Stripe.LatestApiVersion | (string & {});

/**
 * Create a Stripe client with type-safe API version
 */
export function createStripeClient(
  secretKey: string,
  apiVersion: StripeApiVersion
): Stripe {
  return new Stripe(secretKey, {
    apiVersion: apiVersion as Stripe.LatestApiVersion,
    typescript: true,
  });
}

export default Stripe;

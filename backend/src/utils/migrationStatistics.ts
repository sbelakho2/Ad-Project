/**
 * Migration Studio Statistics Library
 * 
 * Provides proper statistical calculations for A/B experiment analysis.
 * 
 * IMPORTANT: This replaces the previous "pseudo-science" implementation that used
 * hardcoded variance (impressions * 0.05) and binary p-values. These were not
 * derived from actual data distributions and undermined the validity of signed reports.
 * 
 * This implementation provides:
 * 1. Welch's t-test for comparing means with unequal variances
 * 2. Proper variance estimation from SUM(x²) and SUM(x) aggregates
 * 3. Confidence intervals for effect sizes
 * 4. Sequential testing adjustments for continuous monitoring
 */

/**
 * Sample statistics for a single arm
 */
export interface ArmStatistics {
  n: number;           // Sample size (impressions)
  sum: number;         // Sum of values (e.g., revenue_micros)
  sumSquares: number;  // Sum of squared values (for variance calculation)
  mean?: number;       // Computed mean
  variance?: number;   // Computed sample variance
  stdDev?: number;     // Computed standard deviation
}

/**
 * Result of a Welch's t-test comparison
 */
export interface WelchTTestResult {
  /** t-statistic value */
  tStatistic: number;
  /** Degrees of freedom (Welch-Satterthwaite approximation) */
  degreesOfFreedom: number;
  /** Two-tailed p-value */
  pValue: number;
  /** 95% confidence interval for the difference in means */
  confidenceInterval: {
    lower: number;
    upper: number;
  };
  /** Effect size (Cohen's d) */
  effectSize: number;
  /** Whether the result is statistically significant at alpha=0.05 */
  isSignificant: boolean;
  /** Sample size recommendation for desired power */
  recommendedSampleSize?: number;
}

/**
 * Minimum sample size for reliable statistics
 */
export const MIN_SAMPLE_SIZE = 30;

/**
 * Critical t-values for common significance levels (approximation for large df)
 */
const T_CRITICAL_95 = 1.96;  // α = 0.05, two-tailed
const T_CRITICAL_99 = 2.576; // α = 0.01, two-tailed

/**
 * Compute sample statistics from raw aggregates
 * 
 * Uses the computational formula for variance:
 * Var(X) = E[X²] - E[X]² = (Σx²/n) - (Σx/n)²
 */
export function computeStatistics(n: number, sum: number, sumSquares: number): ArmStatistics {
  if (n <= 0) {
    return { n: 0, sum: 0, sumSquares: 0, mean: 0, variance: 0, stdDev: 0 };
  }

  const mean = sum / n;
  
  // Sample variance using Bessel's correction (n-1)
  // Var = (Σx² - n*μ²) / (n-1) = (Σx² - (Σx)²/n) / (n-1)
  let variance = 0;
  if (n > 1) {
    variance = (sumSquares - (sum * sum) / n) / (n - 1);
    // Variance can't be negative (numerical precision issues)
    variance = Math.max(0, variance);
  }
  
  const stdDev = Math.sqrt(variance);

  return { n, sum, sumSquares, mean, variance, stdDev };
}

/**
 * Perform Welch's t-test for comparing two sample means
 * 
 * Welch's t-test is robust to unequal variances and unequal sample sizes,
 * making it more appropriate than Student's t-test for A/B experiments.
 * 
 * @param control Statistics for the control arm
 * @param test Statistics for the test arm
 * @returns Test results including p-value and confidence interval
 */
export function welchTTest(
  control: ArmStatistics,
  test: ArmStatistics
): WelchTTestResult {
  const n1 = control.n;
  const n2 = test.n;
  const mean1 = control.mean ?? (control.sum / Math.max(1, n1));
  const mean2 = test.mean ?? (test.sum / Math.max(1, n2));
  const var1 = control.variance ?? 0;
  const var2 = test.variance ?? 0;

  // Handle edge cases
  if (n1 < MIN_SAMPLE_SIZE || n2 < MIN_SAMPLE_SIZE) {
    return {
      tStatistic: 0,
      degreesOfFreedom: 0,
      pValue: 1, // Not significant due to insufficient data
      confidenceInterval: { lower: 0, upper: 0 },
      effectSize: 0,
      isSignificant: false,
      recommendedSampleSize: MIN_SAMPLE_SIZE,
    };
  }

  // Welch's t-statistic
  const se1 = var1 / n1;
  const se2 = var2 / n2;
  const pooledSE = Math.sqrt(se1 + se2);

  if (pooledSE === 0) {
    // Both samples have zero variance - they're identical
    return {
      tStatistic: 0,
      degreesOfFreedom: n1 + n2 - 2,
      pValue: 1,
      confidenceInterval: { lower: 0, upper: 0 },
      effectSize: 0,
      isSignificant: false,
    };
  }

  const tStatistic = (mean2 - mean1) / pooledSE;

  // Welch-Satterthwaite degrees of freedom
  const df = calculateWelchDF(se1, se2, n1, n2);

  // Calculate p-value using t-distribution approximation
  const pValue = tDistributionPValue(Math.abs(tStatistic), df);

  // 95% confidence interval for the difference
  const tCritical = getTCritical(df, 0.05);
  const marginOfError = tCritical * pooledSE;
  const diff = mean2 - mean1;

  // Cohen's d effect size
  const pooledStdDev = Math.sqrt(((n1 - 1) * var1 + (n2 - 1) * var2) / (n1 + n2 - 2));
  const effectSize = pooledStdDev > 0 ? diff / pooledStdDev : 0;

  return {
    tStatistic,
    degreesOfFreedom: df,
    pValue,
    confidenceInterval: {
      lower: diff - marginOfError,
      upper: diff + marginOfError,
    },
    effectSize,
    isSignificant: pValue < 0.05,
  };
}

/**
 * Calculate Welch-Satterthwaite degrees of freedom
 */
function calculateWelchDF(se1: number, se2: number, n1: number, n2: number): number {
  const numerator = Math.pow(se1 + se2, 2);
  const denominator = Math.pow(se1, 2) / (n1 - 1) + Math.pow(se2, 2) / (n2 - 1);
  
  if (denominator === 0) {
    return Math.min(n1, n2) - 1;
  }
  
  return numerator / denominator;
}

/**
 * Approximate p-value from t-statistic using the t-distribution
 * 
 * Uses a numerical approximation that's accurate for most practical purposes.
 * For a more accurate implementation, consider using a statistics library.
 */
function tDistributionPValue(t: number, df: number): number {
  // Use the beta function approximation for the t-distribution CDF
  // This is the incomplete beta function approach
  
  if (df <= 0) return 1;
  if (t === 0) return 1;
  
  // For large df, t-distribution approaches normal
  if (df > 100) {
    return 2 * normalCDF(-Math.abs(t));
  }

  // Approximation using the relationship between t and beta distributions
  const x = df / (df + t * t);
  const beta = incompleteBeta(df / 2, 0.5, x);
  
  return beta; // Two-tailed p-value
}

/**
 * Standard normal CDF approximation (Abramowitz and Stegun)
 */
function normalCDF(z: number): number {
  const a1 =  0.254829592;
  const a2 = -0.284496736;
  const a3 =  1.421413741;
  const a4 = -1.453152027;
  const a5 =  1.061405429;
  const p  =  0.3275911;

  const sign = z < 0 ? -1 : 1;
  z = Math.abs(z) / Math.sqrt(2);

  const t = 1.0 / (1.0 + p * z);
  const y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * Math.exp(-z * z);

  return 0.5 * (1.0 + sign * y);
}

/**
 * Incomplete beta function approximation
 * Uses continued fraction expansion
 */
function incompleteBeta(a: number, b: number, x: number): number {
  if (x === 0) return 0;
  if (x === 1) return 1;
  
  // Use the continued fraction representation
  const bt = Math.exp(
    logGamma(a + b) - logGamma(a) - logGamma(b) +
    a * Math.log(x) + b * Math.log(1 - x)
  );

  if (x < (a + 1) / (a + b + 2)) {
    return bt * betaCF(a, b, x) / a;
  } else {
    return 1 - bt * betaCF(b, a, 1 - x) / b;
  }
}

/**
 * Continued fraction for incomplete beta function
 */
function betaCF(a: number, b: number, x: number): number {
  const maxIterations = 100;
  const epsilon = 3e-7;
  
  let m = 1;
  let aa: number;
  let c = 1;
  let d = 1 - (a + b) * x / (a + 1);
  if (Math.abs(d) < epsilon) d = epsilon;
  d = 1 / d;
  let h = d;

  for (m = 1; m <= maxIterations; m++) {
    const m2 = 2 * m;
    
    // Even step
    aa = m * (b - m) * x / ((a + m2 - 1) * (a + m2));
    d = 1 + aa * d;
    if (Math.abs(d) < epsilon) d = epsilon;
    c = 1 + aa / c;
    if (Math.abs(c) < epsilon) c = epsilon;
    d = 1 / d;
    h *= d * c;
    
    // Odd step
    aa = -(a + m) * (a + b + m) * x / ((a + m2) * (a + m2 + 1));
    d = 1 + aa * d;
    if (Math.abs(d) < epsilon) d = epsilon;
    c = 1 + aa / c;
    if (Math.abs(c) < epsilon) c = epsilon;
    d = 1 / d;
    const delta = d * c;
    h *= delta;
    
    if (Math.abs(delta - 1) < epsilon) break;
  }

  return h;
}

/**
 * Log gamma function approximation (Stirling's approximation with corrections)
 */
function logGamma(x: number): number {
  const c = [
    76.18009172947146,
    -86.50532032941677,
    24.01409824083091,
    -1.231739572450155,
    0.1208650973866179e-2,
    -0.5395239384953e-5
  ];
  
  let y = x;
  let tmp = x + 5.5;
  tmp -= (x + 0.5) * Math.log(tmp);
  let ser = 1.000000000190015;
  
  for (let j = 0; j < 6; j++) {
    ser += c[j] / ++y;
  }
  
  return -tmp + Math.log(2.5066282746310005 * ser / x);
}

/**
 * Get critical t-value for given degrees of freedom and significance level
 */
function getTCritical(df: number, alpha: number): number {
  // For large df, use normal approximation
  if (df > 100) {
    return alpha === 0.05 ? T_CRITICAL_95 : T_CRITICAL_99;
  }
  
  // Lookup table for small df (two-tailed, alpha=0.05)
  const tTable05: Record<number, number> = {
    1: 12.706, 2: 4.303, 3: 3.182, 4: 2.776, 5: 2.571,
    6: 2.447, 7: 2.365, 8: 2.306, 9: 2.262, 10: 2.228,
    15: 2.131, 20: 2.086, 25: 2.060, 30: 2.042, 40: 2.021,
    50: 2.009, 60: 2.000, 80: 1.990, 100: 1.984
  };
  
  // Find closest df in table
  const keys = Object.keys(tTable05).map(Number).sort((a, b) => a - b);
  let closest = 100;
  for (const k of keys) {
    if (k >= df) {
      closest = k;
      break;
    }
  }
  
  return tTable05[closest] ?? T_CRITICAL_95;
}

/**
 * Calculate required sample size for detecting a given effect
 * 
 * Uses the standard formula for two-sample t-test power analysis:
 * n = 2 * (z_α + z_β)² * σ² / δ²
 * 
 * @param effectSize Expected effect size (Cohen's d)
 * @param power Desired power (default 0.80)
 * @param alpha Significance level (default 0.05)
 */
export function requiredSampleSize(
  effectSize: number,
  power = 0.80,
  alpha = 0.05
): number {
  if (effectSize === 0) return Infinity;
  
  // Z-scores for significance and power
  const zAlpha = alpha === 0.05 ? 1.96 : 2.576;
  const zBeta = power === 0.80 ? 0.84 : (power === 0.90 ? 1.28 : 1.645);
  
  // Sample size per group
  const n = 2 * Math.pow(zAlpha + zBeta, 2) / Math.pow(effectSize, 2);
  
  return Math.ceil(n);
}

/**
 * Sequential testing adjustment using O'Brien-Fleming alpha spending
 * 
 * For continuous monitoring, we need to adjust the significance threshold
 * to control the overall Type I error rate.
 * 
 * @param looks Number of interim looks at the data
 * @param currentLook Which look this is (1-indexed)
 * @param alpha Overall significance level (default 0.05)
 */
export function sequentialAlphaSpending(
  looks: number,
  currentLook: number,
  alpha = 0.05
): number {
  // O'Brien-Fleming spending function
  // More conservative early on, closer to nominal alpha at final look
  const informationFraction = currentLook / looks;
  
  // Using the Lan-DeMets approximation to O'Brien-Fleming
  const z = -normalInverse(alpha / 2);
  const spent = 2 * (1 - normalCDF(z / Math.sqrt(informationFraction)));
  
  return spent;
}

/**
 * Inverse normal CDF (probit function) approximation
 */
function normalInverse(p: number): number {
  if (p <= 0) return -Infinity;
  if (p >= 1) return Infinity;
  if (p === 0.5) return 0;
  
  // Rational approximation for central region
  const a = [
    -3.969683028665376e+01,
    2.209460984245205e+02,
    -2.759285104469687e+02,
    1.383577518672690e+02,
    -3.066479806614716e+01,
    2.506628277459239e+00
  ];
  const b = [
    -5.447609879822406e+01,
    1.615858368580409e+02,
    -1.556989798598866e+02,
    6.680131188771972e+01,
    -1.328068155288572e+01
  ];

  const pLow = 0.02425;
  const pHigh = 1 - pLow;

  let q: number, r: number;

  if (p < pLow) {
    q = Math.sqrt(-2 * Math.log(p));
    const num = ((((a[0] * q + a[1]) * q + a[2]) * q + a[3]) * q + a[4]) * q + a[5];
    const den = ((((b[0] * q + b[1]) * q + b[2]) * q + b[3]) * q + b[4]) * q + 1;
    return num / den;
  } else if (p <= pHigh) {
    q = p - 0.5;
    r = q * q;
    const num = ((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5];
    const den = ((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1;
    return (num * q) / den;
  } else {
    q = Math.sqrt(-2 * Math.log(1 - p));
    const num = ((((a[0] * q + a[1]) * q + a[2]) * q + a[3]) * q + a[4]) * q + a[5];
    const den = ((((b[0] * q + b[1]) * q + b[2]) * q + b[3]) * q + b[4]) * q + 1;
    return -(num / den);
  }
}

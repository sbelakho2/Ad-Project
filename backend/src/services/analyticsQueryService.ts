export type AnalyticsQueryOptions = {
	table: 'analytics_impressions' | 'analytics_clicks' | 'analytics_revenue_events';
	metric: 'count' | 'revenue';
	startDate: Date;
	endDate: Date;
	granularity?: 'hour' | 'day';
};

export type AnalyticsQueryResult = {
	sql: string;
	params: [string, Date, Date];
};

const allowedTables = new Set<AnalyticsQueryOptions['table']>([
	'analytics_impressions',
	'analytics_clicks',
	'analytics_revenue_events',
]);

export function buildAnalyticsTimeSeriesQuery(options: AnalyticsQueryOptions): AnalyticsQueryResult {
	if (!allowedTables.has(options.table)) {
		throw new Error(`Unsupported analytics table: ${options.table}`);
	}

	if (options.metric === 'revenue' && options.table !== 'analytics_revenue_events') {
		throw new Error('Revenue metric is only supported for analytics_revenue_events');
	}

	const granularity = options.granularity ?? 'day';
	const metricSql =
		options.metric === 'revenue'
			? 'COALESCE(SUM(revenue_usd), 0) AS value'
			: 'COUNT(*)::bigint AS value';

	const sql = `
		SELECT
			date_trunc($1, observed_at) AS bucket,
			${metricSql}
		FROM ${options.table}
		WHERE observed_at >= $2
			AND observed_at < $3
			AND is_test_mode = false
		GROUP BY 1
		ORDER BY 1 ASC
	`;

	return {
		sql: sql.trim(),
		params: [granularity, options.startDate, options.endDate],
	};
}

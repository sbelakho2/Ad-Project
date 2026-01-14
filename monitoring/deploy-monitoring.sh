#!/bin/bash
# Deploy ApexMediation Monitoring Stack
# Usage: ./deploy-monitoring.sh [start|stop|restart|logs|status]

set -e

COMMAND=${1:-start}
MONITORING_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ "$(uname -s)" == "Darwin" ]]; then
  SED_INPLACE=(sed -i '')
else
  SED_INPLACE=(sed -i)
fi
ALERTMANAGER_RENDERED="$MONITORING_DIR/alertmanager.generated.yml"
GRAFANA_DATASOURCES_RENDERED="$MONITORING_DIR/grafana-datasources.generated.yml"
PROMETHEUS_RENDERED="$MONITORING_DIR/prometheus.generated.yml"

echo "🔍 ApexMediation Monitoring Stack"
echo "   Directory: $MONITORING_DIR"
echo "   Command: $COMMAND"
echo ""

# Load environment variables
if [ -f "$MONITORING_DIR/.env" ]; then
  source "$MONITORING_DIR/.env"
  echo "✅ Environment variables loaded"
else
  echo "⚠️  No .env file found. Creating template..."
  cat > "$MONITORING_DIR/.env" <<EOF
# Grafana
GRAFANA_PASSWORD=changeme
GRAFANA_URL=http://localhost:3000
GRAFANA_API_KEY=

# Email alerts
RESEND_API_KEY=re_...

# Database (for Grafana datasource)
DATABASE_URL=postgresql://...
DB_USER=readonly_user
DB_PASSWORD=...

# Optional: SMS alerts via Twilio
TWILIO_WEBHOOK_URL=
TWILIO_ACCOUNT_SID=
TWILIO_AUTH_TOKEN=

# Optional: Slack notifications
SLACK_WEBHOOK_URL=

# Optional: Discord notifications
DISCORD_WEBHOOK_URL=

# Prometheus target overrides (set to localhost services for local dev)
PROMETHEUS_CLUSTER_LABEL=apexmediation
PROMETHEUS_ENVIRONMENT_LABEL=production
# Hetzner production (monitoring runs in Docker on the app server; scrape host services via bridge).
# If your Docker host bridge differs, override these.
BACKEND_METRICS_TARGET=172.17.0.1:8080
CONSOLE_METRICS_TARGET=172.17.0.1:3000
REDIS_METRICS_TARGET=upstash-redis-exporter:9121
EOF
  echo "⚠️  Please edit monitoring/.env with real credentials, then re-run ./deploy-monitoring.sh start"
  exit 1
fi

BACKEND_METRICS_TARGET=${BACKEND_METRICS_TARGET:-172.17.0.1:8080}
CONSOLE_METRICS_TARGET=${CONSOLE_METRICS_TARGET:-172.17.0.1:3000}
REDIS_METRICS_TARGET=${REDIS_METRICS_TARGET:-upstash-redis-exporter:9121}
PROMETHEUS_CLUSTER_LABEL=${PROMETHEUS_CLUSTER_LABEL:-apexmediation}
PROMETHEUS_ENVIRONMENT_LABEL=${PROMETHEUS_ENVIRONMENT_LABEL:-production}

# On Linux servers, host.docker.internal may not be present.
# Prefer setting BACKEND_METRICS_TARGET/CONSOLE_METRICS_TARGET explicitly in monitoring/.env,
# e.g. 172.17.0.1:8080 and 172.17.0.1:3000 to scrape host-network services.

render_alertmanager_config() {
  if ! command -v envsubst >/dev/null 2>&1; then
    echo "❌ 'envsubst' command not found. Install gettext (e.g., 'sudo apt install gettext-base') before running the monitoring stack."
    exit 1
  fi

  if [ -z "$RESEND_API_KEY" ]; then
    echo "❌ RESEND_API_KEY is not set in monitoring/.env; required for SMTP auth."
    exit 1
  fi

  envsubst '$RESEND_API_KEY $TWILIO_WEBHOOK_URL $TWILIO_ACCOUNT_SID $TWILIO_AUTH_TOKEN $SLACK_WEBHOOK_URL $DISCORD_WEBHOOK_URL' \
    < "$MONITORING_DIR/alertmanager.yml" > "$ALERTMANAGER_RENDERED"

  strip_optional_alertmanager_block() {
    local marker="$1"
    "${SED_INPLACE[@]}" "/# OPTIONAL_${marker}_START/,/# OPTIONAL_${marker}_END/d" "$ALERTMANAGER_RENDERED"
  }

  if [ -z "$TWILIO_WEBHOOK_URL" ]; then
    strip_optional_alertmanager_block "TWILIO_ROUTE"
    strip_optional_alertmanager_block "TWILIO_RECEIVER"
    echo "⚠️  Disabled Twilio SMS alerts (set TWILIO_WEBHOOK_URL to enable)."
  fi

  if [ -z "$SLACK_WEBHOOK_URL" ]; then
    strip_optional_alertmanager_block "SLACK_RECEIVER"
    echo "⚠️  Disabled Slack notifications (set SLACK_WEBHOOK_URL to enable)."
  fi

  if [ -z "$DISCORD_WEBHOOK_URL" ]; then
    strip_optional_alertmanager_block "DISCORD_RECEIVER"
    echo "⚠️  Disabled Discord notifications (set DISCORD_WEBHOOK_URL to enable)."
  fi

  chmod 644 "$ALERTMANAGER_RENDERED"
  echo "✅ Rendered Alertmanager config -> $ALERTMANAGER_RENDERED"
}

render_grafana_datasources() {
  if [ ! -f "$MONITORING_DIR/grafana-datasources.yml" ]; then
    echo "❌ monitoring/grafana-datasources.yml is missing; cannot render Grafana datasources."
    exit 1
  fi

  cp "$MONITORING_DIR/grafana-datasources.yml" "$GRAFANA_DATASOURCES_RENDERED"
  printf "\n" >> "$GRAFANA_DATASOURCES_RENDERED"

  if [[ -n "$DATABASE_URL" && -n "$DB_USER" && -n "$DB_PASSWORD" ]]; then
    cat >> "$GRAFANA_DATASOURCES_RENDERED" <<EOF
  - name: PostgreSQL
    type: postgres
    url: "$DATABASE_URL"
    user: "$DB_USER"
    secureJsonData:
      password: "$DB_PASSWORD"
    jsonData:
      database: apexmediation
      sslmode: require
      maxOpenConns: 10
      maxIdleConns: 5
      connMaxLifetime: 14400
EOF
  else
    echo "⚠️  Skipping PostgreSQL datasource — set DATABASE_URL, DB_USER, and DB_PASSWORD in monitoring/.env to enable it."
  fi

  chmod 600 "$GRAFANA_DATASOURCES_RENDERED"
  echo "✅ Rendered Grafana datasources -> $GRAFANA_DATASOURCES_RENDERED"
}

render_prometheus_config() {
  if ! command -v envsubst >/dev/null 2>&1; then
    echo "❌ 'envsubst' command not found. Install gettext (e.g., 'sudo apt install gettext-base') before running the monitoring stack."
    exit 1
  fi

  envsubst '$PROMETHEUS_CLUSTER_LABEL $PROMETHEUS_ENVIRONMENT_LABEL $BACKEND_METRICS_TARGET $CONSOLE_METRICS_TARGET $REDIS_METRICS_TARGET' \
    < "$MONITORING_DIR/prometheus.yml" > "$PROMETHEUS_RENDERED"
  chmod 644 "$PROMETHEUS_RENDERED"
  echo "✅ Rendered Prometheus config -> $PROMETHEUS_RENDERED"
}

export_dashboards() {
  if ! command -v jq >/dev/null 2>&1; then
    echo "❌ 'jq' is required to export dashboards. Install it (e.g., 'sudo apt install jq')."
    exit 1
  fi

  if ! command -v curl >/dev/null 2>&1; then
    echo "❌ 'curl' is required to reach the Grafana API."
    exit 1
  fi

  local api_key="${GRAFANA_API_KEY:-}"
  if [[ -z "$api_key" ]]; then
    echo "❌ Set GRAFANA_API_KEY (environment or monitoring/.env) before exporting dashboards."
    exit 1
  fi

  local grafana_url="${GRAFANA_URL:-http://localhost:3000}"
  local dashboards_dir="$MONITORING_DIR/grafana"
  if [[ ! -d "$dashboards_dir" ]]; then
    echo "❌ Dashboard directory not found: $dashboards_dir"
    exit 1
  fi

  echo "📤 Exporting dashboards from $grafana_url into $dashboards_dir"
  local failures=0
  while IFS= read -r -d '' file; do
    uid=$(jq -r '.uid // empty' "$file")
    if [[ -z "$uid" ]]; then
      echo "  ⚠️  Skipping $(basename "$file") (missing uid)"
      continue
    fi
    if ! response=$(curl -fsS -H "Authorization: Bearer $api_key" "$grafana_url/api/dashboards/uid/$uid"); then
      echo "  ❌ Failed to fetch $uid"
      failures=$((failures + 1))
      continue
    fi
    if ! printf '%s' "$response" | jq '.dashboard' > "$file"; then
      echo "  ❌ Failed to write dashboard file $file"
      failures=$((failures + 1))
      continue
    fi
    echo "  ✅ Exported $uid -> $file"
  done < <(find "$dashboards_dir" -type f -name '*.json' -print0)

  if [[ $failures -gt 0 ]]; then
    echo "⚠️  Completed with $failures failure(s)."
    exit 1
  fi

  echo "✅ Dashboard export complete."
}

verify_backup_archive() {
  local archive_path="$1"
  if gzip -t "$archive_path" >/dev/null 2>&1; then
    echo "  ✅ Verified $(basename "$archive_path")"
  else
    echo "  ❌ Failed to verify $(basename "$archive_path"). The archive may be corrupt."
    exit 1
  fi
}

case $COMMAND in
  start)
    render_alertmanager_config
    render_grafana_datasources
    render_prometheus_config
    echo "🚀 Starting monitoring stack..."
    cd "$MONITORING_DIR"
    docker-compose up -d
    
    echo ""
    echo "⏳ Waiting for services to be ready..."
    sleep 10
    
    # Check service health
    echo ""
    echo "🏥 Health checks:"
    
    if curl -f -s http://localhost:9090/-/healthy > /dev/null; then
      echo "  ✅ Prometheus: http://localhost:9090"
    else
      echo "  ❌ Prometheus: Not healthy"
    fi
    
    if curl -f -s http://localhost:3100/ready > /dev/null; then
      echo "  ✅ Loki: http://localhost:3100"
    else
      echo "  ❌ Loki: Not healthy"
    fi
    
    if curl -f -s http://localhost:3000/api/health > /dev/null; then
      echo "  ✅ Grafana: http://localhost:3000 (admin / $GRAFANA_PASSWORD)"
    else
      echo "  ❌ Grafana: Not healthy"
    fi
    
    if curl -f -s http://localhost:9093/-/healthy > /dev/null; then
      echo "  ✅ Alertmanager: http://localhost:9093"
    else
      echo "  ❌ Alertmanager: Not healthy"
    fi
    
    echo ""
    echo "✨ Monitoring stack started!"
    echo ""
    echo "Next steps:"
    echo "  1. Open Grafana: http://localhost:3000"
    echo "  2. Login: admin / $GRAFANA_PASSWORD"
    echo "  3. Import dashboards from monitoring/dashboards/"
    echo "  4. Configure alert channels in Alertmanager"
    echo "  5. Test alerts: docker-compose exec prometheus promtool test rules alerts.yml"
    ;;
  
  stop)
    echo "🛑 Stopping monitoring stack..."
    cd "$MONITORING_DIR"
    docker-compose down
    echo "✅ Monitoring stack stopped"
    ;;
  
  restart)
    render_alertmanager_config
    render_grafana_datasources
    render_prometheus_config
    echo "🔄 Restarting monitoring stack..."
    cd "$MONITORING_DIR"
    docker-compose restart
    echo "✅ Monitoring stack restarted"
    ;;
  
  logs)
    SERVICE=${2:-all}
    if [ "$SERVICE" == "all" ]; then
      echo "📋 Showing logs for all services (Ctrl+C to exit)..."
      cd "$MONITORING_DIR"
      docker-compose logs -f
    else
      echo "📋 Showing logs for $SERVICE (Ctrl+C to exit)..."
      cd "$MONITORING_DIR"
      docker-compose logs -f "$SERVICE"
    fi
    ;;
  
  status)
    echo "📊 Monitoring stack status:"
    cd "$MONITORING_DIR"
    docker-compose ps
    
    echo ""
    echo "💾 Disk usage:"
    du -sh prometheus-data loki-data grafana-data alertmanager-data 2>/dev/null || echo "  No data volumes found"
    ;;
  
  backup)
    echo "💾 Backing up monitoring data..."
    BACKUP_DIR="$MONITORING_DIR/backups/$(date +%Y%m%d_%H%M%S)"
    mkdir -p "$BACKUP_DIR"
    
    cd "$MONITORING_DIR"
    docker-compose exec -T prometheus tar czf - /prometheus > "$BACKUP_DIR/prometheus.tar.gz"
    verify_backup_archive "$BACKUP_DIR/prometheus.tar.gz"
    docker-compose exec -T loki tar czf - /loki > "$BACKUP_DIR/loki.tar.gz"
    verify_backup_archive "$BACKUP_DIR/loki.tar.gz"
    docker-compose exec -T grafana tar czf - /var/lib/grafana > "$BACKUP_DIR/grafana.tar.gz"
    verify_backup_archive "$BACKUP_DIR/grafana.tar.gz"
    
    echo "✅ Backup saved to $BACKUP_DIR"
    ;;
  
  test-alerts)
    echo "🧪 Testing alert rules..."
    cd "$MONITORING_DIR"
    docker-compose exec prometheus promtool check rules /etc/prometheus/alerts.yml
    
    echo ""
    echo "📧 Sending test alert..."
    curl -X POST http://localhost:9093/api/v1/alerts \
      -H "Content-Type: application/json" \
      -d '[{
        "labels": {
          "alertname": "TestAlert",
          "severity": "info",
          "service": "monitoring"
        },
        "annotations": {
          "summary": "Test alert from monitoring setup",
          "description": "This is a test alert to verify email delivery is working."
        }
      }]'
    
    echo ""
    echo "✅ Test alert sent. Check your email."
    ;;
    export-dashboards)
      export_dashboards
      ;;
  
  *)
     echo "Usage: ./deploy-monitoring.sh [start|stop|restart|logs|status|backup|test-alerts|export-dashboards]"
    echo ""
    echo "Commands:"
    echo "  start        - Start monitoring stack"
    echo "  stop         - Stop monitoring stack"
    echo "  restart      - Restart monitoring stack"
    echo "  logs [svc]   - Show logs (optionally for specific service)"
    echo "  status       - Show service status and resource usage"
    echo "  backup       - Backup monitoring data"
    echo "  test-alerts  - Test alert rules and email delivery"
     echo "  export-dashboards - Pull the currently running Grafana dashboards back into monitoring/grafana"
    exit 1
    ;;
esac

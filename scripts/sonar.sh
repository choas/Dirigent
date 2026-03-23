#!/bin/sh
# Load .env file from project root if it exists
ENV_FILE="$(dirname "$0")/../.env"
if [ -f "$ENV_FILE" ]; then
    # shellcheck disable=SC1090
    . "$ENV_FILE"
fi

if [ -z "$SONAR_TOKEN" ]; then
    echo "Error: SONAR_TOKEN not set. Add it to .env or export it." >&2
    exit 1
fi

sonar-scanner \
  -Dsonar.projectKey=Dirigent \
  -Dsonar.sources=src \
  -Dsonar.host.url=${SONAR_HOST_URL:-http://localhost:9000} \
  -Dsonar.token="$SONAR_TOKEN"

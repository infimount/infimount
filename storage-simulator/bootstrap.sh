#!/usr/bin/env bash
set -euo pipefail

echo "=== Bootstrapping Storage Simulator ==="

# Wait until HTTP endpoint is reachable (any HTTP status, not only 2xx)
wait_for_http() {
  local url=$1
  echo -n "Waiting for $url "
  until curl -s "$url" >/dev/null 2>&1; do
    echo -n "."
    sleep 1
  done
  echo " OK"
}

# Wait until a TCP port is open
wait_for_tcp() {
  local host=$1
  local port=$2
  echo -n "Waiting for tcp://$host:$port "
  until (echo >/dev/tcp/"$host"/"$port") >/dev/null 2>&1; do
    echo -n "."
    sleep 1
  done
  echo " OK"
}

# Wait for services
wait_for_http "http://localhost:8333"
wait_for_tcp "localhost" "10000"
wait_for_http "http://localhost:4443/storage/v1/b"

echo "Creating test sample..."
echo "Hello Storage Simulator" > sample.txt

# ----------- SeaweedFS S3 Bucket -----------
# Use explicit local credentials so bootstrap doesn't depend on host AWS profile.
export AWS_ACCESS_KEY_ID="admin"
export AWS_SECRET_ACCESS_KEY="password123"
export AWS_DEFAULT_REGION="us-east-1"
export AWS_EC2_METADATA_DISABLED="true"

echo "[S3] Creating bucket..."
aws --endpoint-url http://localhost:8333 \
    --no-cli-pager \
    s3 mb s3://test-bucket --region us-east-1 || true

echo "[S3] Uploading sample..."
aws --endpoint-url http://localhost:8333 \
    --no-cli-pager \
    s3 cp sample.txt s3://test-bucket/sample.txt

# ----------- Azurite -----------------------
echo "[Azurite] Creating container..."
AZURITE_ACCOUNT_KEY="Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw=="
az storage container create \
    --name test-container \
    --account-name devstoreaccount1 \
    --account-key "$AZURITE_ACCOUNT_KEY" \
    --blob-endpoint http://localhost:10000/devstoreaccount1

echo "[Azurite] Uploading blob..."
az storage blob upload \
    --container-name test-container \
    --file sample.txt \
    --name sample.txt \
    --account-name devstoreaccount1 \
    --account-key "$AZURITE_ACCOUNT_KEY" \
    --blob-endpoint http://localhost:10000/devstoreaccount1

# ----------- GCS Emulator ------------------
echo "[GCS] Creating bucket..."
GCS_CREATE_CODE="$(
  curl -s -o /tmp/fake-gcs-create-bucket.json -w "%{http_code}" \
    -X POST \
    -H "Content-Type: application/json" \
    -d '{"name":"test-bucket"}' \
    "http://localhost:4443/storage/v1/b?project=infimount"
)"
if [ "$GCS_CREATE_CODE" != "200" ] && [ "$GCS_CREATE_CODE" != "201" ] && [ "$GCS_CREATE_CODE" != "409" ]; then
  echo "❌ [GCS] Bucket create failed with HTTP $GCS_CREATE_CODE"
  cat /tmp/fake-gcs-create-bucket.json || true
  exit 1
fi

echo "[GCS] Uploading object..."
GCS_UPLOAD_CODE="$(
  curl -s -o /tmp/fake-gcs-upload-object.json -w "%{http_code}" \
    -X POST \
    --data-binary @sample.txt \
    "http://localhost:4443/upload/storage/v1/b/test-bucket/o?uploadType=media&name=sample.txt"
)"
if [ "$GCS_UPLOAD_CODE" != "200" ] && [ "$GCS_UPLOAD_CODE" != "201" ]; then
  echo "❌ [GCS] Upload failed with HTTP $GCS_UPLOAD_CODE"
  cat /tmp/fake-gcs-upload-object.json || true
  exit 1
fi

echo "=== Done. Storage Simulator Ready ==="

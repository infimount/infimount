#!/usr/bin/env bash
set -e

echo "=== Bootstrapping Storage Simulator ==="

# Small helper to wait for a service to respond
wait_for() {
  local url=$1
  echo -n "Waiting for $url "
  until curl -s --fail "$url" >/dev/null 2>&1; do
    echo -n "."
    sleep 1
  done
  echo " OK"
}

# Wait for services
wait_for "http://localhost:8333"
wait_for "http://localhost:10000"
wait_for "http://localhost:4443/storage/v1/b"

echo "Creating test sample..."
echo "Hello Storage Simulator" > sample.txt

# ----------- SeaweedFS S3 Bucket -----------
echo "[S3] Creating bucket..."
aws --endpoint-url http://localhost:8333 \
    s3 mb s3://test-bucket --region us-east-1 || true

echo "[S3] Uploading sample..."
aws --endpoint-url http://localhost:8333 \
    s3 cp sample.txt s3://test-bucket/sample.txt

# ----------- Azurite -----------------------
echo "[Azurite] Creating container..."
az storage container create \
    --name test-container \
    --account-name devstoreaccount1 \
    --account-key Eby8vdM02xNoGuBy+EXAMPLEKEY== \
    --blob-endpoint http://localhost:10000/devstoreaccount1

echo "[Azurite] Uploading blob..."
az storage blob upload \
    --container-name test-container \
    --file sample.txt \
    --name sample.txt \
    --account-name devstoreaccount1 \
    --account-key Eby8vdM02xNoGuBy+EXAMPLEKEY==

# ----------- GCS Emulator ------------------
echo "[GCS] Creating bucket..."
curl -X PUT http://localhost:4443/storage/v1/b/test-bucket >/dev/null 2>&1 || true

echo "[GCS] Uploading object..."
curl -X POST \
     --data-binary @sample.txt \
     "http://localhost:4443/upload/storage/v1/b/test-bucket/o?uploadType=media&name=sample.txt"

echo "=== Done. Storage Simulator Ready ==="

#!/usr/bin/env bash
set -euo pipefail

if [[ "${RUNNER_OS:-Linux}" != "Linux" ]]; then
  echo "Storage simulator CLI installer only supports Linux runners." >&2
  exit 1
fi

sudo apt-get update
sudo apt-get install -y ca-certificates curl unzip

if ! command -v aws >/dev/null 2>&1; then
  curl -fsSL https://awscli.amazonaws.com/awscli-exe-linux-x86_64.zip -o /tmp/awscliv2.zip
  rm -rf /tmp/aws
  unzip -q /tmp/awscliv2.zip -d /tmp
  sudo /tmp/aws/install --update
else
  echo "aws CLI already installed: $(aws --version 2>&1)"
fi

if ! command -v az >/dev/null 2>&1; then
  curl -fsSL https://aka.ms/InstallAzureCLIDeb -o /tmp/install-azure-cli.sh
  sudo bash /tmp/install-azure-cli.sh
else
  echo "Azure CLI already installed: $(az version --query azure-cli -o tsv 2>/dev/null || az --version | head -n 1)"
fi

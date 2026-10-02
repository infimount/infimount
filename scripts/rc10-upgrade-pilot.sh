#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ORIGINAL_HOME="$HOME"
PILOT_ROOT="${INFIMOUNT_RC10_PILOT_ROOT:-$ORIGINAL_HOME/.infimount-rc10-final-pilot}"
PILOT_HOME="$PILOT_ROOT/home"
PACKAGE_DIR="$PILOT_ROOT/packages"
EVIDENCE_DIR="$PILOT_ROOT/evidence"
STATE_SCRIPT="$ROOT_DIR/scripts/rc10-final-pilot-state.mjs"

STABLE_TAG="v0.8.0"
STABLE_VERSION="0.8.0"
STABLE_SHA="8d48cd4b8ba56ee74dbb2f6b7124234142795538575731495000e04ed2b645ab"
CANDIDATE_TAG="v0.8.1-rc.10"
CANDIDATE_VERSION="0.8.1-rc.10"
CANDIDATE_COMMIT="86859130f93757d6893bc128e3955906a4f0130e"
CANDIDATE_SHA="453b74f1892403e56a0f617983d28abed7749f1df692f31eaceae90cd37ccb8a"

require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "RC10 upgrade pilot failed: required command '$1' is unavailable" >&2
    exit 1
  }
}

for command in node curl sha256sum dpkg-query dpkg-deb git sudo apt-get; do
  require_command "$command"
done

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "RC10 upgrade pilot currently targets the published Linux amd64 package." >&2
  exit 1
fi

if pgrep -x infimount >/dev/null 2>&1; then
  echo "RC10 upgrade pilot failed: quit Infimount before running the package exercise." >&2
  exit 1
fi

cd "$ROOT_DIR"
git fetch origin main --tags >/dev/null

test "$(git rev-parse "$CANDIDATE_TAG^{commit}")" = "$CANDIDATE_COMMIT" || {
  echo "RC10 upgrade pilot failed: local $CANDIDATE_TAG does not resolve to $CANDIDATE_COMMIT" >&2
  exit 1
}

CURRENT_VERSION="$(dpkg-query -W -f='${Version}' infimount 2>/dev/null || true)"
if [[ "$CURRENT_VERSION" != "$CANDIDATE_VERSION" ]]; then
  echo "RC10 upgrade pilot failed: expected installed $CANDIDATE_VERSION, found '$CURRENT_VERSION'." >&2
  echo "The script refuses to replace an unknown user installation." >&2
  exit 1
fi

if [[ -e "$PILOT_ROOT" ]]; then
  echo "RC10 upgrade pilot failed: pilot root already exists: $PILOT_ROOT" >&2
  echo "Move/remove it only after preserving any evidence you need." >&2
  exit 1
fi

mkdir -p "$PILOT_HOME" "$PACKAGE_DIR" "$EVIDENCE_DIR"
chmod 700 "$PILOT_ROOT" "$PILOT_HOME" "$PACKAGE_DIR" "$EVIDENCE_DIR"

tree_digest() {
  node - "$1" <<'NODE'
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const root = path.resolve(process.argv[2]);
const hash = crypto.createHash("sha256");
if (!fs.existsSync(root)) {
  hash.update("ABSENT\0");
  process.stdout.write(hash.digest("hex"));
  process.exit(0);
}
function walk(dir, rel = "") {
  const entries = fs.readdirSync(dir, { withFileTypes: true })
    .sort((a, b) => a.name.localeCompare(b.name));
  for (const entry of entries) {
    const childRel = rel ? `${rel}/${entry.name}` : entry.name;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      hash.update(`D\0${childRel}\0`);
      walk(full, childRel);
    } else if (entry.isSymbolicLink()) {
      hash.update(`L\0${childRel}\0${fs.readlinkSync(full)}\0`);
    } else if (entry.isFile()) {
      hash.update(`F\0${childRel}\0`);
      hash.update(fs.readFileSync(full));
      hash.update("\0");
    }
  }
}
walk(root);
process.stdout.write(hash.digest("hex"));
NODE
}

REAL_CONFIG_BEFORE="$(tree_digest "$ORIGINAL_HOME/.infimount")"
echo "real_user_config_digest_before=$REAL_CONFIG_BEFORE"

echo "===== SEED REPRESENTATIVE V0.8.0 STATE ====="
HOME="$PILOT_HOME" node "$STATE_SCRIPT" seed-upgrade --home "$PILOT_HOME"

echo
echo "===== DOWNLOAD PINNED RELEASE PACKAGES ====="
curl -fL --retry 3   "https://github.com/infimount/infimount/releases/download/$STABLE_TAG/Infimount-amd64.deb"   -o "$PACKAGE_DIR/Infimount-v0.8.0-amd64.deb"
curl -fL --retry 3   "https://github.com/infimount/infimount/releases/download/$CANDIDATE_TAG/Infimount-amd64.deb"   -o "$PACKAGE_DIR/Infimount-v0.8.1-rc.10-amd64.deb"

STABLE_ACTUAL="$(sha256sum "$PACKAGE_DIR/Infimount-v0.8.0-amd64.deb" | awk '{print $1}')"
CANDIDATE_ACTUAL="$(sha256sum "$PACKAGE_DIR/Infimount-v0.8.1-rc.10-amd64.deb" | awk '{print $1}')"

echo "stable_sha=$STABLE_ACTUAL"
echo "candidate_sha=$CANDIDATE_ACTUAL"

test "$STABLE_ACTUAL" = "$STABLE_SHA" || {
  echo "RC10 upgrade pilot failed: v0.8.0 package digest mismatch" >&2
  exit 1
}
test "$CANDIDATE_ACTUAL" = "$CANDIDATE_SHA" || {
  echo "RC10 upgrade pilot failed: rc.10 package digest mismatch" >&2
  exit 1
}

test "$(dpkg-deb -f "$PACKAGE_DIR/Infimount-v0.8.0-amd64.deb" Version)" = "$STABLE_VERSION"
test "$(dpkg-deb -f "$PACKAGE_DIR/Infimount-v0.8.1-rc.10-amd64.deb" Version)" = "$CANDIDATE_VERSION"

RESTORE_ARMED=1
restore_candidate() {
  code=$?
  trap - ERR INT TERM
  if [[ "$RESTORE_ARMED" == "1" ]]; then
    installed="$(dpkg-query -W -f='${Version}' infimount 2>/dev/null || true)"
    if [[ "$installed" != "$CANDIDATE_VERSION" ]]; then
      echo "Restoring $CANDIDATE_VERSION after interrupted upgrade pilot..." >&2
      sudo apt-get install -y --allow-downgrades         "$PACKAGE_DIR/Infimount-v0.8.1-rc.10-amd64.deb" >/dev/null || true
    fi
  fi
  exit "$code"
}
trap restore_candidate ERR INT TERM

echo
echo "===== INSTALL ACTUAL STABLE PACKAGE ====="
sudo apt-get install -y --allow-downgrades "$PACKAGE_DIR/Infimount-v0.8.0-amd64.deb"
test "$(dpkg-query -W -f='${Version}' infimount)" = "$STABLE_VERSION"
echo "installed_stable=$STABLE_VERSION"

# No Infimount process is launched while stable is installed. The representative
# state lives only under PILOT_HOME; the user's real ~/.infimount is never used.

echo
echo "===== INSTALL RC.10 OVER STABLE ====="
sudo apt-get install -y "$PACKAGE_DIR/Infimount-v0.8.1-rc.10-amd64.deb"
test "$(dpkg-query -W -f='${Version}' infimount)" = "$CANDIDATE_VERSION"
echo "installed_candidate=$CANDIDATE_VERSION"

HOME="$PILOT_HOME" node "$STATE_SCRIPT" assert-upgrade --home "$PILOT_HOME"

REAL_CONFIG_AFTER="$(tree_digest "$ORIGINAL_HOME/.infimount")"
echo "real_user_config_digest_after=$REAL_CONFIG_AFTER"
test "$REAL_CONFIG_BEFORE" = "$REAL_CONFIG_AFTER" || {
  echo "RC10 upgrade pilot failed: the real user Infimount config changed." >&2
  false
}

cat >"$EVIDENCE_DIR/upgrade.json" <<EOF
{
  "from": "$STABLE_VERSION",
  "to": "$CANDIDATE_VERSION",
  "candidateCommit": "$CANDIDATE_COMMIT",
  "configurationRetained": true,
  "storageRegistryRetained": true,
  "workspaceRegistryRetained": true,
  "realUserConfigUntouched": true,
  "passed": true
}
EOF
chmod 600 "$EVIDENCE_DIR/upgrade.json"

RESTORE_ARMED=0
trap - ERR INT TERM

echo
echo "========================================"
echo "RC10 STABLE -> CANDIDATE UPGRADE PASSED"
echo "from=$STABLE_VERSION"
echo "to=$CANDIDATE_VERSION"
echo "candidate_commit=$CANDIDATE_COMMIT"
echo "isolated_home=$PILOT_HOME"
echo "real_user_config_untouched=yes"
echo "========================================"

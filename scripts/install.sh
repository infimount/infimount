#!/usr/bin/env sh
set -eu

REPO="${INFIMOUNT_REPO:-infimount/infimount}"
VERSION="${INFIMOUNT_VERSION:-${1:-latest}}"
FORMAT="${INFIMOUNT_INSTALL_FORMAT:-auto}"
TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t infimount-install)"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

log() {
  printf '%s\n' "$*"
}

fail() {
  printf 'Infimount install failed: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

download() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl --fail --location --silent --show-error "$url" --output "$dest"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$dest"
  else
    fail "curl or wget is required"
  fi
}

run_root() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  else
    need_cmd sudo
    sudo "$@"
  fi
}

release_base_url() {
  if [ -n "${INFIMOUNT_RELEASE_BASE_URL:-}" ]; then
    printf '%s' "$INFIMOUNT_RELEASE_BASE_URL"
    return
  fi

  if [ "$VERSION" = "latest" ]; then
    printf 'https://github.com/%s/releases/latest/download' "$REPO"
  else
    case "$VERSION" in
      v*) tag="$VERSION" ;;
      *) tag="v$VERSION" ;;
    esac
    printf 'https://github.com/%s/releases/download/%s' "$REPO" "$tag"
  fi
}

linux_package_format() {
  if [ "$FORMAT" != "auto" ]; then
    printf '%s' "$FORMAT"
    return
  fi

  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    distro="${ID:-} ${ID_LIKE:-}"
    case "$distro" in
      *debian*|*ubuntu*) printf 'deb'; return ;;
      *fedora*|*rhel*|*centos*|*suse*) printf 'rpm'; return ;;
    esac
  fi

  if command -v apt-get >/dev/null 2>&1 || command -v dpkg >/dev/null 2>&1; then
    printf 'deb'
  elif command -v dnf >/dev/null 2>&1 || command -v yum >/dev/null 2>&1 || command -v rpm >/dev/null 2>&1; then
    printf 'rpm'
  else
    printf 'appimage'
  fi
}

select_asset() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin)
      printf 'Infimount.dmg'
      ;;
    Linux)
      case "$arch" in
        x86_64|amd64) ;;
        *) fail "Linux release assets currently support x86_64 only; detected $arch" ;;
      esac
      case "$(linux_package_format)" in
        deb) printf 'Infimount-amd64.deb' ;;
        rpm) printf 'Infimount-x86_64.rpm' ;;
        appimage|AppImage) printf 'Infimount-x86_64.AppImage' ;;
        *) fail "unsupported INFIMOUNT_INSTALL_FORMAT=$FORMAT; use auto, deb, rpm, or appimage" ;;
      esac
      ;;
    *)
      fail "unsupported OS for install.sh: $os. On Windows use scripts/install.ps1."
      ;;
  esac
}

verify_checksum() {
  asset="$1"
  sums="$TMP_DIR/SHA256SUMS.txt"
  expected="$TMP_DIR/$asset.sha256"

  awk -v file="$asset" '$2 == file { print; found=1 } END { exit found ? 0 : 1 }' "$sums" > "$expected" || fail "checksum entry not found for $asset"

  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$TMP_DIR" && sha256sum -c "$expected") >/dev/null
  elif command -v shasum >/dev/null 2>&1; then
    (cd "$TMP_DIR" && shasum -a 256 -c "$expected") >/dev/null
  else
    fail "sha256sum or shasum is required for checksum verification"
  fi
}

install_linux() {
  asset="$1"
  path="$TMP_DIR/$asset"

  case "$asset" in
    *.deb)
      if command -v apt-get >/dev/null 2>&1; then
        run_root apt-get install -y "$path"
      else
        run_root dpkg -i "$path" || {
          log "Resolving missing .deb dependencies with apt-get -f install..."
          run_root apt-get install -f -y
        }
      fi
      ;;
    *.rpm)
      if command -v dnf >/dev/null 2>&1; then
        run_root dnf install -y "$path"
      elif command -v yum >/dev/null 2>&1; then
        run_root yum install -y "$path"
      else
        run_root rpm -Uvh "$path"
      fi
      ;;
    *.AppImage)
      install_dir="${INFIMOUNT_INSTALL_DIR:-$HOME/.local/bin}"
      app_path="$install_dir/infimount"
      mkdir -p "$install_dir" "$HOME/.local/share/applications"
      cp "$path" "$app_path"
      chmod +x "$app_path"
      cat > "$HOME/.local/share/applications/infimount.desktop" <<EOF_DESKTOP
[Desktop Entry]
Type=Application
Name=Infimount
Comment=Unified Storage Browser
Exec=$app_path
Terminal=false
Categories=Utility;FileManager;
EOF_DESKTOP
      log "Installed AppImage to $app_path"
      case ":$PATH:" in
        *":$install_dir:"*) ;;
        *) log "Tip: add $install_dir to PATH to run 'infimount' from a terminal." ;;
      esac
      ;;
    *)
      fail "unsupported Linux asset: $asset"
      ;;
  esac
}

install_macos() {
  asset="$1"
  dmg="$TMP_DIR/$asset"
  mount_dir="$TMP_DIR/mount"
  apps_dir="${INFIMOUNT_MACOS_INSTALL_DIR:-/Applications}"
  mkdir -p "$mount_dir"

  need_cmd hdiutil
  hdiutil attach "$dmg" -nobrowse -quiet -mountpoint "$mount_dir"
  detach() { hdiutil detach "$mount_dir" -quiet >/dev/null 2>&1 || true; }
  trap 'detach; cleanup' EXIT INT TERM

  app_src="$(find "$mount_dir" -maxdepth 2 -name 'Infimount.app' -type d | head -n 1)"
  [ -n "$app_src" ] || fail "Infimount.app was not found inside $asset"

  if [ -w "$apps_dir" ]; then
    rm -rf "$apps_dir/Infimount.app"
    ditto "$app_src" "$apps_dir/Infimount.app"
  else
    run_root rm -rf "$apps_dir/Infimount.app"
    run_root ditto "$app_src" "$apps_dir/Infimount.app"
  fi

  xattr -dr com.apple.quarantine "$apps_dir/Infimount.app" >/dev/null 2>&1 || true
  log "Installed Infimount to $apps_dir/Infimount.app"
  log "If macOS Gatekeeper warns on first launch, right-click Infimount and choose Open."
}

main() {
  base_url="$(release_base_url)"
  asset="$(select_asset)"

  log "Installing Infimount from $base_url"
  log "Selected asset: $asset"

  download "$base_url/SHA256SUMS.txt" "$TMP_DIR/SHA256SUMS.txt"
  download "$base_url/$asset" "$TMP_DIR/$asset"
  verify_checksum "$asset"
  log "Checksum verified."

  if [ "${INFIMOUNT_INSTALL_DRY_RUN:-0}" = "1" ]; then
    log "Dry run requested; skipping installation."
    return
  fi

  case "$(uname -s)" in
    Linux) install_linux "$asset" ;;
    Darwin) install_macos "$asset" ;;
  esac

  log "Infimount installation complete."
}

main "$@"

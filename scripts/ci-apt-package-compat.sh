#!/usr/bin/env bash
# Sourced by ci-apt-install.sh AFTER apt-get update. No external side effects.
# Keep the Linux desktop pilot runnable across GitHub Ubuntu 24.04 and 26.04.

ci_apt_has_candidate() {
  local candidate
  # Consume the full APT policy output. Early awk exit can trigger
  # SIGPIPE in apt-cache under pipefail, falsely reporting no candidate.
  candidate="$(LC_ALL=C apt-cache policy "$1" | awk '$1 == "Candidate:" { value = $2 } END { print value }')" || return 1
  [[ -n "$candidate" && "$candidate" != "(none)" ]]
}

ci_apt_resolve_packages() {
  CI_APT_PACKAGES=()
  local package
  for package in "$@"; do
    if [[ "$package" == "webkit2gtk-driver" ]] && ! ci_apt_has_candidate "$package"; then
      if ! ci_apt_has_candidate "webkitgtk-webdriver"; then
        echo "Neither Ubuntu WebKit WebDriver package has an APT candidate." >&2
        LC_ALL=C apt-cache policy webkit2gtk-driver webkitgtk-webdriver >&2 || true
        # Only print Ubuntu repository components, not private apt source URLs.
        grep -hs '^Components:' /etc/apt/sources.list.d/*.sources >&2 || true
        return 1
      fi
      package="webkitgtk-webdriver"
      echo "Using Ubuntu's renamed WebKit WebDriver package: $package" >&2
    fi
    CI_APT_PACKAGES+=("$package")
  done
}

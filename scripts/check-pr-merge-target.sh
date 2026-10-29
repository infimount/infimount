#!/usr/bin/env bash
set -euo pipefail

check_target() {
  local base="${1:-}" draft="${2:-}" event="${3:-}"

  case "$event" in
    push|workflow_dispatch) return 0 ;;
    pull_request) ;;
    *)
      echo "::error::Unexpected CI event for PR merge-target gate." >&2
      return 1
      ;;
  esac

  if [[ -z "$base" || ( "$draft" != "true" && "$draft" != "false" ) ]]; then
    echo "::error::PR base branch or draft state is missing." >&2
    return 1
  fi

  if [[ "$draft" == "true" ]]; then
    echo "Draft PR targets '$base'. Before marking ready, rebase and retarget to main."
    return 0
  fi

  case "$base" in
    main|release/v*) return 0 ;;
    *)
      echo "::error::Ready PR targets '$base', not main or a release/v* branch." >&2
      echo "Stacked feature-branch merges can be marked merged while their code never reaches main." >&2
      return 1
      ;;
  esac
}

if [[ "${1:-}" == "--self-test" ]]; then
  check_target "" "" "push"
  check_target "" "" "workflow_dispatch"
  check_target "main" "false" "pull_request"
  check_target "release/v0.8.2-rc.1" "false" "pull_request"
  check_target "feat/stacked" "true" "pull_request"
  for invalid in "feat/stacked false pull_request" "main unknown pull_request" "'' false pull_request"; do
    read -r base draft event <<< "$invalid"
    if [[ "$base" == "''" ]]; then base=""; fi
    if check_target "$base" "$draft" "$event" >/dev/null 2>&1; then
      echo "Invalid PR merge-target fixture was accepted: $invalid" >&2
      exit 1
    fi
  done
  echo "PR merge-target fixtures passed"
  exit 0
fi

check_target "${1:-}" "${2:-}" "${3:-}"

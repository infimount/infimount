#!/usr/bin/env bash
set -euo pipefail

repo="${1:-infimount/infimount}"

cat <<'EOF'
Configure the GitHub main-branch ruleset to require these workflows before
merge:

- CI
- Coverage
- Integration Tests
- Repo Lint
- Dependency Audit
- Release Rehearsal

Also enable:
- require branch to be up to date
- block force pushes
- block branch deletion
- require one approval when an independent reviewer is available

Check-run names can differ from workflow display names. Use the repository
Rules UI after one successful PR run and select the exact checks shown there.
EOF

echo
echo "Repository rules page:"
echo "https://github.com/${repo}/settings/rules"

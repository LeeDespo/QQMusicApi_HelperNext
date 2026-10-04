#!/usr/bin/env bash
# Real read-only smoke. JSON aggregate remains prefixed by SMOKE_RESULT.
# Usage: scripts/port-smoke.sh [helper-dir] [method ...]
# Explicit allowlist rejects every unlisted method before build or dispatch.
# Python 3 is required. Calls default to 30 seconds (PORT_SMOKE_TIMEOUT_SECONDS: 1..120).
# PORT_SMOKE_SKIP_BUILD=1 uses an already-built binary; default build is bounded to 180s.
# PORT_SMOKE_EUIN selects the encrypted UIN for liked-song pages; unset is a visible skip.
# Login operations are never tested; expiry/risk skips remain visible in skips/cases.
set -uo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$REPO_ROOT/scripts/port_read_smoke.py" "$@"

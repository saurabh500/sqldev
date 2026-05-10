#!/usr/bin/env bash
# Drives the sqldev demo recording. Each block matches a scene in
# docs/demo-script.md. Run interactively — the `read -r` pauses let
# you sync with the voiceover.
#
# Usage:
#   ./scripts/demo.sh                 # interactive
#   PAUSE=0 ./scripts/demo.sh         # no waits — for asciinema replay
#
# Requires: sqldev on $PATH, jq, bat, a populated .sqldev.yml,
# DEV_SA_PASSWORD exported, and a reachable SQL Server.

set -euo pipefail

PAUSE="${PAUSE:-1}"

beat() {
  if [[ "$PAUSE" == "1" ]]; then
    printf '\n\033[2m── press enter for next command ──\033[0m'
    read -r _
  fi
}

scene() {
  printf '\n\n\033[1;36m# %s\033[0m\n' "$1"
}

# Scene 2 — Install
scene "Scene 2 — Install"
sqldev --version
beat

# Scene 3 — .sqldev.yml
scene "Scene 3 — .sqldev.yml"
bat --plain .sqldev.yml
beat
sqldev config show
beat

# Scene 4 — Query
scene "Scene 4 — sqldev query"
sqldev query --sql 'SELECT TOP 3 name FROM sys.tables ORDER BY name'
beat
sqldev query \
  --sql 'SELECT TOP 3 name, type_desc FROM sys.objects ORDER BY name' \
  --format json | jq
beat
echo 'SELECT @@VERSION AS v;' | sqldev query --format json | jq -r '.[0].v'
beat

# Scene 5 — Introspect
scene "Scene 5 — sqldev introspect"
sqldev introspect > schema.json
echo "schemas:      $(jq '.schemas | length' schema.json)"
echo "tables:       $(jq '[.schemas[].tables[]] | length' schema.json)"
echo "foreign keys: $(jq '[.schemas[].tables[].foreign_keys[]?] | length' schema.json)"
beat

# Scene 6 — Safety
scene "Scene 6 — Safety"
# Expected to fail; don't kill the script.
sqldev --env prod query --sql 'SELECT 1' --trust-cert true || true
beat

scene "Done. Cut here."

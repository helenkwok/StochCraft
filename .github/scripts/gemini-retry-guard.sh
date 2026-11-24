#!/usr/bin/env bash
# Stops the Gemini CLI in gemini-review.yml once its requests have failed a set number of times
# (two by default), so a 503 "high demand" storm costs two requests instead of ten.
#
# The CLI retries a failed request up to 10 times, and the model policy for the default Flash
# model fixes that number (general.maxAttempts does not lower it). Every attempt counts against
# the free tier's daily request limit. run-gemini-cli sends the CLI's stderr to
# ${RUNNER_TEMP}/gemini-err.*, and the CLI writes one "Attempt N failed ..." line there per failed
# attempt. This script runs in the background, copies that file to ${RUNNER_TEMP}/gemini-guard/
# every two seconds (the copy survives a step timeout), and stops the CLI when the count is
# reached. The action then reports the failure as usual.
#
# Usage: gemini-retry-guard.sh [failed-attempt-limit]
set -uo pipefail

limit="${1:-2}"
state="${RUNNER_TEMP:?}/gemini-guard"
mkdir -p "${state}"

while sleep 2; do
  err="$(find "${RUNNER_TEMP}" -maxdepth 1 -name 'gemini-err.*' -type f -print 2>/dev/null | head -n 1)"
  if [[ -z "${err}" ]]; then
    continue
  fi
  cp "${err}" "${state}/stderr.log" 2>/dev/null || continue
  failed="$(grep -c -E '^Attempt [0-9]+ failed' "${state}/stderr.log" || true)"
  if (( ${failed:-0} >= limit )); then
    echo "${failed}" > "${state}/stopped"
    echo "gemini-retry-guard: ${failed} failed requests; stopping the Gemini CLI." >> "${state}/guard.log"
    # The CLI runs as 'gemini --yolo --prompt ... --output-format json' (and may relaunch itself
    # with the same arguments); nothing else on the runner has that command line.
    pkill -TERM -f -- '--output-format json' || true
    sleep 10
    pkill -KILL -f -- '--output-format json' || true
    exit 0
  fi
done

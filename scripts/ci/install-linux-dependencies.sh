#!/usr/bin/env bash

set -Eeuo pipefail

readonly apt_timeout_seconds="${TABBY_RS_APT_TIMEOUT_SECONDS:-300}"
readonly apt_retries="${TABBY_RS_APT_RETRIES:-3}"
readonly apt_retry_delay_seconds="${TABBY_RS_APT_RETRY_DELAY_SECONDS:-15}"

validate_positive_integer() {
  local name="$1"
  local value="$2"

  if [[ ! "$value" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: ${name} must be a positive integer, got '${value}'" >&2
    exit 2
  fi
}

validate_positive_integer TABBY_RS_APT_TIMEOUT_SECONDS "$apt_timeout_seconds"
validate_positive_integer TABBY_RS_APT_RETRIES "$apt_retries"
validate_positive_integer TABBY_RS_APT_RETRY_DELAY_SECONDS "$apt_retry_delay_seconds"

if (( $# == 0 )); then
  echo "usage: $0 PACKAGE [PACKAGE ...]" >&2
  exit 2
fi

run_apt() {
  local operation="$1"
  shift
  local attempt
  local status=0

  for ((attempt = 1; attempt <= apt_retries; attempt += 1)); do
    echo "::group::apt-get ${operation} (attempt ${attempt}/${apt_retries}, timeout ${apt_timeout_seconds}s)"
    if sudo timeout --signal=TERM --kill-after=30s "${apt_timeout_seconds}s" \
      apt-get -o Acquire::Retries=3 "$@"; then
      echo "::endgroup::"
      return 0
    else
      status=$?
    fi
    echo "::endgroup::"

    if (( attempt < apt_retries )); then
      echo "apt-get ${operation} failed with status ${status}; retrying in ${apt_retry_delay_seconds}s..." >&2
      sleep "$apt_retry_delay_seconds"
    fi
  done

  echo "::error::apt-get ${operation} failed after ${apt_retries} attempts (last status ${status})" >&2
  echo "runner=${RUNNER_OS:-unknown} arch=$(uname -m)" >&2
  return "$status"
}

run_apt update update
run_apt install install --yes --no-install-recommends "$@"

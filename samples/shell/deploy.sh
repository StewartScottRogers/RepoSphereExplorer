#!/usr/bin/env bash
#
# Deploys a release to a host: checks preconditions, uploads the artifact,
# swaps the symlink, restarts the service and rolls back if the health
# check fails. Strict mode throughout, and every step is a function.

set -Eeuo pipefail
shopt -s inherit_errexit 2>/dev/null || true

readonly SCRIPT_NAME="${0##*/}"
readonly DEFAULT_HEALTH_PATH="/healthz"
readonly RELEASES_TO_KEEP=5

TARGET_HOST=""
ARTIFACT=""
HEALTH_PATH="${DEFAULT_HEALTH_PATH}"
DRY_RUN=0
RELEASE_ID="$(date -u +%Y%m%d-%H%M%S)"

log() {
  printf '%s [%s] %s\n' "$(date -u +%H:%M:%S)" "${1}" "${2}" >&2
}

die() {
  log "fatal" "${1}"
  exit "${2:-1}"
}

usage() {
  cat <<USAGE
Usage: ${SCRIPT_NAME} --host HOST --artifact PATH [--health PATH] [--dry-run]

  --host      SSH destination to deploy to
  --artifact  Local tarball to upload
  --health    Health path to poll after restart (default ${DEFAULT_HEALTH_PATH})
  --dry-run   Print what would happen and stop
USAGE
}

parse_arguments() {
  while (($# > 0)); do
    case "${1}" in
      --host) TARGET_HOST="${2:-}"; shift 2 ;;
      --artifact) ARTIFACT="${2:-}"; shift 2 ;;
      --health) HEALTH_PATH="${2:-}"; shift 2 ;;
      --dry-run) DRY_RUN=1; shift ;;
      -h|--help) usage; exit 0 ;;
      *) usage; die "unknown argument: ${1}" 2 ;;
    esac
  done

  [[ -n "${TARGET_HOST}" ]] || die "--host is required" 2
  [[ -n "${ARTIFACT}" ]] || die "--artifact is required" 2
  [[ -f "${ARTIFACT}" ]] || die "no such artifact: ${ARTIFACT}" 2
}

require_tools() {
  local missing=()
  local tool
  for tool in "$@"; do
    command -v "${tool}" >/dev/null 2>&1 || missing+=("${tool}")
  done

  if ((${#missing[@]} > 0)); then
    die "missing required tools: ${missing[*]}"
  fi
}

remote() {
  if ((DRY_RUN)); then
    log "dry-run" "ssh ${TARGET_HOST} $*"
    return 0
  fi
  ssh -o BatchMode=yes -o ConnectTimeout=10 "${TARGET_HOST}" "$@"
}

upload_artifact() {
  local destination="/srv/releases/${RELEASE_ID}.tar.gz"
  log "info" "uploading $(basename "${ARTIFACT}") to ${destination}"

  if ((DRY_RUN)); then
    log "dry-run" "scp ${ARTIFACT} ${TARGET_HOST}:${destination}"
  else
    scp -q "${ARTIFACT}" "${TARGET_HOST}:${destination}"
  fi

  printf '%s' "${destination}"
}

unpack_release() {
  local tarball="${1}"
  local directory="/srv/releases/${RELEASE_ID}"

  remote "mkdir -p '${directory}' && tar -xzf '${tarball}' -C '${directory}'"
  printf '%s' "${directory}"
}

swap_symlink() {
  local directory="${1}"
  remote "ln -sfn '${directory}' /srv/current"
}

restart_service() {
  remote "systemctl --user restart repo-sphere-explorer.service"
}

health_check() {
  local attempt
  for attempt in 1 2 3 4 5; do
    if remote "curl -fsS --max-time 5 http://127.0.0.1:8080${HEALTH_PATH} >/dev/null"; then
      log "info" "healthy on attempt ${attempt}"
      return 0
    fi
    log "warn" "health check ${attempt} failed, retrying"
    sleep $((attempt * 2))
  done
  return 1
}

previous_release() {
  remote "ls -1dt /srv/releases/*/ | sed -n 2p"
}

rollback() {
  local previous
  previous="$(previous_release || true)"
  if [[ -z "${previous}" ]]; then
    die "health check failed and there is nothing to roll back to" 3
  fi

  log "warn" "rolling back to ${previous}"
  swap_symlink "${previous%/}"
  restart_service
}

prune_releases() {
  remote "ls -1dt /srv/releases/*/ | tail -n +$((RELEASES_TO_KEEP + 1)) | xargs -r rm -rf"
}

main() {
  parse_arguments "$@"
  require_tools ssh scp curl tar

  trap 'log "fatal" "interrupted"; exit 130' INT TERM

  local tarball directory
  tarball="$(upload_artifact)"
  directory="$(unpack_release "${tarball}")"

  swap_symlink "${directory}"
  restart_service

  if ! health_check; then
    rollback
    die "deploy rolled back after a failed health check" 4
  fi

  prune_releases
  log "info" "deployed ${RELEASE_ID} to ${TARGET_HOST}"
}

main "$@"

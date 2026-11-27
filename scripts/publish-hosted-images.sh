#!/usr/bin/env bash
# Called only by the canonical, fully gated release publisher after native tests.
set -euo pipefail
mode="${1:?stage or promote required}"
image_dir="${2:?image artifact directory required}"
asset_dir="${3:?release asset directory required}"
tag="${GITHUB_REF_NAME:?release tag required}"
revision="${GITHUB_SHA:?source commit required}"
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]
[[ "$revision" =~ ^[0-9a-f]{40}$ ]]
[[ "${GITHUB_REPOSITORY:-}" = "infimount/infimount" ]]
[[ "${GITHUB_REF_TYPE:-}" = "tag" ]]
image="ghcr.io/infimount/infimount-hosted"
version="${tag#v}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$asset_dir"
if [ "$mode" = "promote" ]; then
  # Only the final publisher calls this, after validating its downloaded draft.
  manifest="$asset_dir/hosted-image.json"
  jq -e --arg image "$image" --arg tag "$tag" --arg revision "$revision" '
    .schema_version == 1 and .image == $image and .tag == $tag and .source_commit == $revision
  ' "$manifest" >/dev/null
  digest="$(jq -er '.digest' "$manifest")"
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]
  # Prove repository read access and preserve the already attested exact index.
  docker buildx imagetools inspect "$image@$digest" --format '{{json .Manifest}}' > "$work/staged.json"
  jq -e --slurpfile expected "$manifest" '
    .digest == $expected[0].digest and
    ([.manifests[] | {platform:(.platform.os + "/" + .platform.architecture),digest}] | sort_by(.platform)) ==
    ([$expected[0].platforms[] | {platform,digest}] | sort_by(.platform))
  ' "$work/staged.json" >/dev/null
  if docker buildx imagetools inspect "$image:$tag" --format '{{json .Manifest}}' > "$work/existing.json" 2> "$work/inspect.err"; then
    test "$(jq -er '.digest' "$work/existing.json")" = "$digest" || {
      echo "Refusing to replace an existing release image with different bytes." >&2
      exit 1
    }
  else
    # An authentication/keychain/repository lookup failure is not absence of
    # this manifest. Only the exact missing-reference diagnostic permits create.
    diagnostic="$(cat "$work/inspect.err")"
    if [ "$diagnostic" != "ERROR: $image:$tag: not found" ] && [ "$diagnostic" != "$image:$tag: not found" ]; then
      echo "Registry state could not be established. Release image promotion stopped." >&2
      exit 1
    fi
    docker buildx imagetools create --tag "$image:$tag" "$image@$digest"
  fi
  actual="$(docker buildx imagetools inspect "$image:$tag" --format '{{json .Manifest}}' | jq -er '.digest')"
  test "$actual" = "$digest"
  printf 'Promoted validated release image: %s@%s\n' "$image" "$digest"
  exit 0
fi
[[ "$mode" = "stage" ]]
digests=()
for arch in amd64 arm64; do
  archive="$image_dir/$arch.tar.gz"
  inspection="$image_dir/$arch.inspect.json"
  test -s "$archive"
  test -s "$inspection"
  jq -e --arg arch "$arch" --arg revision "$revision" --arg version "$version" '
    length == 1 and .[0].Architecture == $arch and .[0].Os == "linux" and
    .[0].Config.User == "65532:65532" and
    .[0].Config.Labels["org.opencontainers.image.revision"] == $revision and
    .[0].Config.Labels["org.opencontainers.image.version"] == $version and
    .[0].Config.Labels["org.opencontainers.image.source"] == "https://github.com/infimount/infimount"
  ' "$inspection" >/dev/null
  config_digest="$(jq -er '.[0].Id' "$inspection")"
  [[ "$config_digest" =~ ^sha256:[0-9a-f]{64}$ ]]
  docker load --input "$archive"
  local_image="infimount-hosted:qualification-$arch"
  test "$(docker image inspect "$local_image" --format '{{.Id}}')" = "$config_digest"
  # Unique staging tags never move a published version or stable alias.
  staging="$image:build-${GITHUB_RUN_ID:?}-${GITHUB_RUN_ATTEMPT:?}-$arch"
  docker tag "$local_image" "$staging"
  docker push "$staging"
  docker buildx imagetools inspect "$staging" --format '{{json .Manifest}}' > "$work/$arch.remote.json"
  digest="$(jq -er '.digest' "$work/$arch.remote.json")"
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]
  docker buildx imagetools inspect "$image@$digest" --raw > "$work/$arch.manifest.json"
  test "$(jq -er '.config.digest' "$work/$arch.manifest.json")" = "$config_digest"
  digests+=("$image@$digest")
  jq -n --arg platform "linux/$arch" --arg digest "$digest" --arg config "$config_digest" \
    '{platform:$platform,digest:$digest,config_digest:$config}' > "$work/$arch.json"
done

# Stage a combined digest, never a final release version. Remaining asset,
# signature, installer and draft-download validation gates still have to pass.
staging_index="$image:build-${GITHUB_RUN_ID:?}-${GITHUB_RUN_ATTEMPT:?}-index"
docker buildx imagetools create --tag "$staging_index" "${digests[@]}"
docker buildx imagetools inspect "$staging_index" --format '{{json .Manifest}}' > "$work/index.json"
digest="$(jq -er '.digest' "$work/index.json")"
[[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]
jq -e --arg amd64 "${digests[0]#*@}" --arg arm64 "${digests[1]#*@}" '
  (.manifests | length == 2) and
  any(.manifests[]; .platform.os == "linux" and .platform.architecture == "amd64" and .digest == $amd64) and
  any(.manifests[]; .platform.os == "linux" and .platform.architecture == "arm64" and .digest == $arm64)
' "$work/index.json" >/dev/null
jq -s --arg tag "$tag" --arg revision "$revision" --arg image "$image" --arg digest "$digest" \
  '{schema_version:1,tag:$tag,source_commit:$revision,image:$image,digest:$digest,platforms:.}' \
  "$work/amd64.json" "$work/arm64.json" > "$asset_dir/hosted-image.json"
printf 'image=%s\ndigest=%s\n' "$image" "$digest" >> "${GITHUB_OUTPUT:?}"
printf 'Staged qualified image: %s@%s\n' "$image" "$digest"

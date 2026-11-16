#!/usr/bin/env bash
set -euo pipefail

dmg=${1:?Supply the Dockon DMG path}
test_dir=$(mktemp -d /tmp/dockon-dmg.XXXXXX)
mount="$test_dir/mount"
mkdir "$mount"
export DOCKON_DATA_DIR="$test_dir/data"
export DOCKON_WWW_DIR="$test_dir/www"
pid=""
ctl=""
cleanup() {
  if [[ -n "$ctl" ]]; then "$ctl" shutdown >/dev/null 2>&1 || true; fi
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  hdiutil detach "$mount" -quiet || true
}
trap cleanup EXIT

hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$mount" -quiet
app=("$mount"/*.app)
bin="${app[0]}/Contents/MacOS"
desktop=$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "${app[0]}/Contents/Info.plist")
for binary in "$desktop" dockon-daemon dockon-runner dockon-hosts-helper dockonctl; do
  test -x "$bin/$binary"
  lipo -verify_arch "$(uname -m)" "$bin/$binary"
done

ctl="$bin/dockonctl"
"$bin/$desktop" >"$test_dir/desktop.log" 2>&1 &
pid=$!
for ((attempt=0; attempt<60; attempt++)); do
  if "$ctl" health >/dev/null 2>&1; then break; fi
  kill -0 "$pid" 2>/dev/null || break
  sleep 1
done
if ! "$ctl" health; then
  cat "$test_dir/desktop.log" >&2
  exit 1
fi

cat >"$test_dir/site.cjs" <<'JS'
require("node:http").createServer((_request, response) => {
  response.end("Dockon macOS");
}).listen(Number(process.env.PORT), "127.0.0.1");
JS
node -e 'console.log(JSON.stringify({
  schema_version: 1,
  id: "mac-smoke",
  name: "Mac smoke",
  root: process.argv[1],
  executable: process.execPath,
  args: ["site.cjs"],
  port: 0,
}))' "$test_dir" >"$test_dir/site.json"
"$ctl" register "$test_dir/site.json"
operation=$("$ctl" start mac-smoke | jq -er '.data.id')
for ((attempt=0; attempt<60; attempt++)); do
  status=$("$ctl" operation "$operation" | jq -er '.data.status')
  if [[ "$status" != running ]]; then break; fi
  sleep 1
done
if [[ "$status" != completed ]]; then
  "$ctl" operation "$operation"
  exit 1
fi
port=$("$ctl" list | jq -er '.data[] | select(.manifest.id == "mac-smoke") | .port')
test "$(curl --fail --silent --show-error --max-time 10 "http://127.0.0.1:$port/")" = "Dockon macOS"
echo "PASS mounted DMG, native desktop startup, bundled daemon and project HTTP"

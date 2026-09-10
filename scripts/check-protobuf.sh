#!/usr/bin/env bash
# Verify the compiler can resolve the standard imports required by LanceDB.
set -euo pipefail
probe_dir="$(mktemp -d)"
trap 'rm -r "$probe_dir"' EXIT
cat > "$probe_dir/probe.proto" <<'PROTO'
syntax = "proto3";
import "google/protobuf/empty.proto";
message Probe { google.protobuf.Empty value = 1; }
PROTO
"${PROTOC:-protoc}" --proto_path="$probe_dir" \
  --descriptor_set_out="$probe_dir/probe.pb" "$probe_dir/probe.proto"
test -s "$probe_dir/probe.pb"

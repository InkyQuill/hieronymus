# Source from the repository root for resource-bounded local qualification.
# The preserved optional Bun binary is local-only; CI installs pinned Bun separately.
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
export RUST_TEST_THREADS=2
if [ -x "$PWD/qualification/.artifacts/desktop-tools/bun-linux-x64/bun" ]; then
  export PATH="$PWD/qualification/.artifacts/desktop-tools/bun-linux-x64:$PATH"
fi

#!/usr/bin/env just --justfile

@_default:
    just --list

# Clean all build artifacts
clean:
    cargo clean

# Update dependencies, including breaking changes
update:
    cargo +nightly -Z unstable-options update --breaking
    cargo update

# Run cargo clippy
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Test code formatting
test-fmt:
    cargo fmt --all -- --check

# Run cargo fmt
fmt:
    cargo +nightly fmt -- --config imports_granularity=Module,group_imports=StdExternalCrate

# Build and open code documentation
docs:
    cargo doc --no-deps --open

# Quick compile
check:
    cargo check --all-targets --workspace

# Default build
build:
    cargo build --all-targets --workspace

# Run all tests
test *ARGS: build
    cargo test --all-targets --workspace {{ARGS}}

# Find unused dependencies. Install it with `cargo install cargo-udeps`
udeps:
    cargo +nightly udeps --all-targets --workspace

rust-info:
    rustc --version
    cargo --version

# Run all tests as expected by CI
ci-test: rust-info test-fmt clippy test

# Verify that the current version of the crate is not the same as the one published on crates.io
check-if-published CRATE_NAME="vmod-reqwest":
    #!/usr/bin/env bash
    set -euo pipefail
    LOCAL_VERSION="$(cargo metadata --format-version 1 | jq -r --arg CRATE_NAME {{quote(CRATE_NAME)}}  '.packages | map(select(.name == $CRATE_NAME)) | first | .version')"
    echo "Detected crate {{CRATE_NAME}} version:  $LOCAL_VERSION"
    PUBLISHED_VERSION="$(cargo search --quiet {{quote(CRATE_NAME)}} | grep "^{{CRATE_NAME}} =" | sed -E 's/.* = "(.*)".*/\1/')"
    echo "Published crate version: $PUBLISHED_VERSION"
    if [ "$LOCAL_VERSION" = "$PUBLISHED_VERSION" ]; then
        echo "ERROR: The current crate version has already been published."
        exit 1
    else
        echo "The current crate version has not yet been published."
    fi

[private]
docker-build-ver VERSION:
    docker build \
           --progress=plain \
           -t "varnish-img-{{VERSION}}" \
           {{ if VERSION == "latest" { "" } else { "--build-arg VARNISH_VERSION_TAG=varnish" + VERSION } }} \
           --build-arg USER_UID=$(id -u) \
           --build-arg USER_GID=$(id -g) \
           -f docker/Dockerfile \
           .

[private]
docker-run-ver VERSION *ARGS:
    mkdir -p docker/.cache/{{VERSION}}
    touch docker/.cache/{{VERSION}}/.bash_history
    docker run --rm -it \
        -v "$PWD:/app/" \
        -v "$PWD/docker/.cache/{{VERSION}}:/home/user/.cache" \
        -v "$PWD/docker/.cache/{{VERSION}}/.bash_history:/home/user/.bash_history" \
        varnish-img-{{VERSION}} {{ARGS}}

docker-run-latest *ARGS: (docker-build-ver "latest") (docker-run-ver "latest" ARGS)
docker-run-77 *ARGS: (docker-build-ver "77") (docker-run-ver "77" ARGS)
docker-run-76 *ARGS: (docker-build-ver "76") (docker-run-ver "76" ARGS)

# Install Varnish from packagecloud.io. This could be damaging to your system - use with caution.
[private]
install-varnish TAG="varnish77":
    #!/usr/bin/env bash
    set -euo pipefail
    curl -sSf "https://packagecloud.io/install/repositories/varnishcache/{{TAG}}/script.deb.sh" | sudo bash
    echo -e 'Package: varnish varnish-dev\nPin: origin "packagecloud.io"\nPin-Priority: 1001' | sudo tee /etc/apt/preferences.d/varnish
    cat /etc/apt/preferences.d/varnish
    sudo apt-cache policy varnish
    sudo apt-get install -y varnish varnish-dev

#!/usr/bin/env just --justfile

main_crate := 'vmod-reqwest'
features_flag := '--all-features'

# which version of Varnish to install by default.
default_varnish_ver := '7.7'

# if running in CI, treat warnings as errors by setting RUSTFLAGS and RUSTDOCFLAGS to '-D warnings' unless they are already set
# Use `CI=true just ci-test` to run the same tests as in GitHub CI.
# Use `just env-info` to see the current values of RUSTFLAGS and RUSTDOCFLAGS
ci_mode := if env('CI', '') != '' {'1'} else {''}
export RUSTFLAGS := env('RUSTFLAGS', if ci_mode == '1' {'-D warnings'} else {''})
export RUSTDOCFLAGS := env('RUSTDOCFLAGS', if ci_mode == '1' {'-D warnings'} else {''})
export RUST_BACKTRACE := env('RUST_BACKTRACE', if ci_mode == '1' {'1'} else {''})

@_default:
    {{just_executable()}} --list

# Build the project
build:
    cargo build --workspace --all-targets {{features_flag}}

# Quick compile without building a binary
check:
    cargo check --workspace --all-targets {{features_flag}}

# Generate code coverage report to upload to codecov.io
ci-coverage: env-info && \
            (coverage '--codecov --output-path target/llvm-cov/codecov.info')
    # ATTENTION: the full file path above is used in the CI workflow
    mkdir -p target/llvm-cov

# Run all tests as expected by CI
ci-test: env-info test-fmt build clippy test && assert-git-is-clean

# Run tests only relevant to the latest Varnish version
ci-test-latest: ci-test test-doc

# Clean all build artifacts
clean:
    cargo clean
    rm -f Cargo.lock

# Clean all build artifacts and docker cache
clean-all: clean
    rm -rf docker/.cache/*
    touch docker/.cache/empty_file

# Run cargo clippy to lint the code
clippy *args:
    cargo clippy --workspace --all-targets {{features_flag}} {{args}}

# Generate code coverage report. Will install `cargo llvm-cov` if missing.
coverage *args='--no-clean --open':  (cargo-install 'cargo-llvm-cov')
    #!/usr/bin/env bash
    set -euo pipefail
    find . -name '*.profraw' | xargs rm
    rm -rf ./target/debug/coverage
    export LLVM_PROFILE_FILE="varnish-%p-%m.profraw"
    export RUSTFLAGS="-Cinstrument-coverage"
    cargo build --workspace --all-targets {{features_flag}}
    VARNISHTEST_DURATION=10 cargo test --workspace --all-targets {{features_flag}}
    grcov . -s . --binary-path ./target/debug/ -t html --branch --ignore-not-existing -o ./target/debug/coverage/
    open ./target/debug/coverage/index.html
    #
    # TODO: use llvm-cov instead:
    # cargo llvm-cov --workspace --all-targets {{features_flag}} --include-build-script {{args}}

docker-run version=default_varnish_ver *args='':  (docker-build-ver version) (docker-run-ver version args)

# Build and open code documentation
docs *args='--open':
    DOCS_RS=1 cargo doc --no-deps {{args}} --workspace

# Print environment info
env-info:
    @echo "Running {{if ci_mode == '1' {'in CI mode'} else {'in dev mode'} }} on {{os()}} / {{arch()}}"
    {{just_executable()}} --version
    rustc --version
    cargo --version
    rustup --version
    @echo "RUSTFLAGS='$RUSTFLAGS'"
    @echo "RUSTDOCFLAGS='$RUSTDOCFLAGS'"
    @echo "RUST_BACKTRACE='$RUST_BACKTRACE'"

# Reformat all code `cargo fmt`. If nightly is available, use it for better results
fmt:
    #!/usr/bin/env bash
    set -euo pipefail
    if rustup component list --toolchain nightly | grep rustfmt &> /dev/null; then
        echo 'Reformatting Rust code using nightly Rust fmt to sort imports'
        cargo +nightly fmt --all -- --config imports_granularity=Module,group_imports=StdExternalCrate
    else
        echo 'Reformatting Rust with the stable cargo fmt.  Install nightly with `rustup install nightly` for better results'
        cargo fmt --all
    fi

# Run all unit and integration tests
test *args: build
    VARNISHTEST_DURATION=10 cargo test --workspace --all-targets {{features_flag}} {{args}}

# Test documentation generation
test-doc:  (docs '')

# Test code formatting
test-fmt:
    cargo fmt --all -- --check

# Find unused dependencies. Install it with `cargo install cargo-udeps`
udeps:  (cargo-install 'cargo-udeps')
    cargo +nightly udeps --workspace --all-targets {{features_flag}}

# Update all dependencies, including breaking changes. Requires nightly toolchain (install with `rustup install nightly`)
update:
    cargo +nightly -Z unstable-options update --breaking
    cargo update

# Ensure that a certain command is available
[private]
assert-cmd command:
    @if ! type {{command}} > /dev/null; then \
        echo "Command '{{command}}' could not be found. Please make sure it has been installed on your computer." ;\
        exit 1 ;\
    fi

# Make sure the git repo has no uncommitted changes
[private]
assert-git-is-clean:
    @if [ -n "$(git status --untracked-files --porcelain)" ]; then \
      >&2 echo "ERROR: git repo is no longer clean. Make sure compilation and tests artifacts are in the .gitignore, and no repo files are modified." ;\
      >&2 echo "######### git status ##########" ;\
      git status ;\
      git --no-pager diff ;\
      exit 1 ;\
    fi

# Check if a certain Cargo command is installed, and install it if needed
[private]
cargo-install $COMMAND $INSTALL_CMD='' *args='':
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v $COMMAND > /dev/null; then
        if ! command -v cargo-binstall > /dev/null; then
            echo "$COMMAND could not be found. Installing it with    cargo install ${INSTALL_CMD:-$COMMAND} --locked {{args}}"
            cargo install ${INSTALL_CMD:-$COMMAND} --locked {{args}}
        else
            echo "$COMMAND could not be found. Installing it with    cargo binstall ${INSTALL_CMD:-$COMMAND} --locked {{args}}"
            cargo binstall ${INSTALL_CMD:-$COMMAND} --locked {{args}}
        fi
    fi

# Build a Docker image with the given Varnish version
[private]
docker-build-ver version=default_varnish_ver:
    docker build \
           --progress=plain \
           -t "varnish-img-{{version}}" \
           {{ '--build-arg VARNISH_VERSION=' + version }} \
           --build-arg USER_UID=$(id -u) \
           --build-arg USER_GID=$(id -g) \
           -f docker/Dockerfile \
           .

# Start docker container with the given varnish version
[private]
docker-run-ver version *args:
    mkdir -p docker/.cache/{{version}}
    touch docker/.cache/{{version}}/.bash_history
    docker run --rm -it \
        -v "$PWD:/app/" \
        -v "$PWD/docker/.cache/{{version}}:/home/user/.cache" \
        -v "$PWD/docker/.cache/{{version}}/.bash_history:/home/user/.bash_history" \
        varnish-img-{{version}} {{args}}

# Install Varnish from packagecloud.io. This could be damaging to your system - use with caution. Pass non-empty `debug` argument to skip the installation.
[private]
install-varnish version=default_varnish_ver debug='':
    #!/usr/bin/env bash
    set -euo pipefail

    # Assumes major and minor are one digit each. Two digits without dots are treated as (major.minor).
    #  60 or 6.0 -> varnishcache/varnish60lts
    #        7.1 -> varnishcache/varnish71
    #   6.0.14r3 -> varnishplus/60-enterprise

    # Convert version to a tag name used as URL portion
    URL_REPO='{{ if version =~ '^\d\.\d\.\d+r\d+$' { \
        'varnishplus/' + replace_regex(version, '^(\d)\.(\d)\..*$', '$1$2') + '-enterprise' \
    } else if version =~ '^(\d\d|\d(\.\d(\.\d+)?)?)$' { \
        'varnishcache/varnish' + replace_regex(replace_regex(replace_regex(replace_regex(replace_regex(version, \
        '^(\d)(\d)$', '$1.$2') \
        , '^(\d\.\d)(\..*)$', '$1') \
        , '^(\d)$', '$1.0') \
        , '^(\d)\.(\d)$', '$1$2') \
        , '^60$', '60lts') \
    } else { \
      error('Invalid version "' + version + '"') \
    } }}'

    # Policy name is either 'varnish' or 'varnish-plus'
    POLICY='{{ if version =~ '^\d\.\d\.\d+r\d+$' { 'varnish-plus' } else { 'varnish' } }}'

    # Ensure version is valid and convert it to an apt package search string. Assumes major and minor parts are one digit. Two digits are treated as (major.minor).
    PATTERN='{{ if version =~ '^\d\.\d\.\d+r\d+$' { \
        version + '*' \
    } else { \
        replace_regex(replace_regex(replace_regex(version, \
              '^(\d)(\d)$', '$1.$2') \
            , '^(\d\.\d\.\d)$', '$1-') \
            , '^(\d(\.\d)*)$', '$1.') \
        + '*' \
    } }}'

    echo "Installing Varnish '{{version}}' (url_repo='$URL_REPO', pattern='$PATTERN') from packagecloud.io"
    {{ if debug != '' {'exit 0'} else {''} }}

    set -x
    curl -sSf "https://packagecloud.io/install/repositories/$URL_REPO/script.deb.sh" | sudo bash

    {{ if version =~ '^\d\.\d\.\d+r\d+$' {''} else { '''
        echo -e 'Package: varnish varnish-dev\nPin: origin "packagecloud.io"\nPin-Priority: 1001' | sudo tee /etc/apt/preferences.d/varnish
        cat /etc/apt/preferences.d/varnish
    ''' } }}

    sudo apt-cache policy "${POLICY}"
    sudo apt-get install -y "${POLICY}=$PATTERN" "${POLICY}-dev=$PATTERN"

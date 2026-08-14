#!/usr/bin/env bash

set -euo pipefail

python3 scripts/check_plugin_package.py
python3 scripts/check_marketplace_consistency.py
python3 tests/plugin/test_marketplace_consistency.py
python3 tests/plugin/test_local_marketplace.py
python3 tests/plugin/test_package_policy.py
python3 tests/plugin/test_archive.py
python3 tests/plugin/test_supply_chain.py
python3 tests/plugin/test_submission_ready.py
cargo test --locked -p lili-session lifecycle_adapter_does_not_retain_prompt_or_permission_details
cargo test --locked -p lili-session plugin_diagnostics_require_observed_delivery_for_trust
cargo test --locked -p lili-session credentials_rotate_and_debug_output_redacts_the_secret
cargo test --locked -p lili --test permission_hook
cargo test --locked -p lili --features release-tools --test plugin_hook
